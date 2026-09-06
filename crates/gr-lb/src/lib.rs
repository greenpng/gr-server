//! Multi-node load-balancer engine (docs/08 — paid module).
//!
//! Pure routing logic, no IO except what callers wire in:
//! - [`LbPool::set_config`] swap config atomically (panel PUT / hot reload).
//! - [`LbPool::set_nodes`] feed the cluster heartbeat snapshot (control plane
//!   sync loop); self node is excluded by the caller.
//! - [`LbPool::report_health`] merge active-probe results (phase-1 health check).
//! - [`LbPool::select`] returns a [`LbTarget`]:
//!   * `Forward` — Pingora forwards the **unchanged** request stream to the
//!     chosen node (pure header passthrough; no header mutations anywhere).
//!   * `Redirect` — 302 with a sticky cookie (`grlb=<node_id>`) when enabled.
//!
//! Modes (user decisions, docs/08):
//! - `proxy`       : forward visitor request to best node.
//! - `redirect`    : 302 → `{scheme}://{advertise}{path?query}`.
//! - `internal_ip` : visitor from an internal CIDR → 302 to the node's
//!   `internal_addr` (LAN hop); everyone else follows `redirect` semantics.
//! No node-count cap — paid entitlement is unlimited nodes.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, RwLock};

/// Sticky cookie name (value = node_id).
pub const STICKY_COOKIE: &str = "grlb";
/// 302 redirect mode.
pub const MODE_REDIRECT: &str = "redirect";
pub const MODE_PROXY: &str = "proxy";
pub const MODE_INTERNAL_IP: &str = "internal_ip";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LbMode {
    Proxy,
    Redirect,
    InternalIp,
}

impl LbMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            MODE_PROXY => Some(Self::Proxy),
            MODE_REDIRECT => Some(Self::Redirect),
            MODE_INTERNAL_IP => Some(Self::InternalIp),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Proxy => MODE_PROXY,
            Self::Redirect => MODE_REDIRECT,
            Self::InternalIp => MODE_INTERNAL_IP,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    RoundRobin,
    LeastInflight,
    IpHash,
}

impl Strategy {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "round_robin" | "round-robin" | "rr" => Some(Self::RoundRobin),
            "least_inflight" | "least-inflight" | "least_conn" => Some(Self::LeastInflight),
            "ip_hash" | "iphash" | "ip_hash/sticky" => Some(Self::IpHash),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RoundRobin => "round_robin",
            Self::LeastInflight => "least_inflight",
            Self::IpHash => "ip_hash",
        }
    }
}

/// CIDR → preferred nodes rule for `internal_ip` mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CidrRule {
    /// `a.b.c.d/nn` or IPv6 `/nn`.
    pub cidr: String,
    /// Preferred node_ids (in priority order) when visitor matches.
    #[serde(default)]
    pub nodes: Vec<String>,
}

/// Panel/admin config (persisted in PG `admin_settings`, key `lb_config_v1`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LbConfig {
    pub enabled: bool,
    pub mode: String,
    pub strategy: String,
    pub url_scheme: String,
    pub sticky_cookie: bool,
    pub node_weights: HashMap<String, u32>,
    pub internal_cidrs: Vec<CidrRule>,
    pub active_health_check: bool,
    pub health_check_interval_ms: u64,
    pub health_check_timeout_ms: u64,
    pub health_check_fail_threshold: u32,
    /// Proxy mode: TLS to upstream nodes.
    pub tls_upstream: bool,
}

impl Default for LbConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: MODE_PROXY.into(),
            strategy: Strategy::RoundRobin.as_str().into(),
            url_scheme: "http".into(),
            sticky_cookie: false,
            node_weights: HashMap::new(),
            internal_cidrs: Vec::new(),
            active_health_check: true,
            health_check_interval_ms: 5_000,
            health_check_timeout_ms: 2_000,
            health_check_fail_threshold: 3,
            tls_upstream: false,
        }
    }
}

