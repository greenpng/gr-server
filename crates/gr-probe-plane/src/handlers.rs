//! green-v5 HTTP handlers — framework-agnostic (Pingora dispatches).
//! No axum; request/response are plain types.

use gr_probe_core::evaluate::evaluate_session;
use gr_probe_core::{
    active_confidence_version, adopt_decision_report, aggregate_identity_sla,
    aggregate_unknown_buckets, analyze_arm_debounce_ms_after_ingest, analyze_schedule_policy_json,
    probe_field_priority_json,
    calibrate_offline, commercial_id_heat_report, hub_promotion_drafts, new_batch_analyze_clocks,
    next_rotation, pairs_from_json, pairs_from_matrix_cells, rows_from_json_array,
    self_capability_json, SlaSessionRow, SoftEdge, SoftEdgeStore, ANALYZE_IDLE_UPLOAD_MS,
    DEFAULT_CHALLENGE_SECRET, PROMOTE_TO_COMMERCIAL_ID, RUNTIME_CONFIDENCE_VERSION,
};
use gr_probe_store::{
    analyze_claim_batch, analyze_idle_poll_ms_range, cold_promote_window_ms, cold_ttl_ms,
    hot_idle_ms, timeout_matrix_json, AnalyzeDueMerge, HotProbeCache, HotProbeEntry, Store,
    StoreError, ANALYZE_DEBOUNCE_MS, ANALYZE_LOCK_MS, ANALYZE_IDLE_IMMINENT_MS,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::admin::AdminHub;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServiceRole {
    /// Ingest + gateway + static + analyze worker (default single-node)
    All,
    /// HTTP open/ingest/static only
    Ingest,
    /// HTTP gateway early only
    Gateway,
    /// Background analyze/brain consumers only (no public bind required)
    Analyze,
}

impl ServiceRole {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "ingest" => Self::Ingest,
            "gateway" | "gw" => Self::Gateway,
            "analyze" | "brain" | "worker" => Self::Analyze,
            _ => Self::All,
        }
    }
    pub fn serves_ingest(&self) -> bool {
        matches!(self, Self::All | Self::Ingest)
    }
    pub fn serves_gateway(&self) -> bool {
        matches!(self, Self::All | Self::Gateway)
    }
    pub fn runs_analyze_workers(&self) -> bool {
        matches!(self, Self::All | Self::Analyze)
    }
    pub fn needs_listen(&self) -> bool {
        !matches!(self, Self::Analyze)
    }
}

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Store>,
    /// Soft edges: file | store(SQLite/PG) | redis — always promote=false.
    pub soft_store: Arc<dyn SoftEdgeStore>,
    /// Soft backend label for ops (file/store/redis).
    pub soft_store_backend: String,
    /// R100 authenticity pack templates (memory + optional Redis pseudo-static).
    pub r100: Arc<crate::r100_hub::R100Hub>,
    /// Server Soft gate (CLI `--soft-v2-ready` / env). Never trust client body (A-BRAIN-3).
    pub soft_v2_ready: bool,
    /// HMAC secret for H16 challenge seeds (GR_CHALLENGE_SECRET; lab default only if allowed).
    pub challenge_secret: String,
    /// Probe seal secret (GR_SEAL_SECRET; defaults to challenge_secret).
    pub seal_secret: String,
    pub role: ServiceRole,
    pub worker_id: String,
    pub analyze_runs: Arc<AtomicU64>,
    /// Desired analyze worker count (admin hot target).
    pub analyze_workers_target: Arc<AtomicUsize>,
    /// Built-in admin console (dedicated SQLite). None = disabled.
    pub admin: Option<Arc<AdminHub>>,
    /// Hot probe materials by VTID (demote idle → probe_cold).
    pub hot_probe: Arc<HotProbeCache>,
    /// Robots fast lane: session_ids already classified robots at open/gateway
    /// (UA-declared crawler). Later ingests for these skip L1/L3/analyze arms.
    /// sid → marked_ms; pruned by age when large.
    pub robot_sessions: Arc<dashmap::DashMap<String, i64>>,
    /// Drain flag: ops sets it before a maintenance window. `/readyz` returns 503
    /// and probe data routes answer 503 while set (LB / rollout orchestration).
    pub draining: Arc<AtomicBool>,
    /// Process boot time (ms epoch) for uptime metrics.
    pub boot_ms: i64,
}

fn peer_from_headers(headers: &HashMap<String, String>) -> Option<SocketAddr> {
    headers
        .get("x-gr-peer-ip")
        .and_then(|ip| format!("{ip}:0").parse().ok())
        .or_else(|| {
            headers
                .get("x-gr-peer-addr")
                .and_then(|a| a.parse().ok())
        })
}

fn resolve_client_ip(headers: &HashMap<String, String>) -> (Option<String>, &'static str) {
    let peer = peer_from_headers(headers);
    let ip = client_ip_from(headers, peer);
    let src = client_ip_source(headers, peer);
    (ip, src)
}

/// Normalize request host for multi-tenant domain → site_id lookup.
/// Prefer the connection Host header. X-Forwarded-Host is used only when
/// `GR_TRUST_FORWARDED=1` (edge that strips client-supplied forwarded headers).
fn trust_forwarded_headers() -> bool {
    matches!(
        gr_abi::env::get("TRUST_FORWARDED")
            .as_deref()
            .map(|s| s.trim()),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

fn request_hostname(headers: &HashMap<String, String>) -> String {
    let host_raw = headers
        .get("host")
        .or_else(|| headers.get("Host"))
        .map(|s| s.as_str())
        .unwrap_or("");
    let mut host = host_raw
        .split(',')
        .next()
        .unwrap_or(host_raw)
        .trim()
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if host.is_empty() && trust_forwarded_headers() {
        let fwd = headers
            .get("x-forwarded-host")
            .or_else(|| headers.get("X-Forwarded-Host"))
            .map(|s| s.as_str())
            .unwrap_or("");
        host = fwd
            .split(',')
            .next()
            .unwrap_or(fwd)
            .trim()
            .split(':')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
    }
    if host.is_empty() {
        if let Some(origin) = headers
            .get("origin")
            .or_else(|| headers.get("Origin"))
            .map(|s| s.as_str())
        {
            let o = origin
                .trim_start_matches("https://")
                .trim_start_matches("http://");
            host = o
                .split('/')
                .next()
                .unwrap_or("")
                .split(':')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
        }
    }
    host
}

fn is_prod_env() -> bool {
    matches!(
        gr_abi::env::get("DEPLOY_ENV")
             .or_else(|| gr_abi::env::get("DEPLOY_ENV"))
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "prod" | "production" | "live"
    )
}

/// Multi-tenant isolation: resolve site_id from Host, then optionally a matching edge header.
fn resolve_site_id_from_host(st: &AppState, headers: &HashMap<String, String>) -> String {
    let host = request_hostname(headers);
    let mut host_site = String::new();
    if !host.is_empty() {
        if let Some(admin) = st.admin.as_ref() {
            if let Ok(Some(dom)) = admin.db.get_domain_by_hostname(&host) {
                if let Some(sid) = dom.get("site_id").and_then(|v| v.as_str()) {
                    if !sid.is_empty() {
                        host_site = sid.to_string();
                    }
                }
            }
        }
    }
    // Production: Host → site is the only tenant source. Client/edge site headers
    // cannot invent or override a tenant even when GR_TRUST_FORWARDED=1.
    if is_prod_env() {
        return host_site;
    }
    for k in ["x-gr-site-id", "x-site-id"] {
        if let Some(sid) = headers.get(k).map(|s| s.trim()).filter(|s| !s.is_empty()) {
            let sid = sid.to_ascii_lowercase();
            if !host_site.is_empty() && sid != host_site {
                continue;
            }
            if trust_forwarded_headers() {
                return sid;
            }
            if !host_site.is_empty() && sid == host_site {
                return sid;
            }
        }
    }
    host_site
}

/// Session URL id is an index; caller must present the bound site (Host).
fn require_session_site_auth(
    st: &AppState,
    headers: &HashMap<String, String>,
    session_id: &str,
) -> Result<(), ApiError> {
    let bound = lookup_session_site(st, session_id).filter(|s| !s.is_empty());
    let caller = resolve_site_id_from_host(st, headers);
    if is_prod_env() && caller.is_empty() {
        return Err(ApiError(403, "session_site_required".into()));
    }
    if let Some(bound) = bound {
        if !caller.is_empty() && caller != bound {
            return Err(ApiError(403, "session_site_mismatch".into()));
        }
    }
    Ok(())
}

/// Site/domain `collect_enabled` from the synced probe admin store (PG when configured).
/// Missing admin or missing row → allow (lab open); explicit false → reject.
pub fn collect_enabled_from_json(row: &Value) -> Option<bool> {
    row.get("collect_enabled")
        .and_then(|v| v.as_bool())
        .or_else(|| row.get("collect_enabled").and_then(|v| v.as_i64()).map(|i| i != 0))
}

fn site_collect_enabled(st: &AppState, site_id: &str, headers: &HashMap<String, String>) -> bool {
    let Some(admin) = st.admin.as_ref() else {
        return true;
    };
    if !site_id.is_empty() {
        if let Ok(Some(site)) = admin.db.get_site(site_id) {
            if let Some(en) = collect_enabled_from_json(&site) {
                return en;
            }
        }
    }
    let host = request_hostname(headers);
    if !host.is_empty() {
        if let Ok(Some(dom)) = admin.db.get_domain_by_hostname(&host) {
            if let Some(en) = collect_enabled_from_json(&dom) {
                return en;
            }
        }
    }
    true
}

fn reject_collect_disabled() -> ApiError {
    ApiError(403, "collect_disabled".into())
}

/// Authoritative client IP must **overwrite** any client-supplied `fields.server_client_ip`.
fn apply_authoritative_client_ip(fields: &mut Map<String, Value>, headers: &HashMap<String, String>) -> (Option<String>, &'static str) {
    let (ip, src) = resolve_client_ip(headers);
    // iss/opus5 05-S-4: mask to the network class (/24, /48) at the earliest
    // point; the masked value is the single form written to both the payload
    // field and the indexed columns (no raw/masked dual-write). Full IPs only
    // under GR_IP_FORENSIC_MODE=1 (pair with a short retention window).
    let ip = ip.map(|s| gr_probe_core::privacy::apply_ip_policy(&s));
    if let Some(ref ip_s) = ip {
        fields.insert("server_client_ip".into(), json!(ip_s));
        fields.insert("server_client_ip_source".into(), json!(src));
    }
    (ip, src)
}

/// Drop idle VT entries from the in-process **L1** hot map.
///
/// **Dual-write guarantee:** every live ingest/gateway path already writes
/// L2 `probe_batches` + L3 `probe_cold` via `upsert_batch_with_ip`. Demotion
/// **only removes L1** (no `hot_demote` rows).
///
/// **Multi-worker:** analyze uses L2 PG (`build_evidence`), which is shared.
/// L1 is per-process; on VT re-upload any worker promotes L3→L1.
///
/// Also purges L3 rows past cold TTL (best-effort, rate-limited by call site).
///
/// Call sites: opportunistic on each plain ingest (`hot_idle_ms()`), and
/// explicit lab/ops via `ops_demote_idle`.
///
/// **Flood hardening (1.0.10 root-cause fix):**
/// - Throttled: at most one sweep per `arm_sweep_interval_ms` per process —
///   the 178 crawler flood turned per-ingest sweeps into ~520k arm-upserts/min.
/// - Bounded + result-deduped: arms only sessions **without any analysis
///   result** (already-analyzed sessions get their arms from the ingest path),
///   capped at `arm_sweep_cap` per sweep, one set-based roundtrip.
/// - Overflow stays in L1: entries beyond the cap demote on a later sweep
///   instead of being dropped un-armed.
fn demote_hot_to_cold(st: &AppState, idle_ms: i64) -> usize {
    demote_hot_to_cold_opts(st, idle_ms, false)
}

fn demote_hot_to_cold_opts(st: &AppState, idle_ms: i64, force: bool) -> usize {
    if !force {
        // Throttle: plain ingests share one sweep slot per interval.
        static LAST_SWEEP_MS: std::sync::atomic::AtomicI64 =
            std::sync::atomic::AtomicI64::new(0);
        let now = gr_probe_store::hot_now_ms();
        let interval = gr_probe_store::arm_sweep_interval_ms().max(1_000);
        let last = LAST_SWEEP_MS.load(std::sync::atomic::Ordering::Relaxed);
        if now.saturating_sub(last) < interval {
            return 0;
        }
        if LAST_SWEEP_MS
            .compare_exchange(
                last,
                now,
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
            )
            .is_err()
        {
            return 0; // another thread is sweeping
        }
    }
    let idle = st.hot_probe.list_idle(idle_ms);
    if idle.is_empty() {
        // Nothing idle: still run the opportunistic cold TTL purge (cheap, bounded).
        let cutoff = gr_probe_store::hot_now_ms() - cold_ttl_ms();
        let _ = st.store.purge_expired_cold(cutoff);
        return 0;
    }
    // Robots never need the demote arm (early-class result already stands in).
    let cap = gr_probe_store::arm_sweep_cap().max(1) as usize;
    let selected: Vec<&HotProbeEntry> = idle
        .iter()
        .filter(|e| !e.session_id.is_empty())
        .filter(|e| !st.robot_sessions.contains_key(&e.session_id))
        .take(cap)
        .collect();
    let sids: Vec<String> = selected.iter().map(|e| e.session_id.clone()).collect();
    let armed = st
        .store
        .arm_analyze_if_no_result(&sids)
        .unwrap_or(sids.len() as i64);
    let _ = armed;
    let demoted = if selected.len() == idle.len() {
        st.hot_probe.demote_idle(idle_ms)
    } else {
        // Overflow beyond the arm budget stays hot for the next sweep.
        let vts: Vec<String> = selected.iter().map(|e| e.visitor_terminal_id.clone()).collect();
        st.hot_probe.demote_vts(&vts)
    };
    // Opportunistic cold TTL purge (ignore errors — never block ingest).
    let cutoff = gr_probe_store::hot_now_ms() - cold_ttl_ms();
    let _ = st.store.purge_expired_cold(cutoff);
    demoted.len()
}

/// Robots fast lane helpers — UA-declared crawler sessions (facet=robots).
/// Such visitors never reuse the session/VT, so deep identity probing and
/// store-then-analyze adds nothing: skip L1 hot, skip L3 cold (no future
/// promote), skip analyze arms, and write a one-shot early-class result.
fn robot_fastlane_active(st: &AppState) -> bool {
    gr_probe_store::robot_fastlane_enabled()
}

fn mark_robot_session(st: &AppState, sid: &str) {
    if sid.is_empty() {
        return;
    }
    let now = gr_probe_store::hot_now_ms();
    st.robot_sessions.insert(sid.to_string(), now);
    // Bounded: prune stale marks (robots never come back on the same sid).
    if st.robot_sessions.len() > 8192 {
        let cutoff = now - 6 * 60 * 60 * 1000;
        st.robot_sessions.retain(|_, t| *t > cutoff);
    }
}

fn session_is_robot(st: &AppState, sid: &str) -> bool {
    !sid.is_empty() && st.robot_sessions.contains_key(sid)
}

/// Write the early-class result for a confirmed-robots session (idempotent —
/// skipped when any analysis result already exists). Best-effort: never
/// blocks the request path.
fn try_robot_early_result(st: &AppState, sid: &str, robot_name: Option<&str>, source: &str) {
    if !robot_fastlane_active(st) || sid.is_empty() {
        return;
    }
    if let Err(e) = st.store.save_early_class_result(
        sid,
        robot_name,
        source,
        gr_probe_core::GR_PRODUCT_VERSION,
    ) {
        log::debug!("robot early-class result sid={sid} source={source} err={e}");
    }
}

/// If VT is not in L1 hot, load recent non-expired L3 cold rows into L1.
/// Returns number of batches promoted.
fn promote_cold_to_hot_if_needed(
    st: &AppState,
    visitor_terminal_id: &str,
    current_session_id: &str,
) -> usize {
    if visitor_terminal_id.is_empty() {
        return 0;
    }
    if st.hot_probe.is_hot(visitor_terminal_id, hot_idle_ms()) {
        return 0;
    }
    let since = gr_probe_store::hot_now_ms() - cold_promote_window_ms().min(cold_ttl_ms());
    let listed = match st.store.list_cold_for_vt(visitor_terminal_id, since, 32) {
        Ok(v) => v,
        Err(_) => return 0,
    };
    let rows = listed.get("rows").and_then(|r| r.as_array()).cloned().unwrap_or_default();
    let mut n = 0usize;
    for row in rows {
        let batch_id = row.get("batch_id").and_then(|v| v.as_str()).unwrap_or("");
        if batch_id.is_empty() {
            continue;
        }
        let sid = row
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or(current_session_id);
        let payload = row
            .get("payload_full")
            .cloned()
            .or_else(|| {
                row.get("fields_json").map(|f| {
                    if f.get("fields").is_some() {
                        f.clone()
                    } else {
                        json!({"fields": f})
                    }
                })
            })
            .unwrap_or(json!({}));
        let ip = row.get("client_ip").and_then(|v| v.as_str());
        st.hot_probe.promote_batch(
            visitor_terminal_id,
            sid,
            batch_id,
            &payload,
            ip,
        );
        n += 1;
    }
    // Cold→hot = new batch context: re-arm analyze clocks (do not inherit prior terminal).
    if n > 0 {
        let now = gr_probe_store::hot_now_ms();
        let clocks = new_batch_analyze_clocks(now);
        let _ = st.store.merge_session_meta(current_session_id, &clocks);
        let _ = st.store.schedule_analyze_merge(
            current_session_id,
            60_000,
            AnalyzeDueMerge::IdleReset,
        );
    }
    n
}

#[derive(Deserialize)]
pub struct OpenBody {

    pub session_id: Option<String>,
    pub visitor_terminal_id: Option<String>,
    pub meta: Option<Value>,
    /// cf_worker | nginx | app — persisted into session.meta.inject_path
    pub inject_path: Option<String>,
    /// Cool-down ticket from prior analyze (G-PROD-8): skip session packs, keep page rpa.
    pub session_ticket: Option<Value>,
    /// Website business site id (dashboard / SDK binding).
    pub site_id: Option<String>,
    /// Force full identity re-probe (invalidates ticket + cool).
    #[serde(default)]
    pub force_identity: Option<bool>,
    /// Signed storage bind from prior open/complete (anti-tamper cool/VT).
    pub storage_bind: Option<String>,
    /// Optional relay anti-replay (first-party SDK).
    pub relay_ts_ms: Option<i64>,
    pub relay_nonce: Option<String>,
    pub relay_sig: Option<String>,
    /// Allowlisted business cookies collected by the probe script from
    /// `document.cookie` (pv context or same-origin script proxy). Preferred
    /// over the Cookie header (gv is a different origin).
    #[serde(default)]
    pub cookie_fields: Option<Value>,
    /// Site embed token (`grst_…`) — must match the site that minted `/gr.js?grt=`.
    #[serde(default)]
    pub embed_token: Option<String>,
}

#[derive(Deserialize, serde::Serialize)]
pub struct IngestBody {
    pub session_id: String,
    pub batch_id: String,
    #[serde(default = "default_source")]
    pub source: String,
    #[serde(default)]
    pub payload: Value,
    /// Default false: ingest must not wait on analyze (race semantics).
    #[serde(default)]
    pub analyze: bool,
    pub inject_path: Option<String>,
    /// iss/70: immutable capture identity (FE freezeCapture).
    #[serde(default)]
    pub capture_id: Option<String>,
    #[serde(default)]
    pub material_generation: Option<i64>,
    /// Client-side hash of inner fields (fnv/sha); echoed in ack for FE registry.
    #[serde(default)]
    pub payload_hash: Option<String>,
    #[serde(default)]
    pub attempt_id: Option<String>,
    /// iss/72: FE plan_epoch at collect/send time — reject if older than server session epoch.
    #[serde(default)]
    pub plan_epoch: Option<i64>,
    #[serde(default)]
    pub source_kind: Option<String>,
    #[serde(default)]
    pub realm_id: Option<String>,
    #[serde(default)]
    pub realm_kind: Option<String>,
    #[serde(default)]
    pub probe_method_id: Option<String>,
    #[serde(default)]
    pub method_version: Option<String>,
    #[serde(default)]
    pub observation_id: Option<String>,
    /// Allowlisted cookies from the probe script (first capture wins).
    #[serde(default)]
    pub cookie_fields: Option<Value>,
}

#[derive(Deserialize)]
pub struct GatewayEarlyBody {

    pub session_id: Option<String>,
    pub visitor_terminal_id: Option<String>,
    pub inject_path: Option<String>,
    #[serde(default)]
    pub fields: Value,
    /// Default false — early kick never blocks on analyze.
    #[serde(default)]
    pub analyze: bool,
    /// Website business site id (dashboard / robots|browser path).
    pub site_id: Option<String>,
    /// iss/opus5 S-14: optional session ticket (same shape as open). When
    /// present and valid it stamps skip_session_probe onto the cycle bag; when
    /// present but invalid/mismatched nothing is skipped and the request
    /// proceeds as an ordinary unticketed gateway early (parity with open).
    #[serde(default)]
    pub session_ticket: Option<Value>,
}

#[derive(Deserialize)]
pub struct BizVisitBody {
    pub site_id: String,
    pub visitor_terminal_id: String,
    #[serde(default)]
    pub visitor_facet: Option<String>,
    pub session_id: Option<String>,
    pub page_host: Option<String>,
    #[serde(default)]
    pub summary: Option<Value>,
    #[serde(default)]
    pub events: Option<Vec<String>>,
    pub event: Option<String>,
}

#[derive(Deserialize)]
pub struct AnalyzeBody {
    /// Deprecated client field — **ignored**. Soft gate is `AppState.soft_v2_ready` only (A-BRAIN-3 / norm/04).
    #[serde(default)]
    #[allow(dead_code)]
    pub soft_v2_ready: Option<bool>,
    /// Explicit multi-session peers (parallel/batch browsers). Optional —
    /// when empty, server discovers peers via visitor_terminal_id / harness_run.
    #[serde(default)]
    pub peer_session_ids: Vec<String>,
    /// FE pagehide / short-visit close: enable identity emit with min evidence.
    #[serde(default)]
    pub pagehide_flush: Option<bool>,
    /// iss/opus5 04-P0-3: request the full (verbose) analysis envelope.
    /// Honored only with an active ops credential — see `ops_grant_active`.
    #[serde(default)]
    pub verbose: Option<bool>,
}

/// Materials needed for soft multi-session association (not commercial mint alone).
/// Includes residual / host / dual-HW digests so composite_association can score
/// same-host continuity without force-merging public dh_.
const PEER_ASSOC_KEYS: &[&str] = &[
    "os_family",
    "hardware_concurrency",
    "screen_width",
    "screen_height",
    "timezone",
    "webgl_unmasked_renderer",
    "form_class",
    "device_memory",
    "platform",
    "user_agent",
    // Soft host / silicon continuity (never force-merge floors)
    "residual_mean",
    "residual_std",
    "residual_class",
    "hw_curve_webgl",
    "hw_curve_audio",
    "hw_webgl_stable",
    "hw_audio_stable",
    "webrtc_host_ip_hash",
    "webrtc_host_ip_hash_v2",
    "os_instance_hash",
    "architecture",
    "engine_family",
    "engine_obs",
    "unit_surface_id",
    "unit_surface_algo",
    "unit_multiround_stable",
    "residual_entropy_ok",
    "webgl_residual_entropy_ok",
];

/// Build peer field vector + full peer evidence for multi-session link/soft.
/// Prefers explicit peer ids, then store cohort (same VT / harness_run).
///
/// Returns the **first** rich peer for evaluate ensemble, and `used` lists all
/// peers that had usable materials (for soft_edges write path).
///
/// P2: 一次往返取全部 peer 的关联键瘦投影 (原逐 peer 全量 build_evidence —
/// 12 peer 时 ~24 查询 + 全量合并机件只为提 ~29 个键, 178 实测为 peer
/// 证据风暴大头)。首个富 peer 的 evaluate 集成仍需完整证据 — 仅此一次
/// 全量构建。
pub fn resolve_multi_session_peers(
    store: &gr_probe_store::Store,
    session_id: &str,
    explicit: &[String],
) -> (Option<Value>, Option<Value>, Vec<String>) {
    let mut ids: Vec<String> = explicit
        .iter()
        .filter(|s| !s.is_empty() && s.as_str() != session_id)
        .cloned()
        .collect();
    if ids.is_empty() {
        // Same-VT + harness peers (sticky multi-cycle / multi-tab).
        if let Ok(found) = store.list_peer_session_ids(session_id, 12) {
            ids = found;
        }
    }
    let projections: std::collections::HashMap<String, serde_json::Map<String, Value>> = store
        .peer_assoc_fields(&ids, PEER_ASSOC_KEYS)
        .ok()
        .and_then(|v| v.as_object().cloned())
        .map(|m| {
            m.into_iter()
                .filter_map(|(k, v)| v.as_object().cloned().map(|fo| (k, fo)))
                .collect()
        })
        .unwrap_or_default();
    let mut used = Vec::new();
    let mut first_vec: Option<Value> = None;
    let mut first_ev: Option<Value> = None;
    for pid in ids {
        if pid == session_id {
            continue;
        }
        // 无投影条目 = 无可用批次材料 (旧路径 build_evidence 失败/空 fields 同效跳过)
        let Some(fo) = projections.get(&pid) else {
            continue;
        };
        let mut peer_vec = serde_json::Map::new();
        for k in PEER_ASSOC_KEYS {
            if let Some(v) = fo.get(*k) {
                if !v.is_null() && v.as_str() != Some("") {
                    peer_vec.insert((*k).into(), v.clone());
                }
            }
        }
        // Prefer peers with residual or host materials for soft association.
        let rich = peer_vec.contains_key("residual_mean")
            || peer_vec.contains_key("hw_webgl_stable")
            || peer_vec.contains_key("webrtc_host_ip_hash")
            || peer_vec.len() >= 3;
        if !rich && peer_vec.len() < 2 {
            continue;
        }
        used.push(pid.clone());
        if first_vec.is_none() {
            first_vec = Some(Value::Object(peer_vec));
            // evaluate 集成需要首个富 peer 的完整证据 — 仅此一次全量构建。
            first_ev = store.build_evidence(&pid).ok();
        }
        // Cap soft peer fan-out (analyze path walks used for edges).
        if used.len() >= 8 {
            break;
        }
    }
    (first_vec, first_ev, used)
}

/// Collect soft-association peer session candidates (never commercial merge).
///
/// Prefer **signal-rich** peers over `list_recent_session_ids` (lab has ~80% B8-only
/// bags with no residual — those waste evidence builds and starve cross-browser edges).
///
/// Priority:
/// 1. Explicit same-VT / multi-session peers
/// 2. `lan:{webrtc}` binder → other commercial device_ids → their sessions
/// 3. `au:{hw_audio_stable}` binder sessions (same audio silicon, different wg floor)
/// 4. analysis_latest residual_entropy_ok (dh/dv) recent window
/// 5. Fallback raw recent ids only if still under cap
fn soft_assoc_candidate_ids(
    st: &AppState,
    tenant: &str,
    session_id: &str,
    fields: &Value,
    peer_ids: &[String],
) -> Vec<String> {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let my_rm = fo
        .get("residual_mean")
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)));
    let my_rtc = fo
        .get("webrtc_host_ip_hash")
        .or_else(|| fo.get("webrtc_host_ip_hash_v2"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let my_au = fo
        .get("hw_audio_stable")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let mut candidates: Vec<String> = Vec::new();
    let push = |candidates: &mut Vec<String>, sid: String| {
        if sid.is_empty() || sid == session_id {
            return;
        }
        if !candidates.contains(&sid) {
            candidates.push(sid);
        }
    };

    for p in peer_ids {
        push(&mut candidates, p.clone());
    }

    // Binder reverse index: same LAN webrtc host → different dh_ across browsers.
    if let Some(ref rtc) = my_rtc {
        let key = format!("lan:{rtc}");
        if let Ok(look) = st.store.lookup_devices_by_binder(tenant, &key, 12) {
            if let Some(rows) = look.get("rows").and_then(|v| v.as_array()) {
                for row in rows {
                    let Some(did) = row.get("device_id").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    // Skip pure gateway empties for soft host graph — they lack residual.
                    let tier = row.get("tier_last").and_then(|v| v.as_str()).unwrap_or("");
                    let dig = row
                        .get("digest_path_last")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if dig.contains("empty_anchor") || dig.contains("gateway_only") {
                        continue;
                    }
                    if tier == "dg" && dig.contains("empty") {
                        continue;
                    }
                    if let Ok(ds) = st.store.list_device_sessions(did, 6) {
                        if let Some(srows) = ds.get("rows").and_then(|v| v.as_array()) {
                            for s in srows {
                                if let Some(sid) = s.get("session_id").and_then(|v| v.as_str()) {
                                    push(&mut candidates, sid.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Same commercial audio digest across engines (wg may fork at 0.001).
    if let Some(ref au) = my_au {
        let key = format!("au:{au}");
        if let Ok(look) = st.store.lookup_devices_by_binder(tenant, &key, 8) {
            if let Some(rows) = look.get("rows").and_then(|v| v.as_array()) {
                for row in rows {
                    let Some(did) = row.get("device_id").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    if let Ok(ds) = st.store.list_device_sessions(did, 4) {
                        if let Some(srows) = ds.get("rows").and_then(|v| v.as_array()) {
                            for s in srows {
                                if let Some(sid) = s.get("session_id").and_then(|v| v.as_str()) {
                                    push(&mut candidates, sid.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Prefer residual-complete analyses (dh/dv) over B8-only recent sessions.
    if my_rm.is_some() {
        if let Ok(al) = st.store.list_analysis_latest(80, None, None, None) {
            if let Some(rows) = al.get("rows").and_then(|v| v.as_array()) {
                for row in rows {
                    let ok = row
                        .get("residual_entropy_ok")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let tier = row.get("device_tier").and_then(|v| v.as_str()).unwrap_or("");
                    let dig = row.get("digest_path").and_then(|v| v.as_str()).unwrap_or("");
                    if dig.contains("empty_anchor") || dig.contains("gateway_only") {
                        continue;
                    }
                    // Soft host graph only needs residual-capable peers.
                    if !ok && tier != "dh" && tier != "dv" {
                        continue;
                    }
                    if let Some(sid) = row.get("session_id").and_then(|v| v.as_str()) {
                        push(&mut candidates, sid.to_string());
                    }
                }
            }
        }
    }

    // Last resort: recent bag ids (many will be B8-thin; cap keeps cost bounded).
    if candidates.len() < 12 {
        if let Ok(recent) = st.store.list_recent_session_ids(60) {
            for rid in recent {
                push(&mut candidates, rid);
                if candidates.len() >= 32 {
                    break;
                }
            }
        }
    }

    candidates.truncate(40);
    candidates
}

/// Persist soft association edges as **window materials** for peer_similarity scoring.
/// Product surface is `peer_similarity` on analyze result — not a business account graph.
///
/// Sources:
/// 1) Same-VT / explicit peers
/// 2) lan/au binder device_index → sessions (cross-browser same host/silicon)
/// 3) residual_entropy_ok analysis_latest window
///
/// Policy: `promote_to_commercial_id=false` always. Empty residual / gateway-only
/// only soft-attachs to VT's richer peer when host/residual agree.
fn persist_soft_association_edges(
    st: &AppState,
    tenant: &str,
    session_id: &str,
    fields: &Value,
    peer_ids: &[String],
) {
    let fo = match fields.as_object() {
        Some(o) if !o.is_empty() => o,
        _ => return,
    };
    let my_rm = fo
        .get("residual_mean")
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)));
    let my_rtc = fo
        .get("webrtc_host_ip_hash")
        .or_else(|| fo.get("webrtc_host_ip_hash_v2"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let my_au = fo
        .get("hw_audio_stable")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let my_shc = gr_probe_core::engine_surface::soft_host_cluster_from_fields(fields);
    let my_shc_id = my_shc
        .get("soft_host_cluster_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let candidates = soft_assoc_candidate_ids(st, tenant, session_id, fields, peer_ids);

    for pid in candidates {
        if pid == session_id {
            continue;
        }
        let Ok(ev) = st.store.build_evidence(&pid) else {
            continue;
        };
        let Some(pf) = ev.get("fields") else {
            continue;
        };
        let pfo = match pf.as_object() {
            Some(o) if !o.is_empty() => o,
            _ => continue,
        };
        let peer_rm = pfo
            .get("residual_mean")
            .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)));
        let peer_rtc = pfo
            .get("webrtc_host_ip_hash")
            .or_else(|| pfo.get("webrtc_host_ip_hash_v2"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());
        let peer_au = pfo
            .get("hw_audio_stable")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());

        let composite = gr_probe_core::composite_association::composite_associate(fields, pf);
        let likely = composite
            .get("likely_same_machine")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let public_ok = composite
            .get("public_device_link")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let band = composite
            .get("continuity_band")
            .and_then(|v| v.as_str())
            .unwrap_or("distinct");
        let assoc = composite
            .get("assoc_score")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let wg_disagree = composite
            .get("wg_disagree")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // residual@0.005 agreement (soft host quanta — not commercial 0.001 floor)
        let residual_soft_agree = match (my_rm, peer_rm) {
            (Some(a), Some(b)) => ((a / 0.005).round() - (b / 0.005).round()).abs() < 0.5,
            _ => false,
        };
        let rtc_agree = match (my_rtc, peer_rtc) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        };
        let audio_agree = match (my_au, peer_au) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        };
        let peer_shc = gr_probe_core::engine_surface::soft_host_cluster_from_fields(pf);
        let peer_shc_id = peer_shc
            .get("soft_host_cluster_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let shc_agree = !my_shc_id.is_empty()
            && !peer_shc_id.is_empty()
            && my_shc_id == peer_shc_id;

        // Soft edge gates (never promote):
        // - composite likely_same_machine OR public_device_link
        // - residual@0.005 + webrtc (cross-engine same host; wg may disagree @0.001)
        // - residual@0.005 + same hw_audio_stable (audio silicon continuity)
        // - identical soft_host_cluster_id with residual present
        let soft_ok = likely
            || public_ok
            || (residual_soft_agree && rtc_agree)
            || (residual_soft_agree && audio_agree)
            || (shc_agree && residual_soft_agree && assoc >= 0.35);
        if !soft_ok {
            continue;
        }
        // Guard CGNAT false merge: residual-only without host/audio/composite.
        if residual_soft_agree
            && !rtc_agree
            && !audio_agree
            && !likely
            && !public_ok
            && !shc_agree
        {
            continue;
        }
        // Empty-anchor / no residual peer: only attach if composite already likely
        // (same-VT sticky B8→rich) or host+residual both agree.
        if my_rm.is_none() && peer_rm.is_none() && !likely {
            continue;
        }

        let conf = if public_ok {
            (assoc.max(0.72)).min(0.95)
        } else if likely {
            (assoc.max(0.62)).min(0.90)
        } else if residual_soft_agree && rtc_agree {
            if wg_disagree {
                0.74
            } else {
                0.80
            }
        } else if residual_soft_agree && audio_agree {
            if wg_disagree {
                0.70
            } else {
                0.76
            }
        } else if shc_agree {
            0.66
        } else {
            (assoc * 0.9).clamp(0.45, 0.70)
        };
        let reason = if public_ok {
            "composite_public_link_soft_persist"
        } else if likely {
            "composite_likely_same_machine"
        } else if residual_soft_agree && rtc_agree && wg_disagree {
            "residual_xbr_webrtc_wg_floor_diverge"
        } else if residual_soft_agree && rtc_agree {
            "residual_xbr_webrtc_host"
        } else if residual_soft_agree && audio_agree && wg_disagree {
            "residual_xbr_audio_wg_floor_diverge"
        } else if residual_soft_agree && audio_agree {
            "residual_xbr_audio_silicon"
        } else if shc_agree {
            "soft_host_cluster_id"
        } else {
            "composite_soft_band"
        };
        let priority = if public_ok || (residual_soft_agree && rtc_agree && !wg_disagree) {
            "P0_soft_host"
        } else if likely
            || (residual_soft_agree && rtc_agree)
            || (residual_soft_agree && audio_agree)
        {
            "P1_soft_host"
        } else {
            "P2_soft_host"
        };
        let edge = SoftEdge {
            a_session: session_id.to_string(),
            b_session: pid,
            priority: priority.into(),
            promote_to_commercial_id: false,
            confidence: conf,
            reason: format!("{reason}|band={band}|assoc={assoc:.3}"),
        };
        let _ = st.soft_store.put_edge(tenant, &edge);
    }
}

pub fn default_source() -> String {
    "main".into()
}

/// Merge inject_path into session meta.
/// Primary paths (nginx / cf_worker) never downgrade to app once set.
pub fn merge_inject_meta(meta: Option<Value>, inject_path: Option<String>) -> Value {
    let mut m = meta.unwrap_or_else(|| json!({}));
    if !m.is_object() {
        m = json!({});
    }
    if let Some(ip) = inject_path {
        if let Some(obj) = m.as_object_mut() {
            let existing = obj
                .get("inject_path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let primary = |s: &str| s == "nginx" || s == "cf_worker";
            if primary(existing) && !primary(&ip) {
                // keep existing primary
            } else {
                obj.insert("inject_path".into(), json!(ip));
            }
        }
    }
    m
}

#[derive(Deserialize)]
pub struct EvaluateBody {
    pub evidence: Value,
    pub peer: Option<Value>,
    pub strategy: Option<Value>,
    /// **Offline / lab only** (`POST /v1/evaluate`).
    /// Default **false**. This is **not** the session Soft gate —
    /// live `analyze_session` always uses `AppState.soft_v2_ready` and ignores body
    /// (A-BRAIN-3 / norm/04). Callers must not treat this as production Soft policy.
    #[serde(default)]
    pub soft_v2_ready: bool,
}

pub struct ApiError(pub u16, pub String);

/// Open/cool wire budget: never ship full evaluate (battle_log / evidence) to FE.
/// Full result remains available via `/v1/session/:id/analyses`.
fn thin_identity_result(v: &Value) -> Value {
    if v.is_null() {
        return Value::Null;
    }
    let device_id = v
        .pointer("/product/device_id")
        .or_else(|| v.get("device_id"))
        .cloned()
        .unwrap_or(Value::Null);
    let real_band = v
        .get("real_band")
        .or_else(|| v.pointer("/product/real_band"))
        .cloned()
        .unwrap_or(Value::Null);
    // bot lives at root `bot.verdict` (not product.bot_verdict).
    let bot_verdict = v
        .pointer("/bot/verdict")
        .or_else(|| v.pointer("/product/bot/verdict"))
        .or_else(|| v.pointer("/product/bot_verdict"))
        .or_else(|| v.get("bot_verdict"))
        .cloned()
        .unwrap_or(Value::Null);
    let recommended_action = v
        .pointer("/product/recommended_action/action")
        .or_else(|| v.pointer("/recommended_action/action"))
        .cloned()
        .unwrap_or(Value::Null);
    json!({
        "thin": true,
        "real_band": real_band,
        "device_id": device_id,
        "device_tier": v.pointer("/product/device_tier").or_else(|| v.get("device_tier")).cloned().unwrap_or(Value::Null),
        "product_version": v.get("product_version").cloned().unwrap_or(Value::Null),
        "analysis_terminal": v.get("analysis_terminal").cloned().unwrap_or(Value::Null),
        "bot_verdict": bot_verdict,
        "recommended_action": recommended_action,
        "note": "open cool summary only; full analysis via GET /v1/session/:id/analyses (or /result?strategy_id=… for product_public)",
    })
}

/// Soft cycle-closed body (HTTP 200) — no browser console red error for expected product halt.
/// FE: halt identity uploads + cool; do not treat as transport failure.
fn soft_cycle_closed_json(reason: &str, detail: &str) -> Value {
    let biz = match reason {
        "cycle_complete" => "identity_complete_cool",
        "cycle_purged" => "purged",
        "incomplete_ttl" => "incomplete_expired",
        _ => "inactive",
    };
    let cycle_status = match reason {
        "cycle_complete" => "complete",
        "cycle_purged" => "purged",
        _ => "inactive",
    };
    json!({
        "ok": true,
        "accepted": false,
        "identity_accepted": false,
        "code": reason,
        "expired_reason": reason,
        "halt_uploads": true,
        "skip_identity_probe": true,
        "cycle_closed": true,
        "business_state": biz,
        "product_version": gr_probe_core::GR_PRODUCT_VERSION,
        "detail": detail.chars().take(200).collect::<String>(),
        "cycle_probe_status": {
            "algo": "cycle_probe_status_v3_soft200",
            "halt_uploads": true,
            "cycle_closed": true,
            "skip_identity_probe": true,
            "expired_reason": reason,
            "business_state": biz,
            "cycle_status": cycle_status,
            "http_soft_close": true,
        }
    })
}

fn classify_session_expired_reason(msg: &str) -> &'static str {
    if msg.contains("cycle_complete") {
        "cycle_complete"
    } else if msg.contains("cycle_purged") {
        "cycle_purged"
    } else if msg.contains("incomplete_ttl") {
        "incomplete_ttl"
    } else {
        "session_expired"
    }
}

impl ApiError {
    pub fn status(&self) -> u16 { self.0 }
    pub fn to_json(&self) -> Value {
        // Soft close (v5.8.124): expected complete/cool → HTTP 200 + halt fields (not 410).
        if self.0 == 200 && self.1.starts_with("SOFT_CYCLE_CLOSED|") {
            let detail = &self.1["SOFT_CYCLE_CLOSED|".len()..];
            let reason = classify_session_expired_reason(detail);
            return soft_cycle_closed_json(reason, detail);
        }
        let mut o = json!({"ok": false, "error": self.1});
        if self.0 == 409 && self.1.starts_with("idempotency_conflict") {
            return json!({
                "ok": false,
                "error": {
                    "code": "idempotency_conflict",
                    "message": "same Idempotency-Key with a different request body",
                    "retryable": false
                }
            });
        }
        if self.0 == 400 && (self.1 == "idempotency_key_required" || self.1 == "idempotency_key_invalid") {
            return json!({
                "ok": false,
                "error": {
                    "code": self.1,
                    "message": self.1,
                    "retryable": false
                }
            });
        }
        // Legacy 410 path (keep for unexpected clients / purge edge cases).
        if self.0 == 410 {
            let reason = classify_session_expired_reason(&self.1);
            if let Some(map) = o.as_object_mut() {
                map.insert("code".into(), json!(reason));
                map.insert("expired_reason".into(), json!(reason));
                map.insert("halt_uploads".into(), json!(true));
                map.insert("skip_identity_probe".into(), json!(true));
                map.insert("cycle_closed".into(), json!(true));
                let biz = match reason {
                    "cycle_complete" => "identity_complete_cool",
                    "cycle_purged" => "purged",
                    "incomplete_ttl" => "incomplete_expired",
                    _ => "inactive",
                };
                map.insert("business_state".into(), json!(biz));
                map.insert(
                    "cycle_probe_status".into(),
                    json!({
                        "algo": "cycle_probe_status_v2",
                        "halt_uploads": true,
                        "cycle_closed": true,
                        "skip_identity_probe": true,
                        "expired_reason": reason,
                        "business_state": biz,
                        "cycle_status": if reason == "cycle_complete" { "complete" }
                            else if reason == "cycle_purged" { "purged" }
                            else { "inactive" },
                    }),
                );
            }
        }
        o
    }
}

pub type AppResult = Result<Value, ApiError>;

fn append_observation_event(
    st: &AppState,
    tenant_id: &str,
    session_id: &str,
    batch_id: &str,
    source: &str,
    envelope: &Value,
    capture_id: Option<&String>,
    attempt_id: Option<&String>,
) -> Result<Value, StoreError> {
    let validated = envelope.get("validated").cloned().unwrap_or(json!({}));
    st.store.insert_observation_event(json!({
        "observation_id": envelope.get("observation_id"),
        "tenant_id": tenant_id,
        "session_id": session_id,
        "batch_id": batch_id,
        "source": source,
        "source_kind": validated.get("source_kind"),
        "realm_kind": validated.get("realm_kind"),
        "probe_method_id": validated.get("probe_method_id"),
        "capture_id": capture_id,
        "attempt_id": attempt_id,
        "envelope": envelope,
    }))
}

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::NotFound(m) => ApiError(404, m),
            // Product soft-close: HTTP 200 + body (no browser console 410 red for complete/cool).
            StoreError::SessionExpired(m) => {
                ApiError(200, format!("SOFT_CYCLE_CLOSED|{m}"))
            }
            other => ApiError(500, other.to_string()),
        }
    }
}

/// True if `s` looks like a content hash token (8–16 lowercase hex).
pub fn is_content_hash_token(s: &str) -> bool {
    let n = s.len();
    (8..=16).contains(&n) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Insert content hash into a logical static filename (**Standard C**).
///
/// Cache-bust is **filename-only** — no product version in the URL path.
///
/// Examples:
/// - `gr.race.min.js` + `a1b2c3d4e5f6` → `gr.race.a1b2c3d4e5f6.min.js`
/// - `collectors/registry.static.lite.min.js` + hash → `collectors/registry.static.lite.<hash>.min.js`
/// - `gr_seal_v2.wasm` + hash → `gr_seal_v2.<hash>.wasm`
/// - `nest_frame.html` + hash → `nest_frame.<hash>.html`
pub fn content_hashed_filename(logical: &str, hash: &str) -> String {
    let logical = logical.trim_start_matches('/');
    let hash = hash.trim();
    if hash.is_empty() {
        return logical.to_string();
    }
    if let Some(stem) = logical.strip_suffix(".min.js") {
        return format!("{stem}.{hash}.min.js");
    }
    if let Some(stem) = logical.strip_suffix(".js") {
        // avoid double-insert if already hashed
        if let Some((base, maybe_hash)) = stem.rsplit_once('.') {
            if is_content_hash_token(maybe_hash) {
                return logical.to_string();
            }
            let _ = base;
        }
        return format!("{stem}.{hash}.js");
    }
    if let Some(stem) = logical.strip_suffix(".wasm") {
        return format!("{stem}.{hash}.wasm");
    }
    if let Some(stem) = logical.strip_suffix(".html") {
        return format!("{stem}.{hash}.html");
    }
    if let Some(stem) = logical.strip_suffix(".css") {
        return format!("{stem}.{hash}.css");
    }
    format!("{logical}.{hash}")
}

/// Strip content-hash token from a relative path → logical on-disk name.
///
/// `gr.race.a1b2c3d4e5f6.min.js` → `gr.race.min.js`
pub fn strip_content_hash_filename(rel: &str) -> String {
    let rel = rel.trim_start_matches('/');
    let (dir, file) = match rel.rfind('/') {
        Some(i) => (&rel[..i], &rel[i + 1..]),
        None => ("", rel),
    };
    let unhashed = if let Some(stem) = file.strip_suffix(".min.js") {
        if let Some((base, hash)) = stem.rsplit_once('.') {
            if is_content_hash_token(hash) {
                format!("{base}.min.js")
            } else {
                file.to_string()
            }
        } else {
            file.to_string()
        }
    } else if let Some(stem) = file.strip_suffix(".wasm") {
        if let Some((base, hash)) = stem.rsplit_once('.') {
            if is_content_hash_token(hash) {
                format!("{base}.wasm")
            } else {
                file.to_string()
            }
        } else {
            file.to_string()
        }
    } else if let Some(stem) = file.strip_suffix(".html") {
        if let Some((base, hash)) = stem.rsplit_once('.') {
            if is_content_hash_token(hash) {
                format!("{base}.html")
            } else {
                file.to_string()
            }
        } else {
            file.to_string()
        }
    } else if let Some(stem) = file.strip_suffix(".css") {
        if let Some((base, hash)) = stem.rsplit_once('.') {
            if is_content_hash_token(hash) {
                format!("{base}.css")
            } else {
                file.to_string()
            }
        } else {
            file.to_string()
        }
    } else if let Some(stem) = file.strip_suffix(".js") {
        if let Some((base, hash)) = stem.rsplit_once('.') {
            if is_content_hash_token(hash) {
                format!("{base}.js")
            } else {
                file.to_string()
            }
        } else {
            file.to_string()
        }
    } else {
        file.to_string()
    };
    if dir.is_empty() {
        unhashed
    } else {
        format!("{dir}/{unhashed}")
    }
}

/// Protocol (wire) logical name → greenpng/v8 physical FE file.
///
/// The seal v2 wire names keep the underscore form (`gr_seal_v2.wasm`,
/// `gr_seal_v2_loader.js` — stable URL contract), while the repo files use dot
/// names (`gr.seal_v2.wasm`, `gr.seal_v2_loader.js`). Without this alias every
/// hash lookup misses and URLs degrade to `missing00000.*`.
fn seal_v2_physical(logical_rel: &str) -> &str {
    match logical_rel.trim_start_matches('/') {
        "gr_seal_v2.wasm" => "gr.seal_v2.wasm",
        "gr_seal_v2_loader.js" => "gr.seal_v2_loader.js",
        _ => logical_rel,
    }
}

/// Hash file bytes (12 hex) for URL filename token; missing file → `"missing00000"`.
pub fn file_content_hash12(static_dir: &std::path::Path, logical_rel: &str) -> String {
    let physical = seal_v2_physical(logical_rel);
    let path = static_dir.join(physical.trim_start_matches('/'));
    match std::fs::read(&path) {
        Ok(bytes) => gr_probe_core::sha256_hex(&bytes)
            .chars()
            .take(12)
            .collect(),
        Err(_) => "missing00000".into(),
    }
}

/// Public basename with **no meaningful product tokens** (anti-fingerprint for black-hat targeting).
///
/// Examples (hash = 12 hex of file bytes):
/// - `gr.race.min.js` → `a1b2c3d4e5f6.min.js`
/// - `collectors/registry.static.hard.min.js` → `9f8e7d6c5b4a.min.js`  (flat under /dist/)
/// - `gr_seal_v2.wasm` → `c5b97b9ad21c.wasm`
/// - `nest_frame.html` → `eb829e0c23e8.html`
pub fn opaque_public_filename(logical: &str, hash: &str) -> String {
    let logical = logical.trim_start_matches('/');
    let hash = hash.trim();
    let hash = if hash.is_empty() {
        "missing00000"
    } else {
        hash
    };
    if logical.ends_with(".min.js") {
        return format!("{hash}.min.js");
    }
    if logical.ends_with(".wasm") {
        return format!("{hash}.wasm");
    }
    if logical.ends_with(".html") {
        return format!("{hash}.html");
    }
    if logical.ends_with(".css") {
        return format!("{hash}.css");
    }
    if logical.ends_with(".js") {
        return format!("{hash}.js");
    }
    format!("{hash}.bin")
}

/// Build Standard-C **opaque** URL: `{dist_root}/{contenthash12}.{ext}` (flat, no logical name).
///
/// Physical files stay at logical paths under `fe/`; `OPAQUE_MAP.json` maps public token → logical.
pub fn hashed_asset_url(
    dist_root: &str,
    static_dir: &std::path::Path,
    logical_rel: &str,
) -> String {
    let h = file_content_hash12(static_dir, logical_rel);
    let name = opaque_public_filename(logical_rel, &h);
    // Keep reverse map warm for static resolver.
    let _ = opaque_map_insert(static_dir, &name, logical_rel);
    format!(
        "{}/{}",
        dist_root.trim_end_matches('/'),
        name.trim_start_matches('/')
    )
}

/// Persist/update reverse map entry: public basename → logical on-disk path.
pub fn opaque_map_insert(static_dir: &std::path::Path, public_name: &str, logical: &str) -> bool {
    use std::collections::BTreeMap;
    let map_path = static_dir.join("OPAQUE_MAP.json");
    let mut map: BTreeMap<String, String> = std::fs::read_to_string(&map_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let logical = logical.trim_start_matches('/').to_string();
    let prev = map.insert(public_name.to_string(), logical.clone());
    if prev.as_ref() == Some(&logical) {
        return true;
    }
    if let Ok(body) = serde_json::to_string(&map) {
        let _ = std::fs::write(&map_path, body);
        return true;
    }
    false
}

/// Lookup logical path for an opaque public basename (`a1b2….min.js`).
pub fn opaque_map_lookup(static_dir: &std::path::Path, public_name: &str) -> Option<String> {
    let map_path = static_dir.join("OPAQUE_MAP.json");
    let map: serde_json::Value = std::fs::read_to_string(map_path).ok().and_then(|s| serde_json::from_str(&s).ok())?;
    map.get(public_name)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Rebuild full opaque map for all known FE ship files (call from bootstrap).
pub fn rebuild_opaque_map(static_dir: &std::path::Path) {
    use std::collections::BTreeMap;
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    let mut stack = vec![static_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for ent in rd.flatten() {
            let p = ent.path();
            if p.is_dir() {
                let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                // skip bulky/history trees
                if matches!(name, "dist" | "admin" | "site_templates" | "node_modules") {
                    continue;
                }
                stack.push(p);
                continue;
            }
            let rel = match p.strip_prefix(static_dir) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let lower = rel.to_ascii_lowercase();
            if !(lower.ends_with(".min.js")
                || lower.ends_with(".wasm")
                || lower.ends_with(".html")
                || (lower.ends_with(".js") && !lower.ends_with(".min.js") && rel.contains("seal")))
            {
                // ship primarily min.js/wasm/html; include seal loader .js
                if !lower.ends_with(".js") {
                    continue;
                }
            }
            // skip source maps / tests
            if lower.contains(".test.") || lower.ends_with(".map") {
                continue;
            }
            let h = file_content_hash12(static_dir, &rel);
            let pub_name = opaque_public_filename(&rel, &h);
            map.insert(pub_name, rel);
        }
    }
    let map_path = static_dir.join("OPAQUE_MAP.json");
    if let Ok(body) = serde_json::to_string(&map) {
        let _ = std::fs::write(map_path, body);
    }
}

/// Resolve FE content identity (load-path version segments + diagnostics).
///
/// - `fe_version`: `fe/VERSION` — **in the public load path** since 1.0.3
///   (`/dist/v/<fe_version>/g/<asset_gen>/`): version-keyed URLs force fresh
///   fetches across releases so stale assets/404s can never be served from
///   browser/CDN cache (replaces the old flat anti-leak path).
/// - `asset_gen`: opaque 12-hex from FE tarball/race — second path segment
///   (`g/<gen>`) so FE-only rebuilds also rotate URLs; also used inside
///   **filenames** for dynamic packs when per-file hash is unavailable
pub fn resolve_fe_asset_identity(static_dir: &std::path::Path) -> (String, String) {
    let runtime_v = gr_probe_core::GR_PRODUCT_VERSION.to_string();
    let fe_version = std::fs::read_to_string(static_dir.join("VERSION"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| runtime_v.clone());
    let from_file = std::fs::read_to_string(static_dir.join("ASSET_GEN"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s.len() >= 6);
    let asset_gen = if let Some(g) = from_file {
        g.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(16)
            .collect::<String>()
    } else {
        // Derive from on-disk race (or boot) so gen still moves when FE is rsynced without ASSET_GEN.
        let candidates = [
            static_dir.join("gr.race.min.js"),
            static_dir.join("gr.boot.min.js"),
            static_dir.join("gr.min.js"),
        ];
        let mut gen = String::new();
        for p in &candidates {
            if let Ok(bytes) = std::fs::read(p) {
                let h = gr_probe_core::sha256_hex(&bytes);
                gen = h.chars().take(12).collect();
                break;
            }
        }
        if gen.is_empty() {
            // Last resort: runtime version slug (not ideal for FE-only, but stable).
            gen = format!("r{}", runtime_v.replace('.', ""));
        }
        gen
    };
    (fe_version, asset_gen)
}

/// FE bootstrap + full manifest (protocol 2) — **version-keyed opaque names**.
///
/// - pin only: fixed `/g5/gr.js` (no-store)
/// - all other assets: `/g5/dist/v/<fe_version>/g/<asset_gen>/<filehash12>.<ext>`
///   (version+gen path segments rotate URLs every release/FE rebuild so
///   browser/CDN caches can never serve stale assets or stale 404s; opaque
///   basenames stay for file identity — no `gr`/`race`/`registry` tokens,
///   no `?v=` query busting)
/// Physical files stay at logical paths under `fe/`; `OPAQUE_MAP.json` reverses public→logical.
pub fn sdk_bootstrap(
    st: &AppState,
    headers: &std::collections::HashMap<String, String>,
    query: &std::collections::HashMap<String, String>,
    static_dir: &std::path::Path,
) -> Value {
    let runtime_v = gr_probe_core::GR_PRODUCT_VERSION;
    let (fe_version, asset_gen) = resolve_fe_asset_identity(static_dir);
    // Refresh reverse map so static resolver can serve opaque URLs.
    rebuild_opaque_map(static_dir);
    // prefix = /g5 (first_party/hybrid) or absolute pv origin (dual_domain).
    let edge_opt = resolve_bootstrap_edge(st, headers, query);
    let path_prefix = bootstrap_asset_prefix(edge_opt.as_ref(), headers);
    // Version-keyed dist root (greenpng 1.0.3+): `/dist/v/<fe_version>/g/<asset_gen>/…`.
    // Every release / FE rebuild changes the URL PATH itself, so browser and CDN
    // caches can never serve a stale asset (or a stale 404) across releases —
    // the 178 v1.0.2 incident had a transient 404 cached ~1y at the CF edge under
    // the old blanket-immutable header. No `?v=` query busting (unreliable);
    // the plane strips the `v/<ver>/[g/<gen>/]` segment when resolving
    // (strip_version_route) and versioned paths are long-immutable by policy.
    let dist_root = format!(
        "{}/dist/v/{}/g/{}",
        path_prefix.trim_end_matches('/'),
        fe_version,
        asset_gen
    );
    let flat_dist = format!("{}/dist", path_prefix.trim_end_matches('/'));
    let v = runtime_v;
    let require_sealed = gr_abi::env::flag("REQUIRE_SEALED_INGEST")
        && !gr_abi::env::flag("ALLOW_PLAIN_INGEST");
    let cfg = gr_probe_store::get_runtime_cfg();
    // pin: eternal fixed name only; pass through ?grt= when the caller had it.
    let pin_url = {
        let base = format!("{path_prefix}/gr.js");
        match query.get("grt").map(|s| s.trim()).filter(|s| !s.is_empty()) {
            Some(t) => format!("{base}?grt={t}"),
            None => base,
        }
    };
    // Standard C helper — content-hash in basename under /dist/
    let h = |logical: &str| hashed_asset_url(&dist_root, static_dir, logical);
    let gl = h("gr.gl_governor.min.js");
    let micro = h("gr.micro.min.js");
    let entry = h("gr.entry.min.js");
    let pack_loader = h("pack_loader.min.js");
    let lite = h("collectors/registry.static.lite.min.js");
    let hard = h("collectors/registry.static.hard.min.js");
    let mid = h("collectors/registry.mid.min.js");
    let dense = h("collectors/registry.dense.min.js");
    let b10x = h("collectors/registry.b10x.min.js");
    let r_rt = h("collectors/registry.random.rt.min.js");
    let fe_impl = h("gr.fe_impl.min.js");
    let race = h("gr.race.min.js");
    let loader = h("gr.loader.min.js");
    // Secondary modules boot may load outside pin wave graph (true content-hash URLs).
    let privacy_guard = h("gr.privacy_guard.min.js");
    let probe_self_heal = h("probe_self_heal.min.js");
    let origin_coordinator = h("origin_coordinator.min.js");
    let probe_lifecycle = h("probe_lifecycle.min.js");
    let upload_queue = h("upload_queue.min.js");
    let storage = h("storage.min.js");
    let rpa_monitor = h("rpa_monitor.min.js");
    let sandbox_tree = h("sandbox_tree.min.js");
    let nest_frame = h("nest_frame.html");
    // pack_tokens: logical pack stem → full opaque URL (no meaningful names on the wire).
    let mut pack_tokens = serde_json::Map::new();
    if let Ok(body) = std::fs::read_to_string(static_dir.join("OPAQUE_MAP.json")) {
        if let Ok(map) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&body) {
            for (pub_name, logical_v) in map {
                let logical = match logical_v.as_str() {
                    Some(s) => s,
                    None => continue,
                };
                if let Some(rest) = logical.strip_prefix("collectors/") {
                    if let Some(stem) = rest.strip_suffix(".min.js") {
                        pack_tokens.insert(
                            stem.to_string(),
                            json!(format!(
                                "{}/{}",
                                dist_root.trim_end_matches('/'),
                                pub_name
                            )),
                        );
                    }
                }
            }
        }
    }
    let pack_url_template = format!("{dist_root}/{{pack_token}}.min.js");
    // Seal wasm/loader + nest_frame: content-hash in filename (no version path).
    let mut seal_meta = gr_probe_core::seal_v2_public_meta();
    if let Some(obj) = seal_meta.as_object_mut() {
        obj.insert(
            "wasm_url".into(),
            json!(h("gr_seal_v2.wasm")),
        );
        obj.insert(
            "wasm_url_flat".into(),
            json!(h("gr_seal_v2.wasm")),
        );
        obj.insert(
            "loader_url".into(),
            json!(h("gr_seal_v2_loader.js")),
        );
        obj.insert(
            "loader_url_flat".into(),
            json!(h("gr_seal_v2_loader.js")),
        );
    }
    let mut out = json!({
        "ok": true,
        "protocol": 2,
        // Runtime binary / protocol version (install-runtime). JSON only — not in asset URLs.
        "product_version": v,
        "version": v,
        // FE content identity (install-fe). Diagnostics only — not in asset URLs.
        "fe_version": fe_version,
        "asset_gen": asset_gen,
        "fe_impl_version": fe_version,
        // Compatibility fields (protocol 1 consumers / legacy loader)
        "entry_url": entry.clone(),
        "micro_url": micro.clone(),
        "gl_governor_url": gl.clone(),
        "asset_base": dist_root.clone(),
        "asset_base_flat": flat_dist.clone(),
        "asset_route": "opaque_content_hash",
        // Eternal pin only fixed name; loader is content-hashed (not legacy flat).
        "pin_url": pin_url,
        "loader_url": loader.clone(),
        "loader_url_legacy": loader.clone(),
        "assets": {
            "pin": pin_url,
            "loader": loader,
            "entry": entry.clone(),
            "micro": micro.clone(),
            "race": race.clone(),
            "gl_governor": gl.clone(),
            "lite": lite.clone(),
            "hard": hard.clone(),
            "mid": mid.clone(),
            "dense": dense.clone(),
            "b10x": b10x.clone(),
            "pack_loader": pack_loader.clone(),
            "fe_impl": fe_impl.clone(),
            "random_rt": r_rt.clone(),
            "privacy_guard": privacy_guard,
            "probe_self_heal": probe_self_heal,
            "origin_coordinator": origin_coordinator,
            "probe_lifecycle": probe_lifecycle,
            "upload_queue": upload_queue,
            "storage": storage,
            "rpa_monitor": rpa_monitor,
            "sandbox_tree": sandbox_tree,
            "nest_frame": nest_frame,
        },
        // protocol 2: ordered load graph (pin executes by wave; same wave parallel)
        // pack_loader is already inlined in entry (and race). Keep assets.pack_loader for
        // heal/fallback loads, but do not pin-wave double-fetch it.
        "scripts": [
            {
                "id": "gl_governor",
                "url": gl,
                "wave": 0,
                "async": true,
                "priority": "high",
                "blocking": false
            },
            {
                "id": "micro",
                "url": micro,
                "wave": 0,
                "async": true,
                "priority": "high",
                "blocking": false
            },
            {
                "id": "entry",
                "url": entry,
                "wave": 1,
                "async": true,
                "deps": ["micro"],
                "priority": "high",
                "blocking": false
            },
            {
                "id": "lite",
                "url": lite,
                "wave": 1,
                "async": true,
                "deps": ["entry"]
            }
        ],
        "layers": {
            "hard": hard,
            "mid": mid,
            "dense": dense,
            "b10x": b10x,
            "lite": lite.clone(),
            "r_rt": r_rt,
            "pack_loader": pack_loader.clone(),
            "pack_url_template": pack_url_template,
            "pack_tokens": pack_tokens
        },
        "preload": [
            entry.clone(),
            lite.clone(),
            hard.clone()
        ],
        "brain_hints": {
            "early_static": [
                "B0_bootstrap",
                "B1_conflict",
                "B2_hardware",
                "B3_system",
                "B12_anti_camouflage"
            ],
            "upload_concurrency": 12,
            "mid_ramp_concurrency": 16,
            "max_light_concurrent": 2,
            "resource_bus": true,
            "prefetch_hard_after_ms": 0
        },
        "upload_policy": {
            "require_sealed_ingest": require_sealed,
            "seal_mode": "session_grant_v1",
            "ingest_sealed_path": "/v1/ingest/sealed"
        },
        "require_sealed_ingest": require_sealed,
        "seal_mode": "session_grant_v1",
        "ingest_sealed_path": "/v1/ingest/sealed",
        "config_version": cfg.version,
        "policy": {
            "cycle_cool_ms": cfg.cycle_cool_ms,
            "return_identity_idle_ms": cfg.return_identity_idle_ms,
            "rpa_idle_analyze_ms": cfg.rpa_idle_analyze_ms,
            "analyze_idle_upload_ms": cfg.analyze_idle_upload_ms,
        },
        "seal_v2": seal_meta,
        "note": "protocol 2 Standard-C opaque: pin fixed name only; all other assets /dist/<contenthash>.ext (no meaningful filenames)",
    });
    // Attach site edge topology when resolvable (panel SSOT for pv/gv).
    if let Some(edge) = edge_opt {
        if let Some(obj) = out.as_object_mut() {
            if let Some(cf) = edge.get("cookie_fields").cloned() {
                obj.insert("cookie_fields".into(), cf.clone());
            }
            if let Some(tok) = edge.get("embed_token").and_then(|v| v.as_str()) {
                if !tok.is_empty() {
                    obj.insert("embed_token".into(), json!(tok));
                }
            }
            obj.insert("edge".into(), edge);
        }
    }
    out
}

/// Asset URL prefix for bootstrap (where FE **loads** packs/wasm — NOT upload apiBase).
///
/// Axes:
/// - fe_load=first_party → `/g5` (www nginx; page-relative)
/// - fe_load=pv → `https://pv.host` (static under `/dist`, not `/g5/dist`)
/// - Never use upload api_base=https://gv for assets (causes gv+/g5/dist MIME JSON 404)
fn bootstrap_asset_prefix(
    edge: Option<&Value>,
    headers: &std::collections::HashMap<String, String>,
) -> String {
    if let Some(edge) = edge {
        // Prefer script_base (fe_load), then explicit fe_load+pv_base — never upload api_base for packs.
        let script = edge
            .get("script_base")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .trim_end_matches('/');
        if !script.is_empty() {
            if script.starts_with("http://") || script.starts_with("https://") {
                // Absolute pv/cdn: strip accidental /g5 so assets are /dist on that host
                let s = script.trim_end_matches('/').trim_end_matches("/g5");
                return s.to_string();
            }
            if script.starts_with('/') {
                return script.to_string();
            }
        }
        let fe = edge
            .get("fe_load")
            .and_then(|v| v.as_str())
            .unwrap_or("first_party");
        if fe == "pv" {
            let pv = edge
                .get("pv_base")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .trim_end_matches('/')
                .trim_end_matches("/g5");
            if pv.starts_with("http://") || pv.starts_with("https://") {
                return pv.to_string();
            }
        }
        // first_party load → always page-relative /gr even when api_base is https://gv
        // (canonical asset prefix since the GR rename; /g5 stays accepted at the edge)
        if fe == "first_party" || fe.is_empty() {
            return "/gr".into();
        }
    }
    // Host is pv.* → assets at host root /dist (not /gr/dist)
    if let Some(host) = headers
        .get("host")
        .or_else(|| headers.get("Host"))
        .map(|s| s.split(':').next().unwrap_or(s).to_ascii_lowercase())
        .filter(|s| !s.is_empty())
    {
        if host.starts_with("pv.") {
            let proto = headers
                .get("x-forwarded-proto")
                .or_else(|| headers.get("X-Forwarded-Proto"))
                .map(|s| s.as_str())
                .filter(|s| *s == "http" || *s == "https")
                .unwrap_or("https");
            return format!("{proto}://{host}");
        }
        // Host is gv.* → still default /gr so first_party clients (bootstrap via gv) load packs on www
        // Callers with site_id get script_base from edge above.
    }
    "/gr".into()
}

fn resolve_bootstrap_edge(
    st: &AppState,
    headers: &std::collections::HashMap<String, String>,
    query: &std::collections::HashMap<String, String>,
) -> Option<Value> {
    let admin = st.admin.as_ref()?;
    let mut site: Option<Value> = None;
    if let Some(sid) = query
        .get("site_id")
        .or_else(|| query.get("siteId"))
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        site = admin.db.get_site(sid).ok().flatten();
    }
    if site.is_none() {
        if let Some(tok) = query
            .get("grt")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            site = admin.db.find_site_by_embed_token(tok).ok().flatten();
        }
    }
    if site.is_none() {
        let host = headers
            .get("host")
            .or_else(|| headers.get("Host"))
            .map(|s| s.split(':').next().unwrap_or(s).to_ascii_lowercase())
            .filter(|s| !s.is_empty());
        if let Some(h) = host {
            if let Ok(Some(dom)) = admin.db.get_domain_by_hostname(&h) {
                if let Some(sid) = dom.get("site_id").and_then(|v| v.as_str()) {
                    site = admin.db.get_site(sid).ok().flatten();
                }
            }
        }
    }
    site.as_ref().map(crate::admin::sdk::resolve_edge_bases)
}

/// Public load-balancer / FE DNS-timing probe response.
///
/// **Must not** expose DB DSN, queue internals, device_id recipe, or algorithm maps.
/// FE B9 only measures RTT and does not parse the body.
pub fn health_public(_st: &AppState) -> Value {
    json!({
        "ok": true,
        "service": "gr-service",
    })
}

/// Liveness: process is up. Do not check dependencies.
pub fn health_livez(_st: &AppState) -> Value {
    json!({
        "ok": true,
        "status": "live",
        "service": "gr-service",
    })
}

/// Readiness: shared store answers. Used by drain/LB to stop new traffic.
/// While `st.draining` is set the node reports 503 so the LB pulls it out of
/// rotation before the process exits.
pub fn health_readyz(st: &AppState) -> Value {
    if st.draining.load(Ordering::Relaxed) {
        return json!({
            "ok": false,
            "status": "draining",
            "db": st.store.backend_name(),
        });
    }
    match st.store.pending_analyze_job_count() {
        Ok(_) => json!({
            "ok": true,
            "status": "ready",
            "db": st.store.backend_name(),
        }),
        Err(_) => json!({
            "ok": false,
            "status": "not_ready",
            "db": st.store.backend_name(),
        }),
    }
}

/// Ops/admin detail health — queue depth, backends, algorithm docs.
/// Never return raw DB DSN; only backend kind names.
pub fn health_detail(st: &AppState) -> Value {
    let pending = st.store.pending_analyze_job_count().unwrap_or(-1);
    let queue = st.store.analyze_queue_stats().unwrap_or_else(|_| {
        json!({
            "pending": pending,
            "due_now": -1,
            "locked": -1,
            "oldest_lag_ms": -1,
            "backend": st.store.backend_name(),
        })
    });
    let (biz_store, biz_backend) = match st.admin.as_ref() {
        Some(a) => (true, a.biz.backend_name()),
        None => (false, "none"),
    };
    // Strip any accidental DSN/password from backend_label → kind only.
    let db_kind = st.store.backend_name();
    json!({
        "ok": true,
        "service": "gr-service",
        "role": format!("{:?}", st.role).to_ascii_lowercase(),
        "worker_id": st.worker_id,
        "product_version": gr_probe_core::GR_PRODUCT_VERSION,
        "version": gr_probe_core::GR_PRODUCT_VERSION,
        // Never emit connection strings on any health surface.
        "db": db_kind,
        "backend": db_kind,
        "biz_store": biz_store,
        "biz_backend": biz_backend,
        "soft_v2_ready": st.soft_v2_ready,
        "pending_analyze_jobs": pending,
        "analyze_queue": queue,
        "analyze_runs": st.analyze_runs.load(Ordering::Relaxed),
        "analyze_debounce_ms": ANALYZE_DEBOUNCE_MS,
        "analyze_schedule": analyze_schedule_policy_json(),
        "probe_field_priority": probe_field_priority_json(),
        "device_segment_composition": gr_probe_core::device_segment_composition_json(),
        "per_batch_analyze": false,
        "metrics": {
            "analyze_queue_pending": queue.get("pending").cloned().unwrap_or(json!(pending)),
            "analyze_queue_due_now": queue.get("due_now").cloned().unwrap_or(json!(-1)),
            "analyze_queue_locked": queue.get("locked").cloned().unwrap_or(json!(-1)),
            "analyze_oldest_lag_ms": queue.get("oldest_lag_ms").cloned().unwrap_or(json!(-1)),
            "analyze_runs": st.analyze_runs.load(Ordering::Relaxed),
        },
    })
}

/// Backward-compat name: public surface only.
pub fn health(st: &AppState) -> Value {
    health_public(st)
}

/// Cookie names in this blocklist are never captured even if a site owner
/// allowlists them (defense in depth; auth-ish values must not leak to SDKs).
const COOKIE_CAPTURE_BLOCKLIST: [&str; 9] = [
    "password", "passwd", "pwd", "jwt", "secret", "credential", "token", "apikey", "api_key",
];
/// Max allowlisted cookie fields captured per session (site allowlist cap).
const COOKIE_CAPTURE_MAX_FIELDS: usize = 16;
/// Max length of a captured cookie value (truncation, not rejection).
const COOKIE_CAPTURE_MAX_VALUE_LEN: usize = 256;

/// Allowlist name validation: bare printable cookie names only.
fn cookie_name_ok(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
    {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    !COOKIE_CAPTURE_BLOCKLIST
        .iter()
        .any(|bad| lower.contains(bad))
}

/// Parse a Cookie header value into (name, value) pairs; first occurrence wins.
fn cookie_pairs(cookie_header: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for part in cookie_header.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (k, v) = match part.split_once('=') {
            Some((k, v)) => (k.trim(), v.trim().trim_matches('"')),
            None => (part, ""),
        };
        if k.is_empty() {
            continue;
        }
        out.push((k.to_string(), v.to_string()));
    }
    out
}

/// Site cookie allowlist from the admin console (sites table, cookie_fields).
fn site_cookie_allowlist(st: &AppState, site_id: &str) -> Vec<String> {
    let Some(admin) = st.admin.as_ref() else {
        return Vec::new();
    };
    let Ok(Some(site)) = admin.db.get_site(site_id) else {
        return Vec::new();
    };
    site.get("cookie_fields")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .filter(|s| cookie_name_ok(s))
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Filter parsed cookie pairs against the site allowlist (pure; ordered by
/// allowlist, bounded, value-truncated — never blind-copy a raw header).
fn capture_allowlisted_cookies(
    allow: &[String],
    pairs: &[(String, String)],
) -> Value {
    let mut out = serde_json::Map::new();
    let mut seen = std::collections::HashSet::new();
    for name in allow {
        if out.len() >= COOKIE_CAPTURE_MAX_FIELDS {
            break;
        }
        if !seen.insert(name.clone()) {
            continue;
        }
        if let Some((_, v)) = pairs.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)) {
            if v.is_empty() {
                continue;
            }
            let truncated: String = v.chars().take(COOKIE_CAPTURE_MAX_VALUE_LEN).collect();
            out.insert(name.clone(), json!(truncated));
        }
    }
    Value::Object(out)
}

/// Capture allowlisted cookies. Prefer body/`meta.cookie_fields` (FE reads
/// `document.cookie` on pv or same-origin proxy). Cookie header is a
/// same-origin compatibility fallback only — gv never sees www cookies.
fn capture_cookie_fields_from_value(st: &AppState, site_id: &str, raw: &Value) -> Value {
    if site_id.is_empty() {
        return json!({});
    }
    let allow = site_cookie_allowlist(st, site_id);
    if allow.is_empty() {
        return json!({});
    }
    let pairs: Vec<(String, String)> = match raw {
        Value::Object(map) => map
            .iter()
            .filter_map(|(k, v)| {
                let val = v.as_str().map(|s| s.to_string()).or_else(|| {
                    if v.is_null() {
                        None
                    } else {
                        Some(v.to_string().trim_matches('"').to_string())
                    }
                })?;
                Some((k.clone(), val))
            })
            .collect(),
        Value::Array(arr) => arr
            .iter()
            .filter_map(|v| {
                let obj = v.as_object()?;
                let k = obj.get("name").and_then(|x| x.as_str())?;
                let val = obj.get("value").and_then(|x| x.as_str()).unwrap_or("");
                Some((k.to_string(), val.to_string()))
            })
            .collect(),
        _ => Vec::new(),
    };
    capture_allowlisted_cookies(&allow, &pairs)
}

fn capture_cookie_fields_from_header(
    st: &AppState,
    site_id: &str,
    headers: &HashMap<String, String>,
) -> Value {
    if site_id.is_empty() {
        return json!({});
    }
    let allow = site_cookie_allowlist(st, site_id);
    if allow.is_empty() {
        return json!({});
    }
    let Some(raw) = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("cookie"))
        .map(|(_, v)| v.as_str())
    else {
        return json!({});
    };
    capture_allowlisted_cookies(&allow, &cookie_pairs(raw))
}

fn cookie_value_nonempty(v: &Value) -> bool {
    v.as_object().map(|o| !o.is_empty()).unwrap_or(false)
}

/// True when the session meta already holds captured cookie fields.
fn meta_has_cookie_fields(meta: &Value) -> bool {
    meta.get("cookie_fields")
        .and_then(|v| v.as_object())
        .map(|o| !o.is_empty())
        .unwrap_or(false)
}

pub fn open_session(
    st: &AppState,
    body: OpenBody,
    headers: &HashMap<String, String>,
) -> AppResult {
    let inject_path = body.inject_path.clone();
    let mut meta = merge_inject_meta(body.meta, inject_path.clone());
    // Authoritative product version (unified FE+core SSOT). Cool is scoped to this tag.
    if let Some(obj) = meta.as_object_mut() {
        obj.insert(
            "product_version".into(),
            json!(gr_probe_core::GR_PRODUCT_VERSION),
        );
        obj.insert("version".into(), json!(gr_probe_core::GR_PRODUCT_VERSION));
    }
    let host_site = resolve_site_id_from_host(st, headers);
    let mut site_id = host_site;
    if site_id.is_empty() && !is_prod_env() {
        site_id = body
            .site_id
            .clone()
            .or_else(|| {
                meta.get("site_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_default();
    }
    let result_policy = gr_probe_core::load_panel_policy(panel_data_dir(st).as_deref())
        .resolve_result_policy(if site_id.is_empty() {
            None
        } else {
            Some(site_id.as_str())
        });
    if !site_collect_enabled(st, &site_id, headers) {
        return Err(reject_collect_disabled());
    }
    if !site_id.is_empty() {
        if let Some(obj) = meta.as_object_mut() {
            obj.insert(
                "result_policy".into(),
                serde_json::to_value(&result_policy).unwrap_or_else(|_| json!({})),
            );
            obj.insert("site_id".into(), json!(site_id.clone()));
        }
    }
    {
        let existing = meta.get("business_context").cloned();
        let ctx = gr_probe_core::canonicalize_business_context(&meta, existing.as_ref());
        if let Some(obj) = meta.as_object_mut() {
            obj.insert("business_context".into(), ctx);
        }
    }
    if let Some((status, err)) = crate::embed_gate::check_open_token(
        st,
        &site_id,
        body.embed_token.as_deref(),
        None,
    ) {
        return Err(ApiError(status, err.get("error").and_then(|v| v.as_str()).unwrap_or("embed_token").into()));
    }
    // Cookie capture: body / meta.cookie_fields first (FE document.cookie),
    // Cookie header only as same-origin compatibility (never the spec path).
    if !site_id.is_empty() {
        let from_body = body
            .cookie_fields
            .as_ref()
            .map(|v| capture_cookie_fields_from_value(st, &site_id, v))
            .unwrap_or_else(|| json!({}));
        let from_meta = meta
            .get("cookie_fields")
            .map(|v| capture_cookie_fields_from_value(st, &site_id, v))
            .unwrap_or_else(|| json!({}));
        let captured = if cookie_value_nonempty(&from_body) {
            from_body
        } else if cookie_value_nonempty(&from_meta) {
            from_meta
        } else {
            capture_cookie_fields_from_header(st, &site_id, headers)
        };
        if cookie_value_nonempty(&captured) {
            if let Some(obj) = meta.as_object_mut() {
                obj.insert("cookie_fields".into(), captured);
            }
        }
    }
    // Multi-source facet: FE open hint + vtid (not unconditional js).
    let fe_hint = meta
        .get("fe")
        .and_then(|v| v.as_str())
        .map(|s| s.contains("gr") || s.contains("boot") || !s.is_empty())
        .unwrap_or(false)
        || meta.get("collectors").is_some()
        || inject_path
            .as_deref()
            .map(|s| matches!(s, "nginx" | "app" | "cf_worker" | "cloudflare"))
            .unwrap_or(false)
        || meta
            .get("identity_class")
            .and_then(|v| v.as_str())
            == Some("js");
    let has_vtid = body
        .visitor_terminal_id
        .as_ref()
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    let backend_js = meta
        .get("client_class")
        .and_then(|v| v.as_str())
        == Some("js")
        || meta.get("source").and_then(|v| v.as_str()) == Some("backend_sdk");
    let ua_open = meta
        .get("user_agent")
        .or_else(|| meta.get("ua"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    meta = crate::admin::facet::merge_visit_class(
        Some(meta),
        &crate::admin::facet::ClassInputs {
            ua: ua_open,
            is_pixel: false,
            has_fe_main: false,
            fe_open_hint: fe_hint || has_vtid, // FE always continues vt → treat as js path
            has_vtid,
            backend_claims_js: backend_js,
        },
    );
    let facet = meta
        .get("visitor_facet")
        .and_then(|v| v.as_str())
        .unwrap_or("browser")
        .to_string();
    let robot_name_open = meta.get("robot_name").cloned().unwrap_or(Value::Null);
    // force_identity from body or meta or classic force_reprobe — kills ticket + cool skip.
    let force_identity_req = body.force_identity.unwrap_or(false)
        || meta
            .get("force_identity")
            .or_else(|| meta.get("force_identity_probe"))
            .or_else(|| meta.get("force_reprobe"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    if force_identity_req {
        if let Some(obj) = meta.as_object_mut() {
            obj.insert("force_identity".into(), json!(true));
            obj.insert("force_identity_probe".into(), json!(true));
            obj.remove("session_ticket");
            obj.insert("skip_session_probe".into(), json!(false));
        }
    }
    let current_pv = gr_probe_store::product_version_from_meta(&meta);
    // Optional storage_bind validation (tampered cool → force re-probe)
    if let Some(bind) = body.storage_bind.as_deref().filter(|s| !s.is_empty()) {
        let now_b = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        match gr_probe_core::validate_storage_bind(&st.challenge_secret, bind, now_b, force_identity_req) {
            Ok(v) => {
                if let Some(obj) = meta.as_object_mut() {
                    obj.insert("storage_bind_ok".into(), json!(true));
                    if let Some(pv) = v.get("product_version").and_then(|x| x.as_str()) {
                        if !current_pv.is_empty() && !pv.is_empty() && pv != current_pv {
                            obj.insert("storage_bind_version_mismatch".into(), json!(true));
                            obj.insert("force_identity".into(), json!(true));
                        }
                    }
                }
            }
            Err(reason) => {
                if let Some(obj) = meta.as_object_mut() {
                    obj.insert("storage_bind_ok".into(), json!(false));
                    obj.insert("storage_bind_err".into(), json!(reason));
                    if reason == "sig_mismatch" || reason == "expired" {
                        obj.insert("force_identity".into(), json!(true));
                    }
                }
                let _ = st.store.insert_ops_server_event(json!({
                    "code": "storage_bind_invalid",
                    "severity": "warn",
                    "stage": "open",
                    "session_id": body.session_id.clone().unwrap_or_default(),
                    "visitor_terminal_id": body.visitor_terminal_id.clone().unwrap_or_default(),
                    "product_version": current_pv,
                    "detail_json": {"reason": reason},
                }));
            }
        }
    }
    let force_identity_req = force_identity_req
        || meta
            .get("force_identity")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let mut skip_session = false;
    if !force_identity_req {
        if let Some(ticket) = body.session_ticket.as_ref() {
            // Validate against optional session_id / cycle_id in body/ticket
            let sid_hint = body
                .session_id
                .as_deref()
                .or_else(|| ticket.get("session_id").and_then(|v| v.as_str()))
                .or_else(|| ticket.get("cycle_id").and_then(|v| v.as_str()))
                .unwrap_or("");
            if gr_probe_core::validate_session_ticket_ex(
                ticket,
                sid_hint,
                None,
                None,
                if current_pv.is_empty() {
                    None
                } else {
                    Some(current_pv.as_str())
                },
                force_identity_req,
            ) {
                skip_session = true;
                if let Some(obj) = meta.as_object_mut() {
                    obj.insert("session_ticket".into(), ticket.clone());
                    obj.insert("skip_session_probe".into(), json!(true));
                }
            } else {
                let _ = st.store.insert_ops_server_event(json!({
                    "code": "ticket_rejected",
                    "severity": "info",
                    "stage": "open",
                    "session_id": sid_hint,
                    "visitor_terminal_id": body.visitor_terminal_id.clone().unwrap_or_default(),
                    "product_version": current_pv,
                    "detail_json": {"reason": "validate_failed_or_version_or_silicon"},
                }));
            }
        }
    }
    // cycle_id is the probe evidence bag (replaces session_id semantics on hot path).
    // Accept session_id body field as cycle hint for wire compat.
    let cycle_hint = body.session_id.clone();
    // Open contract: FE must send visitor_terminal_id. If missing, server mints and
    // surfaces it so FE dual-writes (micro ≡ storage). Never leave open without a vt.
    let vt_missing = body
        .visitor_terminal_id
        .as_ref()
        .map(|s| s.trim().is_empty())
        .unwrap_or(true);
    let mut open_vt = body.visitor_terminal_id.clone().filter(|s| !s.trim().is_empty());
    if open_vt.is_none() {
        let minted = format!(
            "vt_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| format!("{:x}", d.as_millis()))
                .unwrap_or_else(|_| "0".into())
        );
        let _ = st.store.insert_ops_server_event(json!({
            "code": "vt_mint",
            "severity": "warn",
            "stage": "open",
            "session_id": body.session_id.clone().unwrap_or_default(),
            "visitor_terminal_id": minted,
            "product_version": current_pv,
            "detail_json": {
                "reason": "server_open_missing",
                "vt_mint_reason": "server_open_missing",
            },
        }));
        open_vt = Some(minted);
    }
    let vt_body = open_vt.clone();
    let out = st
        .store
        .open_cycle(cycle_hint, open_vt, Some(meta))?;
    if vt_missing {
        // already logged mint; mark response path for FE adopt
        let _ = st.store.insert_ops_server_event(json!({
            "code": "open_vt_forced",
            "severity": "info",
            "stage": "open",
            "session_id": out
                .get("cycle_id")
                .or_else(|| out.get("session_id"))
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            "visitor_terminal_id": vt_body.clone().unwrap_or_default(),
            "product_version": current_pv,
            "detail_json": {"contract": "open_requires_vt"},
        }));
    }
    let sid = out
        .get("cycle_id")
        .or_else(|| out.get("session_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    // Robots fast lane (1.0.10): mark UA-declared crawler sessions at open —
    // later FE ingests for these (headless automation) skip L1/L3/arms.
    if robot_fastlane_active(st) && facet == "robots" {
        mark_robot_session(st, &sid);
        try_robot_early_result(st, &sid, robot_name_open.as_str(), "open");
    }
    let phase = out
        .get("phase")
        .and_then(|v| v.as_str())
        .unwrap_or("active");
    let mut skip_identity = !force_identity_req
        && (skip_session
            || out
                .get("skip_identity_probe")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            || phase == "cool");
    // Version / force must never skip identity even if store returned cool phase.
    if force_identity_req {
        skip_identity = false;
    }
    let received = if skip_identity {
        Vec::new()
    } else {
        st.store.list_received_batches(&sid).unwrap_or_default()
    };
    let window = st.store.session_window(&sid).ok();
    // H16/PoHW: signed challenge seed (HMAC-SHA256 digest + TTL) for B20.
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let challenge_seed_ttl_ms = gr_probe_core::DEFAULT_CHALLENGE_TTL_MS;
    let challenge_mat = gr_probe_core::issue_challenge_seed(
        &sid,
        now_ms,
        challenge_seed_ttl_ms,
        &st.challenge_secret,
    );
    let challenge_seed = challenge_mat
        .get("challenge_seed")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let challenge_seed_exp_ms = challenge_mat.get("challenge_seed_exp_ms").cloned().unwrap_or(json!(0));
    let challenge_seed_sig = challenge_mat
        .get("challenge_seed_sig")
        .cloned()
        .unwrap_or(json!(""));
    let vt_id = out
        .get("visitor_terminal_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or(vt_body)
        .unwrap_or_default();
    if !site_id.is_empty() && !vt_id.is_empty() {
        let biz = st.admin.as_ref().map(|a| a.biz.clone());
        let page_host = request_hostname(headers);
        crate::admin::biz_store::try_record_visit(
            &biz,
            crate::admin::biz_store::VisitUpsert {
                site_id: site_id.clone(),
                visitor_terminal_id: vt_id.clone(),
                visitor_facet: facet.clone(),
                session_id: sid.clone(),
                page_host,
                ua_hash: String::new(),
                summary: json!({
                    "source": "session_open",
                    "phase": phase,
                    "visitor_facet": facet,
                    "robot_name": robot_name_open,
                }),
                event: Some("open_ok".into()),
                event_detail: json!({"session_id": sid, "visitor_facet": facet}),
            },
        );
    }
    let probe_status = if !sid.is_empty() {
        cycle_probe_status_snapshot(st, &sid)
    } else {
        json!({})
    };
    let halt_open = skip_identity
        || probe_status
            .get("halt_uploads")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let business_state = if skip_identity {
        json!("identity_complete_cool")
    } else {
        probe_status
            .get("business_state")
            .cloned()
            .unwrap_or(json!("probing"))
    };
    // Build response via Map inserts to avoid json! recursion_limit on nested objects.
    // Thin last_identity_result (full analyze can be 200KB+; duplicated under session → 500KB open).
    let thin_lir = out
        .get("last_identity_result")
        .map(thin_identity_result)
        .unwrap_or(Value::Null);
    let mut session_out = out.clone();
    if let Some(obj) = session_out.as_object_mut() {
        if obj.contains_key("last_identity_result") {
            obj.insert("last_identity_result".into(), thin_lir.clone());
        }
        // Drop heavy meta tails if present (battle_log_history etc. not needed on open).
        if let Some(meta) = obj.get_mut("meta").and_then(|m| m.as_object_mut()) {
            meta.remove("battle_log");
            meta.remove("battle_log_history");
            meta.remove("belief");
        }
    }
    let mut resp = serde_json::Map::new();
    resp.insert("ok".into(), json!(true));
    resp.insert("session".into(), session_out);
    resp.insert("session_id".into(), json!(sid.clone()));
    resp.insert(
        "cycle_id".into(),
        out.get("cycle_id").cloned().unwrap_or(json!(sid.clone())),
    );
    resp.insert("visitor_terminal_id".into(), json!(vt_id));
    resp.insert("visitor_facet".into(), json!(facet));
    resp.insert(
        "site_id".into(),
        if site_id.is_empty() {
            Value::Null
        } else {
            json!(site_id)
        },
    );
    resp.insert("phase".into(), json!(phase));
    resp.insert("skip_session_probe".into(), json!(skip_identity));
    resp.insert("skip_identity_probe".into(), json!(skip_identity));
    // Server authority for version-scoped re-probe (FE must not rely on sticky cycle cookies).
    let force_identity = force_identity_req
        || (!skip_identity
            && (out
                .get("force_identity_probe")
                .and_then(|v| v.as_bool())
                .unwrap_or(true)
                || out
                    .get("client_hint_superseded")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                || out
                    .get("reprobe_reason")
                    .and_then(|v| v.as_str())
                    .map(|s| !s.is_empty())
                    .unwrap_or(false)
                || out
                    .pointer("/meta/cool_invalidated_reason")
                    .and_then(|v| v.as_str())
                    .map(|s| !s.is_empty())
                    .unwrap_or(false)));
    resp.insert("force_identity_probe".into(), json!(force_identity));
    // Signed storage bind for FE anti-tamper cool/VT (HttpOnly not available to JS).
    let cool_until_ms = out
        .get("cool_until_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let now_bind = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let site_for_cool = out
        .get("site_id")
        .and_then(|v| v.as_str())
        .or_else(|| body.site_id.as_deref())
        .unwrap_or("");
    let bind_cool = if cool_until_ms > now_bind {
        cool_until_ms
    } else {
        now_bind + gr_probe_store::cycle_cool_ms_for_site(site_for_cool)
    };
    let storage_bind = gr_probe_core::issue_storage_bind(
        &st.challenge_secret,
        &vt_id,
        bind_cool,
        &current_pv,
        now_bind,
        bind_cool,
    );
    resp.insert("storage_bind".into(), storage_bind);
    resp.insert("debug".into(), json!(false));
    resp.insert(
        "reprobe_reason".into(),
        out.get("reprobe_reason")
            .cloned()
            .or_else(|| {
                out.pointer("/meta/cool_invalidated_reason")
                    .cloned()
            })
            .unwrap_or(Value::Null),
    );
    resp.insert(
        "client_hint_superseded".into(),
        out.get("client_hint_superseded")
            .cloned()
            .unwrap_or(json!(false)),
    );
    resp.insert(
        "superseded_cycle_id".into(),
        out.get("superseded_cycle_id").cloned().unwrap_or(Value::Null),
    );
    // One-VT-one-active-cycle: FE adopts server bag when client mint diverged.
    resp.insert(
        "converged_to_active".into(),
        out.get("converged_to_active")
            .cloned()
            .unwrap_or(json!(false)),
    );
    resp.insert(
        "resumed".into(),
        out.get("resumed").cloned().unwrap_or(json!(false)),
    );
    resp.insert(
        "need_hard_anchor".into(),
        out.get("need_hard_anchor")
            .cloned()
            .unwrap_or(json!(false)),
    );
    resp.insert(
        "client_hint_honored".into(),
        out.get("client_hint_honored")
            .cloned()
            .unwrap_or(Value::Null),
    );
    resp.insert("halt_uploads".into(), json!(halt_open));
    resp.insert("cycle_probe_status".into(), probe_status);
    resp.insert("business_state".into(), business_state);
    resp.insert(
        "page_probe_required".into(),
        out.get("page_probe_required")
            .cloned()
            .unwrap_or(json!(true)),
    );
    resp.insert("cool_until_ms".into(), out.get("cool_until_ms").cloned().unwrap_or(Value::Null));
    resp.insert(
        "product_version".into(),
        out.get("product_version")
            .cloned()
            .unwrap_or_else(|| json!(gr_probe_core::GR_PRODUCT_VERSION)),
    );
    resp.insert(
        "product_version_last".into(),
        out.get("product_version_last").cloned().unwrap_or(Value::Null),
    );
    resp.insert(
        "cycle_expires_ms".into(),
        out.get("cycle_expires_ms").cloned().unwrap_or(Value::Null),
    );
    resp.insert("last_identity_result".into(), thin_lir);
    resp.insert("received_batches".into(), json!(received));
    // session_window can be large; omit on cool skip (FE does not need it).
    resp.insert(
        "session_window".into(),
        if skip_identity {
            Value::Null
        } else {
            window.unwrap_or(Value::Null)
        },
    );
    resp.insert("challenge_seed".into(), json!(challenge_seed));
    resp.insert("challenge_seed_ttl_ms".into(), json!(challenge_seed_ttl_ms));
    resp.insert("challenge_seed_exp_ms".into(), challenge_seed_exp_ms);
    resp.insert("challenge_seed_sig".into(), challenge_seed_sig);
    resp.insert("challenge_algo".into(), json!(gr_probe_core::CHALLENGE_ALGO));
    // Browser sealed upload: epoch-bound v2 grant (never master SEAL_SECRET).
    let require_sealed = gr_abi::env::flag("REQUIRE_SEALED_INGEST")
        && !gr_abi::env::flag("ALLOW_PLAIN_INGEST");
    let fe_epoch = gr_probe_core::GR_PRODUCT_VERSION;
    let mut seal_grant = if gr_probe_core::seal_require_v2() {
        gr_probe_core::issue_session_seal_grant_v2(
            st.seal_secret.as_bytes(),
            &sid,
            now_ms as u64,
            gr_probe_core::SESSION_SEAL_TTL_MS,
            fe_epoch,
        )
    } else {
        // Lab break-glass: legacy v1 grant (explicit GR_SEAL_REQUIRE_V2=0)
        gr_probe_core::issue_session_seal_grant(
            st.seal_secret.as_bytes(),
            &sid,
            now_ms as u64,
            gr_probe_core::SESSION_SEAL_TTL_MS,
        )
    };
    // Bind open challenge seed into grant so FE/WASM challenge_bind is non-empty & session-specific.
    if let Some(obj) = seal_grant.as_object_mut() {
        obj.insert("challenge_seed".into(), json!(challenge_seed));
        obj.insert(
            "challenge_bind_hint".into(),
            json!(gr_probe_core::compute_challenge_bind(
                &sid,
                &challenge_seed,
                fe_epoch
            )),
        );
    }
    resp.insert("seal_grant".into(), seal_grant);
    resp.insert("require_sealed_ingest".into(), json!(require_sealed));
    resp.insert("seal_v2".into(), gr_probe_core::seal_v2_public_meta());
    resp.insert(
        "result_policy".into(),
        serde_json::to_value(&result_policy).unwrap_or_else(|_| json!({})),
    );
    resp.insert(
        "capabilities".into(),
        json!({
            "tier": "T0",
            "pending": !skip_identity,
            "note": "filled after first FE/static ingest via project_capabilities"
        }),
    );
    let rt = gr_probe_store::get_runtime_cfg();
    let fe_pol = gr_probe_store::fe_retry_policy_json(&rt);
    resp.insert(
        "policy".into(),
        json!({
            "cycle_cool_ms": rt.cycle_cool_ms,
            "cycle_incomplete_ms": rt.cycle_incomplete_ms,
            "inactivity_window_ms": rt.session_inactivity_ms,
            "hard_max_session_ms": rt.session_hard_max_ms,
            "upload_dedupe": "cycle|batch_id|source",
            "analyze_on_ingest_default": false,
            "session_ticket_ttl_ms": gr_probe_core::TICKET_TTL_MS,
            "route_authority": "server",
            "cycle_id_is_session_id": true,
            "challenge_seed_ttl_ms": challenge_seed_ttl_ms,
            "require_sealed_ingest": require_sealed,
            "seal_mode": if gr_probe_core::seal_require_v2() { "session_grant_v2" } else { "session_grant_v1" },
            "ingest_sealed_path": "/v1/ingest/sealed",
            "hot_idle_ms": hot_idle_ms(),
            "cold_ttl_ms": cold_ttl_ms(),
            "cold_promote_window_ms": cold_promote_window_ms(),
            "config_version": rt.version,
            "return_identity_idle_ms": rt.return_identity_idle_ms,
            "analyze_idle_upload_ms": rt.analyze_idle_upload_ms,
            "analyze_debounce_ms": rt.analyze_debounce_ms,
            "complete_on_commercial_silicon": rt.complete_on_commercial_silicon,
            "fe_retry": fe_pol,
            "analyze_reads": "l2_probe_batches",
            "hot_tier": "l1_process_memory",
            "cold_tier": "l3_probe_cold",
            "rpa_collect": result_policy.rpa_collect,
            "primary_device_lane": result_policy.primary_device_lane,
            "response_profile": result_policy.response_profile,
            "include_signals": result_policy.include_signals,
        }),
    );
    resp.insert("config_version".into(), json!(gr_probe_store::config_version()));
    resp.insert("timeouts".into(), timeout_matrix_json());
    // Legacy plan metadata is returned for compatibility; current installs
    // expose the full RPA and device surface.
    {
        let data_dir = panel_data_dir(st);
        let page_host = headers
            .get("x-forwarded-host")
            .or_else(|| headers.get("host"))
            .map(|s| s.split(':').next().unwrap_or(s).to_ascii_lowercase());
        let ent = gr_probe_core::plan_entitlement::resolve_site_entitlement(
            if site_id.is_empty() {
                None
            } else {
                Some(site_id.as_str())
            },
            page_host.as_deref(),
            data_dir.as_deref(),
        );
        resp.insert("entitlement".into(), ent.to_json());
        resp.insert(
            "rpa_enabled".into(),
            json!(ent.rpa_enabled && result_policy.rpa_collect),
        );
        resp.insert(
            "device_precisions".into(),
            json!(ent.device_precisions.clone()),
        );
        resp.insert(
            "product_positioning".into(),
            json!({
                "mode": "request_analysis",
                "intercepts_requests": false,
            }),
        );
    }
    Ok(Value::Object(resp))
}

pub fn parse_ingest_body(headers: &HashMap<String, String>, bytes: &[u8]) -> Result<IngestBody, ApiError> {
    // Accept application/json and text/plain (sendBeacon pagehide flush, no CORS preflight).
    let ct = headers
        .get("content-type")
        .map(|s| s.as_str())
        .unwrap_or("application/json");
    if !(ct.starts_with("application/json")
        || ct.starts_with("text/plain")
        || ct.starts_with("application/x-www-form-urlencoded")
        || ct.is_empty())
    {
        // still try JSON — browsers vary
    }
    // Normalize shapes:
    //  A) { session_id, batch_id, source, payload: { fields: {...} } }  — FE upload_queue
    //  B) { session_id, batch_id, source, fields: {...} }              — lab/smoke convenience
    //  C) { session_id, batch_id, source, payload: { k:v... } }        — flat fields in payload
    let mut raw: Value = serde_json::from_slice(bytes).map_err(|e| {
        ApiError(
            400,
            format!("invalid ingest body ({ct}): {e}"),
        )
    })?;
    if let Some(obj) = raw.as_object_mut() {
        let has_payload_fields = obj
            .get("payload")
            .and_then(|p| p.get("fields"))
            .map(|f| f.is_object())
            .unwrap_or(false);
        if !has_payload_fields {
            if let Some(fields) = obj.get("fields").cloned() {
                if fields.is_object() {
                    let mut payload = obj
                        .get("payload")
                        .cloned()
                        .unwrap_or_else(|| json!({}));
                    if let Some(po) = payload.as_object_mut() {
                        po.insert("fields".into(), fields);
                    } else {
                        payload = json!({ "fields": fields });
                    }
                    obj.insert("payload".into(), payload);
                }
            }
        }
    }
    serde_json::from_value::<IngestBody>(raw).map_err(|e| {
        ApiError(
            400,
            format!("invalid ingest body ({ct}): {e}"),
        )
    })
}

/// A-DF-1 / norm/03: batch_id must resolve in component_catalog (pack_id or alias).
fn validate_ingest_batch_id(batch_id: &str) -> Result<(), ApiError> {
    let bid = batch_id.trim();
    if bid.is_empty() {
        return Err(ApiError(400, "batch_id empty".into()));
    }
    // Allow ops/lab inject markers without catalog entry
    if bid.starts_with("ops.") || bid.starts_with("lab.") || bid == "B8_gateway" {
        return Ok(());
    }
    match gr_probe_core::load_catalog() {
        Ok(cat) => {
            if cat.resolve(bid).is_some() {
                return Ok(());
            }
            // B8_gateway is alias of B8_gateway_early batch_id
            if cat.resolve("B8_gateway_early").is_some()
                && (bid == "B8_gateway" || bid == "gateway.b8")
            {
                return Ok(());
            }
            Err(ApiError(
                400,
                format!("batch_id not in catalog: {bid} (A-DF-1 ingest allowlist)"),
            ))
        }
        Err(e) => {
            // Fail open only if catalog unloadable (should not happen in prod)
            log::warn!("catalog load failed during ingest allowlist: {e}");
            Ok(())
        }
    }
}

/// Production gate: plain `/v1/ingest` rejected when GR_REQUIRE_SEALED_INGEST is set
/// (unless GR_ALLOW_PLAIN_INGEST for lab/break-glass). See sealed-ingest-production-gate.md.
pub fn ingest(st: &AppState, headers: &HashMap<String, String>, body_bytes: &[u8]) -> AppResult {
    if gr_abi::env::flag("REQUIRE_SEALED_INGEST") && !gr_abi::env::flag("ALLOW_PLAIN_INGEST") {
        let _ = st.store.insert_ops_server_event(json!({
            "code": "sealed_required_reject",
            "severity": "warn",
            "stage": "ingest",
            "product_version": gr_probe_core::GR_PRODUCT_VERSION,
            "detail_json": {
                "worker_id": st.worker_id,
                "path": "/v1/ingest",
                "hint": "/v1/ingest/sealed",
                "product_version": gr_probe_core::GR_PRODUCT_VERSION,
            },
        }));
        return Err(ApiError(
            426,
            "sealed_ingest_required: use POST /v1/ingest/sealed (session seal grant from open; set GR_ALLOW_PLAIN_INGEST=1 only for lab)".into(),
        ));
    }
    ingest_plain_body(st, headers, body_bytes, false)
}

/// iss/70: build FE Transport ACK from upsert outcome + optional capture metadata.
fn build_ingest_ack(
    body: &IngestBody,
    server_payload_hash: &str,
    upsert: &Value,
    payload: &Value,
) -> Value {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let merged = upsert
        .get("merged")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || payload
            .get("merged")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let unchanged = upsert
        .get("cold_skipped_unchanged")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || upsert
            .get("unchanged")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let was_insert = upsert
        .get("was_insert")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let gen = body.material_generation.unwrap_or(1);
    let dedupe_key = format!(
        "{}|{}|{}|{}",
        body.session_id, body.batch_id, body.source, gen
    );
    // Prefer explicit store signals; conflict only when store marks conflict.
    let store_conflict = upsert
        .get("conflict")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let upsert_ok = upsert
        .get("ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let durability = upsert
        .get("durability_state")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    // Same capture_id re-POST (transport retry) → duplicate even if server stamps change.
    let same_capture = upsert
        .get("same_capture")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let ack_type = if store_conflict {
        "conflict"
    } else if !upsert_ok || durability == "failed" {
        "rejected"
    } else if merged {
        "merged"
    } else if unchanged || same_capture {
        "duplicate"
    } else if was_insert {
        "stored"
    } else {
        "stored"
    };
    json!({
        "type": ack_type,
        "dedupe_key": dedupe_key,
        "session_id": body.session_id,
        "batch_id": body.batch_id,
        "source": body.source,
        "payload_hash": server_payload_hash,
        "material_hash": body.payload_hash,
        "client_payload_hash": body.payload_hash,
        "capture_id": body.capture_id,
        "material_generation": gen,
        "server_received_ms": now,
        "attempt_id": body.attempt_id,
        "durability_state": if durability.is_empty() {
            if upsert_ok {
                "stored_durable"
            } else {
                "failed"
            }
        } else {
            durability
        },
        "seq_end": payload.pointer("/fields/rpa_seq_end"),
        "segment_id": payload.pointer("/fields/rpa_segment_id"),
        "observation_id": payload.pointer("/observation/observation_id"),
        "source_kind": payload.pointer("/observation/validated/source_kind"),
        "probe_method_id": payload.pointer("/observation/validated/probe_method_id"),
    })
}

/// Ingest after sealed unseal (must NOT re-apply sealed gate).
///
/// `seal_verified=true` marks requests that already passed seal v2 signature
/// verification (`POST /v1/ingest/sealed` → [`ingest_sealed`]). P0-1 fix
/// (iss/grok4.6/03, proposal A): a verified seal IS the browser credential —
/// the per-site backend SDK key (`x-gr-sdk-key`, admin `sdk_enforce`) applies
/// only to non-sealed backend/ingest traffic, never to verified browser seals.
pub fn ingest_plain_body(
    st: &AppState,
    headers: &HashMap<String, String>,
    body_bytes: &[u8],
    seal_verified: bool,
) -> AppResult {
    // Shared body of ingest — originally continued directly after sealed gate.
    // NOTE: keep gate only in `ingest`.
    // Body size guard (first-party relay abuse)
    if body_bytes.len() > 512 * 1024 {
        let _ = st.store.insert_ops_server_event(json!({
            "code": "ingest_body_too_large",
            "severity": "error",
            "stage": "ingest",
            "detail_json": {"len": body_bytes.len()},
        }));
        return Err(ApiError(413, "ingest body too large".into()));
    }
    // Optional first-party relay anti-replay (when headers present).
    let relay_ts = headers
        .get("x-g5-ts")
        .or_else(|| headers.get("x-gr-relay-ts"))
        .and_then(|s| s.parse::<i64>().ok());
    let relay_nonce = headers
        .get("x-g5-nonce")
        .or_else(|| headers.get("x-gr-relay-nonce"))
        .cloned();
    let relay_sig = headers
        .get("x-g5-sig")
        .or_else(|| headers.get("x-gr-relay-sig"))
        .cloned();
    if let (Some(ts), Some(nonce), Some(sig)) = (relay_ts, relay_nonce.as_ref(), relay_sig.as_ref()) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let bh = gr_probe_core::sha256_hex(body_bytes);
        // session_id unknown until parse — use body hash + empty sid for header-only gate,
        // or re-check after parse. Pre-parse: verify with wildcard sid from header.
        let sid_h = headers
            .get("x-g5-sid")
            .or_else(|| headers.get("x-gr-session-id"))
            .map(|s| s.as_str())
            .unwrap_or("");
        if !gr_probe_core::verify_relay_sig(
            &st.challenge_secret,
            sid_h,
            ts,
            nonce,
            &bh,
            sig,
            now,
            120_000,
        ) {
            let _ = st.store.insert_ops_server_event(json!({
                "code": "relay_sig_invalid",
                "severity": "error",
                "stage": "ingest",
                "detail_json": {"session_id": sid_h},
            }));
            return Err(ApiError(401, "relay signature invalid".into()));
        }
    }
    let body = parse_ingest_body(&headers, body_bytes)?;
    require_session_site_auth(st, headers, &body.session_id)?;
    validate_ingest_batch_id(&body.batch_id)?;
    // iss/opus5 05 low (seal replay ledger): signed (nonced) result batches
    // are recorded as consumed; the exact (session, batch, nonce) tuple
    // arriving twice is a replay and is rejected persistently (409).
    if let Some(nonce) = relay_nonce.as_deref() {
        match st.store.mark_seal_consumed(&body.session_id, &body.batch_id, nonce) {
            Ok(true) => {}
            Ok(false) => {
                let _ = st.store.insert_ops_server_event(json!({
                    "code": "seal_replay_detected",
                    "severity": "error",
                    "stage": "ingest",
                    "detail_json": {
                        "session_id": body.session_id,
                        "batch_id": body.batch_id,
                        "nonce": nonce,
                    },
                }));
                return Err(ApiError(409, "replayed sealed batch".into()));
            }
            Err(e) => {
                // Ledger write failure: reject the batch anyway — a sealed
                // batch that cannot be ledgered must not silently proceed.
                let _ = st.store.insert_ops_server_event(json!({
                    "code": "seal_ledger_error",
                    "severity": "warn",
                    "stage": "ingest",
                    "detail_json": {"err": e.to_string()},
                }));
                return Err(ApiError(503, "seal ledger unavailable".into()));
            }
        }
    }
    {
        let mut ingest_site = resolve_site_id_from_host(st, headers);
        if ingest_site.is_empty() {
            ingest_site = st
                .store
                .merge_session_meta(&body.session_id, &json!({}))
                .ok()
                .and_then(|m| {
                    m.get("site_id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_default();
        }
        let ingest_result_policy =
            gr_probe_core::load_panel_policy(panel_data_dir(st).as_deref())
                .resolve_result_policy(if ingest_site.is_empty() {
                    None
                } else {
                    Some(ingest_site.as_str())
                });
        if !site_collect_enabled(st, &ingest_site, headers) {
            return Err(reject_collect_disabled());
        }
        // A disabled RPA policy is an intentional no-op for B11. Return a
        // successful transport response so the browser queue does not retry
        // data the merchant explicitly chose not to collect.
        if !ingest_result_policy.rpa_collect && body.batch_id.contains("B11") {
            return Ok(json!({
                "ok": true,
                "ignored": true,
                "code": "rpa_collection_disabled",
                "batch_id": body.batch_id,
                "session_id": body.session_id,
                "ack": {
                    "type": "disabled",
                    "batch_id": body.batch_id,
                    "session_id": body.session_id,
                },
            }));
        }
        // Cookie capture fallback: sessions opened without a browser Cookie
        // (pure backend SDK / nginx-first flow) can still bind business
        // identifiers when the ingest request carries them. First capture wins.
        if !ingest_site.is_empty() {
            let current = st
                .store
                .merge_session_meta(&body.session_id, &json!({}))
                .ok()
                .unwrap_or(json!({}));
            if !meta_has_cookie_fields(&current) {
                let from_body = body
                    .cookie_fields
                    .as_ref()
                    .map(|v| capture_cookie_fields_from_value(st, &ingest_site, v))
                    .unwrap_or_else(|| json!({}));
                let captured = if cookie_value_nonempty(&from_body) {
                    from_body
                } else {
                    capture_cookie_fields_from_header(st, &ingest_site, headers)
                };
                if cookie_value_nonempty(&captured) {
                    let _ = st.store.merge_session_meta(
                        &body.session_id,
                        &json!({"cookie_fields": captured}),
                    );
                }
            }
        }
    }
    // iss/72: reject client plan_epoch older than server session plan_epoch (stale route).
    // Only when client explicitly sends plan_epoch > 0 (legacy clients omit → accepted).
    if let Some(client_pe) = body.plan_epoch.filter(|p| *p > 0) {
        let meta = st
            .store
            .merge_session_meta(&body.session_id, &json!({}))
            .ok()
            .unwrap_or(json!({}));
        let server_pe = meta
            .get("plan_epoch")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        if server_pe > 0 && client_pe < server_pe {
            let _ = st.store.insert_ops_server_event(json!({
                "code": "stale_plan_epoch",
                "severity": "warn",
                "stage": "ingest",
                "session_id": body.session_id,
                "detail_json": {
                    "client_plan_epoch": client_pe,
                    "server_plan_epoch": server_pe,
                    "batch_id": body.batch_id,
                },
            }));
            return Ok(json!({
                "ok": false,
                "error": "stale_plan_epoch",
                "code": "stale_plan_epoch",
                "session_id": body.session_id,
                "batch_id": body.batch_id,
                "source": body.source,
                "plan_epoch": server_pe,
                "client_plan_epoch": client_pe,
                "ack": {
                    "type": "stale_plan",
                    "session_id": body.session_id,
                    "batch_id": body.batch_id,
                    "source": body.source,
                    "plan_epoch": server_pe,
                    "client_plan_epoch": client_pe,
                },
            }));
        }
    }
    // iss/49: payload-hash conflict audit (dedupe / integrity, not a secret seal)
    let payload_hash = gr_probe_core::sha256_hex(body_bytes);
    let payload_hash_hdr = headers
        .get("x-gr-payload-hash")
        .or_else(|| headers.get("x-g5-payload-hash"))
        .map(|s| s.as_str());
    if let Some(claimed) = payload_hash_hdr {
        if !claimed.is_empty() && claimed != payload_hash {
            let _ = st.store.insert_ops_server_event(json!({
                "code": "ingest_payload_hash_mismatch",
                "severity": "warn",
                "stage": "ingest",
                "detail_json": {
                    "claimed": claimed,
                    "computed": &payload_hash,
                    "batch_id": &body.batch_id,
                },
            }));
            return Err(ApiError(400, "payload_hash mismatch (iss/49 ingest audit)".into()));
        }
    }
    // Optional per-site backend SDK key (admin sdk_enforce).
    // P0-1 (iss/grok4.6/03, proposal A): seal-v2-verified requests are already
    // browser-authenticated (session-bound signature + anti-replay); skip the
    // backend SDK key check for them. Plain backend traffic keeps the check —
    // prod hardcodes enforce, so this must come after seal verification.
    if !seal_verified {
        if let Some(admin) = st.admin.as_ref() {
            let key = headers
                .get("x-gr-sdk-key")
                .or_else(|| headers.get("X-Gr-Sdk-Key"))
                .map(|s| s.as_str());
            let host = headers.get("host").map(|s| s.as_str());
            let origin = headers.get("origin").map(|s| s.as_str());
            if let Err(e) = crate::admin::sdk::check_backend_key(&admin.db, key, host, origin) {
                return Err(ApiError(401, e));
            }
        }
    }
    // v57 race: FE may upload before open returns — always ensure session exists.
    // Prefer VT active incomplete (one bag) so dual B8 / multi-session_id cannot fork cycles.
    {
        let vt = body
            .payload
            .pointer("/fields/visitor_terminal_id")
            .or_else(|| body.payload.get("visitor_terminal_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let mut meta = json!({
            "fe_race": true,
            "ingest_auto_open": true,
            "inject_path": body.inject_path.clone().unwrap_or_else(|| "unknown".into()),
            "product_version": gr_probe_core::GR_PRODUCT_VERSION,
            "version": gr_probe_core::GR_PRODUCT_VERSION,
        });
        if let Some(sid) = body
            .payload
            .pointer("/fields/site_id")
            .and_then(|v| v.as_str())
        {
            if let Some(obj) = meta.as_object_mut() {
                obj.insert("site_id".into(), json!(sid));
            }
        }
        let _ = st.store.open_session(Some(body.session_id.clone()), vt, Some(meta));
    }
    let mut payload = body.payload.clone();
    if let Some(ip) = body.inject_path.as_ref() {
        if let Some(obj) = payload.as_object_mut() {
            obj.entry("inject_path".to_string())
                .or_insert_with(|| json!(ip));
        }
    }
    // iss/58 A1: one server-stamped sample per batch (do not overwrite client series
    // with identical t_server — that collapses Theil–Sen slopes to zero).
    {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as f64)
            .unwrap_or(0.0);
        let fields_obj = if payload.get("fields").map(|v| v.is_object()).unwrap_or(false) {
            payload.get_mut("fields").and_then(|v| v.as_object_mut())
        } else {
            payload.as_object_mut()
        };
        if let Some(fields) = fields_obj {
            fields.insert("t_server_recv_ms".into(), json!(now_ms));
            let tp = fields.get("t_perf").and_then(|v| v.as_f64());
            let tw = fields
                .get("t_wall_ms")
                .and_then(|v| v.as_f64())
                .unwrap_or(now_ms);
            if let Some(tp) = tp {
                let point = json!({
                    "t_perf": tp,
                    "t_server_ms": now_ms,
                    "t_wall_ms": tw,
                });
                match fields.get_mut("clock_skew_samples") {
                    Some(Value::Array(arr)) => {
                        arr.push(point);
                        if arr.len() > 32 {
                            let n = arr.len();
                            arr.drain(0..n - 32);
                        }
                    }
                    _ => {
                        fields.insert("clock_skew_samples".into(), json!([point]));
                    }
                }
            }
        }
    }
    // Multi-source visitor class: FE source → js; crawler UA → robots(+name)
    {
        let ua = payload
            .pointer("/fields/user_agent")
            .and_then(|v| v.as_str())
            .or_else(|| headers.get("user-agent").map(|s| s.as_str()))
            .unwrap_or("");
        let has_fe = body.source == "main"
            || body.source == "worker"
            || body.source == "iframe"
            || body.source.starts_with("sandbox")
            || body.source.starts_with("iframe");
        let facet_meta = crate::admin::facet::merge_visit_class(
            None,
            &crate::admin::facet::ClassInputs {
                ua: ua.to_string(),
                is_pixel: false,
                has_fe_main: has_fe,
                fe_open_hint: has_fe,
                has_vtid: false,
                backend_claims_js: false,
            },
        );
        let _ = st.store.merge_session_meta(&body.session_id, &facet_meta);
    }
    // Sanitize custom_link / client_tags (SDK business association keys; never device mint).
    {
        let mut fields_mut = payload.get("fields").cloned().unwrap_or(json!({}));
        if let Some(fo) = fields_mut.as_object_mut() {
            let _ = gr_probe_core::sanitize_custom_link_fields(fo);
            // Also accept top-level payload custom_link
            if !fo.contains_key("custom_link") {
                if let Some(cl) = payload.get("custom_link") {
                    fo.insert("custom_link".into(), cl.clone());
                    let _ = gr_probe_core::sanitize_custom_link_fields(fo);
                }
            }
        }
        if let Some(po) = payload.as_object_mut() {
            po.insert("fields".into(), fields_mut);
        }
    }
    // H16: validate challenge response materials when present (HMAC seed path)
    if let Some(fo) = payload.get("fields").and_then(|f| f.as_object()) {
        if fo.get("challenge_seed").is_some() || fo.get("challenge_seed_sig").is_some()
            || fo.get("pohw_triad").is_some()
        {
            let mut fields_mut = payload.get("fields").cloned().unwrap_or(json!({}));
            if let Some(fo) = fields_mut.as_object() {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let (ok_ch, reasons) = gr_probe_core::evaluate_challenge_fields(
                    &body.session_id,
                    fo,
                    &st.challenge_secret,
                    now_ms,
                );
                if let Some(obj) = fields_mut.as_object_mut() {
                    obj.insert(
                        "challenge_eval".into(),
                        json!({"ok": ok_ch, "reasons": reasons}),
                    );
                }
            }
            if let Some(po) = payload.as_object_mut() {
                po.insert("fields".into(), fields_mut);
            }
        }
    }
    let mut client_ip: Option<String> = None;
    if let Some(obj) = payload.as_object_mut() {
        // Always stamp server product_version on batch fields (SSOT for lab filtering).
        obj.entry("product_version".to_string())
            .or_insert_with(|| json!(gr_probe_core::GR_PRODUCT_VERSION));
        // iss/70: propagate capture identity into payload for store ACK/dedupe.
        if let Some(ref cid) = body.capture_id {
            obj.entry("capture_id".to_string())
                .or_insert_with(|| json!(cid));
        }
        if let Some(ref ph) = body.payload_hash {
            obj.entry("payload_hash".to_string())
                .or_insert_with(|| json!(ph));
        }
        if let Some(gen) = body.material_generation {
            obj.entry("material_generation".to_string())
                .or_insert_with(|| json!(gen));
        }
        let fields = obj
            .entry("fields".to_string())
            .or_insert_with(|| json!({}));
        if let Some(fo) = fields.as_object_mut() {
            fo.entry("product_version".to_string())
                .or_insert_with(|| json!(gr_probe_core::GR_PRODUCT_VERSION));
            fo.insert(
                "server_product_version".into(),
                json!(gr_probe_core::GR_PRODUCT_VERSION),
            );
            if let Some(ref cid) = body.capture_id {
                fo.entry("capture_id".to_string())
                    .or_insert_with(|| json!(cid));
            }
            if let Some(ref ph) = body.payload_hash {
                fo.entry("client_payload_hash".to_string())
                    .or_insert_with(|| json!(ph));
            }
            if let Some(gen) = body.material_generation {
                fo.entry("material_generation".to_string())
                    .or_insert_with(|| json!(gen));
            }
            let (ip, _src) = apply_authoritative_client_ip(fo, headers);
            // Write-time geo enrichment (MMDB + builtin heuristics): fills
            // server_asn / server_country / network_class only when absent.
            // Same hook the gateway B8 path applies; without it the main
            // sealed-ingest source stored client IPs with zero geo (178:
            // 52k/6h rows country/asn 0%).
            gr_probe_core::enrich_fields_if_empty(fo, ip.as_deref());
            client_ip = ip;
        }
    }
    let claimed = gr_probe_core::ClaimedEnvelope {
        source_kind: body.source_kind.clone(),
        realm_id: body.realm_id.clone(),
        realm_kind: body.realm_kind.clone(),
        probe_method_id: body.probe_method_id.clone(),
        method_version: body.method_version.clone(),
        observation_id: body.observation_id.clone(),
        attempt_id: body.attempt_id.clone(),
        capture_id: body.capture_id.clone(),
        material_generation: body.material_generation,
    };
    let envelope = gr_probe_core::stamp_observation_envelope(
        &mut payload,
        &body.session_id,
        &body.batch_id,
        &body.source,
        body.inject_path.as_deref(),
        &claimed,
    );
    // Robots fast lane: sessions classified robots at open/gateway skip the
    // cold tier, the L1 hot tier and every analyze arm — early result stands.
    let is_robot = robot_fastlane_active(st) && session_is_robot(st, &body.session_id);
    if is_robot {
        try_robot_early_result(st, &body.session_id, None, "ingest_fastlane");
    }
    let mut upsert = st.store.upsert_batch_with_ip_opts(
        &body.session_id,
        &body.batch_id,
        &body.source,
        &payload,
        client_ip.as_deref(),
        is_robot,
    )?;
    // Hot tier: L1 process cache; L2/L3 dual-write already done above.
    {
        let mut vt = payload
            .pointer("/fields/visitor_terminal_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if vt.is_empty() {
            if let Ok(w) = st.store.session_window(&body.session_id) {
                if let Some(s) = w.get("visitor_terminal_id").and_then(|v| v.as_str()) {
                    vt = s.to_string();
                }
            }
        }
        let mut promoted = 0usize;
        if !vt.is_empty() && !is_robot {
            // New/returning VT: pull recent L3 cold into L1 before writing this batch.
            promoted = promote_cold_to_hot_if_needed(st, &vt, &body.session_id);
            st.hot_probe.upsert_batch(
                &vt,
                &body.session_id,
                &body.batch_id,
                &payload,
                client_ip.as_deref(),
            );
        }
        let _demoted = demote_hot_to_cold(st, hot_idle_ms());
        if let Some(obj) = upsert.as_object_mut() {
            // surface promote count for lab/ops (analyze still uses L2 batches)
            obj.insert("hot_promoted_from_cold".into(), json!(promoted));
            obj.insert("hot_idle_ms".into(), json!(hot_idle_ms()));
        }
    }
    {
        let validated = envelope.get("validated").cloned().unwrap_or(json!({}));
        let obs_row = json!({
            "observation_id": envelope.get("observation_id"),
            "tenant_id": lookup_session_site(st, &body.session_id).unwrap_or_default(),
            "session_id": body.session_id,
            "batch_id": body.batch_id,
            "source": body.source,
            "source_kind": validated.get("source_kind"),
            "realm_kind": validated.get("realm_kind"),
            "probe_method_id": validated.get("probe_method_id"),
            "capture_id": body.capture_id,
            "attempt_id": body.attempt_id,
            "envelope": envelope,
        });
        match st.store.insert_observation_event(obs_row) {
            Ok(o) => {
                if let Some(obj) = upsert.as_object_mut() {
                    obj.insert("observation_appended".into(), json!(true));
                    obj.insert("observation_id".into(), o.get("observation_id").cloned().unwrap_or(Value::Null));
                }
            }
            Err(_) => {
                if let Some(obj) = upsert.as_object_mut() {
                    obj.insert("observation_appended".into(), json!(false));
                }
            }
        }
    }
    // Brain-owned analyze arm (v5.8.105): NOT per-batch.
    // Triggers: coverage 100% → now; idle 60s quiet; no-result 180s since cycle open.
    // Explicit body.analyze / pagehide still force immediate analyze.
    // Never arm analyze on a failed ingest (rollback / conflict / durability fail).
    let upsert_ok = upsert
        .get("ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let fields = payload
        .get("fields")
        .cloned()
        .unwrap_or(Value::Null);
    let pagehide_any = fields
        .get("pagehide_flush")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || fields
            .get("behavior_pagehide")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        || fields
            .get("rpa_flush_reason")
            .and_then(|v| v.as_str())
            .map(|s| {
                s == "pagehide"
                    || s == "idle_30s"
                    || s == "idle_45s"
                    || s == "hidden"
                    || s == "flush"
            })
            .unwrap_or(false)
        || payload
            .get("pagehide_flush")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let rpa_trigger = body.batch_id.contains("B11")
        && (pagehide_any
            || fields
                .get("rpa_idle_flush")
                .and_then(|v| v.as_bool())
                .unwrap_or(false));
    let is_b8 = body.batch_id == "B8_gateway" || body.batch_id == "B8_gateway_early";
    let b8_only_skip = if is_b8 {
        let rec = st
            .store
            .list_received_batches(&body.session_id)
            .unwrap_or_default();
        let has_fe = rec.iter().any(|b| {
            let id = b
                .get("batch_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            id != "B8_gateway"
                && id != "B8_gateway_early"
                && !id.starts_with("ops.")
                && !id.starts_with("B_ops")
        });
        !has_fe
    } else {
        false
    };
    let explicit_analyze = body.analyze || rpa_trigger || pagehide_any;
    // Soft packs that do not change silicon digests — after first analysis, do not thrash
    // evaluate (shared-algorithm: reuse result until materials fingerprint moves).
    let soft_non_identity_batch = matches!(
        body.batch_id.as_str(),
        "B11_interaction"
            | "B28_permissions_media"
            | "B29_sensors_battery"
            | "B16_fast_signals"
    ) || body.batch_id.starts_with("B_ops")
        || body.batch_id.starts_with("ops.");
    // Brain schedule v2 only — NOT per-batch, NOT identity/milestone 800ms thrash.
    // Arms: coverage_complete → now; continuous upload → IdleReset (+60s quiet);
    // no first result yet → min(idle 60s, first_upload+180s); pre-cold/explicit separate.
    let brain_analyze_armed = if !upsert_ok {
        false
    } else if is_robot {
        // Robots fast lane: the early-class result already closed this session —
        // deep identity analysis of a UA-declared crawler adds nothing.
        false
    } else if b8_only_skip && !explicit_analyze {
        false
    } else if soft_non_identity_batch && !explicit_analyze {
        // Soft pack: only re-arm long idle if we already have an analysis; else normal path
        // falls through by treating as continuous upload with longer debounce.
        let has_prev = st
            .store
            .latest_analysis(&body.session_id)
            .ok()
            .flatten()
            .is_some();
        if has_prev {
            // materials unchanged for identity — skip re-schedule (avoid waste)
            false
        } else {
            let _ = st.store.schedule_analyze_merge(
                &body.session_id,
                ANALYZE_IDLE_UPLOAD_MS,
                AnalyzeDueMerge::IdleReset,
            );
            true
        }
    } else if explicit_analyze {
        let _ = st.store.schedule_analyze_merge(
            &body.session_id,
            0,
            AnalyzeDueMerge::Replace,
        );
        true
    } else {
        // Coverage floors from received batches via brain checklist (no full evaluate).
        let rec = st
            .store
            .list_received_batches(&body.session_id)
            .unwrap_or_default();
        let mut present = std::collections::HashSet::new();
        for b in &rec {
            if let Some(id) = b.get("batch_id").and_then(|v| v.as_str()) {
                present.insert(id.to_string());
            }
        }
        let sources: Vec<String> = rec
            .iter()
            .filter_map(|b| b.get("source").and_then(|v| v.as_str()).map(|s| s.to_string()))
            .collect();
        let light_ev = json!({
            "batches": rec,
            "sources": sources,
            "meta": {"inject_path": "nginx"},
        });
        let checklist = gr_probe_core::brain::coverage_checklist(
            &light_ev,
            &present,
            st.soft_v2_ready,
            true,
        )
        .ok();
        let coverage_complete = checklist
            .as_ref()
            .and_then(|c| c.get("coverage_complete").and_then(|v| v.as_bool()))
            .unwrap_or(false);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        // First upload of this batch/cycle = min batch created_ms (batch-scoped 180s arm).
        let first_upload_ms = rec
            .iter()
            .filter_map(|b| b.get("created_ms").and_then(|v| v.as_i64()))
            .min()
            .or_else(|| {
                st.store
                    .session_window(&body.session_id)
                    .ok()
                    .and_then(|w| {
                        w.get("created_ms")
                            .and_then(|v| v.as_i64())
                            .or_else(|| w.get("opened_ms").and_then(|v| v.as_i64()))
                    })
            })
            .unwrap_or(now);
        let last_analyze_ok = st
            .store
            .latest_analysis(&body.session_id)
            .ok()
            .flatten()
            .and_then(|a| {
                a.get("created_ms")
                    .and_then(|v| v.as_i64())
                    .or_else(|| a.get("updated_ms").and_then(|v| v.as_i64()))
            });
        let debounce = analyze_arm_debounce_ms_after_ingest(
            now,
            first_upload_ms,
            last_analyze_ok,
            coverage_complete,
        );
        // Strict v2 merge — never compress idle to 800ms on identity milestones.
        // - coverage complete → due now (PullEarlier 0)
        // - no-result ceiling (debounce < 60s) → PullEarlier to that absolute due
        // - normal continuous upload → IdleReset (+60s quiet window, never delay imminent)
        let merge = if coverage_complete {
            AnalyzeDueMerge::PullEarlier
        } else if debounce < ANALYZE_IDLE_UPLOAD_MS {
            AnalyzeDueMerge::PullEarlier
        } else {
            AnalyzeDueMerge::IdleReset
        };
        let _ = st
            .store
            .schedule_analyze_merge(&body.session_id, debounce, merge);
        let _ = ANALYZE_IDLE_IMMINENT_MS; // referenced for docs/health coupling
        true
    };
    let fast_analyze = explicit_analyze; // kept for response field compat
    // Multi-channel probe status for FE (do not rely only on 410).
    let probe_status = cycle_probe_status_snapshot(st, &body.session_id);
    // iss/70: explicit ACK type for FE Transport FSM (stored/duplicate/merged/conflict).
    let ack = build_ingest_ack(
        &body,
        &payload_hash,
        &upsert,
        &payload,
    );
    crate::dual_log::emit(
        crate::dual_log::Channel::ProbeBusiness,
        if upsert_ok {
            "ingest_ack"
        } else {
            "ingest_failed"
        },
        json!({
            "session_id": body.session_id,
            "batch_id": body.batch_id,
            "source": body.source,
            "ok": upsert_ok,
            "ack_type": ack.get("type"),
            "durability_state": ack.get("durability_state"),
        }),
    );
    let mut out = json!({
        "ok": upsert_ok
            && ack.get("type").and_then(|t| t.as_str()) != Some("conflict")
            && ack.get("type").and_then(|t| t.as_str()) != Some("rejected"),
        "durability_state": ack.get("durability_state"),
        "cold_written": upsert.get("cold_written"),
        "accepted": ack.get("type").and_then(|t| t.as_str()) != Some("conflict")
            && ack.get("type").and_then(|t| t.as_str()) != Some("rejected"),
        "session_id": body.session_id,
        "cycle_id": body.session_id,
        "batch_id": body.batch_id,
        "source": body.source,
        "payload_hash": payload_hash,
        "capture_id": body.capture_id,
        "material_generation": body.material_generation,
        "client_payload_hash": body.payload_hash,
        "ack": ack,
        "ingest": upsert,
        "analyze_deferred": !body.analyze,
        "rpa_analyze_scheduled": rpa_trigger,
        "fast_analyze_scheduled": fast_analyze,
        "brain_analyze_armed": brain_analyze_armed,
        "analyze_schedule": analyze_schedule_policy_json(),
        // Dual signals: nested + top-level for robust FE parsers
        "cycle_probe_status": probe_status,
        "session_active": probe_status.get("session_active"),
        "accepts_identity_ingest": probe_status.get("accepts_identity_ingest"),
        "halt_uploads": probe_status.get("halt_uploads"),
        "skip_identity_probe": probe_status.get("skip_identity_probe"),
        "cycle_status": probe_status.get("cycle_status"),
        "probe_complete": probe_status.get("probe_complete"),
        "analysis_terminal": probe_status.get("analysis_terminal"),
        "received_batch_ids": probe_status.get("received_batch_ids"),
        "identity_coverage": probe_status.get("identity_coverage"),
    });
    if upsert_ok && (body.analyze || rpa_trigger) {
        let evidence = st.store.build_evidence(&body.session_id)?;
        let result = evaluate_session(&evidence, None, None, None, st.soft_v2_ready)
            .map_err(|e| ApiError(422, e))?;
        let rev = st.store.save_analysis(&body.session_id, &result)?;
        persist_brain_control(&st.store, &body.session_id, &result, rev);
        if let Some(page) = result.get("page") {
            if let Some(pid) = page.get("page_id").and_then(|v| v.as_str()) {
                let prev = page
                    .get("page_rev")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(1);
                let _ = st.store.save_page_result(&body.session_id, pid, prev, page);
            }
        }
        let cycle_complete = st
            .store
            .maybe_complete_cycle_from_analysis(&body.session_id, &result)
            .ok()
            .flatten();
        // Refresh status after possible cycle close.
        let probe_status2 = cycle_probe_status_snapshot(st, &body.session_id);
        // Align with store::analysis_completes_cycle — never halt solely on soft probe_complete.
        let closes = gr_probe_store::analysis_completes_cycle(&result) || cycle_complete.is_some();
        let halt = closes
            || probe_status2
                .get("halt_uploads")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
        out.as_object_mut().unwrap().insert(
            "analysis".into(),
            json!({
                "rev": rev,
                "real_band": result.get("real_band"),
                "result": result,
                "product": result.get("product"),
                "sdk_return": result.get("sdk_return"),
                "rpa_analyze": result.get("rpa_analyze"),
                "probe_complete": result.get("probe_complete"),
                "analysis_terminal": result.get("analysis_terminal"),
                "cycle_complete": cycle_complete,
                "cycle_closes": closes,
                "halt_uploads": halt,
                "skip_identity_probe": halt,
            }),
        );
        if let Some(map) = out.as_object_mut() {
            map.insert("cycle_probe_status".into(), probe_status2.clone());
            map.insert("halt_uploads".into(), json!(halt));
            map.insert("skip_identity_probe".into(), json!(halt));
            map.insert("probe_complete".into(), result.get("probe_complete").cloned().unwrap_or(json!(false)));
            map.insert("analysis_terminal".into(), result.get("analysis_terminal").cloned().unwrap_or(json!(false)));
            map.insert("cycle_complete".into(), json!(cycle_complete));
            map.insert("cycle_closes".into(), json!(closes));
        }
    }
    Ok(out)
}

/// Snapshot of cycle/probe lifecycle for FE multi-channel coordination.
/// Prefer these flags over inferring from HTTP 410 alone.
fn server_cycle_facts(store: &Store, session_id: &str) -> gr_probe_store::ServerCycleFacts {
    let window = store.session_window(session_id).ok();
    let active = window
        .as_ref()
        .and_then(|w| w.get("active"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let cycle_status = window
        .as_ref()
        .and_then(|w| w.get("cycle_status"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let expired_reason = window
        .as_ref()
        .and_then(|w| w.get("expired_reason"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let batches = store.list_received_batches(session_id).unwrap_or_default();
    let mut batch_ids: Vec<String> = batches
        .iter()
        .filter_map(|b| b.get("batch_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    batch_ids.sort();
    batch_ids.dedup();
    let latest = store.latest_analysis(session_id).ok().flatten();
    let (probe_complete, analysis_terminal, stop_probe, coverage_complete) =
        if let Some(ref a) = latest {
            let r = a.get("result").cloned().unwrap_or_else(|| a.clone());
            (
                r.get("probe_complete")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                r.get("analysis_terminal")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                r.pointer("/route_plan/stop_probe")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                r.pointer("/coverage/coverage_complete")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            )
        } else {
            (false, false, false, false)
        };
    gr_probe_store::ServerCycleFacts {
        session_id: session_id.to_string(),
        active,
        cycle_status,
        expired_reason,
        analysis_terminal,
        probe_complete,
        stop_probe,
        coverage_complete,
        received_batch_ids: batch_ids,
    }
}

/// iss/opus5 03-P0-1: planned-vs-actual evidence ledger → withholding band.
///
/// The server issued a signed `route_plan` (expected pack set) in the last
/// analyze; the store holds what actually arrived. A session that is **still
/// alive** with a high missing ratio means the client withheld material —
/// that evasion is a strong signal, so the ambiguous "insufficient" band is
/// upgraded to `evidence_withheld` instead of rewarding the evader.
/// Dead/closed sessions (network drop, early leave) keep their honest band.
fn apply_evidence_withheld(store: &Store, session_id: &str, result: &mut Value) {
    let route_plan = result.get("route_plan").cloned().unwrap_or_default();
    let expected = gr_probe_core::evidence_ledger::expected_pack_ids(&route_plan);
    let min_expected = gr_probe_core::evidence_ledger::ew_min_expected();
    if expected.len() < min_expected.max(1) {
        return;
    }
    let batches = store.list_received_batches(session_id).unwrap_or_default();
    let received: Vec<String> = batches
        .iter()
        .filter_map(|b| b.get("batch_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    let facts = server_cycle_facts(store, session_id);
    // A live session is not necessarily an intentional withholding case:
    // pagehide/unload can arrive before the session window is closed. Treat
    // explicit short-visit lifecycle evidence as a dead/early-leave path so
    // missing packs remain visible in the ledger without penalizing a real
    // visitor who immediately closes the page.
    let meta = store.session_meta(session_id).ok().flatten();
    let short_visit = explicit_short_visit(result, meta.as_ref());
    let session_alive = facts.active && !facts.analysis_terminal && !short_visit;
    if let Some(ew) = gr_probe_core::evidence_ledger::assess_evidence_withheld(
        &expected,
        &received,
        session_alive,
        min_expected,
        gr_probe_core::evidence_ledger::ew_miss_ratio(),
    ) {
        if let Some(obj) = result.as_object_mut() {
            obj.insert("evidence_withheld".into(), ew.clone());
            let band = obj
                .get("real_band")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if matches!(band.as_str(), "insufficient" | "watch") {
                obj.insert("real_band".into(), json!("evidence_withheld"));
                let cred = obj
                    .get("credibility")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.5);
                obj.insert(
                    "credibility".into(),
                    json!(((cred.min(0.45) * 1000.0).round()) / 1000.0),
                );
                let mut reasons = obj.get("reasons").cloned().unwrap_or(json!([]));
                if let Some(arr) = reasons.as_array_mut() {
                    arr.push(json!("planned_missing_high"));
                } else {
                    reasons = json!(["planned_missing_high"]);
                }
                obj.insert("reasons".into(), reasons);
                // Mirror into diagnostics for ops consumers that read there.
                if let Some(diag) = obj.get_mut("diagnostics").and_then(|d| d.as_object_mut()) {
                    diag.insert("real_band".into(), json!("evidence_withheld"));
                }
            }
        }
        let _ = store.insert_ops_server_event(json!({
            "code": "evidence_withheld",
            "severity": "warn",
            "stage": "analyze",
            "session_id": session_id,
            "product_version": gr_probe_core::GR_PRODUCT_VERSION,
            "detail_json": ew,
        }));
    }
}

fn explicit_short_visit(result: &Value, meta: Option<&Value>) -> bool {
    fn marked(value: &Value) -> bool {
        value
            .get("pagehide_flush")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || value
                .get("stop_reason")
                .and_then(|v| v.as_str())
                .map(|s| matches!(s, "pagehide" | "beforeunload" | "unload" | "early_leave"))
                .unwrap_or(false)
    }

    marked(result) || meta.map(marked).unwrap_or(false)
}

fn cycle_probe_status_snapshot(st: &AppState, session_id: &str) -> Value {
    let facts = server_cycle_facts(&st.store, session_id);
    let snap = gr_probe_store::derive_server_snapshot(&facts);
    let mut out = gr_probe_store::snapshot_to_json(&snap, session_id);
    let identity_needed = [
        "B0_bootstrap",
        "B1_conflict",
        "B2_hardware",
        "B3_system",
        "B8_gateway",
        "B10_hw_curves",
        "B12_anti_camouflage",
    ];
    let present: Vec<&str> = identity_needed
        .iter()
        .copied()
        .filter(|id| {
            facts
                .received_batch_ids
                .iter()
                .any(|b| b == id || (*id == "B8_gateway" && b == "B8_gateway_early"))
        })
        .collect();
    let latest = st.store.latest_analysis(session_id).ok().flatten();
    let real_band = latest
        .as_ref()
        .and_then(|a| {
            let r = a.get("result").cloned().unwrap_or_else(|| a.clone());
            r.get("real_band").cloned()
        })
        .unwrap_or(Value::Null);
    if let Some(map) = out.as_object_mut() {
        // Primary residual only — B10x_silicon_* MUST NOT count as has_b10.
        // Bug lab (7browser×4site): B10x without B10_hw_curves set has_b10=true via
        // starts_with("B10_") → identity_complete_cool + FE halt while B10 still pending.
        let has_b10 = facts.received_batch_ids.iter().any(|b| {
            b == "B10_hw_curves" || b == "mid.curves"
        });
        // Hard materials present (B10 + form) — milestone, not schedule-final.
        let has_form = present.iter().any(|id| {
            *id == "B0_bootstrap" || *id == "B2_hardware" || *id == "B3_system"
        });
        let hard_complete = has_b10 && (has_form || present.len() >= 3);
        // Commercial materials milestone (dh/dv + silicon) — informational only.
        let commercial_final = latest
            .as_ref()
            .map(|a| {
                let r = a.get("result").cloned().unwrap_or_else(|| a.clone());
                gr_probe_store::result_commercial_identity_final(&r)
                    || r.get("commercial_identity_final")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
            })
            .unwrap_or(false);
        // Brain schedule final: analysis_completes_cycle (coverage+stop/terminal + silicon).
        // dh_/commercial alone never makes final_ok (maximize probe policy v5.8.53+).
        let brain_schedule_final = latest
            .as_ref()
            .map(|a| {
                let r = a.get("result").cloned().unwrap_or_else(|| a.clone());
                gr_probe_store::analysis_completes_cycle(&r)
            })
            .unwrap_or(false)
            || (snap.analysis_terminal && hard_complete && snap.cycle_closed);
        let final_ok = brain_schedule_final;
        map.insert(
            "identity_coverage".into(),
            json!({
                "needed": identity_needed,
                "present": present,
                "count_present": present.len(),
                "count_needed": identity_needed.len(),
                "complete_enough": has_b10 && present.len() >= 2,
                "has_b10": has_b10,
                "hard_complete": hard_complete,
                "final_analysis_ok": final_ok,
                "commercial_identity_final": commercial_final,
                "brain_schedule_final": brain_schedule_final,
            }),
        );
        map.insert("has_b10".into(), json!(has_b10));
        map.insert("hard_complete".into(), json!(hard_complete));
        map.insert("final_analysis_ok".into(), json!(final_ok));
        map.insert("commercial_identity_final".into(), json!(commercial_final));
        map.insert("brain_schedule_final".into(), json!(brain_schedule_final));
        let device_link_bound = latest
            .as_ref()
            .map(|a| {
                let r = a.get("result").cloned().unwrap_or_else(|| a.clone());
                r.get("device_link_bound")
                    .and_then(|v| v.as_bool())
                    .or_else(|| {
                        r.pointer("/link/details/device_link_bound")
                            .and_then(|v| v.as_bool())
                    })
                    .or_else(|| {
                        r.pointer("/link/decision")
                            .and_then(|v| v.as_str())
                            .map(|d| d == "LINK" || d == "MACHINE_BOUND")
                    })
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        map.insert("device_link_bound".into(), json!(device_link_bound));
        map.insert(
            "terminal_states".into(),
            json!({
                "commercial_identity_final": commercial_final,
                "brain_schedule_final": brain_schedule_final,
                "device_link_bound": device_link_bound,
            }),
        );
        // Authoritative halt only when brain schedule final OR cycle already closed.
        if final_ok || snap.cycle_closed {
            map.insert("halt_uploads".into(), json!(true));
            map.insert("skip_identity_probe".into(), json!(true));
            map.insert("accepts_identity_ingest".into(), json!(false));
            if snap.cycle_closed || final_ok {
                map.insert("business_state".into(), json!("identity_complete_cool"));
            }
        } else if !snap.cycle_closed {
            map.insert("halt_uploads".into(), json!(false));
            map.insert("skip_identity_probe".into(), json!(false));
            map.insert("accepts_identity_ingest".into(), json!(true));
            if commercial_final {
                map.insert("business_state".into(), json!("probing_after_identity_milestone"));
            }
        }
        map.insert(
            "received_batch_ids".into(),
            json!(facts.received_batch_ids),
        );
        map.insert(
            "received_batch_count".into(),
            json!(facts.received_batch_ids.len()),
        );
        map.insert("real_band".into(), real_band);
    }
    out
}

/// Multi-party status reconcile: FE reports local view, BE analyzes + corrects.
/// POST /v1/session/:id/probe_status
pub fn probe_status_reconcile(
    st: &AppState,
    session_id: &str,
    body: &Value,
    headers: &HashMap<String, String>,
) -> AppResult {
    require_session_site_auth(st, headers, session_id)?;
    let facts = server_cycle_facts(&st.store, session_id);
    let client = if body.is_null() || body.as_object().map(|o| o.is_empty()).unwrap_or(true) {
        None
    } else {
        Some(gr_probe_store::ClientCycleView {
            session_id: body
                .get("session_id")
                .and_then(|v| v.as_str())
                .unwrap_or(session_id)
                .to_string(),
            stop_probe: body.get("stop_probe").and_then(|v| v.as_bool()),
            skip_identity: body
                .get("skip_identity")
                .or_else(|| body.get("skip_identity_probe"))
                .and_then(|v| v.as_bool()),
            halted: body.get("halted").and_then(|v| v.as_bool()),
            phase: body
                .get("phase")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            local_uploads_done: body
                .get("local_uploads_done")
                .and_then(|v| v.as_bool()),
            sent_batch_ids: body
                .get("sent_batch_ids")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default(),
            last_http_status: body
                .get("last_http_status")
                .and_then(|v| v.as_u64())
                .map(|n| n as u16),
            last_error_code: body
                .get("last_error_code")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        })
    };
    let mut r = gr_probe_store::reconcile(&facts, client.as_ref());
    // Self-heal: if BE should complete cycle, do it now.
    if r.be_should_complete_cycle {
        match st.store.complete_cycle(session_id) {
            Ok(done) => {
                r.corrections.push(gr_probe_store::Correction {
                    action: "be_completed_cycle".into(),
                    reason: "self_heal_analysis_terminal_stuck".into(),
                    authority: "server".into(),
                });
                let facts2 = server_cycle_facts(&st.store, session_id);
                let r2 = gr_probe_store::reconcile(&facts2, client.as_ref());
                let mut out = gr_probe_store::reconcile_to_json(&r2, session_id);
                if let Some(map) = out.as_object_mut() {
                    map.insert("self_healed_complete".into(), json!(true));
                    map.insert("complete_cycle".into(), done);
                    // Merge first-pass corrections
                    let mut all = r
                        .corrections
                        .iter()
                        .chain(r2.corrections.iter())
                        .map(|c| {
                            json!({
                                "action": c.action,
                                "reason": c.reason,
                                "authority": c.authority,
                            })
                        })
                        .collect::<Vec<_>>();
                    all.dedup();
                    map.insert("corrections".into(), json!(all));
                }
                return Ok(out);
            }
            Err(e) => {
                r.corrections.push(gr_probe_store::Correction {
                    action: "be_complete_cycle_failed".into(),
                    reason: e.to_string(),
                    authority: "server".into(),
                });
            }
        }
    }
    Ok(gr_probe_store::reconcile_to_json(&r, session_id))
}

/// Re-export: trusted reverse proxy check (loopback always trusted for local nginx).
pub use gr_probe_core::peer_is_trusted_proxy;
/// Re-export: CF → True-Client-IP → XFF → peer.
pub use gr_probe_core::client_ip_from;
pub use gr_probe_core::client_ip_source;

/// Sealed probe upload: verify signature → decrypt → reverse compress → plain ingest path
/// (**bypasses** REQUIRE_SEALED gate — already sealed).
pub fn ingest_sealed(
    st: &AppState,
    headers: &HashMap<String, String>,
    body_bytes: &[u8],
) -> AppResult {
    let env: gr_probe_core::SealedEnvelope = serde_json::from_slice(body_bytes)
        .map_err(|e| ApiError(400, format!("invalid sealed envelope: {e}")))?;
    // Uniqueness: reject empty correlating ids
    if env.session_id.is_empty() || env.visitor_terminal_id.is_empty() || env.batch_id.is_empty() {
        return Err(ApiError(
            400,
            "session_id, visitor_terminal_id, batch_id required".into(),
        ));
    }
    let prev = gr_abi::env::get("SEAL_SECRET_PREV")
        .filter(|s| !s.trim().is_empty());
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let payload = gr_probe_core::unseal_probe_payload_auto(
        st.seal_secret.as_bytes(),
        prev.as_ref().map(|s| s.as_bytes()),
        &env,
        now_ms,
    )
    .map_err(|e| match e {
        gr_probe_core::SealError::BadSignature => ApiError(401, "invalid signature".into()),
        gr_probe_core::SealError::Expired => ApiError(401, "seal_grant_expired: re-open session".into()),
        gr_probe_core::SealError::TooLarge => ApiError(413, "decompressed payload exceeds limit".into()),
        gr_probe_core::SealError::Envelope(msg) => {
            // Strict policy rejects (suite/epoch/wasm/algo) — not a soft 400 parse error.
            let code = if msg.contains("seal_v1") || msg.contains("suite") || msg.contains("epoch")
                || msg.contains("wasm") || msg.contains("b10_algo") || msg.contains("fe_impl")
            {
                422
            } else {
                400
            };
            ApiError(code, format!("seal_policy: {msg}"))
        }
        other => ApiError(400, format!("unseal failed: {other}")),
    })?;
    let key_mode = env
        .key_mode
        .clone()
        .unwrap_or_else(|| "master".into());
    // Materialize as standard ingest body (plaintext after verify).
    // Payload may be either {fields,...} or full ingest-shaped {session_id,payload:...}.
    let inner_payload = if payload.get("payload").is_some() {
        payload.get("payload").cloned().unwrap_or(json!({}))
    } else {
        payload.clone()
    };
    let mut fields = inner_payload
        .get("fields")
        .cloned()
        .unwrap_or_else(|| {
            if inner_payload.get("fields").is_none() && payload.get("fields").is_some() {
                payload.get("fields").cloned().unwrap_or(json!({}))
            } else {
                json!({})
            }
        });
    if fields.as_object().is_none() {
        fields = json!({});
    }
    // site_id for ops join (v150 gap: sealed_ok had empty site_id).
    // Order: Host domain map → payload/fields → evidence later.
    let mut sealed_site = resolve_site_id_from_host(st, headers);
    if sealed_site.is_empty() {
        if let Some(s) = fields
            .get("site_id")
            .or_else(|| payload.get("site_id"))
            .or_else(|| inner_payload.get("site_id"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            sealed_site = s.to_string();
        }
    }
    let _ = st.store.insert_ops_server_event(json!({
        "code": "sealed_ok",
        "severity": "info",
        "stage": "ingest_sealed",
        "session_id": env.session_id,
        "visitor_terminal_id": env.visitor_terminal_id,
        "site_id": sealed_site,
        "product_version": gr_probe_core::GR_PRODUCT_VERSION,
        "detail_json": {
            "worker_id": st.worker_id,
            "batch_id": env.batch_id,
            "session_id": env.session_id,
            "visitor_terminal_id": env.visitor_terminal_id,
            "site_id": sealed_site,
            "key_mode": key_mode,
            "product_version": gr_probe_core::GR_PRODUCT_VERSION,
        },
    }));
    if let Some(fo) = fields.as_object_mut() {
        fo.entry("visitor_terminal_id".to_string())
            .or_insert_with(|| json!(env.visitor_terminal_id.clone()));
    }
    let mut full = if inner_payload.is_object() {
        inner_payload
    } else {
        json!({})
    };
    if let Some(obj) = full.as_object_mut() {
        obj.insert("fields".into(), fields);
    }
    let source = payload
        .get("source")
        .or_else(|| full.get("source"))
        .and_then(|v| v.as_str())
        .unwrap_or("main")
        .to_string();
    let inject_path = payload
        .get("inject_path")
        .or_else(|| full.get("inject_path"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let plain = IngestBody {
        session_id: env.session_id.clone(),
        batch_id: env.batch_id.clone(),
        source,
        payload: full.clone(),
        analyze: false,
        inject_path,
        capture_id: full
            .get("capture_id")
            .or_else(|| payload.get("capture_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        material_generation: full
            .get("material_generation")
            .or_else(|| payload.get("material_generation"))
            .and_then(|v| v.as_i64()),
        payload_hash: full
            .get("payload_hash")
            .or_else(|| payload.get("payload_hash"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        attempt_id: full
            .get("attempt_id")
            .or_else(|| payload.get("attempt_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        plan_epoch: full
            .get("plan_epoch")
            .or_else(|| payload.get("plan_epoch"))
            .and_then(|v| v.as_i64()),
        source_kind: full
            .get("source_kind")
            .or_else(|| payload.get("source_kind"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        realm_id: full
            .get("realm_id")
            .or_else(|| payload.get("realm_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        realm_kind: full
            .get("realm_kind")
            .or_else(|| payload.get("realm_kind"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        probe_method_id: full
            .get("probe_method_id")
            .or_else(|| payload.get("probe_method_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        method_version: full
            .get("method_version")
            .or_else(|| payload.get("method_version"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        observation_id: full
            .get("observation_id")
            .or_else(|| payload.get("observation_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        cookie_fields: full.get("cookie_fields").cloned().or_else(|| payload.get("cookie_fields").cloned()),
    };
    let body_bytes = serde_json::to_vec(&plain).map_err(|e| ApiError(500, e.to_string()))?;
    // Critical: do not call `ingest` (would 426 under REQUIRE_SEALED).
    let mut out = ingest_plain_body(st, headers, &body_bytes, true)?;
    if let Some(obj) = out.as_object_mut() {
        obj.insert("sealed".into(), json!(true));
        obj.insert("visitor_terminal_id".into(), json!(env.visitor_terminal_id));
        obj.insert("key_mode".into(), json!(key_mode));
    }
    Ok(out)
}

/// Cross-filter analysis + linked probe_cold fields (no full result_json scan).
/// Prefers `analysis_latest` materialized scalars (P0).
pub fn ops_cross_query(st: &AppState, query: &HashMap<String, String>) -> AppResult {
    let device_id = query.get("device_id").map(|s| s.as_str());
    let client_ip = query.get("client_ip").map(|s| s.as_str());
    let bot_verdict = query
        .get("bot_verdict")
        .or_else(|| query.get("robots"))
        .map(|s| s.as_str());
    let real_band = query.get("real_band").map(|s| s.as_str());
    let field_key = query.get("field_key").map(|s| s.as_str());
    let field_value = query.get("field_value").map(|s| s.as_str());
    let limit = query
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);
    let mut out = st.store.cross_query_analysis(
        device_id,
        client_ip,
        bot_verdict,
        real_band,
        field_key,
        field_value,
        limit,
    )?;
    if let Some(obj) = out.as_object_mut() {
        obj.insert("hot_vts".into(), json!(st.hot_probe.len_hot()));
    }
    Ok(out)
}

/// P1: commercial device master + session membership.
pub fn ops_device(st: &AppState, device_id: &str, query: &HashMap<String, String>) -> AppResult {
    if device_id.is_empty() {
        return Err(ApiError(400, "device_id required".into()));
    }
    let mut out = st.store.get_device(device_id)?;
    let with_sessions = query
        .get("sessions")
        .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
        .unwrap_or(true);
    let with_materials = query
        .get("materials")
        .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
        .unwrap_or(true);
    let lim = query
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);
    if with_sessions {
        if let Ok(sess) = st.store.list_device_sessions(device_id, lim) {
            if let Some(obj) = out.as_object_mut() {
                obj.insert("sessions".into(), sess);
            }
        }
    }
    // Soft heat (in-memory) for collision visibility
    let heat = commercial_id_heat_report(st.soft_store.as_ref(), "default", device_id);
    if let Some(obj) = out.as_object_mut() {
        obj.insert("soft_heat".into(), heat);
    }
    // Materials detail from last analysis (collision audit: included keys + digests + posture)
    if with_materials {
        let last_sid = out
            .pointer("/device/last_session_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                out.pointer("/sessions/rows/0/session_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            });
        if let Some(sid) = last_sid {
            if let Ok(Some(analysis)) = st.store.latest_analysis(&sid) {
                let trust = analysis
                    .pointer("/device/trust")
                    .or_else(|| analysis.pointer("/product/trust"))
                    .cloned()
                    .unwrap_or(json!({}));
                let materials_blob = json!({
                    "session_id": sid,
                    "device_id": analysis.pointer("/device/device_id")
                        .or_else(|| analysis.pointer("/product/device_id"))
                        .cloned()
                        .unwrap_or(json!(device_id)),
                    "device_tier": analysis.pointer("/device/device_tier")
                        .or_else(|| analysis.pointer("/product/device_tier"))
                        .cloned()
                        .unwrap_or(Value::Null),
                    "collision_risk": trust.get("collision_risk")
                        .or_else(|| analysis.pointer("/device/collision_risk"))
                        .or_else(|| analysis.pointer("/product/collision_risk"))
                        .cloned()
                        .unwrap_or(json!(false)),
                    "webgl_residual_entropy_ok": trust.get("webgl_residual_entropy_ok")
                        .cloned()
                        .unwrap_or(json!(false)),
                    "digest_path": trust.get("digest_path")
                        .or_else(|| analysis.pointer("/device/digest_path"))
                        .cloned()
                        .unwrap_or(Value::Null),
                    "materials_included": trust.get("materials_included")
                        .cloned()
                        .unwrap_or(json!([])),
                    "soft_extras_included": trust.get("soft_extras_included")
                        .cloned()
                        .unwrap_or(json!([])),
                    "analysis_posture": trust.get("analysis_posture")
                        .or_else(|| trust.get("eligibility_reasons"))
                        .cloned()
                        .unwrap_or(json!([])),
                    "id_warnings": trust.get("id_warnings").cloned().unwrap_or(json!([])),
                    "trust_sum": trust.get("trust_sum").or_else(|| trust.get("sum")).cloned(),
                    "has_both_curves": trust.get("has_both_curves").cloned(),
                    "webrtc_in_digest": trust.get("webrtc_in_digest").cloned(),
                    "keys": {
                        "form_class": trust.pointer("/materials/form_class").cloned(),
                        "hw_webgl_stable": trust.pointer("/materials/hw_webgl_stable").cloned(),
                        "hw_webgl_fine": trust.pointer("/materials/hw_webgl_fine").cloned(),
                        "hw_webgl_peak_sig": trust.pointer("/materials/hw_webgl_peak_sig").cloned(),
                        "hw_audio_stable": trust.pointer("/materials/hw_audio_stable").cloned(),
                        "hw_audio_fine": trust.pointer("/materials/hw_audio_fine").cloned(),
                        "residual_mean": trust.pointer("/materials/residual_mean").cloned(),
                        "residual_mean_bucket": trust.pointer("/materials/residual_mean_bucket").cloned(),
                        "cores_class": trust.pointer("/materials/cores_class").cloned(),
                        "architecture": trust.pointer("/materials/architecture").cloned(),
                        "os_family": trust.pointer("/materials/os_family").cloned(),
                        "os_instance_hash": trust.pointer("/materials/os_instance_hash").cloned(),
                        "webrtc_host_ip_hash": trust.pointer("/materials/webrtc_host_ip_hash").cloned(),
                        "unit_surface_id": trust.pointer("/materials/unit_surface_id").cloned(),
                    },
                    "source": "analysis_results.latest",
                    "webgl_comm_algo": "webgl_comm_v4",
                });
                if let Some(obj) = out.as_object_mut() {
                    obj.insert("materials".into(), materials_blob);
                }
            } else if let Some(obj) = out.as_object_mut() {
                obj.insert(
                    "materials".into(),
                    json!({
                        "ok": false,
                        "note": "no analysis_results for last_session; re-analyze or pass fields to materials_detail offline",
                        "last_session_id": sid,
                    }),
                );
            }
        }
    }
    Ok(out)
}

/// P0: list analysis_latest scalars (tier / time / site filters; no TOAST).
/// `site_id` filters server-side BEFORE the limit is applied (panel QA P3
/// fix, 2026-09-08: `limit=1&site_id=X` used to return 0 rows when the
/// globally-latest row belonged to another site).
pub fn ops_analysis_latest(st: &AppState, query: &HashMap<String, String>) -> AppResult {
    let limit = query
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);
    let tier = query.get("device_tier").map(|s| s.as_str());
    let since_ms = query.get("since_ms").and_then(|s| s.parse().ok());
    let site_id = query
        .get("site_id")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    st.store
        .list_analysis_latest(limit, tier, since_ms, site_id)
        .map_err(|e| ApiError(500, e.to_string()))
}

/// Panel Integrations "test query" (ops-gated): run the IP-enrichment
/// pipeline for one IP against the saved panel policy, or an incoming
/// `config.ip_enrichment` override (pre-save connectivity check).
pub fn ops_ip_enrichment_test(st: &AppState, body: &Value) -> AppResult {
    let ip = body
        .get("ip")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if ip.is_empty() {
        return Err(ApiError(400, "ip_required".into()));
    }
    let integrations = match body.get("config").and_then(|c| c.get("ip_enrichment")) {
        Some(ie) => json!({ "ip_enrichment": ie }),
        None => {
            let dir = panel_data_dir(st);
            gr_probe_core::load_panel_policy(dir.as_deref()).integrations
        }
    };
    Ok(fetch_ip_provider(&integrations, &ip))
}

/// Probe completeness board: main_complete / missing_probe by site × product_version.
///
/// `GET /v1/ops/probe_completeness?hours=24`
pub fn ops_probe_completeness(st: &AppState, query: &HashMap<String, String>) -> AppResult {
    let hours = query
        .get("hours")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(24)
        .clamp(1, 168 * 4);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let since = now - hours * 3600 * 1000;
    let mut body = st
        .store
        .ops_probe_completeness(since)
        .map_err(|e| ApiError(500, e.to_string()))?;
    if let Some(o) = body.as_object_mut() {
        o.insert("hours".into(), json!(hours));
        o.insert("generated_ms".into(), json!(now));
    }
    Ok(body)
}

/// Velocity windows for a device/ip.
///
/// `GET /v1/ops/velocity?device_id=…&client_ip=…`
pub fn ops_velocity(st: &AppState, query: &HashMap<String, String>) -> AppResult {
    let device_id = query.get("device_id").map(|s| s.as_str());
    let client_ip = query.get("client_ip").map(|s| s.as_str());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    st.store
        .velocity_summary(device_id, client_ip, now)
        .map_err(|e| ApiError(500, e.to_string()))
}

/// Ops: VT primary identity prefers silicon-complete session over latest B8-only.
///
/// `GET /v1/ops/vt_best_silicon?visitor_terminal_id=vt_…&limit=40`
/// Uses analysis_latest scalars + batch counts when available; selection is pure
/// `gr_probe_core::select_vt_best_silicon` (display/ops, not mint).
pub fn ops_vt_best_silicon(st: &AppState, query: &HashMap<String, String>) -> AppResult {
    let limit = query
        .get("limit")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(40)
        .clamp(1, 200);
    // Peer sessions for this VT (same bag family).
    let mut session_ids: Vec<String> = Vec::new();
    // Seed from a known session if provided; else scan analysis_latest.
    if let Some(sid) = query.get("session_id").map(|s| s.trim()).filter(|s| !s.is_empty()) {
        session_ids.push(sid.to_string());
        if let Ok(peers) = st.store.list_peer_session_ids(sid, limit as usize) {
            for p in peers {
                if !session_ids.contains(&p) {
                    session_ids.push(p);
                }
            }
        }
    }
    let mut vt = query
        .get("visitor_terminal_id")
        .or_else(|| query.get("vt"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_default();
    // Resolve VT from session when only session_id given (ops UI Load → VT best).
    if vt.is_empty() {
        for sid in &session_ids {
            if let Ok(Some(a)) = st.store.latest_analysis(sid) {
                if let Some(s) = a
                    .pointer("/fields/visitor_terminal_id")
                    .or_else(|| a.get("visitor_terminal_id"))
                    .or_else(|| a.pointer("/product/visitor_terminal_id"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                {
                    vt = s.to_string();
                    break;
                }
            }
            if let Ok(w) = st.store.session_window(sid) {
                if let Some(s) = w
                    .get("visitor_terminal_id")
                    .or_else(|| w.pointer("/meta/visitor_terminal_id"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                {
                    vt = s.to_string();
                    break;
                }
            }
        }
    }
    if vt.is_empty() {
        return Err(ApiError(
            400,
            "visitor_terminal_id required (or session_id with known VT)".into(),
        ));
    }
    let latest = st
        .store
        .list_analysis_latest(limit.max(100), None, None, None)
        .unwrap_or_else(|_| json!({"rows": []}));
    let rows = latest
        .get("rows")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut cands: Vec<Value> = Vec::new();
    for r in &rows {
        let rvt = r
            .get("visitor_terminal_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let sid = r
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if rvt != vt && !session_ids.iter().any(|s| s == sid) {
            continue;
        }
        if rvt != vt && !session_ids.is_empty() && !session_ids.iter().any(|s| s == sid) {
            continue;
        }
        if rvt != vt {
            continue;
        }
        let digest = r
            .get("digest_path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let provisional = digest.contains("empty_anchor")
            || digest.contains("gateway_only")
            || r.get("device_tier").and_then(|v| v.as_str()) == Some("dg");
        let mut batches_n = 0i64;
        if !sid.is_empty() {
            if let Ok(ev) = st.store.build_evidence(sid) {
                if let Some(arr) = ev.get("batches").and_then(|b| b.as_array()) {
                    batches_n = arr.len() as i64;
                }
            }
        }
        cands.push(json!({
            "session_id": sid,
            "visitor_terminal_id": rvt,
            "device_id": r.get("device_id"),
            "device_tier": r.get("device_tier"),
            "digest_path": digest,
            "real_band": r.get("real_band"),
            "created_ms": r.get("created_ms"),
            "mint_silicon_ok": r.get("mint_silicon_ok"),
            "provisional_gateway": provisional,
            "batches_n": batches_n,
            "algo_group_id": r.get("algo_group_id"),
        }));
    }
    // If analysis_latest empty for VT, fall back to peer session ids only.
    if cands.is_empty() && !session_ids.is_empty() {
        for sid in &session_ids {
            if let Ok(Some(a)) = st.store.latest_analysis(sid) {
                let digest = a
                    .pointer("/device/digest_path")
                    .or_else(|| a.pointer("/product/digest_path"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let tier = a
                    .pointer("/device/device_tier")
                    .or_else(|| a.pointer("/product/device_tier"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("dg");
                let provisional = a
                    .pointer("/device/provisional_gateway")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(digest.contains("empty_anchor") || tier == "dg");
                cands.push(json!({
                    "session_id": sid,
                    "visitor_terminal_id": vt,
                    "device_id": a.pointer("/device/device_id").or_else(|| a.pointer("/product/device_id")),
                    "device_tier": tier,
                    "digest_path": digest,
                    "created_ms": a.get("created_ms"),
                    "mint_silicon_ok": a.pointer("/device/mint_eligible"),
                    "provisional_gateway": provisional,
                    "batches_n": 0,
                    "algo_group_id": a.pointer("/device/algo_group_id"),
                }));
            }
        }
    }
    let selected = gr_probe_core::select_vt_best_silicon(&cands);
    Ok(json!({
        "ok": true,
        "visitor_terminal_id": vt,
        "candidates_n": cands.len(),
        "candidates": cands,
        "best": selected,
        "policy": "prefer_dh_silicon_over_latest_b8; ops_display_not_mint",
    }))
}

/// Ops: dump algorithm-group field registry (identity + score groups).
pub fn ops_algo_groups_registry(_st: &AppState) -> AppResult {
    Ok(json!({
        "ok": true,
        "registry": gr_probe_core::field_registry_json(),
    }))
}

/// P2/P3: reverse binder lookup (wg:… / au:… / lan:… / id:…).
pub fn ops_binder_lookup(st: &AppState, query: &HashMap<String, String>) -> AppResult {
    let key = query
        .get("binder_key")
        .or_else(|| query.get("key"))
        .map(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError(400, "binder_key required".into()))?;
    let tenant = query
        .get("tenant")
        .map(|s| s.as_str())
        .unwrap_or("default");
    let limit = query
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);
    st.store
        .lookup_devices_by_binder(tenant, key, limit)
        .map_err(|e| ApiError(500, e.to_string()))
}

/// Hot cache peek (lab/ops).
pub fn ops_hot_probe(st: &AppState, vtid: &str) -> AppResult {
    match st.hot_probe.get_by_vt(vtid) {
        Some(e) => Ok(json!({
            "ok": true,
            "visitor_terminal_id": e.visitor_terminal_id,
            "session_id": e.session_id,
            "client_ip": e.client_ip,
            "last_update_ms": e.last_update_ms,
            "batch_ids": e.batches.keys().collect::<Vec<_>>(),
            "hot": true,
        })),
        None => Ok(json!({"ok": true, "hot": false, "visitor_terminal_id": vtid})),
    }
}

/// Force idle demotion of hot probe material into `probe_cold` (lab/ops).
/// Query: `idle_ms` (default hot_idle_ms(); use 0 to demote everything).
pub fn ops_demote_idle(st: &AppState, query: &HashMap<String, String>) -> AppResult {
    let idle_ms = query
        .get("idle_ms")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or_else(hot_idle_ms)
        .max(0);
    let before = st.hot_probe.len_hot();
    let demoted = demote_hot_to_cold_opts(st, idle_ms, true);
    let after = st.hot_probe.len_hot();
    Ok(json!({
        "ok": true,
        "idle_ms": idle_ms,
        "hot_before": before,
        "demoted": demoted,
        "hot_after": after,
        "cold_ttl_ms": cold_ttl_ms(),
        "note": "L1 demote only; L2 batches + L3 cold dual-write on ingest. Re-ingest re-warms L1; promote-from-cold if L1 miss.",
    }))
}

/// 1.0.10 陈臂收割 (ops 触发版 — supervisor 每 5min 也会自动跑)。
/// Query: `idle_ms` (default 24h) — 已分析且超过该时长无租约活动的 pending 臂
/// + robots 会话的全部 pending 臂 (无时限: 爬虫判定粘性, 深探无增量)。
pub fn ops_harvest_arms(st: &AppState, query: &HashMap<String, String>) -> AppResult {
    let idle_ms = query
        .get("idle_ms")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(24 * 60 * 60 * 1000)
        .max(60_000);
    let pending_before = st.store.pending_analyze_job_count().unwrap_or(-1);
    let deleted = st.store.harvest_stale_arms(idle_ms)?;
    let pending_after = st.store.pending_analyze_job_count().unwrap_or(-1);
    Ok(json!({
        "ok": true,
        "idle_ms": idle_ms,
        "deleted": deleted,
        "pending_before": pending_before,
        "pending_after": pending_after,
        "rules": [
            "robots sessions: all pending arms (early-class result already closed them)",
            "analyzed sessions: pending arms idle > idle_ms"
        ],
        "note": "supervisor auto-runs this every 5min; this endpoint forces a sweep",
    }))
}

/// Panel retention: small-batch purge of aged analysis/sessions/velocity/cold.
///
/// Query: `batches` (1–20, default 1), optional `limit` override (10–5000).
/// Reads retention days from shared `panel_policy.json` (control-plane writes).
pub fn ops_retention_purge(st: &AppState, query: &HashMap<String, String>) -> AppResult {
    let batches = query
        .get("batches")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(1)
        .clamp(1, 20);
    let data_dir = panel_data_dir(st);
    let policy = gr_probe_core::load_panel_policy(data_dir.as_deref());
    let ret = &policy.retention;
    if !ret.enabled {
        return Ok(json!({
            "ok": true,
            "skipped": true,
            "reason": "retention.disabled",
            "retention": ret,
        }));
    }
    let limit = query
        .get("limit")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(ret.batch_delete_limit as i64)
        .clamp(10, 5000);
    let now = gr_probe_store::hot_now_ms();
    let analysis_days = ret.analysis_retention_days.max(1) as i64;
    let session_days = ret.session_retention_days.max(1) as i64;
    // Use the stricter (shorter) of analysis/session for the main cutoff when purging rows.
    let keep_days = analysis_days.min(session_days);
    let older_than_ms = now.saturating_sub(keep_days * 86_400_000);
    let velocity_days = ret.velocity_retention_days.max(1) as i64;
    let velocity_older = Some(now.saturating_sub(velocity_days * 86_400_000));
    // iss/opus5 04-P1-6: windows for the previously unbounded tables.
    let ops_older = Some(now.saturating_sub(ret.ops_retention_days.max(1) as i64 * 86_400_000));
    let master_older =
        Some(now.saturating_sub(ret.master_retention_days.max(1) as i64 * 86_400_000));
    const RET_KEYS: [&str; 17] = [
        "deleted_analysis",
        "deleted_analysis_latest",
        "deleted_sessions",
        "deleted_batches",
        "deleted_velocity",
        "deleted_cold",
        "deleted_page_results",
        "deleted_observation_events",
        "deleted_ops_client_events",
        "deleted_ops_server_events",
        "deleted_api_idempotency",
        "deleted_devices",
        "deleted_device_sessions",
        "deleted_soft_edges",
        "deleted_soft_heat",
        "deleted_device_index_devices",
        "deleted_device_index_keys",
    ];
    // Align cold TTL with policy when env not set higher priority — still purge cold in batch helper.
    let mut totals = json!({});
    if let Some(t) = totals.as_object_mut() {
        for k in RET_KEYS {
            t.insert(k.into(), json!(0i64));
        }
    }
    let mut batch_results = Vec::new();
    for i in 0..batches {
        let r = st
            .store
            .retention_purge_batch(older_than_ms, limit, velocity_older, ops_older, master_older)
            .map_err(|e| ApiError(500, e.to_string()))?;
        // Accumulate counts
        if let Some(t) = totals.as_object_mut() {
            for key in RET_KEYS {
                let add = r.get(key).and_then(|v| v.as_i64()).unwrap_or(0);
                let cur = t.get(key).and_then(|v| v.as_i64()).unwrap_or(0);
                t.insert(key.into(), json!(cur + add));
            }
        }
        let any = RET_KEYS
            .iter()
            .any(|k| r.get(*k).and_then(|v| v.as_i64()).unwrap_or(0) > 0);
        batch_results.push(json!({"batch": i + 1, "result": r}));
        if !any {
            break; // nothing left — stop early
        }
        // Small pause between batches inside one request to avoid IO spike
        if i + 1 < batches {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    Ok(json!({
        "ok": true,
        "batches_requested": batches,
        "batches_ran": batch_results.len(),
        "limit_per_batch": limit,
        "older_than_ms": older_than_ms,
        "keep_days": keep_days,
        "velocity_older_than_ms": velocity_older,
        "totals": totals,
        "batches": batch_results,
        "retention": ret,
        "backend": st.store.backend_name(),
        "note": "Deletes are LIMIT-batched; never full-table storms.",
    }))
}

/// iss/opus5 05-S-5: validate + normalize a DSAR subject selector.
/// Returns (kind, value) or an ApiError. Selector value is never returned in
/// responses or audit — only its sha256 fingerprint (GDPR: don't re-log PII).
fn dsar_validate_subject(kind_raw: &str, value_raw: &str) -> Result<(String, String, String), ApiError> {
    let kind = match kind_raw.trim() {
        "visitor_terminal_id" | "vt" => "visitor_terminal_id",
        "device_id" => "device_id",
        "client_ip" => "client_ip",
        "site_id" => "site_id",
        other => {
            return Err(ApiError(
                400,
                format!(
                    "unknown subject kind '{other}' (want visitor_terminal_id|device_id|client_ip|site_id)"
                ),
            ))
        }
    }
    .to_string();
    let value = value_raw.trim().to_string();
    if value.is_empty() {
        return Err(ApiError(400, "subject value required".into()));
    }
    if value.len() > 512 {
        return Err(ApiError(400, "subject value too long (>512)".into()));
    }
    // Reject control chars outright (audit-log safety).
    if value.chars().any(|ch| ch.is_control()) {
        return Err(ApiError(400, "subject value contains control characters".into()));
    }
    let fp = gr_probe_core::sha256_hex(value.as_bytes());
    let fp = fp.chars().take(16).collect::<String>();
    Ok((kind, value, fp))
}

/// iss/opus5 05-S-5: DSAR erasure — `POST /v1/ops/dsar/erase`
/// Body: {"kind": "visitor_terminal_id|device_id|client_ip|site_id", "value": "..."}
/// Cascade-deletes every row held for the subject across hot/warm/cold tables,
/// audits the action (selector fingerprinted, never raw), and relays the
/// control-plane site teardown when kind=site_id (best-effort).
pub fn ops_dsar_erase(st: &AppState, body: &[u8]) -> AppResult {
    let v: Value = serde_json::from_slice(body)
        .map_err(|e| ApiError(400, format!("bad json: {e}")))?;
    let kind_raw = v.get("kind").and_then(|x| x.as_str()).unwrap_or("");
    let value_raw = v.get("value").and_then(|x| x.as_str()).unwrap_or("");
    let (kind, value, fp) = dsar_validate_subject(kind_raw, value_raw)?;

    let res = st
        .store
        .subject_erase(&kind, &value)
        .map_err(|e| ApiError(500, e.to_string()))?;

    // Control-plane teardown for kind=site_id is the admin plane's job (it
    // owns sites/tenants); the admin DSAR wrapper performs both calls.
    if let Some(a) = st.admin.as_ref() {
        a.db.audit(
            "ops",
            "dsar.erase",
            &format!("{kind}:{fp}"),
            json!({
                "kind": kind,
                "selector_sha256_16": fp,
                "sessions_matched": res.get("sessions_matched").cloned().unwrap_or(json!(0)),
            }),
        );
    }
    Ok(json!({
        "ok": true,
        "dsar": "erase",
        "subject_kind": kind,
        "selector_sha256_16": fp,
        "result": res,
        "note": "erasure is irreversible; selector stored in audit as sha256 fingerprint only",
    }))
}

/// iss/opus5 05-S-5: DSAR access — `GET /v1/ops/dsar/export?kind=&value=`
/// Returns everything held for the subject as a JSON bundle.
pub fn ops_dsar_export(st: &AppState, query: &HashMap<String, String>) -> AppResult {
    let kind_raw = query.get("kind").map(|s| s.as_str()).unwrap_or("");
    let value_raw = query.get("value").map(|s| s.as_str()).unwrap_or("");
    let (kind, value, fp) = dsar_validate_subject(kind_raw, value_raw)?;
    let res = st
        .store
        .subject_export(&kind, &value)
        .map_err(|e| ApiError(500, e.to_string()))?;
    if let Some(a) = st.admin.as_ref() {
        a.db.audit(
            "ops",
            "dsar.export",
            &format!("{kind}:{fp}"),
            json!({
                "kind": kind,
                "selector_sha256_16": fp,
                "sessions_matched": res.get("export")
                    .and_then(|e| e.get("sessions_matched")).cloned().unwrap_or(json!(0)),
            }),
        );
    }
    Ok(json!({
        "ok": true,
        "dsar": "export",
        "subject_kind": kind,
        "selector_sha256_16": fp,
        "result": res,
    }))
}

/// Outcome distribution for admin Overview charts.
///
/// `GET /v1/ops/outcome_distribution?hours=24`
pub fn ops_outcome_distribution(st: &AppState, query: &HashMap<String, String>) -> AppResult {
    let hours = query
        .get("hours")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(24)
        .clamp(1, 168 * 4);
    let now = gr_probe_store::hot_now_ms();
    let since = now - hours * 3600 * 1000;
    let mut body = st
        .store
        .ops_outcome_distribution(since)
        .map_err(|e| ApiError(500, e.to_string()))?;
    if let Some(o) = body.as_object_mut() {
        o.insert("hours".into(), json!(hours));
        o.insert("generated_ms".into(), json!(now));
        // Attach effective default strategy for UI note
        let data_dir = panel_data_dir(st);
        let policy = gr_probe_core::load_panel_policy(data_dir.as_deref());
        o.insert(
            "panel_strategy_default".into(),
            json!(policy.strategy.default_strategy_id),
        );
    }
    Ok(body)
}

/// Explicit L3 cold TTL purge (also runs on analyze maintenance loop).
/// Query: `ttl_ms` optional override (default cold_ttl_ms).
pub fn ops_purge_expired_cold(st: &AppState, query: &HashMap<String, String>) -> AppResult {
    let ttl = query
        .get("ttl_ms")
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|&n| n >= 60_000)
        .unwrap_or_else(cold_ttl_ms);
    let now = gr_probe_store::hot_now_ms();
    let cutoff = now - ttl;
    let deleted = st
        .store
        .purge_expired_cold(cutoff)
        .map_err(|e| ApiError(500, e.to_string()))?;
    if let Some(admin) = st.admin.as_ref() {
        crate::admin::panel_config::record_purge(&admin.db, deleted, ttl);
    }
    Ok(json!({
        "ok": true,
        "deleted": deleted,
        "ttl_ms": ttl,
        "cutoff_ms": cutoff,
        "now_ms": now,
        "backend": st.store.backend_name(),
        "shared_across_workers": st.store.backend_name() == "postgres",
        "note": "probe_cold rows with created_ms < cutoff; multi-node PG DELETE is idempotent",
    }))
}

/// Timeout matrix (hot/cold/session/FE) for ops/FE alignment.
pub fn ops_timeouts(_st: &AppState) -> AppResult {
    Ok(json!({
        "ok": true,
        "timeouts": timeout_matrix_json(),
        "hot_vts_this_process": _st.hot_probe.len_hot(),
    }))
}

/// Force promote cold→L1 for a VT (lab).
pub fn ops_promote_cold(st: &AppState, query: &HashMap<String, String>) -> AppResult {
    let vt = query
        .get("visitor_terminal_id")
        .or_else(|| query.get("vtid"))
        .map(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError(400, "visitor_terminal_id required".into()))?;
    let sid = query
        .get("session_id")
        .map(|s| s.as_str())
        .unwrap_or("");
    // Force promote even if already hot: clear is_hot gate by using promote always
    let since = gr_probe_store::hot_now_ms() - cold_promote_window_ms().min(cold_ttl_ms());
    let listed = st.store.list_cold_for_vt(vt, since, 32)?;
    let rows = listed.get("rows").and_then(|r| r.as_array()).cloned().unwrap_or_default();
    let mut n = 0usize;
    for row in &rows {
        let batch_id = row.get("batch_id").and_then(|v| v.as_str()).unwrap_or("");
        if batch_id.is_empty() {
            continue;
        }
        let sess = row
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or(sid);
        let payload = row
            .get("payload_full")
            .cloned()
            .unwrap_or_else(|| json!({"fields": row.get("fields_json").cloned().unwrap_or(json!({}))}));
        let ip = row.get("client_ip").and_then(|v| v.as_str());
        st.hot_probe.promote_batch(vt, sess, batch_id, &payload, ip);
        n += 1;
    }
    Ok(json!({
        "ok": true,
        "visitor_terminal_id": vt,
        "promoted": n,
        "hot": st.hot_probe.is_hot(vt, hot_idle_ms()),
        "hot_batches": st.hot_probe.get_by_vt(vt).map(|e| e.batches.keys().cloned().collect::<Vec<_>>()),
    }))
}

/// Read cold row + decompress full payload (lab/ops verification).
pub fn ops_probe_cold(st: &AppState, query: &HashMap<String, String>) -> AppResult {
    let sid = query
        .get("session_id")
        .map(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError(400, "session_id required".into()))?;
    let batch_id = query
        .get("batch_id")
        .map(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError(400, "batch_id required".into()))?;
    let source = query.get("source").map(|s| s.as_str());
    Ok(st.store.get_probe_cold(sid, batch_id, source)?)
}

/// Batch vs cold storage volume for a session.
pub fn ops_probe_volume(st: &AppState, query: &HashMap<String, String>) -> AppResult {
    let sid = query
        .get("session_id")
        .map(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError(400, "session_id required".into()))?;
    Ok(st.store.probe_volume_stats(sid)?)
}

/// Coarse engine family from User-Agent (edge or FE).
fn ua_engine_family(ua: &str) -> &'static str {
    let u = ua.to_ascii_lowercase();
    if u.contains("firefox/") || u.contains("fxios") {
        return "firefox";
    }
    if u.contains("edg/") || u.contains("edgios") {
        return "edge";
    }
    if (u.contains("safari/") && !u.contains("chrome/") && !u.contains("chromium"))
        || u.contains("iphone") && u.contains("safari")
    {
        return "safari";
    }
    if u.contains("opr/") || u.contains("opera") {
        return "chrome"; // Opera is blink
    }
    if u.contains("chrome/") || u.contains("crios") || u.contains("chromium") {
        return "chrome";
    }
    "unknown"
}

fn ua_eng_matches_protocol(ua_eng: &str, protocol_eng: &str) -> bool {
    let p = protocol_eng.to_ascii_lowercase();
    match ua_eng {
        "firefox" => p.contains("firefox") || p == "gecko" || p.starts_with("ff"),
        "safari" => p.contains("safari") || p.contains("apple") || p == "webkit",
        "edge" => p.contains("edge") || p.contains("chrome") || p == "blink",
        "chrome" => {
            p.contains("chrome")
                || p == "blink"
                || p.contains("edge")
                || p == "unknown_tls13" // untagged modern TLS often chrome-class
        }
        _ => true,
    }
}

pub fn gateway_early(st: &AppState, headers: &HashMap<String, String>, body: GatewayEarlyBody) -> AppResult {
    let ua = headers.get("user-agent").map(|s| s.as_str()).unwrap_or("");
    let mut meta = merge_inject_meta(None, body.inject_path.clone());
    let mut site_id = body.site_id.clone().unwrap_or_default();
    // Multi-tenant: Host/domain → site_id when FE/micro kick omitted it.
    if site_id.is_empty() {
        site_id = resolve_site_id_from_host(st, headers);
    }
    if !site_id.is_empty() {
        if let Some(obj) = meta.as_object_mut() {
            obj.insert("site_id".into(), json!(site_id.clone()));
        }
    }
    // Binary board: robots if crawler UA; else browser (gateway early is usually JS micro-kick).
    meta = crate::admin::facet::merge_visit_class(
        Some(meta),
        &crate::admin::facet::ClassInputs {
            ua: ua.to_string(),
            is_pixel: false,
            has_fe_main: false,
            fe_open_hint: body
                .fields
                .get("micro_kick")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                || body
                    .fields
                    .get("early_kick")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            has_vtid: body
                .visitor_terminal_id
                .as_ref()
                .map(|s| !s.is_empty())
                .unwrap_or(false),
            backend_claims_js: false,
        },
    );
    let facet = meta
        .get("visitor_facet")
        .and_then(|v| v.as_str())
        .unwrap_or("browser")
        .to_string();
    let robot_name_gw = meta.get("robot_name").cloned().unwrap_or(Value::Null);
    let vt_hint = body.visitor_terminal_id.clone();
    // Require client cycle id for B8 — never mint an orphan bag per dual-fire/retry.
    // Missing session_id previously created dozens of B8-only cycles (no UA, no identity join).
    let cycle_hint = body
        .session_id
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if cycle_hint.is_none() {
        return Err(ApiError(
            400,
            "gateway early requires session_id (cycle bag) for join".into(),
        ));
    }
    // iss/opus5 S-14: bind an optional session ticket to this cycle. A valid
    // ticket claims skip_session_probe; an invalid/mismatched one grants
    // nothing (request proceeds unticketed, mirroring `open`).
    if let Some(ticket) = body.session_ticket.as_ref() {
        let sid_hint = cycle_hint.as_deref().unwrap_or("");
        let current_pv = gr_probe_core::GR_PRODUCT_VERSION;
        if gr_probe_core::validate_session_ticket_ex(
            ticket,
            sid_hint,
            Some(&body.fields),
            None,
            Some(current_pv),
            false,
        ) {
            if let Some(obj) = meta.as_object_mut() {
                obj.insert("session_ticket".into(), ticket.clone());
                obj.insert("skip_session_probe".into(), json!(true));
            }
        } else {
            let _ = st.store.insert_ops_server_event(json!({
                "code": "ticket_rejected",
                "severity": "info",
                "stage": "gateway_early",
                "session_id": sid_hint,
                "visitor_terminal_id": body.visitor_terminal_id.clone().unwrap_or_default(),
                "product_version": current_pv,
                "detail_json": {"reason": "validate_failed_or_version_or_silicon"},
            }));
        }
    }
    // Always stamp server product_version so early open cannot mint unversioned active bags.
    if let Some(obj) = meta.as_object_mut() {
        obj.entry("product_version")
            .or_insert_with(|| json!(gr_probe_core::GR_PRODUCT_VERSION));
        obj.entry("version")
            .or_insert_with(|| json!(gr_probe_core::GR_PRODUCT_VERSION));
        obj.insert("early_kick".into(), json!(true));
    }
    // Gateway early participates in the same cycle bag when vt known.
    let session = st.store.open_cycle(
        cycle_hint.clone(),
        body.visitor_terminal_id,
        Some(meta),
    )?;
    let vt_id = session
        .get("visitor_terminal_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or(vt_hint)
        .unwrap_or_default();
    if session
        .get("skip_identity_probe")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let sid = session
            .get("cycle_id")
            .or_else(|| session.get("session_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if !site_id.is_empty() && !vt_id.is_empty() {
            let biz = st.admin.as_ref().map(|a| a.biz.clone());
            crate::admin::biz_store::try_record_visit(
                &biz,
                crate::admin::biz_store::VisitUpsert {
                    site_id: site_id.clone(),
                    visitor_terminal_id: vt_id.clone(),
                    visitor_facet: facet.clone(),
                    session_id: sid.clone(),
                    page_host: headers.get("host").cloned().unwrap_or_default(),
                    ua_hash: crate::admin::biz_store::ua_hash(ua),
                    summary: json!({"source": "gateway_early", "skip": true}),
                    event: Some("facet".into()),
                    event_detail: json!({"visitor_facet": facet}),
                },
            );
        }
        return Ok(json!({
            "ok": true,
            "session_id": sid,
            "cycle_id": sid,
            "visitor_terminal_id": vt_id,
            "visitor_facet": facet,
            "site_id": if site_id.is_empty() { Value::Null } else { json!(site_id) },
            "phase": session.get("phase"),
            "skip_identity_probe": true,
            "kicked": null,
            "analyze_deferred": true,
            "note": "vt cool — identity gateway skip; page rpa still allowed",
            "last_identity_result": session
                .get("last_identity_result")
                .map(thin_identity_result)
                .unwrap_or(Value::Null),
        }));
    }
    let sid = session
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError(500, "missing session_id".into()))?
        .to_string();
    // Robots fast lane (1.0.10 root fix): UA-declared crawler — the verdict is
    // already final at the edge. No L1 hot, no L3 cold, no analyze arms; write
    // the early-class result and let the session expire by inactivity.
    let is_robot = robot_fastlane_active(st) && facet == "robots";
    if is_robot {
        mark_robot_session(st, &sid);
        try_robot_early_result(st, &sid, robot_name_gw.as_str(), "gateway_early");
    }
    let mut fields = if body.fields.is_object() {
        body.fields.clone()
    } else {
        json!({})
    };
    // Resolve authoritative client IP **first** (never trust body.fields spoof).
    let mut client_ip: Option<String> = None;
    if let Some(obj) = fields.as_object_mut() {
        // Denorm site_id onto gateway bag (ops board / analysis scalars).
        if !site_id.is_empty() {
            obj.entry("site_id").or_insert_with(|| json!(site_id.clone()));
        }
        // ── Edge-authoritative stamps (Pingora / nginx peer; never trust body spoof) ──
        // Gateway User-Agent: ALWAYS stamp as gateway_user_agent (source_trust GatewayEdge).
        // Also fill user_agent when empty so B8 has edge UA for soft conflict vs FE.
        if !ua.is_empty() {
            obj.insert("gateway_user_agent".into(), json!(ua));
            obj.insert("edge_user_agent".into(), json!(ua));
        }
        // iss/45 B11: Client Hints (sec-ch-ua*) when present on edge request
        for (hk, fk) in [
            ("sec-ch-ua", "sec_ch_ua"),
            ("sec-ch-ua-mobile", "sec_ch_ua_mobile"),
            ("sec-ch-ua-platform", "sec_ch_ua_platform"),
            ("sec-ch-ua-full-version-list", "sec_ch_ua_full_version_list"),
            ("sec-ch-ua-arch", "sec_ch_ua_arch"),
            ("sec-ch-ua-model", "sec_ch_ua_model"),
            ("sec-ch-ua-platform-version", "sec_ch_ua_platform_version"),
        ] {
            if let Some(v) = headers.get(hk).filter(|s| !s.is_empty()) {
                obj.insert(fk.into(), json!(v));
            }
        }
        if obj.contains_key("sec_ch_ua") {
            obj.insert("sec_ch_ua_present".into(), json!(true));
        }
        // HTTP method / version for JA4H partial
        if let Some(m) = headers.get("x-gr-http-method").filter(|s| !s.is_empty()) {
            obj.insert("http_method".into(), json!(m));
        } else {
            obj.insert("http_method".into(), json!("GET"));
        }
        if let Some(ver) = headers
            .get("x-gr-http-version")
            .or_else(|| headers.get(":version"))
            .filter(|s| !s.is_empty())
        {
            obj.insert("http_version".into(), json!(ver));
        }
        if headers.contains_key("cookie") || headers.contains_key("Cookie") {
            obj.insert("cookie_present".into(), json!(true));
        }
        if let Some(al) = headers
            .get("accept-language")
            .or_else(|| headers.get("Accept-Language"))
            .filter(|s| !s.is_empty())
        {
            obj.insert("gateway_accept_language".into(), json!(al));
            obj.insert("accept_language".into(), json!(al));
        }
        // iss/45 B8 JA4H: HTTP header name order (from http_util headers_map)
        if let Some(order) = headers
            .get("x-gr-http-header-order")
            .filter(|s| !s.is_empty())
        {
            obj.insert("http_header_order".into(), json!(order));
            obj.insert("ja4h_lite_order".into(), json!(order));
            obj.insert("ja4h_lite_role".into(), json!("header_order_conf_only"));
            obj.insert("ja4h_lite__diagnostic_only".into(), json!(false));
            obj.insert("ja4h_lite__commercial_mint".into(), json!(false));
            obj.insert("ja4h_lite__hard_browser_uniqueness".into(), json!(false));
            obj.insert("ja4h_lite__case_entropy_available".into(), json!(false));
            obj.insert("ja4h_lite__full_foxio".into(), json!(false));
        }
        if let Some(h) = headers
            .get("x-gr-http-header-order-hash")
            .filter(|s| !s.is_empty())
        {
            obj.insert("http_header_order_hash".into(), json!(h));
            let tag = if h.starts_with("h_") {
                h.clone()
            } else {
                format!("h_{h}")
            };
            obj.insert("ja4h_lite".into(), json!(tag));
        }
        // Normalize + JA4H partial composite
        gr_probe_core::ensure_ja4h_fields(obj);
        if !ua.is_empty() {
            let fe_ua_empty = !obj
                .get("user_agent")
                .and_then(|v| v.as_str())
                .map(|s| !s.is_empty())
                .unwrap_or(false);
            if fe_ua_empty {
                obj.insert("user_agent".into(), json!(ua));
            }
            // Explicit mismatch flag for BR demotion when body claimed a different UA
            let body_ua = obj
                .get("user_agent")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !body_ua.is_empty() && body_ua != ua {
                obj.insert("gateway_fe_ua_mismatch".into(), json!(true));
                obj.insert("fe_claimed_user_agent".into(), json!(body_ua));
                // Prefer edge UA for gateway bag identity (FE body cannot override)
                obj.insert("user_agent".into(), json!(ua));
            }
        }
        // HTTP request fingerprint (header order + accept family) — pure edge observation
        if let Some(ho) = headers.get("x-gr-http-header-order") {
            obj.insert("http_header_order".into(), json!(ho));
        }
        if let Some(hh) = headers.get("x-gr-http-header-order-hash") {
            obj.insert("http_header_order_hash".into(), json!(hh));
        }
        if let Some(al) = headers.get("accept-language") {
            obj.insert("gateway_accept_language".into(), json!(al));
        }
        if let Some(ae) = headers.get("accept-encoding") {
            obj.insert("gateway_accept_encoding".into(), json!(ae));
        }
        if let Some(ac) = headers.get("accept") {
            obj.insert("gateway_accept".into(), json!(ac));
        }
        // CF edge presence: only true when real CF headers exist (lab nginx has none)
        let cf_present = headers.get("cf-connecting-ip").map(|s| !s.is_empty()).unwrap_or(false)
            || headers.get("cf-ray").map(|s| !s.is_empty()).unwrap_or(false)
            || headers
                .get("cf-visitor")
                .map(|s| !s.is_empty())
                .unwrap_or(false);
        obj.insert("cf_edge_present".into(), json!(cf_present));
        obj.insert(
            "edge_transport".into(),
            json!(if headers.contains_key("x-tls-ja4") || headers.contains_key("x-gr-tls-ja4-source") {
                "pingora_tls"
            } else {
                "http_plain_or_upstream"
            }),
        );
        if let Some(src) = headers.get("x-gr-tls-ja4-source") {
            obj.insert("tls_ja4_source".into(), json!(src));
        }

        let (ip, _src) = apply_authoritative_client_ip(obj, headers);
        client_ip = ip.clone();
        // Tag CDN-proxied B8 (pv → gateway) so analytics can separate true-edge gv path.
        if let Some(via) = headers.get("x-gr-via-cdn") {
            obj.entry("b8_via_cdn").or_insert(json!(via));
        }
        // Country/ASN multi-source:
        // 1) CF when present  2) trusted x-asn  3) builtin heuristic denorm (loopback/private/cloud)
        if cf_present {
            if let Some(cc) = headers.get("cf-ipcountry").map(|s| s.as_str()) {
                obj.entry("server_country").or_insert(json!(cc));
                obj.insert("server_country_source".into(), json!("cf"));
            }
            if let Some(asn) = headers.get("cf-asn").map(|s| s.as_str()) {
                obj.entry("server_asn").or_insert(json!(asn));
                obj.insert("server_asn_source".into(), json!("cf"));
            }
        } else if let Some(asn) = headers.get("x-asn").map(|s| s.as_str()) {
            obj.entry("server_asn").or_insert(json!(asn));
            obj.insert("server_asn_source".into(), json!("x_asn"));
        }
        // Built-in / optional MMDB hook — does not overwrite CF/x-asn
        gr_probe_core::enrich_fields_if_empty(obj, client_ip.as_deref());
        // H13: trusted edge TLS/H2 fingerprints (never trust client-only invent)
        let mut hdr_map = serde_json::Map::new();
        for (name, val) in headers.iter() {
            hdr_map.insert(name.to_ascii_lowercase(), json!(val));
        }
        // Strip any client-claimed protocol fields before inject
        for k in [
            "ja4",
            "tls_ja4",
            "ja3",
            "h2_fingerprint",
            "protocol_engine",
            "protocol_fp_source",
            "cipher_suites_order",
            "tls_extensions_order",
            "ja4_r",
        ] {
            obj.remove(k);
        }
        gr_probe_core::inject_protocol_from_headers(obj, &hdr_map);
        // M2 (iss/74): client-hint headers (sec-ch-ua*) → gw_ua_ch_* canonical
        // keys so gateway coherence compares FE-claimed hints vs gateway-seen.
        gr_probe_core::inject_gateway_ua_ch(obj, &hdr_map);
        // Side-channel join (QUIC/SYN/WebRTC): IP + peer port when known (iss/50 G1).
        // Weak IP-only joins never hard-merge commercial identity.
        let peer_port = headers
            .get("x-gr-peer-port")
            .and_then(|s| s.parse::<u16>().ok())
            .or_else(|| {
                headers
                    .get("x-gr-peer-addr")
                    .and_then(|a| a.rsplit_once(':'))
                    .and_then(|(_, p)| p.parse().ok())
            })
            .or_else(|| {
                headers
                    .get("x-forwarded-port")
                    .and_then(|s| s.parse().ok())
            });
        let peer_dst = headers
            .get("x-gr-local-port")
            .and_then(|s| s.parse::<u16>().ok());
        let join_scope = crate::listen::JoinScope {
            client_ip: client_ip.clone(),
            src_port: peer_port,
            dst_port: peer_dst,
            conn_token: headers.get("x-gr-quic-dcid").cloned().or_else(|| {
                headers
                    .get("x-gr-conn-token")
                    .cloned()
            }),
        };
        crate::listen::enrich_protocol_fields_scoped(obj, &join_scope);
        // Re-assert IP after enrich (enrich must not leave spoof)
        if let Some(ref ip_s) = client_ip {
            obj.insert("server_client_ip".into(), json!(ip_s));
            obj.insert(
                "server_client_ip_source".into(),
                json!(client_ip_source(headers, peer_from_headers(headers))),
            );
        }
        // H2 capture extras from trusted headers (set by Pingora service layer)
        if headers.get("x-gr-h2-preface-ok").map(|s| s == "1").unwrap_or(false) {
            obj.insert("h2_settings_available".into(), json!(true));
            obj.insert("h2_preface_ok".into(), json!(true));
        }
        if let Some(wu) = headers.get("x-gr-h2-window-update").and_then(|s| s.parse::<u32>().ok()) {
            obj.insert("h2_connection_window_update".into(), json!(wu));
        }
        if let Some(hh) = headers.get("x-gr-h2-hash") {
            obj.insert("h2_fingerprint_hash".into(), json!(hh));
        }
        // Prefer full H2 fingerprint from x-h2-fingerprint if present
        if let Some(hf) = headers.get("x-h2-fingerprint") {
            obj.insert("h2_fingerprint".into(), json!(hf));
            obj.insert("h2_fingerprint_partial_v1".into(), json!(hf));
            if !obj.contains_key("protocol_fp_source") {
                obj.insert("protocol_fp_source".into(), json!("gr_h2_capture"));
            }
        }
        if let Some(p) = headers.get("x-gr-h2-priority-fp") {
            obj.insert("h2_priority_fingerprint".into(), json!(p));
            // iss/46 H2: never hard browser uniqueness / commercial mint
            obj.insert("h2_priority_fingerprint__diagnostic_only".into(), json!(true));
        }
        if let Some(seq) = headers.get("x-gr-h2-frame-seq") {
            let arr: Vec<u8> = seq
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();
            if !arr.is_empty() {
                obj.insert("h2_frame_type_sequence".into(), json!(arr));
            }
        }
        if let Some(v) = headers.get("x-gr-h2-capture-version") {
            obj.insert("h2_capture_version".into(), json!(v));
        }
        // TCP TLS depth from ClientHello ext parse (iss/45 B3)
        if let Some(ks) = headers.get("x-tls-key-share-groups") {
            obj.insert("tls_key_share_groups".into(), json!(ks));
        }
        if let Some(pm) = headers.get("x-tls-psk-modes") {
            obj.insert("tls_psk_modes".into(), json!(pm));
        }
        if headers.get("x-tls-ech").map(|s| s == "1").unwrap_or(false) {
            obj.insert("tls_ech_present".into(), json!(true));
        }
        if headers.get("x-tls-alps").map(|s| s == "1").unwrap_or(false) {
            obj.insert("tls_alps_present".into(), json!(true));
        }
        if headers
            .get("x-tls-session-ticket")
            .map(|s| s == "1")
            .unwrap_or(false)
        {
            obj.insert("tls_session_ticket".into(), json!(true));
        }
        if headers
            .get("x-tls-early-data")
            .map(|s| s == "1")
            .unwrap_or(false)
        {
            obj.insert("tls_early_data".into(), json!(true));
        }
        if let Some(pr) = headers.get("x-tls-ext-presence") {
            obj.insert("tls_ext_presence".into(), json!(pr));
        }
        if headers.get("x-gr-proxy-protocol").map(|s| s == "1").unwrap_or(false) {
            obj.insert("proxy_protocol_present".into(), json!(true));
            if let Some(v) = headers.get("x-gr-proxy-version") {
                obj.insert("proxy_protocol_version".into(), json!(v));
            }
        }
        // Full HTTP/3 application server.
        // G1: never stomp stronger port-scoped side enrich with weaker IP-only H3 inject.
        // Only fill **missing** h3_* keys from headers (scoped enrich wins when present).
        if headers.get("x-gr-h3-app").map(|s| s == "1").unwrap_or(false) {
            // Only fill missing keys — never overwrite port-scoped enrich (G1).
            if !obj.contains_key("h3_app_present") {
                obj.insert("h3_app_present".into(), json!(true));
            }
            if !obj.contains_key("h3_alpn") {
                if let Some(a) = headers.get("x-gr-h3-alpn") {
                    obj.insert("h3_alpn".into(), json!(a));
                }
            }
            if !obj.contains_key("h3_pseudo_order") {
                if let Some(o) = headers.get("x-gr-h3-pseudo-order") {
                    obj.insert("h3_pseudo_order".into(), json!(o));
                }
            }
            if !obj.contains_key("h3_settings_fp") {
                if let Some(fp) = headers.get("x-gr-h3-settings-fp") {
                    obj.insert("h3_settings_fp".into(), json!(fp));
                }
            }
            if !obj.contains_key("h3_rtt_ms") {
                if let Some(rtt) = headers.get("x-gr-h3-rtt-ms").and_then(|s| s.parse::<f64>().ok()) {
                    obj.insert("h3_rtt_ms".into(), json!(rtt));
                }
            }
            if !obj.contains_key("h3_src_port") {
                if let Some(sp) = headers.get("x-gr-h3-src-port").and_then(|s| s.parse::<u16>().ok()) {
                    obj.insert("h3_src_port".into(), json!(sp));
                }
            }
            if let Some(js) = headers.get("x-gr-h3-join-strength") {
                if !obj.contains_key("h3_header_join_strength") {
                    obj.insert("h3_header_join_strength".into(), json!(js));
                }
            }
            if !obj.contains_key("protocol_fp_source") {
                obj.insert("protocol_fp_source".into(), json!("gr_h3_app"));
            }
        }
        // Re-stamp protocol export honesty AFTER H2 extras + H3 materialization
        // so h3_pseudo_order always gets __diagnostic_only markers (iss/50 G1 skeptic).
        gr_probe_core::annotate_protocol_export_honesty(obj);
        // P-V3 TCP depth + SYN option order (p0f-class)
        gr_probe_core::merge_cf_into_gateway_fields(obj, headers);
        crate::listen::tcp_depth::apply_tcp_depth_to_fields(obj, headers);
        if let Some(opts) = headers.get("x-gr-tcp-syn-opts") {
            obj.insert("tcp_syn_option_order".into(), json!(opts));
        }
        // Peer IP for ops (distinct from client IP when behind nginx)
        if let Some(peer) = headers.get("x-gr-peer-ip") {
            obj.insert("gateway_peer_ip".into(), json!(peer));
        }
        // Cross: gateway UA engine vs JA4 protocol_engine (for BR / soft)
        let g_ua = obj
            .get("gateway_user_agent")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let pe = obj
            .get("protocol_engine")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if !g_ua.is_empty() {
            let ua_eng = ua_engine_family(&g_ua);
            obj.insert("gateway_ua_engine".into(), json!(ua_eng));
            if !pe.is_empty()
                && pe != "absent"
                && pe != "unknown"
                && pe != "unknown_tls13"
                && ua_eng != "unknown"
                && !ua_eng_matches_protocol(ua_eng, &pe)
            {
                obj.insert("gateway_ua_vs_ja4_mismatch".into(), json!(true));
            } else if ua_eng != "unknown"
                && (pe == "unknown_tls13" || pe == "unknown" || pe.is_empty())
            {
                // Fill soft protocol_engine hint from gateway UA when JA4 untagged
                obj.insert("protocol_engine_ua_hint".into(), json!(ua_eng));
            }
        }
    }
    let mut payload = json!({
        "fields": fields,
        "inject_path": body.inject_path,
        "early": true
    });
    let gw_env = gr_probe_core::stamp_observation_envelope(
        &mut payload,
        &sid,
        "B8_gateway",
        "gateway",
        body.inject_path.as_deref().or(Some("nginx")),
        &gr_probe_core::ClaimedEnvelope {
            source_kind: Some("gateway".into()),
            realm_kind: Some("none".into()),
            probe_method_id: Some("b8_gateway_tls".into()),
            method_version: Some("v1".into()),
            ..Default::default()
        },
    );
    // Also expose protocol fingerprints on evidence.gateway_fields for xsrc
    // Store merges dual-fire B8 (CDN join + edge TCP enrich) on re-upsert.
    let gateway_fields = fields.clone();
    let upsert = st.store.upsert_batch_with_ip_opts(
        &sid,
        "B8_gateway",
        "gateway",
        &payload,
        client_ip.as_deref(),
        is_robot,
    )?;
    let gw_obs = append_observation_event(
        st,
        &site_id,
        &sid,
        "B8_gateway",
        "gateway",
        &gw_env,
        None,
        None,
    )
    .ok();
    let mut cf_map = gr_probe_core::extract_cf_edge_fields(headers);
    let cf_edge_present = !cf_map.is_empty();
    if cf_edge_present {
        // 与 B8 主行同一挂钩: 只补空 (MMDB/内置启发式), 不覆盖 cf-* 头。
        // 仅在真有 CF 边缘证据时补 — 无条件补会把空 map 变非空, 凭空
        // 制造 cloudflare 观察行。178 实测: 该子观察行此前无任何 server_* 字段
        // (main/worker:d1 已由封签 ingest 挂钩覆盖)。
        gr_probe_core::enrich_fields_if_empty(&mut cf_map, client_ip.as_deref());
    }
    let mut cf_obs_id = Value::Null;
    if cf_edge_present {
        let mut cf_payload = json!({
            "fields": Value::Object(cf_map.clone()),
            "inject_path": "cloudflare",
            "early": true
        });
        let cf_env = gr_probe_core::stamp_observation_envelope(
            &mut cf_payload,
            &sid,
            "B8_gateway",
            "cloudflare",
            Some("cloudflare"),
            &gr_probe_core::ClaimedEnvelope {
                source_kind: Some("cloud_edge".into()),
                realm_kind: Some("none".into()),
                probe_method_id: Some("cf_http_headers".into()),
                method_version: Some("v1".into()),
                ..Default::default()
            },
        );
        let _ = st.store.upsert_batch_with_ip(
            &sid,
            "B8_gateway",
            "cloudflare",
            &cf_payload,
            client_ip.as_deref(),
        );
        if let Ok(o) = append_observation_event(
            st,
            &site_id,
            &sid,
            "B8_gateway",
            "cloudflare",
            &cf_env,
            None,
            None,
        ) {
            cf_obs_id = o.get("observation_id").cloned().unwrap_or(Value::Null);
        }
    }
    if !vt_id.is_empty() && !is_robot {
        let _ = promote_cold_to_hot_if_needed(st, &vt_id, &sid);
        st.hot_probe.upsert_batch(
            &vt_id,
            &sid,
            "B8_gateway",
            &payload,
            client_ip.as_deref(),
        );
        let _ = demote_hot_to_cold(st, hot_idle_ms());
    }
    // Persist gateway_fields into session meta for later evidence merge if store supports it
    let mut out = json!({
        "ok": true,
        "session_id": sid,
        "visitor_terminal_id": vt_id,
        "visitor_facet": facet,
        "site_id": if site_id.is_empty() { Value::Null } else { json!(site_id) },
        "kicked": "B8_gateway",
        "ingest": upsert,
        "analyze_deferred": !body.analyze,
        "gateway_fields": gateway_fields,
        "observation_id": gw_obs
            .as_ref()
            .and_then(|o| o.get("observation_id"))
            .cloned()
            .unwrap_or(Value::Null),
        "cf_edge_present": cf_edge_present,
        "cf_observation_id": cf_obs_id,
        "protocol_fp": {
            "ja4": fields.get("ja4"),
            "h2_fingerprint": fields.get("h2_fingerprint"),
            "protocol_engine": fields.get("protocol_engine"),
            "source": fields.get("protocol_fp_source"),
        }
    });
    if !site_id.is_empty() && !vt_id.is_empty() {
        let biz = st.admin.as_ref().map(|a| a.biz.clone());
        crate::admin::biz_store::try_record_visit(
            &biz,
            crate::admin::biz_store::VisitUpsert {
                site_id: site_id.clone(),
                visitor_terminal_id: vt_id.clone(),
                visitor_facet: facet.clone(),
                session_id: sid.clone(),
                page_host: headers.get("host").cloned().unwrap_or_default(),
                ua_hash: crate::admin::biz_store::ua_hash(ua),
                summary: json!({
                    "source": "gateway_early",
                    "visitor_facet": facet,
                    "robot_name": robot_name_gw,
                }),
                event: Some("facet".into()),
                event_detail: json!({"visitor_facet": facet, "robot_name": robot_name_gw}),
            },
        );
    }
    // Edge-first analyze only when FE identity evidence already present, or explicit
    // analyze flag for nojs/robots. B8-only FE races must not mint empty_anchor dg_ noise.
    let rec = st.store.list_received_batches(&sid).unwrap_or_default();
    let has_fe_identity = rec.iter().any(|b| {
        let id = b
            .get("batch_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        id.starts_with("B0")
            || id.starts_with("B10")
            || id.starts_with("B1_")
            || id.starts_with("B2_")
            || id.starts_with("B3_")
            || id == "B12_anti_camouflage"
    });
    let is_nojs_path = body
        .fields
        .get("micro_kick")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        == false
        && body
            .fields
            .get("early_kick")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        && !has_fe_identity;
    // Gateway path must NOT force 40ms analyze when FE identity arrives — that was
    // per-upload thrash. Arm idle quiet only; explicit analyze / nojs still fire.
    if body.analyze {
        let _ = st.store.schedule_analyze_merge(&sid, 0, AnalyzeDueMerge::Replace);
    } else if is_nojs_path {
        // Nojs/pixel: short delayed analyze without FE batches.
        let _ = st.store.schedule_analyze_merge(&sid, 80, AnalyzeDueMerge::PullEarlier);
    } else if has_fe_identity {
        // FE materials present on gateway early — only re-arm idle clock, not fire now.
        let _ = st.store.schedule_analyze_merge(
            &sid,
            ANALYZE_IDLE_UPLOAD_MS,
            AnalyzeDueMerge::IdleReset,
        );
    }
    if body.analyze && (has_fe_identity || is_nojs_path) {
        let evidence = st.store.build_evidence(&sid)?;
        let result = evaluate_session(&evidence, None, None, None, st.soft_v2_ready)
            .map_err(|e| ApiError(422, e))?;
        let rev = st.store.save_analysis(&sid, &result)?;
        persist_brain_control(&st.store, &sid, &result, rev);
        out.as_object_mut().unwrap().insert(
            "analysis".into(),
            json!({"rev": rev, "real_band": result.get("real_band")}),
        );
    }
    Ok(out)
}

pub fn analyze_session(
    st: &AppState,
    session_id: &str,
    body: AnalyzeBody,
    headers: &HashMap<String, String>,
) -> AppResult {
    require_session_site_auth(st, headers, session_id)?;
    // iss/opus5 04-P0-3: default response is the slim projection; the full
    // envelope (diagnostics/brain/coverage internals) requires verbose=1 AND
    // an active ops credential.
    let verbose_full = body.verbose.unwrap_or(false) && ops_grant_active(st, headers);
    // Enforce session window before analyze (same as ingest mutate path).
    let _ = st.store.require_active_session(&session_id)?;
    // Canonical evidence from server store only (G-P0-7) — never client-supplied evidence body.
    let mut evidence = st.store.build_evidence(&session_id)?;
    // Promote page_id from fields to evidence root for product page block
    if evidence.get("page_id").is_none() {
        if let Some(pid) = evidence.pointer("/fields/page_id").cloned() {
            if let Some(obj) = evidence.as_object_mut() {
                obj.insert("page_id".into(), pid);
            }
        }
    }
    // Short-visit close: FE signals pagehide without overwriting probe batches.
    if body.pagehide_flush.unwrap_or(false) {
        if let Some(obj) = evidence.as_object_mut() {
            obj.insert("pagehide_flush".into(), json!(true));
            let mut fields = obj.get("fields").cloned().unwrap_or(json!({}));
            if let Some(fo) = fields.as_object_mut() {
                fo.insert("pagehide_flush".into(), json!(true));
                fo.insert("rpa_flush_reason".into(), json!("pagehide"));
            }
            obj.insert("fields".into(), fields);
        }
        let _ = st.store.merge_session_meta(
            session_id,
            &json!({"pagehide_flush": true, "stop_reason": "pagehide"}),
        );
    }
    // Shared-algorithm reuse: analyze_mask_v1 — when the full evidence input
    // hash + product version match the last analysis, return the stored result
    // (skip evaluate). Falls back to the legacy silicon-materials fingerprint
    // only for sessions analyzed before masks existed (stored_mask.is_none()).
    let cur_mask = gr_probe_core::analyze_mask::build_mask(
        &evidence,
        gr_probe_core::GR_PRODUCT_VERSION,
    );
    let stored_mask: Option<Value> = st
        .store
        .session_meta(session_id)
        .ok()
        .flatten()
        .and_then(|m| m.get("analyzed_mask_v1").cloned());
    let force_reanalyze = body.pagehide_flush.unwrap_or(false);
    if !force_reanalyze {
        if let Ok(Some(prev)) = st.store.latest_analysis(session_id) {
            if gr_probe_core::analyze_mask::mask_matches(stored_mask.as_ref(), &cur_mask) {
                // Reuse marker + observability stamps (selection / analyze_dirty)
                // so a reused envelope is shaped exactly like a fresh analyze
                // (the stored slim form never carries those stamps).
                let mut reused = prev.clone();
                gr_probe_core::analyze_mask::stamp_observability(
                    &mut reused,
                    &evidence,
                    &session_id,
                    gr_probe_core::GR_PRODUCT_VERSION,
                    stored_mask.as_ref(),
                    &cur_mask,
                );
                if let Some(obj) = reused.as_object_mut() {
                    obj.insert(
                        "analyze_reuse".into(),
                        json!({
                            "algo": gr_probe_core::analyze_mask::ANALYZE_MASK_SCHEMA,
                            "skipped_evaluate": true,
                            "input_hash": gr_probe_core::analyze_mask::evidence_input_hash(
                                &evidence,
                            ),
                        }),
                    );
                }
                return Ok(if verbose_full {
                    reused
                } else {
                    gr_probe_core::response_slim::slim_analyze_result(&reused)
                });
            }
            if stored_mask.is_none() {
                // Legacy sessions: silicon fp cache (unchanged historical behavior).
                let prev_fp = prev
                    .pointer("/diagnostics/silicon_materials_fp")
                    .or_else(|| prev.pointer("/_storage/silicon_materials_fp"))
                    .or_else(|| prev.get("silicon_materials_fp"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let fields = evidence.get("fields").cloned().unwrap_or(json!({}));
                let cur_fp = gr_probe_core::silicon_materials_fingerprint(&fields, None);
                if !prev_fp.is_empty() && prev_fp == cur_fp {
                    // Legacy reuse: fp matched, no mask stored. Stamp the same
                    // observability envelope as a fresh analyze for consistent
                    // response shape.
                    let mut reused = prev.clone();
                    let input_hash = gr_probe_core::analyze_mask::evidence_input_hash(&evidence);
                    let sel = gr_probe_core::selection_provenance::build_selection_record(
                        &reused,
                        &session_id,
                        gr_probe_core::GR_PRODUCT_VERSION,
                        &input_hash,
                    );
                    if let Some(obj) = reused.as_object_mut() {
                        obj.insert(
                            "analyze_reuse".into(),
                            json!({
                                "algo": "silicon_materials_fp_v3",
                                "skipped_evaluate": true,
                                "fp": cur_fp,
                            }),
                        );
                        obj.insert("selection".into(), sel);
                        obj.insert(
                            "analyze_dirty".into(),
                            json!({
                                "algo": gr_probe_core::analyze_mask::ANALYZE_MASK_SCHEMA,
                                "changed_families": ["fields", "rest"],
                                "mask_matched": false,
                                "input_hash": input_hash,
                            }),
                        );
                    }
                    return Ok(if verbose_full {
                        reused
                    } else {
                        gr_probe_core::response_slim::slim_analyze_result(&reused)
                    });
                }
            }
        }
    }
    let (peer_vec, peer_ev, peer_ids) =
        resolve_multi_session_peers(&st.store, &session_id, &[]);
    // A-BRAIN-3 / norm/04: Soft amplify is server policy only — never trust body.soft_v2_ready.
    let mut result = evaluate_session(
        &evidence,
        None,
        peer_vec.as_ref(),
        peer_ev.as_ref(),
        st.soft_v2_ready,
    )
    .map_err(|e| ApiError(422, e))?;
    // Stamp materials fingerprint for next-call reuse
    {
        let fields = evidence.get("fields").cloned().unwrap_or(json!({}));
        let fp = gr_probe_core::silicon_materials_fingerprint(&fields, None);
        if let Some(obj) = result.as_object_mut() {
            let mut diag = obj.get("diagnostics").cloned().unwrap_or(json!({}));
            if let Some(d) = diag.as_object_mut() {
                d.insert("silicon_materials_fp".into(), json!(fp));
            }
            obj.insert("diagnostics".into(), diag);
        }
    }
    // Production multi-tenant DeviceIndex upsert (SQLite/PG) after successful analyze.
    let tenant_id = result
        .pointer("/session/tenant_id")
        .or_else(|| result.get("tenant_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_string();
    // Soft commercial heat (never promote) when multi-segment or legacy commercial id present.
    if let Some(dv) = result
        .pointer("/device/device_id")
        .or_else(|| result.pointer("/product/device_id"))
        .and_then(|v| v.as_str())
        .filter(|s| gr_probe_core::is_commercial_device_id(s))
        .map(|s| s.to_string())
    {
        let _ = st
            .soft_store
            .record_device_id_sighting(&tenant_id, &dv, session_id);
    }
    // Soft materials: window peers for peer_similarity score only (not account graph).
    if let Some(fields) = evidence.get("fields") {
        persist_soft_association_edges(st, &tenant_id, session_id, fields, &peer_ids);
    }
    // peer_similarity_v1: unit-time clustering severity across other vtids (independent field).
    let peer_similarity = {
        let mut self_fields = evidence.get("fields").cloned().unwrap_or(json!({}));
        // Stamp public device_id for segment/exact match in peer score
        if let Some(dv) = result
            .pointer("/device/device_id")
            .or_else(|| result.pointer("/product/device_id"))
            .cloned()
        {
            if let Some(fo) = self_fields.as_object_mut() {
                fo.entry("device_id".to_string()).or_insert(dv);
            }
        }
        let self_vtid = evidence
            .get("visitor_terminal_id")
            .or_else(|| evidence.pointer("/meta/visitor_terminal_id"))
            .or_else(|| self_fields.get("visitor_terminal_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let mut peer_pairs: Vec<(Option<String>, Value)> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut peer_id_list: Vec<String> = peer_ids.clone();
        // Time-window peers (other concurrent browsers / vtids on same host lab or farm).
        // list_peer_session_ids only covers same-VT / harness — window peers are required
        // for peer_similarity farm severity across chrome+firefox sequential matrix runs.
        if let Ok(recent) = st.store.list_recent_session_ids(48) {
            for pid in recent {
                if pid != session_id && !peer_id_list.contains(&pid) {
                    peer_id_list.push(pid);
                }
                if peer_id_list.len() >= 40 {
                    break;
                }
            }
        }
        for pid in &peer_id_list {
            if pid == session_id || !seen.insert(pid.clone()) {
                continue;
            }
            if let Ok(ev) = st.store.build_evidence(pid) {
                let pvt = ev
                    .get("visitor_terminal_id")
                    .or_else(|| ev.pointer("/meta/visitor_terminal_id"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                if let Some(pf) = ev.get("fields") {
                    // Prefer peers with host/silicon materials for scoring.
                    let fo = pf.as_object();
                    let useful = fo
                        .map(|o| {
                            o.contains_key("residual_mean")
                                || o.contains_key("hw_webgl_stable")
                                || o.contains_key("webrtc_host_ip_hash")
                                || o.contains_key("os_instance_hash")
                                || o.contains_key("hw_silicon_fine")
                        })
                        .unwrap_or(false);
                    if useful || fo.map(|o| o.len() >= 4).unwrap_or(false) {
                        peer_pairs.push((pvt, pf.clone()));
                    }
                }
            }
            if peer_pairs.len() >= 48 {
                break;
            }
        }
        // Soft edges as additional window materials
        if let Ok(edges) = st.soft_store.list_edges(&tenant_id) {
            for e in edges.into_iter().take(64) {
                if peer_pairs.len() >= 48 {
                    break;
                }
                let oid = if e.a_session == session_id {
                    e.b_session.clone()
                } else if e.b_session == session_id {
                    e.a_session.clone()
                } else {
                    continue;
                };
                if !seen.insert(oid.clone()) {
                    continue;
                }
                if let Ok(ev) = st.store.build_evidence(&oid) {
                    let pvt = ev
                        .get("visitor_terminal_id")
                        .or_else(|| ev.pointer("/meta/visitor_terminal_id"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    if let Some(pf) = ev.get("fields") {
                        peer_pairs.push((pvt, pf.clone()));
                    }
                }
            }
        }
        gr_probe_core::compute_peer_similarity(
            &self_fields,
            self_vtid.as_deref(),
            &peer_pairs,
            gr_probe_core::DEFAULT_WINDOW_SEC,
        )
    };
    // homogenization_v1: hot-bucket / same-host crowding as **product signal** (iss/50 H6).
    let homogenization = {
        let fields = evidence.get("fields").cloned().unwrap_or(json!({}));
        let env_class = fields
            .get("env_class")
            .or_else(|| fields.pointer("/env_signals/env_class"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let env_signals = fields
            .get("env_signals")
            .cloned()
            .unwrap_or_else(|| {
                json!({
                    "bucket_member_count": fields.get("bucket_member_count").cloned().unwrap_or(json!(0)),
                    "supercluster": fields.get("supercluster").cloned().unwrap_or(json!(false)),
                    "env_class": env_class,
                })
            });
        let cfg = gr_probe_core::SoftConfig::default();
        gr_probe_core::homogenization_product_signal(
            env_class,
            &env_signals,
            cfg.hot_bucket_threshold,
            cfg.p1_window_ms,
        )
    };
    // Attach peer_similarity + homogenization onto result product/device for SDK consumers.
    if let Some(obj) = result.as_object_mut() {
        obj.insert("peer_similarity".into(), peer_similarity.clone());
        obj.insert("homogenization".into(), homogenization.clone());
        if let Some(product) = obj.get_mut("product").and_then(|p| p.as_object_mut()) {
            product.insert("peer_similarity".into(), peer_similarity.clone());
            product.insert("homogenization".into(), homogenization.clone());
        }
        if let Some(device) = obj.get_mut("device").and_then(|d| d.as_object_mut()) {
            device.insert("peer_similarity".into(), peer_similarity.clone());
            device.insert("homogenization".into(), homogenization.clone());
        }
        // Re-apply slim projection so homogenization is on sdk_projection
        if let Some(product) = obj.get("product").cloned() {
            let gate = product.get("sdk_return").cloned().unwrap_or(json!({"emit": true}));
            let gated = gr_probe_core::apply_identity_return_gate(&product, &gate);
            if let Some(p) = obj.get_mut("product") {
                *p = gated;
            }
        }
    }
    // Analysis observability: selection provenance + dirty-mask stamp (analyze_mask_v1).
    // Stamped after all result assembly so candidates/winner reflect the final output.
    gr_probe_core::analyze_mask::stamp_observability(
        &mut result,
        &evidence,
        session_id,
        gr_probe_core::GR_PRODUCT_VERSION,
        stored_mask.as_ref(),
        &cur_mask,
    );
    // iss/opus5 03-P0-1: planned-vs-actual ledger → evidence_withheld band
    // (server-side liveness from cycle facts; applied before persist).
    apply_evidence_withheld(&st.store, session_id, &mut result);
    let rev = st.store.save_analysis(&session_id, &result)?;
    // Persist the updated analyzed mask for incremental-skip bookkeeping.
    let _ = st.store.merge_session_meta(
        &session_id,
        &json!({
            "analyzed_mask_v1": gr_probe_core::analyze_mask::build_mask(
                &evidence,
                gr_probe_core::GR_PRODUCT_VERSION,
            ),
            "analysis_rev": rev,
        }),
    );
    let _device_index = st
        .store
        .device_index_link_upsert_from_result(&tenant_id, &result);
    // iss/22 P1b: persist control plane warm-start (belief / battle_log / priors)
    persist_brain_control(&st.store, &session_id, &result, rev);
    // Persist page-scoped result when page_id present (G-PROD-2)
    if let Some(page) = result.get("page") {
        if let Some(pid) = page.get("page_id").and_then(|v| v.as_str()) {
            let prev = page
                .get("page_rev")
                .and_then(|v| v.as_i64())
                .unwrap_or(1);
            let _ = st.store.save_page_result(&session_id, pid, prev, page);
        }
    }
    // Store cool-down ticket only when issued (silicon-grade); never force skip without ticket.
    if let Some(ticket) = result.get("session_ticket").filter(|t| t.is_object()) {
        let _ = st.store.merge_session_meta(
            &session_id,
            &json!({
                "session_ticket": ticket,
                "skip_session_probe": true,
            }),
        );
    } else {
        let _ = st.store.merge_session_meta(
            &session_id,
            &json!({
                "skip_session_probe": false,
            }),
        );
    }
    // Persist ops_events from evaluate (B10 SLA, ticket blocks, …)
    if let Some(arr) = result.get("ops_events").and_then(|v| v.as_array()) {
        let vt = evidence
            .get("visitor_terminal_id")
            .or_else(|| evidence.pointer("/meta/visitor_terminal_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let pv = evidence
            .get("product_version")
            .or_else(|| evidence.pointer("/meta/product_version"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let eng = evidence
            .pointer("/fields/engine_family")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let site = evidence
            .get("site_id")
            .or_else(|| evidence.pointer("/meta/site_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        for ev in arr {
            let code = ev.get("code").and_then(|v| v.as_str()).unwrap_or("analyze_ops");
            // Always stamp vtid/session into detail for retry-outcome joins.
            let mut detail = ev.clone();
            if let Some(obj) = detail.as_object_mut() {
                if !vt.is_empty() {
                    obj.entry("visitor_terminal_id")
                        .or_insert_with(|| json!(vt));
                }
                obj.entry("session_id")
                    .or_insert_with(|| json!(session_id));
                if !pv.is_empty() {
                    obj.entry("product_version")
                        .or_insert_with(|| json!(pv));
                }
                if !site.is_empty() {
                    obj.entry("site_id").or_insert_with(|| json!(site));
                }
            }
            let sev_in = detail
                .get("severity")
                .and_then(|v| v.as_str())
                .unwrap_or("warn");
            let (code_n, sev_n, detail_n) = gr_probe_core::enrich_ops_event(code, sev_in, &detail);
            let _ = st.store.insert_ops_server_event(json!({
                "code": code_n,
                "severity": sev_n,
                "stage": detail_n.get("stage").cloned().unwrap_or(json!("analyze")),
                "session_id": session_id,
                "visitor_terminal_id": vt,
                "site_id": site,
                "product_version": pv,
                "engine_family": eng,
                "detail_json": detail_n,
            }));
        }
    }
    let cycle_complete = st
        .store
        .maybe_complete_cycle_from_analysis(&session_id, &result)
        .ok()
        .flatten();
    let history_count = st
        .store
        .list_analyses(&session_id)
        .map(|h| h.len())
        .unwrap_or(0);
    let probe_status = cycle_probe_status_snapshot(st, &session_id);
    let closes = gr_probe_store::analysis_completes_cycle(&result) || cycle_complete.is_some();
    let halt = closes
        || probe_status
            .get("halt_uploads")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    // iss/opus5 04-P0-3: slim default projection (route/coverage/terminal keys
    // + identity conclusions); verbose=1 + ops credential returns the full
    // envelope including diagnostics/brain internals.
    let (result_out, device_out, product_out, diag_out) = if verbose_full {
        (
            result.clone(),
            result.get("device").cloned().unwrap_or(Value::Null),
            result.get("product").cloned().unwrap_or(Value::Null),
            result.get("diagnostics").cloned().unwrap_or(Value::Null),
        )
    } else {
        (
            gr_probe_core::response_slim::slim_analyze_result(&result),
            result
                .get("device")
                .map(gr_probe_core::response_slim::slim_device)
                .unwrap_or(Value::Null),
            result
                .get("product")
                .map(gr_probe_core::response_slim::slim_product)
                .unwrap_or(Value::Null),
            Value::Null,
        )
    };
    // iss/opus5 06-P1-5: monthly per-site session counter (billing signal).
    // Soft quota semantics: the analyze response is never withheld; usage rides
    // along so the panel/SDK can surface quota pressure (degrade, not interrupt).
    let usage_bucket = lookup_session_site(st, session_id)
        .filter(|s| !s.is_empty())
        .and_then(|site| st.store.bump_monthly_sessions(&site).ok());
    Ok(json!({
        "ok": true,
        "usage": usage_bucket.map(|(m, n)| json!({"month": m, "sessions": n})).unwrap_or(Value::Null),
        "session_id": session_id,
        "cycle_id": session_id,
        "rev": rev,
        "analysis_rev": rev,
        "history_count": history_count,
        "result": result_out,
        "route_plan": result.get("route_plan"),
        "device": device_out,
        "product": product_out,
        "diagnostics": diag_out,
        "page": result.get("page"),
        "session_ticket": result.get("session_ticket"),
        "skip_session_probe": result.get("skip_session_probe").cloned().unwrap_or(json!(halt)),
        "skip_identity_probe": halt,
        "halt_uploads": halt,
        "cycle_closes": closes,
        "cycle_complete": cycle_complete,
        "probe_complete": result.get("probe_complete"),
        "analysis_terminal": result.get("analysis_terminal"),
        "cycle_probe_status": probe_status,
        "sdk_return": result.get("sdk_return"),
        "rpa_analyze": result.get("rpa_analyze"),
        "real_band": result.get("real_band"),
        "link": result.get("link"),
        "multi_session_peers": peer_ids,
        "peer_similarity": result.get("peer_similarity"),
        "inject_path": evidence.pointer("/meta/inject_path"),
        "batch_count": evidence.get("batches").and_then(|b| b.as_array()).map(|a| a.len()),
        "evidence_authority": "server_store",
        "last_upload_ms": evidence.get("last_upload_ms"),
    }))
}

/// Brain-owned cycle complete → cool (requires silicon for cool_until > 0).
/// Lab/ops path; same store `complete_cycle` used by maybe_complete_cycle_from_analysis.
pub fn complete_session(
    st: &AppState,
    session_id: &str,
    headers: &HashMap<String, String>,
) -> AppResult {
    require_session_site_auth(st, headers, session_id)?;
    // Allow complete even if window already soft-expired — complete_cycle is authoritative.
    let done = st
        .store
        .complete_cycle(session_id)
        .map_err(|e| match e {
            StoreError::NotFound(m) => ApiError(404, m),
            other => ApiError(500, other.to_string()),
        })?;
    Ok(json!({
        "ok": true,
        "session_id": session_id,
        "complete_cycle": done,
        "cool_until_ms": done.get("cool_until_ms"),
        "cool_silicon_ok": done.get("cool_silicon_ok"),
        "phase": done.get("phase"),
        "brain_cool_owner": true,
    }))
}

pub fn list_pages(st: &AppState, session_id: &str, headers: &HashMap<String, String>) -> AppResult {
    require_session_result_auth(st, headers, session_id)?;
    let pages = st.store.list_page_results(&session_id)?;
    Ok(json!({
        "ok": true,
        "session_id": session_id,
        "pages": pages,
    }))
}

pub fn get_page(st: &AppState, session_id: &str, page_id: &str, headers: &HashMap<String, String>) -> AppResult {
    require_session_result_auth(st, headers, session_id)?;
    let page = st
        .store
        .latest_page_result(&session_id, &page_id)?
        .ok_or_else(|| ApiError(404, format!("page {page_id}")))?;
    Ok(json!({
        "ok": true,
        "session_id": session_id,
        "page_id": page_id,
        "page": page,
    }))
}

/// Minimal first-party 1×1 pixel (GA4/AdSense-style hit stub).
pub fn pixel_hit(
    st: &AppState,
    headers: &HashMap<String, String>,
    q: &HashMap<String, String>,
) -> (u16, &'static str, Vec<u8>) {
    let sid = q.get("sid").cloned().unwrap_or_else(|| format!("sess_{}", now_hex()));
    let event = q.get("e").cloned().unwrap_or_else(|| "page_view".into());
    let inject_path = q.get("inject_path").cloned();
    let site_id = q.get("site_id").cloned().unwrap_or_default();
    let ua = q
        .get("ua")
        .cloned()
        .or_else(|| headers.get("user-agent").cloned())
        .unwrap_or_default();
    let mut meta = merge_inject_meta(Some(json!({"pixel": true})), inject_path.clone());
    if !site_id.is_empty() {
        if let Some(obj) = meta.as_object_mut() {
            obj.insert("site_id".into(), json!(site_id.clone()));
        }
    }
    meta = crate::admin::facet::merge_facet_meta(Some(meta), &ua, true, false);
    let facet = meta
        .get("visitor_facet")
        .and_then(|v| v.as_str())
        .unwrap_or("browser")
        .to_string();
    let robot_name_px = meta.get("robot_name").cloned().unwrap_or(Value::Null);
    let vt_id = q
        .get("vt")
        .or_else(|| q.get("visitor_terminal_id"))
        .cloned()
        .unwrap_or_else(|| format!("px_{}", now_hex()));
    if let Some(obj) = meta.as_object_mut() {
        obj.insert("visitor_terminal_id".into(), json!(vt_id.clone()));
    }
    let _ = st
        .store
        .open_session(Some(sid.clone()), Some(vt_id.clone()), Some(meta));
    // Robots fast lane: crawler-UA pixel hits get the early result and skip
    // the cold tier (no future promote value — robots mint fresh sessions).
    let is_robot = robot_fastlane_active(st) && facet == "robots";
    if is_robot {
        mark_robot_session(st, &sid);
        try_robot_early_result(st, &sid, robot_name_px.as_str(), "pixel");
    }
    let _ = st.store.upsert_batch_with_ip_opts(
        &sid,
        "B_ops_hit",
        "main",
        &json!({
            "fields": {
                "ops_event": event,
                "pixel": true,
                "inject_path": inject_path,
                "visitor_facet": facet,
                "site_id": site_id,
                "qs": q
            }
        }),
        None,
        is_robot,
    );
    if !site_id.is_empty() {
        let biz = st.admin.as_ref().map(|a| a.biz.clone());
        crate::admin::biz_store::try_record_visit(
            &biz,
            crate::admin::biz_store::VisitUpsert {
                site_id: site_id.clone(),
                visitor_terminal_id: vt_id.clone(),
                visitor_facet: facet.clone(),
                session_id: sid.clone(),
                page_host: headers.get("host").cloned().unwrap_or_default(),
                ua_hash: crate::admin::biz_store::ua_hash(&ua),
                summary: json!({
                    "source": "pixel",
                    "event": event,
                    "visitor_facet": facet,
                    "robot_name": robot_name_px,
                }),
                event: Some("sdk_load".into()),
                event_detail: json!({"channel": "pixel", "visitor_facet": facet}),
            },
        );
    }
    // 1x1 GIF
    const GIF: &[u8] = &[
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0xff, 0xff,
        0xff, 0x00, 0x00, 0x00, 0x21, 0xf9, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x2c, 0x00, 0x00,
        0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x02, 0x02, 0x44, 0x01, 0x00, 0x3b,
    ];
    (200, "image/gif", GIF.to_vec())
}

/// SDK / server upload of website business visit (gr_biz only).
pub fn biz_visit(
    st: &AppState,
    headers: &HashMap<String, String>,
    body: BizVisitBody,
) -> AppResult {
    let admin = st
        .admin
        .as_ref()
        .ok_or_else(|| ApiError(503, "biz_store_unavailable".into()))?;
    let key = headers
        .get("x-gr-sdk-key")
        .or_else(|| headers.get("X-Gr-Sdk-Key"))
        .map(|s| s.as_str());
    let host = headers.get("host").map(|s| s.as_str());
    let origin = headers.get("origin").map(|s| s.as_str());
    crate::admin::sdk::check_biz_visit_auth(&admin.db, &body.site_id, key, host, origin)
        .map_err(|e| ApiError(401, e))?;
    let ua_biz = headers
        .get("user-agent")
        .map(|s| s.as_str())
        .unwrap_or("");
    // Prefer server UA classify; client may send js/nojs — normalize to browser|robots.
    let facet = {
        let from_client = body.visitor_facet.as_deref().unwrap_or("");
        let classified = crate::admin::facet::classify_visit_class(
            &crate::admin::facet::ClassInputs {
                ua: ua_biz.to_string(),
                fe_open_hint: true,
                has_vtid: !body.visitor_terminal_id.is_empty(),
                ..Default::default()
            },
        );
        if classified.facet == "robots" {
            "robots".to_string()
        } else if from_client == "robots" {
            "robots".to_string()
        } else {
            "browser".to_string()
        }
    };
    let event = body
        .event
        .clone()
        .or_else(|| {
            body.events
                .as_ref()
                .and_then(|ev| ev.first().cloned())
        })
        .unwrap_or_else(|| "facet".into());
    let page_host = body.page_host.clone().unwrap_or_else(|| {
        origin
            .and_then(|o| {
                o.trim_start_matches("https://")
                    .trim_start_matches("http://")
                    .split('/')
                    .next()
                    .map(|s| s.to_string())
            })
            .unwrap_or_default()
    });
    let out = admin
        .biz
        .upsert_visit(&crate::admin::biz_store::VisitUpsert {
            site_id: body.site_id.clone(),
            visitor_terminal_id: body.visitor_terminal_id.clone(),
            visitor_facet: facet.clone(),
            session_id: body.session_id.clone().unwrap_or_default(),
            page_host,
            ua_hash: crate::admin::biz_store::ua_hash(
                headers.get("user-agent").map(|s| s.as_str()).unwrap_or(""),
            ),
            summary: body.summary.unwrap_or_else(|| {
                json!({
                    "source": "sdk_biz_visit",
                    "events": body.events.clone().unwrap_or_default(),
                })
            }),
            event: Some(event),
            event_detail: json!({
                "visitor_facet": facet,
                "events": body.events.unwrap_or_default(),
            }),
        })
        .map_err(|e| ApiError(400, e))?;
    Ok(out)
}

pub fn now_hex() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{ms:x}")
}

/// Whether ops read/write (except client_event upload) requires a credential.
/// On when: prod deploy **or** `GR_REQUIRE_OPS_AUTH=1/true`.
pub fn ops_auth_required() -> bool {
    gr_abi::env::flag("REQUIRE_OPS_AUTH")
        || matches!(
            gr_abi::env::get("DEPLOY_ENV")
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "prod" | "production" | "live"
        )
}

fn header_ci<'a>(headers: &'a HashMap<String, String>, name: &str) -> &'a str {
    let lower = name.to_ascii_lowercase();
    headers
        .get(name)
        .or_else(|| headers.get(&lower))
        .or_else(|| {
            headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v)
        })
        .map(|s| s.as_str())
        .unwrap_or("")
}

fn presented_ops_secrets(headers: &HashMap<String, String>) -> Vec<String> {
    let mut out = Vec::new();
    let auth = header_ci(headers, "authorization");
    if let Some(b) = auth
        .strip_prefix("Bearer ")
        .or_else(|| auth.strip_prefix("bearer "))
    {
        let t = b.trim();
        if !t.is_empty() {
            out.push(t.to_string());
        }
    }
    for k in [
        "x-gr-ops-token",
        "X-Gr-Ops-Token",
        "x-gr-result-token",
        "X-Gr-Result-Token",
        "x-business-token",
        "X-Business-Token",
        "x-gr-sdk-key",
        "X-Gr-Sdk-Key",
        "x-session-token",
        "X-Session-Token",
    ] {
        let v = header_ci(headers, k).trim();
        if !v.is_empty() && !out.iter().any(|x| x == v) {
            out.push(v.to_string());
        }
    }
    // query-style not used — headers only
    out
}

fn secret_matches_env(presented: &[String]) -> bool {
    let mut allowed: Vec<String> = Vec::new();
    for name in ["OPS_TOKEN", "RESULT_TOKEN"] {
        if let Some(v) = gr_abi::env::get(name) {
            let t = v.trim();
            if !t.is_empty() {
                allowed.push(t.to_string());
            }
        }
    }
    if let Some(map) = gr_abi::env::get("SITE_RESULT_TOKENS") {
        for part in map.split(',') {
            let mut it = part.splitn(2, ':');
            let _site = it.next().unwrap_or("");
            let tok = it.next().unwrap_or("").trim();
            if !tok.is_empty() {
                allowed.push(tok.to_string());
            }
        }
    }
    if allowed.is_empty() {
        return false;
    }
    // iss/opus5 05 low: constant-time comparison (remote timing over HTTP).
    presented
        .iter()
        .any(|p| allowed.iter().any(|a| gr_probe_core::session_ticket::token_eq(a, p)))
}

/// Gate sensitive `/v1/ops/*` (not client_event) and webhook outbox admin.
/// Accepts: `GR_OPS_TOKEN`/`GR_RESULT_TOKEN` · site result map · admin session.
pub fn require_ops_auth(st: &AppState, headers: &HashMap<String, String>) -> Result<(), ApiError> {
    if !ops_auth_required() {
        return Ok(());
    }
    let presented = presented_ops_secrets(headers);
    if secret_matches_env(&presented) {
        return Ok(());
    }
    // Admin console session escape hatch removed (P3 schema convergence):
    // ops/result secrets are the only path; mx-console sessions no longer exist.
    // Production with no secrets configured: fail closed.
    let has_any_secret = !gr_abi::env::get("OPS_TOKEN").unwrap_or_default().trim().is_empty()
        || !gr_abi::env::get("RESULT_TOKEN").unwrap_or_default().trim().is_empty()
        || !gr_abi::env::get("SITE_RESULT_TOKENS")
            .unwrap_or_default()
            .trim()
            .is_empty();
    if !has_any_secret && st.admin.is_none() {
        return Err(ApiError(
            503,
            "ops_auth_misconfigured: set GR_OPS_TOKEN or GR_RESULT_TOKEN (or admin hub)".into(),
        ));
    }
    Err(ApiError(
        401,
        "ops auth required (X-Gr-Ops-Token / X-Gr-Result-Token / Bearer RESULT_TOKEN / admin session) — set GR_OPS_TOKEN or use result token".into(),
    ))
}
/// Active ops credential check (iss/opus5 04-P0-3): unlike `require_ops_auth`
/// (which passes open when ops auth is not globally required), this returns
/// true only when the caller actually presented a matching ops/result secret
/// or a valid admin session. Lab fallback: when no ops secrets are configured
/// anywhere and deploy env is not prod, verbose is granted (debug convenience).
pub fn ops_grant_active(st: &AppState, headers: &HashMap<String, String>) -> bool {
    let presented = presented_ops_secrets(headers);
    if secret_matches_env(&presented) {
        return true;
    }
    // Admin console session escape hatch removed (P3 schema convergence).
    // 豁免路径本意: 无任何凭据配置且不强制 ops auth 的环境 (lab/开发) 允许匿名 verbose。
    // admin hub 是节点标配基础设施, 不算"配置了 secret" —— 否则同一请求打到
    // admin hub 未挂上的节点反而放行、挂上的节点被拒 (节点行为漂移)。prod 不受影响:
    // ops_auth_required() 在 prod deploy env 恒 true, 该豁免恒不生效。
    let any_secret = !gr_abi::env::get("OPS_TOKEN").unwrap_or_default().trim().is_empty()
        || !gr_abi::env::get("RESULT_TOKEN")
            .unwrap_or_default()
            .trim()
            .is_empty()
        || !gr_abi::env::get("SITE_RESULT_TOKENS")
            .unwrap_or_default()
            .trim()
            .is_empty();
    !any_secret && !ops_auth_required()
}

/// A-SVC-4 + iss/48: accept global `GR_RESULT_TOKEN` **or** per-site backend SDK key
/// (`X-Gr-Sdk-Key` / `X-Business-Token` matching site result credential or admin DB key).
pub fn require_result_token(
    headers: &HashMap<String, String>,
    admin: Option<&AdminHub>,
) -> Result<(), ApiError> {
    require_result_token_bound(headers, admin, None)
}

pub fn require_result_token_bound(
    headers: &HashMap<String, String>,
    admin: Option<&AdminHub>,
    session_site_id: Option<&str>,
) -> Result<(), ApiError> {
    // iss/opus5 §3.2 P0-3: result reads are FAIL-CLOSED by default. Open
    // access requires an explicit escape hatch — `GR_ALLOW_OPEN_RESULTS=1`
    // (or `GR_REQUIRE_RESULT_TOKEN=0/false/no/off/open`) — and logs a loud
    // warning once. Deploy-env heuristics no longer open results.
    static OPEN_WARN_ONCE: std::sync::Once = std::sync::Once::new();
    let truthy = |v: &str| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on");
    let allow_open = gr_abi::env::get("ALLOW_OPEN_RESULTS")
        .map(|v| truthy(v.trim()))
        .unwrap_or(false);
    let explicit_open = gr_abi::env::get("REQUIRE_RESULT_TOKEN")
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off" | "open"
            )
        })
        .unwrap_or(false);
    if allow_open || explicit_open {
        OPEN_WARN_ONCE.call_once(|| {
            eprintln!(
                "[gr][SECURITY-WARNING] /result reads are OPEN (no token required) via {}. \
                 Anyone who can reach this service can read analysis results. \
                 Unset the escape hatch to restore fail-closed behavior.",
                if allow_open {
                    "GR_ALLOW_OPEN_RESULTS"
                } else {
                    "GR_REQUIRE_RESULT_TOKEN=open"
                }
            );
        });
        return Ok(());
    }
    let expected = gr_abi::env::get("RESULT_TOKEN").unwrap_or_default();
    let expected = expected.trim();
    let site_map = gr_abi::env::get("SITE_RESULT_TOKENS").unwrap_or_default();
    let auth = headers
        .get("authorization")
        .or_else(|| headers.get("Authorization"))
        .map(|s| s.as_str())
        .unwrap_or("");
    let xbt = headers
        .get("x-business-token")
        .or_else(|| headers.get("X-Business-Token"))
        .map(|s| s.as_str())
        .unwrap_or("");
    let bearer = auth
        .strip_prefix("Bearer ")
        .or_else(|| auth.strip_prefix("bearer "))
        .unwrap_or("");
    let xrt = headers
        .get("x-gr-result-token")
        .or_else(|| headers.get("X-Gr-Result-Token"))
        .or_else(|| headers.get("x-gr-ops-token"))
        .or_else(|| headers.get("X-Gr-Ops-Token"))
        .map(|s| s.as_str())
        .unwrap_or("");
    let site_tok = headers
        .get("x-gr-sdk-key")
        .or_else(|| headers.get("X-Gr-Sdk-Key"))
        .map(|s| s.as_str())
        .unwrap_or("");
    let presented = !bearer.is_empty() || !xbt.is_empty() || !xrt.is_empty() || !site_tok.is_empty();

    let eq = gr_probe_core::session_ticket::token_eq;
    if !expected.is_empty() && (eq(bearer, expected) || eq(xbt, expected) || eq(xrt, expected)) {
        return Ok(());
    }
    for part in site_map.split(',') {
        let mut it = part.splitn(2, ':');
        let site = it.next().unwrap_or("").trim();
        let tok = it.next().unwrap_or("").trim();
        if tok.is_empty() {
            continue;
        }
        if eq(site_tok, tok) || eq(xbt, tok) || eq(bearer, tok) {
            if let Some(want) = session_site_id.map(str::trim).filter(|s| !s.is_empty()) {
                if !site.is_empty() && site != want {
                    return Err(ApiError(403, "result token site mismatch".into()));
                }
            }
            return Ok(());
        }
    }
    if !site_tok.is_empty() {
        if let Some(hub) = admin {
            let hash = crate::admin::sdk::hash_secret(site_tok);
            if let Ok(Some((_kid, site, _origins))) = hub.db.find_active_backend_key(&hash) {
                if let Some(want) = session_site_id.map(str::trim).filter(|s| !s.is_empty()) {
                    if site != want {
                        return Err(ApiError(403, "sdk key site mismatch".into()));
                    }
                }
                return Ok(());
            }
        }
    }
    if expected.is_empty() && site_map.trim().is_empty() && !presented {
        return Err(ApiError(
            503,
            "result token not configured (set GR_RESULT_TOKEN)".into(),
        ));
    }
    Err(ApiError(
        401,
        "result token required (GR_RESULT_TOKEN / X-Gr-Sdk-Key) — iss/48 per-tenant".into(),
    ))
}

fn lookup_session_site(st: &AppState, session_id: &str) -> Option<String> {
    st.store.session_window(session_id).ok().and_then(|w| {
        w.pointer("/meta/site_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    })
}

fn require_session_result_auth(
    st: &AppState,
    headers: &HashMap<String, String>,
    session_id: &str,
) -> Result<(), ApiError> {
    let site = lookup_session_site(st, session_id);
    require_result_token_bound(headers, st.admin.as_deref(), site.as_deref())
}

/// Merge gateway net fields + live velocity into a result clone for product_public projection.
fn enrich_result_for_public(st: &AppState, _session_id: &str, result: &Value) -> Value {
    let mut out = result.clone();
    let Some(root) = out.as_object_mut() else {
        return out;
    };
    if !root.contains_key("product") {
        root.insert("product".into(), json!({}));
    }
    let fields_obj = root.get("fields").cloned().or_else(|| {
        root.get("device")
            .and_then(|d| d.get("trust"))
            .and_then(|t| t.get("materials"))
            .cloned()
    });
    let client_ip = root
        .get("client_ip")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            fields_obj
                .as_ref()
                .and_then(|f| f.get("server_client_ip"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        });
    let Some(prod) = root.get_mut("product").and_then(|p| p.as_object_mut()) else {
        return out;
    };
    if let Some(Value::Object(fields)) = fields_obj {
        for k in [
            "server_asn",
            "server_country",
            "server_datacenter",
            "server_network_class",
            "server_asn_org",
            "server_client_ip",
            "ja4",
            "tls_ja4",
            "protocol_engine",
            "user_agent",
            "alpn",
        ] {
            if !prod.contains_key(k) {
                if let Some(v) = fields.get(k) {
                    prod.insert(k.to_string(), v.clone());
                }
            }
        }
    }
    // Live re-classify client_ip when evaluate slim dropped gateway net tags.
    if let Some(ref ip) = client_ip {
        prod.entry("server_client_ip".to_string())
            .or_insert_with(|| json!(ip));
        let mut scratch = serde_json::Map::new();
        if let Some(a) = prod.get("server_asn").cloned() {
            scratch.insert("server_asn".into(), a);
        }
        if let Some(c) = prod.get("server_country").cloned() {
            scratch.insert("server_country".into(), c);
        }
        gr_probe_core::enrich_fields_if_empty(&mut scratch, Some(ip.as_str()));
        for (k, v) in scratch {
            prod.entry(k).or_insert(v);
        }
    }
    let did = prod
        .get("device_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    if let Ok(vs) = st
        .store
        .velocity_summary(did.as_deref(), client_ip.as_deref(), now)
    {
        if let Some(score) = vs.get("velocity_score") {
            prod.insert("velocity_score".into(), score.clone());
        }
        prod.insert("velocity_windows".into(), vs);
    }
    out
}

fn panel_data_dir(st: &AppState) -> Option<std::path::PathBuf> {
    st.admin.as_ref().map(|a| {
        // probe_admin/ → parent install/data dir where control plane writes panel_policy.json
        a.db.data_dir
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| a.db.data_dir.clone())
    })
}

fn provider_ip_allowed(ip: &str) -> bool {
    let Ok(parsed) = ip.parse::<std::net::IpAddr>() else {
        return false;
    };
    if parsed.is_loopback() || parsed.is_unspecified() || parsed.is_multicast() {
        return false;
    }
    let private = match parsed {
        std::net::IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
        std::net::IpAddr::V6(v6) => {
            let octets = v6.octets();
            (octets[0] & 0xfe) == 0xfc || v6.is_unicast_link_local()
        }
    };
    !private
}

/// Panel QA P1a fix (2026-09-08): stored visitor IPs are privacy-masked CIDRs
/// (`a.b.c.0/24` for v4, `first3::/48` for v6 — single-write at ingestion, no
/// raw/masked dual track). A masked CIDR is not an IP literal, so the
/// enrichment gate used to reject every real session with a misleading
/// `non_public_ip`. Strip the mask suffix and query the network address
/// instead: the /24 is already public in the result payload, and the bare
/// network address is exactly what geo/IP providers expect. Values that are
/// not masked (bare IPs, hashes, "unknown") pass through unchanged.
fn provider_ip_target(ip: &str) -> std::borrow::Cow<'_, str> {
    if let Some((base, mask)) = ip.trim().rsplit_once('/') {
        if !mask.is_empty()
            && mask.bytes().all(|b| b.is_ascii_digit())
            && base.parse::<std::net::IpAddr>().is_ok()
        {
            return std::borrow::Cow::Owned(base.to_string());
        }
    }
    std::borrow::Cow::Borrowed(ip.trim())
}

fn integration_url_allowed(raw: &str) -> Result<reqwest::Url, &'static str> {
    let url = reqwest::Url::parse(raw).map_err(|_| "invalid_url")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || url.username() != ""
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("unsafe_url");
    }
    if !gr_abi::env::flag("ALLOW_LAB_INTEGRATION_HTTP") {
        if let Some(host) = url.host_str() {
            let host_l = host.to_ascii_lowercase();
            if host_l == "localhost"
                || host_l.ends_with(".localhost")
                || host_l.ends_with(".local")
                || host_l == "127.0.0.1"
                || host_l == "::1"
                || host.parse::<std::net::IpAddr>().is_ok_and(|ip| {
                    ip.is_loopback()
                        || ip.is_unspecified()
                        || ip.is_multicast()
                        || match ip {
                            std::net::IpAddr::V4(v) => v.is_private() || v.is_link_local(),
                            std::net::IpAddr::V6(v) => {
                                let o = v.octets();
                                (o[0] & 0xfe) == 0xfc || v.is_unicast_link_local()
                            }
                        }
                })
            {
                return Err("private_target_blocked");
            }
        }
    }
    Ok(url)
}

fn remote_retryable(status: Option<u16>, error: Option<&reqwest::Error>) -> bool {
    status.map(|s| s == 429 || s >= 500).unwrap_or_else(|| {
        error
            .map(|e| e.is_timeout() || e.is_connect())
            .unwrap_or(false)
    })
}

fn provider_value_string(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        obj.get(*key).and_then(|v| {
            v.as_str()
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.to_string())
        })
    })
}

fn map_ip_provider_response(provider: &str, ip: &str, body: &Value) -> Value {
    let obj = body
        .as_object()
        .or_else(|| body.get("data").and_then(|v| v.as_object()));
    let Some(obj) = obj else {
        return json!({
            "provider": provider,
            "status": "invalid_response",
            "ip": ip,
        });
    };
    let org = provider_value_string(obj, &["org", "organization", "as_name"]);
    let asn = provider_value_string(obj, &["asn", "autonomous_system_number"])
        .or_else(|| org.as_deref().and_then(|s| s.split_whitespace().next()).map(str::to_string));
    let country = provider_value_string(obj, &["country", "country_code", "countryCode"]);
    let datacenter = obj
        .get("datacenter")
        .or_else(|| obj.get("is_datacenter"))
        .and_then(|v| v.as_bool());
    let network_class = provider_value_string(obj, &["network_class", "networkClass"]);
    json!({
        "provider": provider,
        "status": "ok",
        "ip": ip,
        "asn": asn,
        "country": country,
        "org": org,
        "datacenter": datacenter,
        "network_class": network_class,
    })
}

fn fetch_ip_provider(policy: &Value, ip: &str) -> Value {
    let ie = policy.get("ip_enrichment").unwrap_or(&Value::Null);
    if ie.get("enabled").and_then(|v| v.as_bool()) != Some(true) {
        return json!({"status": "disabled"});
    }
    // P1a: unmask the stored /24 (v4) or /48 (v6) CIDR to its network
    // address before the public-IP gate, so real sessions reach the provider.
    let unmasked = provider_ip_target(ip);
    let ip: &str = unmasked.as_ref();
    if !provider_ip_allowed(ip) {
        return json!({"status": "skipped", "reason": "non_public_ip"});
    }
    let provider = ie
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("dbip_lite");
    if matches!(provider, "dbip_lite" | "maxmind_geolite2") {
        return json!({
            "provider": provider,
            "status": "local",
            "ip": ip,
            "note": "MMDB/heuristic enrichment runs in gr-probe-core",
        });
    }
    let provider_cfg = ie
        .get("providers")
        .and_then(|v| v.get(provider))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let Some(base_url) = provider_cfg.get("base_url").and_then(|v| v.as_str()).or_else(|| {
        provider_cfg.get("url").and_then(|v| v.as_str())
    }) else {
        return json!({"provider": provider, "status": "error", "error": "provider_url_missing"});
    };
    let target = if provider == "ipinfo" {
        if base_url.contains("{ip}") {
            base_url.replace("{ip}", ip)
        } else {
            format!("{}/{}/json", base_url.trim_end_matches('/'), ip)
        }
    } else {
        base_url.to_string()
    };
    let Ok(url) = integration_url_allowed(&target) else {
        return json!({"provider": provider, "status": "error", "error": "unsafe_provider_url"});
    };
    let client = match reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_millis(500))
        .timeout(std::time::Duration::from_millis(1200))
        .build()
    {
        Ok(c) => c,
        Err(_) => return json!({"provider": provider, "status": "error", "error": "client_init"}),
    };
    let token = provider_cfg
        .get("token")
        .and_then(|v| v.as_str())
        .filter(|s| !s.starts_with("****"))
        .unwrap_or("");
    let auth = provider_cfg
        .get("header_auth")
        .and_then(|v| v.as_str())
        .filter(|s| !s.starts_with("****"))
        .unwrap_or("");
    let mut last_status = None;
    let mut last_error = None;
    for attempt in 0..2 {
        let mut request = if provider == "custom_http" {
            client.post(url.clone()).json(&json!({"ip": ip}))
        } else {
            client.get(url.clone())
        };
        request = request.header("Accept", "application/json");
        if provider == "ipinfo" && !token.is_empty() {
            request = request.bearer_auth(token);
        }
        if provider == "custom_http" && !auth.is_empty() {
            if let Some((name, value)) = auth.split_once(':') {
                if !name.trim().is_empty() {
                    request = request.header(name.trim(), value.trim());
                }
            } else {
                request = request.bearer_auth(auth);
            }
        }
        match request.send() {
            Ok(response) => {
                let status = response.status().as_u16();
                last_status = Some(status);
                let body = response.json::<Value>().unwrap_or(Value::Null);
                if status < 400 {
                    return map_ip_provider_response(provider, ip, &body);
                }
                if !remote_retryable(Some(status), None) || attempt == 1 {
                    break;
                }
            }
            Err(error) => {
                last_error = Some(error);
                if !remote_retryable(None, last_error.as_ref()) || attempt == 1 {
                    break;
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let error = if last_error.as_ref().is_some_and(|e| e.is_timeout()) {
        "timeout"
    } else if last_status == Some(429) {
        "rate_limited"
    } else if last_status.is_some_and(|s| s >= 500) {
        "upstream_5xx"
    } else {
        "upstream_unavailable"
    };
    json!({
        "provider": provider,
        "status": "error",
        "error": error,
        "http_status": last_status,
    })
}

pub fn get_result(
    st: &AppState,
    session_id: &str,
    headers: &HashMap<String, String>,
    query: &HashMap<String, String>,
) -> AppResult {
    require_session_result_auth(st, headers, session_id)?;
    // Admin-session diagnostic projection removed (P3 schema convergence):
    // ops/result secrets are the only elevated path.
    let admin_ok = false;
    let projection = gr_probe_core::resolve_result_projection(headers, query, admin_ok)
        .map_err(|(c, m)| ApiError(c, m))?;
    let data_dir = panel_data_dir(st);
    let attach_public = |result: &Value| -> Value {
        let enriched = enrich_result_for_public(st, session_id, result);
        let mut qmap = query.clone();
        // Official-site in-memory vault (by domain / site) overrides local panel strategy.
        let vault = gr_probe_core::strategy_vault::global_vault();
        let page_host = enriched
            .pointer("/fields/page_host")
            .or_else(|| enriched.pointer("/fields/page_url"))
            .or_else(|| enriched.pointer("/meta/page_url"))
            .and_then(|v| v.as_str())
            .and_then(|u| {
                let s = u.trim();
                let host = if let Some(rest) = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")) {
                    rest.split('/').next().unwrap_or(rest)
                } else {
                    s.split('/').next().unwrap_or(s)
                };
                let host = host.split(':').next().unwrap_or(host);
                if host.is_empty() {
                    None
                } else {
                    Some(host.to_ascii_lowercase())
                }
            });
        let site_owned: Option<String> = enriched
            .pointer("/fields/site_id")
            .or_else(|| enriched.pointer("/meta/site_id"))
            .or_else(|| enriched.pointer("/product/site_id"))
            .or_else(|| enriched.pointer("/session_meta/site_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| qmap.get("site_id").cloned());
        let site = site_owned.as_deref();
        let local_policy =
            gr_probe_core::load_panel_policy(data_dir.as_deref()).resolve_result_policy(site);
        let vault_entry = page_host
            .as_deref()
            .and_then(|h| vault.get_by_domain(h))
            .or_else(|| site.and_then(|s| vault.get_by_site(s)))
            // lab: shop.gr.local ↔ site_id shop
            .or_else(|| {
                site.and_then(|s| match s {
                    "shop" => vault.get_by_domain("shop.gr.local"),
                    "news" => vault.get_by_domain("news.gr.local"),
                    "site-a" => vault.get_by_domain("site-a.gr.local"),
                    "site-b" => vault.get_by_domain("site-b.gr.local"),
                    _ => None,
                })
            });
        if !qmap.contains_key("strategy_id") && !qmap.contains_key("strategy") {
            let sid = gr_probe_core::load_panel_policy(data_dir.as_deref()).resolve_strategy_id(site);
            qmap.insert("strategy_id".into(), sid);
        }
        if !qmap.contains_key("response_profile") && !qmap.contains_key("profile") {
            qmap.insert(
                "response_profile".into(),
                local_policy.response_profile.clone(),
            );
        }
        qmap.insert(
            "primary_device_lane".into(),
            local_policy.primary_device_lane.clone(),
        );
        let mut ctx = gr_probe_core::ProjectCtx::from_str_map(&qmap);
        if let Some(cap) = qmap.get("profile_cap") {
            ctx.profile = ctx.profile.clamp_to(gr_probe_core::ResponseProfile::parse(cap));
        }
        if ctx.session_id.is_none() {
            ctx.session_id = Some(session_id.to_string());
        }
        let mut product_public = gr_probe_core::project_public(&enriched, &ctx);
        let data_dir_ent = panel_data_dir(st);
        let ent = gr_probe_core::plan_entitlement::resolve_site_entitlement(
            site,
            page_host.as_deref(),
            data_dir_ent.as_deref(),
        );
        let product_for_ent = enriched.get("product").unwrap_or(&enriched);
        product_public = gr_probe_core::plan_entitlement::apply_to_product_public(
            product_public,
            &ent,
            product_for_ent,
        );
        if let Some(obj) = product_public.as_object_mut() {
            if !local_policy.include_signals {
                obj.insert("signals".into(), json!([]));
            }
            if !local_policy.rpa_collect {
                if let Some(rpa) = obj
                    .get_mut("scores")
                    .and_then(|v| v.get_mut("rpa"))
                    .and_then(|v| v.as_object_mut())
                {
                    rpa.insert("score".into(), Value::Null);
                    rpa.insert("human_score".into(), Value::Null);
                    rpa.insert("bot_score".into(), Value::Null);
                    rpa.insert("status".into(), json!("disabled_by_config"));
                    rpa.insert("coverage".into(), json!("disabled_by_config"));
                }
                if let Some(axis) = obj
                    .get_mut("capability_axes")
                    .and_then(|v| v.get_mut("automation_risk"))
                    .and_then(|v| v.as_object_mut())
                {
                    axis.insert("score".into(), Value::Null);
                    axis.insert("status".into(), json!("disabled_by_config"));
                    axis.insert("coverage".into(), json!("disabled_by_config"));
                }
            }
            obj.insert(
                "result_policy".into(),
                serde_json::to_value(&local_policy).unwrap_or_else(|_| json!({})),
            );
        }
        // Attach configured IP enrichment provider note (admin Integrations → analysis results).
        if let Some(obj) = product_public.as_object_mut() {
            let pol = gr_probe_core::load_panel_policy(data_dir_ent.as_deref());
            let provider_result = obj
                .get("network")
                .and_then(|v| v.get("ip"))
                .and_then(|v| v.as_str())
                .map(|ip| fetch_ip_provider(&pol.integrations, ip))
                .unwrap_or_else(|| json!({"status": "skipped", "reason": "ip_missing"}));
            if let Some(net) = obj.get_mut("network").and_then(|n| n.as_object_mut()) {
                let ie = pol.integrations.get("ip_enrichment");
                net.insert(
                    "integrations".into(),
                    json!({
                        "enabled": ie.and_then(|v| v.get("enabled")).and_then(|v| v.as_bool()).unwrap_or(false),
                        "provider": ie.and_then(|v| v.get("provider")).cloned().unwrap_or(Value::Null),
                        "result": provider_result,
                        "note": "IP reputation/info from admin-panel providers is mapped into this result; secrets stay server-side",
                    }),
                );
                if provider_result.get("status").and_then(|v| v.as_str()) == Some("ok") {
                    net.insert("ip_info".into(), provider_result);
                }
            }
            if let Some(ve) = vault_entry.as_ref() {
                obj.insert(
                    "cloud_site".into(),
                    json!({
                        "domain": ve.domain,
                        "site_id": ve.site_id,
                        "source": "official_site_memory_vault",
                    }),
                );
            }
        }
        if let Some(obj) = product_public.as_object_mut() {
            if let Some(bctx) = st
                .store
                .merge_session_meta(session_id, &json!({}))
                .ok()
                .and_then(|m| m.get("business_context").cloned())
            {
                obj.insert(
                    "business_context".into(),
                    gr_probe_core::project_business_context(&bctx, projection.as_str()),
                );
            }
            // Allowlisted cookies captured server-side at session open/ingest
            // (business-identifier binding). Site owner opted in via the site's
            // cookie_fields allowlist; the SDK key is site-scoped.
            if let Some(cf) = st
                .store
                .merge_session_meta(session_id, &json!({}))
                .ok()
                .and_then(|m| m.get("cookie_fields").cloned())
            {
                if cf.as_object().map(|o| !o.is_empty()).unwrap_or(false) {
                    obj.insert("cookie_fields".into(), cf);
                }
            }
        }
        gr_probe_core::assemble_result_response(session_id, result, product_public, projection)
    };
    match st.store.latest_analysis(&session_id)? {
        Some(r) => Ok(attach_public(&r)),
        None => {
            // build + evaluate if batches exist but no analysis yet
            let evidence = st.store.build_evidence(&session_id)?;
            let batches = evidence
                .get("batches")
                .and_then(|b| b.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            if batches == 0 {
                return Err(ApiError(
                    404,
                    format!("no analysis or batches for {session_id}"),
                ));
            }
            let mut result = evaluate_session(&evidence, None, None, None, st.soft_v2_ready)
                .map_err(|e| ApiError(422, e))?;
            // iss/opus5 03-P0-1: planned-vs-actual ledger → evidence_withheld
            // band on the fallback evaluate path (same rule as analyze_session).
            apply_evidence_withheld(&st.store, &session_id, &mut result);
            let rev = st.store.save_analysis(&session_id, &result)?;
            persist_brain_control(&st.store, &session_id, &result, rev);
            let mut body = attach_public(&result);
            if let Some(obj) = body.as_object_mut() {
                obj.insert("analysis_rev".into(), json!(rev));
            }
            Ok(body)
        }
    }
}

pub fn get_evidence(
    st: &AppState,
    session_id: &str,
    headers: &HashMap<String, String>,
) -> AppResult {
    require_session_result_auth(st, headers, session_id)?;
    // Admin-session diagnostic projection removed (P3 schema convergence).
    let admin_ok = false;
    if gr_probe_core::result_token_enforced()
        && !gr_probe_core::diagnostic_scope_from_headers(headers)
        && !admin_ok
    {
        return Err(ApiError(
            403,
            "evidence requires diagnostic/ops scope".into(),
        ));
    }
    let evidence = st.store.build_evidence(&session_id)?;
    Ok(json!({"ok": true, "evidence": evidence}))
}

pub fn list_analyses(
    st: &AppState,
    session_id: &str,
    headers: &HashMap<String, String>,
    query: &HashMap<String, String>,
) -> AppResult {
    require_session_site_auth(st, headers, session_id)?;
    // iss/opus5 04-P0-3: the FE brain poll hits this endpoint every few
    // seconds; full results (~0.5 MB each × history) are internal state.
    // Default to the slim projection; `?verbose=1` + ops credential for full.
    let want_verbose = matches!(
        query.get("verbose").map(|s| s.as_str()),
        Some("1") | Some("true") | Some("yes")
    );
    let verbose_full = want_verbose && ops_grant_active(st, headers);
    let mut history = st.store.list_analyses(&session_id)?;
    if !verbose_full {
        for entry in history.iter_mut() {
            if let Some(res) = entry.get("result").cloned() {
                if let Some(obj) = entry.as_object_mut() {
                    obj.insert(
                        "result".into(),
                        gr_probe_core::response_slim::slim_analyze_result(&res),
                    );
                }
            }
        }
    }
    let window = st.store.session_window(&session_id).ok();
    let probe = cycle_probe_status_snapshot(st, session_id);
    Ok(json!({
        "ok": true,
        "session_id": session_id,
        "count": history.len(),
        "analyses": history,
        "session_window": window,
        "cycle_probe_status": probe,
        "halt_uploads": probe.get("halt_uploads"),
        "business_state": probe.get("business_state"),
    }))
}

pub fn list_batches(st: &AppState, session_id: &str, headers: &HashMap<String, String>) -> AppResult {
    require_session_result_auth(st, headers, session_id)?;
    let batches = st.store.list_received_batches(&session_id)?;
    Ok(json!({
        "ok": true,
        "session_id": session_id,
        "batches": batches,
    }))
}

pub fn session_window(st: &AppState, session_id: &str, headers: &HashMap<String, String>) -> AppResult {
    require_session_result_auth(st, headers, session_id)?;
    let w = st.store.session_window(&session_id)?;
    let probe = cycle_probe_status_snapshot(st, session_id);
    Ok(json!({
        "ok": true,
        "window": w,
        "cycle_probe_status": probe,
        "halt_uploads": probe.get("halt_uploads"),
        "business_state": probe.get("business_state"),
    }))
}

/// Lite ops surface: session timeline + coverage/device for brain tuning.
pub fn ops_session(st: &AppState, session_id: &str) -> AppResult {
    let window = st.store.session_window(&session_id).ok();
    let batches = st.store.list_received_batches(&session_id).unwrap_or_default();
    let analyses = st.store.list_analyses(&session_id).unwrap_or_default();
    let evidence = st.store.build_evidence(&session_id).ok();
    let latest = st.store.latest_analysis(&session_id).ok().flatten();
    let compact: Vec<Value> = analyses
        .iter()
        .map(|a| {
            json!({
                "rev": a.get("rev"),
                "created_ms": a.get("created_ms"),
                "real_band": a.get("real_band"),
                "device_id": a.get("device_id"),
                "stop_probe": a.pointer("/result/route_plan/stop_probe")
                    .or_else(|| a.pointer("/route_plan/stop_probe")),
                "coverage_complete": a.pointer("/result/coverage/coverage_complete")
                    .or_else(|| a.pointer("/coverage/coverage_complete")),
                "packs": a.pointer("/result/route_plan/packs")
                    .and_then(|p| p.as_array())
                    .map(|arr| arr.len())
                    .or_else(|| {
                        a.pointer("/route_plan/packs")
                            .and_then(|p| p.as_array())
                            .map(|arr| arr.len())
                    }),
            })
        })
        .collect();
    Ok(json!({
        "ok": true,
        "session_id": session_id,
        "window": window,
        "batch_count": batches.len(),
        "batches": batches,
        "analysis_count": compact.len(),
        "timeline": compact,
        "latest": latest.as_ref().map(|r| {
            let digest = r
                .pointer("/device/digest_path")
                .or_else(|| r.pointer("/product/digest_path"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let tier = r
                .pointer("/device/device_tier")
                .or_else(|| r.pointer("/product/device_tier"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let provisional = r
                .pointer("/device/provisional_gateway")
                .and_then(|v| v.as_bool())
                .unwrap_or(
                    digest.contains("empty_anchor")
                        || digest.contains("gateway_only")
                        || tier == "dg",
                );
            let vtid = r
                .pointer("/fields/visitor_terminal_id")
                .or_else(|| r.get("visitor_terminal_id"))
                .or_else(|| r.pointer("/product/visitor_terminal_id"))
                .cloned()
                .unwrap_or(Value::Null);
            json!({
                "real_band": r.get("real_band"),
                "device_id": r.pointer("/device/device_id").or_else(|| r.pointer("/product/device_id")),
                "device_tier": if tier.is_empty() { Value::Null } else { json!(tier) },
                "digest_path": digest,
                "provisional_gateway": provisional,
                "mint_silicon_ok": r.pointer("/device/mint_eligible").or_else(|| r.get("mint_silicon_ok")),
                "visitor_terminal_id": vtid,
                "coverage": r.get("coverage"),
                "analysis_terminal": r.get("analysis_terminal"),
                "stop_probe": r.pointer("/route_plan/stop_probe"),
                "packs": r.pointer("/route_plan/packs"),
                "analysis_rev": r.get("analysis_rev"),
                "note": if provisional {
                    "latest may be B8/gateway-only — use GET /v1/ops/vt_best_silicon?visitor_terminal_id=…"
                } else {
                    ""
                },
            })
        }),
        "evidence_summary": evidence.as_ref().map(|e| json!({
            "sources": e.get("sources"),
            "batch_count": e.get("batches").and_then(|b| b.as_array()).map(|a| a.len()),
            "has_gateway": e.get("has_gateway"),
            "has_cloudflare": e.get("has_cloudflare"),
            "field_keys": e.get("fields").and_then(|f| f.as_object()).map(|o| o.keys().cloned().collect::<Vec<_>>()),
        })),
        "ops_hints": {
            "vt_best_silicon": "/v1/ops/vt_best_silicon?visitor_terminal_id=<vt>&session_id=<sid>",
            "events_export": "/v1/ops/events/export?limit=500",
        },
    }))
}

/// FE silent health/error channel (no console). Rate-limited per IP+code.
///
/// Product policy (low traffic): **open** residual/curve *summaries* for triage
/// (`residual_mean`, `n_paths`, `modes`, …). Still strip secrets and raw curve
/// series / huge arrays (those already land in probe_batches).
pub fn ops_client_event(
    st: &AppState,
    headers: &HashMap<String, String>,
    body: Value,
) -> AppResult {
    // Admin / env kill-switch for client error upload (default enabled).
    let disabled = gr_abi::env::get("OPS_CLIENT_EVENTS")
        .map(|v| {
            let t = v.trim().to_ascii_lowercase();
            t == "0" || t == "false" || t == "off"
        })
        .unwrap_or(false)
        || st
            .admin
            .as_ref()
            .and_then(|a| a.db.get_setting("ops_client_events_enabled").ok().flatten())
            .map(|v| {
                let t = v.trim().to_ascii_lowercase();
                t == "0" || t == "false" || t == "off"
            })
            .unwrap_or(false);
    if disabled {
        return Ok(json!({
            "ok": true,
            "stored": false,
            "disabled": true,
            "note": "ops_client_events_enabled=0 or GR_OPS_CLIENT_EVENTS=0"
        }));
    }
    // Body size guard — raised so multipath/upload diagnostics fit (traffic low).
    let raw = serde_json::to_vec(&body).unwrap_or_default();
    if raw.len() > 48 * 1024 {
        return Err(ApiError(413, "ops_client_event too large".into()));
    }
    let code = body
        .get("code")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if code.is_empty() || code.len() > 64 {
        return Err(ApiError(400, "code required".into()));
    }
    // Allowlist stages/codes loosely
    let stage = body
        .get("stage")
        .and_then(|v| v.as_str())
        .unwrap_or("client")
        .to_string();
    // iss/opus5 §2.4: the comment promised 60 events/min/IP but nothing was
    // implemented. Enforce it via the shared limiter, keyed on the socket
    // peer (client-supplied XFF/X-Real are spoofable and must not pick the
    // bucket); fall back to forwarded headers only behind a trusted proxy.
    let sock_peer = headers.get("x-gr-peer-ip").map(|s| s.as_str());
    let ip = if peer_is_trusted_proxy(sock_peer) {
        headers
            .get("x-real-ip")
            .or_else(|| headers.get("x-forwarded-for"))
            .map(|s| s.split(',').next().unwrap_or(s).trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| sock_peer.map(|s| s.to_string()))
            .unwrap_or_else(|| "unknown".to_string())
    } else {
        sock_peer
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown".to_string())
    };
    if let Err(e) = crate::rate_limit::check(st, &format!("ce:{ip}"), "client_event") {
        return Err(ApiError(429, e));
    }
    let detail = body.get("detail").cloned().unwrap_or(json!({}));
    // Open probe diagnostics; strip secrets + raw series dumps only.
    let mut detail_clean = Map::new();
    if let Some(o) = detail.as_object() {
        for (k, v) in o {
            let lk = k.to_ascii_lowercase();
            if lk.contains("password")
                || lk.contains("cookie")
                || lk.contains("token")
                || lk.contains("authorization")
                || lk.contains("secret")
                || lk.contains("set-cookie")
            {
                continue;
            }
            // Drop raw residual/curve *series* (long arrays / base64 blobs).
            // Keep scalars/summaries: residual_mean, residual_std, n_paths, modes, …
            let is_long_array = matches!(v, Value::Array(a) if a.len() > 16);
            let is_long_str = matches!(v, Value::String(s) if s.len() > 128);
            let looks_series_key =
                lk.contains("curve") || lk.contains("hist") || lk.contains("payload");
            if looks_series_key && is_long_array {
                let n = v.as_array().map(|a| a.len()).unwrap_or(0);
                detail_clean.insert(k.clone(), json!({"_n": n, "_note": "series_omitted"}));
                continue;
            }
            if lk.contains("curve") && is_long_str {
                detail_clean.insert(k.clone(), json!({"_note": "blob_omitted"}));
                continue;
            }
            // Cap residual_paths-style arrays to compact path rows (no embedded curves).
            if (lk == "residual_paths" || lk.ends_with("_paths")) {
                if let Some(arr) = v.as_array() {
                    let compact: Vec<Value> = arr
                        .iter()
                        .take(8)
                        .map(|p| {
                            if let Some(po) = p.as_object() {
                                json!({
                                    "path_id": po.get("path_id"),
                                    "ok": po.get("ok"),
                                    "mean": po.get("mean"),
                                    "std": po.get("std"),
                                    "entropy_ok": po.get("entropy_ok"),
                                    "shader_mode": po.get("shader_mode"),
                                    "select_score": po.get("select_score"),
                                })
                            } else {
                                p.clone()
                            }
                        })
                        .collect();
                    detail_clean.insert(k.clone(), Value::Array(compact));
                    continue;
                }
            }
            // Truncate long strings (stacks / messages)
            let v2 = if let Some(s) = v.as_str() {
                if s.len() > 512 {
                    json!(format!("{}…[trunc:{}]", &s[..200], s.len()))
                } else {
                    v.clone()
                }
            } else if let Some(a) = v.as_array() {
                if a.len() > 32 {
                    Value::Array(a.iter().take(32).cloned().collect())
                } else {
                    v.clone()
                }
            } else {
                v.clone()
            };
            detail_clean.insert(k.clone(), v2);
        }
    }
    // Commercial taxonomy: normalize false 5xx / attach fail_class · impact_band · actionability
    // (sticky FE may still send misclassified codes — server is SSOT for ops slices).
    let sev_in = body
        .get("severity")
        .and_then(|v| v.as_str())
        .unwrap_or("error");
    let (code_norm, sev_norm, detail_tax) =
        gr_probe_core::enrich_ops_event(&code, sev_in, &Value::Object(detail_clean));
    let row = json!({
        "event_id": body.get("event_id").cloned().unwrap_or(Value::Null),
        "ts_ms": body.get("ts_ms").and_then(|v| v.as_i64()).unwrap_or(0),
        "site_id": body.get("site_id").and_then(|v| v.as_str()).unwrap_or(""),
        "visitor_terminal_id": body.get("visitor_terminal_id").and_then(|v| v.as_str()).unwrap_or(""),
        "session_id": body.get("session_id").and_then(|v| v.as_str()).unwrap_or(""),
        "product_version": body.get("product_version").and_then(|v| v.as_str()).unwrap_or(""),
        "inject_path": body.get("inject_path").and_then(|v| v.as_str()).unwrap_or(""),
        "engine_family": body.get("engine_family").and_then(|v| v.as_str()).unwrap_or(""),
        "ua_hash": body.get("ua_hash").and_then(|v| v.as_str()).unwrap_or(""),
        "stage": stage,
        "code": code_norm,
        "severity": sev_norm,
        "detail_json": detail_tax,
        "sample_rate": body.get("sample_rate").and_then(|v| v.as_f64()).unwrap_or(1.0),
        // iss/opus5 05-S-4: store the masked network class (the rate-limit
        // bucket above keeps the full peer for granularity; only the stored
        // copy is truncated).
        "client_ip": if ip.is_empty() || ip == "unknown" {
            Value::Null
        } else {
            json!(gr_probe_core::privacy::apply_ip_policy(&ip))
        },
    });
    let out = st
        .store
        .insert_ops_client_event(row)
        .map_err(|e| ApiError(500, e.to_string()))?;
    Ok(json!({"ok": true, "stored": out, "taxonomy": gr_probe_core::OPS_TAXONOMY_ALGO}))
}

pub fn ops_events_list(st: &AppState, query: &HashMap<String, String>) -> AppResult {
    let source = query.get("source").map(|s| s.as_str()).unwrap_or("server");
    let limit = query
        .get("limit")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(50);
    let code = query.get("code").cloned();
    let site_id = query.get("site_id").cloned();
    let since_ms = query.get("since_ms").and_then(|s| s.parse::<i64>().ok());
    let product_version = query
        .get("product_version")
        .cloned()
        .or_else(|| query.get("version").cloned());
    st.store
        .list_ops_events(source, limit, code, site_id, since_ms, product_version)
        .map_err(|e| ApiError(500, e.to_string()))
}

/// Combined export for ops/admin: both sources, optional result-token gate in production.
pub fn ops_events_export(st: &AppState, query: &HashMap<String, String>) -> AppResult {
    let limit = query
        .get("limit")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(500)
        .clamp(1, 2000);
    let site_id = query.get("site_id").cloned();
    let code = query.get("code").cloned();
    let since_ms = query.get("since_ms").and_then(|s| s.parse::<i64>().ok());
    let product_version = query
        .get("product_version")
        .cloned()
        .or_else(|| query.get("version").cloned());
    let client = st
        .store
        .list_ops_events(
            "client",
            limit,
            code.clone(),
            site_id.clone(),
            since_ms,
            product_version.clone(),
        )
        .map_err(|e| ApiError(500, e.to_string()))?;
    let server = st
        .store
        .list_ops_events(
            "server",
            limit,
            code,
            site_id,
            since_ms,
            product_version,
        )
        .map_err(|e| ApiError(500, e.to_string()))?;
    Ok(json!({
        "ok": true,
        "exported_at_ms": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0),
        "client": client,
        "server": server,
        "format": "json",
        "note": "Use Accept or download as file from ops/admin UI"
    }))
}

pub fn ops_b10_health(st: &AppState, query: &HashMap<String, String>) -> AppResult {
    let hours = query
        .get("hours")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(24)
        .clamp(1, 168);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let since = now - hours * 3600 * 1000;
    st.store
        .ops_b10_health(since, 32)
        .map_err(|e| ApiError(500, e.to_string()))
}

/// iss/60 L4: identity governance / shared metrics for admin dashboard.
/// Ops: set/clear the drain flag for this node. While set, `readyz` returns 503
/// and probe data routes answer 503 (LB keeps the node out of rotation).
pub fn ops_drain_set(st: &AppState, drain: bool) -> Value {
    st.draining.store(drain, Ordering::Relaxed);
    crate::dual_log::emit(
        crate::dual_log::Channel::System,
        if drain { "ops.drain.on" } else { "ops.drain.off" },
        json!({
            "worker_id": st.worker_id,
            "role": format!("{:?}", st.role),
            "draining": drain,
        }),
    );
    json!({
        "ok": true,
        "draining": drain,
        "worker_id": st.worker_id,
    })
}

/// Ops: cluster view — heartbeats last written by every node with admin-DB
/// access (shared `panel_config` store), plus this process' own state.
pub fn ops_nodes(st: &AppState) -> Value {
    let mut out = json!({
        "ok": true,
        "algo": "ops_nodes_v1",
        "self": {
            "worker_id": st.worker_id,
            "role": format!("{:?}", st.role),
            "store": st.store.backend_name(),
            "draining": st.draining.load(Ordering::Relaxed),
            "product_version": env!("CARGO_PKG_VERSION"),
        },
    });
    if let Some(admin) = st.admin.as_ref() {
        let beats = crate::admin::panel_config::list_heartbeats(&admin.db);
        if let Some(o) = out.as_object_mut() {
            o.insert("nodes".into(), beats);
        }
    } else {
        if let Some(o) = out.as_object_mut() {
            o.insert(
                "nodes".into(),
                json!([{ "worker_id": st.worker_id, "note": "admin_db_disabled" }]),
            );
        }
    }
    out
}

/// Ops: process + queue + rate-limit metrics (no per-session or PII data).
pub fn ops_metrics(st: &AppState) -> Value {
    // RSS/VMS from /proc/self/statm (Linux). Missing file → null (non-Linux).
    let mut rss_kb: Option<u64> = None;
    let mut vms_kb: Option<u64> = None;
    if let Ok(statm) = std::fs::read_to_string("/proc/self/statm") {
        let fields: Vec<&str> = statm.split_whitespace().take(2).collect();
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if fields.len() == 2 && page_size > 0 {
            if let (Ok(size), Ok(resident)) =
                (fields[0].parse::<u64>(), fields[1].parse::<u64>())
            {
                let kb = (page_size as u64) / 1024;
                vms_kb = Some(size.saturating_mul(kb));
                rss_kb = Some(resident.saturating_mul(kb));
            }
        }
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    json!({
        "ok": true,
        "algo": "ops_metrics_v1",
        "process": {
            "worker_id": st.worker_id,
            "role": format!("{:?}", st.role),
            "store_backend": st.store.backend_name(),
            "draining": st.draining.load(Ordering::Relaxed),
            "uptime_ms": (now_ms - st.boot_ms).max(0),
            "boot_ms": st.boot_ms,
            "product_version": env!("CARGO_PKG_VERSION"),
            "analyze_workers_target": st.analyze_workers_target.load(Ordering::Relaxed),
            "analyze_runs": st.analyze_runs.load(Ordering::Relaxed),
            "hot_vts": st.hot_probe.len_hot(),
            "pending_analyze_jobs": st.store.pending_analyze_job_count().ok(),
        },
        "mem_kb": { "rss": rss_kb, "vms": vms_kb },
        "rate_limit": crate::rate_limit::stats(),
    })
}

/// Ops: replay a session through the pure evaluate chain and diff the result
/// against what was stored at analyze time. Uses the same canonical evidence
/// (build_evidence) and the same visibility stamps as the analyze path, so an
/// identical run reports `identical: true`; volatile peer-dependent keys and
/// storage-only markers are excluded from the diff.
pub fn ops_replay(st: &AppState, body: Value) -> Value {
    let session_id = body
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if session_id.is_empty() {
        return json!({"ok": false, "error": "session_id_required"});
    }
    let events = match st.store.list_observation_events(&session_id, 200) {
        Ok(ev) => ev,
        Err(e) => return json!({"ok": false, "error": format!("events: {e}")}),
    };
    let stored = match st.store.latest_analysis(&session_id) {
        Ok(v) => v,
        Err(e) => return json!({"ok": false, "error": format!("stored: {e}")}),
    };
    let evidence = match st.store.build_evidence(&session_id) {
        Ok(e) => e,
        Err(e) => return json!({"ok": false, "error": format!("evidence: {e}")}),
    };
    let cur_mask = gr_probe_core::analyze_mask::build_mask(
        &evidence,
        gr_probe_core::GR_PRODUCT_VERSION,
    );
    let stored_mask = st
        .store
        .session_meta(&session_id)
        .ok()
        .flatten()
        .and_then(|m| m.get("analyzed_mask_v1").cloned());
    // Same peer context as the original analyze call — evaluate is peer-aware.
    let (peer_vec, peer_ev, _peer_ids) = resolve_multi_session_peers(&st.store, &session_id, &[]);
    let mut recomputed = match gr_probe_core::evaluate::evaluate_session(
        &evidence,
        None,
        peer_vec.as_ref(),
        peer_ev.as_ref(),
        st.soft_v2_ready,
    ) {
        Ok(r) => r,
        Err(e) => return json!({"ok": false, "error": format!("evaluate: {e}")}),
    };
    // Same observability stamps as analyze_session so the diff is about the
    // algorithm, not about handler-side bookkeeping.
    gr_probe_core::analyze_mask::stamp_observability(
        &mut recomputed,
        &evidence,
        &session_id,
        gr_probe_core::GR_PRODUCT_VERSION,
        stored_mask.as_ref(),
        &cur_mask,
    );
    // Reproduce the same handler-side evidence policy as the live analyze save
    // (evidence_withheld band upgrade, opus5 P0-1) — otherwise replay always
    // diverges on the stored band by construction.
    apply_evidence_withheld(&st.store, &session_id, &mut recomputed);
    // Semantic equality on decision-relevant paths is the authoritative check
    // (handler attachments like peer_similarity / re-gated sdk_return are
    // intentionally out of scope). Full top-level diff stays informational.
    let recomputed_stored = gr_probe_store::slim_analysis_result_for_storage(&recomputed);
    let empty_stored = json!({});
    let stored_v = stored.as_ref().unwrap_or(&empty_stored);
    let semantic = gr_probe_core::selection_provenance::semantic_diff(&recomputed_stored, stored_v);
    let diff = gr_probe_core::selection_provenance::diff_results(
        &recomputed_stored,
        stored_v,
        &["analysis_rev", "analyzed_ms"],
    );
    json!({
        "ok": true,
        "algo": "replay_v1",
        "session_id": session_id,
        "event_count": events.len(),
        "input_hash": gr_probe_core::analyze_mask::evidence_input_hash(&evidence),
        "has_stored": stored.is_some(),
        "stored_rev": stored
            .as_ref()
            .and_then(|r| r.get("analysis_rev"))
            .cloned()
            .unwrap_or(Value::Null),
        "mask_matched": gr_probe_core::analyze_mask::mask_matches(
            stored_mask.as_ref(),
            &cur_mask,
        ),
        "mask_debug": {
            "stored": stored_mask.clone().unwrap_or(Value::Null),
            "current": cur_mask,
        },
        "semantic": semantic,
        "diff": diff,
    })
}

pub fn ops_identity_governance(_st: &AppState) -> Value {
    json!({
        "ok": true,
        "algo": "identity_governance_ops_v1",
        "product_version": env!("CARGO_PKG_VERSION"),
        "shared": gr_probe_core::shared_state_paths(),
        "shared_metrics": gr_probe_core::shared_governance_metrics(),
        "l2": gr_probe_core::l2_metrics(),
        "mu_census": gr_probe_core::mu_census_snapshot(),
        "hnsw": gr_probe_core::hnsw_stats(),
        "contrastive": gr_probe_core::contrastive_stats(),
        "fs_taus_adopted": gr_probe_core::adopted_fs_taus().map(|(m, s)| json!({"tau_merge": m, "tau_split": s})),
        "confidence": {
            "runtime_version": active_confidence_version(),
            "adopt": adopt_decision_report(),
        },
        "notes": [
            "Hot-bucket distribution is process-local; multi-worker see shared_governance files",
            "Assertions never hard-ban; downweight/deepen/ephemeral only",
        ],
    })
}

pub fn ops_overview(st: &AppState) -> Value {
    let pending = st.store.pending_analyze_job_count().unwrap_or(-1);
    let queue = st.store.analyze_queue_stats().unwrap_or(json!({}));
    json!({
        "ok": true,
        "service": "gr-service",
        "backend": st.store.backend_name(),
        "role": format!("{:?}", st.role).to_ascii_lowercase(),
        "pending_analyze_jobs": pending,
        "analyze_queue": queue,
        "analyze_runs": st.analyze_runs.load(Ordering::Relaxed),
        "soft_store": {
            "backend": st.soft_store_backend,
            "fuse_threshold": st.soft_store.fuse_threshold(),
            "fuse_owner": st.soft_store.fuse_owner(),
            "promote": PROMOTE_TO_COMMERCIAL_ID,
        },
        "confidence_runtime_version": active_confidence_version(),
        "confidence_adopt": adopt_decision_report(),
        "identity_governance_ops": "/v1/ops/identity_governance",
        "links": {
            "session_ops": "/v1/ops/session/:id",
            "ops_ui": "/ops",
            "identity_sla": "/v1/ops/identity_sla",
            "identity_governance": "/v1/ops/identity_governance",
            "device": "/v1/ops/device/:device_id",
            "device_heat": "/v1/ops/device_heat/:device_id",
            "device_index": "/v1/ops/device_index?tenant_id=",
            "analysis_latest": "/v1/ops/analysis_latest",
            "binder_lookup": "/v1/ops/binder_lookup?binder_key=",
            "cross_query": "/v1/ops/cross_query",
            "soft_edges": "/v1/ops/soft_edges",
            "unknown_hub": "/v1/ops/unknown_hub_aggregate",
            "probe_coverage_gaps": "/v1/ops/probe_coverage_gaps",
            "precision_matrix": "/v1/ops/precision_matrix",
            "probe_health": "/v1/ops/probe_health",
            "self_capability": "/v1/ops/self_capability",
            "conf_calibrate": "/v1/ops/conf_calibrate",
            "r100_pack": "/v1/r100/pack/Rxx_spotcheck.js",
            "r100_status": "/v1/ops/r100",
            "collision_kpi": "/v1/ops/collision_kpi",
            "dsar_erase": "POST /v1/ops/dsar/erase {kind,value}",
            "dsar_export": "/v1/ops/dsar/export?kind=&value=",
            "analyses": "/v1/session/:id/analyses",
            "evidence": "/v1/session/:id/evidence",
            "health": "/v1/health",
            "brain_report": "/opt/green-v5/reports/brain_dynamics_latest.json",
        },
        "policy": {
            "terminal": "coverage_complete (static+B8+mid), not real_band alone",
            "resources": "maximize analyze; no thrift caps",
            "soft_never_promote": true,
        }
    })
}

/// Identity SLA 大盘 — prefer `analysis_latest` scalars (no result_json TOAST).
pub fn ops_identity_sla(st: &AppState) -> AppResult {
    // P0 path: materialised scalars (fast). Fallback only if table empty / PG missing.
    let latest = st
        .store
        .list_analysis_latest(500, None, None, None)
        .unwrap_or_else(|_| json!({"ok": true, "count": 0, "rows": []}));
    let scalar_rows: Vec<Value> = latest
        .get("rows")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let (rows, source, n): (Vec<SlaSessionRow>, &str, usize) = if !scalar_rows.is_empty() {
        (
            scalar_rows
                .iter()
                .map(SlaSessionRow::from_analysis_latest_scalar)
                .collect(),
            "analysis_latest",
            scalar_rows.len(),
        )
    } else {
        // SQLite / pre-backfill: full evaluate JSON (slow on large PG TOAST — avoid in prod)
        let analyses = st
            .store
            .list_recent_latest_analyses(200)
            .map_err(|e| ApiError(500, e.to_string()))?;
        let n = analyses.len();
        (
            analyses
                .iter()
                .map(SlaSessionRow::from_evaluate_result)
                .collect(),
            "store_recent_analyses_fallback",
            n,
        )
    };
    let mut report = aggregate_identity_sla(&rows);
    if let Some(obj) = report.as_object_mut() {
        obj.insert("source".into(), json!(source));
        obj.insert("n_store_analyses".into(), json!(n));
    }
    // Soft heat for top collision ids (in-memory soft store — cheap)
    let mut heats = Vec::new();
    if let Some(arr) = report
        .get("unique_device_id_heatmap")
        .and_then(|v| v.as_array())
    {
        for item in arr.iter().take(20) {
            if let Some(dv) = item.get("device_id").and_then(|v| v.as_str()) {
                heats.push(commercial_id_heat_report(st.soft_store.as_ref(), "default", dv));
            }
        }
    }
    let alerts = report.get("alerts").cloned().unwrap_or(json!({}));
    Ok(json!({
        "ok": true,
        "sla": report,
        "alerts": alerts,
        "device_heats": heats,
        "soft_promote": false,
        "confidence_runtime_version": active_confidence_version(),
        "confidence_adopt": adopt_decision_report(),
        "query_source": source,
        "daily_report_hint": "scripts/report_identity_sla_daily.sh",
    }))
}

#[derive(Deserialize)]
pub struct SlaPostBody {

    /// Optional array of evaluate results and/or matrix cells
    pub cells: Option<Vec<Value>>,}

pub fn ops_identity_sla_post(_st: &AppState, body: SlaPostBody) -> Value {
    let cells = body.cells.unwrap_or_default();
    let rows = rows_from_json_array(&cells);
    let report = aggregate_identity_sla(&rows);
    json!({
        "ok": true,
        "sla": report,
        "soft_promote": false,
        "confidence_runtime_version": RUNTIME_CONFIDENCE_VERSION,
    })
}

pub fn ops_device_heat(st: &AppState, device_id: &str) -> AppResult {
    Ok(commercial_id_heat_report(
        st.soft_store.as_ref(),
        "default",
        device_id,
    ))
}

#[derive(Deserialize)]
pub struct SoftEdgeBody {

    pub tenant: Option<String>,
    pub a_session: String,
    pub b_session: String,
    pub priority: Option<String>,
    pub confidence: Option<f64>,
    pub reason: Option<String>,}

pub fn ops_soft_edge_put(st: &AppState, body: SoftEdgeBody) -> AppResult {
    let tenant = body.tenant.unwrap_or_else(|| "default".into());
    let edge = SoftEdge {
        a_session: body.a_session,
        b_session: body.b_session,
        priority: body.priority.unwrap_or_else(|| "p1".into()),
        promote_to_commercial_id: true, // forced false in store
        confidence: body.confidence.unwrap_or(0.0),
        reason: body.reason.unwrap_or_else(|| "api".into()),
    };
    st.soft_store
        .put_edge(&tenant, &edge)
        .map_err(|e| ApiError(409, e))?;
    let list = st
        .soft_store
        .list_edges(&tenant)
        .map_err(|e| ApiError(500, e))?;
    Ok(json!({
        "ok": true,
        "n_edges": list.len(),
        "promote_to_commercial_id": PROMOTE_TO_COMMERCIAL_ID,
        "fuse_threshold": st.soft_store.fuse_threshold(),
    }))
}

pub fn ops_soft_edges_list(st: &AppState) -> AppResult {
    let list = st
        .soft_store
        .list_edges("default")
        .map_err(|e| ApiError(500, e))?;
    Ok(json!({
        "ok": true,
        "edges": list.iter().map(|e| e.to_value()).collect::<Vec<_>>(),
        "promote_to_commercial_id": PROMOTE_TO_COMMERCIAL_ID,
    }))
}

#[derive(Deserialize)]
pub struct HeatRecordBody {

    pub tenant: Option<String>,
    pub device_id: String,
    pub session_id: String,}

pub fn ops_device_heat_record(st: &AppState, body: HeatRecordBody) -> AppResult {
    let tenant = body.tenant.unwrap_or_else(|| "default".into());
    let n = st
        .soft_store
        .record_device_id_sighting(&tenant, &body.device_id, &body.session_id)
        .map_err(|e| ApiError(400, e))?;
    Ok(json!({
        "ok": true,
        "session_count": n,
        "heat": commercial_id_heat_report(st.soft_store.as_ref(), &tenant, &body.device_id),
    }))
}

#[derive(Deserialize)]
pub struct ConfCalBody {

    pub pairs: Option<Value>,
    pub matrix_cells: Option<Vec<Value>>,}

pub fn ops_conf_calibrate(body: ConfCalBody) -> Value {
    let mut pairs = body
        .pairs
        .as_ref()
        .map(pairs_from_json)
        .unwrap_or_default();
    if let Some(cells) = body.matrix_cells.as_ref() {
        pairs.extend(pairs_from_matrix_cells(cells));
    }
    let report = calibrate_offline(&pairs);
    let adopt = adopt_decision_report();
    json!({
        "ok": report.get("ok").and_then(|v| v.as_bool()).unwrap_or(false),
        "report": report,
        "n_pairs": pairs.len(),
        "runtime_confidence_version": active_confidence_version(),
        "default_if_not_adopted": RUNTIME_CONFIDENCE_VERSION,
        "confidence_adopt": adopt,
        "silent_relabel_forbidden": true,
        "note": "POST pairs for offline ECE; set GR_CONF_ADOPT=1 + ECE gate for hot-path adopt",
    })
}

/// Export multi-tenant DeviceIndex (production path).
pub fn ops_device_index(st: &AppState, query: &HashMap<String, String>) -> AppResult {
    let tenant = query
        .get("tenant_id")
        .map(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("default");
    let exp = st
        .store
        .device_index_export(tenant)
        .map_err(|e| ApiError(500, e.to_string()))?;
    Ok(json!({
        "ok": true,
        "tenant_id": tenant,
        "device_index": exp,
        "multi_tenant": true,
        "backend": st.store.backend_name(),
    }))
}

/// iss/38 N2 / D-15: service-level capability bitmap (not session envelope).
pub fn ops_self_capability() -> Value {
    let cap = self_capability_json();
    json!({
        "ok": true,
        "self_capability": cap,
        "note": "service-level; distinct from per-session capability envelope",
    })
}

// ── R100 pseudo-static templates ────────────────────────────────────────────

/// GET /v1/r100/pack/{Rxx_spotcheck}.js — application/javascript body.
pub fn r100_pack_js(st: &AppState, pack_id: &str) -> Result<String, ApiError> {
    let pid = pack_id.trim().trim_end_matches(".js");
    st.r100
        .render_js(pid)
        .map_err(|e| ApiError(404, e))
}

/// GET /v1/r100/random.js?seed=… — server-side pick one template (ops/lab).
pub fn r100_random_js(st: &AppState, query: &HashMap<String, String>) -> Result<(String, String), ApiError> {
    let seed = query
        .get("seed")
        .and_then(|s| s.parse::<u64>().ok())
        .or_else(|| {
            query
                .get("session_id")
                .map(|s| {
                    use std::collections::hash_map::DefaultHasher;
                    use std::hash::{Hash, Hasher};
                    let mut h = DefaultHasher::new();
                    s.hash(&mut h);
                    h.finish()
                })
        })
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(1)
        });
    st.r100
        .pick_js(seed)
        .map_err(|e| ApiError(503, e))
}

/// GET /v1/ops/r100 — catalog status.
pub fn ops_r100_status(st: &AppState) -> Value {
    json!({
        "ok": true,
        "r100": st.r100.status(),
        "mode": "pseudo_static_templates",
        "load_path": "FE: registry.random.rt.js (static) + GET /v1/r100/pack/Rxx.js (template)",
        "brain": "1 Rxx_spotcheck per tick; same progressive plan path as mid/dense",
    })
}

/// POST /v1/ops/r100/seed_redis — push memory catalog into Redis.
pub fn ops_r100_seed_redis(st: &AppState) -> Value {
    match st.r100.seed_redis_from_memory() {
        Ok(n) => json!({
            "ok": true,
            "seeded": n,
            "source": st.r100.source_label(),
        }),
        Err(e) => json!({
            "ok": false,
            "error": e,
            "source": st.r100.source_label(),
        }),
    }
}

#[derive(Deserialize)]
pub struct UnknownHubBody {
    /// Optional explicit session fragments: [{session_id, unknown_bucket}, ...]
    pub sessions: Option<Vec<Value>>,
    /// When true (default), also scan recent store session meta for unknown_bucket.
    pub from_store: Option<bool>,
    pub limit: Option<usize>,
}

/// iss/38 N1 / D-14: aggregate unknown_bucket into Hub-shaped summary.
pub fn ops_unknown_hub_aggregate(st: &AppState, body: UnknownHubBody) -> Value {
    let mut sessions = body.sessions.unwrap_or_default();
    let from_store = body.from_store.unwrap_or(true);
    let limit = body.limit.unwrap_or(200).min(2000);
    if from_store {
        if let Ok(analyses) = st.store.list_recent_latest_analyses(limit) {
            for a in analyses {
                let sid = a
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let ub = a
                    .pointer("/result/unknown_bucket")
                    .or_else(|| a.get("unknown_bucket"))
                    .or_else(|| a.pointer("/result/product/unknown_bucket"))
                    .cloned()
                    .unwrap_or(Value::Null);
                // Also try session meta via window if we patch later; for now analysis path.
                if !ub.is_null() {
                    sessions.push(json!({
                        "session_id": sid,
                        "unknown_bucket": ub,
                    }));
                }
            }
        }
        // Fallback: recent ids + session meta_json via merge identity (read window meta if present)
        if let Ok(ids) = st.store.list_recent_session_ids(limit) {
            for sid in ids {
                if sessions.iter().any(|s| s.get("session_id").and_then(|v| v.as_str()) == Some(sid.as_str())) {
                    continue;
                }
                if let Ok(w) = st.store.session_window(&sid) {
                    if let Some(ub) = w.get("unknown_bucket").cloned().filter(|v| !v.is_null()) {
                        sessions.push(json!({
                            "session_id": sid,
                            "unknown_bucket": ub,
                        }));
                    } else if let Some(meta) = w.get("meta").cloned() {
                        if let Some(ub) = meta.get("unknown_bucket").cloned().filter(|v| !v.is_null()) {
                            sessions.push(json!({
                                "session_id": sid,
                                "unknown_bucket": ub,
                                "meta": meta,
                            }));
                        }
                    }
                }
            }
        }
    }
    let hub = aggregate_unknown_buckets(&sessions);
    let drafts = hub_promotion_drafts(&hub);
    json!({
        "ok": true,
        "hub": hub,
        "promotion_drafts": drafts,
        "source_sessions_n": sessions.len(),
        "from_store": from_store,
    })
}

/// Unified probe observability: route/gap/B10/upload fail codes (architecture review §11.8).
pub fn ops_probe_health(st: &AppState, query: &HashMap<String, String>) -> Value {
    let limit = query
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(200)
        .min(2000);
    let mut b10_ok = 0u64;
    let mut b10_missing = 0u64;
    let mut dh = 0u64;
    let mut dv = 0u64;
    let mut dg = 0u64;
    let mut gap_n = 0u64;
    let mut sealed_ok = 0u64;
    let mut sealed_reject = 0u64;
    let mut upload_fail: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    let mut lane_counts: std::collections::HashMap<String, u64> = std::collections::HashMap::new();

    if let Ok(analyses) = st.store.list_recent_latest_analyses(limit) {
        for a in analyses {
            let id = a
                .get("device_id")
                .or_else(|| a.pointer("/result/product/device_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if gr_probe_core::device_segments::is_multi_segment_id(id) || gr_probe_core::is_dv_id(id) {
                // Multi-segment product surface (dv0/4/5/6) counts under commercial dv bucket.
                dv += 1;
            } else if gr_probe_core::is_dh_id(id) {
                dh += 1;
            } else if gr_probe_core::is_dg_id(id) {
                dg += 1;
            }
            let digest = a
                .get("digest_path")
                .or_else(|| a.pointer("/result/product/digest_path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if digest.contains("real_curves") {
                b10_ok += 1;
            } else if digest.contains("empty_anchor") {
                b10_missing += 1;
            }
            if a.pointer("/result/product/probe_coverage_gap/present")
                .and_then(|v| v.as_bool())
                == Some(true)
            {
                gap_n += 1;
            }
            if let Some(packs) = a
                .pointer("/result/route_plan/packs")
                .and_then(|v| v.as_array())
            {
                for p in packs {
                    let lane = p
                        .get("pack_lane")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    *lane_counts.entry(lane.to_string()).or_default() += 1;
                }
            }
        }
    }
    // Client ops events (upload fails)
    if let Ok(ev) = st.store.list_ops_events("client", limit as i64, None, None, None, None) {
        if let Some(arr) = ev.get("events").and_then(|v| v.as_array()) {
            for e in arr {
                let stage = e.get("stage").and_then(|v| v.as_str()).unwrap_or("");
                let code = e.get("code").and_then(|v| v.as_str()).unwrap_or("");
                if stage == "upload" || code.starts_with("upload_") || code.contains("sla") {
                    *upload_fail.entry(code.to_string()).or_default() += 1;
                }
            }
        }
    }
    if let Ok(ev) = st.store.list_ops_events("server", limit as i64, None, None, None, None) {
        if let Some(arr) = ev.get("events").and_then(|v| v.as_array()) {
            for e in arr {
                let code = e.get("code").and_then(|v| v.as_str()).unwrap_or("");
                if code == "sealed_ok" {
                    sealed_ok += 1;
                }
                if code == "sealed_required_reject" {
                    sealed_reject += 1;
                }
            }
        }
    }
    let b10_health = st
        .store
        .ops_b10_health(
            (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0)
                - 3_600_000)
                .max(0),
            20,
        )
        .unwrap_or(json!(null));

    let mut upload_top: Vec<Value> = upload_fail
        .into_iter()
        .map(|(k, n)| json!({"code": k, "count": n}))
        .collect();
    upload_top.sort_by(|a, b| {
        b.get("count")
            .and_then(|v| v.as_u64())
            .cmp(&a.get("count").and_then(|v| v.as_u64()))
    });
    upload_top.truncate(20);

    json!({
        "ok": true,
        "algo": "probe_health_v1",
        "window_analyses": limit,
        "require_sealed_ingest": gr_abi::env::flag("REQUIRE_SEALED_INGEST"),
        "allow_plain_ingest": gr_abi::env::flag("ALLOW_PLAIN_INGEST"),
        "device_tier": {"dh": dh, "dv": dv, "dg": dg},
        "b10": {
            "real_curves_n": b10_ok,
            "empty_anchor_n": b10_missing,
            "complete_ratio": if b10_ok + b10_missing > 0 {
                b10_ok as f64 / (b10_ok + b10_missing) as f64
            } else { 0.0 }
        },
        "probe_coverage_gap_sessions": gap_n,
        "route_plan_pack_lanes": lane_counts,
        "upload_fail_top": upload_top,
        "sealed": {"ok": sealed_ok, "required_reject": sealed_reject},
        "b10_health_detail": b10_health,
        "links": {
            "probe_coverage_gaps": "/v1/ops/probe_coverage_gaps",
            "precision_matrix": "/v1/ops/precision_matrix",
            "unknown_hub": "/v1/ops/unknown_hub_aggregate",
        },
    })
}

/// Aggregate engine-aware probe_coverage_gap across recent analyses (B10x catalog feedback).
pub fn ops_probe_coverage_gaps(st: &AppState, body: UnknownHubBody) -> Value {
    let mut sessions = body.sessions.unwrap_or_default();
    let from_store = body.from_store.unwrap_or(true);
    let limit = body.limit.unwrap_or(300).min(2000);
    if from_store {
        if let Ok(analyses) = st.store.list_recent_latest_analyses(limit) {
            for a in analyses {
                let sid = a
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let gap = a
                    .pointer("/result/product/probe_coverage_gap")
                    .or_else(|| a.pointer("/product/probe_coverage_gap"))
                    .or_else(|| a.pointer("/result/probe_coverage_gap"))
                    .cloned()
                    .unwrap_or(Value::Null);
                let env_gap = a
                    .pointer("/result/product/env/probe_coverage_gap")
                    .cloned()
                    .unwrap_or(Value::Null);
                let g = if !gap.is_null() { gap } else { env_gap };
                if g.is_null() {
                    continue;
                }
                sessions.push(json!({
                    "session_id": sid,
                    "device_id": a.get("device_id").cloned().unwrap_or(Value::Null),
                    "probe_coverage_gap": g,
                    "product": {
                        "probe_coverage_gap": g,
                        "device_id": a.get("device_id").cloned().unwrap_or(Value::Null),
                    },
                }));
            }
        }
    }
    let hub = gr_probe_core::engine_surface::aggregate_probe_coverage_gaps(&sessions);
    json!({
        "ok": true,
        "hub": hub,
        "source_sessions_n": sessions.len(),
        "from_store": from_store,
        "algo": "ops_probe_coverage_gaps_v1",
    })
}

#[derive(Deserialize)]
pub struct PrecisionMatrixBody {
    pub session_ids: Option<Vec<String>>,
    pub limit: Option<usize>,
    /// Optional reference engine for peer compare (default blink).
    pub peer_engine: Option<String>,
}

/// Same-host residual precision matrix KPI from recent analysis residual materials.
pub fn ops_precision_matrix(st: &AppState, body: PrecisionMatrixBody) -> Value {
    let limit = body.limit.unwrap_or(40).min(200);
    let peer_eng = body
        .peer_engine
        .unwrap_or_else(|| "blink".into())
        .to_ascii_lowercase();
    let mut rows: Vec<Value> = Vec::new();
    let mut peer_curve: Option<Vec<f64>> = None;
    let mut by_engine: std::collections::HashMap<String, Vec<Value>> =
        std::collections::HashMap::new();

    if let Ok(analyses) = st.store.list_recent_latest_analyses(limit) {
        for a in analyses {
            let sid = a
                .get("session_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if let Some(ref want) = body.session_ids {
                if !want.iter().any(|w| w == &sid) {
                    continue;
                }
            }
            // Prefer materials residual paths curve; fall back to product residual
            let fields = a
                .pointer("/result/device/trust/materials")
                .or_else(|| a.pointer("/result/product/trust/materials"))
                .or_else(|| a.pointer("/fields"))
                .cloned()
                .unwrap_or(Value::Null);
            let eng = a
                .pointer("/result/product/env/engine_family")
                .or_else(|| fields.get("residual_probe_engine"))
                .or_else(|| fields.get("engine_family"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let device_id = a
                .get("device_id")
                .or_else(|| a.pointer("/result/product/device_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let curve: Vec<f64> = fields
                .get("hw_curve_webgl")
                .or_else(|| {
                    // residual_paths chosen
                    let paths = fields.get("residual_paths").and_then(|v| v.as_array());
                    let chosen = fields
                        .pointer("/residual_select/chosen_path_id")
                        .and_then(|v| v.as_str());
                    paths.and_then(|arr| {
                        arr.iter()
                            .find(|p| {
                                chosen.is_some()
                                    && p.get("path_id").and_then(|v| v.as_str()) == chosen
                            })
                            .or_else(|| arr.iter().find(|p| p.get("ok").and_then(|v| v.as_bool()) == Some(true)))
                            .and_then(|p| p.get("curve"))
                    })
                })
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_f64().or_else(|| x.as_i64().map(|i| i as f64)))
                        .collect()
                })
                .unwrap_or_default();
            if curve.len() < 8 {
                continue;
            }
            if eng == peer_eng && peer_curve.is_none() {
                peer_curve = Some(curve.clone());
            }
            let row = gr_probe_core::engine_surface::precision_matrix_row(
                &sid,
                &eng,
                &device_id,
                &curve,
                None,
            );
            by_engine.entry(eng.clone()).or_default().push(row.clone());
            rows.push(row);
        }
    }

    // Recompute vs peer when peer curve known
    if let Some(ref pc) = peer_curve {
        let peer_c = gr_probe_core::engine_surface::webgl_comm_components(pc);
        let peer_mu = peer_c
            .get("mu_0p001")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let peer_sd = peer_c
            .get("sd_0p001")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        for r in rows.iter_mut() {
            let mu = r
                .pointer("/commercial/mu_0p001")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let sd = r
                .pointer("/commercial/sd_0p001")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if let Some(obj) = r.as_object_mut() {
                obj.insert(
                    "vs_peer_commercial".into(),
                    json!({
                        "peer_engine": peer_eng,
                        "mu_eq": mu == peer_mu && !mu.is_empty(),
                        "sd_eq": sd == peer_sd && !sd.is_empty(),
                        "peer_mu": peer_mu,
                        "peer_sd": peer_sd,
                    }),
                );
            }
        }
    }

    // Summary KPI
    let mut mu_eq_n = 0u64;
    let mut sd_eq_n = 0u64;
    let mut n = 0u64;
    for r in &rows {
        if let Some(v) = r.get("vs_peer_commercial") {
            n += 1;
            if v.get("mu_eq").and_then(|x| x.as_bool()) == Some(true) {
                mu_eq_n += 1;
            }
            if v.get("sd_eq").and_then(|x| x.as_bool()) == Some(true) {
                sd_eq_n += 1;
            }
        }
    }

    json!({
        "ok": true,
        "algo": "same_host_precision_matrix_v1",
        "peer_engine": peer_eng,
        "rows_n": rows.len(),
        "by_engine_n": by_engine.iter().map(|(k,v)| json!({"engine": k, "n": v.len()})).collect::<Vec<_>>(),
        "kpi": {
            "rows_with_peer": n,
            "mu_0p001_eq_ratio": if n > 0 { mu_eq_n as f64 / n as f64 } else { 0.0 },
            "sd_0p001_eq_ratio": if n > 0 { sd_eq_n as f64 / n as f64 } else { 0.0 },
            "note": "sd_eq_ratio is the main cross-engine commercial fork KPI on same host",
        },
        "rows": rows,
    })
}

#[derive(Deserialize)]
pub struct ChallengeRotateBody {
    pub tenant: Option<String>,
    pub epoch: Option<u64>,
    pub base_secret: Option<String>,
}

/// iss/39 R7: challenge/pack epoch rotation skeleton.
pub fn ops_challenge_rotate(body: ChallengeRotateBody) -> Value {
    let tenant = body.tenant.unwrap_or_else(|| "default".into());
    let epoch = body.epoch.unwrap_or(0);
    let secret = body
        .base_secret
        .unwrap_or_else(|| DEFAULT_CHALLENGE_SECRET.to_string());
    let rot = if epoch == 0 {
        next_rotation(&tenant, 0, &secret)
    } else {
        next_rotation(&tenant, epoch, &secret)
    };
    json!({"ok": true, "rotation": rot})
}

#[derive(Deserialize)]
pub struct CollisionKpiBody {
    pub observations: Option<Vec<Value>>,
}

/// iss/38 N4: POST fixture/fleet observations → collision KPI report.
pub fn ops_collision_kpi(body: CollisionKpiBody) -> Value {
    let obs = body.observations.unwrap_or_default();
    let report = gr_probe_core::report_collision_kpi(&obs);
    json!({
        "ok": report.get("ok").and_then(|v| v.as_bool()).unwrap_or(false),
        "collision_kpi": report,
        "note": "fixture/fleet pool only — not global UV SLA",
    })
}

pub fn evaluate_pure(body: EvaluateBody) -> AppResult {
    // Offline path: soft comes from body (default false). Session analyze never uses this.
    let soft = body.soft_v2_ready;
    let result = evaluate_session(
        &body.evidence,
        body.strategy.as_ref(),
        body.peer.as_ref(),
        None,
        soft,
    )
    .map_err(|e| ApiError(422, e))?;
    Ok(json!({
        "ok": true,
        "result": result,
        "soft_v2_ready": soft,
        "soft_v2_ready_source": "evaluate_body_default_false",
        "note": "pure evaluate is offline/lab; session analyze uses AppState.soft_v2_ready only",
    }))
}


pub async fn analyze_worker_loop(
    store: Arc<Store>,
    soft_v2_ready: bool,
    worker_id: String,
    analyze_runs: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
    wakeup: Option<Arc<tokio::sync::Notify>>,
    queue_depth: Option<Arc<AtomicU64>>,
) {
    log::info!("analyze worker started worker_id={worker_id}");
    let (idle_min, idle_max) = analyze_idle_poll_ms_range();
    let mut idle_sleep_ms = idle_min;
    loop {
        // Hot-scaling (gpt5.5 P1): the supervisor retires workers by setting
        // this flag; exit gracefully between jobs.
        if cancel.load(Ordering::Relaxed) {
            log::info!("analyze worker stopping (scaled down) worker_id={worker_id}");
            return;
        }
        // Flood drain mode (1.0.10): pending > soft cap → raise the claim
        // batch so each roundtrip drains 4× the jobs (178 flood measured
        // ~110 jobs/min at batch 4 with 4 workers; claim was not the
        // bottleneck but roundtrips scale linearly with backlog here).
        let base_batch = gr_probe_store::analyze_claim_batch();
        let flood_batch = gr_probe_store::analyze_claim_batch_flood();
        let depth = queue_depth
            .as_ref()
            .map(|d| d.load(Ordering::Relaxed))
            .unwrap_or(0);
        let qmax = gr_probe_store::analyze_queue_max().max(1) as u64;
        let claim_limit = if depth > qmax && flood_batch > base_batch {
            flood_batch
        } else {
            base_batch
        };
        let claimed = match store.claim_due_analyze_jobs(
            &worker_id,
            claim_limit,
            ANALYZE_LOCK_MS,
        ) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("claim_due_analyze_jobs failed: {e}");
                tokio::time::sleep(Duration::from_millis(idle_min.max(100))).await;
                continue;
            }
        };
        if claimed.is_empty() {
            // Exponential backoff on empty queue — 20ms fixed poll caused ~1–2k TPS
            // of empty claim SQL on multi-worker prod (fsync / load storm).
            // P2: LISTEN/NOTIFY 唤醒优先 (调度臂 due<1s 时 pg_notify), 退避计时
            // 只作兜底 — 空轮询 claim 风暴与 worker 数解耦。
            if let Some(n) = wakeup.as_ref() {
                tokio::select! {
                    _ = n.notified() => {
                        idle_sleep_ms = idle_min;
                    }
                    _ = tokio::time::sleep(Duration::from_millis(idle_sleep_ms)) => {
                        idle_sleep_ms = idle_sleep_ms.saturating_mul(2).clamp(idle_min, idle_max);
                    }
                }
            } else {
                tokio::time::sleep(Duration::from_millis(idle_sleep_ms)).await;
                idle_sleep_ms = idle_sleep_ms.saturating_mul(2).clamp(idle_min, idle_max);
            }
            continue;
        }
        idle_sleep_ms = idle_min;
        for sid in claimed {
            if cancel.load(Ordering::Relaxed) {
                // Remaining claimed rows keep their lock until ANALYZE_LOCK_MS
                // expires, then are re-claimed by the surviving workers.
                log::info!("analyze worker stopping mid-batch worker_id={worker_id}");
                return;
            }
            let store2 = store.clone();
            let wid = worker_id.clone();
            let soft = soft_v2_ready;
            let sid2 = sid.clone();
            let out = tokio::task::spawn_blocking(move || {
                // blocking path — evaluate is CPU/sync
                let rt_sid = sid2;
                // use a tiny helper without async
                if let Err(e) = store2.require_active_session(&rt_sid) {
                    let _ = store2.complete_analyze_job(&rt_sid, &wid);
                    return Err(e.to_string());
                }
                let mut evidence = store2.build_evidence(&rt_sid).map_err(|e| e.to_string())?;
                // Same promotion as analyze_session: page_id from merged fields → evidence root
                if evidence.get("page_id").is_none() {
                    if let Some(pid) = evidence.pointer("/fields/page_id").cloned() {
                        if let Some(obj) = evidence.as_object_mut() {
                            obj.insert("page_id".into(), pid);
                        }
                    }
                }
                let (peer_vec, peer_ev, _peers) =
                    resolve_multi_session_peers(&store2, &rt_sid, &[]);
                let mut result = evaluate_session(
                    &evidence,
                    None,
                    peer_vec.as_ref(),
                    peer_ev.as_ref(),
                    soft,
                )
                .map_err(|e| e)?;
                // Parity with the HTTP analyze path: stamp materials fp and the
                // observability envelope (selection / analyze_dirty), and persist
                // the input mask, so worker-saved analyses are replay-identical
                // and mask reuse works regardless of which path saved last.
                {
                    let fields = evidence.get("fields").cloned().unwrap_or(json!({}));
                    let fp = gr_probe_core::silicon_materials_fingerprint(&fields, None);
                    if let Some(obj) = result.as_object_mut() {
                        let mut diag = obj.get("diagnostics").cloned().unwrap_or(json!({}));
                        if let Some(d) = diag.as_object_mut() {
                            d.insert("silicon_materials_fp".into(), json!(fp));
                        }
                        obj.insert("diagnostics".into(), diag);
                    }
                }
                let cur_mask = gr_probe_core::analyze_mask::build_mask(
                    &evidence,
                    gr_probe_core::GR_PRODUCT_VERSION,
                );
                // P2: prior_meta 一次读出 — 掩码差分 / brain 单调 / history
                // 滚动共用 (原 session_meta + persist_brain_control prior +
                // mask merge + ticket merge 各自读改写)。
                let prior_meta: Value = store2
                    .session_meta(&rt_sid)
                    .ok()
                    .flatten()
                    .unwrap_or(json!({}));
                let prev_mask: Option<Value> = prior_meta.get("analyzed_mask_v1").cloned();
                gr_probe_core::analyze_mask::stamp_observability(
                    &mut result,
                    &evidence,
                    &rt_sid,
                    gr_probe_core::GR_PRODUCT_VERSION,
                    prev_mask.as_ref(),
                    &cur_mask,
                );
                // Parity with the HTTP analyze path (iss/opus5 03-P0-1):
                // apply the planned-vs-actual evidence ledger here too, so a
                // worker-saved result carries the same withholding band as the
                // HTTP path for identical evidence. Without this, the worker
                // overwrite (and later analyzed_mask_v1 reuse) freezes the
                // inferior "insufficient" band and replay diverges.
                apply_evidence_withheld(&store2, &rt_sid, &mut result);
                // P2 单事务写回: analysis 落库 + 剪枝 + 查询表物化 + meta
                // (mask/brain/ticket) 合并 + 任务收尾同事务一次 commit
                // (原 save→persist_brain_control→mask merge→ticket merge
                // ~25 往返 → ~5)。查询表物化失败仅告警不回滚 — 与旧
                // best-effort 语义一致。
                let (brain_pieces, brain_plan_epoch, battle_log) =
                    brain_control_pieces(&result);
                let bundle = gr_probe_store::AnalysisBundleMeta {
                    analyzed_mask_v1: Some(cur_mask),
                    session_ticket: result.get("session_ticket").cloned(),
                    brain_pieces,
                    brain_plan_epoch,
                    battle_log,
                };
                let rev = store2
                    .save_analysis_bundle(&rt_sid, &result, &bundle, &wid)
                    .map_err(|e| e.to_string())?;
                // Persist page-scoped result when page_id present (parity with /analyze)
                if let Some(page) = result.get("page") {
                    if let Some(pid) = page.get("page_id").and_then(|v| v.as_str()) {
                        let prev = page
                            .get("page_rev")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(1);
                        let _ = store2.save_page_result(&rt_sid, pid, prev, page);
                    }
                }
                let _ = store2.maybe_complete_cycle_from_analysis(&rt_sid, &result);
                Ok::<_, String>((rt_sid, rev))
            })
            .await;
            match out {
                Ok(Ok((sid, rev))) => {
                    analyze_runs.fetch_add(1, Ordering::Relaxed);
                    log::debug!("auto-analyze complete sid={sid} rev={rev}");
                }
                // On failure leave lock; after lock_ms another worker can claim again.
                Ok(Err(e)) => {
                    log::debug!("auto-analyze skipped/failed sid={sid} err={e}");
                    crate::webhook_outbox::analyze_dlq_push(&sid, &e, 1);
                }
                Err(e) => {
                    log::warn!("auto-analyze join failed sid={sid} err={e}");
                    crate::webhook_outbox::analyze_dlq_push(&sid, &format!("join:{e}"), 1);
                }
            }
        }
    }
}



/// P2: persist_brain_control 的 result→片段纯提取 (worker 单事务写回路径)。
/// meta 读改写 / analysis_rev / plan_epoch 单调 / battle_log_history 滚动
/// 由 store 层 assemble_bundle_meta_patch 在锁定行上完成 — 本函数零 IO。
/// 返回 (pieces, plan_epoch 候选, 本轮 battle_log)。
fn brain_control_pieces(
    result: &Value,
) -> (serde_json::Map<String, Value>, Option<i64>, Option<Value>) {
    let mut pieces = serde_json::Map::new();
    if let Some(v) = result.get("plan_version") {
        pieces.insert("plan_version".into(), v.clone());
    }
    let pe = result
        .get("plan_epoch")
        .and_then(|v| v.as_i64())
        .or_else(|| result.pointer("/route_plan/plan_epoch").and_then(|v| v.as_i64()))
        .or_else(|| result.get("plan_version").and_then(|v| v.as_i64()));
    if let Some(v) = result.get("stop_reason") {
        pieces.insert("stop_reason".into(), v.clone());
    }
    let src = result
        .get("control_persist")
        .cloned()
        .unwrap_or_else(|| result.clone());
    for k in [
        "belief",
        "missions",
        "capability_envelope",
        "battle_log",
        "policy_band",
        "direction_priors",
        "unknown_bucket",
        "evidence_rev",
    ] {
        if let Some(v) = src.get(k) {
            pieces.insert(k.to_string(), v.clone());
        }
    }
    (pieces, pe, result.get("battle_log").cloned())
}

/// iss/22 P1b: write belief / battle_log / direction_priors / unknown_bucket into session meta.
/// P2: worker 路径已迁移 save_analysis_bundle 单事务写回 (brain_control_pieces +
/// assemble_bundle_meta_patch); 本函数保留给 HTTP /analyze 路径。
fn persist_brain_control(store: &Store, session_id: &str, result: &Value, analysis_rev: i64) {
    let mut patch = Map::new();
    patch.insert("analysis_rev".into(), json!(analysis_rev));
    if let Some(v) = result.get("plan_version") {
        patch.insert("plan_version".into(), v.clone());
    }
    // iss/72: monotone plan_epoch in session meta for stale-plan rejection.
    let pe = result
        .get("plan_epoch")
        .and_then(|v| v.as_i64())
        .or_else(|| result.pointer("/route_plan/plan_epoch").and_then(|v| v.as_i64()))
        .or_else(|| result.get("plan_version").and_then(|v| v.as_i64()))
        .unwrap_or(analysis_rev);
    let prior = store
        .merge_session_meta(session_id, &json!({}))
        .ok()
        .unwrap_or(json!({}));
    let prior_pe = prior
        .get("plan_epoch")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if pe >= prior_pe {
        patch.insert("plan_epoch".into(), json!(pe));
    }
    if let Some(v) = result.get("stop_reason") {
        patch.insert("stop_reason".into(), v.clone());
    }
    let src = result
        .get("control_persist")
        .cloned()
        .unwrap_or_else(|| result.clone());
    for k in [
        "belief",
        "missions",
        "capability_envelope",
        "battle_log",
        "policy_band",
        "direction_priors",
        "unknown_bucket",
        "evidence_rev",
    ] {
        if let Some(v) = src.get(k) {
            patch.insert(k.to_string(), v.clone());
        }
    }
    // Rolling battle history (cap 32) — read prior via merge no-op patch
    if let Some(bl) = result.get("battle_log") {
        let prior = store
            .merge_session_meta(session_id, &json!({}))
            .ok()
            .unwrap_or(json!({}));
        let mut hist = prior
            .get("battle_log_history")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        hist.push(bl.clone());
        if hist.len() > 32 {
            hist = hist.split_off(hist.len() - 32);
        }
        patch.insert("battle_log_history".into(), json!(hist));
    }
    let _ = store.merge_session_meta(session_id, &Value::Object(patch));
}

/// Parse cookie value by name from Cookie header.
fn cookie_value(headers: &HashMap<String, String>, name: &str) -> Option<String> {
    let raw = headers.get("cookie")?;
    for part in raw.split(';') {
        let p = part.trim();
        if let Some(rest) = p.strip_prefix(&format!("{name}=")) {
            let v = rest.trim();
            if !v.is_empty() {
                return Some(
                    urlencoding_decode(v).unwrap_or_else(|| v.to_string()),
                );
            }
        }
    }
    None
}

fn urlencoding_decode(s: &str) -> Option<String> {
    // Minimal percent-decode for cycle_/vt_ ids (no full crate dep).
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let h = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            if let Ok(v) = u8::from_str_radix(h, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// Fill missing GatewayEarlyBody fields from query / cookies.
///
/// Browser `navigator.sendBeacon(url)` posts an **empty** body to `/s0?session_id=…`.
/// Without this merge, POST path ignored query and returned 400
/// `"gateway early requires session_id (cycle bag) for join"`.
pub fn gateway_early_fill_from_query(
    body: &mut GatewayEarlyBody,
    headers: &HashMap<String, String>,
    q: &HashMap<String, String>,
) {
    let sid_empty = body
        .session_id
        .as_ref()
        .map(|s| s.trim().is_empty())
        .unwrap_or(true);
    if sid_empty {
        body.session_id = q
            .get("session_id")
            .cloned()
            .or_else(|| q.get("sid").cloned())
            .or_else(|| cookie_value(headers, "gr_cycle_v1"));
    }
    let vt_empty = body
        .visitor_terminal_id
        .as_ref()
        .map(|s| s.trim().is_empty())
        .unwrap_or(true);
    if vt_empty {
        body.visitor_terminal_id = q
            .get("visitor_terminal_id")
            .cloned()
            .or_else(|| q.get("vt").cloned())
            .or_else(|| cookie_value(headers, "gr_vt_v1"));
    }
    if body
        .inject_path
        .as_ref()
        .map(|s| s.trim().is_empty())
        .unwrap_or(true)
    {
        if let Some(ip) = q.get("inject_path").cloned() {
            if !ip.trim().is_empty() {
                body.inject_path = Some(ip);
            }
        }
    }
    if body
        .site_id
        .as_ref()
        .map(|s| s.trim().is_empty())
        .unwrap_or(true)
    {
        if let Some(s) = q.get("site_id").cloned() {
            if !s.trim().is_empty() {
                body.site_id = Some(s);
            }
        }
    }
    // Stamp beacon / early_kick markers when query says so (empty POST body has no fields).
    let early_q = q
        .get("early_kick")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
        || q.get("e").map(|v| v == "b8_beacon" || v == "nojs_page").unwrap_or(false);
    if early_q || body.fields.as_object().map(|o| o.is_empty()).unwrap_or(true) {
        let via = q
            .get("e")
            .cloned()
            .unwrap_or_else(|| "s0_post_query".into());
        let ua = q
            .get("ua")
            .cloned()
            .or_else(|| headers.get("user-agent").cloned())
            .unwrap_or_default();
        if let Some(obj) = body.fields.as_object_mut() {
            obj.entry("early_kick".to_string()).or_insert(json!(true));
            obj.entry("via".to_string()).or_insert(json!(via));
            if !ua.is_empty() {
                obj.entry("user_agent".to_string()).or_insert(json!(ua));
            }
            obj.entry("cookie_cycle".to_string())
                .or_insert(json!(cookie_value(headers, "gr_cycle_v1").is_some()));
            obj.entry("cookie_vt".to_string())
                .or_insert(json!(cookie_value(headers, "gr_vt_v1").is_some()));
        } else {
            body.fields = json!({
                "user_agent": ua,
                "early_kick": true,
                "via": via,
                "cookie_cycle": cookie_value(headers, "gr_cycle_v1").is_some(),
                "cookie_vt": cookie_value(headers, "gr_vt_v1").is_some(),
            });
        }
    }
}

/// GET /s0?session_id=&ua= — also honors parent-domain cookies gr_cycle_v1 / gr_vt_v1
/// so nojs pixel can join a FE-minted cycle when cookies are shared across subdomains.
pub fn gateway_early_get_compat(
    st: &AppState,
    headers: &HashMap<String, String>,
    q: &HashMap<String, String>,
) -> AppResult {
    let mut body = GatewayEarlyBody {
        session_id: None,
        visitor_terminal_id: None,
        inject_path: Some("gateway".into()),
        fields: json!({}),
        analyze: false,
        site_id: None,
        session_ticket: None,
    };
    gateway_early_fill_from_query(&mut body, headers, q);
    if body
        .fields
        .get("via")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .is_empty()
        || body.fields.get("via").and_then(|v| v.as_str()) == Some("s0_post_query")
    {
        if let Some(obj) = body.fields.as_object_mut() {
            obj.insert("via".into(), json!("s0_get_compat"));
        }
    }
    gateway_early(st, headers, body)
}

// --- Association API (iss/48) — backend key only; not probe evidence ---

#[derive(Debug, Deserialize)]
pub struct AssocObserveBody {
    pub event_id: Option<String>,
    pub idempotency_key: String,
    pub event_type: String,
    pub subject_ref: String,
    pub site_id: Option<String>,
    pub probe_context: Option<Value>,
    pub attributes: Option<Value>,
    pub occurred_at_ms: Option<i64>,
    pub operation: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct AssocAssessBody {
    pub subject_ref: String,
    pub site_id: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct AssocLabelBody {
    pub label_id: Option<String>,
    pub outcome: String,
    pub subject_ref: Option<String>,
    pub site_id: Option<String>,
    pub assessment_id: Option<String>,
    pub event_id: Option<String>,
    pub label_source: Option<String>,
    pub detail: Option<Value>,
}

fn assoc_tenant(
    st: &AppState,
    headers: &HashMap<String, String>,
    body_site: Option<&str>,
) -> Result<(String, Arc<AdminHub>), ApiError> {
    let admin = st
        .admin
        .as_ref()
        .ok_or_else(|| ApiError(503, "association_store_unavailable".into()))?
        .clone();
    // Browser clients cannot write subject bindings — require backend key when enforce on.
    // Also reject Sec-Fetch-Site: cross-site browser navigations without key.
    let key = headers
        .get("x-gr-sdk-key")
        .or_else(|| headers.get("X-Gr-Sdk-Key"))
        .map(|s| s.as_str());
    let host = headers.get("host").map(|s| s.as_str());
    let origin = headers.get("origin").map(|s| s.as_str());
    let lab = body_site
        .or_else(|| headers.get("x-gr-tenant").map(|s| s.as_str()))
        .or_else(|| headers.get("X-Gr-Tenant").map(|s| s.as_str()));
    // Reject explicit client-forged tenant when a key is present and maps elsewhere later.
    if key.is_none() {
        // Without key: only allowed when sdk_enforce off (lab)
        if admin.db.sdk_enforce_enabled() {
            return Err(ApiError(401, "sdk_key_required".into()));
        }
    }
    let tenant = crate::admin::sdk::resolve_backend_tenant(
        &admin.db,
        key,
        host,
        origin,
        lab,
    )
    .map_err(|e| ApiError(401, e))?;
    Ok((tenant, admin))
}

pub fn assoc_observe(
    st: &AppState,
    headers: &HashMap<String, String>,
    body: AssocObserveBody,
) -> AppResult {
    let (tenant, admin) = assoc_tenant(st, headers, body.site_id.as_deref())?;
    gr_probe_core::validate_subject_ref(&body.subject_ref).map_err(|e| ApiError(400, e))?;
    if let Some(ref attrs) = body.attributes {
        gr_probe_core::reject_raw_pii_fields(attrs).map_err(|e| ApiError(400, e))?;
    }
    if body.idempotency_key.trim().is_empty() {
        return Err(ApiError(400, "idempotency_key_required".into()));
    }
    let pc = body.probe_context.clone().unwrap_or(json!({}));
    let event_id = body
        .event_id
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("ev_{}", uuidish()));
    let payload = json!({
        "event_type": body.event_type,
        "attributes": body.attributes,
        "operation": body.operation,
        "probe_context": pc,
        "schema_version": gr_probe_core::ASSOC_SCHEMA_VERSION,
        "source": "backend",
    });
    let occurred = body.occurred_at_ms.unwrap_or_else(now_ms_i64);
    let out = admin
        .association
        .observe(
            &tenant,
            &body.idempotency_key,
            &event_id,
            &body.event_type,
            &body.subject_ref,
            pc.get("session_id").and_then(|v| v.as_str()),
            pc.get("visitor_terminal_id").and_then(|v| v.as_str()),
            pc.get("device_id").and_then(|v| v.as_str()),
            pc.get("identity_state").and_then(|v| v.as_str()),
            &payload,
            occurred,
        )
        .map_err(|e| ApiError(500, e))?;
    Ok(out)
}

pub fn assoc_assess(
    st: &AppState,
    headers: &HashMap<String, String>,
    body: AssocAssessBody,
) -> AppResult {
    let (tenant, admin) = assoc_tenant(st, headers, body.site_id.as_deref())?;
    gr_probe_core::validate_subject_ref(&body.subject_ref).map_err(|e| ApiError(400, e))?;
    let limit = body.limit.unwrap_or(50).clamp(1, 200);
    let events = admin
        .association
        .list_events_for_subject(&tenant, &body.subject_ref, limit)
        .map_err(|e| ApiError(500, e))?;
    let labels = admin
        .association
        .list_labels_for_subject(&tenant, &body.subject_ref, 50)
        .map_err(|e| ApiError(500, e))?;
    let mut assessment =
        gr_probe_core::assess_from_events(&tenant, &body.subject_ref, &events, &labels);
    if let Some(obj) = assessment.as_object_mut() {
        obj.insert("events_returned".into(), json!(events.len()));
    }
    Ok(assessment)
}

pub fn assoc_behavior_profile_put(
    st: &AppState,
    headers: &HashMap<String, String>,
    body: Value,
) -> AppResult {
    let site = body.get("site_id").and_then(|v| v.as_str());
    let (tenant, admin) = assoc_tenant(st, headers, site)?;
    let subject = body
        .get("subject_ref")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError(400, "subject_ref_required".into()))?;
    gr_probe_core::validate_subject_ref(subject).map_err(|e| ApiError(400, e))?;
    let session_id = body
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    let profile = if let Some(p) = body.get("profile") {
        p.clone()
    } else if let Some(fields) = body.get("fields") {
        gr_probe_core::behavior_profile_vec32(fields)
    } else {
        return Err(ApiError(400, "profile_or_fields_required".into()));
    };
    admin
        .association
        .put_behavior_profile(&tenant, subject, session_id, &profile)
        .map_err(|e| ApiError(500, e))
}

pub fn assoc_behavior_self_sim(
    st: &AppState,
    headers: &HashMap<String, String>,
    body: Value,
) -> AppResult {
    let site = body.get("site_id").and_then(|v| v.as_str());
    let (tenant, admin) = assoc_tenant(st, headers, site)?;
    let subject = body
        .get("subject_ref")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError(400, "subject_ref_required".into()))?;
    gr_probe_core::validate_subject_ref(subject).map_err(|e| ApiError(400, e))?;
    admin
        .association
        .compare_behavior_profiles(&tenant, subject)
        .map_err(|e| ApiError(500, e))
}

pub fn assoc_label(
    st: &AppState,
    headers: &HashMap<String, String>,
    body: AssocLabelBody,
) -> AppResult {
    let (tenant, admin) = assoc_tenant(st, headers, body.site_id.as_deref())?;
    if body.outcome.trim().is_empty() {
        return Err(ApiError(400, "outcome_required".into()));
    }
    let label_id = body
        .label_id
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("lbl_{}", uuidish()));
    let subject = body
        .subject_ref
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError(400, "subject_ref_required_on_label".into()))?;
    gr_probe_core::validate_subject_ref(subject).map_err(|e| ApiError(400, e))?;
    let detail = body.detail.clone().unwrap_or(json!({
        "subject_ref": subject,
    }));
    admin
        .association
        .label(
            &tenant,
            &label_id,
            subject,
            &body.outcome,
            body.assessment_id.as_deref(),
            body.event_id.as_deref(),
            body.label_source.as_deref(),
            &detail,
        )
        .map_err(|e| ApiError(500, e))
}

fn uuidish() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{t:x}")
}

fn now_ms_i64() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod content_hash_name_tests {
    use super::{
        content_hashed_filename, opaque_public_filename, strip_content_hash_filename,
    };

    #[test]
    fn roundtrip_race_and_nested() {
        let h = "a1b2c3d4e5f6";
        assert_eq!(
            content_hashed_filename("gr.race.min.js", h),
            "gr.race.a1b2c3d4e5f6.min.js"
        );
        assert_eq!(
            strip_content_hash_filename("gr.race.a1b2c3d4e5f6.min.js"),
            "gr.race.min.js"
        );
        assert_eq!(
            content_hashed_filename("collectors/registry.static.lite.min.js", h),
            "collectors/registry.static.lite.a1b2c3d4e5f6.min.js"
        );
        assert_eq!(
            strip_content_hash_filename(
                "collectors/registry.static.lite.a1b2c3d4e5f6.min.js"
            ),
            "collectors/registry.static.lite.min.js"
        );
        assert_eq!(
            content_hashed_filename("gr_seal_v2.wasm", h),
            "gr_seal_v2.a1b2c3d4e5f6.wasm"
        );
        assert_eq!(
            strip_content_hash_filename("gr_seal_v2.a1b2c3d4e5f6.wasm"),
            "gr_seal_v2.wasm"
        );
        assert_eq!(
            content_hashed_filename("nest_frame.html", h),
            "nest_frame.a1b2c3d4e5f6.html"
        );
        assert_eq!(
            strip_content_hash_filename("nest_frame.a1b2c3d4e5f6.html"),
            "nest_frame.html"
        );
    }

    #[test]
    fn opaque_public_has_no_meaningful_tokens() {
        let h = "a1b2c3d4e5f6";
        assert_eq!(opaque_public_filename("gr.race.min.js", h), "a1b2c3d4e5f6.min.js");
        assert_eq!(
            opaque_public_filename("collectors/registry.static.hard.min.js", h),
            "a1b2c3d4e5f6.min.js"
        );
        assert_eq!(opaque_public_filename("gr_seal_v2.wasm", h), "a1b2c3d4e5f6.wasm");
        assert_eq!(opaque_public_filename("nest_frame.html", h), "a1b2c3d4e5f6.html");
        let s = opaque_public_filename("gr.entry.min.js", h);
        assert!(!s.contains("gr"));
        assert!(!s.contains("entry"));
        assert!(!s.contains("race"));
    }
}

#[cfg(test)]
mod fe_identity_tests {
    use super::resolve_fe_asset_identity;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmpdir() -> std::path::PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("gr_fe_id_{n}"));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn reads_version_and_asset_gen_files() {
        let dir = tmpdir();
        std::fs::write(dir.join("VERSION"), "6.0.14
").unwrap();
        std::fs::write(dir.join("ASSET_GEN"), "deadbeef0123
").unwrap();
        let (v, g) = resolve_fe_asset_identity(&dir);
        assert_eq!(v, "6.0.14");
        assert_eq!(g, "deadbeef0123");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn derives_gen_from_race_when_missing_asset_gen() {
        let dir = tmpdir();
        std::fs::write(dir.join("VERSION"), "6.0.14").unwrap();
        let mut f = std::fs::File::create(dir.join("gr.race.min.js")).unwrap();
        f.write_all(b"window.__GR_BUILD_IMPL__=\"6.0.14\";fake-race").unwrap();
        let (v, g) = resolve_fe_asset_identity(&dir);
        assert_eq!(v, "6.0.14");
        assert_eq!(g.len(), 12);
        let (_, g2) = resolve_fe_asset_identity(&dir);
        assert_eq!(g, g2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod collect_enabled_tests {
    use super::collect_enabled_from_json;
    use serde_json::json;

    #[test]
    fn json_false_is_off() {
        assert_eq!(collect_enabled_from_json(&json!({"collect_enabled": false})), Some(false));
        assert_eq!(collect_enabled_from_json(&json!({"collect_enabled": 0})), Some(false));
    }

    #[test]
    fn json_true_is_on() {
        assert_eq!(collect_enabled_from_json(&json!({"collect_enabled": true})), Some(true));
        assert_eq!(collect_enabled_from_json(&json!({"collect_enabled": 1})), Some(true));
    }

    #[test]
    fn missing_is_none() {
        assert_eq!(collect_enabled_from_json(&json!({"site_id": "x"})), None);
    }
}

#[cfg(test)]
mod short_visit_tests {
    use super::explicit_short_visit;
    use serde_json::json;

    #[test]
    fn lifecycle_signal_suppresses_withholding_penalty() {
        assert!(explicit_short_visit(&json!({"pagehide_flush": true}), None));
        assert!(explicit_short_visit(&json!({}), Some(&json!({"stop_reason": "unload"}))));
        assert!(explicit_short_visit(&json!({"stop_reason": "early_leave"}), None));
    }

    #[test]
    fn ordinary_active_session_is_not_short_visit() {
        assert!(!explicit_short_visit(&json!({}), Some(&json!({"stop_reason": "timeout"}))));
        assert!(!explicit_short_visit(&json!({"pagehide_flush": false}), None));
    }
}

#[cfg(test)]
mod result_token_tests {
    use super::{require_result_token, require_result_token_bound};
    use std::collections::HashMap;
    use std::sync::Mutex;

    static ENV: Mutex<()> = Mutex::new(());

    fn headers_with(k: &str, v: &str) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert(k.to_string(), v.to_string());
        m
    }

    #[test]
    fn fail_closed_when_required_and_unconfigured() {
        let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("GR_REQUIRE_RESULT_TOKEN", "1");
        std::env::remove_var("GR_RESULT_TOKEN");
        std::env::remove_var("GR_SITE_RESULT_TOKENS");
        let err = require_result_token(&HashMap::new(), None).unwrap_err();
        assert_eq!(err.0, 503);
    }

    #[test]
    fn rejects_wrong_token() {
        let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("GR_REQUIRE_RESULT_TOKEN", "1");
        std::env::set_var("GR_RESULT_TOKEN", "correct-token");
        let err = require_result_token(&headers_with("authorization", "Bearer nope"), None).unwrap_err();
        assert_eq!(err.0, 401);
        assert!(require_result_token(&headers_with("authorization", "Bearer correct-token"), None).is_ok());
    }

    #[test]
    fn site_map_token_must_match_session_site() {
        let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("GR_REQUIRE_RESULT_TOKEN", "1");
        std::env::remove_var("GR_RESULT_TOKEN");
        std::env::set_var("GR_SITE_RESULT_TOKENS", "site-a:tok-a,site-b:tok-b");
        let h = headers_with("x-gr-sdk-key", "tok-a");
        assert!(require_result_token_bound(&h, None, Some("site-a")).is_ok());
        let err = require_result_token_bound(&h, None, Some("site-b")).unwrap_err();
        assert_eq!(err.0, 403);
    }

    #[test]
    fn explicit_open_lab_still_allows() {
        let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("GR_REQUIRE_RESULT_TOKEN", "0");
        assert!(require_result_token(&HashMap::new(), None).is_ok());
    }
}

#[cfg(test)]
mod cookie_capture_tests {
    use super::{
        capture_allowlisted_cookies, cookie_name_ok, cookie_pairs, meta_has_cookie_fields,
        COOKIE_CAPTURE_MAX_FIELDS, COOKIE_CAPTURE_MAX_VALUE_LEN,
    };
    use serde_json::{json, Value};

    fn allow(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_mixed_cookie_header() {
        let pairs = cookie_pairs("a=1; b = \"two\"; c=; d=e=f; x");
        assert_eq!(pairs.len(), 5);
        assert!(pairs.contains(&("a".to_string(), "1".to_string())));
        assert!(pairs.contains(&("b".to_string(), "two".to_string())));
        assert!(pairs.contains(&("d".to_string(), "e=f".to_string())));
        assert!(pairs.contains(&("x".to_string(), "".to_string())));
    }

    #[test]
    fn name_validation_rejects_illegal_and_sensitive() {
        assert!(cookie_name_ok("user_id"));
        assert!(cookie_name_ok("UGID-9"));
        assert!(cookie_name_ok("_ga_ABC123"));
        assert!(!cookie_name_ok(""));
        assert!(!cookie_name_ok("session token"));
        assert!(!cookie_name_ok("x;y"));
        assert!(!cookie_name_ok("auth_token"));
        assert!(!cookie_name_ok("password_hash"));
        assert!(!cookie_name_ok("secret"));
        assert!(!cookie_name_ok("api_key"));
        assert!(!cookie_name_ok(format!("{}_big", "n".repeat(64)).as_str()));
    }

    #[test]
    fn capture_respects_allowlist_order_and_dedup() {
        let pairs = cookie_pairs("b=2; a=1; user=u9");
        let got = capture_allowlisted_cookies(
            &allow(&["a", "b", "a", "user", "missing"]),
            &pairs,
        );
        let obj = got.as_object().expect("object");
        let keys: Vec<&String> = obj.keys().collect();
        assert_eq!(keys, vec!["a", "b", "user"]);
        assert_eq!(obj["a"], json!("1"));
        assert_eq!(obj["b"], json!("2"));
        assert_eq!(obj["user"], json!("u9"));
    }

    #[test]
    fn capture_filters_empty_values_and_truncates() {
        let long = "v".repeat(500);
        let pairs = cookie_pairs(&format!("id={long}; empty="));
        let got = capture_allowlisted_cookies(&allow(&["id", "empty"]), &pairs);
        let obj = got.as_object().expect("object");
        assert_eq!(obj.len(), 1);
        let v = obj["id"].as_str().expect("str");
        assert_eq!(v.len(), COOKIE_CAPTURE_MAX_VALUE_LEN);
        assert_eq!(v, &"v".repeat(COOKIE_CAPTURE_MAX_VALUE_LEN));
    }

    #[test]
    fn capture_caps_field_count() {
        let allowlist: Vec<String> = (0..30).map(|i| format!("k{i}")).collect();
        let pairs = cookie_pairs(
            &(0..30)
                .map(|i| format!("k{i}={i}"))
                .collect::<Vec<_>>()
                .join("; "),
        );
        let got = capture_allowlisted_cookies(&allowlist, &pairs);
        assert_eq!(got.as_object().map(|o| o.len()).unwrap_or(0), COOKIE_CAPTURE_MAX_FIELDS);
    }

    #[test]
    fn capture_skips_non_allowlisted_names() {
        let pairs = cookie_pairs("user_id=7; session=abc; extra=x");
        let got = capture_allowlisted_cookies(&allow(&["user_id"]), &pairs);
        let obj = got.as_object().expect("object");
        assert_eq!(obj.len(), 1);
        assert_eq!(obj["user_id"], json!("7"));
    }

    #[test]
    fn meta_has_cookie_fields_detection() {
        assert!(!meta_has_cookie_fields(&json!({})));
        assert!(!meta_has_cookie_fields(&json!({"cookie_fields": {}})));
        assert!(meta_has_cookie_fields(&json!({"cookie_fields": {"user_id": "7"}})));
        assert!(!meta_has_cookie_fields(&json!({"cookie_fields": null})));
        assert!(!meta_has_cookie_fields(&Value::Null));
    }
}

#[cfg(test)]
mod ip_provider_tests {
    use super::{
        fetch_ip_provider, integration_url_allowed, map_ip_provider_response, provider_ip_allowed,
        provider_ip_target,
    };
    use serde_json::{json, Value};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::Mutex;
    use std::thread;
    use std::time::Duration;

    static ENV: Mutex<()> = Mutex::new(());

    fn serve_responses(responses: Vec<(u16, &'static str)>, delay: Option<Duration>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture");
        let addr = listener.local_addr().expect("fixture address");
        thread::spawn(move || {
            for (status, body) in responses {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let mut request = [0u8; 2048];
                let _ = stream.read(&mut request);
                if let Some(wait) = delay {
                    thread::sleep(wait);
                }
                let reason = if status < 400 { "OK" } else { "Error" };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        format!("http://{addr}")
    }

    fn custom_policy(url: &str) -> Value {
        json!({
            "ip_enrichment": {
                "enabled": true,
                "provider": "custom_http",
                "providers": {
                    "custom_http": {
                        "base_url": url,
                        "header_auth": "X-Test-Auth: fixture-auth-value"
                    }
                }
            }
        })
    }

    #[test]
    fn rejects_non_public_targets_and_urls() {
        // 并行测试隔离: 同模块其它用例会临时置 GR_ALLOW_LAB_INTEGRATION_HTTP=1
        // (持有整个用例时长, 含秒级超时用例), 本用例必须持 ENV 锁并防御性清除,
        // 否则读到他例的开关 → 私网放行 → 断言翻转(与运行顺序/线程调度相关)。
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("GR_ALLOW_LAB_INTEGRATION_HTTP");
        assert!(!provider_ip_allowed("127.0.0.1"));
        assert!(!provider_ip_allowed("10.0.0.2"));
        assert!(!provider_ip_allowed("::1"));
        assert!(provider_ip_allowed("8.8.8.8"));
        assert_eq!(
            integration_url_allowed("file:///etc/passwd").unwrap_err(),
            "unsafe_url"
        );
        assert_eq!(
            integration_url_allowed("http://127.0.0.1:8080").unwrap_err(),
            "private_target_blocked"
        );
    }

    #[test]
    fn maps_provider_payload_without_returning_auth() {
        let body = json!({
            "asn": "AS15169",
            "country": "US",
            "org": "Google LLC",
            "is_datacenter": true,
            "token": "should-not-be-projected"
        });
        let mapped = map_ip_provider_response("custom_http", "8.8.8.8", &body);
        assert_eq!(mapped["status"], "ok");
        assert_eq!(mapped["asn"], "AS15169");
        assert_eq!(mapped["country"], "US");
        assert_eq!(mapped["datacenter"], true);
        assert!(mapped.get("token").is_none());
        assert!(!mapped.to_string().contains("fixture-auth-value"));
    }

    #[test]
    fn retries_transient_upstream_and_maps_success() {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("GR_ALLOW_LAB_INTEGRATION_HTTP", "1");
        let url = serve_responses(
            vec![
                (503, r#"{"error":"temporary"}"#),
                (200, r#"{"asn":"AS64500","countryCode":"ZZ","organization":"Fixture Net"}"#),
            ],
            None,
        );
        let result = fetch_ip_provider(&custom_policy(&url), "8.8.8.8");
        assert_eq!(result["status"], "ok");
        assert_eq!(result["asn"], "AS64500");
        assert_eq!(result["country"], "ZZ");
        std::env::remove_var("GR_ALLOW_LAB_INTEGRATION_HTTP");
    }

    #[test]
    fn reports_timeout_after_retry_budget() {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("GR_ALLOW_LAB_INTEGRATION_HTTP", "1");
        let url = serve_responses(
            vec![(200, r#"{"asn":"AS64500"}"#), (200, r#"{"asn":"AS64500"}"#)],
            Some(Duration::from_millis(1400)),
        );
        let result = fetch_ip_provider(&custom_policy(&url), "8.8.8.8");
        assert_eq!(result["status"], "error");
        assert_eq!(result["error"], "timeout");
        std::env::remove_var("GR_ALLOW_LAB_INTEGRATION_HTTP");
    }

    /// Panel QA P1a (2026-09-08): stored visitor IPs are privacy-masked CIDRs
    /// (`a.b.c.0/24`); the enrichment pipeline must query the network address,
    /// not reject the masked literal as non_public.
    #[test]
    fn masked_cidr_resolves_to_network_address() {
        assert_eq!(provider_ip_target("183.192.38.0/24").as_ref(), "183.192.38.0");
        assert_eq!(
            provider_ip_target("2001:db8:abcd::/48").as_ref(),
            "2001:db8:abcd::"
        );
        // Bare IPs / non-IP strings pass through unchanged.
        assert_eq!(provider_ip_target("8.8.8.8").as_ref(), "8.8.8.8");
        assert_eq!(provider_ip_target("unknown").as_ref(), "unknown");
        // A masked PRIVATE subnet still skips (correct non_public verdict).
        assert!(!provider_ip_allowed(provider_ip_target("10.0.0.0/24").as_ref()));
    }

    #[test]
    fn masked_ip_reaches_provider_end_to_end() {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("GR_ALLOW_LAB_INTEGRATION_HTTP", "1");
        let url = serve_responses(
            vec![(
                200,
                r#"{"asn":"AS24400","countryCode":"CN","organization":"Shanghai Mobile"}"#,
            )],
            None,
        );
        let result = fetch_ip_provider(&custom_policy(&url), "183.192.38.0/24");
        assert_eq!(
            result["status"], "ok",
            "masked /24 must reach the provider: {result}"
        );
        assert_eq!(result["asn"], "AS24400");
        assert_eq!(result["country"], "CN");
        std::env::remove_var("GR_ALLOW_LAB_INTEGRATION_HTTP");
    }

    #[test]
    fn uses_post_for_custom_provider() {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("GR_ALLOW_LAB_INTEGRATION_HTTP", "1");
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture");
        let addr = listener.local_addr().expect("fixture address");
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept fixture");
            let mut request = [0u8; 2048];
            let n = stream.read(&mut request).expect("read fixture request");
            String::from_utf8_lossy(&request[..n]).into_owned()
        });
        let url = format!("http://{addr}");
        // The server closes after reading, so the client gets a transport error
        // after the request is fully emitted. The request itself is the assertion.
        let _ = fetch_ip_provider(&custom_policy(&url), "8.8.8.8");
        let request = worker.join().expect("fixture worker");
        assert!(request.starts_with("POST / HTTP/1.1"));
        assert!(request.contains(r#""ip":"8.8.8.8""#));
        assert!(request
            .to_ascii_lowercase()
            .contains("x-test-auth: fixture-auth-value"));
        std::env::remove_var("GR_ALLOW_LAB_INTEGRATION_HTTP");
    }
}

