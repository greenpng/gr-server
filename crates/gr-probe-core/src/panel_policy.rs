//! Shared panel policy file (control-plane writes, probe-plane reads).
//!
//! Path resolution order:
//! 1. `GR_PANEL_POLICY_PATH` / `GR_PANEL_POLICY_PATH`
//! 2. `{data_dir}/panel_policy.json`
//! 3. `{cwd}/tests/data/lab-domain/panel_policy.json` (lab fallback; `data/lab-domain` symlink)
//!
//! Policy is **management config only** — never rewrites mint digests.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

static CACHE: RwLock<Option<(u64, PanelPolicy)>> = RwLock::new(None);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyPolicy {
    #[serde(default = "default_strategy_id")]
    pub default_strategy_id: String,
    #[serde(default)]
    pub site_strategies: serde_json::Map<String, Value>,
}

fn default_strategy_id() -> String {
    "balanced".into()
}

impl Default for StrategyPolicy {
    fn default() -> Self {
        Self {
            default_strategy_id: default_strategy_id(),
            site_strategies: serde_json::Map::new(),
        }
    }
}

/// Result shaping is a customer-controlled policy, independent of legacy
/// plan/entitlement metadata. Analysis remains full-fidelity internally; this
/// policy controls what the public projection and browser collection expose.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultPolicy {
    #[serde(default = "default_true")]
    pub rpa_collect: bool,
    #[serde(default = "default_primary_device_lane")]
    pub primary_device_lane: String,
    #[serde(default = "default_response_profile")]
    pub response_profile: String,
    #[serde(default = "default_true")]
    pub include_signals: bool,
    /// Optional per-site overrides keyed by site_id. Values are deliberately
    /// opaque here so older policy files remain forward compatible.
    #[serde(default)]
    pub site_overrides: serde_json::Map<String, Value>,
}

fn default_primary_device_lane() -> String {
    "dv0".into()
}

fn default_response_profile() -> String {
    "standard".into()
}

impl Default for ResultPolicy {
    fn default() -> Self {
        Self {
            rpa_collect: true,
            primary_device_lane: default_primary_device_lane(),
            response_profile: default_response_profile(),
            include_signals: true,
            site_overrides: serde_json::Map::new(),
        }
    }
}

impl ResultPolicy {
    fn apply_patch(&mut self, patch: &Value) {
        let Some(obj) = patch.as_object() else {
            return;
        };
        if let Some(v) = obj.get("rpa_collect").and_then(|v| v.as_bool()) {
            self.rpa_collect = v;
        }
        if let Some(v) = obj
            .get("primary_device_lane")
            .and_then(|v| v.as_str())
            .filter(|v| matches!(*v, "dv0" | "dv4" | "dv5" | "dv6"))
        {
            self.primary_device_lane = v.to_string();
        }
        if let Some(v) = obj
            .get("response_profile")
            .and_then(|v| v.as_str())
            .filter(|v| matches!(*v, "basic" | "standard" | "advanced"))
        {
            self.response_profile = v.to_string();
        }
        if let Some(v) = obj.get("include_signals").and_then(|v| v.as_bool()) {
            self.include_signals = v;
        }
    }

    /// Resolve defaults plus an optional site-specific patch.
    pub fn for_site(&self, site_id: Option<&str>) -> Self {
        let mut out = self.clone();
        if let Some(sid) = site_id.map(str::trim).filter(|s| !s.is_empty()) {
            if let Some(patch) = self.site_overrides.get(sid) {
                out.apply_patch(patch);
            }
        }
        out.site_overrides.clear();
        out
    }
}