impl LbConfig {
    pub fn parsed_mode(&self) -> Option<LbMode> {
        LbMode::parse(&self.mode)
    }
    pub fn parsed_strategy(&self) -> Option<Strategy> {
        Strategy::parse(&self.strategy)
    }
    /// Normalize + validate; returns Err with a panel-readable message.
    pub fn validate(&self) -> Result<(), String> {
        if self.parsed_mode().is_none() {
            return Err(format!(
                "mode must be one of {MODE_PROXY}|{MODE_REDIRECT}|{MODE_INTERNAL_IP}"
            ));
        }
        if self.parsed_strategy().is_none() {
            return Err("strategy must be round_robin|least_inflight|ip_hash".into());
        }
        if self.url_scheme != "http" && self.url_scheme != "https" {
            return Err("url_scheme must be http|https".into());
        }
        if self.health_check_interval_ms < 1000 || self.health_check_interval_ms > 300_000 {
            return Err("health_check_interval_ms must be 1000..300000".into());
        }
        if self.health_check_timeout_ms < 100 || self.health_check_timeout_ms > 30_000 {
            return Err("health_check_timeout_ms must be 100..30000".into());
        }
        if self.health_check_fail_threshold < 1 || self.health_check_fail_threshold > 20 {
            return Err("health_check_fail_threshold must be 1..20".into());
        }
        for r in &self.internal_cidrs {
            if parse_cidr(&r.cidr).is_none() {
                return Err(format!("bad internal cidr: {}", r.cidr));
            }
        }
        Ok(())
    }
}

/// One balanceable node (built from `gr_cluster::NodeInfo` by the sync loop).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LbNode {
    pub node_id: String,
    /// Gateway address used for forwarding / 302 — `host:port`.
    pub advertise: String,
    /// Optional LAN address for `internal_ip` mode.
    pub internal_addr: Option<String>,
    /// Cluster-reported in-flight load (least_inflight strategy).
    pub load_inflight: u64,
    /// Cluster-reported degraded flag (health check / cold boot).
    pub degraded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LbTarget {
    /// Proxy mode: forward the request stream untouched.
    Forward { addr: String, tls: bool, sni: String },
    /// 302 target + optional sticky cookie value.
    Redirect { location: String, sticky: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NodeHealthView {
    pub node_id: String,
    pub advertise: String,
    pub internal_addr: Option<String>,
    pub degraded: bool,
    pub load_inflight: u64,
    pub probe_ok: bool,
    pub probe_fails: u32,
    pub active_health_check: bool,
    pub healthy: bool,
}

#[derive(Debug, Clone)]
struct NodeHealth {
    probe_ok: bool,
    fails: u32,
}

struct Inner {
    config: LbConfig,
    nodes: HashMap<String, LbNode>,
    health: HashMap<String, NodeHealth>,
    rr: usize,
}

#[derive(Clone)]
pub struct LbPool {
    inner: Arc<RwLock<Inner>>,
}

impl std::fmt::Debug for LbPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LbPool(enabled={})", self.is_enabled())
    }
}

