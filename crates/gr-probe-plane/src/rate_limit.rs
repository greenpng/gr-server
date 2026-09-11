//! Shared rate limit: open / ingest / analyze / complete / result (per site
//! + route) and client_event (per IP **and** optional per-site total).
//!
//! Multi-node: PostgreSQL `probe_rate_limit_windows` is the shared authority —
//! every node bumps the same per-minute counter, so the quota is cluster-wide.
//! SQLite / store outage falls back to a process-local window (single-node mode
//! or degraded best-effort; a DB blip must not turn into a site-wide 429).
//! Lab (`GR_REQUIRE_RESULT_TOKEN=0`) is open unless `GR_RATE_LIMIT_FORCE=1`.
//!
//! v1.0.14 policy: site-wide totals default **0 (unlimited)** — a blunt
//! aggregate cap throttles real users mixed into bot floods. The telemetry
//! route (`client_event`) is instead bounded **per IP** (default 100/min):
//! a single misbehaving source is capped without affecting anyone else.
//! Both layers stay panel-hot (`/api/config` → publish).

use crate::handlers::AppState;
use dashmap::DashMap;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

struct Bucket {
    window_ms: AtomicU64,
    count: AtomicU64,
}

static BUCKETS: OnceLock<DashMap<String, Bucket>> = OnceLock::new();

/// Ops counters: checks performed while the limiter is enabled, and rejections.
static CHECKED: AtomicU64 = AtomicU64::new(0);
static REJECTED: AtomicU64 = AtomicU64::new(0);

fn buckets() -> &'static DashMap<String, Bucket> {
    BUCKETS.get_or_init(DashMap::new)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn now_window() -> i64 {
    (now_ms() / 60_000) as i64
}

fn limit_for(route: &str) -> u64 {
    // Panel hot config wins once published (config_version > 0); before any
    // publish the env/builtin seed keeps the legacy env-only behavior.
    let env_key = match route {
        "open" => "RATE_LIMIT_OPEN_PER_MIN",
        "ingest" => "RATE_LIMIT_INGEST_PER_MIN",
        "analyze" => "RATE_LIMIT_ANALYZE_PER_MIN",
        "complete" => "RATE_LIMIT_COMPLETE_PER_MIN",
        "result" => "RATE_LIMIT_RESULT_PER_MIN",
        "client_event" => "RATE_LIMIT_CLIENT_EVENT_PER_MIN",
        _ => "RATE_LIMIT_DEFAULT_PER_MIN",
    };
    let nonneg = |v: i64| if v >= 0 { Some(v as u64) } else { None };
    if gr_probe_store::config_version() > 0 {
        let c = gr_probe_store::get_runtime_cfg();
        let from_cfg = match route {
            "open" => nonneg(c.rate_limit_open_per_min),
            "ingest" => nonneg(c.rate_limit_ingest_per_min),
            "analyze" => nonneg(c.rate_limit_analyze_per_min),
            "complete" => nonneg(c.rate_limit_complete_per_min),
            "result" => nonneg(c.rate_limit_result_per_min),
            "client_event" => nonneg(c.rate_limit_client_event_per_min),
            _ => None,
        };
        if let Some(v) = from_cfg {
            return v;
        }
    }
    gr_abi::env::get(env_key)
        .and_then(|s| s.parse().ok())
        .unwrap_or(match route {
            // v1.0.14: site totals default unlimited (see module doc). The
            // per-IP telemetry cap below is the always-on protection layer.
            "open" => 0,
            "ingest" => 0,
            "analyze" => 0,
            "complete" => 0,
            "result" => 0,
            "client_event" => 0,
            _ => 0,
        })
}