/// Data lifecycle for commercial multi-tenant stores.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionPolicy {
    /// Master switch for background purge.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// How long to keep analysis_results / analysis_latest rows.
    #[serde(default = "default_analysis_days")]
    pub analysis_retention_days: u32,
    /// How long to keep idle sessions and their batches (after analysis TTL).
    #[serde(default = "default_session_days")]
    pub session_retention_days: u32,
    /// L3 cold archive TTL (aligns with GR_COLD_TTL_MS when set).
    #[serde(default = "default_cold_days")]
    pub cold_ttl_days: u32,
    /// Max rows deleted per batch (avoids spike load).
    #[serde(default = "default_batch")]
    pub batch_delete_limit: u32,
    /// Background purge tick interval seconds.
    #[serde(default = "default_interval")]
    pub purge_interval_sec: u32,
    /// Velocity hit retention days.
    #[serde(default = "default_velocity_days")]
    pub velocity_retention_days: u32,
    /// iss/opus5 04-P1-6: ops event streams + observation_events +
    /// api_idempotency retention days (previously unbounded growth).
    #[serde(default = "default_ops_days")]
    pub ops_retention_days: u32,
    /// iss/opus5 04-P1-6: commercial master tables (devices, device_sessions,
    /// device_index_*, soft_edges, soft_heat) retention days.
    #[serde(default = "default_master_days")]
    pub master_retention_days: u32,
}