impl LbPool {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(Inner {
                config: LbConfig::default(),
                nodes: HashMap::new(),
                health: HashMap::new(),
                rr: 0,
            })),
        }
    }

    pub fn set_config(&self, cfg: LbConfig) {
        let mut g = self.inner.write().unwrap();
        g.config = cfg;
        // Reset RR cursor on config swap (weight changes must not repeat tails).
        g.rr = 0;
    }

    pub fn config(&self) -> LbConfig {
        self.inner.read().unwrap().config.clone()
    }

    /// Replace the node set (cluster snapshot diff). Nodes absent from the new
    /// feed are dropped (stale peer removed); health state is preserved for
    /// nodes that stay.
    pub fn set_nodes(&self, nodes: Vec<LbNode>) {
        let mut g = self.inner.write().unwrap();
        let mut next: HashMap<String, LbNode> = HashMap::new();
        for n in nodes {
            next.insert(n.node_id.clone(), n);
        }
        let mut health = std::mem::take(&mut g.health);
        health.retain(|id, _| next.contains_key(id));
        g.health = health;
        if g.health.is_empty() && !g.config.active_health_check {
            // Seed health entries so views render uniformly.
            for id in next.keys() {
                g.health.entry(id.clone()).or_insert(NodeHealth {
                    probe_ok: true,
                    fails: 0,
                });
            }
        }
        g.nodes = next;
    }

    pub fn nodes(&self) -> Vec<LbNode> {
        self.inner.read().unwrap().nodes.values().cloned().collect()
    }

    pub fn report_health(&self, node_id: &str, ok: bool) {
        let mut g = self.inner.write().unwrap();
        let cfg = g.config.clone();
        let h = g.health.entry(node_id.to_string()).or_insert(NodeHealth {
            probe_ok: true,
            fails: 0,
        });
        if ok {
            h.probe_ok = true;
            h.fails = 0;
        } else {
            h.fails = h.fails.saturating_add(1);
            if h.fails >= cfg.health_check_fail_threshold.max(1) {
                h.probe_ok = false;
            }
        }
    }

    /// Active-probe targets: (node_id, http_addr) of every node when enabled.
    pub fn probe_targets(&self) -> Vec<(String, String)> {
        let g = self.inner.read().unwrap();
        if !g.config.active_health_check {
            return Vec::new();
        }
        let mut out: Vec<(String, String)> = g
            .nodes
            .values()
            .map(|n| (n.node_id.clone(), n.advertise.clone()))
            .collect();
        out.sort();
        out
    }

    /// Live status view for panel/ops.
    pub fn status_view(&self) -> serde_json::Value {
        let g = self.inner.read().unwrap();
        let cfg = &g.config;
        let mut nodes: Vec<NodeHealthView> = g
            .nodes
            .iter()
            .map(|(id, n)| {
                let h = g.health.get(id).cloned().unwrap_or(NodeHealth {
                    probe_ok: true,
                    fails: 0,
                });
                let active_check = cfg.active_health_check;
                let healthy = !n.degraded && (!active_check || h.probe_ok);
                NodeHealthView {
                    node_id: n.node_id.clone(),
                    advertise: n.advertise.clone(),
                    internal_addr: n.internal_addr.clone(),
                    degraded: n.degraded,
                    load_inflight: n.load_inflight,
                    probe_ok: h.probe_ok,
                    probe_fails: h.fails,
                    active_health_check: active_check,
                    healthy,
                }
            })
            .collect();
        nodes.sort_by(|a, b| a.node_id.cmp(&b.node_id));
        serde_json::json!({
            "enabled": cfg.enabled,
            "mode": cfg.mode,
            "strategy": cfg.strategy,
            "url_scheme": cfg.url_scheme,
            "sticky_cookie": cfg.sticky_cookie,
            "active_health_check": cfg.active_health_check,
            "nodes": nodes,
            "healthy_count": nodes.iter().filter(|n| n.healthy).count(),
            "total_nodes": nodes.len(),
        })
    }

    pub fn is_enabled(&self) -> bool {
        let g = self.inner.read().unwrap();
        g.config.enabled && !g.nodes.is_empty()
    }

    /// Route a visitor request. `visitor` = real client IP (used for
    /// internal-IP CIDR + ip_hash); `sticky` = `grlb` cookie value (node_id);
    /// `path_and_query` = raw `path?query` for 302 Location.
    pub fn select(
        &self,
        visitor: Option<IpAddr>,
        sticky: Option<&str>,
        path_and_query: &str,
    ) -> Option<LbTarget> {
        let mut g = self.inner.write().unwrap();
        if !g.config.enabled || g.nodes.is_empty() {
            return None;
        }
        let cfg = g.config.clone();
        let mut order: Vec<String> = g.nodes.keys().cloned().collect();
        order.sort();
        if order.is_empty() {
            return None;
        }
        // Healthy candidates (± active probe state).
        let ui = |id: &str| {
            let h = g.health.get(id).cloned().unwrap_or(NodeHealth {
                probe_ok: true,
                fails: 0,
            });
            let n = g.nodes.get(id).cloned().unwrap();
            !n.degraded && (!cfg.active_health_check || h.probe_ok)
        };
        let candidates: Vec<String> = order
            .iter()
            .filter(|id| ui(id))
            .cloned()
            .collect();
        if candidates.is_empty() {
            return None;
        }

        // Internal-IP CIDR match (mode only): prefer rule-listed nodes.
        let internal = if cfg.parsed_mode() == Some(LbMode::InternalIp) {
            visitor.and_then(|ip| {
                cfg.internal_cidrs
                    .iter()
                    .find(|r| parse_cidr(&r.cidr).map(|n| n.contains(ip)).unwrap_or(false))
            })
        } else {
            None
        };

        let mut picked = sticky_pick(&candidates, cfg.sticky_cookie, sticky);
        if picked.is_none() {
            picked = if let Some(rule) = internal {
                rule.nodes
                    .iter()
                    .find(|id| ui(id))
                    .cloned()
                    .or_else(|| {
                        // Fallback: strategy over all candidates.
                        None
                    })
            } else {
                None
            };
        }
        let picked = picked.or_else(|| {
            match cfg.parsed_strategy() {
                Some(Strategy::LeastInflight) => least_inflight(&candidates, &g.nodes),
                Some(Strategy::IpHash) => ip_hash(&candidates, visitor),
                _ => pick_rr(&candidates, &cfg.node_weights, &mut g.rr),
            }
        })?;

        let node = g.nodes.get(&picked)?;
        let sticky_val = if cfg.sticky_cookie {
            Some(node.node_id.clone())
        } else {
            None
        };
        let use_internal = internal.is_some() || cfg.parsed_mode() == Some(LbMode::InternalIp);
        Some(match cfg.parsed_mode() {
            Some(LbMode::Proxy) => LbTarget::Forward {
                addr: node.advertise.clone(),
                tls: cfg.tls_upstream,
                sni: host_of(&node.advertise),
            },
            _ => {
                let host = if use_internal {
                    node.internal_addr.as_deref().unwrap_or(&node.advertise)
                } else {
                    &node.advertise
                };
                LbTarget::Redirect {
                    location: format!("{}://{}{}", cfg.url_scheme, host, path_and_query),
                    sticky: sticky_val,
                }
            }
        })
    }
}