/// Per-IP cap for the client_event telemetry route (panel hot; default 100).
fn limit_client_event_per_ip() -> u64 {
    if gr_probe_store::config_version() > 0 {
        let c = gr_probe_store::get_runtime_cfg();
        if c.rate_limit_client_event_per_ip_per_min >= 0 {
            return c.rate_limit_client_event_per_ip_per_min as u64;
        }
    }
    gr_abi::env::get("RATE_LIMIT_CLIENT_EVENT_PER_IP_PER_MIN")
        .and_then(|s| s.parse().ok())
        .unwrap_or(100)
}

fn enabled() -> bool {
    if gr_abi::env::get("RATE_LIMIT_FORCE").as_deref() == Some("1") {
        return true;
    }
    if gr_abi::env::get("RATE_LIMIT_OFF").as_deref() == Some("1") {
        return false;
    }
    gr_probe_core::result_token_enforced()
}

fn rl_err(route: &str) -> String {
    format!("rate_limited:{route}")
}

/// Process-local window bucket (single-node SQLite, or fallback when the shared
/// store is unreachable). Not authoritative across processes.
fn check_local(key: &str, cap: u64, route_tag: &str) -> Result<(), String> {
    let now = now_ms();
    let window = now / 60_000;
    let b = buckets().entry(key.to_string()).or_insert_with(|| Bucket {
        window_ms: AtomicU64::new(window),
        count: AtomicU64::new(0),
    });
    let prev_w = b.window_ms.load(Ordering::Relaxed);
    if prev_w != window {
        b.window_ms.store(window, Ordering::Relaxed);
        b.count.store(0, Ordering::Relaxed);
    }
    let n = b.count.fetch_add(1, Ordering::Relaxed) + 1;
    if n > cap {
        return reject(route_tag);
    }
    Ok(())
}

fn reject(route: &str) -> Result<(), String> {
    REJECTED.fetch_add(1, Ordering::Relaxed);
    Err(rl_err(route))
}

/// Bump one shared window and enforce its cap. PG is authoritative; any store
/// error degrades to the process-local window instead of failing closed.
fn check_key(st: &AppState, key: &str, cap: u64, route_tag: &str) -> Result<(), String> {
    if cap == 0 {
        return Ok(());
    }
    if st.store.backend_name() == "postgres" {
        match st.store.bump_rate_limit_window(key, now_window()) {
            Ok(n) => {
                if n > cap as i64 {
                    return reject(route_tag);
                }
                Ok(())
            }
            Err(_) => check_local(key, cap, route_tag),
        }
    } else {
        check_local(key, cap, route_tag)
    }
}

/// Cluster-wide check for the site-scoped routes (open/ingest/analyze/
/// complete/result). PG backend bumps the shared per-minute counter; any store
/// error degrades to the local window rather than failing closed.
pub fn check(st: &AppState, site: &str, route: &str) -> Result<(), String> {
    if !enabled() {
        return Ok(());
    }
    CHECKED.fetch_add(1, Ordering::Relaxed);
    let cap = limit_for(route);
    if cap == 0 {
        return Ok(());
    }
    let key = format!("{}:{}", site, route);
    check_key(st, &key, cap, route)
}

/// Cluster-wide check for the FE ops telemetry route: **per-IP window first**
/// (default 100/min — caps a single flooding source without touching anyone
/// else), then the optional site-wide total (default 0 = unlimited). Rejection
/// names the layer that tripped (`client_event:ip` vs `client_event:site`) so
/// ops can tell a bot cap from a total cap in the aggregated 4xx window log.
pub fn check_client_event(st: &AppState, site: &str, ip: &str) -> Result<(), String> {
    if !enabled() {
        return Ok(());
    }
    CHECKED.fetch_add(1, Ordering::Relaxed);
    let per_ip = limit_client_event_per_ip();
    if per_ip > 0 {
        let key = format!("ce:{}", ip);
        check_key(st, &key, per_ip, "client_event:ip")?;
    }
    let total = limit_for("client_event");
    if total > 0 {
        let site_key = if site.is_empty() { "_" } else { site };
        let key = format!("{}:client_event", site_key);
        check_key(st, &key, total, "client_event:site")?;
    }
    Ok(())
}