fn default_true() -> bool {
    true
}
fn default_analysis_days() -> u32 {
    30
}
fn default_session_days() -> u32 {
    30
}
fn default_cold_days() -> u32 {
    7
}
fn default_batch() -> u32 {
    200
}
fn default_interval() -> u32 {
    300
}
fn default_velocity_days() -> u32 {
    7
}
fn default_ops_days() -> u32 {
    14
}
fn default_master_days() -> u32 {
    90
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            analysis_retention_days: default_analysis_days(),
            session_retention_days: default_session_days(),
            cold_ttl_days: default_cold_days(),
            batch_delete_limit: default_batch(),
            purge_interval_sec: default_interval(),
            velocity_retention_days: default_velocity_days(),
            ops_retention_days: default_ops_days(),
            master_retention_days: default_master_days(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PanelPolicy {
    #[serde(default)]
    pub strategy: StrategyPolicy,
    #[serde(default)]
    pub result_policy: ResultPolicy,
    #[serde(default)]
    pub retention: RetentionPolicy,
    /// Opaque integrations blob (IP providers etc.) — control plane owns schema.
    #[serde(default)]
    pub integrations: Value,
    /// Legacy per-site plan metadata; it no longer gates device lanes or RPA.
    #[serde(default)]
    pub site_entitlements: serde_json::Map<String, Value>,
    #[serde(default)]
    pub updated_ms: i64,
}

impl PanelPolicy {
    pub fn resolve_strategy_id(&self, site_id: Option<&str>) -> String {
        if let Some(sid) = site_id.map(str::trim).filter(|s| !s.is_empty()) {
            if let Some(v) = self.strategy.site_strategies.get(sid) {
                if let Some(s) = v.as_str().filter(|s| !s.is_empty()) {
                    return s.to_string();
                }
            }
        }
        let d = self.strategy.default_strategy_id.trim();
        if d.is_empty() {
            "balanced".into()
        } else {
            d.to_string()
        }
    }

    pub fn resolve_result_policy(&self, site_id: Option<&str>) -> ResultPolicy {
        self.result_policy.for_site(site_id)
    }

    pub fn resolve_entitlement(&self, site_id: Option<&str>) -> crate::plan_entitlement::PlanEntitlement {
        let _ = (self, site_id);
        // Stored plan/expiry/revocation fields are legacy metadata only.
        crate::plan_entitlement::PlanEntitlement::full()
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Resolve policy file path from optional data_dir.
pub fn policy_path(data_dir: Option<&Path>) -> PathBuf {
    if let Some(p) = gr_abi::env::get("PANEL_POLICY_PATH")
         .or_else(|| gr_abi::env::get("PANEL_POLICY_PATH"))
    {
        let t = p.trim();
        if !t.is_empty() {
            return PathBuf::from(t);
        }
    }
    if let Some(d) = data_dir {
        return d.join("panel_policy.json");
    }
    // Lab fallback (T2: tests/data first, data/lab-domain symlink still works)
    for lab in [
        PathBuf::from("tests/data/lab-domain/panel_policy.json"),
        PathBuf::from("data/lab-domain/panel_policy.json"),
    ] {
        if lab.exists() {
            return lab;
        }
    }
    PathBuf::from("panel_policy.json")
}

pub fn load_from_path(path: &Path) -> PanelPolicy {
    match fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => PanelPolicy::default(),
    }
}

/// Cached load (mtime via file length+updated_ms heuristic; re-read every call if uncached).
pub fn load_cached(data_dir: Option<&Path>) -> PanelPolicy {
    let path = policy_path(data_dir);
    let meta_ms = fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(guard) = CACHE.read() {
        if let Some((ms, ref pol)) = *guard {
            if ms == meta_ms {
                return pol.clone();
            }
        }
    }
    let pol = load_from_path(&path);
    if let Ok(mut w) = CACHE.write() {
        *w = Some((meta_ms, pol.clone()));
    }
    pol
}

pub fn save_to_path(path: &Path, mut policy: PanelPolicy) -> Result<(), String> {
    policy.updated_ms = now_ms();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let s = serde_json::to_string_pretty(&policy).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, s.as_bytes()).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    if let Ok(mut w) = CACHE.write() {
        *w = None; // invalidate
    }
    Ok(())
}

pub fn save(data_dir: Option<&Path>, policy: PanelPolicy) -> Result<PathBuf, String> {
    let path = policy_path(data_dir);
    save_to_path(&path, policy)?;
    Ok(path)
}

pub fn merge_strategy(mut base: PanelPolicy, default_id: &str, site_map: &Value) -> PanelPolicy {
    if !default_id.trim().is_empty() {
        base.strategy.default_strategy_id = default_id.trim().to_string();
    }
    if let Some(obj) = site_map.as_object() {
        base.strategy.site_strategies = obj.clone();
    }
    base
}

pub fn merge_result_policy(mut base: PanelPolicy, patch: &Value) -> PanelPolicy {
    base.result_policy.apply_patch(patch);
    if let Some(overrides) = patch.get("site_overrides").and_then(|v| v.as_object()) {
        base.result_policy.site_overrides = overrides.clone();
    }
    base
}

pub fn merge_entitlements(mut base: PanelPolicy, site_map: &Value) -> PanelPolicy {
    if let Some(obj) = site_map.as_object() {
        for (k, v) in obj {
            base.site_entitlements.insert(k.clone(), v.clone());
        }
    }
    base
}

pub fn merge_retention(mut base: PanelPolicy, patch: &Value) -> PanelPolicy {
    if let Some(obj) = patch.as_object() {
        let r = &mut base.retention;
        if let Some(v) = obj.get("enabled").and_then(|x| x.as_bool()) {
            r.enabled = v;
        }
        if let Some(v) = obj.get("analysis_retention_days").and_then(|x| x.as_u64()) {
            r.analysis_retention_days = v.clamp(1, 3650) as u32;
        }
        if let Some(v) = obj.get("session_retention_days").and_then(|x| x.as_u64()) {
            r.session_retention_days = v.clamp(1, 3650) as u32;
        }
        if let Some(v) = obj.get("cold_ttl_days").and_then(|x| x.as_u64()) {
            r.cold_ttl_days = v.clamp(1, 365) as u32;
        }
        if let Some(v) = obj.get("batch_delete_limit").and_then(|x| x.as_u64()) {
            r.batch_delete_limit = v.clamp(10, 5000) as u32;
        }
        if let Some(v) = obj.get("purge_interval_sec").and_then(|x| x.as_u64()) {
            r.purge_interval_sec = v.clamp(30, 86400) as u32;
        }
        if let Some(v) = obj.get("velocity_retention_days").and_then(|x| x.as_u64()) {
            r.velocity_retention_days = v.clamp(1, 90) as u32;
        }
    }
    // Always clamp (also covers full-object patches deserialized elsewhere).
    {
        let r = &mut base.retention;
        r.analysis_retention_days = r.analysis_retention_days.clamp(1, 3650);
        r.session_retention_days = r.session_retention_days.clamp(1, 3650);
        r.cold_ttl_days = r.cold_ttl_days.clamp(1, 365);
        r.batch_delete_limit = r.batch_delete_limit.clamp(10, 5000);
        r.purge_interval_sec = r.purge_interval_sec.clamp(30, 86400);
        r.velocity_retention_days = r.velocity_retention_days.clamp(1, 90);
    }
    base
}

pub fn policy_public_view(p: &PanelPolicy) -> Value {
    json!({
        "strategy": {
            "default_strategy_id": p.strategy.default_strategy_id,
            "site_strategies": p.strategy.site_strategies,
        },
        "result_policy": p.result_policy,
        "retention": p.retention,
        "integrations": p.integrations,
        "updated_ms": p.updated_ms,
        "storage_note": {
            "analysis_encoding": "Default z1:deflate+base64. Optional AES-256-GCM: set GR_ANALYSIS_AT_REST_KEY (64-hex or base64 of 32 bytes) → e1: prefix.",
            "cold_archive": "probe_cold compact+deflate; TTL via retention.cold_ttl_days / GR_COLD_TTL_MS",
            "purge": "batched deletes (batch_delete_limit, purge_interval_sec); never full-table lock storms",
            "pg_performance": "analysis_latest denorm scalars + BRIN/BTREE on created_ms/site_id/device_id; avoid TOAST scans for dashboards",
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_site_override() {
        let mut p = PanelPolicy::default();
        p.strategy.default_strategy_id = "balanced".into();
        p.strategy
            .site_strategies
            .insert("shop".into(), json!("bot_control"));
        assert_eq!(p.resolve_strategy_id(Some("shop")), "bot_control");
        assert_eq!(p.resolve_strategy_id(Some("news")), "balanced");
        assert_eq!(p.resolve_strategy_id(None), "balanced");
    }

    #[test]
    fn result_policy_resolves_valid_site_override() {
        let mut p = PanelPolicy::default();
        p.result_policy.rpa_collect = false;
        p.result_policy.site_overrides.insert(
            "shop".into(),
            json!({
                "rpa_collect": true,
                "primary_device_lane": "dv5",
                "response_profile": "advanced",
                "include_signals": false
            }),
        );
        let shop = p.resolve_result_policy(Some("shop"));
        assert!(shop.rpa_collect);
        assert_eq!(shop.primary_device_lane, "dv5");
        assert_eq!(shop.response_profile, "advanced");
        assert!(!shop.include_signals);
        assert!(!p.resolve_result_policy(Some("news")).rpa_collect);
    }

    #[test]
    fn retention_clamps() {
        let mut p = PanelPolicy::default();
        p = merge_retention(
            p,
            &json!({"analysis_retention_days": 99999, "batch_delete_limit": 1}),
        );
        assert_eq!(p.retention.analysis_retention_days, 3650);
        assert_eq!(p.retention.batch_delete_limit, 10);
    }

    #[test]
    fn legacy_expiry_does_not_reduce_surface() {
        let mut p = PanelPolicy::default();
        p.site_entitlements.insert(
            "site_paid".into(),
            json!({"plan": "paid", "expires_at_ms": 1}),
        );
        p.site_entitlements.insert(
            "site_live".into(),
            json!({"plan": "paid", "expires_at_ms": now_ms() + 86_400_000}),
        );
        // 2026-09 unified tier: every site resolves full-featured regardless
        // of the stored plan string or expiry (free == paid capability set).
        for sid in ["site_paid", "site_live"] {
            let ent = p.resolve_entitlement(Some(sid));
            assert!(ent.rpa_enabled, "unified tier keeps RPA on for {sid}");
            assert!(ent.allows_precision("dv0"), "unified tier keeps dv0 for {sid}");
        }
    }
}