fn sticky_pick(candidates: &[String], sticky_on: bool, sticky: Option<&str>) -> Option<String> {
    if !sticky_on {
        return None;
    }
    let s = sticky.unwrap_or("").trim();
    if s.is_empty() {
        return None;
    }
    candidates.iter().find(|id| id.as_str() == s).cloned()
}

fn least_inflight(
    candidates: &[String],
    nodes: &HashMap<String, LbNode>,
) -> Option<String> {
    candidates
        .iter()
        .min_by_key(|id| nodes.get(*id).map(|n| n.load_inflight).unwrap_or(u64::MAX))
        .cloned()
}

fn ip_hash(candidates: &[String], visitor: Option<IpAddr>) -> Option<String> {
    let key = match visitor {
        Some(IpAddr::V4(a)) => u32::from(a) as u64,
        Some(IpAddr::V6(a)) => u128::from(a) as u64,
        None => {
            // No real IP available: fall back to a stable constant offset so
            // behavior stays deterministic for the same candidate set.
            0_u64
        }
    };
    let n = candidates.len();
    let idx = (key % n as u64) as usize;
    candidates.get(idx).cloned()
}

/// Weighted round-robin over candidates in stable (sorted) order.
fn pick_rr(
    candidates: &[String],
    weights: &HashMap<String, u32>,
    rr: &mut usize,
) -> Option<String> {
    let total: u64 = candidates
        .iter()
        .map(|id| weights.get(id).copied().filter(|w| *w > 0).unwrap_or(1) as u64)
        .sum();
    if total == 0 {
        return None;
    }
    let mut step = *rr % total as usize;
    for id in candidates {
        let w = weights.get(id).copied().filter(|w| *w > 0).unwrap_or(1) as usize;
        if step < w {
            *rr = rr.wrapping_add(1);
            return Some(id.clone());
        }
        step -= w;
    }
    // Unreachable when total == sum of weights; keep compiler honest.
    let first = candidates.first().cloned();
    *rr = rr.wrapping_add(1);
    first
}

/// Host part of a `host:port` (or bare `host`) string, IPv6 aware.
pub fn host_of(addr: &str) -> String {
    if let Some((h, _)) = addr.rsplit_once(':') {
        h.trim_start_matches('[').trim_end_matches(']').to_string()
    } else {
        addr.to_string()
    }
}