/// Ops view of the limiter: enabled state, per-route caps, counters, local bucket
/// count. Safe to expose to ops scope (no bucket keys).
pub fn stats() -> Value {
    let mut caps = serde_json::Map::new();
    for r in [
        "open",
        "ingest",
        "analyze",
        "complete",
        "result",
        "client_event",
    ] {
        caps.insert(r.into(), json!(limit_for(r)));
    }
    let cfg_version = gr_probe_store::config_version();
    json!({
        "enabled": enabled(),
        "force": gr_abi::env::get("RATE_LIMIT_FORCE").as_deref() == Some("1"),
        "shared_backend": "postgres_probe_rate_limit_windows",
        "caps_per_min": Value::Object(caps),
        // v1.0.14: the per-IP telemetry layer (default 100; 0 = off).
        "caps_per_ip_per_min": { "client_event": limit_client_event_per_ip() },
        "caps_source": if cfg_version > 0 { "panel_runtime_cfg" } else { "env_or_builtin" },
        "config_version": cfg_version,
        "checked": CHECKED.load(Ordering::Relaxed),
        "rejected": REJECTED.load(Ordering::Relaxed),
        "local_bucket_count": buckets().len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gr_probe_store::{set_runtime_cfg, RuntimeCfg};

    /// Panel runtime cfg + the v1.0.14 rate-limit policy, in one test: the
    /// tests share process-global cfg state, so they must not run in parallel.
    ///
    /// 1.0.10: once the panel publishes (config_version > 0) the runtime cfg
    /// caps win over env/builtin; stats exposes the source.
    /// v1.0.14: site totals default 0 (unlimited); the client_event per-IP
    /// layer defaults 100, is panel-hot, and can be disabled independently.
    #[test]
    fn panel_caps_win_and_v1014_per_ip_policy() {
        let mut c = RuntimeCfg::default();
        c.version = 1;
        c.rate_limit_open_per_min = 7;
        c.rate_limit_ingest_per_min = 9;
        set_runtime_cfg(c.clone());
        assert_eq!(limit_for("open"), 7);
        assert_eq!(limit_for("ingest"), 9);
        // Untouched routes keep their defaults through the panel cfg object —
        // v1.0.14: that default is 0 (unlimited) for every site total.
        assert_eq!(limit_for("result"), 0);
        assert_eq!(limit_for("client_event"), 0);
        let s = stats();
        assert_eq!(s["caps_source"], serde_json::json!("panel_runtime_cfg"));
        assert_eq!(s["caps_per_min"]["open"], serde_json::json!(7));
        assert_eq!(s["caps_per_min"]["client_event"], serde_json::json!(0));

        // v1.0.14: per-IP telemetry layer is panel-hot alongside the totals.
        c.rate_limit_client_event_per_ip_per_min = 250;
        c.rate_limit_client_event_per_min = 5_000;
        set_runtime_cfg(c.clone());
        assert_eq!(limit_client_event_per_ip(), 250);
        assert_eq!(limit_for("client_event"), 5_000);
        let s = stats();
        assert_eq!(
            s["caps_per_ip_per_min"]["client_event"],
            serde_json::json!(250)
        );
        // Per-IP layer can be disabled independently (0 = off).
        c.rate_limit_client_event_per_ip_per_min = 0;
        set_runtime_cfg(c.clone());
        assert_eq!(limit_client_event_per_ip(), 0);

        // Restore process state (also pins the unpublished builtin defaults).
        set_runtime_cfg(RuntimeCfg::default());
        assert_eq!(gr_probe_store::config_version(), 0);
        assert_eq!(limit_for("open"), 0);
        assert_eq!(limit_for("client_event"), 0);
        assert_eq!(limit_client_event_per_ip(), 100);
    }
}