/// Minimal CIDR matcher (no external dep): `ip/prefix`.
pub fn parse_cidr(s: &str) -> Option<IpNet> {
    let (net, bits) = s.trim().split_once('/')?;
    let base = net.trim().parse::<IpAddr>().ok()?;
    let prefix: u8 = bits.trim().parse().ok()?;
    let max = match base {
        IpAddr::V4(_) => 32,
        IpAddr::V6(_) => 128,
    };
    if prefix > max {
        return None;
    }
    Some(IpNet { base, prefix })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpNet {
    pub base: IpAddr,
    pub prefix: u8,
}

impl IpNet {
    pub fn contains(&self, ip: IpAddr) -> bool {
        match (ip, self.base) {
            (IpAddr::V4(a), IpAddr::V4(b)) => {
                let mask = if self.prefix == 0 {
                    0u32
                } else {
                    u32::MAX << (32 - self.prefix as u32)
                };
                (u32::from(a) & mask) == (u32::from(b) & mask)
            }
            (IpAddr::V6(a), IpAddr::V6(b)) => {
                let mask = if self.prefix == 0 {
                    0u128
                } else {
                    u128::MAX << (128 - self.prefix as u128)
                };
                (u128::from(a) & mask) == (u128::from(b) & mask)
            }
            _ => false,
        }
    }
}

pub mod health;
pub use health::spawn_active_probe;

#[cfg(test)]
mod tests {
    use super::*;

    fn pool_of(cfg: LbConfig, nodes: Vec<LbNode>) -> LbPool {
        let p = LbPool::new();
        p.set_config(cfg);
        p.set_nodes(nodes);
        p
    }

    fn node(id: &str, advertise: &str) -> LbNode {
        LbNode {
            node_id: id.into(),
            advertise: advertise.into(),
            internal_addr: None,
            load_inflight: 0,
            degraded: false,
        }
    }

    #[test]
    fn cidr_match() {
        let n = parse_cidr("10.0.0.0/8").unwrap();
        assert!(n.contains("10.1.2.3".parse().unwrap()));
        assert!(!n.contains("11.0.0.1".parse().unwrap()));
        let n6 = parse_cidr("fd00::/8").unwrap();
        assert!(n6.contains("fd12:3456::1".parse().unwrap()));
        assert!(!n6.contains("fe00::1".parse().unwrap()));
        assert!(parse_cidr("10.0.0.1/33").is_none());
        assert!(parse_cidr("nope").is_none());
    }

    #[test]
    fn weight_rr_distributes() {
        let cfg = LbConfig {
            enabled: true,
            mode: MODE_REDIRECT.into(),
            node_weights: {
                let mut m = HashMap::new();
                m.insert("a".into(), 2);
                m.insert("b".into(), 1);
                m
            },
            ..Default::default()
        };
        let p = pool_of(cfg, vec![node("a", "10.0.0.1:28765"), node("b", "10.0.0.2:28765")]);
        let mut seq = Vec::new();
        for _ in 0..6 {
            let t = p.select(None, None, "/x").unwrap();
            match t {
                LbTarget::Redirect { location, sticky } => {
                    assert!(location.ends_with("/x"));
                    let host = location.split("://").nth(1).unwrap().split('/').next().unwrap();
                    seq.push(if host.starts_with("10.0.0.1") { "a" } else { "b" });
                    assert_eq!(sticky, None); // sticky_cookie off
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        // 6 picks with a=2,b=1 → pattern aab repeated twice (order stable).
        assert_eq!(seq, vec!["a", "a", "b", "a", "a", "b"]);
    }

    #[test]
    fn sticky_pins_node() {
        let cfg = LbConfig {
            enabled: true,
            mode: MODE_REDIRECT.into(),
            sticky_cookie: true,
            ..Default::default()
        };
        let p = pool_of(cfg, vec![node("a", "10.0.0.1:28765"), node("b", "10.0.0.2:28765")]);
        for _ in 0..5 {
            let t = p.select(None, Some("b"), "/y").unwrap();
            match t {
                LbTarget::Redirect { location, sticky } => {
                    assert!(location.contains("10.0.0.2"));
                    assert_eq!(sticky.as_deref(), Some("b"));
                }
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    #[test]
    fn degraded_and_unhealthy_excluded() {
        let cfg = LbConfig {
            enabled: true,
            mode: MODE_REDIRECT.into(),
            active_health_check: true,
            health_check_fail_threshold: 2,
            ..Default::default()
        };
        let mut a = node("a", "10.0.0.1:28765");
        a.degraded = true;
        let mut b = node("b", "10.0.0.2:28765");
        b.internal_addr = Some("192.168.1.2:28765".into());
        let p = pool_of(cfg, vec![a, b.clone()]);
        // a degraded → b always picked.
        for _ in 0..4 {
            let t = p.select(None, None, "/").unwrap();
            match t {
                LbTarget::Redirect { location, .. } => assert!(location.contains("10.0.0.2")),
                other => panic!("unexpected {other:?}"),
            }
        }
        // b fails probe twice → no candidates → None.
        p.report_health("b", false);
        p.report_health("b", false);
        assert_eq!(p.select(None, None, "/"), None);
        // One success recovers instantly.
        p.report_health("b", true);
        assert!(p.select(None, None, "/").is_some());
    }

    #[test]
    fn proxy_forward_target() {
        let cfg = LbConfig {
            enabled: true,
            mode: MODE_PROXY.into(),
            tls_upstream: true,
            ..Default::default()
        };
        let p = pool_of(cfg, vec![node("a", "10.0.0.5:28443")]);
        let t = p.select(None, None, "/v1/session/open").unwrap();
        assert_eq!(
            t,
            LbTarget::Forward {
                addr: "10.0.0.5:28443".into(),
                tls: true,
                sni: "10.0.0.5".into(),
            }
        );
    }

    #[test]
    fn internal_ip_prefers_rule_nodes() {
        let cfg = LbConfig {
            enabled: true,
            mode: MODE_INTERNAL_IP.into(),
            internal_cidrs: vec![CidrRule {
                cidr: "192.168.0.0/16".into(),
                nodes: vec!["b".into()],
            }],
            ..Default::default()
        };
        let mut b = node("b", "10.0.0.2:28765");
        b.internal_addr = Some("192.168.1.2:28765".into());
        let p = pool_of(cfg, vec![node("a", "10.0.0.1:28765"), b]);
        // LAN visitor → b internal.
        let t = p
            .select(Some("192.168.9.9".parse().unwrap()), None, "/")
            .unwrap();
        match t {
            LbTarget::Redirect { location, .. } => assert!(location.contains("192.168.1.2")),
            other => panic!("unexpected {other:?}"),
        }
        // Internet visitor → strategy over all (a first by id sort, rr starts at a).
        let t = p
            .select(Some("8.8.8.8".parse().unwrap()), None, "/")
            .unwrap();
        match t {
            LbTarget::Redirect { location, .. } => assert!(location.contains("10.0.0.1")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn least_inflight_picks_min() {
        let cfg = LbConfig {
            enabled: true,
            mode: MODE_PROXY.into(),
            strategy: Strategy::LeastInflight.as_str().into(),
            ..Default::default()
        };
        let mut a = node("a", "10.0.0.1:28765");
        a.load_inflight = 10;
        let mut b = node("b", "10.0.0.2:28765");
        b.load_inflight = 2;
        let p = pool_of(cfg, vec![a, b]);
        for _ in 0..3 {
            let t = p.select(None, None, "/").unwrap();
            assert!(matches!(t, LbTarget::Forward { addr, .. } if addr.contains("10.0.0.2")));
        }
    }

    #[test]
    fn disabled_returns_none() {
        let p = LbPool::new();
        p.set_nodes(vec![node("a", "10.0.0.1:28765")]);
        assert_eq!(p.select(None, None, "/"), None);
    }

    #[test]
    fn config_validate() {
        let c = LbConfig::default();
        assert!(c.validate().is_ok());
        assert!(LbConfig {
            mode: "bogus".into(),
            ..c.clone()
        }
        .validate()
        .is_err());
        assert!(LbConfig {
            internal_cidrs: vec![CidrRule {
                cidr: "bad".into(),
                nodes: vec![],
            }],
            ..c
        }
        .validate()
        .is_err());
    }
}
