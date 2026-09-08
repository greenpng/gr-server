//! Persistence for green-v5: SQLite (single-node) or PostgreSQL (multi-worker).

mod evidence_merge;
mod hot_cold;
mod pg;
pub mod probe_state;
mod runtime_cfg;
mod write_amp;

pub use hot_cold::{
    cold_doc_from_batch, cold_promote_window_ms, cold_ttl_ms, compact_probe_payload,
    compress_json_payload, decompress_json_payload, hot_idle_ms, now_ms as hot_now_ms,
    timeout_matrix_json, HotProbeCache, HotProbeEntry, DEFAULT_COLD_PROMOTE_WINDOW_MS,
    DEFAULT_COLD_TTL_MS, DEFAULT_HOT_IDLE_MS,
};
pub use runtime_cfg::{
    analyze_idle_upload_ms, cfg_from_stored, clamp_global, cold_purge_interval_ms,
    complete_on_commercial_silicon, config_version, cycle_cool_ms, cycle_cool_ms_for_site,
    cycle_incomplete_ms, cycle_incomplete_ms_for_site, effective_for_site, fe_retry_policy_json,
    field_help_json, get_runtime_cfg, global_to_json, parse_global_patch, parse_site_override,
    return_identity_idle_ms, rpa_idle_analyze_ms, session_hard_max_ms_rt, session_inactivity_ms_rt,
    set_runtime_cfg, sites_to_json, RuntimeCfg, SiteOverride,
};
pub use write_amp::{
    analysis_history_keep, analysis_storage_mode, decode_analysis_result_json,
    encode_analysis_result_json, session_touch_min_interval_ms, slim_analysis_result_for_storage,
};
pub use probe_state::{
    derive_server_snapshot, fe_should_halt_from_response, halt_from_analysis_flags, reconcile,
    reconcile_to_json, result_closes_cycle, snapshot_to_json, BusinessState, ClientCycleView,
    Correction, CycleSnapshot, ReconcileResult, ServerCycleFacts,
};

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

pub use pg::PgStore;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("session expired: {0}")]
    SessionExpired(String),
    #[error("{0}")]
    Msg(String),
}

/// Session retention policy (legacy names — identity window now uses cycle TTL below).
pub const SESSION_INACTIVITY_MS: i64 = 30 * 60 * 1000; // 30 minutes (legacy; not primary cycle clock)
/// Legacy hard max; superseded by [`CYCLE_INCOMPLETE_MS`] for probe cycles.
pub const SESSION_HARD_MAX_MS: i64 = 24 * 60 * 60 * 1000; // 24 hours
/// After identity probe **complete** on a vt: no re-probe / re-analyze for this long
/// **within the same product_version**. Version bump forces a new probe cycle even if
/// cool wall-clock has not expired (see open_cycle cool gate).
pub const CYCLE_COOL_MS: i64 = 24 * 60 * 60 * 1000; // 24 hours

/// Cool is valid only when time remains **and** last complete was for `current` product version.
///
/// - No current version in open meta → legacy time-only cool (service should always inject).
/// - Current set but VT has no `product_version_last` → **re-probe** (bind version once).
/// - Both set and equal → cool; mismatch → re-probe.
///
/// Authority is **product_version completeness**, not sticky `cycle_id` cookies:
/// a cycle bag completed under v-a must not skip identity under v-b.
pub fn cool_valid_for_product_version(
    cool_until_ms: i64,
    now_ms: i64,
    product_version_last: &str,
    current_product_version: &str,
) -> bool {
    if cool_until_ms <= now_ms {
        return false;
    }
    let cur = current_product_version.trim();
    let last = product_version_last.trim();
    if cur.is_empty() {
        // Legacy / tests without product_version on open.
        return true;
    }
    if last.is_empty() {
        return false;
    }
    cur == last
}

/// Why identity cool / cycle resume is rejected (server → FE corrections).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReprobeReason {
    ProductVersionMismatch,
    MissingProductVersionLast,
    CycleVersionMismatch,
    CycleCompleteSuperseded,
    CycleIncompleteExpired,
}

impl ReprobeReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProductVersionMismatch => "product_version_mismatch",
            Self::MissingProductVersionLast => "missing_product_version_last",
            Self::CycleVersionMismatch => "cycle_version_mismatch",
            Self::CycleCompleteSuperseded => "cycle_complete_superseded",
            Self::CycleIncompleteExpired => "cycle_incomplete_expired",
        }
    }
}

/// Read product_version from open/complete meta (FE or service-injected).
pub fn product_version_from_meta(meta: &Value) -> String {
    meta.get("product_version")
        .or_else(|| meta.get("version"))
        .or_else(|| meta.get("sdk_v"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_default()
}

/// Whether an existing cycle row may be **resumed** under `current` product version.
///
/// - No current version → legacy allow.
/// - Cycle missing product_version while current is set → **reject** (re-bind under new VERSION).
/// - Both set and equal → allow; mismatch → reject (re-probe for new version).
pub fn cycle_compatible_with_product_version(cycle_meta: &Value, current_product_version: &str) -> bool {
    let cur = current_product_version.trim();
    if cur.is_empty() {
        return true;
    }
    let cyc = product_version_from_meta(cycle_meta);
    if cyc.is_empty() {
        return false;
    }
    cyc == cur
}

/// Cool wall-clock remains but not valid for current product version → reason.
pub fn cool_invalid_reason(
    cool_until_ms: i64,
    now_ms: i64,
    product_version_last: &str,
    current_product_version: &str,
) -> Option<ReprobeReason> {
    if cool_until_ms <= now_ms {
        return None;
    }
    if cool_valid_for_product_version(
        cool_until_ms,
        now_ms,
        product_version_last,
        current_product_version,
    ) {
        return None;
    }
    let cur = current_product_version.trim();
    if cur.is_empty() {
        return None;
    }
    if product_version_last.trim().is_empty() {
        Some(ReprobeReason::MissingProductVersionLast)
    } else {
        Some(ReprobeReason::ProductVersionMismatch)
    }
}
/// Incomplete cycle retention: if not complete within this age, purge cycle evidence.
pub const CYCLE_INCOMPLETE_MS: i64 = 72 * 60 * 60 * 1000; // 72 hours
/// Default debounce before auto-analyze after batch upsert (W0).
/// 50ms merges static-wave multi-batch uploads without feeling laggy.
pub const ANALYZE_DEBOUNCE_MS: i64 = 50;

/// How to merge a new analyze arm with an existing pending job.
///
/// - [`AnalyzeDueMerge::PullEarlier`]: milestone / coverage / no-result ceiling —
///   only move due **sooner** (`MIN`).
/// - [`AnalyzeDueMerge::IdleReset`]: after a new probe upload, reset the idle quiet
///   window to `now+debounce`, but **do not** delay an imminent/sooner arm
///   (keeps milestone PullEarlier dues).
/// - [`AnalyzeDueMerge::Replace`]: set due exactly to `now+debounce` (explicit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalyzeDueMerge {
    PullEarlier,
    IdleReset,
    Replace,
}

impl AnalyzeDueMerge {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PullEarlier => "pull_earlier",
            Self::IdleReset => "idle_reset",
            Self::Replace => "replace",
        }
    }
}

/// Guard: idle re-arm will not push past a due already within this many ms (or past).
pub const ANALYZE_IDLE_IMMINENT_MS: i64 = 10_000;
/// Legacy short coalesce — **not** used for normal ingest arms (brain owns schedule).
/// Brain idle / no-result arms use gr_probe_core::analyze_schedule constants via service.
pub const BRAIN_ANALYZE_IDLE_MS: i64 = 60_000;
pub const BRAIN_ANALYZE_NO_RESULT_MS: i64 = 180_000;
/// How long a worker may hold a session analyze lock.
pub const ANALYZE_LOCK_MS: i64 = 15_000;
/// Empty-queue poll backoff (analyze workers). Prod 178: 20ms fixed poll × 25
/// claimers ≈ 1k–5k TPS of empty claim SQL + fsync under synchronous_commit=on.
pub const ANALYZE_IDLE_POLL_MS_MIN: u64 = 40;
pub const ANALYZE_IDLE_POLL_MS_MAX: u64 = 500;
/// Claim batch size per poll when work is available.
pub const ANALYZE_CLAIM_BATCH: usize = 4;

/// Env override for idle poll min/max (ms).
pub fn analyze_idle_poll_ms_range() -> (u64, u64) {
    let min = gr_abi::env::get("ANALYZE_IDLE_POLL_MS_MIN")
        .and_then(|s| s.parse().ok())
        .unwrap_or(ANALYZE_IDLE_POLL_MS_MIN)
        .clamp(10, 5_000);
    let max = gr_abi::env::get("ANALYZE_IDLE_POLL_MS_MAX")
        .and_then(|s| s.parse().ok())
        .unwrap_or(ANALYZE_IDLE_POLL_MS_MAX)
        .clamp(min, 30_000);
    (min, max)
}
/// Max pending analyze jobs before backpressure (G-P0-6). Override via `GR_ANALYZE_QUEUE_MAX`.
pub fn analyze_queue_max() -> i64 {
    gr_abi::env::get("ANALYZE_QUEUE_MAX")
        .and_then(|s| s.parse().ok())
        .unwrap_or(512)
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Report-friendly scalars from full analysis JSON (ops / dashboards; avoid TOAST scans).
///
/// Keep these columns queryable on `analysis_results` + `analysis_latest` so
/// collision/tier/device filters never need `result_json` full-table detoast.
#[derive(Debug, Clone, Default)]
pub struct AnalysisReportScalars {
    pub real_band: Option<String>,
    pub device_id: Option<String>,
    pub bot_verdict: Option<String>,
    /// Internal evaluate recommended_action.action at analyze time (not strategy-projected).
    pub product_action: Option<String>,
    pub device_confidence: Option<f64>,
    pub client_ip: Option<String>,
    pub device_tier: Option<String>,
    pub collision_risk: Option<bool>,
    pub product_version: Option<String>,
    pub digest_path: Option<String>,
    pub residual_entropy_ok: Option<bool>,
    // Extended facets for fast multi-filter queries (P1)
    pub association_level: Option<String>,
    pub authenticity_band: Option<String>,
    pub site_id: Option<String>,
    pub inject_path: Option<String>,
    pub form_class: Option<String>,
    pub os_family: Option<String>,
    pub platform: Option<String>,
    pub os_score: Option<f64>,
    pub br_score: Option<f64>,
    pub rpa_score: Option<f64>,
    pub os_status: Option<String>,
    pub br_status: Option<String>,
    pub rpa_status: Option<String>,
    pub country: Option<String>,
    pub asn: Option<String>,
    pub residual_algo: Option<String>,
    pub has_webrtc_host: Option<bool>,
    /// Multi-source mint gate (ops dashboard scalars)
    pub mint_residual_ok: Option<bool>,
    pub mint_host_ok: Option<bool>,
    pub mint_silicon_ok: Option<bool>,
    pub mint_conflict_pressure: Option<f64>,
    pub mint_single_source_pressure: Option<f64>,
    pub mint_ok_keys_n: Option<i32>,
    pub mint_conf_only_keys_n: Option<i32>,
    pub mint_conflict_keys_n: Option<i32>,
    /// Compact JSON summary of multi_source_mint_gate (no full decisions map).
    pub mint_gate_summary: Option<String>,
    /// Binder keys for device_index reverse lookup (wg:/au:/lan:…)
    pub binder_keys: Vec<String>,
}

fn opt_str_ptr(result: &Value, paths: &[&str]) -> Option<String> {
    for p in paths {
        if let Some(s) = result.pointer(p).and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            return Some(s.to_string());
        }
    }
    None
}

/// Coarse IP class for analysis denorm (no MMDB; gateway path owns ASN/country).
fn coarse_network_class(ip: &str) -> Option<String> {
    let ip = ip.trim();
    if ip.is_empty() {
        return None;
    }
    if ip == "127.0.0.1" || ip == "::1" || ip.starts_with("127.") {
        return Some("loopback".into());
    }
    if let Ok(addr) = ip.parse::<std::net::IpAddr>() {
        match addr {
            std::net::IpAddr::V4(v) => {
                if v.is_private() || v.is_link_local() {
                    return Some("private".into());
                }
            }
            std::net::IpAddr::V6(v) => {
                let o = v.octets();
                if (o[0] & 0xfe) == 0xfc || v.is_unicast_link_local() {
                    return Some("private".into());
                }
            }
        }
        return Some("public".into());
    }
    None
}

fn opt_f64_ptr(result: &Value, paths: &[&str]) -> Option<f64> {
    for p in paths {
        if let Some(n) = result.pointer(p).and_then(|v| v.as_f64()) {
            return Some(n);
        }
    }
    None
}

/// Extract report scalars from evaluate result JSON.
pub fn analysis_report_scalars(result: &Value) -> AnalysisReportScalars {
    let real_band = result
        .get("real_band")
        .or_else(|| result.pointer("/product/real_band"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    // Prefer published id; when identity governance withholds (JSON null), fall back to
    // stable candidate / dv0 segment so ops scalars are not empty while digests exist (iss/75).
    // Note: `or_else` does not run on Some(Null) — must filter empty/null at each step.
    let device_id = [
        result.pointer("/product/device_id"),
        result.pointer("/device/device_id"),
        result.get("device_id"),
        result.pointer("/device/device_id_stable_candidate"),
        result.pointer("/device/device_id_segments/dv0"),
        result.pointer("/device/device_id_match"),
    ]
    .into_iter()
    .flatten()
    .find_map(|v| v.as_str().filter(|s| !s.is_empty()).map(|s| s.to_string()));
    let bot_verdict = result
        .pointer("/bot/verdict")
        .or_else(|| result.get("bot_verdict"))
        .or_else(|| result.pointer("/product/bot_verdict"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    // Analyze-time action (pre product_public strategy projection). Used for historical charts.
    let product_action = result
        .pointer("/recommended_action/action")
        .or_else(|| result.pointer("/product/recommended_action/action"))
        .or_else(|| result.pointer("/recommended_action"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let device_confidence = result
        .pointer("/product/device_confidence")
        .or_else(|| result.pointer("/device/device_confidence"))
        .and_then(|v| v.as_f64());
    let client_ip = result
        .get("server_client_ip")
        .or_else(|| result.pointer("/device/trust/materials/server_client_ip"))
        .or_else(|| result.pointer("/soft/member/server_client_ip"))
        .or_else(|| result.pointer("/gateway_fields/server_client_ip"))
        .or_else(|| result.pointer("/fields/server_client_ip"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let device_tier = result
        .pointer("/device/device_tier")
        .or_else(|| result.pointer("/product/device_tier"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| {
            device_id.as_ref().and_then(|id| {
                if is_commercial_device_id_str(id) {
                    if id.starts_with("dv0-")
                        || id.starts_with("dv4-")
                        || id.starts_with("dv5-")
                        || id.starts_with("dv6-")
                    {
                        Some("multi".into())
                    } else if id.starts_with("dh-") || id.starts_with("dh_") {
                        Some("dh".into())
                    } else if id.starts_with("dv-") || id.starts_with("dv_") {
                        Some("dv".into())
                    } else if id.starts_with("dg-") || id.starts_with("dg_") {
                        Some("dg".into())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
        });
    let collision_risk = result
        .pointer("/device/collision_risk")
        .or_else(|| result.pointer("/product/collision_risk"))
        .and_then(|v| v.as_bool());
    let product_version = result
        .get("product_version")
        .or_else(|| result.pointer("/product/product_version"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let digest_path = result
        .pointer("/device/digest_path")
        .or_else(|| result.pointer("/device/trust/digest_path"))
        .or_else(|| result.pointer("/product/digest_path"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let mut residual_entropy_ok = result
        .pointer("/device/webgl_residual_entropy_ok")
        .or_else(|| result.pointer("/device/trust/webgl_residual_entropy_ok"))
        .or_else(|| result.pointer("/product/webgl_residual_entropy_ok"))
        .and_then(|v| v.as_bool());
    // Heal false-negative after slim storage: real_curves + commercial dh_ + material digests.
    if residual_entropy_ok != Some(true) {
        let dig = result
            .pointer("/device/digest_path")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let did = result
            .pointer("/device/device_id")
            .or_else(|| result.pointer("/product/device_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let has_mat = result
            .pointer("/device/trust/materials/hw_webgl_stable")
            .or_else(|| result.pointer("/device/trust/materials/hw_audio_stable"))
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty());
        if dig.contains("real_curves")
            && is_commercial_device_id_str(did)
            && has_mat
        {
            residual_entropy_ok = Some(true);
        }
    }

    let association_level = opt_str_ptr(
        result,
        &[
            "/product/association_level",
            "/device/association_level",
        ],
    );
    let authenticity_band = opt_str_ptr(
        result,
        &[
            "/product/authenticity_band",
            "/device/authenticity_band",
            "/product/real_band",
        ],
    )
    .or_else(|| real_band.clone());
    let site_id = opt_str_ptr(
        result,
        &["/meta/site_id", "/session_meta/site_id", "/fields/site_id", "/product/site_id"],
    );
    let inject_path = opt_str_ptr(
        result,
        &[
            "/meta/inject_path",
            "/session_meta/inject_path",
            "/fields/inject_path",
        ],
    );
    let form_class = opt_str_ptr(
        result,
        &[
            "/fields/form_class",
            "/device/trust/materials/form_class",
            "/product/form_class",
        ],
    );
    let os_family = opt_str_ptr(
        result,
        &[
            "/fields/os_family",
            "/device/trust/materials/os_family",
            "/product/os/family",
        ],
    );
    let platform = opt_str_ptr(result, &["/fields/platform", "/product/platform"]);
    let os_score = opt_f64_ptr(result, &["/product/os/score", "/product/os/confidence"]);
    let br_score = opt_f64_ptr(result, &["/product/br/score", "/product/br/confidence"]);
    let rpa_score = opt_f64_ptr(result, &["/product/rpa/score", "/product/rpa/confidence"]);
    let os_status = opt_str_ptr(result, &["/product/os/status", "/product/os/state"]);
    let br_status = opt_str_ptr(result, &["/product/br/status", "/product/br/state"]);
    let rpa_status = opt_str_ptr(result, &["/product/rpa/status", "/product/rpa/state"]);
    let country = opt_str_ptr(
        result,
        &[
            "/fields/server_country",
            "/gateway_fields/server_country",
            "/gateway_fields/cf_ipcountry",
            "/fields/cf_ipcountry",
            "/cf_fields/cf_ipcountry",
            "/product/country",
            "/product/server_country",
        ],
    );
    let asn = opt_str_ptr(
        result,
        &[
            "/fields/server_asn",
            "/gateway_fields/server_asn",
            "/product/server_asn",
            "/product/asn",
            "/fields/asn",
        ],
    );
    let residual_algo = opt_str_ptr(
        result,
        &["/fields/residual_algo", "/device/residual_algo", "/product/residual_algo"],
    );
    let has_webrtc_host = result
        .pointer("/fields/webrtc_host_ip_hash")
        .or_else(|| result.pointer("/device/trust/materials/webrtc_host_ip_hash"))
        .and_then(|v| v.as_str())
        .map(|s| !s.is_empty());

    // multi_source_mint_gate summary for analysis_latest ops
    let gate = result
        .pointer("/device/multi_source_mint_gate")
        .or_else(|| result.pointer("/device/link_or_mint/multi_source_mint_gate"))
        .or_else(|| result.pointer("/product/multi_source_mint_gate"));
    let mint_residual_ok = gate
        .and_then(|g| g.get("residual_mint_ok"))
        .and_then(|v| v.as_bool());
    let mint_host_ok = gate
        .and_then(|g| g.get("host_mint_ok"))
        .and_then(|v| v.as_bool());
    let mint_silicon_ok = gate
        .and_then(|g| g.get("silicon_mint_ok"))
        .and_then(|v| v.as_bool());
    let mint_conflict_pressure = gate
        .and_then(|g| g.get("conflict_pressure"))
        .and_then(|v| v.as_f64());
    let mint_single_source_pressure = gate
        .and_then(|g| g.get("single_source_pressure"))
        .and_then(|v| v.as_f64());
    let mint_ok_keys_n = gate
        .and_then(|g| g.get("mint_ok_keys"))
        .and_then(|v| v.as_array())
        .map(|a| a.len() as i32);
    let mint_conf_only_keys_n = gate
        .and_then(|g| g.get("conf_only_keys"))
        .and_then(|v| v.as_array())
        .map(|a| a.len() as i32);
    let mint_conflict_keys_n = gate
        .and_then(|g| g.get("conflict_keys"))
        .and_then(|v| v.as_array())
        .map(|a| a.len() as i32);
    let mint_gate_summary = gate.map(|g| {
        serde_json::json!({
            "algo": g.get("algo"),
            "residual_mint_ok": g.get("residual_mint_ok"),
            "host_mint_ok": g.get("host_mint_ok"),
            "silicon_mint_ok": g.get("silicon_mint_ok"),
            "conflict_pressure": g.get("conflict_pressure"),
            "single_source_pressure": g.get("single_source_pressure"),
            "mint_ok_keys": g.get("mint_ok_keys"),
            "conf_only_keys": g.get("conf_only_keys"),
            "conflict_keys": g.get("conflict_keys"),
            "source_conflicts_n": g.get("source_conflicts_n"),
        })
        .to_string()
    });

    let mut binder_keys: Vec<String> = Vec::new();
    if let Some(id) = device_id.as_ref() {
        // Self key always present for reverse lookups of the commercial id body
        if id.len() > 3 {
            binder_keys.push(format!("id:{}", id));
        }
    }
    for (prefix, paths) in [
        (
            "wg:",
            &[
                "/device/trust/materials/hw_webgl_stable",
                "/fields/hw_webgl_stable",
            ][..],
        ),
        (
            "au:",
            &[
                "/device/trust/materials/hw_audio_stable",
                "/fields/hw_audio_stable",
            ][..],
        ),
        (
            "lan:",
            &[
                "/fields/webrtc_host_ip_hash",
                "/device/trust/materials/webrtc_host_ip_hash",
            ][..],
        ),
    ] {
        if let Some(s) = opt_str_ptr(result, paths) {
            binder_keys.push(format!("{prefix}{s}"));
        }
    }

    AnalysisReportScalars {
        real_band,
        device_id,
        bot_verdict,
        product_action,
        device_confidence,
        client_ip,
        device_tier,
        collision_risk,
        product_version,
        digest_path,
        residual_entropy_ok,
        association_level,
        authenticity_band,
        site_id,
        inject_path,
        form_class,
        os_family,
        platform,
        os_score,
        br_score,
        rpa_score,
        os_status,
        br_status,
        rpa_status,
        country,
        asn,
        residual_algo,
        has_webrtc_host,
        mint_residual_ok,
        mint_host_ok,
        mint_silicon_ok,
        mint_conflict_pressure,
        mint_single_source_pressure,
        mint_ok_keys_n,
        mint_conf_only_keys_n,
        mint_conflict_keys_n,
        mint_gate_summary,
        binder_keys,
    }
}

pub(crate) fn hex_now() -> String {
    let mut buf = [0u8; 16];
    if getrandom::getrandom(&mut buf).is_err() {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(now_ms().to_le_bytes());
        h.update(std::process::id().to_le_bytes());
        buf.copy_from_slice(&h.finalize()[..16]);
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// iss/opus5 06-P1-5: UTC billing bucket "YYYY-MM" for the monthly session
/// counter (civil-from-days, Howard Hinnant's algorithm — no chrono dep).
pub(crate) fn month_key_utc() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { yoe + era * 400 + 1 } else { yoe + era * 400 };
    format!("{y:04}-{m:02}")
}

/// Silicon materials present on evaluate result (hard commercial path).
/// Cycle must not close / cool without this — otherwise FE stops before B10 lands.
///
/// **Primary B10 batch proof is required** (`b10_present` / real_curves digest).
/// Residual field fragments alone must NOT count — they caused thin_surface CIF
/// + coverage_complete without `B10_hw_curves` (lab false complete).
pub fn result_has_silicon(result: &Value) -> bool {
    if result
        .get("b10_present")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return true;
    }
    let dig = result
        .pointer("/device/digest_path")
        .or_else(|| result.pointer("/product/digest_path"))
        .or_else(|| result.get("digest_path"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    // real_curves digest is produced after primary residual land.
    dig.contains("real_curves")
}

/// True for multi-segment commercial ids (`dv0-|dv4-|dv5-|dv6-`) and legacy exclusive prefixes.
fn is_commercial_device_id_str(id: &str) -> bool {
    id.starts_with("dv0-")
        || id.starts_with("dv4-")
        || id.starts_with("dv5-")
        || id.starts_with("dv6-")
        || id.starts_with("dh-")
        || id.starts_with("dh_")
        || id.starts_with("dv-")
        || id.starts_with("dv_")
        || id.starts_with("dg-")
        || id.starts_with("dg_")
}

/// Milestone: commercial-grade **device_id materials** present (multi-segment or legacy + silicon).
///
/// This is **NOT** cycle-final and must **NOT** halt soft/mid/brain packs.
/// Product scores continue to improve until brain schedule completes
/// (`coverage_complete` + stop, or `analysis_terminal`).
///
/// Never true for thin empty_anchor / gateway_only / bare ticket with no silicon.
pub fn result_commercial_identity_final(result: &Value) -> bool {
    if !result_has_silicon(result) {
        return false;
    }
    let did = result
        .pointer("/product/device_id")
        .or_else(|| result.pointer("/device/device_id"))
        .or_else(|| result.get("device_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let tier = result
        .pointer("/product/device_tier")
        .or_else(|| result.pointer("/device/device_tier"))
        .or_else(|| result.get("device_tier"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let multi_flag = result
        .pointer("/product/multi_segment")
        .or_else(|| result.pointer("/device/multi_segment"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let commercial = is_commercial_device_id_str(did)
        || multi_flag
        || tier == "dh"
        || tier == "dv"
        || tier == "multi";
    if !commercial {
        return false;
    }
    let dig = result
        .pointer("/device/digest_path")
        .or_else(|| result.pointer("/product/digest_path"))
        .or_else(|| result.get("digest_path"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    // real_curves is the primary commercial hard path; b10_present is the batch proof.
    if dig.contains("real_curves") {
        return true;
    }
    result
        .get("b10_present")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// True when evaluate result should close the probe cycle (full schedule done → cool).
///
/// **Maximize probe (v5.8.53+)**: thin mint alone never closes.
/// **Hard final (178 fix)**: commercial silicon + B10x satisfied (or explicit terminal).
///
/// Final when **silicon present** and any of:
/// - `analysis_terminal`
/// - `brain_schedule_final`
/// - `stop_probe ∧ coverage_complete`
/// - `commercial_identity_final ∧ ¬route_plan.b10x_must_land` (hard identity ready)
pub fn analysis_completes_cycle(result: &Value) -> bool {
    // Never close cycle without primary residual proof (B10_hw_curves).
    if !result_has_silicon(result) {
        return false;
    }
    let b10_present = result
        .get("b10_present")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || result
            .pointer("/cycle_probe_status/has_b10")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        || result
            .pointer("/identity_coverage/has_b10")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let dig = result
        .pointer("/device/digest_path")
        .or_else(|| result.pointer("/product/digest_path"))
        .or_else(|| result.get("digest_path"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !b10_present && !dig.contains("real_curves") {
        return false;
    }
    if result
        .get("analysis_terminal")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return true;
    }
    if result
        .get("brain_schedule_final")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return true;
    }
    let stop = result
        .pointer("/route_plan/stop_probe")
        .and_then(|v| v.as_bool())
        .or_else(|| result.get("stop_probe").and_then(|v| v.as_bool()))
        .unwrap_or(false);
    let cov = result
        .pointer("/coverage/coverage_complete")
        .and_then(|v| v.as_bool())
        .or_else(|| {
            result
                .pointer("/brain/coverage/coverage_complete")
                .and_then(|v| v.as_bool())
        })
        .unwrap_or(false);
    if stop && cov {
        return true;
    }
    // Hard commercial path: silicon already required above; B10x must not still be required.
    if !complete_on_commercial_silicon() {
        return false;
    }
    let commercial = result
        .get("commercial_identity_final")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || result_commercial_identity_final(result);
    let b10x_must = result
        .pointer("/route_plan/b10x_must_land")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    commercial && !b10x_must
}

pub(crate) fn cycle_status_from_meta(meta: &Value) -> &str {
    meta.get("cycle_status")
        .and_then(|v| v.as_str())
        .unwrap_or("active")
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS sessions (
  session_id TEXT PRIMARY KEY,
  visitor_terminal_id TEXT,
  created_ms INTEGER NOT NULL,
  updated_ms INTEGER NOT NULL,
  meta_json TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS probe_batches (
  session_id TEXT NOT NULL,
  batch_id TEXT NOT NULL,
  source TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  created_ms INTEGER NOT NULL,
  -- iss/72 material identity (logical key includes generation; PK stays session/batch/source)
  material_generation INTEGER NOT NULL DEFAULT 1,
  material_hash TEXT,
  capture_id TEXT,
  PRIMARY KEY (session_id, batch_id, source),
  FOREIGN KEY (session_id) REFERENCES sessions(session_id)
);

CREATE TABLE IF NOT EXISTS observation_events (
  observation_id TEXT PRIMARY KEY,
  tenant_id TEXT NOT NULL DEFAULT '',
  session_id TEXT NOT NULL,
  batch_id TEXT NOT NULL,
  source TEXT NOT NULL DEFAULT '',
  source_kind TEXT NOT NULL DEFAULT '',
  realm_kind TEXT NOT NULL DEFAULT '',
  probe_method_id TEXT NOT NULL DEFAULT '',
  capture_id TEXT,
  attempt_id TEXT,
  envelope_json TEXT NOT NULL,
  created_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_obs_session ON observation_events(session_id, created_ms);

CREATE TABLE IF NOT EXISTS api_idempotency (
  tenant_id TEXT NOT NULL DEFAULT '',
  route TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  body_hash TEXT NOT NULL,
  status INTEGER NOT NULL,
  response_json TEXT NOT NULL,
  created_ms INTEGER NOT NULL,
  PRIMARY KEY (tenant_id, route, idempotency_key)
);

CREATE TABLE IF NOT EXISTS analysis_results (
  session_id TEXT NOT NULL,
  rev INTEGER NOT NULL,
  result_json TEXT NOT NULL,
  created_ms INTEGER NOT NULL,
  real_band TEXT,
  device_id TEXT,
  bot_verdict TEXT,
  device_confidence REAL,
  PRIMARY KEY (session_id, rev),
  FOREIGN KEY (session_id) REFERENCES sessions(session_id)
);

CREATE TABLE IF NOT EXISTS analyze_jobs (
  session_id TEXT PRIMARY KEY,
  due_ms INTEGER NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending',
  locked_until INTEGER NOT NULL DEFAULT 0,
  locked_by TEXT NOT NULL DEFAULT '',
  updated_ms INTEGER NOT NULL,
  FOREIGN KEY (session_id) REFERENCES sessions(session_id)
);

CREATE TABLE IF NOT EXISTS page_results (
  session_id TEXT NOT NULL,
  page_id TEXT NOT NULL,
  page_rev INTEGER NOT NULL,
  result_json TEXT NOT NULL,
  created_ms INTEGER NOT NULL,
  PRIMARY KEY (session_id, page_id, page_rev),
  FOREIGN KEY (session_id) REFERENCES sessions(session_id)
);

CREATE INDEX IF NOT EXISTS idx_batches_session ON probe_batches(session_id);
CREATE INDEX IF NOT EXISTS idx_analysis_session ON analysis_results(session_id);
CREATE INDEX IF NOT EXISTS idx_analyze_jobs_due ON analyze_jobs(status, due_ms, locked_until);
CREATE INDEX IF NOT EXISTS idx_page_results_session ON page_results(session_id, page_id);
CREATE INDEX IF NOT EXISTS idx_sessions_vt ON sessions(visitor_terminal_id);
CREATE INDEX IF NOT EXISTS idx_sessions_created_ms ON sessions(created_ms);
CREATE INDEX IF NOT EXISTS idx_batches_created_ms ON probe_batches(created_ms);
CREATE INDEX IF NOT EXISTS idx_analysis_created_ms ON analysis_results(created_ms);
-- idx_analysis_device_id created after open-time ALTER migrate (old DBs lack device_id column)

-- visitor_terminal cool / active-cycle index (vt-scoped 24h cool + resume)
CREATE TABLE IF NOT EXISTS visitor_terminals (
  vt_id TEXT PRIMARY KEY,
  last_complete_ms INTEGER NOT NULL DEFAULT 0,
  cool_until_ms INTEGER NOT NULL DEFAULT 0,
  active_cycle_id TEXT NOT NULL DEFAULT '',
  updated_ms INTEGER NOT NULL,
  meta_json TEXT NOT NULL DEFAULT '{}'
);

-- DeviceIndex multi-tenant production tables (SQLite + PG)
CREATE TABLE IF NOT EXISTS device_index_devices (
  tenant_id TEXT NOT NULL,
  device_id TEXT NOT NULL,
  binder_obs_json TEXT NOT NULL,
  updated_ms INTEGER NOT NULL,
  PRIMARY KEY (tenant_id, device_id)
);
CREATE TABLE IF NOT EXISTS device_index_keys (
  tenant_id TEXT NOT NULL,
  binder_key TEXT NOT NULL,
  device_id TEXT NOT NULL,
  PRIMARY KEY (tenant_id, binder_key, device_id)
);
CREATE INDEX IF NOT EXISTS idx_device_index_keys_dev ON device_index_keys(tenant_id, device_id);
CREATE INDEX IF NOT EXISTS idx_device_index_keys_binder ON device_index_keys(tenant_id, binder_key);

-- Client/server ops events (lab sqlite parity with PG ops tables)
CREATE TABLE IF NOT EXISTS ops_client_events (
  event_id TEXT PRIMARY KEY,
  ts_ms INTEGER NOT NULL,
  server_recv_ms INTEGER NOT NULL,
  site_id TEXT NOT NULL DEFAULT '',
  visitor_terminal_id TEXT NOT NULL DEFAULT '',
  session_id TEXT NOT NULL DEFAULT '',
  product_version TEXT NOT NULL DEFAULT '',
  inject_path TEXT NOT NULL DEFAULT '',
  engine_family TEXT NOT NULL DEFAULT '',
  ua_hash TEXT NOT NULL DEFAULT '',
  stage TEXT NOT NULL DEFAULT '',
  code TEXT NOT NULL DEFAULT '',
  severity TEXT NOT NULL DEFAULT 'error',
  detail_json TEXT NOT NULL DEFAULT '{}',
  sample_rate REAL NOT NULL DEFAULT 1.0,
  client_ip TEXT
);
CREATE INDEX IF NOT EXISTS idx_ops_ce_ts ON ops_client_events(server_recv_ms DESC);
CREATE INDEX IF NOT EXISTS idx_ops_ce_code ON ops_client_events(code, server_recv_ms DESC);
CREATE INDEX IF NOT EXISTS idx_ops_ce_pv ON ops_client_events(product_version, server_recv_ms DESC);

CREATE TABLE IF NOT EXISTS ops_server_events (
  event_id TEXT PRIMARY KEY,
  ts_ms INTEGER NOT NULL,
  site_id TEXT NOT NULL DEFAULT '',
  visitor_terminal_id TEXT NOT NULL DEFAULT '',
  session_id TEXT NOT NULL DEFAULT '',
  product_version TEXT NOT NULL DEFAULT '',
  engine_family TEXT NOT NULL DEFAULT '',
  stage TEXT NOT NULL DEFAULT '',
  code TEXT NOT NULL DEFAULT '',
  severity TEXT NOT NULL DEFAULT 'error',
  detail_json TEXT NOT NULL DEFAULT '{}',
  client_ip TEXT
);
CREATE INDEX IF NOT EXISTS idx_ops_se_ts ON ops_server_events(ts_ms DESC);
CREATE INDEX IF NOT EXISTS idx_ops_se_code ON ops_server_events(code, ts_ms DESC);

-- Soft edge store (multi-worker durable; promote never true at product layer)
CREATE TABLE IF NOT EXISTS soft_edges (
  tenant_id TEXT NOT NULL,
  a_session TEXT NOT NULL,
  b_session TEXT NOT NULL,
  priority TEXT NOT NULL DEFAULT 'p1',
  confidence REAL NOT NULL DEFAULT 0,
  reason TEXT NOT NULL DEFAULT '',
  created_ms INTEGER NOT NULL,
  PRIMARY KEY (tenant_id, a_session, b_session)
);
CREATE TABLE IF NOT EXISTS soft_heat (
  tenant_id TEXT NOT NULL,
  device_id TEXT NOT NULL,
  session_id TEXT NOT NULL,
  first_ms INTEGER NOT NULL,
  last_ms INTEGER NOT NULL,
  PRIMARY KEY (tenant_id, device_id, session_id)
);
CREATE INDEX IF NOT EXISTS idx_soft_heat_device ON soft_heat(tenant_id, device_id);
"#;

struct SqliteStore {
    conn: Mutex<Connection>,
    path: PathBuf,
}

enum Backend {
    Sqlite(SqliteStore),
    Postgres(PgStore),
}

/// Unified store facade: SQLite for local/single-node, Postgres for multi-worker.
pub struct Store {
    backend: Backend,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        Ok(Self {
            backend: Backend::Sqlite(SqliteStore::open(path)?),
        })
    }

    pub fn open_postgres(dsn: &str) -> Result<Self, StoreError> {
        Ok(Self {
            backend: Backend::Postgres(PgStore::open(dsn)?),
        })
    }

    /// Prefer `database_url` when non-empty; otherwise require `GR_DATABASE_URL`.
    /// SQLite probe store is not used at runtime.
    /// iss/opus5 06-P1-5: monthly per-site session counter (billing signal).
    pub fn bump_monthly_sessions(&self, site_id: &str) -> Result<(String, u64), StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.bump_monthly_sessions(site_id),
            Backend::Postgres(p) => p.bump_monthly_sessions(site_id),
        }
    }

    /// iss/opus5 05 low (seal replay ledger): `false` = exact
    /// (session_id, batch_id, nonce) tuple already consumed (replay).
    pub fn mark_seal_consumed(
        &self,
        session_id: &str,
        batch_id: &str,
        nonce: &str,
    ) -> Result<bool, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.mark_seal_consumed(session_id, batch_id, nonce),
            Backend::Postgres(p) => p.mark_seal_consumed(session_id, batch_id, nonce),
        }
    }

    pub fn open_auto(db_path: impl AsRef<Path>, database_url: Option<&str>) -> Result<Self, StoreError> {
        if let Some(url) = database_url {
            let t = url.trim();
            if !t.is_empty() {
                return Self::open_postgres(t);
            }
        }
        for k in ["GR_DATABASE_URL", "GR_DATABASE_URL"] {
            if let Ok(v) = std::env::var(k) {
                let t = v.trim();
                if !t.is_empty() {
                    return Self::open_postgres(t);
                }
            }
        }
        let _ = db_path;
        Err(StoreError::Msg(
            "GR_DATABASE_URL is required; SQLite probe store was removed".into(),
        ))
    }

    pub fn path(&self) -> PathBuf {
        match &self.backend {
            Backend::Sqlite(s) => s.path.clone(),
            Backend::Postgres(s) => PathBuf::from(s.label()),
        }
    }

    pub fn backend_name(&self) -> &'static str {
        match &self.backend {
            Backend::Sqlite(_) => "sqlite",
            Backend::Postgres(_) => "postgres",
        }
    }

    pub fn backend_label(&self) -> String {
        match &self.backend {
            Backend::Sqlite(s) => s.path.display().to_string(),
            Backend::Postgres(s) => s.label().to_string(),
        }
    }

    pub fn open_session(
        &self,
        session_id: Option<String>,
        visitor_terminal_id: Option<String>,
        meta: Option<Value>,
    ) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.open_session(session_id, visitor_terminal_id, meta),
            Backend::Postgres(s) => s.open_session(session_id, visitor_terminal_id, meta),
        }
    }

    /// Product open: resume/cool/new **probe cycle** (session_id column stores cycle_id).
    /// Returns phase, cycle_id, session_id (=cycle_id), skip_identity_probe, etc.
    pub fn open_cycle(
        &self,
        cycle_id_hint: Option<String>,
        visitor_terminal_id: Option<String>,
        meta: Option<Value>,
    ) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.open_cycle(cycle_id_hint, visitor_terminal_id, meta),
            Backend::Postgres(s) => s.open_cycle(cycle_id_hint, visitor_terminal_id, meta),
        }
    }

    pub fn insert_ops_client_event(&self, row: Value) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Postgres(s) => s.insert_ops_client_event(row),
            Backend::Sqlite(s) => sqlite_insert_ops_client(s, row),
        }
    }

    pub fn insert_ops_server_event(&self, row: Value) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Postgres(s) => s.insert_ops_server_event(row),
            Backend::Sqlite(s) => sqlite_insert_ops_server(s, row),
        }
    }

    /// Append-only observation lineage. Duplicate observation_id is ignored.
    pub fn insert_observation_event(&self, row: Value) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Postgres(s) => s.insert_observation_event(row),
            Backend::Sqlite(s) => sqlite_insert_observation_event(s, row),
        }
    }

    /// Shared per-minute rate-limit counter. PostgreSQL is the multi-node
    /// authority; SQLite (single-process) returns an error so callers use the
    /// in-process window instead.
    pub fn bump_rate_limit_window(&self, key: &str, window_ms: i64) -> Result<i64, StoreError> {
        match &self.backend {
            Backend::Postgres(s) => s.bump_rate_limit_window(key, window_ms),
            Backend::Sqlite(_) => Err(StoreError::Msg("rate_limit_local".into())),
        }
    }

    pub fn lookup_api_idempotency(
        &self,
        tenant_id: &str,
        route: &str,
        idempotency_key: &str,
    ) -> Result<Option<Value>, StoreError> {
        match &self.backend {
            Backend::Postgres(s) => s.lookup_api_idempotency(tenant_id, route, idempotency_key),
            Backend::Sqlite(s) => sqlite_lookup_api_idempotency(s, tenant_id, route, idempotency_key),
        }
    }

    pub fn put_api_idempotency(
        &self,
        tenant_id: &str,
        route: &str,
        idempotency_key: &str,
        body_hash: &str,
        status: i64,
        response: &Value,
    ) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Postgres(s) => s.put_api_idempotency(
                tenant_id,
                route,
                idempotency_key,
                body_hash,
                status,
                response,
            ),
            Backend::Sqlite(s) => sqlite_put_api_idempotency(
                s,
                tenant_id,
                route,
                idempotency_key,
                body_hash,
                status,
                response,
            ),
        }
    }

    pub fn list_ops_events(
        &self,
        source: &str,
        limit: i64,
        code: Option<String>,
        site_id: Option<String>,
        since_ms: Option<i64>,
        product_version: Option<String>,
    ) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Postgres(s) => {
                s.list_ops_events(source, limit, code, site_id, since_ms, product_version)
            }
            Backend::Sqlite(s) => {
                sqlite_list_ops_events(s, source, limit, code, site_id, since_ms, product_version)
            }
        }
    }

    pub fn ops_b10_health(&self, since_ms: i64, limit_sites: i64) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Postgres(s) => s.ops_b10_health(since_ms, limit_sites),
            Backend::Sqlite(s) => sqlite_ops_b10_health(s, since_ms, limit_sites),
        }
    }

    /// Probe completeness board: main_complete / missing / gateway_only by site × product_version.
    pub fn ops_probe_completeness(&self, since_ms: i64) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Postgres(s) => s.ops_probe_completeness(since_ms),
            Backend::Sqlite(s) => sqlite_ops_probe_completeness(s, since_ms),
        }
    }

    /// Outcome distribution for panel charts: bot_verdict / real_band by site (denorm scalars).
    pub fn ops_outcome_distribution(&self, since_ms: i64) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Postgres(s) => s.ops_outcome_distribution(since_ms),
            Backend::Sqlite(s) => sqlite_ops_outcome_distribution(s, since_ms),
        }
    }

    /// Record velocity hits (device_id / client_ip) for 5m/1h/24h windows.
    pub fn velocity_record(
        &self,
        session_id: &str,
        device_id: Option<&str>,
        client_ip: Option<&str>,
        hit_ms: i64,
    ) -> Result<(), StoreError> {
        match &self.backend {
            Backend::Postgres(s) => s.velocity_record(session_id, device_id, client_ip, hit_ms),
            Backend::Sqlite(s) => sqlite_velocity_record(s, session_id, device_id, client_ip, hit_ms),
        }
    }

    /// Summarize velocity windows for product denorm / ops.
    pub fn velocity_summary(
        &self,
        device_id: Option<&str>,
        client_ip: Option<&str>,
        now_ms: i64,
    ) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Postgres(s) => s.velocity_summary(device_id, client_ip, now_ms),
            Backend::Sqlite(s) => sqlite_velocity_summary(s, device_id, client_ip, now_ms),
        }
    }

    /// Mark cycle identity-complete; start vt 24h cool.
    pub fn complete_cycle(&self, cycle_id: &str) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.complete_cycle(cycle_id),
            Backend::Postgres(s) => s.complete_cycle(cycle_id),
        }
    }

    /// Drop incomplete-cycle evidence (batches + jobs + unconfirmed analyses).
    /// Batched retention purge — never full-table storms.
    ///
    /// Deletes up to `limit` oldest analysis rows older than `older_than_ms`, then
    /// orphaned sessions without recent activity (same limit). Returns counts.
    /// iss/opus5 04-P1-6: `ops_older_than_ms` bounds observation_events /
    /// ops_*_events / api_idempotency; `master_older_than_ms` bounds the
    /// commercial master tables (devices, device_sessions, device_index_*,
    /// soft_edges, soft_heat). `None` skips that group.
    pub fn retention_purge_batch(
        &self,
        older_than_ms: i64,
        limit: i64,
        velocity_older_than_ms: Option<i64>,
        ops_older_than_ms: Option<i64>,
        master_older_than_ms: Option<i64>,
    ) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Postgres(s) => s.retention_purge_batch(
                older_than_ms,
                limit,
                velocity_older_than_ms,
                ops_older_than_ms,
                master_older_than_ms,
            ),
            Backend::Sqlite(s) => s.retention_purge_batch(
                older_than_ms,
                limit,
                velocity_older_than_ms,
                ops_older_than_ms,
                master_older_than_ms,
            ),
        }
    }

    pub fn purge_cycle_evidence(&self, cycle_id: &str) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.purge_cycle_evidence(cycle_id),
            Backend::Postgres(s) => s.purge_cycle_evidence(cycle_id),
        }
    }

    /// iss/opus5 05-S-5: DSAR erase — cascade-delete all data for a subject
    /// (visitor_terminal_id | device_id | client_ip | site_id).
    pub fn subject_erase(&self, kind: &str, value: &str) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Postgres(s) => s.subject_erase(kind, value),
            Backend::Sqlite(s) => sqlite_subject_erase(s, kind, value),
        }
    }

    /// iss/opus5 05-S-5: DSAR export — everything held for a subject as JSON.
    pub fn subject_export(&self, kind: &str, value: &str) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Postgres(s) => s.subject_export(kind, value),
            Backend::Sqlite(s) => sqlite_subject_export(s, kind, value),
        }
    }

    /// If analyze result is terminal / ticketed, mark cycle complete (idempotent).
    pub fn maybe_complete_cycle_from_analysis(
        &self,
        cycle_id: &str,
        result: &Value,
    ) -> Result<Option<Value>, StoreError> {
        if !analysis_completes_cycle(result) {
            return Ok(None);
        }
        Ok(Some(self.complete_cycle(cycle_id)?))
    }

    pub fn require_active_session(&self, session_id: &str) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.require_active_session(session_id),
            Backend::Postgres(s) => s.require_active_session(session_id),
        }
    }

    pub fn upsert_batch(
        &self,
        session_id: &str,
        batch_id: &str,
        source: &str,
        payload: &Value,
    ) -> Result<Value, StoreError> {
        self.upsert_batch_with_ip(session_id, batch_id, source, payload, None)
    }

    pub fn upsert_batch_with_ip(
        &self,
        session_id: &str,
        batch_id: &str,
        source: &str,
        payload: &Value,
        client_ip: Option<&str>,
    ) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => {
                // SQLite: same upsert; client_ip folded into payload fields for lab.
                let mut p = payload.clone();
                if let Some(ip) = client_ip {
                    if let Some(obj) = p.as_object_mut() {
                        let fields = obj
                            .entry("fields".to_string())
                            .or_insert_with(|| json!({}));
                        if let Some(fo) = fields.as_object_mut() {
                            fo.entry("server_client_ip".to_string())
                                .or_insert_with(|| json!(ip));
                        }
                    }
                }
                s.upsert_batch(session_id, batch_id, source, &p)
            }
            Backend::Postgres(s) => {
                s.upsert_batch_with_ip(session_id, batch_id, source, payload, client_ip)
            }
        }
    }

    pub fn cross_query_analysis(
        &self,
        device_id: Option<&str>,
        client_ip: Option<&str>,
        bot_verdict: Option<&str>,
        real_band: Option<&str>,
        field_key: Option<&str>,
        field_value: Option<&str>,
        limit: i64,
    ) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Postgres(s) => s.cross_query_analysis(
                device_id,
                client_ip,
                bot_verdict,
                real_band,
                field_key,
                field_value,
                limit,
            ),
            Backend::Sqlite(_) => Ok(json!({
                "ok": true,
                "count": 0,
                "rows": [],
                "note": "cross_query requires PostgreSQL cold store"
            })),
        }
    }

    /// Commercial device master (PG analysis_latest write path).
    pub fn get_device(&self, device_id: &str) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Postgres(s) => s.get_device(device_id),
            Backend::Sqlite(_) => Ok(json!({
                "ok": true,
                "found": false,
                "device_id": device_id,
                "note": "devices table requires PostgreSQL"
            })),
        }
    }

    pub fn list_device_sessions(&self, device_id: &str, limit: i64) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Postgres(s) => s.list_device_sessions(device_id, limit),
            Backend::Sqlite(_) => Ok(json!({
                "ok": true,
                "device_id": device_id,
                "count": 0,
                "rows": [],
                "note": "device_sessions requires PostgreSQL"
            })),
        }
    }

    pub fn lookup_devices_by_binder(
        &self,
        tenant_id: &str,
        binder_key: &str,
        limit: i64,
    ) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Postgres(s) => s.lookup_devices_by_binder(tenant_id, binder_key, limit),
            Backend::Sqlite(s) => s.lookup_devices_by_binder(tenant_id, binder_key, limit),
        }
    }

    pub fn list_analysis_latest(
        &self,
        limit: i64,
        device_tier: Option<&str>,
        since_ms: Option<i64>,
        site_id: Option<&str>,
    ) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Postgres(s) => s.list_analysis_latest(limit, device_tier, since_ms, site_id),
            Backend::Sqlite(s) => s.list_analysis_latest(limit, device_tier, since_ms, site_id),
        }
    }

    /// Soft edge put (durable multi-worker).
    pub fn soft_edge_put(
        &self,
        tenant_id: &str,
        a_session: &str,
        b_session: &str,
        priority: &str,
        confidence: f64,
        reason: &str,
    ) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => {
                s.soft_edge_put(tenant_id, a_session, b_session, priority, confidence, reason)
            }
            Backend::Postgres(s) => {
                s.soft_edge_put(tenant_id, a_session, b_session, priority, confidence, reason)
            }
        }
    }

    pub fn soft_edge_list(&self, tenant_id: &str) -> Result<Vec<Value>, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.soft_edge_list(tenant_id),
            Backend::Postgres(s) => s.soft_edge_list(tenant_id),
        }
    }

    pub fn soft_heat_record(
        &self,
        tenant_id: &str,
        device_id: &str,
        session_id: &str,
    ) -> Result<i64, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.soft_heat_record(tenant_id, device_id, session_id),
            Backend::Postgres(s) => s.soft_heat_record(tenant_id, device_id, session_id),
        }
    }

    pub fn soft_heat_get(&self, tenant_id: &str, device_id: &str) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.soft_heat_get(tenant_id, device_id),
            Backend::Postgres(s) => s.soft_heat_get(tenant_id, device_id),
        }
    }

    /// Soft edge count for fuse (tenant).
    pub fn soft_edge_count(&self, tenant_id: &str) -> Result<i64, StoreError> {
        Ok(self.soft_edge_list(tenant_id)?.len() as i64)
    }

    pub fn get_probe_cold(
        &self,
        session_id: &str,
        batch_id: &str,
        source: Option<&str>,
    ) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Postgres(s) => s.get_probe_cold(session_id, batch_id, source),
            Backend::Sqlite(_) => Ok(json!({
                "ok": true,
                "found": false,
                "note": "probe_cold requires PostgreSQL"
            })),
        }
    }

    pub fn probe_volume_stats(&self, session_id: &str) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Postgres(s) => s.probe_volume_stats(session_id),
            Backend::Sqlite(_) => Ok(json!({"ok": true, "note": "volume stats require PostgreSQL"})),
        }
    }

    /// List non-expired cold rows for a VT (newest first), within promote window.
    pub fn list_cold_for_vt(
        &self,
        visitor_terminal_id: &str,
        since_ms: i64,
        limit: i64,
    ) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Postgres(s) => s.list_cold_for_vt(visitor_terminal_id, since_ms, limit),
            Backend::Sqlite(_) => Ok(json!({"ok": true, "rows": [], "count": 0})),
        }
    }

    /// Delete cold rows older than TTL; returns deleted count.
    pub fn purge_expired_cold(&self, older_than_ms: i64) -> Result<i64, StoreError> {
        match &self.backend {
            Backend::Postgres(s) => s.purge_expired_cold(older_than_ms),
            Backend::Sqlite(_) => Ok(0),
        }
    }

    pub fn schedule_analyze(&self, session_id: &str, debounce_ms: i64) -> Result<(), StoreError> {
        // Default: pull earlier (safe for milestone-style arms and legacy callers).
        self.schedule_analyze_merge(session_id, debounce_ms, AnalyzeDueMerge::PullEarlier)
    }

    pub fn schedule_analyze_merge(
        &self,
        session_id: &str,
        debounce_ms: i64,
        merge: AnalyzeDueMerge,
    ) -> Result<(), StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.schedule_analyze_merge(session_id, debounce_ms, merge),
            Backend::Postgres(s) => s.schedule_analyze_merge(session_id, debounce_ms, merge),
        }
    }

    pub fn claim_due_analyze_jobs(
        &self,
        worker_id: &str,
        limit: usize,
        lock_ms: i64,
    ) -> Result<Vec<String>, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.claim_due_analyze_jobs(worker_id, limit, lock_ms),
            Backend::Postgres(s) => s.claim_due_analyze_jobs(worker_id, limit, lock_ms),
        }
    }

    pub fn complete_analyze_job(&self, session_id: &str, worker_id: &str) -> Result<bool, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.complete_analyze_job(session_id, worker_id),
            Backend::Postgres(s) => s.complete_analyze_job(session_id, worker_id),
        }
    }

    pub fn pending_analyze_job_count(&self) -> Result<i64, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.pending_analyze_job_count(),
            Backend::Postgres(s) => s.pending_analyze_job_count(),
        }
    }

    pub fn analyze_queue_stats(&self) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.analyze_queue_stats(),
            Backend::Postgres(s) => s.analyze_queue_stats(),
        }
    }

    pub fn build_evidence(&self, session_id: &str) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.build_evidence(session_id),
            Backend::Postgres(s) => s.build_evidence(session_id),
        }
    }

    pub fn save_analysis(&self, session_id: &str, result: &Value) -> Result<i64, StoreError> {
        let rev = match &self.backend {
            Backend::Sqlite(s) => s.save_analysis(session_id, result)?,
            Backend::Postgres(s) => s.save_analysis(session_id, result)?,
        };
        // Velocity hit (best-effort; never fail analysis on velocity errors)
        let sc = analysis_report_scalars(result);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let _ = self.velocity_record(
            session_id,
            sc.device_id.as_deref(),
            sc.client_ip.as_deref(),
            now,
        );
        Ok(rev)
    }

    pub fn force_session_times(
        &self,
        session_id: &str,
        created_ms: i64,
        updated_ms: i64,
    ) -> Result<(), StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.force_session_times(session_id, created_ms, updated_ms),
            Backend::Postgres(s) => s.force_session_times(session_id, created_ms, updated_ms),
        }
    }

    pub fn latest_analysis(&self, session_id: &str) -> Result<Option<Value>, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.latest_analysis(session_id),
            Backend::Postgres(s) => s.latest_analysis(session_id),
        }
    }

    /// Parsed session meta_json, or None when the session is absent.
    pub fn session_meta(&self, session_id: &str) -> Result<Option<Value>, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.session_meta(session_id),
            Backend::Postgres(s) => s.session_meta(session_id),
        }
    }

    /// Append-only observation events for a session (oldest first).
    pub fn list_observation_events(
        &self,
        session_id: &str,
        limit: i64,
    ) -> Result<Vec<Value>, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.list_observation_events(session_id, limit),
            Backend::Postgres(s) => s.list_observation_events(session_id, limit),
        }
    }

    pub fn list_analyses(&self, session_id: &str) -> Result<Vec<Value>, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.list_analyses(session_id),
            Backend::Postgres(s) => s.list_analyses(session_id),
        }
    }

    pub fn list_received_batches(&self, session_id: &str) -> Result<Vec<Value>, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.list_received_batches(session_id),
            Backend::Postgres(s) => s.list_received_batches(session_id),
        }
    }

    /// Peer sessions for multi-browser / multi-session link:
    /// same visitor_terminal_id and/or same meta.harness_run (generic cohort tags).
    pub fn list_peer_session_ids(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<String>, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.list_peer_session_ids(session_id, limit),
            Backend::Postgres(s) => s.list_peer_session_ids(session_id, limit),
        }
    }

    pub fn has_batch(
        &self,
        session_id: &str,
        batch_id: &str,
        source: &str,
    ) -> Result<bool, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.has_batch(session_id, batch_id, source),
            Backend::Postgres(s) => s.has_batch(session_id, batch_id, source),
        }
    }

    pub fn session_window(&self, session_id: &str) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.session_window(session_id),
            Backend::Postgres(s) => s.session_window(session_id),
        }
    }

    /// Recent sessions for guest SLA 大盘 (newest first).
    pub fn list_recent_session_ids(&self, limit: usize) -> Result<Vec<String>, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.list_recent_session_ids(limit),
            Backend::Postgres(s) => s.list_recent_session_ids(limit),
        }
    }

    /// Latest analysis result JSON per session (for SLA aggregation).
    pub fn list_recent_latest_analyses(&self, limit: usize) -> Result<Vec<Value>, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.list_recent_latest_analyses(limit),
            Backend::Postgres(s) => s.list_recent_latest_analyses(limit),
        }
    }

    /// Persist page-scoped product result (rpa + page_rev).
    pub fn save_page_result(
        &self,
        session_id: &str,
        page_id: &str,
        page_rev: i64,
        result: &Value,
    ) -> Result<(), StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.save_page_result(session_id, page_id, page_rev, result),
            Backend::Postgres(s) => s.save_page_result(session_id, page_id, page_rev, result),
        }
    }

    pub fn latest_page_result(
        &self,
        session_id: &str,
        page_id: &str,
    ) -> Result<Option<Value>, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.latest_page_result(session_id, page_id),
            Backend::Postgres(s) => s.latest_page_result(session_id, page_id),
        }
    }

    pub fn list_page_results(&self, session_id: &str) -> Result<Vec<Value>, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.list_page_results(session_id),
            Backend::Postgres(s) => s.list_page_results(session_id),
        }
    }

    /// Patch session meta_json (merge object keys). Used for session_ticket / inject.
    pub fn merge_session_meta(&self, session_id: &str, patch: &Value) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.merge_session_meta(session_id, patch),
            Backend::Postgres(s) => s.merge_session_meta(session_id, patch),
        }
    }

    /// DeviceIndex upsert (sqlite lab + single-region PG lab).
    pub fn device_index_upsert(
        &self,
        tenant_id: &str,
        device_id: &str,
        binder_obs: &Value,
        binder_keys: &[String],
    ) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.device_index_upsert(tenant_id, device_id, binder_obs, binder_keys),
            Backend::Postgres(s) => {
                s.device_index_upsert(tenant_id, device_id, binder_obs, binder_keys)
            }
        }
    }

    /// Export store DeviceIndex as FileDeviceIndex v1 JSON shape.
    pub fn device_index_export(&self, tenant_id: &str) -> Result<Value, StoreError> {
        match &self.backend {
            Backend::Sqlite(s) => s.device_index_export(tenant_id),
            Backend::Postgres(s) => s.device_index_export(tenant_id),
        }
    }

    /// Production multi-tenant DeviceIndex: link_or_mint-shaped upsert after analyze.
    pub fn device_index_link_upsert_from_result(
        &self,
        tenant_id: &str,
        result: &Value,
    ) -> Result<Value, StoreError> {
        let device = result.get("device").cloned().unwrap_or(Value::Null);
        let did = device
            .get("device_id")
            .and_then(|v| v.as_str())
            .filter(|s| is_commercial_device_id_str(s) && s.len() > 4)
            .map(|s| s.to_string());
        let Some(device_id) = did else {
            return Ok(json!({
                "ok": true,
                "skipped": true,
                "reason": "no_commercial_device_id",
                "tenant_id": tenant_id,
            }));
        };
        let trust = device.get("trust").cloned().unwrap_or(json!({}));
        let binder_obs = json!({
            "device_id": device_id,
            "residual_class": trust.get("residual_class"),
            "materials_included": trust.get("materials_included"),
            "eligible": trust.get("eligible"),
            "digest_path": device.get("digest_path").or_else(|| trust.get("digest_path")),
            "association_level": device.get("association_level"),
            "soft_class": device.get("soft_stack").or_else(|| trust.get("soft_stack")),
            "updated_from": "analyze_result",
        });
        let mut keys: Vec<String> = Vec::new();
        if let Some(arr) = trust
            .get("materials_included")
            .and_then(|a| a.as_array())
            .or_else(|| device.get("binder_keys").and_then(|a| a.as_array()))
        {
            for x in arr {
                if let Some(s) = x.as_str() {
                    keys.push(format!("mat:{s}"));
                }
            }
        }
        // Host separators when present on device projection
        for k in ["webrtc_host_ip_hash", "os_instance_hash", "unit_surface_id"] {
            if let Some(s) = device
                .get(k)
                .or_else(|| trust.get(k))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                keys.push(format!("{k}:{s}"));
            }
        }
        if keys.is_empty() {
            keys.push(format!("dv:{device_id}"));
        }
        let mut out = self.device_index_upsert(tenant_id, &device_id, &binder_obs, &keys)?;
        if let Some(obj) = out.as_object_mut() {
            obj.insert("production_path".into(), json!(true));
            obj.insert("multi_tenant".into(), json!(true));
            obj.insert("backend".into(), json!(self.backend_name()));
        }
        Ok(out)
    }
}

impl SqliteStore {
    fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StoreError::Msg(e.to_string()))?;
        }
        let conn = Connection::open(&path)?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        conn.execute_batch(SCHEMA)?;
        // Existing lab DBs may predate analysis_results scalar columns. CREATE TABLE
        // IF NOT EXISTS does not add columns; indexes on device_id must wait until
        // ALTER succeeds (duplicate-column errors ignored).
        for ddl in [
            "ALTER TABLE analysis_results ADD COLUMN real_band TEXT",
            "ALTER TABLE analysis_results ADD COLUMN device_id TEXT",
            "ALTER TABLE analysis_results ADD COLUMN bot_verdict TEXT",
            "ALTER TABLE analysis_results ADD COLUMN device_confidence REAL",
            "ALTER TABLE analysis_results ADD COLUMN client_ip TEXT",
            "ALTER TABLE analysis_results ADD COLUMN device_tier TEXT",
            "ALTER TABLE analysis_results ADD COLUMN collision_risk INTEGER",
            "ALTER TABLE analysis_results ADD COLUMN product_version TEXT",
            "ALTER TABLE analysis_results ADD COLUMN digest_path TEXT",
            "ALTER TABLE analysis_results ADD COLUMN residual_entropy_ok INTEGER",
            // iss/72 material identity columns on probe_batches
            "ALTER TABLE probe_batches ADD COLUMN material_generation INTEGER NOT NULL DEFAULT 1",
            "ALTER TABLE probe_batches ADD COLUMN material_hash TEXT",
            "ALTER TABLE probe_batches ADD COLUMN capture_id TEXT",
        ] {
            let _ = conn.execute(ddl, []);
        }
        let _ = conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_analysis_device_id ON analysis_results(device_id);
             CREATE INDEX IF NOT EXISTS idx_batches_capture ON probe_batches(capture_id);
             CREATE INDEX IF NOT EXISTS idx_batches_mat_gen ON probe_batches(session_id, batch_id, material_generation);",
        );
        Ok(Self {
            conn: Mutex::new(conn),
            path,
        })
    }

    /// iss/opus5 06-P1-5: monthly per-site session counter (billing signal).
    /// Best-effort: returns `(month_key, sessions_this_month)`.
    fn bump_monthly_sessions(&self, site_id: &str) -> Result<(String, u64), StoreError> {
        let month = month_key_utc();
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS usage_monthly (
               month TEXT NOT NULL,
               site_id TEXT NOT NULL,
               sessions INTEGER NOT NULL DEFAULT 0,
               PRIMARY KEY (month, site_id)
             )",
        )?;
        conn.execute(
            "INSERT INTO usage_monthly(month, site_id, sessions) VALUES(?1,?2,1)
             ON CONFLICT(month, site_id) DO UPDATE SET sessions = sessions + 1",
            params![month, site_id],
        )?;
        let n: i64 = conn.query_row(
            "SELECT sessions FROM usage_monthly WHERE month=?1 AND site_id=?2",
            params![month, site_id],
            |r| r.get(0),
        )?;
        Ok((month, n as u64))
    }

    /// iss/opus5 05 low (seal replay ledger): record a consumed
    /// (session_id, batch_id, nonce) tuple. Returns `false` when the exact
    /// tuple was already consumed (replay) — caller rejects 409.
    fn mark_seal_consumed(
        &self,
        session_id: &str,
        batch_id: &str,
        nonce: &str,
    ) -> Result<bool, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS seal_consumed (
               session_id TEXT NOT NULL,
               batch_id TEXT NOT NULL,
               nonce TEXT NOT NULL,
               consumed_ms INTEGER NOT NULL DEFAULT 0,
               PRIMARY KEY (session_id, batch_id, nonce)
             )",
        )?;
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO seal_consumed(session_id, batch_id, nonce, consumed_ms)
             VALUES(?1, ?2, ?3, ?4)",
            params![session_id, batch_id, nonce, now_ms()],
        )?;
        Ok(inserted > 0)
    }

    fn open_session(
        &self,
        session_id: Option<String>,
        visitor_terminal_id: Option<String>,
        meta: Option<Value>,
    ) -> Result<Value, StoreError> {
        let sid = session_id.unwrap_or_else(|| format!("sess_{}", hex_now()));
        let mut meta_v = meta.unwrap_or_else(|| json!({}));
        if !meta_v.is_object() {
            meta_v = json!({});
        }
        let ts = now_ms();
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let prev: Option<(String, String)> = conn
            .query_row(
                "SELECT visitor_terminal_id, meta_json FROM sessions WHERE session_id=?1",
                params![sid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let vt = if let Some((prev_vt, prev_s)) = prev {
            if let Ok(Value::Object(prev_m)) = serde_json::from_str::<Value>(&prev_s) {
                if let Some(obj) = meta_v.as_object_mut() {
                    let prev_ip = prev_m
                        .get("inject_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let new_ip = obj
                        .get("inject_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let primary = |s: &str| s == "nginx" || s == "cf_worker";
                    let keep_ip = if primary(prev_ip) {
                        prev_ip.to_string()
                    } else if primary(new_ip) {
                        new_ip.to_string()
                    } else if !new_ip.is_empty() {
                        new_ip.to_string()
                    } else {
                        prev_ip.to_string()
                    };
                    for (k, v) in prev_m {
                        obj.entry(k).or_insert(v);
                    }
                    if !keep_ip.is_empty() {
                        obj.insert("inject_path".into(), json!(keep_ip));
                    }
                }
            }
            if !prev_vt.is_empty() {
                prev_vt
            } else {
                visitor_terminal_id
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| format!("vt_{}", hex_now()))
            }
        } else {
            visitor_terminal_id.unwrap_or_else(|| format!("vt_{}", hex_now()))
        };
        let inject_path = meta_v
            .get("inject_path")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let meta_s = serde_json::to_string(&meta_v)?;
        conn.execute(
            "INSERT INTO sessions(session_id, visitor_terminal_id, created_ms, updated_ms, meta_json)
             VALUES(?1,?2,?3,?3,?4)
             ON CONFLICT(session_id) DO UPDATE SET
               updated_ms=excluded.updated_ms,
               visitor_terminal_id=COALESCE(NULLIF(visitor_terminal_id, ''), excluded.visitor_terminal_id),
               meta_json=excluded.meta_json",
            params![sid, vt, ts, meta_s],
        )?;
        Ok(json!({
            "session_id": sid,
            "cycle_id": sid,
            "visitor_terminal_id": vt,
            "created_ms": ts,
            "inject_path": inject_path,
            "meta": meta_v,
            "phase": "active",
            "skip_identity_probe": false,
        }))
    }

    fn ensure_vt_schema(conn: &Connection) -> Result<(), StoreError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS visitor_terminals (
               vt_id TEXT PRIMARY KEY,
               last_complete_ms INTEGER NOT NULL DEFAULT 0,
               cool_until_ms INTEGER NOT NULL DEFAULT 0,
               active_cycle_id TEXT NOT NULL DEFAULT '',
               updated_ms INTEGER NOT NULL,
               meta_json TEXT NOT NULL DEFAULT '{}',
               product_version_last TEXT NOT NULL DEFAULT ''
             );
             CREATE INDEX IF NOT EXISTS idx_sessions_vt ON sessions(visitor_terminal_id);",
        )?;
        // migrate older DBs
        let _ = conn.execute(
            "ALTER TABLE visitor_terminals ADD COLUMN product_version_last TEXT NOT NULL DEFAULT ''",
            [],
        );
        Ok(())
    }

    /// (last_complete_ms, cool_until_ms, active_cycle_id, product_version_last)
    fn load_vt_row(
        conn: &Connection,
        vt: &str,
    ) -> Result<Option<(i64, i64, String, String)>, StoreError> {
        // Prefer full row; fall back if column missing mid-migration.
        let row = conn
            .query_row(
                "SELECT last_complete_ms, cool_until_ms, active_cycle_id,
                        COALESCE(product_version_last,'')
                 FROM visitor_terminals WHERE vt_id=?1",
                params![vt],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional();
        match row {
            Ok(v) => Ok(v),
            Err(_) => {
                let legacy = conn
                    .query_row(
                        "SELECT last_complete_ms, cool_until_ms, active_cycle_id
                         FROM visitor_terminals WHERE vt_id=?1",
                        params![vt],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, String::new())),
                    )
                    .optional()?;
                Ok(legacy)
            }
        }
    }

    fn upsert_vt(
        conn: &Connection,
        vt: &str,
        last_complete_ms: i64,
        cool_until_ms: i64,
        active_cycle_id: &str,
        product_version_last: &str,
    ) -> Result<(), StoreError> {
        let ts = now_ms();
        conn.execute(
            "INSERT INTO visitor_terminals(
               vt_id, last_complete_ms, cool_until_ms, active_cycle_id, updated_ms,
               meta_json, product_version_last)
             VALUES(?1,?2,?3,?4,?5,'{}',?6)
             ON CONFLICT(vt_id) DO UPDATE SET
               last_complete_ms=excluded.last_complete_ms,
               cool_until_ms=excluded.cool_until_ms,
               active_cycle_id=excluded.active_cycle_id,
               updated_ms=excluded.updated_ms,
               product_version_last=excluded.product_version_last",
            params![
                vt,
                last_complete_ms,
                cool_until_ms,
                active_cycle_id,
                ts,
                product_version_last
            ],
        )?;
        Ok(())
    }

    fn cycle_row_meta(
        conn: &Connection,
        cycle_id: &str,
    ) -> Result<Option<(String, i64, i64, Value)>, StoreError> {
        let row = conn
            .query_row(
                "SELECT visitor_terminal_id, created_ms, updated_ms, meta_json
                 FROM sessions WHERE session_id=?1",
                params![cycle_id],
                |r| {
                    let meta_s: String = r.get(3)?;
                    let meta: Value = serde_json::from_str(&meta_s).unwrap_or(json!({}));
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, meta))
                },
            )
            .optional()?;
        Ok(row)
    }

    fn open_cycle(
        &self,
        cycle_id_hint: Option<String>,
        visitor_terminal_id: Option<String>,
        meta: Option<Value>,
    ) -> Result<Value, StoreError> {
        let ts = now_ms();
        let mut meta_v = meta.unwrap_or_else(|| json!({}));
        if !meta_v.is_object() {
            meta_v = json!({});
        }
        let mut conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        Self::ensure_vt_schema(&conn)?;

        // Client vt preferred; server mint only when absent (no-js / gateway path).
        let vt = visitor_terminal_id
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("vt_{}", hex_now()));
        let identity_class = meta_v
            .get("identity_class")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                if meta_v.get("fe").is_some() {
                    "js".into()
                } else {
                    "js".into()
                }
            });
        let current_product_version = product_version_from_meta(&meta_v);
        if let Some(obj) = meta_v.as_object_mut() {
            obj.entry("identity_class")
                .or_insert_with(|| json!(identity_class.clone()));
            obj.entry("cycle_status")
                .or_insert_with(|| json!("active"));
            if !current_product_version.is_empty() {
                obj.entry("product_version")
                    .or_insert_with(|| json!(current_product_version.clone()));
            }
        }

        let vt_row = Self::load_vt_row(&conn, &vt)?;
        let (last_complete_ms, cool_until_ms, active_cycle_id, product_version_last) =
            vt_row.unwrap_or((0, 0, String::new(), String::new()));

        // --- cool: within 24h of last complete **for the same product_version** ---
        // Version change (or missing last stamp) → invalidate cool and re-probe.
        let mut cool_ok = cool_valid_for_product_version(
            cool_until_ms,
            ts,
            &product_version_last,
            &current_product_version,
        );
        // Lab/ops force re-probe (FE ?gr_force=1 / meta.force_identity) — ignore cool.
        let force_identity = meta_v
            .get("force_identity")
            .or_else(|| meta_v.get("force_reprobe"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if force_identity {
            cool_ok = false;
        }
        // After invalidation, use zero cool for the rest of this open (fresh probe).
        let mut cool_until_ms = cool_until_ms;
        let mut reprobe_reason: Option<ReprobeReason> = None;
        if cool_until_ms > ts && !cool_ok {
            reprobe_reason = cool_invalid_reason(
                cool_until_ms,
                ts,
                &product_version_last,
                &current_product_version,
            )
            .or(Some(if product_version_last.is_empty() {
                ReprobeReason::MissingProductVersionLast
            } else {
                ReprobeReason::ProductVersionMismatch
            }));
            // Drop wall-clock cool so subsequent opens don't re-evaluate mismatch noise.
            let _ = Self::upsert_vt(
                &conn,
                &vt,
                last_complete_ms,
                0,
                &active_cycle_id,
                &product_version_last,
            );
            cool_until_ms = 0;
            if let Some(obj) = meta_v.as_object_mut() {
                obj.insert(
                    "cool_invalidated_reason".into(),
                    json!(reprobe_reason.map(|r| r.as_str()).unwrap_or("product_version_mismatch")),
                );
                obj.insert(
                    "product_version_last".into(),
                    json!(product_version_last.clone()),
                );
            }
        } else if cool_ok {
            let cid = if !active_cycle_id.is_empty() {
                active_cycle_id.clone()
            } else {
                cycle_id_hint
                    .clone()
                    .unwrap_or_else(|| format!("cycle_{}", hex_now()))
            };
            // Ensure row exists for bookkeeping
            if Self::cycle_row_meta(&conn, &cid)?.is_none() {
                let meta_s = serde_json::to_string(&meta_v)?;
                conn.execute(
                    "INSERT INTO sessions(session_id, visitor_terminal_id, created_ms, updated_ms, meta_json)
                     VALUES(?1,?2,?3,?3,?4)
                     ON CONFLICT(session_id) DO NOTHING",
                    params![cid, vt, ts, meta_s],
                )?;
            }
            let has_b10: bool = conn
                .query_row(
                    "SELECT 1 FROM probe_batches WHERE session_id=?1 AND batch_id='B10_hw_curves' LIMIT 1",
                    params![cid],
                    |_| Ok(1i32),
                )
                .optional()
                .ok()
                .flatten()
                .is_some();
            // Release before latest_analysis (also takes lock).
            drop(conn);
            let last = self.latest_analysis(&cid).ok().flatten();
            let silicon_ok = has_b10
                || last
                    .as_ref()
                    .map(|v| {
                        let r = v.get("result").cloned().unwrap_or_else(|| v.clone());
                        result_has_silicon(&r)
                    })
                    .unwrap_or(false);
            conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
            if !silicon_ok {
                // fall through — cool without silicon is invalid
                if let Some(obj) = meta_v.as_object_mut() {
                    obj.insert(
                        "cool_invalidated_reason".into(),
                        json!("cool_without_silicon"),
                    );
                    obj.insert("force_identity".into(), json!(true));
                }
                Self::upsert_vt(
                    &conn,
                    &vt,
                    last_complete_ms,
                    0,
                    &active_cycle_id,
                    &product_version_last,
                )?;
            } else {
                return Ok(json!({
                    "session_id": cid,
                    "cycle_id": cid,
                    "visitor_terminal_id": vt,
                    "created_ms": ts,
                    "phase": "cool",
                    "skip_identity_probe": true,
                    "skip_session_probe": true,
                    "cool_until_ms": cool_until_ms,
                    "last_complete_ms": last_complete_ms,
                    "last_identity_result": last,
                    "page_probe_required": true,
                    "product_version": current_product_version,
                    "product_version_last": product_version_last,
                    "meta": meta_v,
                    "policy": {
                        "cycle_cool_ms": cycle_cool_ms(),
                        "cycle_incomplete_ms": cycle_incomplete_ms(),
                        "cool_scoped_by_product_version": true,
                        "cool_requires_silicon": true,
                    }
                }));
            }
        }

        // v57 race: client cycle_id_hint wins over VT active resume (fresh cycle per page).
        // Resume only when bag is incomplete **and** stamped product_version matches current.
        if let Some(hint) = cycle_id_hint.as_ref().filter(|h| !h.is_empty()) {
            if let Some((row_vt, created_ms, _, row_meta)) = Self::cycle_row_meta(&conn, hint)? {
                let status = cycle_status_from_meta(&row_meta);
                let age = ts - created_ms;
                let ver_ok =
                    cycle_compatible_with_product_version(&row_meta, &current_product_version);
                if status == "complete" {
                    reprobe_reason = Some(ReprobeReason::CycleCompleteSuperseded);
                } else if !ver_ok {
                    reprobe_reason = Some(ReprobeReason::CycleVersionMismatch);
                } else if age >= cycle_incomplete_ms() {
                    reprobe_reason = Some(ReprobeReason::CycleIncompleteExpired);
                } else if (row_vt == vt || row_vt.is_empty()) && age < cycle_incomplete_ms() {
                    if let Some(obj) = meta_v.as_object_mut() {
                        if let Some(rm) = row_meta.as_object() {
                            for (k, v) in rm {
                                obj.entry(k.clone()).or_insert_with(|| v.clone());
                            }
                        }
                        obj.insert("cycle_status".into(), json!("active"));
                        if !current_product_version.is_empty() {
                            obj.insert(
                                "product_version".into(),
                                json!(current_product_version.clone()),
                            );
                        }
                    }
                    let meta_s = serde_json::to_string(&meta_v)?;
                    conn.execute(
                        "UPDATE sessions SET updated_ms=?1, visitor_terminal_id=?2, meta_json=?3
                         WHERE session_id=?4",
                        params![ts, vt, meta_s, hint],
                    )?;
                    Self::upsert_vt(
                        &conn,
                        &vt,
                        last_complete_ms,
                        cool_until_ms,
                        hint,
                        &product_version_last,
                    )?;
                    drop(conn);
                    return Ok(json!({
                        "session_id": hint,
                        "cycle_id": hint,
                        "visitor_terminal_id": vt,
                        "created_ms": created_ms,
                        "phase": "active",
                        "skip_identity_probe": false,
                        "force_identity_probe": true,
                        "resumed": true,
                        "cycle_expires_ms": created_ms + cycle_incomplete_ms(),
                        "product_version": current_product_version,
                        "product_version_last": product_version_last,
                        "meta": meta_v,
                        "policy": {
                            "cycle_cool_ms": cycle_cool_ms(),
                            "cycle_incomplete_ms": cycle_incomplete_ms(),
                            "cool_scoped_by_product_version": true,
                        }
                    }));
                }
                // Unresumable sticky hint (complete / wrong version / expired): mint fresh
                // under a NEW id and tell FE to adopt it (no user cookie clear required).
                let cid = format!("cycle_{}", hex_now());
                if let Some(obj) = meta_v.as_object_mut() {
                    obj.insert("cycle_status".into(), json!("active"));
                    obj.insert("cycle_id".into(), json!(cid.clone()));
                    if let Some(r) = reprobe_reason {
                        obj.insert("client_hint_superseded_reason".into(), json!(r.as_str()));
                        obj.insert("superseded_cycle_id".into(), json!(hint.clone()));
                    }
                }
                let meta_s = serde_json::to_string(&meta_v)?;
                conn.execute(
                    "INSERT INTO sessions(session_id, visitor_terminal_id, created_ms, updated_ms, meta_json)
                     VALUES(?1,?2,?3,?3,?4)",
                    params![cid, vt, ts, meta_s],
                )?;
                Self::upsert_vt(
                    &conn,
                    &vt,
                    last_complete_ms,
                    0,
                    &cid,
                    &product_version_last,
                )?;
                drop(conn);
                return Ok(json!({
                    "session_id": cid,
                    "cycle_id": cid,
                    "visitor_terminal_id": vt,
                    "created_ms": ts,
                    "phase": "new",
                    "skip_identity_probe": false,
                    "skip_session_probe": false,
                    "force_identity_probe": true,
                    "cycle_expires_ms": ts + cycle_incomplete_ms(),
                    "resumed": false,
                    "client_hint_honored": false,
                    "client_hint_superseded": true,
                    "superseded_cycle_id": hint,
                    "reprobe_reason": reprobe_reason.map(|r| r.as_str()),
                    "product_version": current_product_version,
                    "product_version_last": product_version_last,
                    "meta": meta_v,
                    "policy": {
                        "cycle_cool_ms": cycle_cool_ms(),
                        "cycle_incomplete_ms": cycle_incomplete_ms(),
                        "cool_scoped_by_product_version": true,
                    }
                }));
            } else {
                let cid = hint.clone();
                if let Some(obj) = meta_v.as_object_mut() {
                    obj.insert("cycle_status".into(), json!("active"));
                    obj.insert("cycle_id".into(), json!(cid.clone()));
                }
                let meta_s = serde_json::to_string(&meta_v)?;
                conn.execute(
                    "INSERT INTO sessions(session_id, visitor_terminal_id, created_ms, updated_ms, meta_json)
                     VALUES(?1,?2,?3,?3,?4)",
                    params![cid, vt, ts, meta_s],
                )?;
                Self::upsert_vt(
                    &conn,
                    &vt,
                    last_complete_ms,
                    0,
                    &cid,
                    &product_version_last,
                )?;
                drop(conn);
                return Ok(json!({
                    "session_id": cid,
                    "cycle_id": cid,
                    "visitor_terminal_id": vt,
                    "created_ms": ts,
                    "phase": "new",
                    "skip_identity_probe": false,
                    "skip_session_probe": false,
                    "force_identity_probe": true,
                    "cycle_expires_ms": ts + cycle_incomplete_ms(),
                    "resumed": false,
                    "client_hint_honored": true,
                    "reprobe_reason": reprobe_reason.map(|r| r.as_str()),
                    "product_version": current_product_version,
                    "product_version_last": product_version_last,
                    "meta": meta_v,
                    "policy": {
                        "cycle_cool_ms": cycle_cool_ms(),
                        "cycle_incomplete_ms": cycle_incomplete_ms(),
                        "cool_scoped_by_product_version": true,
                    }
                }));
            }
        }

        // No client hint: resume VT active incomplete cycle only if version-compatible.
        if !active_cycle_id.is_empty() {
            if let Some((row_vt, created_ms, _upd, row_meta)) =
                Self::cycle_row_meta(&conn, &active_cycle_id)?
            {
                let status = cycle_status_from_meta(&row_meta).to_string();
                let age = ts - created_ms;
                let ver_ok =
                    cycle_compatible_with_product_version(&row_meta, &current_product_version);
                if status == "complete" {
                    // fall through
                } else if age >= cycle_incomplete_ms() {
                    drop(conn);
                    let _ = self.purge_cycle_evidence(&active_cycle_id);
                    let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
                    Self::upsert_vt(&conn, &vt, last_complete_ms, 0, "", &product_version_last)?;
                    drop(conn);
                    return self.open_cycle(None, Some(vt), Some(meta_v));
                } else if !ver_ok {
                    // Active bag under old product_version — abandon, open fresh.
                    Self::upsert_vt(&conn, &vt, last_complete_ms, 0, "", &product_version_last)?;
                    // fall through to new cycle
                } else if status == "active" && (row_vt == vt || row_vt.is_empty()) {
                    // Heal sticky incomplete cycles that already have commercial final analysis.
                    drop(conn);
                    if let Ok(Some(last)) = self.latest_analysis(&active_cycle_id) {
                        let r = last.get("result").cloned().unwrap_or_else(|| last.clone());
                        if analysis_completes_cycle(&r) {
                            let _ = self.complete_cycle(&active_cycle_id);
                            // Re-open so client gets cool + last product (no re-probe loop).
                            return self.open_cycle(None, Some(vt), Some(meta_v));
                        }
                    }
                    conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
                    if let Some(obj) = meta_v.as_object_mut() {
                        if let Some(rm) = row_meta.as_object() {
                            for (k, v) in rm {
                                obj.entry(k.clone()).or_insert_with(|| v.clone());
                            }
                        }
                        obj.insert("cycle_status".into(), json!("active"));
                    }
                    let meta_s = serde_json::to_string(&meta_v)?;
                    conn.execute(
                        "UPDATE sessions SET updated_ms=?1, visitor_terminal_id=?2, meta_json=?3
                         WHERE session_id=?4",
                        params![ts, vt, meta_s, active_cycle_id],
                    )?;
                    Self::upsert_vt(
                        &conn,
                        &vt,
                        last_complete_ms,
                        cool_until_ms,
                        &active_cycle_id,
                        &product_version_last,
                    )?;
                    let expires = created_ms + cycle_incomplete_ms();
                    drop(conn);
                    return Ok(json!({
                        "session_id": active_cycle_id,
                        "cycle_id": active_cycle_id,
                        "visitor_terminal_id": vt,
                        "created_ms": created_ms,
                        "phase": "active",
                        "skip_identity_probe": false,
                        "skip_session_probe": false,
                        "force_identity_probe": true,
                        "cycle_expires_ms": expires,
                        "resumed": true,
                        "product_version": current_product_version,
                        "product_version_last": product_version_last,
                        "meta": meta_v,
                        "policy": {
                            "cycle_cool_ms": cycle_cool_ms(),
                            "cycle_incomplete_ms": cycle_incomplete_ms(),
                            "cool_scoped_by_product_version": true,
                        }
                    }));
                }
            }
        }

        // --- new cycle (no client hint) ---
        let cid = format!("cycle_{}", hex_now());
        if let Some(obj) = meta_v.as_object_mut() {
            obj.insert("cycle_status".into(), json!("active"));
            obj.insert("cycle_id".into(), json!(cid.clone()));
        }
        let meta_s = serde_json::to_string(&meta_v)?;
        conn.execute(
            "INSERT INTO sessions(session_id, visitor_terminal_id, created_ms, updated_ms, meta_json)
             VALUES(?1,?2,?3,?3,?4)",
            params![cid, vt, ts, meta_s],
        )?;
        Self::upsert_vt(
            &conn,
            &vt,
            last_complete_ms,
            0,
            &cid,
            &product_version_last,
        )?;
        drop(conn);
        Ok(json!({
            "session_id": cid,
            "cycle_id": cid,
            "visitor_terminal_id": vt,
            "created_ms": ts,
            "phase": "new",
            "skip_identity_probe": false,
            "skip_session_probe": false,
            "force_identity_probe": true,
            "cycle_expires_ms": ts + cycle_incomplete_ms(),
            "resumed": false,
            "reprobe_reason": reprobe_reason.map(|r| r.as_str()),
            "product_version": current_product_version,
            "product_version_last": product_version_last,
            "meta": meta_v,
            "policy": {
                "cycle_cool_ms": cycle_cool_ms(),
                "cycle_incomplete_ms": cycle_incomplete_ms(),
                "cool_scoped_by_product_version": true,
            }
        }))
    }

    fn complete_cycle(&self, cycle_id: &str) -> Result<Value, StoreError> {
        let ts = now_ms();
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        Self::ensure_vt_schema(&conn)?;
        let row = Self::cycle_row_meta(&conn, cycle_id)?
            .ok_or_else(|| StoreError::NotFound(format!("cycle {cycle_id}")))?;
        let (vt, created_ms, _upd, mut meta) = row;
        // Prefer analysis_results.product_version (same conn), then session meta / env.
        let mut product_version = conn
            .query_row(
                "SELECT product_version FROM analysis_results
                 WHERE session_id=?1 AND product_version IS NOT NULL AND product_version <> ''
                 ORDER BY created_ms DESC LIMIT 1",
                params![cycle_id],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten()
            .unwrap_or_default();
        if product_version.is_empty() {
            product_version = product_version_from_meta(&meta);
        }
        if product_version.is_empty() {
            product_version = gr_abi::env::get("PRODUCT_VERSION").unwrap_or_default();
        }
        let has_b10: bool = conn
            .query_row(
                "SELECT 1 FROM probe_batches WHERE session_id=?1 AND batch_id='B10_hw_curves' LIMIT 1",
                params![cycle_id],
                |_| Ok(1i32),
            )
            .optional()
            .ok()
            .flatten()
            .is_some();
        // silicon from latest result_json without re-locking via latest_analysis
        let last_json: Option<String> = conn
            .query_row(
                "SELECT result_json FROM analysis_results WHERE session_id=?1 ORDER BY rev DESC LIMIT 1",
                params![cycle_id],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten();
        let silicon_ok = has_b10
            || last_json
                .as_ref()
                .and_then(|s| decode_analysis_result_json(s).ok())
                .map(|v| {
                    let r = v.get("result").cloned().unwrap_or(v);
                    result_has_silicon(&r) || result_commercial_identity_final(&r)
                })
                .unwrap_or(false);
        let site_for_cool = meta
            .get("site_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let cool_until = if silicon_ok {
            ts + cycle_cool_ms_for_site(&site_for_cool)
        } else {
            0
        };
        if let Some(obj) = meta.as_object_mut() {
            obj.insert("cycle_status".into(), json!("complete"));
            obj.insert("completed_ms".into(), json!(ts));
            obj.insert("skip_session_probe".into(), json!(silicon_ok));
            obj.insert("cool_silicon_ok".into(), json!(silicon_ok));
            if !silicon_ok {
                obj.insert("cool_denied_reason".into(), json!("complete_without_silicon"));
            }
            if !product_version.is_empty() {
                obj.insert("product_version".into(), json!(product_version.clone()));
            }
        }
        let meta_s = serde_json::to_string(&meta)?;
        conn.execute(
            "UPDATE sessions SET updated_ms=?1, meta_json=?2 WHERE session_id=?3",
            params![ts, meta_s, cycle_id],
        )?;
        // cancel pending analyze jobs — identity done
        let _ = conn.execute(
            "DELETE FROM analyze_jobs WHERE session_id=?1",
            params![cycle_id],
        );
        Self::upsert_vt(
            &conn,
            &vt,
            ts,
            cool_until,
            cycle_id,
            &product_version,
        )?;
        Ok(json!({
            "ok": true,
            "cycle_id": cycle_id,
            "session_id": cycle_id,
            "visitor_terminal_id": vt,
            "phase": "complete",
            "completed_ms": ts,
            "cool_until_ms": cool_until,
            "cool_silicon_ok": silicon_ok,
            "created_ms": created_ms,
            "cycle_cool_ms": cycle_cool_ms(),
            "product_version": product_version,
        }))
    }

    fn purge_cycle_evidence(&self, cycle_id: &str) -> Result<Value, StoreError> {
        let ts = now_ms();
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        Self::ensure_vt_schema(&conn)?;
        let row = Self::cycle_row_meta(&conn, cycle_id)?;
        let vt = row.as_ref().map(|r| r.0.clone()).unwrap_or_default();
        let n_batches = conn.execute(
            "DELETE FROM probe_batches WHERE session_id=?1",
            params![cycle_id],
        )?;
        let n_jobs = conn.execute(
            "DELETE FROM analyze_jobs WHERE session_id=?1",
            params![cycle_id],
        )?;
        // Keep final analysis for history if any; delete all revs for incomplete purge
        let n_analysis = conn.execute(
            "DELETE FROM analysis_results WHERE session_id=?1",
            params![cycle_id],
        )?;
        if let Some((_, _, _, mut meta)) = row {
            if let Some(obj) = meta.as_object_mut() {
                obj.insert("cycle_status".into(), json!("purged"));
                obj.insert("purged_ms".into(), json!(ts));
            }
            let meta_s = serde_json::to_string(&meta)?;
            conn.execute(
                "UPDATE sessions SET updated_ms=?1, meta_json=?2 WHERE session_id=?3",
                params![ts, meta_s, cycle_id],
            )?;
        }
        if !vt.is_empty() {
            if let Some((lc, cu, active, pvl)) = Self::load_vt_row(&conn, &vt)? {
                if active == cycle_id {
                    Self::upsert_vt(&conn, &vt, lc, cu, "", &pvl)?;
                }
            }
        }
        Ok(json!({
            "ok": true,
            "cycle_id": cycle_id,
            "purged_batches": n_batches,
            "purged_jobs": n_jobs,
            "purged_analyses": n_analysis,
            "purged_ms": ts,
        }))
    }

    fn require_active_session(&self, session_id: &str) -> Result<Value, StoreError> {
        let w = self.session_window(session_id)?;
        let active = w.get("active").and_then(|v| v.as_bool()).unwrap_or(false);
        if !active {
            let reason = w
                .get("expired_reason")
                .and_then(|v| v.as_str())
                .unwrap_or("expired");
            return Err(StoreError::SessionExpired(format!(
                "{session_id} ({reason})"
            )));
        }
        Ok(w)
    }

    fn upsert_batch(
        &self,
        session_id: &str,
        batch_id: &str,
        source: &str,
        payload: &Value,
    ) -> Result<Value, StoreError> {
        self.require_active_session(session_id)?;
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let exists: Option<String> = conn
            .query_row(
                "SELECT session_id FROM sessions WHERE session_id=?1",
                params![session_id],
                |r| r.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Err(StoreError::NotFound(format!("session {session_id}")));
        }
        let ts = now_ms();
        // Stamp client capture identity into payload for retry/duplicate detection.
        let mut payload_owned = payload.clone();
        if let Some(obj) = payload_owned.as_object_mut() {
            if let Some(cid) = obj.get("capture_id").cloned() {
                let fields = obj.entry("fields".to_string()).or_insert_with(|| json!({}));
                if let Some(fo) = fields.as_object_mut() {
                    fo.entry("capture_id".to_string()).or_insert(cid);
                }
            }
            if let Some(ph) = obj.get("payload_hash").cloned() {
                let fields = obj.entry("fields".to_string()).or_insert_with(|| json!({}));
                if let Some(fo) = fields.as_object_mut() {
                    fo.entry("client_payload_hash".to_string()).or_insert(ph);
                }
            }
            if let Some(gen) = obj.get("material_generation").cloned() {
                let fields = obj.entry("fields".to_string()).or_insert_with(|| json!({}));
                if let Some(fo) = fields.as_object_mut() {
                    fo.entry("material_generation".to_string()).or_insert(gen);
                }
            }
        }
        // Also accept capture fields already nested under fields from FE body envelope.
        let client_cap = payload_owned
            .pointer("/fields/capture_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                payload
                    .get("capture_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            });
        let client_ph = payload_owned
            .pointer("/fields/client_payload_hash")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                payload
                    .get("payload_hash")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            });
        let client_gen: i64 = payload_owned
            .pointer("/fields/material_generation")
            .or_else(|| payload_owned.get("material_generation"))
            .and_then(|v| v.as_i64())
            .or_else(|| payload.get("material_generation").and_then(|v| v.as_i64()))
            .unwrap_or(1)
            .max(1);
        let payload_s = serde_json::to_string(&payload_owned)?;
        // Pre-read for iss/70 ACK: stored vs duplicate vs update (+ generation columns).
        let prev: Option<(String, i64)> = conn
            .query_row(
                "SELECT payload_json, COALESCE(material_generation, 1) FROM probe_batches
                 WHERE session_id=?1 AND batch_id=?2 AND source=?3",
                params![session_id, batch_id, source],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .or_else(|_| {
                // Pre-migration DBs may lack material_generation column.
                conn.query_row(
                    "SELECT payload_json FROM probe_batches
                     WHERE session_id=?1 AND batch_id=?2 AND source=?3",
                    params![session_id, batch_id, source],
                    |r| Ok((r.get::<_, String>(0)?, 1i64)),
                )
                .optional()
            })?;
        let was_insert = prev.is_none();
        let prev_gen = prev.as_ref().map(|p| p.1).unwrap_or(0);
        let prev_payload = prev.as_ref().map(|p| p.0.clone());
        let unchanged = prev_payload.as_ref().map(|p| p == &payload_s).unwrap_or(false);
        let mut same_capture = false;
        if let Some(ref prev_s) = prev_payload {
            if let (Ok(prev_v), Some(ref cap)) = (
                serde_json::from_str::<Value>(prev_s),
                client_cap.as_ref(),
            ) {
                let prev_cap = prev_v
                    .pointer("/fields/capture_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !prev_cap.is_empty() && prev_cap == cap.as_str() {
                    same_capture = true;
                }
            }
            if !same_capture {
                if let (Ok(prev_v), Some(ref ph)) = (
                    serde_json::from_str::<Value>(prev_s),
                    client_ph.as_ref(),
                ) {
                    let prev_ph = prev_v
                        .pointer("/fields/client_payload_hash")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if !prev_ph.is_empty() && prev_ph == ph.as_str() {
                        same_capture = true;
                    }
                }
            }
        }
        // Stale generation: client recollect older than stored → conflict (do not clobber).
        if !was_insert && !same_capture && prev_gen > 0 && client_gen < prev_gen {
            drop(conn);
            return Ok(json!({
                "ok": false,
                "conflict": true,
                "error": "stale_material_generation",
                "session_id": session_id,
                "batch_id": batch_id,
                "source": source,
                "created_ms": ts,
                "was_insert": false,
                "unchanged": false,
                "same_capture": false,
                "material_generation": prev_gen,
                "client_material_generation": client_gen,
                "rows_affected": 0,
                "durability_state": "rejected",
                "cold_written": false,
            }));
        }
        // Transport retry of same capture: do not rewrite payload (preserve first material).
        if same_capture && !was_insert {
            let touch_iv = session_touch_min_interval_ms();
            let _ = conn.execute(
                "UPDATE sessions SET updated_ms=?1
                 WHERE session_id=?2
                   AND (updated_ms IS NULL OR (?1 - updated_ms) >= ?3)",
                params![ts, session_id, touch_iv],
            );
            drop(conn);
            return Ok(json!({
                "ok": true,
                "session_id": session_id,
                "batch_id": batch_id,
                "source": source,
                "created_ms": ts,
                "analyze_scheduled": false,
                "analyze_deferred": true,
                "analyze_queue_full": false,
                "analyze_debounce_ms": 0,
                "analyze_policy": "brain_owned_v1",
                "per_batch_analyze": false,
                "was_insert": false,
                "unchanged": true,
                "same_capture": true,
                "material_generation": prev_gen.max(client_gen),
                "material_hash": client_ph,
                "capture_id": client_cap,
                "rows_affected": 0,
                "cold_skipped_unchanged": true,
                "merged": payload.get("merged").cloned().unwrap_or(Value::Bool(false)),
                "durability_state": "duplicate_durable",
                "cold_written": true,
            }));
        }
        let mat_hash = client_ph.clone();
        let cap_id = client_cap.clone();
        // Prefer full-column insert; fall back if columns not yet migrated.
        let insert_res = conn.execute(
            "INSERT INTO probe_batches(session_id, batch_id, source, payload_json, created_ms,
                material_generation, material_hash, capture_id)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT(session_id, batch_id, source) DO UPDATE SET
               payload_json=excluded.payload_json,
               created_ms=excluded.created_ms,
               material_generation=excluded.material_generation,
               material_hash=excluded.material_hash,
               capture_id=excluded.capture_id
             WHERE probe_batches.payload_json IS NOT excluded.payload_json
                OR COALESCE(probe_batches.material_generation,1) < excluded.material_generation",
            params![
                session_id,
                batch_id,
                source,
                payload_s,
                ts,
                client_gen,
                mat_hash,
                cap_id
            ],
        );
        if insert_res.is_err() {
            conn.execute(
                "INSERT INTO probe_batches(session_id, batch_id, source, payload_json, created_ms)
                 VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(session_id, batch_id, source) DO UPDATE SET
                   payload_json=excluded.payload_json,
                   created_ms=excluded.created_ms
                 WHERE probe_batches.payload_json IS NOT excluded.payload_json",
                params![session_id, batch_id, source, payload_s, ts],
            )?;
        }
        let rows = conn.changes();
        let touch_iv = session_touch_min_interval_ms();
        conn.execute(
            "UPDATE sessions SET updated_ms=?1
             WHERE session_id=?2
               AND (updated_ms IS NULL OR (?1 - updated_ms) >= ?3)",
            params![ts, session_id, touch_iv],
        )?;
        drop(conn);
        // v5.8.105: do **not** schedule analyze on every batch.
        // Brain-owned arms (coverage / idle 60s / no-result 180s) are applied by the
        // service layer after ingest via `schedule_analyze` with computed debounce.
        Ok(json!({
            "ok": true,
            "session_id": session_id,
            "batch_id": batch_id,
            "source": source,
            "created_ms": ts,
            "analyze_scheduled": false,
            "analyze_deferred": true,
            "analyze_queue_full": false,
            "analyze_debounce_ms": 0,
            "analyze_policy": "brain_owned_v1",
            "per_batch_analyze": false,
            "was_insert": was_insert,
            "unchanged": unchanged || (!was_insert && rows == 0),
            "same_capture": same_capture,
            "material_generation": client_gen,
            "material_hash": client_ph,
            "capture_id": client_cap,
            "rows_affected": rows,
            "cold_skipped_unchanged": unchanged || (!was_insert && rows == 0),
            "merged": payload.get("merged").cloned().unwrap_or(Value::Bool(false)),
            "durability_state": if same_capture {
                "duplicate_durable"
            } else {
                "stored_durable"
            },
            "cold_written": true,
        }))
    }

    fn retention_purge_batch(
        &self,
        older_than_ms: i64,
        limit: i64,
        velocity_older_than_ms: Option<i64>,
        ops_older_than_ms: Option<i64>,
        master_older_than_ms: Option<i64>,
    ) -> Result<Value, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let lim = limit.clamp(10, 5000);
        // Oldest analysis revs
        let n_analysis = conn
            .execute(
                r#"
                DELETE FROM analysis_results WHERE rowid IN (
                  SELECT rowid FROM analysis_results
                  WHERE created_ms < ?1
                  ORDER BY created_ms ASC
                  LIMIT ?2
                )
                "#,
                params![older_than_ms, lim],
            )
            .unwrap_or(0) as i64;
        let n_sessions = conn
            .execute(
                r#"
                DELETE FROM sessions WHERE session_id IN (
                  SELECT s.session_id FROM sessions s
                  WHERE s.updated_ms < ?1
                    AND NOT EXISTS (
                      SELECT 1 FROM analysis_results ar WHERE ar.session_id = s.session_id
                    )
                  ORDER BY s.updated_ms ASC
                  LIMIT ?2
                )
                "#,
                params![older_than_ms, lim],
            )
            .unwrap_or(0) as i64;
        let n_batches = conn
            .execute(
                r#"
                DELETE FROM probe_batches WHERE rowid IN (
                  SELECT pb.rowid FROM probe_batches pb
                  WHERE NOT EXISTS (SELECT 1 FROM sessions s WHERE s.session_id = pb.session_id)
                  LIMIT ?1
                )
                "#,
                params![lim],
            )
            .unwrap_or(0) as i64;
        let mut n_velocity = 0i64;
        if let Some(v_old) = velocity_older_than_ms {
            let _ = conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS velocity_hits (
                  id INTEGER PRIMARY KEY AUTOINCREMENT,
                  key_kind TEXT NOT NULL,
                  key_value TEXT NOT NULL,
                  session_id TEXT,
                  hit_ms INTEGER NOT NULL
                );",
            );
            n_velocity = conn
                .execute(
                    r#"
                    DELETE FROM velocity_hits WHERE rowid IN (
                      SELECT rowid FROM velocity_hits WHERE hit_ms < ?1 ORDER BY hit_ms ASC LIMIT ?2
                    )
                    "#,
                    params![v_old, lim],
                )
                .unwrap_or(0) as i64;
        }
        // iss/opus5 04-P1-6: previously unbounded tables.
        // page_results follow the analysis window (same lifecycle).
        let n_page = conn
            .execute(
                r#"
                DELETE FROM page_results WHERE rowid IN (
                  SELECT rowid FROM page_results
                  WHERE created_ms < ?1 ORDER BY created_ms ASC LIMIT ?2
                )
                "#,
                params![older_than_ms, lim],
            )
            .unwrap_or(0) as i64;
        let mut n_obs = 0i64;
        let mut n_ops_client = 0i64;
        let mut n_ops_server = 0i64;
        let mut n_idem = 0i64;
        if let Some(ops_old) = ops_older_than_ms {
            n_obs = conn
                .execute(
                    r#"
                    DELETE FROM observation_events WHERE rowid IN (
                      SELECT rowid FROM observation_events
                      WHERE created_ms < ?1 ORDER BY created_ms ASC LIMIT ?2
                    )
                    "#,
                    params![ops_old, lim],
                )
                .unwrap_or(0) as i64;
            n_ops_client = conn
                .execute(
                    r#"
                    DELETE FROM ops_client_events WHERE rowid IN (
                      SELECT rowid FROM ops_client_events
                      WHERE ts_ms < ?1 ORDER BY ts_ms ASC LIMIT ?2
                    )
                    "#,
                    params![ops_old, lim],
                )
                .unwrap_or(0) as i64;
            n_ops_server = conn
                .execute(
                    r#"
                    DELETE FROM ops_server_events WHERE rowid IN (
                      SELECT rowid FROM ops_server_events
                      WHERE ts_ms < ?1 ORDER BY ts_ms ASC LIMIT ?2
                    )
                    "#,
                    params![ops_old, lim],
                )
                .unwrap_or(0) as i64;
            n_idem = conn
                .execute(
                    r#"
                    DELETE FROM api_idempotency WHERE rowid IN (
                      SELECT rowid FROM api_idempotency
                      WHERE created_ms < ?1 ORDER BY created_ms ASC LIMIT ?2
                    )
                    "#,
                    params![ops_old, lim],
                )
                .unwrap_or(0) as i64;
        }
        let mut n_soft_edges = 0i64;
        let mut n_soft_heat = 0i64;
        let mut n_di_devices = 0i64;
        let mut n_di_keys = 0i64;
        if let Some(master_old) = master_older_than_ms {
            n_soft_edges = conn
                .execute(
                    r#"
                    DELETE FROM soft_edges WHERE rowid IN (
                      SELECT rowid FROM soft_edges
                      WHERE created_ms < ?1 ORDER BY created_ms ASC LIMIT ?2
                    )
                    "#,
                    params![master_old, lim],
                )
                .unwrap_or(0) as i64;
            n_soft_heat = conn
                .execute(
                    r#"
                    DELETE FROM soft_heat WHERE rowid IN (
                      SELECT rowid FROM soft_heat
                      WHERE last_ms < ?1 ORDER BY last_ms ASC LIMIT ?2
                    )
                    "#,
                    params![master_old, lim],
                )
                .unwrap_or(0) as i64;
            n_di_devices = conn
                .execute(
                    r#"
                    DELETE FROM device_index_devices WHERE rowid IN (
                      SELECT rowid FROM device_index_devices
                      WHERE updated_ms < ?1 ORDER BY updated_ms ASC LIMIT ?2
                    )
                    "#,
                    params![master_old, lim],
                )
                .unwrap_or(0) as i64;
            // device_index_keys has no timestamp: drop keys whose device left the index.
            n_di_keys = conn
                .execute(
                    r#"
                    DELETE FROM device_index_keys WHERE rowid IN (
                      SELECT k.rowid FROM device_index_keys k
                      WHERE NOT EXISTS (
                        SELECT 1 FROM device_index_devices d
                        WHERE d.tenant_id = k.tenant_id AND d.device_id = k.device_id
                      )
                      LIMIT ?1
                    )
                    "#,
                    params![lim],
                )
                .unwrap_or(0) as i64;
        }
        Ok(json!({
            "ok": true,
            "backend": "sqlite",
            "deleted_analysis": n_analysis,
            "deleted_sessions": n_sessions,
            "deleted_batches": n_batches,
            "deleted_velocity": n_velocity,
            "deleted_page_results": n_page,
            "deleted_observation_events": n_obs,
            "deleted_ops_client_events": n_ops_client,
            "deleted_ops_server_events": n_ops_server,
            "deleted_api_idempotency": n_idem,
            "deleted_soft_edges": n_soft_edges,
            "deleted_soft_heat": n_soft_heat,
            "deleted_device_index_devices": n_di_devices,
            "deleted_device_index_keys": n_di_keys,
            "limit": lim,
            "older_than_ms": older_than_ms,
        }))
    }

    fn schedule_analyze(&self, session_id: &str, debounce_ms: i64) -> Result<(), StoreError> {
        self.schedule_analyze_merge(session_id, debounce_ms, AnalyzeDueMerge::PullEarlier)
    }

    fn schedule_analyze_merge(
        &self,
        session_id: &str,
        debounce_ms: i64,
        merge: AnalyzeDueMerge,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let now = now_ms();
        let pending: i64 = conn.query_row(
            "SELECT COUNT(*) FROM analyze_jobs WHERE status='pending'",
            [],
            |r| r.get(0),
        )?;
        let already: bool = conn
            .query_row(
                "SELECT 1 FROM analyze_jobs WHERE session_id=?1 AND status='pending'",
                params![session_id],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        if !already && pending >= analyze_queue_max() {
            return Err(StoreError::Msg(format!(
                "analyze_queue_full pending={pending} max={}",
                analyze_queue_max()
            )));
        }
        let new_due = now + debounce_ms.max(0);
        let existing: Option<i64> = conn
            .query_row(
                "SELECT due_ms FROM analyze_jobs WHERE session_id=?1 AND status='pending'",
                params![session_id],
                |r| r.get(0),
            )
            .optional()?;
        let due = match (merge, existing) {
            (_, None) => new_due,
            (AnalyzeDueMerge::Replace, _) => new_due,
            (AnalyzeDueMerge::PullEarlier, Some(ex)) => ex.min(new_due),
            (AnalyzeDueMerge::IdleReset, Some(ex)) => {
                // Reset quiet window, but never delay an imminent/sooner arm (milestone).
                if ex <= now + ANALYZE_IDLE_IMMINENT_MS {
                    ex
                } else {
                    new_due
                }
            }
        };
        conn.execute(
            "INSERT INTO analyze_jobs(session_id, due_ms, status, locked_until, locked_by, updated_ms)
             VALUES(?1, ?2, 'pending', 0, '', ?3)
             ON CONFLICT(session_id) DO UPDATE SET
               due_ms=excluded.due_ms,
               status='pending',
               updated_ms=excluded.updated_ms
             WHERE analyze_jobs.status IS NOT 'pending'
                OR analyze_jobs.due_ms IS NOT excluded.due_ms",
            params![session_id, due, now],
        )?;
        Ok(())
    }

    fn claim_due_analyze_jobs(
        &self,
        worker_id: &str,
        limit: usize,
        lock_ms: i64,
    ) -> Result<Vec<String>, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let now = now_ms();
        let mut stmt = conn.prepare(
            "SELECT session_id FROM analyze_jobs
             WHERE status='pending' AND due_ms<=?1 AND locked_until<?1
             ORDER BY due_ms ASC
             LIMIT ?2",
        )?;
        let candidates: Vec<String> = stmt
            .query_map(params![now, limit as i64], |r| r.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        let mut claimed = Vec::new();
        let until = now + lock_ms.max(1000);
        for sid in candidates {
            let n = conn.execute(
                "UPDATE analyze_jobs SET locked_until=?1, locked_by=?2, updated_ms=?3
                 WHERE session_id=?4 AND status='pending' AND due_ms<=?3 AND locked_until<?3",
                params![until, worker_id, now, sid],
            )?;
            if n == 1 {
                claimed.push(sid);
            }
        }
        Ok(claimed)
    }

    fn complete_analyze_job(&self, session_id: &str, worker_id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let now = now_ms();
        let row: Option<(i64, String)> = conn
            .query_row(
                "SELECT due_ms, locked_by FROM analyze_jobs WHERE session_id=?1",
                params![session_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((due_ms, locked_by)) = row else {
            return Ok(false);
        };
        if locked_by != worker_id && !locked_by.is_empty() {
            return Ok(false);
        }
        if due_ms > now {
            conn.execute(
                "UPDATE analyze_jobs SET locked_until=0, locked_by='', status='pending', updated_ms=?1
                 WHERE session_id=?2",
                params![now, session_id],
            )?;
            return Ok(true);
        }
        conn.execute(
            "DELETE FROM analyze_jobs WHERE session_id=?1",
            params![session_id],
        )?;
        Ok(false)
    }

    fn pending_analyze_job_count(&self) -> Result<i64, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let n: i64 = conn.query_row(
            "SELECT COUNT(1) FROM analyze_jobs WHERE status='pending'",
            [],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    fn analyze_queue_stats(&self) -> Result<Value, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let now = now_ms();
        let (pending, due_now, locked, oldest_lag_ms): (i64, i64, i64, i64) = conn.query_row(
            "SELECT
               COUNT(1),
               SUM(CASE WHEN due_ms<=?1 AND locked_until<?1 THEN 1 ELSE 0 END),
               SUM(CASE WHEN locked_until>=?1 THEN 1 ELSE 0 END),
               COALESCE(MIN(CASE WHEN due_ms<=?1 THEN (?1 - due_ms) END), 0)
             FROM analyze_jobs WHERE status='pending'",
            params![now],
            |r| Ok((r.get(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0), r.get::<_, Option<i64>>(2)?.unwrap_or(0), r.get(3)?)),
        )?;
        let (idle_min, idle_max) = analyze_idle_poll_ms_range();
        Ok(json!({
            "pending": pending,
            "due_now": due_now,
            "locked": locked,
            "oldest_lag_ms": oldest_lag_ms.max(0),
            "debounce_ms": ANALYZE_DEBOUNCE_MS,
            "lock_ms": ANALYZE_LOCK_MS,
            "claim_batch": ANALYZE_CLAIM_BATCH,
            "idle_poll_ms_min": idle_min,
            "idle_poll_ms_max": idle_max,
            "backend": "sqlite",
        }))
    }

    fn payload_fields(payload: &Value) -> Map<String, Value> {
        let mut out = Map::new();
        if let Some(obj) = payload.as_object() {
            if let Some(f) = obj.get("fields").and_then(|v| v.as_object()) {
                for (k, v) in f {
                    out.insert(k.clone(), v.clone());
                }
            } else {
                for (k, v) in obj {
                    if k != "batch_id" && k != "source" && k != "session_id" && k != "inject_path" && k != "early" {
                        out.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        out
    }

    fn build_evidence(&self, session_id: &str) -> Result<Value, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let meta: Option<(String, String, i64)> = conn
            .query_row(
                "SELECT visitor_terminal_id, meta_json, updated_ms FROM sessions WHERE session_id=?1",
                params![session_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((vt, meta_s, updated_ms)) = meta else {
            return Err(StoreError::NotFound(format!("session {session_id}")));
        };
        let mut stmt = conn.prepare(
            "SELECT batch_id, source, payload_json FROM probe_batches
             WHERE session_id=?1 ORDER BY created_ms ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;

        let mut batches = Vec::new();
        let mut sources = Vec::new();
        let mut fields = Map::new();
        let mut fields_by_source = Map::new();
        let mut gateway_fields = Map::new();
        let mut cf_fields = Map::new();
        fields.insert("session_id".into(), json!(session_id));
        fields.insert("visitor_terminal_id".into(), json!(vt));

        for row in rows {
            let (batch_id, source, payload_s) = row?;
            let base_src = source.split(':').next().unwrap_or(source.as_str()).to_string();
            if !sources.iter().any(|s: &String| s == &source || s == &base_src) {
                sources.push(if source.contains(':') {
                    base_src.clone()
                } else {
                    source.clone()
                });
            }
            batches.push(json!({"batch_id": batch_id, "source": source}));
            let payload: Value = serde_json::from_str(&payload_s)?;
            let pf = Self::payload_fields(&payload);
            evidence_merge::merge_batch_with_id(
                &base_src,
                Some(batch_id.as_str()),
                pf,
                &mut fields,
                &mut fields_by_source,
                &mut gateway_fields,
                &mut cf_fields,
            );
        }

        let source_conflicts = evidence_merge::detect_source_conflicts(&fields_by_source);
        let multi_source_consistency =
            evidence_merge::assess_multi_source_consistency(&fields_by_source);
        // Promote sandbox capability + health into merged fields for product_scores / brain
        if let Some(obj) = multi_source_consistency.as_object() {
            for key in [
                "sandbox_blocked",
                "sandbox_ok",
                "js_ok_sandbox_dead",
                "sandbox_all_empty",
                "sandbox_under_two_kinds",
                "sandbox_thin_vs_main",
                "sandbox_partial",
                "sandbox_capability_score",
                "sandbox_capability_band",
                "sandbox_payload_source_n",
                "sandbox_sources_received_n",
            ] {
                if let Some(v) = obj.get(key).cloned() {
                    fields.entry(key.to_string()).or_insert(v);
                }
            }
            if let Some(r) = obj.get("match_ratio").cloned() {
                fields
                    .entry("multi_source_match_ratio".to_string())
                    .or_insert(r);
            }
        }
        let source_auth_view =
            evidence_merge::build_source_auth_view(&fields_by_source, &source_conflicts);
        let realm_conflict_graph = evidence_merge::structured_realm_diff(&fields_by_source);
        let authentic_fields_for_mint = evidence_merge::authentic_fields_for_mint(
            &fields,
            &fields_by_source,
            &source_conflicts,
        );
        let meta_v: Value = serde_json::from_str(&meta_s).unwrap_or(json!({}));
        let has_gateway = !gateway_fields.is_empty()
            || sources
                .iter()
                .any(|s| s == "gateway" || s.starts_with("gateway"));
        let has_cloudflare = ["bot_score", "country", "cf_ray", "colo", "asn", "cf_connecting_ip", "cf_edge_present"]
            .iter()
            .any(|k| cf_fields.contains_key(*k))
            || sources.iter().any(|s| s == "cloudflare" || s == "cf")
            || fields
                .get("cf_edge_present")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

        // iss/22 P1b: warm-start control plane from session meta (belief / priors / battle)
        let mut meta_out = meta_v.clone();
        let evidence_rev = batches.len() as i64;
        if let Some(obj) = meta_out.as_object_mut() {
            obj.insert("evidence_rev".into(), json!(evidence_rev));
        }

        Ok(json!({
            "session_id": session_id,
            "visitor_terminal_id": vt,
            "sources": sources,
            "batches": batches,
            "fields": fields,
            "fields_by_source": fields_by_source,
            "source_conflicts": source_conflicts,
            "multi_source_consistency": multi_source_consistency,
            "source_auth_view": source_auth_view,
            "realm_conflict_graph": realm_conflict_graph,
            "authentic_fields_for_mint": authentic_fields_for_mint,
            "meta": meta_out,
            // Convenience mirrors for brain (same content as meta.belief / meta.direction_priors)
            "session_meta": meta_out,
            "prior_belief": meta_v.get("belief").cloned().unwrap_or(Value::Null),
            "evidence_rev": evidence_rev,
            "has_gateway": has_gateway,
            "has_cloudflare": has_cloudflare,
            "gateway_fields": gateway_fields,
            "cf_fields": cf_fields,
            "last_upload_ms": updated_ms,
            "updated_ms": updated_ms,
        }))
    }

    fn save_analysis(&self, session_id: &str, result: &Value) -> Result<i64, StoreError> {
        self.require_active_session(session_id)?;
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        // Best-effort migrate existing sqlite files (ignore "duplicate column").
        for ddl in [
            "ALTER TABLE analysis_results ADD COLUMN real_band TEXT",
            "ALTER TABLE analysis_results ADD COLUMN device_id TEXT",
            "ALTER TABLE analysis_results ADD COLUMN bot_verdict TEXT",
            "ALTER TABLE analysis_results ADD COLUMN device_confidence REAL",
            "ALTER TABLE analysis_results ADD COLUMN client_ip TEXT",
            "ALTER TABLE analysis_results ADD COLUMN device_tier TEXT",
            "ALTER TABLE analysis_results ADD COLUMN collision_risk INTEGER",
            "ALTER TABLE analysis_results ADD COLUMN product_version TEXT",
            "ALTER TABLE analysis_results ADD COLUMN digest_path TEXT",
            "ALTER TABLE analysis_results ADD COLUMN residual_entropy_ok INTEGER",
            "ALTER TABLE analysis_results ADD COLUMN site_id TEXT",
            "ALTER TABLE analysis_results ADD COLUMN country TEXT",
            "ALTER TABLE analysis_results ADD COLUMN asn TEXT",
            "ALTER TABLE analysis_results ADD COLUMN network_class TEXT",
        ] {
            let _ = conn.execute(ddl, []);
        }
        let next: i64 = conn.query_row(
            "SELECT COALESCE(MAX(rev), 0) + 1 FROM analysis_results WHERE session_id=?1",
            params![session_id],
            |r| r.get(0),
        )?;
        let ts = now_ms();
        let slim = slim_analysis_result_for_storage(result);
        let s = encode_analysis_result_json(&slim).map_err(StoreError::Msg)?;
        let mut sc = analysis_report_scalars(result);
        // Session meta fallback for site_id (gateway/open may stamp meta before evaluate).
        if sc.site_id.is_none() {
            if let Ok(meta_s) = conn.query_row(
                "SELECT meta_json FROM sessions WHERE session_id=?1",
                params![session_id],
                |r| r.get::<_, String>(0),
            ) {
                if let Ok(meta) = serde_json::from_str::<Value>(&meta_s) {
                    sc.site_id = meta
                        .get("site_id")
                        .or_else(|| meta.get("siteId"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string());
                }
            }
        }
        let coll = sc.collision_risk.map(|b| if b { 1i64 } else { 0 });
        let ent = sc.residual_entropy_ok.map(|b| if b { 1i64 } else { 0 });
        let mut network_class = opt_str_ptr(
            result,
            &[
                "/fields/server_network_class",
                "/gateway_fields/server_network_class",
                "/product/server_network_class",
                "/device/trust/materials/server_network_class",
            ],
        );
        // Fallback: coarse client_ip class so loopback/private still denorm when
        // evaluate slim/materials omitted gateway network tags (no core dep).
        if network_class.is_none() {
            if let Some(ref ip) = sc.client_ip {
                network_class = coarse_network_class(ip);
            }
        }
        conn.execute(
            "INSERT INTO analysis_results(session_id, rev, result_json, created_ms,
                real_band, device_id, bot_verdict, device_confidence, client_ip,
                device_tier, collision_risk, product_version, digest_path, residual_entropy_ok,
                site_id, country, asn, network_class)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
            params![
                session_id,
                next,
                s,
                ts,
                sc.real_band,
                sc.device_id,
                sc.bot_verdict,
                sc.device_confidence,
                sc.client_ip,
                sc.device_tier,
                coll,
                sc.product_version,
                sc.digest_path,
                ent,
                sc.site_id,
                sc.country,
                sc.asn,
                network_class
            ],
        )?;
        let keep = analysis_history_keep();
        if next > keep {
            let min_keep = next - keep;
            let _ = conn.execute(
                "DELETE FROM analysis_results WHERE session_id=?1 AND rev <= ?2",
                params![session_id, min_keep],
            );
        }
        Ok(next)
    }

    fn force_session_times(
        &self,
        session_id: &str,
        created_ms: i64,
        updated_ms: i64,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let n = conn.execute(
            "UPDATE sessions SET created_ms=?1, updated_ms=?2 WHERE session_id=?3",
            params![created_ms, updated_ms, session_id],
        )?;
        if n == 0 {
            return Err(StoreError::NotFound(format!("session {session_id}")));
        }
        Ok(())
    }

    fn session_meta(&self, session_id: &str) -> Result<Option<Value>, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let row: Option<String> = conn
            .query_row(
                "SELECT meta_json FROM sessions WHERE session_id=?1",
                params![session_id],
                |r| r.get(0),
            )
            .optional()?;
        match row {
            Some(s) => Ok(serde_json::from_str(&s).ok()),
            None => Ok(None),
        }
    }

    fn list_observation_events(
        &self,
        session_id: &str,
        limit: i64,
    ) -> Result<Vec<Value>, StoreError> {
        let lim = limit.clamp(1, 1000);
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let mut stmt = conn
            .prepare(
                "SELECT observation_id, batch_id, source_kind, realm_kind,
                        probe_method_id, envelope_json, created_ms
                 FROM observation_events
                 WHERE session_id=?1
                 ORDER BY created_ms ASC
                 LIMIT ?2",
            )
            .map_err(StoreError::Sqlite)?;
        let rows = stmt
            .query_map(params![session_id, lim], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            })
            .map_err(StoreError::Sqlite)?;
        let mut out = Vec::new();
        for row in rows {
            let (oid, batch, sk, rk, pm, env_s, created_ms) = row.map_err(StoreError::Sqlite)?;
            let env = serde_json::from_str(&env_s).unwrap_or(json!({}));
            out.push(json!({
                "observation_id": oid,
                "batch_id": batch,
                "source_kind": sk,
                "realm_kind": rk,
                "probe_method_id": pm,
                "envelope": env,
                "created_ms": created_ms,
            }));
        }
        Ok(out)
    }

    fn latest_analysis(&self, session_id: &str) -> Result<Option<Value>, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let row: Option<(i64, String, i64, Option<String>)> = conn
            .query_row(
                "SELECT rev, result_json, created_ms, site_id FROM analysis_results
                 WHERE session_id=?1 ORDER BY rev DESC LIMIT 1",
                params![session_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        Ok(match row {
            Some((rev, js, created_ms, denorm_site)) => {
                let mut result: Value = decode_analysis_result_json(&js).map_err(StoreError::Msg)?;
                if let Some(obj) = result.as_object_mut() {
                    obj.insert("analysis_rev".into(), json!(rev));
                    obj.insert("analyzed_ms".into(), json!(created_ms));
                    // Ensure panel strategy site map can resolve when JSON body omitted site_id.
                    if let Some(site) = denorm_site
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        let need = !obj
                            .get("meta")
                            .and_then(|m| m.get("site_id"))
                            .and_then(|v| v.as_str())
                            .map(|s| !s.is_empty())
                            .unwrap_or(false)
                            && !obj
                                .get("fields")
                                .and_then(|m| m.get("site_id"))
                                .and_then(|v| v.as_str())
                                .map(|s| !s.is_empty())
                                .unwrap_or(false);
                        if need {
                            let meta = obj
                                .entry("meta".to_string())
                                .or_insert_with(|| json!({}));
                            if let Some(m) = meta.as_object_mut() {
                                m.insert("site_id".into(), json!(site));
                            }
                        }
                    }
                }
                Some(result)
            }
            None => None,
        })
    }

    fn list_analyses(&self, session_id: &str) -> Result<Vec<Value>, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT rev, result_json, created_ms FROM analysis_results
             WHERE session_id=?1 ORDER BY rev ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (rev, js, created_ms) = row?;
            let mut result: Value = decode_analysis_result_json(&js).map_err(StoreError::Msg)?;
            if let Some(obj) = result.as_object_mut() {
                obj.insert("analysis_rev".into(), json!(rev));
                obj.insert("analyzed_ms".into(), json!(created_ms));
            }
            out.push(json!({
                "rev": rev,
                "created_ms": created_ms,
                "real_band": result.get("real_band"),
                "device_id": result.pointer("/device/device_id"),
                "result": result,
            }));
        }
        Ok(out)
    }

    fn save_page_result(
        &self,
        session_id: &str,
        page_id: &str,
        page_rev: i64,
        result: &Value,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        // Ensure table exists for DBs created before page_results migration
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS page_results (
              session_id TEXT NOT NULL,
              page_id TEXT NOT NULL,
              page_rev INTEGER NOT NULL,
              result_json TEXT NOT NULL,
              created_ms INTEGER NOT NULL,
              PRIMARY KEY (session_id, page_id, page_rev)
            );",
        )?;
        let ts = now_ms();
        let s = serde_json::to_string(result)?;
        conn.execute(
            "INSERT INTO page_results(session_id, page_id, page_rev, result_json, created_ms)
             VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(session_id, page_id, page_rev) DO UPDATE SET
               result_json=excluded.result_json,
               created_ms=excluded.created_ms",
            params![session_id, page_id, page_rev, s, ts],
        )?;
        Ok(())
    }

    fn latest_page_result(
        &self,
        session_id: &str,
        page_id: &str,
    ) -> Result<Option<Value>, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let _ = conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS page_results (
              session_id TEXT NOT NULL,
              page_id TEXT NOT NULL,
              page_rev INTEGER NOT NULL,
              result_json TEXT NOT NULL,
              created_ms INTEGER NOT NULL,
              PRIMARY KEY (session_id, page_id, page_rev)
            );",
        );
        let row: Option<(i64, String, i64)> = conn
            .query_row(
                "SELECT page_rev, result_json, created_ms FROM page_results
                 WHERE session_id=?1 AND page_id=?2 ORDER BY page_rev DESC LIMIT 1",
                params![session_id, page_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        Ok(match row {
            Some((rev, js, created_ms)) => {
                let mut result: Value = serde_json::from_str(&js)?;
                if let Some(obj) = result.as_object_mut() {
                    obj.insert("page_rev".into(), json!(rev));
                    obj.insert("analyzed_ms".into(), json!(created_ms));
                }
                Some(result)
            }
            None => None,
        })
    }

    fn list_page_results(&self, session_id: &str) -> Result<Vec<Value>, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let _ = conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS page_results (
              session_id TEXT NOT NULL,
              page_id TEXT NOT NULL,
              page_rev INTEGER NOT NULL,
              result_json TEXT NOT NULL,
              created_ms INTEGER NOT NULL,
              PRIMARY KEY (session_id, page_id, page_rev)
            );",
        );
        let mut stmt = conn.prepare(
            "SELECT page_id, page_rev, result_json, created_ms FROM page_results
             WHERE session_id=?1 ORDER BY created_ms ASC, page_rev ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (page_id, rev, js, created_ms) = row?;
            let result: Value = serde_json::from_str(&js)?;
            out.push(json!({
                "page_id": page_id,
                "page_rev": rev,
                "created_ms": created_ms,
                "result": result,
            }));
        }
        Ok(out)
    }

    fn merge_session_meta(&self, session_id: &str, patch: &Value) -> Result<Value, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let prev: Option<String> = conn
            .query_row(
                "SELECT meta_json FROM sessions WHERE session_id=?1",
                params![session_id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(prev_s) = prev else {
            return Err(StoreError::NotFound(format!("session {session_id}")));
        };
        let mut meta: Value = serde_json::from_str(&prev_s).unwrap_or(json!({}));
        if let (Some(obj), Some(p)) = (meta.as_object_mut(), patch.as_object()) {
            for (k, v) in p {
                obj.insert(k.clone(), v.clone());
            }
        }
        let ts = now_ms();
        let meta_s = serde_json::to_string(&meta)?;
        conn.execute(
            "UPDATE sessions SET meta_json=?1, updated_ms=?2 WHERE session_id=?3",
            params![meta_s, ts, session_id],
        )?;
        Ok(meta)
    }

    fn device_index_upsert(
        &self,
        tenant_id: &str,
        device_id: &str,
        binder_obs: &Value,
        binder_keys: &[String],
    ) -> Result<Value, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS device_index_devices (
              tenant_id TEXT NOT NULL,
              device_id TEXT NOT NULL,
              binder_obs_json TEXT NOT NULL,
              updated_ms INTEGER NOT NULL,
              PRIMARY KEY (tenant_id, device_id)
            );
            CREATE TABLE IF NOT EXISTS device_index_keys (
              tenant_id TEXT NOT NULL,
              binder_key TEXT NOT NULL,
              device_id TEXT NOT NULL,
              PRIMARY KEY (tenant_id, binder_key, device_id)
            );",
        )?;
        let ts = now_ms();
        let obs_s = serde_json::to_string(binder_obs)?;
        conn.execute(
            "INSERT INTO device_index_devices(tenant_id, device_id, binder_obs_json, updated_ms)
             VALUES(?1,?2,?3,?4)
             ON CONFLICT(tenant_id, device_id) DO UPDATE SET
               binder_obs_json=excluded.binder_obs_json,
               updated_ms=excluded.updated_ms",
            params![tenant_id, device_id, obs_s, ts],
        )?;
        for k in binder_keys {
            conn.execute(
                "INSERT OR IGNORE INTO device_index_keys(tenant_id, binder_key, device_id)
                 VALUES(?1,?2,?3)",
                params![tenant_id, k, device_id],
            )?;
        }
        Ok(json!({
            "ok": true,
            "tenant_id": tenant_id,
            "device_id": device_id,
            "keys_n": binder_keys.len(),
            "updated_ms": ts,
            "contract": "file_device_index_v1_store_skeleton"
        }))
    }

    fn device_index_export(&self, tenant_id: &str) -> Result<Value, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS device_index_devices (
              tenant_id TEXT NOT NULL, device_id TEXT NOT NULL,
              binder_obs_json TEXT NOT NULL, updated_ms INTEGER NOT NULL,
              PRIMARY KEY (tenant_id, device_id));
             CREATE TABLE IF NOT EXISTS device_index_keys (
              tenant_id TEXT NOT NULL, binder_key TEXT NOT NULL, device_id TEXT NOT NULL,
              PRIMARY KEY (tenant_id, binder_key, device_id));",
        )?;
        let mut devices = Map::new();
        {
            let mut stmt = conn.prepare(
                "SELECT device_id, binder_obs_json FROM device_index_devices WHERE tenant_id=?1",
            )?;
            let rows = stmt.query_map(params![tenant_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (id, js) = row?;
                let obs: Value = serde_json::from_str(&js).unwrap_or(json!({}));
                devices.insert(id, obs);
            }
        }
        let mut index = Map::new();
        {
            let mut stmt = conn.prepare(
                "SELECT binder_key, device_id FROM device_index_keys WHERE tenant_id=?1",
            )?;
            let rows = stmt.query_map(params![tenant_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (k, id) = row?;
                let arr = index.entry(k).or_insert_with(|| json!([]));
                if let Some(a) = arr.as_array_mut() {
                    a.push(json!(id));
                }
            }
        }
        Ok(json!({
            "version": "file_device_index_v1",
            "tenant_id": tenant_id,
            "algo": "link_or_mint_v1",
            "server_mint_algo": "server_mint_v1",
            "devices": devices,
            "index": index,
            "source": "gr_store_sqlite_production",
            "multi_tenant": true,
        }))
    }

    fn lookup_devices_by_binder(
        &self,
        tenant_id: &str,
        binder_key: &str,
        limit: i64,
    ) -> Result<Value, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let lim = limit.clamp(1, 200);
        let mut stmt = conn.prepare(
            "SELECT dik.device_id, did.binder_obs_json
             FROM device_index_keys dik
             JOIN device_index_devices did
               ON did.tenant_id = dik.tenant_id AND did.device_id = dik.device_id
             WHERE dik.tenant_id=?1 AND dik.binder_key=?2
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![tenant_id, binder_key, lim], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, js) = row?;
            let obs: Value = serde_json::from_str(&js).unwrap_or(json!({}));
            out.push(json!({"device_id": id, "binder_obs": obs}));
        }
        Ok(json!({
            "ok": true,
            "count": out.len(),
            "rows": out,
            "tenant_id": tenant_id,
            "binder_key": binder_key,
            "source": "device_index_keys_sqlite",
        }))
    }

    fn list_analysis_latest(
        &self,
        limit: i64,
        device_tier: Option<&str>,
        since_ms: Option<i64>,
        site_id: Option<&str>,
    ) -> Result<Value, StoreError> {
        // SQLite: project from latest analysis rows (no materialised analysis_latest table).
        let analyses = self.list_recent_latest_analyses(limit.clamp(1, 2000) as usize)?;
        let mut rows = Vec::new();
        for a in analyses {
            let device = a.get("device").cloned().unwrap_or(Value::Null);
            let product = a.get("product").cloned().unwrap_or(Value::Null);
            let trust = device.get("trust").cloned().unwrap_or(Value::Null);
            let tier = device
                .get("device_tier")
                .or_else(|| product.get("device_tier"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if let Some(filter) = device_tier {
                if !filter.is_empty() && tier != filter {
                    continue;
                }
            }
            let site = a
                .get("site_id")
                .or_else(|| a.pointer("/fields/site_id"))
                .or_else(|| product.get("site_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if let Some(want) = site_id {
                if site != want {
                    continue;
                }
            }
            let updated = a
                .get("analyzed_ms")
                .or_else(|| a.get("updated_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            if let Some(since) = since_ms {
                if updated > 0 && updated < since {
                    continue;
                }
            }
            rows.push(json!({
                "session_id": a.get("session_id"),
                "device_id": device.get("device_id").or_else(|| product.get("device_id")),
                "device_tier": tier,
                "site_id": site,
                "digest_path": device.get("digest_path").or_else(|| trust.get("digest_path")),
                "residual_entropy_ok": trust.get("residual_entropy_ok"),
                "has_webrtc_host": trust.get("materials_included").and_then(|m| m.as_array()).map(|arr| {
                    arr.iter().any(|x| x.as_str() == Some("webrtc_host") || x.as_str().map(|s| s.contains("webrtc")).unwrap_or(false))
                }),
                "collision_risk": device.get("collision_risk"),
                "product_version": a.get("product_version"),
                "updated_ms": updated,
            }));
        }
        Ok(json!({
            "ok": true,
            "count": rows.len(),
            "rows": rows,
            "source": "sqlite_recent_analyses_projection",
        }))
    }

    fn soft_edge_put(
        &self,
        tenant_id: &str,
        a_session: &str,
        b_session: &str,
        priority: &str,
        confidence: f64,
        reason: &str,
    ) -> Result<Value, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let ts = now_ms();
        conn.execute(
            "INSERT INTO soft_edges(tenant_id, a_session, b_session, priority, confidence, reason, created_ms)
             VALUES(?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(tenant_id, a_session, b_session) DO UPDATE SET
               priority=excluded.priority, confidence=excluded.confidence,
               reason=excluded.reason, created_ms=excluded.created_ms",
            params![tenant_id, a_session, b_session, priority, confidence, reason, ts],
        )?;
        Ok(json!({"ok": true, "tenant_id": tenant_id, "created_ms": ts}))
    }

    fn soft_edge_list(&self, tenant_id: &str) -> Result<Vec<Value>, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT a_session, b_session, priority, confidence, reason, created_ms
             FROM soft_edges WHERE tenant_id=?1 ORDER BY created_ms DESC LIMIT 5000",
        )?;
        let rows = stmt.query_map(params![tenant_id], |r| {
            Ok(json!({
                "a_session": r.get::<_, String>(0)?,
                "b_session": r.get::<_, String>(1)?,
                "priority": r.get::<_, String>(2)?,
                "confidence": r.get::<_, f64>(3)?,
                "reason": r.get::<_, String>(4)?,
                "created_ms": r.get::<_, i64>(5)?,
                "promote_to_commercial_id": false,
            }))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    fn soft_heat_record(
        &self,
        tenant_id: &str,
        device_id: &str,
        session_id: &str,
    ) -> Result<i64, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let ts = now_ms();
        conn.execute(
            "INSERT INTO soft_heat(tenant_id, device_id, session_id, first_ms, last_ms)
             VALUES(?1,?2,?3,?4,?4)
             ON CONFLICT(tenant_id, device_id, session_id) DO UPDATE SET last_ms=excluded.last_ms",
            params![tenant_id, device_id, session_id, ts],
        )?;
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM soft_heat WHERE tenant_id=?1 AND device_id=?2",
            params![tenant_id, device_id],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    fn soft_heat_get(&self, tenant_id: &str, device_id: &str) -> Result<Value, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT session_id FROM soft_heat WHERE tenant_id=?1 AND device_id=?2 ORDER BY last_ms DESC LIMIT 200",
        )?;
        let rows = stmt.query_map(params![tenant_id, device_id], |r| r.get::<_, String>(0))?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }
        let n = sessions.len() as i64;
        Ok(json!({
            "device_id": device_id,
            "session_count": n,
            "sessions": sessions,
            "collision_style": n >= 2,
            "soft_promote": false,
        }))
    }

    fn list_received_batches(&self, session_id: &str) -> Result<Vec<Value>, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT batch_id, source, created_ms FROM probe_batches
             WHERE session_id=?1 ORDER BY created_ms ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (batch_id, source, created_ms) = row?;
            out.push(json!({
                "batch_id": batch_id,
                "source": source,
                "created_ms": created_ms,
                "dedupe_key": format!("{session_id}|{batch_id}|{source}")
            }));
        }
        Ok(out)
    }

    fn list_peer_session_ids(&self, session_id: &str, limit: usize) -> Result<Vec<String>, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let row: Option<(String, String)> = conn
            .query_row(
                "SELECT visitor_terminal_id, meta_json FROM sessions WHERE session_id=?1",
                params![session_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((vt, meta_s)) = row else {
            return Ok(vec![]);
        };
        let harness = serde_json::from_str::<Value>(&meta_s)
            .ok()
            .and_then(|m| {
                m.get("harness_run")
                    .or_else(|| m.pointer("/harness/harness_run"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_default();
        let lim = limit.max(1) as i64;
        let mut out = Vec::new();
        if !vt.is_empty() {
            let mut stmt = conn.prepare(
                "SELECT session_id FROM sessions
                 WHERE session_id != ?1 AND visitor_terminal_id = ?2
                 ORDER BY updated_ms DESC LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![session_id, vt, lim], |r| r.get::<_, String>(0))?;
            for r in rows {
                out.push(r?);
            }
        }
        if out.len() < limit as usize && !harness.is_empty() {
            // SQLite: scan recent sessions and filter harness_run in meta JSON
            let mut stmt = conn.prepare(
                "SELECT session_id, meta_json FROM sessions
                 WHERE session_id != ?1 ORDER BY updated_ms DESC LIMIT 200",
            )?;
            let rows = stmt.query_map(params![session_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (sid, meta_s) = row?;
                if out.contains(&sid) {
                    continue;
                }
                let ok = serde_json::from_str::<Value>(&meta_s)
                    .ok()
                    .and_then(|m| {
                        m.get("harness_run")
                            .or_else(|| m.pointer("/harness/harness_run"))
                            .and_then(|v| v.as_str())
                            .map(|s| s == harness)
                    })
                    .unwrap_or(false);
                if ok {
                    out.push(sid);
                }
                if out.len() >= limit {
                    break;
                }
            }
        }
        out.truncate(limit);
        Ok(out)
    }

    fn has_batch(
        &self,
        session_id: &str,
        batch_id: &str,
        source: &str,
    ) -> Result<bool, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let n: i64 = conn.query_row(
            "SELECT COUNT(1) FROM probe_batches
             WHERE session_id=?1 AND batch_id=?2 AND source=?3",
            params![session_id, batch_id, source],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    fn list_recent_session_ids(&self, limit: usize) -> Result<Vec<String>, StoreError> {
        let lim = limit.max(1).min(5000) as i64;
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT session_id FROM sessions ORDER BY updated_ms DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![lim], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    fn list_recent_latest_analyses(&self, limit: usize) -> Result<Vec<Value>, StoreError> {
        let ids = self.list_recent_session_ids(limit)?;
        let mut out = Vec::new();
        for sid in ids {
            if let Some(mut a) = self.latest_analysis(&sid)? {
                if let Some(obj) = a.as_object_mut() {
                    obj.insert("session_id".into(), json!(sid));
                }
                out.push(a);
            }
        }
        Ok(out)
    }

    fn session_window(&self, session_id: &str) -> Result<Value, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
        let row: Option<(i64, i64, String)> = conn
            .query_row(
                "SELECT created_ms, updated_ms, meta_json FROM sessions WHERE session_id=?1",
                params![session_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((created_ms, updated_ms, meta_s)) = row else {
            return Err(StoreError::NotFound(format!("session {session_id}")));
        };
        let meta: Value = serde_json::from_str(&meta_s).unwrap_or(json!({}));
        let status = cycle_status_from_meta(&meta).to_string();
        let now = now_ms();
        let idle = now - updated_ms;
        let age = now - created_ms;
        // Primary clock: cycle incomplete TTL (72h). Complete cycles reject identity ingest.
        let (active, expired_reason) = if status == "complete" {
            (false, "cycle_complete")
        } else if status == "purged" {
            (false, "cycle_purged")
        } else if age > cycle_incomplete_ms() {
            (false, "incomplete_ttl")
        } else {
            (true, "")
        };
        Ok(json!({
            "session_id": session_id,
            "cycle_id": session_id,
            "created_ms": created_ms,
            "updated_ms": updated_ms,
            "idle_ms": idle,
            "age_ms": age,
            "cycle_status": status,
            "inactivity_window_ms": SESSION_INACTIVITY_MS,
            "hard_max_session_ms": SESSION_HARD_MAX_MS,
            "cycle_incomplete_ms": cycle_incomplete_ms(),
            "cycle_cool_ms": cycle_cool_ms(),
            "active": active,
            "expired_reason": expired_reason,
            // iss/38 D-14: expose meta so Unknown Hub can aggregate unknown_bucket
            "meta": meta,
            "unknown_bucket": meta.get("unknown_bucket").cloned().unwrap_or(Value::Null),
        }))
    }
}

fn sqlite_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn sqlite_s_field(row: &Value, k: &str) -> String {
    row.get(k)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn sqlite_i_field(row: &Value, k: &str, default: i64) -> i64 {
    row.get(k).and_then(|v| v.as_i64()).unwrap_or(default)
}

fn sqlite_f_field(row: &Value, k: &str, default: f64) -> f64 {
    row.get(k).and_then(|v| v.as_f64()).unwrap_or(default)
}

fn sqlite_insert_ops_client(s: &SqliteStore, row: Value) -> Result<Value, StoreError> {
    let ts = sqlite_now_ms();
    let mut event_id = sqlite_s_field(&row, "event_id");
    if event_id.is_empty() {
        event_id = format!("oce_{ts}");
    }
    let detail = row.get("detail_json").cloned().unwrap_or(json!({}));
    let detail_s = serde_json::to_string(&detail).unwrap_or_else(|_| "{}".into());
    let sev = {
        let s = sqlite_s_field(&row, "severity");
        if s.is_empty() {
            "error".into()
        } else {
            s
        }
    };
    let ip = sqlite_s_field(&row, "client_ip");
    let conn = s.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
    conn.execute(
        r#"INSERT OR REPLACE INTO ops_client_events(
            event_id, ts_ms, server_recv_ms, site_id, visitor_terminal_id, session_id,
            product_version, inject_path, engine_family, ua_hash, stage, code, severity,
            detail_json, sample_rate, client_ip)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)"#,
        params![
            event_id,
            sqlite_i_field(&row, "ts_ms", ts),
            ts,
            sqlite_s_field(&row, "site_id"),
            sqlite_s_field(&row, "visitor_terminal_id"),
            sqlite_s_field(&row, "session_id"),
            sqlite_s_field(&row, "product_version"),
            sqlite_s_field(&row, "inject_path"),
            sqlite_s_field(&row, "engine_family"),
            sqlite_s_field(&row, "ua_hash"),
            sqlite_s_field(&row, "stage"),
            sqlite_s_field(&row, "code"),
            sev,
            detail_s,
            sqlite_f_field(&row, "sample_rate", 1.0),
            if ip.is_empty() { None } else { Some(ip) },
        ],
    )?;
    Ok(json!({"ok": true, "event_id": event_id, "server_recv_ms": ts, "backend": "sqlite"}))
}

fn sqlite_insert_ops_server(s: &SqliteStore, row: Value) -> Result<Value, StoreError> {
    let ts = sqlite_now_ms();
    let mut event_id = sqlite_s_field(&row, "event_id");
    if event_id.is_empty() {
        event_id = format!("ose_{ts}");
    }
    let detail = row.get("detail_json").cloned().unwrap_or(json!({}));
    let detail_s = serde_json::to_string(&detail).unwrap_or_else(|_| "{}".into());
    let sev = {
        let s = sqlite_s_field(&row, "severity");
        if s.is_empty() {
            "error".into()
        } else {
            s
        }
    };
    let ip = sqlite_s_field(&row, "client_ip");
    let conn = s.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
    conn.execute(
        r#"INSERT OR REPLACE INTO ops_server_events(
            event_id, ts_ms, site_id, visitor_terminal_id, session_id,
            product_version, engine_family, stage, code, severity, detail_json, client_ip)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)"#,
        params![
            event_id,
            sqlite_i_field(&row, "ts_ms", ts),
            sqlite_s_field(&row, "site_id"),
            sqlite_s_field(&row, "visitor_terminal_id"),
            sqlite_s_field(&row, "session_id"),
            sqlite_s_field(&row, "product_version"),
            sqlite_s_field(&row, "engine_family"),
            sqlite_s_field(&row, "stage"),
            sqlite_s_field(&row, "code"),
            sev,
            detail_s,
            if ip.is_empty() { None } else { Some(ip) },
        ],
    )?;
    Ok(json!({"ok": true, "event_id": event_id, "ts_ms": ts, "backend": "sqlite"}))
}

fn sqlite_insert_observation_event(s: &SqliteStore, row: Value) -> Result<Value, StoreError> {
    let ts = sqlite_now_ms();
    let mut observation_id = sqlite_s_field(&row, "observation_id");
    if observation_id.is_empty() {
        observation_id = format!("obs_{ts}");
    }
    let envelope = row
        .get("envelope_json")
        .cloned()
        .or_else(|| row.get("envelope").cloned())
        .unwrap_or(json!({}));
    // iss/opus5 04-P0-2: pointer + summary by default (same policy as PG).
    let envelope = crate::pg::slim_observation_envelope(&envelope);
    let envelope_s = serde_json::to_string(&envelope).unwrap_or_else(|_| "{}".into());
    let conn = s.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
    let _ = conn.execute_batch(
        r#"CREATE TABLE IF NOT EXISTS observation_events (
  observation_id TEXT PRIMARY KEY,
  tenant_id TEXT NOT NULL DEFAULT '',
  session_id TEXT NOT NULL,
  batch_id TEXT NOT NULL,
  source TEXT NOT NULL DEFAULT '',
  source_kind TEXT NOT NULL DEFAULT '',
  realm_kind TEXT NOT NULL DEFAULT '',
  probe_method_id TEXT NOT NULL DEFAULT '',
  capture_id TEXT,
  attempt_id TEXT,
  envelope_json TEXT NOT NULL,
  created_ms INTEGER NOT NULL
);"#,
    );
    conn.execute(
        r#"INSERT OR IGNORE INTO observation_events(
            observation_id, tenant_id, session_id, batch_id, source,
            source_kind, realm_kind, probe_method_id, capture_id, attempt_id,
            envelope_json, created_ms)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)"#,
        params![
            observation_id,
            sqlite_s_field(&row, "tenant_id"),
            sqlite_s_field(&row, "session_id"),
            sqlite_s_field(&row, "batch_id"),
            sqlite_s_field(&row, "source"),
            sqlite_s_field(&row, "source_kind"),
            sqlite_s_field(&row, "realm_kind"),
            sqlite_s_field(&row, "probe_method_id"),
            {
                let s = sqlite_s_field(&row, "capture_id");
                if s.is_empty() { None } else { Some(s) }
            },
            {
                let s = sqlite_s_field(&row, "attempt_id");
                if s.is_empty() { None } else { Some(s) }
            },
            envelope_s,
            sqlite_i_field(&row, "created_ms", ts),
        ],
    )?;
    Ok(json!({"ok": true, "observation_id": observation_id, "appended": true, "backend": "sqlite"}))
}

/// iss/opus5 05-S-5: DSAR helpers (SQLite). Local mask copy — must match
/// gr-probe-core privacy::mask_ip_subnet (store has no core dep).
fn sqlite_dsar_mask_ip(ip: &str) -> String {
    let t = ip.trim();
    if t.is_empty() || t.contains('/') {
        return t.to_string();
    }
    if let Ok(v4) = t.parse::<std::net::Ipv4Addr>() {
        let o = v4.octets();
        return format!("{}.{}.{}.0/24", o[0], o[1], o[2]);
    }
    if let Ok(v6) = t.parse::<std::net::Ipv6Addr>() {
        let s = v6.segments();
        return format!("{:x}:{:x}:{:x}::/48", s[0], s[1], s[2]);
    }
    t.to_string()
}

fn sqlite_dsar_resolve_sessions(
    conn: &rusqlite::Connection,
    kind: &str,
    value: &str,
) -> Result<Vec<String>, StoreError> {
    let ip_masked = sqlite_dsar_mask_ip(value);
    let sql = match kind {
        "visitor_terminal_id" | "vt" => (
            "SELECT session_id FROM sessions WHERE visitor_terminal_id=?1".to_string(),
            vec![value.to_string()],
        ),
        "device_id" => (
            // analysis_results stores the envelope; the device tag is inside
            // result_json (device.device_id) — json_extract on the decoded doc.
            // (SQLite analysis_results has no device_id column; PG does.)
            "SELECT session_id FROM analysis_results
             WHERE json_extract(result_json,'$.device.device_id')=?1
                OR json_extract(result_json,'$.product.device_id')=?1"
                .to_string(),
            vec![value.to_string(), value.to_string()],
        ),
        "client_ip" => (
            // SQLite parity: sessions/probe_batches carry no client_ip column
            // (client_ip is folded into payload fields); the ops event tables
            // do carry it, so resolve the session set from there.
            "SELECT session_id FROM ops_client_events
               WHERE client_ip=?1 OR client_ip=?2
             UNION
             SELECT session_id FROM ops_server_events
               WHERE client_ip=?1 OR client_ip=?2"
                .to_string(),
            vec![value.to_string(), ip_masked.clone()],
        ),
        "site_id" => (
            "SELECT session_id FROM sessions WHERE json_extract(meta_json,'$.site_id')=?1
             OR json_extract(meta_json,'$.siteId')=?1"
                .to_string(),
            vec![value.to_string()],
        ),
        _ => {
            return Err(StoreError::Msg(format!(
                "unknown subject kind {kind} (want visitor_terminal_id|device_id|client_ip|site_id)"
            )))
        }
    };
    let (sql, binds) = sql;
    let mut stmt = conn.prepare(&sql).map_err(|e| StoreError::Msg(e.to_string()))?;
    let rows: Vec<String> = stmt
        .query_map(rusqlite::params_from_iter(binds.iter()), |r| r.get(0))
        .map_err(|e| StoreError::Msg(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();
    if rows.len() > 50_000 {
        return Err(StoreError::Msg("dsar selector too broad (>50000 sessions)".into()));
    }
    Ok(rows)
}

fn sqlite_subject_erase(s: &SqliteStore, kind: &str, value: &str) -> Result<Value, StoreError> {
    let conn = s.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
    let sessions = sqlite_dsar_resolve_sessions(&conn, kind, value)?;
    let ip_masked = sqlite_dsar_mask_ip(value);
    let mut counts = serde_json::Map::new();
    // Helper as a fn (not closure) so `counts` stays borrowable in loops.
    fn bump(counts: &mut serde_json::Map<String, Value>, name: &str, n: usize) {
        if n > 0 {
            let cur = counts.get(name).and_then(|v| v.as_i64()).unwrap_or(0);
            counts.insert(name.to_string(), json!(cur + n as i64));
        }
    }
    let mut del = |counts: &mut serde_json::Map<String, Value>,
                   conn: &rusqlite::Connection,
                   name: &str,
                   sql: &str,
                   n_bind: bool| {
        let n = if n_bind {
            conn.execute(sql, params![value]).unwrap_or(0)
        } else {
            conn.execute(sql, []).unwrap_or(0)
        };
        bump(counts, name, n);
    };
    if !sessions.is_empty() {
        // Batched IN-list (sqlite has no ANY($1)); chunks of 500.
        for chunk in sessions.chunks(500) {
            let marks = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let plist = rusqlite::params_from_iter(chunk.iter());
            for (name, table) in [
                ("analyze_jobs", "analyze_jobs"),
                ("page_results", "page_results"),
                ("observation_events", "observation_events"),
                ("analysis_results", "analysis_results"),
                ("probe_batches", "probe_batches"),
                ("ops_client_events", "ops_client_events"),
                ("ops_server_events", "ops_server_events"),
                ("soft_heat", "soft_heat"),
            ] {
                let sql = format!("DELETE FROM {table} WHERE session_id IN ({marks})");
                let n = conn.execute(&sql, plist.clone()).unwrap_or(0);
                bump(&mut counts, name, n);
            }
            let sql = format!(
                "DELETE FROM soft_edges WHERE a_session IN ({marks}) OR b_session IN ({marks})"
            );
            let doubled: Vec<&String> = chunk.iter().chain(chunk.iter()).collect();
            let n = conn
                .execute(&sql, rusqlite::params_from_iter(doubled))
                .unwrap_or(0);
            bump(&mut counts, "soft_edges", n);
            let sql = format!("DELETE FROM sessions WHERE session_id IN ({marks})");
            let n = conn.execute(&sql, plist).unwrap_or(0);
            bump(&mut counts, "sessions", n);
        }
    }
    match kind {
        "visitor_terminal_id" | "vt" => {
            del(&mut counts, &conn, "visitor_terminals", "DELETE FROM visitor_terminals WHERE vt_id=?1", true);
            del(&mut counts, &conn, "ops_client_events", "DELETE FROM ops_client_events WHERE visitor_terminal_id=?1", true);
            del(&mut counts, &conn, "ops_server_events", "DELETE FROM ops_server_events WHERE visitor_terminal_id=?1", true);
        }
        "device_id" => {
            del(&mut counts, &conn, "device_index_devices", "DELETE FROM device_index_devices WHERE device_id=?1", true);
            del(&mut counts, &conn, "device_index_keys", "DELETE FROM device_index_keys WHERE device_id=?1", true);
            del(&mut counts, &conn, "soft_heat", "DELETE FROM soft_heat WHERE device_id=?1", true);
        }
        "client_ip" => {
            let v = value.replace('\'', "");
            let m = ip_masked.replace('\'', "");
            // Only ops tables carry client_ip in the sqlite schema.
            for (name, table) in [
                ("ops_client_events", "ops_client_events"),
                ("ops_server_events", "ops_server_events"),
            ] {
                let sql = format!("DELETE FROM {table} WHERE client_ip='{v}' OR client_ip='{m}'");
                del(&mut counts, &conn, name, &sql, false);
            }
        }
        "site_id" => {
            del(&mut counts, &conn, "ops_client_events", "DELETE FROM ops_client_events WHERE site_id=?1", true);
            del(&mut counts, &conn, "ops_server_events", "DELETE FROM ops_server_events WHERE site_id=?1", true);
            del(&mut counts, &conn, "observation_events", "DELETE FROM observation_events WHERE tenant_id=?1", true);
        }
        _ => {}
    }
    Ok(json!({
        "ok": true,
        "backend": "sqlite",
        "subject_kind": kind,
        "sessions_matched": sessions.len(),
        "deleted": counts,
    }))
}

fn sqlite_subject_export(s: &SqliteStore, kind: &str, value: &str) -> Result<Value, StoreError> {
    let conn = s.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
    let sessions = sqlite_dsar_resolve_sessions(&conn, kind, value)?;
    let mut out = serde_json::Map::new();
    out.insert("sessions_matched".into(), json!(sessions.len()));
    out.insert("session_ids".into(), json!(sessions));
    if !sessions.is_empty() {
        let mut sess_rows = Vec::new();
        for chunk in sessions.chunks(500) {
            let marks = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT session_id, visitor_terminal_id, created_ms, updated_ms, meta_json
                 FROM sessions WHERE session_id IN ({marks}) LIMIT 5000"
            );
            let mut stmt = conn.prepare(&sql).map_err(|e| StoreError::Msg(e.to_string()))?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
                    Ok(json!({
                        "session_id": r.get::<_, String>(0)?,
                        "visitor_terminal_id": r.get::<_, Option<String>>(1)?,
                        "created_ms": r.get::<_, i64>(2)?,
                        "updated_ms": r.get::<_, i64>(3)?,
                        "meta": serde_json::from_str::<Value>(&r.get::<_, String>(4)?).unwrap_or(json!({})),
                    }))
                })
                .map_err(|e| StoreError::Msg(e.to_string()))?;
            for r in rows.flatten() {
                sess_rows.push(r);
            }
        }
        out.insert("sessions".into(), json!(sess_rows));
        let mut ana_rows = Vec::new();
        for chunk in sessions.chunks(500) {
            let marks = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT session_id, rev, result_json, created_ms FROM analysis_results
                 WHERE session_id IN ({marks}) ORDER BY created_ms ASC LIMIT 5000"
            );
            let mut stmt = conn.prepare(&sql).map_err(|e| StoreError::Msg(e.to_string()))?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
                    let raw: String = r.get(2)?;
                    Ok(json!({
                        "session_id": r.get::<_, String>(0)?,
                        "rev": r.get::<_, i64>(1)?,
                        "result": crate::decode_analysis_result_json(&raw).unwrap_or(json!({})),
                        "created_ms": r.get::<_, i64>(3)?,
                    }))
                })
                .map_err(|e| StoreError::Msg(e.to_string()))?;
            for r in rows.flatten() {
                ana_rows.push(r);
            }
        }
        out.insert("analysis_results".into(), json!(ana_rows));
    }
    Ok(json!({
        "ok": true,
        "backend": "sqlite",
        "subject_kind": kind,
        "export": Value::Object(out),
    }))
}

fn sqlite_ensure_api_idempotency(conn: &rusqlite::Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        r#"CREATE TABLE IF NOT EXISTS api_idempotency (
  tenant_id TEXT NOT NULL DEFAULT '',
  route TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  body_hash TEXT NOT NULL,
  status INTEGER NOT NULL,
  response_json TEXT NOT NULL,
  created_ms INTEGER NOT NULL,
  PRIMARY KEY (tenant_id, route, idempotency_key)
);"#,
    )?;
    Ok(())
}

fn sqlite_lookup_api_idempotency(
    s: &SqliteStore,
    tenant_id: &str,
    route: &str,
    idempotency_key: &str,
) -> Result<Option<Value>, StoreError> {
    let conn = s.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
    sqlite_ensure_api_idempotency(&conn)?;
    let mut stmt = conn.prepare(
        "SELECT body_hash, status, response_json FROM api_idempotency
         WHERE tenant_id=?1 AND route=?2 AND idempotency_key=?3",
    )?;
    let row = stmt.query_row(params![tenant_id, route, idempotency_key], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
        ))
    });
    match row {
        Ok((body_hash, status, response_s)) => {
            let response: Value = serde_json::from_str(&response_s).unwrap_or(json!({}));
            Ok(Some(json!({
                "hit": true,
                "body_hash": body_hash,
                "status": status,
                "response": response,
            })))
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(StoreError::Sqlite(e)),
    }
}

fn sqlite_put_api_idempotency(
    s: &SqliteStore,
    tenant_id: &str,
    route: &str,
    idempotency_key: &str,
    body_hash: &str,
    status: i64,
    response: &Value,
) -> Result<Value, StoreError> {
    let ts = sqlite_now_ms();
    let response_s = serde_json::to_string(response).unwrap_or_else(|_| "{}".into());
    let conn = s.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
    sqlite_ensure_api_idempotency(&conn)?;
    conn.execute(
        r#"INSERT OR IGNORE INTO api_idempotency(
            tenant_id, route, idempotency_key, body_hash, status, response_json, created_ms)
         VALUES(?1,?2,?3,?4,?5,?6,?7)"#,
        params![
            tenant_id,
            route,
            idempotency_key,
            body_hash,
            status,
            response_s,
            ts
        ],
    )?;
    Ok(json!({"ok": true, "backend": "sqlite"}))
}

fn sqlite_list_ops_events(
    s: &SqliteStore,
    source: &str,
    limit: i64,
    code: Option<String>,
    site_id: Option<String>,
    since_ms: Option<i64>,
    product_version: Option<String>,
) -> Result<Value, StoreError> {
    let lim = limit.clamp(1, 500);
    let (table, ts_col) = if source == "client" {
        ("ops_client_events", "server_recv_ms")
    } else {
        ("ops_server_events", "ts_ms")
    };
    let mut sql = format!(
        "SELECT event_id, {ts_col}, site_id, visitor_terminal_id, session_id, product_version, engine_family, stage, code, severity, detail_json FROM {table} WHERE 1=1"
    );
    let mut args: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    if let Some(c) = code.filter(|s| !s.is_empty()) {
        sql.push_str(" AND code=?");
        args.push(Box::new(c));
    }
    if let Some(sid) = site_id.filter(|s| !s.is_empty()) {
        sql.push_str(" AND site_id=?");
        args.push(Box::new(sid));
    }
    if let Some(pv) = product_version.filter(|s| !s.is_empty()) {
        sql.push_str(" AND product_version LIKE ?");
        args.push(Box::new(format!("{pv}%")));
    }
    if let Some(since) = since_ms {
        sql.push_str(&format!(" AND {ts_col} >= ?"));
        args.push(Box::new(since));
    }
    sql.push_str(&format!(" ORDER BY {ts_col} DESC LIMIT ?"));
    args.push(Box::new(lim));
    let conn = s.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
    let mut stmt = conn.prepare(&sql)?;
    let params_ref: Vec<&dyn rusqlite::types::ToSql> = args.iter().map(|a| a.as_ref()).collect();
    let rows = stmt.query_map(params_ref.as_slice(), |r| {
        let detail: String = r.get(10)?;
        Ok(json!({
            "event_id": r.get::<_, String>(0)?,
            "ts_ms": r.get::<_, i64>(1)?,
            "site_id": r.get::<_, String>(2)?,
            "visitor_terminal_id": r.get::<_, String>(3)?,
            "session_id": r.get::<_, String>(4)?,
            "product_version": r.get::<_, String>(5)?,
            "engine_family": r.get::<_, String>(6)?,
            "stage": r.get::<_, String>(7)?,
            "code": r.get::<_, String>(8)?,
            "severity": r.get::<_, String>(9)?,
            "detail": serde_json::from_str::<Value>(&detail).unwrap_or(json!({})),
        }))
    })?;
    let mut events = Vec::new();
    for row in rows {
        events.push(row?);
    }
    Ok(json!({
        "ok": true,
        "backend": "sqlite",
        "source": source,
        "count": events.len(),
        "events": events,
    }))
}

/// SQLite parity for ops B10 health: session×batch B10 coverage + top error codes.
fn sqlite_ops_b10_health(
    s: &SqliteStore,
    since_ms: i64,
    limit_sites: i64,
) -> Result<Value, StoreError> {
    let lim = limit_sites.clamp(1, 64);
    let conn = s.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
    // Sessions since window
    let total: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE created_ms >= ?1",
            params![since_ms],
            |r| r.get(0),
        )
        .unwrap_or(0);
    // Sessions with any B10-like batch (batch_id contains B10 or gateway B8 early still counts as partial — only B10*)
    let with_b10: i64 = conn
        .query_row(
            r#"SELECT COUNT(DISTINCT session_id) FROM probe_batches
               WHERE session_id IN (SELECT session_id FROM sessions WHERE created_ms >= ?1)
                 AND (batch_id LIKE 'B10%' OR batch_id LIKE '%B10%')"#,
            params![since_ms],
            |r| r.get(0),
        )
        .unwrap_or(0);
    // Engine breakdown from session meta when present
    let mut by_engine: Vec<Value> = Vec::new();
    {
        let mut stmt = conn.prepare(
            r#"SELECT
                 COALESCE(NULLIF(json_extract(meta_json,'$.engine_family'),''),
                          NULLIF(json_extract(meta_json,'$.belief.axes.engine.claim'),''),
                          'unknown') AS eng,
                 COUNT(*) AS n,
                 SUM(CASE WHEN EXISTS (
                   SELECT 1 FROM probe_batches b
                   WHERE b.session_id = s.session_id
                     AND (b.batch_id LIKE 'B10%' OR b.batch_id LIKE '%B10%')
                 ) THEN 1 ELSE 0 END) AS n_b10
               FROM sessions s
               WHERE s.created_ms >= ?1
               GROUP BY eng
               ORDER BY n DESC
               LIMIT ?2"#,
        )?;
        let rows = stmt.query_map(params![since_ms, lim], |r| {
            let n: i64 = r.get(1)?;
            let nb: i64 = r.get(2)?;
            let rate = if n > 0 { nb as f64 / n as f64 } else { 0.0 };
            Ok(json!({
                "engine": r.get::<_, String>(0)?,
                "sessions": n,
                "with_b10": nb,
                "b10_rate": rate,
            }))
        })?;
        for row in rows {
            by_engine.push(row?);
        }
    }
    let mut top_errors: Vec<Value> = Vec::new();
    {
        let mut stmt = conn.prepare(
            r#"SELECT code, COUNT(*) AS n FROM (
                 SELECT code, ts_ms AS t FROM ops_server_events WHERE ts_ms >= ?1
                 UNION ALL
                 SELECT code, server_recv_ms AS t FROM ops_client_events WHERE server_recv_ms >= ?1
               ) GROUP BY code ORDER BY n DESC LIMIT 20"#,
        )?;
        let rows = stmt.query_map(params![since_ms], |r| {
            Ok(json!({
                "code": r.get::<_, String>(0)?,
                "n": r.get::<_, i64>(1)?,
            }))
        })?;
        for row in rows {
            top_errors.push(row?);
        }
    }
    let rate = if total > 0 {
        with_b10 as f64 / total as f64
    } else {
        0.0
    };
    Ok(json!({
        "ok": true,
        "backend": "sqlite",
        "since_ms": since_ms,
        "sessions": total,
        "with_b10": with_b10,
        "b10_rate": rate,
        "by_engine": by_engine,
        "top_errors": top_errors,
    }))
}

fn sqlite_ensure_velocity(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS velocity_hits (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          key_kind TEXT NOT NULL,
          key_value TEXT NOT NULL,
          session_id TEXT,
          hit_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_velocity_hits_kv_ms
          ON velocity_hits(key_kind, key_value, hit_ms DESC);
        "#,
    )?;
    Ok(())
}

fn sqlite_ops_probe_completeness(s: &SqliteStore, since_ms: i64) -> Result<Value, StoreError> {
    let conn = s.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
    // Group latest analysis_results per session by product_version + site from meta
    let mut by: Vec<Value> = Vec::new();
    let mut total = 0i64;
    let mut total_main = 0i64;
    let mut total_missing = 0i64;
    {
        let mut stmt = conn.prepare(
            r#"
            WITH latest AS (
              SELECT ar.session_id, ar.product_version, ar.digest_path, ar.created_ms,
                     ar.site_id AS denorm_site
              FROM analysis_results ar
              INNER JOIN (
                SELECT session_id, MAX(rev) AS rev
                FROM analysis_results
                WHERE created_ms >= ?1
                GROUP BY session_id
              ) m ON ar.session_id = m.session_id AND ar.rev = m.rev
            )
            SELECT
              COALESCE(NULLIF(l.denorm_site,''),
                       NULLIF(json_extract(s.meta_json,'$.site_id'),''),
                       NULLIF(json_extract(s.meta_json,'$.site'),''),
                       '(none)') AS site_id,
              COALESCE(NULLIF(l.product_version,''), '(none)') AS product_version,
              COUNT(*) AS n,
              SUM(CASE WHEN EXISTS(
                    SELECT 1 FROM probe_batches pb
                    WHERE pb.session_id = l.session_id AND pb.batch_id = 'B0_bootstrap'
                  ) AND EXISTS(
                    SELECT 1 FROM probe_batches pb2
                    WHERE pb2.session_id = l.session_id AND pb2.batch_id = 'B10_hw_curves'
                  ) THEN 1 ELSE 0 END) AS n_main,
              SUM(CASE WHEN l.digest_path = 'gateway_only_v1' THEN 1 ELSE 0 END) AS n_gw,
              SUM(CASE WHEN l.digest_path = 'real_curves_v1' THEN 1 ELSE 0 END) AS n_rc,
              SUM(CASE WHEN l.digest_path IS NULL OR l.digest_path = '' OR l.digest_path = 'gateway_only_v1'
                       THEN 1 ELSE 0 END) AS n_thin
            FROM latest l
            LEFT JOIN sessions s ON s.session_id = l.session_id
            GROUP BY 1, 2
            ORDER BY n DESC
            LIMIT 200
            "#,
        )?;
        let rows = stmt.query_map(params![since_ms], |r| {
            let n: i64 = r.get(2)?;
            let n_main: i64 = r.get(3)?;
            let n_gw: i64 = r.get(4)?;
            let n_rc: i64 = r.get(5)?;
            let n_thin: i64 = r.get(6)?;
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                n,
                n_main,
                n_gw,
                n_rc,
                n_thin,
            ))
        })?;
        for row in rows {
            let (site, pv, n, n_main, n_gw, n_rc, n_thin) = row?;
            total += n;
            total_main += n_main;
            // approximate missing as thin/gateway when os_status not denorm'd on sqlite
            total_missing += n_thin;
            by.push(json!({
                "site_id": site,
                "product_version": pv,
                "sessions": n,
                "main_complete": n_main,
                "main_complete_rate": if n > 0 { n_main as f64 / n as f64 } else { 0.0 },
                "gateway_only": n_gw,
                "gateway_only_rate": if n > 0 { n_gw as f64 / n as f64 } else { 0.0 },
                "real_curves": n_rc,
                "missing_probe_est": n_thin,
                "missing_probe_rate": if n > 0 { n_thin as f64 / n as f64 } else { 0.0 },
            }));
        }
    }
    // Fallback: sessions without analysis — still count batch main_complete
    if total == 0 {
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE created_ms >= ?1",
                params![since_ms],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let n_main: i64 = conn
            .query_row(
                r#"SELECT COUNT(*) FROM sessions s WHERE s.created_ms >= ?1
                   AND EXISTS(SELECT 1 FROM probe_batches pb WHERE pb.session_id=s.session_id AND pb.batch_id='B0_bootstrap')
                   AND EXISTS(SELECT 1 FROM probe_batches pb2 WHERE pb2.session_id=s.session_id AND pb2.batch_id='B10_hw_curves')"#,
                params![since_ms],
                |r| r.get(0),
            )
            .unwrap_or(0);
        total = n;
        total_main = n_main;
        by.push(json!({
            "site_id": "(all)",
            "product_version": "(no_analysis_yet)",
            "sessions": n,
            "main_complete": n_main,
            "main_complete_rate": if n > 0 { n_main as f64 / n as f64 } else { 0.0 },
        }));
    }
    Ok(json!({
        "ok": true,
        "backend": "sqlite",
        "since_ms": since_ms,
        "sessions": total,
        "main_complete": total_main,
        "main_complete_rate": if total > 0 { total_main as f64 / total as f64 } else { 0.0 },
        "missing_probe": total_missing,
        "missing_probe_rate": if total > 0 { total_missing as f64 / total as f64 } else { 0.0 },
        "by_site_version": by,
        "definition": {
            "main_complete": "has probe_batches B0_bootstrap AND B10_hw_curves",
            "missing_probe_est": "digest gateway_only or empty (sqlite approx)",
        }
    }))
}

fn sqlite_ops_outcome_distribution(s: &SqliteStore, since_ms: i64) -> Result<Value, StoreError> {
    let conn = s.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
    let mut by_bot: Vec<Value> = Vec::new();
    let mut total = 0i64;
    {
        let mut stmt = conn.prepare(
            r#"
            WITH latest AS (
              SELECT ar.session_id, ar.bot_verdict, ar.real_band, ar.site_id
              FROM analysis_results ar
              INNER JOIN (
                SELECT session_id, MAX(rev) AS rev
                FROM analysis_results
                WHERE created_ms >= ?1
                GROUP BY session_id
              ) m ON ar.session_id = m.session_id AND ar.rev = m.rev
            )
            SELECT COALESCE(NULLIF(bot_verdict,''), '(none)') AS k, COUNT(*) AS n
            FROM latest GROUP BY 1 ORDER BY n DESC LIMIT 50
            "#,
        )?;
        let rows = stmt.query_map(params![since_ms], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (k, n) = row?;
            total += n;
            by_bot.push(json!({"key": k, "count": n}));
        }
    }
    let mut by_band: Vec<Value> = Vec::new();
    {
        let mut stmt = conn.prepare(
            r#"
            WITH latest AS (
              SELECT ar.real_band
              FROM analysis_results ar
              INNER JOIN (
                SELECT session_id, MAX(rev) AS rev
                FROM analysis_results
                WHERE created_ms >= ?1
                GROUP BY session_id
              ) m ON ar.session_id = m.session_id AND ar.rev = m.rev
            )
            SELECT COALESCE(NULLIF(real_band,''), '(none)') AS k, COUNT(*) AS n
            FROM latest GROUP BY 1 ORDER BY n DESC LIMIT 50
            "#,
        )?;
        let rows = stmt.query_map(params![since_ms], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (k, n) = row?;
            by_band.push(json!({"key": k, "count": n}));
        }
    }
    let mut allow_n = 0i64;
    let mut challenge_n = 0i64;
    let mut deny_n = 0i64;
    let mut other_n = 0i64;
    for item in &by_bot {
        let k = item
            .get("key")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let n = item.get("count").and_then(|v| v.as_i64()).unwrap_or(0);
        if k.contains("bot") || k == "deny" || k.contains("automated") {
            deny_n += n;
        } else if k.contains("suspect") || k.contains("risk") || k.contains("challenge") {
            challenge_n += n;
        } else if k.contains("human") || k == "allow" || k.contains("real") || k == "ok" {
            allow_n += n;
        } else if k == "(none)" || k.is_empty() {
            other_n += n;
        } else {
            challenge_n += n;
        }
    }
    let mut by_action_proxy = Vec::new();
    if allow_n > 0 {
        by_action_proxy.push(json!({"action": "allow", "count": allow_n}));
    }
    if challenge_n > 0 {
        by_action_proxy.push(json!({"action": "challenge", "count": challenge_n}));
    }
    if deny_n > 0 {
        by_action_proxy.push(json!({"action": "deny", "count": deny_n}));
    }
    if other_n > 0 {
        by_action_proxy.push(json!({"action": "unknown", "count": other_n}));
    }
    Ok(json!({
        "ok": true,
        "backend": "sqlite",
        "since_ms": since_ms,
        "sessions": total,
        "by_bot_verdict": by_bot,
        "by_real_band": by_band,
        "by_action_proxy": by_action_proxy,
        "by_site_bot": [],
        "note": "Action proxy derived from bot_verdict denorm; live recommended_action uses panel strategy at get_result.",
    }))
}

fn sqlite_velocity_record(
    s: &SqliteStore,
    session_id: &str,
    device_id: Option<&str>,
    client_ip: Option<&str>,
    hit_ms: i64,
) -> Result<(), StoreError> {
    let conn = s.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
    sqlite_ensure_velocity(&conn)?;
    if let Some(d) = device_id.map(str::trim).filter(|x| !x.is_empty()) {
        conn.execute(
            "INSERT INTO velocity_hits(key_kind, key_value, session_id, hit_ms) VALUES('device_id',?1,?2,?3)",
            params![d, session_id, hit_ms],
        )?;
    }
    if let Some(ip) = client_ip.map(str::trim).filter(|x| !x.is_empty()) {
        conn.execute(
            "INSERT INTO velocity_hits(key_kind, key_value, session_id, hit_ms) VALUES('client_ip',?1,?2,?3)",
            params![ip, session_id, hit_ms],
        )?;
    }
    Ok(())
}

fn sqlite_velocity_summary(
    s: &SqliteStore,
    device_id: Option<&str>,
    client_ip: Option<&str>,
    now_ms: i64,
) -> Result<Value, StoreError> {
    let conn = s.conn.lock().map_err(|e| StoreError::Msg(e.to_string()))?;
    sqlite_ensure_velocity(&conn)?;
    let windows = [300_000i64, 3_600_000, 86_400_000];
    let mut device = json!({});
    let mut ip = json!({});
    for w in windows {
        let since = now_ms - w;
        let label = match w {
            300_000 => "5m",
            3_600_000 => "1h",
            _ => "24h",
        };
        if let Some(d) = device_id.map(str::trim).filter(|x| !x.is_empty()) {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM velocity_hits WHERE key_kind='device_id' AND key_value=?1 AND hit_ms>=?2",
                    params![d, since],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            device[label] = json!(n);
        }
        if let Some(addr) = client_ip.map(str::trim).filter(|x| !x.is_empty()) {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM velocity_hits WHERE key_kind='client_ip' AND key_value=?1 AND hit_ms>=?2",
                    params![addr, since],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            ip[label] = json!(n);
        }
    }
    let d1h = device.get("1h").and_then(|v| v.as_i64()).unwrap_or(0).max(0) as f64;
    let score = (d1h / 20.0).clamp(0.0, 1.0);
    Ok(json!({
        "ok": true,
        "backend": "sqlite",
        "device_id": device,
        "client_ip": ip,
        "velocity_score": score,
        "windows_ms": { "5m": 300_000, "1h": 3_600_000, "24h": 86_400_000 },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// iss/opus5 06-P1-5: monthly per-site session counter accumulates per
    /// month bucket and per site (billing signal, soft quota).
    #[test]
    fn monthly_sessions_counter_per_site_and_month() {
        let db = std::env::temp_dir().join(format!("gr_usage_{}.sqlite", hex_now()));
        let _ = std::fs::remove_file(&db);
        let store = Store::open(&db).unwrap();
        let (m1, n1) = store.bump_monthly_sessions("site_a").unwrap();
        let (m2, n2) = store.bump_monthly_sessions("site_a").unwrap();
        let (m3, n3) = store.bump_monthly_sessions("site_b").unwrap();
        assert_eq!(m1, m2, "same UTC month bucket");
        assert_eq!(m3, m1);
        assert_eq!(n1, 1);
        assert_eq!(n2, 2, "same site accumulates");
        assert_eq!(n3, 1, "sites count independently");
        // Bucket shape: YYYY-MM
        assert_eq!(m1.len(), 7);
        assert!(m1.chars().nth(4) == Some('-'));
        let _ = std::fs::remove_file(&db);
    }

    /// iss/opus5 05 low: seal replay ledger accepts first tuple, rejects the
    /// exact (session, batch, nonce) replay, accepts a fresh nonce.
    #[test]
    fn seal_consumed_ledger_rejects_replay() {
        let db = std::env::temp_dir().join(format!("gr_seal_{}.sqlite", hex_now()));
        let _ = std::fs::remove_file(&db);
        let store = Store::open(&db).unwrap();
        assert!(store
            .mark_seal_consumed("s_1", "B5_main", "n_abc")
            .unwrap());
        assert!(
            !store
                .mark_seal_consumed("s_1", "B5_main", "n_abc")
                .unwrap(),
            "exact tuple replay must be detected"
        );
        // Same session/batch, fresh nonce → allowed (distinct relay event).
        assert!(store
            .mark_seal_consumed("s_1", "B5_main", "n_def")
            .unwrap());
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn cycle_open_resume_complete_cool() {
        let db = std::env::temp_dir().join(format!("gr_cycle_{}.sqlite", hex_now()));
        let _ = std::fs::remove_file(&db);
        let store = Store::open(&db).unwrap();
        let vt = "vt_test_cycle_align_001".to_string();
        let o1 = store
            .open_cycle(None, Some(vt.clone()), Some(json!({"fe": "test"})))
            .unwrap();
        assert_eq!(o1["phase"], "new");
        let cid = o1["cycle_id"].as_str().unwrap().to_string();
        assert!(cid.starts_with("cycle_"));
        store
            .upsert_batch(
                &cid,
                "B0_bootstrap",
                "main",
                &json!({"fields": {"user_agent": "Chrome", "os_family": "linux"}}),
            )
            .unwrap();
        // Brain cool requires silicon (B10) — not complete-without-silicon
        store
            .upsert_batch(
                &cid,
                "B10_hw_curves",
                "main",
                &json!({"fields": {
                    "hw_curve_webgl": [0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8],
                    "hw_curve_audio": [0.0,0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8,0.9,0.1,0.2,0.3,0.4,0.5,0.6],
                    "residual_mean": 0.26
                }}),
            )
            .unwrap();
        // resume same cycle
        let o2 = store
            .open_cycle(None, Some(vt.clone()), Some(json!({"fe": "test"})))
            .unwrap();
        assert_eq!(o2["phase"], "active");
        assert_eq!(o2["cycle_id"], cid);
        assert_eq!(o2["resumed"], true);
        let received = store.list_received_batches(&cid).unwrap();
        assert!(received.len() >= 2);
        // complete → cool (brain-owned; silicon_ok via B10)
        store
            .save_analysis(
                &cid,
                &json!({
                    "real_band": "likely_real",
                    "analysis_terminal": true,
                    "coverage": {"coverage_complete": true},
                    "route_plan": {"stop_probe": true},
                    "product": {
                        "os": {"score": 0.8, "status": "real"},
                        "br": {"score": 0.7, "status": "real"},
                        "device": {
                            "device_id": "dh-2_testsilicon0001",
                            "device_tier": "dh",
                            "device_algo_group": "dh-2"
                        }
                    }
                }),
            )
            .unwrap();
        let done = store.complete_cycle(&cid).unwrap();
        assert_eq!(done["phase"], "complete");
        assert_eq!(done["cool_silicon_ok"], true, "brain cool requires silicon: {}", done);
        assert!(done["cool_until_ms"].as_i64().unwrap() > now_ms(), "cool_until set: {}", done);
        // identity upload rejected
        let win = store.session_window(&cid).unwrap();
        assert_eq!(win["active"], false);
        assert_eq!(win["expired_reason"], "cycle_complete");
        // open during cool
        let o3 = store
            .open_cycle(None, Some(vt.clone()), Some(json!({"fe": "test"})))
            .unwrap();
        assert_eq!(o3["phase"], "cool");
        assert_eq!(o3["skip_identity_probe"], true);
        assert_eq!(o3["cycle_id"], cid);
        // force cool expiry → new cycle
        {
            let conn = match &store.backend {
                Backend::Sqlite(s) => s.conn.lock().unwrap(),
                _ => panic!("expected sqlite"),
            };
            conn.execute(
                "UPDATE visitor_terminals SET cool_until_ms=0 WHERE vt_id=?1",
                params![vt],
            )
            .unwrap();
        }
        let o4 = store
            .open_cycle(None, Some(vt), Some(json!({"fe": "test"})))
            .unwrap();
        assert_eq!(o4["phase"], "new");
        assert_ne!(o4["cycle_id"], cid);
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn analysis_completes_cycle_helpers() {
        // Terminal without silicon must NOT close (thin empty_anchor).
        assert!(!analysis_completes_cycle(&json!({"analysis_terminal": true})));
        assert!(!analysis_completes_cycle(&json!({
            "route_plan": {"stop_probe": true},
            "coverage": {"coverage_complete": true}
        })));
        // Terminal + real_curves digest → close.
        assert!(analysis_completes_cycle(&json!({
            "analysis_terminal": true,
            "device": {"digest_path": "real_curves_v1"}
        })));
        // Residual field fragments alone must NOT close (no primary B10 / real_curves).
        assert!(!analysis_completes_cycle(&json!({
            "route_plan": {"stop_probe": true},
            "coverage": {"coverage_complete": true},
            "fields": {"residual_std": 0.09, "hw_curve_webgl": [0.1]}
        })));
        // stop+coverage + b10_present → close.
        assert!(analysis_completes_cycle(&json!({
            "route_plan": {"stop_probe": true},
            "coverage": {"coverage_complete": true},
            "b10_present": true,
            "device": {"digest_path": "real_curves_v1"}
        })));
        assert!(!analysis_completes_cycle(&json!({"session_ticket": {"t": 1}})));
        // Early ticket must not freeze cycle (short-visit multi-batch upload).
        assert!(!analysis_completes_cycle(&json!({
            "session_ticket": {"t": 1},
            "skip_session_probe": true,
            "product": {"os": {"status": "real"}, "br": {"status": "real"}}
        })));
        assert!(!analysis_completes_cycle(&json!({"real_band": "watch"})));
        // Soft probe_complete alone must NOT close cycle (was root of 410-while-uploading).
        assert!(!analysis_completes_cycle(&json!({"probe_complete": true})));
        assert!(!analysis_completes_cycle(&json!({
            "probe_complete": true,
            "route_plan": {"stop_probe": true},
            "coverage": {"coverage_complete": false}
        })));
        // 178 fix: commercial + silicon + B10x not required → close cycle (cool).
        // Thin commercial without silicon still does not close (checked above).
        assert!(analysis_completes_cycle(&json!({
            "analysis_terminal": false,
            "probe_complete": false,
            "b10_present": true,
            "product": {"device_id": "dh_165ec677551bfb41", "device_tier": "dh", "digest_path": "real_curves_v1"},
            "device": {"digest_path": "real_curves_v1", "device_id": "dh_165ec677551bfb41", "device_tier": "dh"},
            "coverage": {"coverage_complete": false},
            "route_plan": {"stop_probe": false, "b10x_must_land": false}
        })));
        // B10x still required → must NOT close.
        assert!(!analysis_completes_cycle(&json!({
            "b10_present": true,
            "product": {"device_id": "dh_165ec677551bfb41", "device_tier": "dh", "digest_path": "real_curves_v1"},
            "device": {"digest_path": "real_curves_v1", "device_id": "dh_165ec677551bfb41", "device_tier": "dh"},
            "route_plan": {"b10x_must_land": true}
        })));
        // Commercial materials still detectable as milestone.
        assert!(result_commercial_identity_final(&json!({
            "b10_present": true,
            "product": {"device_id": "dh_165ec677551bfb41", "device_tier": "dh"},
            "device": {"digest_path": "real_curves_v1"},
        })));
        // Soft stack dv + silicon residual is commercial milestone (not cycle close alone).
        assert!(result_commercial_identity_final(&json!({
            "b10_present": true,
            "product": {"device_id": "dv_abc", "device_tier": "dv"},
            "device": {"digest_path": "soft_aware_v3"},
            "fields": {"residual_std": 0.1, "hw_curve_webgl": [0.1]}
        })));
        // Brain schedule complete + silicon → close.
        assert!(analysis_completes_cycle(&json!({
            "analysis_terminal": false,
            "route_plan": {"stop_probe": true},
            "coverage": {"coverage_complete": true},
            "device": {"digest_path": "real_curves_v1"},
            "product": {"device_id": "dh_x", "device_tier": "dh"}
        })));
        // dg / empty_anchor never commercial-final.
        assert!(!result_commercial_identity_final(&json!({
            "product": {"device_id": "dg_x", "device_tier": "dg"},
            "device": {"digest_path": "empty_anchor_v1"},
            "b10_present": false
        })));
    }

    /// End-to-end lifecycle: open → multi-batch → soft complete (no 410) →
    /// terminal complete → 410 → cool open skip → reconcile corrections.
    
    #[test]
    fn analysis_report_scalars_falls_back_to_stable_candidate_when_public_id_withheld() {
        let sc = analysis_report_scalars(&json!({
            "real_band": "likely_real",
            "product": { "device_id": null, "device_tier": "multi" },
            "device": {
                "device_id": null,
                "device_id_stable_candidate": "dv0-8f227482b7-74a056e2ba-691e27fb49-da265bc1a3-e0ed3a73f1-3308a42c73-f5bb7aae0a-650a0171eb-c11a93d9e5-151439a578",
                "device_id_segments": {
                    "dv0": "dv0-8f227482b7-74a056e2ba-691e27fb49-da265bc1a3-e0ed3a73f1-3308a42c73-f5bb7aae0a-650a0171eb-c11a93d9e5-151439a578"
                },
                "device_tier": "multi"
            }
        }));
        assert_eq!(
            sc.device_id.as_deref().unwrap_or("").split('-').nth(1),
            Some("8f227482b7"),
            "ops scalar must not be empty when stable candidate exists"
        );
    }

    #[test]
    fn multi_segment_counts_as_commercial_identity_final() {
        assert!(result_commercial_identity_final(&json!({
            "b10_present": true,
            "product": {
                "device_id": "dv0-0.26-abc-def-0-linux-x86_64-c8-UTC-0-0",
                "device_tier": "multi",
                "multi_segment": true
            },
            "device": {"digest_path": "real_curves_v1"}
        })));
        assert!(result_commercial_identity_final(&json!({
            "b10_present": true,
            "device": {
                "device_id": "dv4-0.2600-abc-def-0-linux-0-c8-UTC-0-0",
                "device_tier": "multi",
                "digest_path": "real_curves_v1"
            }
        })));
        // thin multi-segment without silicon still false
        assert!(!result_commercial_identity_final(&json!({
            "product": {
                "device_id": "dv0-0-0-0-0-0-0-0-0-0-0",
                "device_tier": "multi"
            },
            "device": {"digest_path": "empty_anchor_v1"}
        })));
    }

#[test]
    fn full_status_flow_soft_then_terminal_then_410_then_cool() {
        let db = std::env::temp_dir().join(format!("gr_flow_{}.sqlite", hex_now()));
        let _ = std::fs::remove_file(&db);
        let store = Store::open(&db).unwrap();
        let vt = "vt_flow_status_001".to_string();
        let o1 = store
            .open_cycle(None, Some(vt.clone()), Some(json!({"fe": "test"})))
            .unwrap();
        let cid = o1["cycle_id"].as_str().unwrap().to_string();

        // Wave1 batches while active
        for bid in ["B0_bootstrap", "B1_conflict", "B2_hardware", "B3_system"] {
            store
                .upsert_batch(&cid, bid, "main", &json!({"fields": {"k": bid}}))
                .unwrap();
        }
        let win1 = store.session_window(&cid).unwrap();
        assert_eq!(win1["active"], true);
        assert_eq!(win1["expired_reason"], "");

        // Soft analysis: probe_complete without terminal → cycle stays active
        store
            .save_analysis(
                &cid,
                &json!({
                    "probe_complete": true,
                    "analysis_terminal": false,
                    "coverage": {"coverage_complete": true},
                    "route_plan": {"stop_probe": false},
                    "real_band": "likely_real"
                }),
            )
            .unwrap();
        let maybe = store
            .maybe_complete_cycle_from_analysis(
                &cid,
                &json!({
                    "probe_complete": true,
                    "analysis_terminal": false,
                    "coverage": {"coverage_complete": true},
                    "route_plan": {"stop_probe": false}
                }),
            )
            .unwrap();
        assert!(maybe.is_none(), "soft complete must not close cycle");
        // More uploads still accepted
        store
            .upsert_batch(&cid, "B12_anti_camouflage", "main", &json!({"fields": {}}))
            .unwrap();

        // Thin terminal without silicon must NOT close (keep uploading B10).
        let thin = store
            .maybe_complete_cycle_from_analysis(
                &cid,
                &json!({
                    "analysis_terminal": true,
                    "probe_complete": true,
                    "coverage": {"coverage_complete": true},
                    "route_plan": {"stop_probe": true},
                    "device": {"digest_path": "empty_anchor_v1", "device_tier": "dg"}
                }),
            )
            .unwrap();
        assert!(thin.is_none(), "thin terminal must not close cycle");

        // Hard materials land, then terminal + silicon → complete
        store
            .upsert_batch(
                &cid,
                "B10_hw_curves",
                "main",
                &json!({"fields": {"residual_std": 0.09, "hw_curve_webgl": [0.1]}}),
            )
            .unwrap();
        let closed = store
            .maybe_complete_cycle_from_analysis(
                &cid,
                &json!({
                    "analysis_terminal": true,
                    "probe_complete": true,
                    "coverage": {"coverage_complete": true},
                    "route_plan": {"stop_probe": true},
                    "device": {"digest_path": "real_curves_v1", "device_tier": "dh"},
                    "fields": {"residual_std": 0.09, "hw_curve_webgl": [0.1]},
                    "b10_present": true
                }),
            )
            .unwrap();
        assert!(closed.is_some());
        let win2 = store.session_window(&cid).unwrap();
        assert_eq!(win2["active"], false);
        assert_eq!(win2["expired_reason"], "cycle_complete");
        let err = store
            .upsert_batch(&cid, "B11_interaction", "main", &json!({"fields": {}}))
            .unwrap_err();
        match err {
            StoreError::SessionExpired(m) => {
                assert!(m.contains("cycle_complete"), "got {m}");
            }
            other => panic!("expected SessionExpired, got {other}"),
        }

        // Cool open (silicon complete)
        let ocool = store
            .open_cycle(None, Some(vt.clone()), Some(json!({"fe": "test"})))
            .unwrap();
        assert_eq!(ocool["phase"], "cool");
        assert_eq!(ocool["skip_identity_probe"], true);

        // Reconcile: FE still thinks active → must get halt correction
        let facts = ServerCycleFacts {
            session_id: cid.clone(),
            active: false,
            cycle_status: "complete".into(),
            expired_reason: "cycle_complete".into(),
            analysis_terminal: true,
            probe_complete: true,
            stop_probe: true,
            coverage_complete: true,
            received_batch_ids: vec!["B0_bootstrap".into()],
        };
        let client = ClientCycleView {
            session_id: cid.clone(),
            halted: Some(false),
            stop_probe: Some(false),
            last_http_status: Some(410),
            last_error_code: Some("cycle_complete".into()),
            ..Default::default()
        };
        let r = reconcile(&facts, Some(&client));
        assert!(r.fe_should_halt, "410 + complete → FE halt");
        assert!(r
            .corrections
            .iter()
            .any(|c| c.action == "fe_halt_uploads"));

        // Reconcile false cool while server would be active (fresh cycle after cool clear)
        {
            let conn = match &store.backend {
                Backend::Sqlite(s) => s.conn.lock().unwrap(),
                _ => panic!("sqlite"),
            };
            conn.execute(
                "UPDATE visitor_terminals SET cool_until_ms=0 WHERE vt_id=?1",
                params![vt],
            )
            .unwrap();
        }
        let o4 = store
            .open_cycle(None, Some(vt), Some(json!({"fe": "test"})))
            .unwrap();
        assert_eq!(o4["phase"], "new");
        let cid2 = o4["cycle_id"].as_str().unwrap().to_string();
        assert_ne!(cid2, cid);
        let facts2 = ServerCycleFacts {
            session_id: cid2.clone(),
            active: true,
            cycle_status: "active".into(),
            ..Default::default()
        };
        let false_cool = ClientCycleView {
            session_id: cid2,
            phase: Some("cool".into()),
            skip_identity: Some(true),
            halted: Some(true),
            ..Default::default()
        };
        let r2 = reconcile(&facts2, Some(&false_cool));
        assert!(r2.fe_should_resume);
        assert!(r2
            .corrections
            .iter()
            .any(|c| c.action == "fe_clear_false_cool"));

        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn maybe_complete_idempotent_and_require_active() {
        let db = std::env::temp_dir().join(format!("gr_idem_{}.sqlite", hex_now()));
        let _ = std::fs::remove_file(&db);
        let store = Store::open(&db).unwrap();
        let o = store
            .open_cycle(None, Some("vt_idem".into()), None)
            .unwrap();
        let cid = o["cycle_id"].as_str().unwrap().to_string();
        store.require_active_session(&cid).unwrap();
        // Thin terminal must not complete (no silicon).
        let thin = json!({"analysis_terminal": true});
        assert!(store
            .maybe_complete_cycle_from_analysis(&cid, &thin)
            .unwrap()
            .is_none());
        let term = json!({
            "analysis_terminal": true,
            "b10_present": true,
            "digest_path": "real_curves"
        });
        let c1 = store.maybe_complete_cycle_from_analysis(&cid, &term).unwrap();
        assert!(c1.is_some());
        // Second complete is ok (idempotent complete_cycle)
        let c2 = store.maybe_complete_cycle_from_analysis(&cid, &term).unwrap();
        assert!(c2.is_some());
        assert!(store.require_active_session(&cid).is_err());
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn api_idempotency_same_key_keeps_first_hash() {
        let db = std::env::temp_dir().join(format!("gr_api_idem_{}.sqlite", hex_now()));
        let _ = std::fs::remove_file(&db);
        let store = Store::open(&db).unwrap();
        store
            .put_api_idempotency(
                "site-a",
                "open",
                "idem_abc12345",
                "hash_a",
                200,
                &json!({"ok": true, "session_id": "s1"}),
            )
            .unwrap();
        let hit = store
            .lookup_api_idempotency("site-a", "open", "idem_abc12345")
            .unwrap()
            .expect("hit");
        assert_eq!(hit["body_hash"], "hash_a");
        assert_eq!(hit["status"], 200);
        store
            .put_api_idempotency(
                "site-a",
                "open",
                "idem_abc12345",
                "hash_b",
                200,
                &json!({"ok": true, "session_id": "s2"}),
            )
            .unwrap();
        let still = store
            .lookup_api_idempotency("site-a", "open", "idem_abc12345")
            .unwrap()
            .expect("still");
        assert_eq!(still["body_hash"], "hash_a");
        assert_eq!(still["response"]["session_id"], "s1");
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn open_ingest_evidence_roundtrip() {
        let db = std::env::temp_dir().join(format!("gr_store_test_{}.sqlite", hex_now()));
        let _ = std::fs::remove_file(&db);
        let store = Store::open(&db).unwrap();
        let open = store.open_session(None, None, None).unwrap();
        let sid = open["session_id"].as_str().unwrap().to_string();
        store
            .upsert_batch(
                &sid,
                "B0_bootstrap",
                "main",
                &json!({"fields": {"user_agent": "Chrome", "os_family": "windows"}}),
            )
            .unwrap();
        store
            .upsert_batch(
                &sid,
                "B8_gateway",
                "gateway",
                &json!({"fields": {"gateway_ua_family": "chrome"}}),
            )
            .unwrap();
        let ev = store.build_evidence(&sid).unwrap();
        assert_eq!(ev["session_id"], sid);
        assert!(ev["batches"].as_array().unwrap().len() >= 2);
        assert_eq!(ev["fields"]["user_agent"], "Chrome");
        let rev = store
            .save_analysis(&sid, &json!({"real_band": "watch"}))
            .unwrap();
        assert_eq!(rev, 1);
        let latest = store.latest_analysis(&sid).unwrap().unwrap();
        assert_eq!(latest["real_band"], "watch");
        assert_eq!(latest["analysis_rev"], 1);
        let rev2 = store
            .save_analysis(&sid, &json!({"real_band": "likely_real", "device": {"device_id": "dv_x"}}))
            .unwrap();
        assert_eq!(rev2, 2);
        let hist = store.list_analyses(&sid).unwrap();
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0]["rev"], 1);
        assert_eq!(hist[1]["rev"], 2);
        let received = store.list_received_batches(&sid).unwrap();
        assert_eq!(received.len(), 2);
        assert!(store.has_batch(&sid, "B0_bootstrap", "main").unwrap());
        let win = store.session_window(&sid).unwrap();
        assert_eq!(win["active"], true);
        assert_eq!(win["cycle_incomplete_ms"], cycle_incomplete_ms());
        // Ingest no longer auto-arms analyze (service layer schedules); arm explicitly.
        store.schedule_analyze(&sid, 0).unwrap();
        assert!(store.pending_analyze_job_count().unwrap() >= 1);
        let qs = store.analyze_queue_stats().unwrap();
        assert_eq!(qs["backend"], "sqlite");
        assert!(qs["pending"].as_i64().unwrap() >= 1);
        let _ = std::fs::remove_file(&db);
    }

    /// Multi-source B11 RPA streams differ by design — must not appear in source_conflicts.
    #[test]
    fn multi_source_rpa_batches_no_xsrc_conflict() {
        let db = std::env::temp_dir().join(format!("gr_rpa_xsrc_{}.sqlite", hex_now()));
        let _ = std::fs::remove_file(&db);
        let store = Store::open(&db).unwrap();
        let open = store.open_session(None, None, None).unwrap();
        let sid = open["session_id"].as_str().unwrap().to_string();
        store
            .upsert_batch(
                &sid,
                "B0_bootstrap",
                "main",
                &json!({"fields": {"platform": "Linux x86_64", "form_class": "desktop", "hardware_concurrency": 8}}),
            )
            .unwrap();
        store
            .upsert_batch(
                &sid,
                "B11_interaction",
                "main",
                &json!({"fields": {
                    "behavior_early_bound": true,
                    "behavior_events": [{"kind":"click","source":"main"}],
                    "behavior_count": 4,
                    "rpa_source": "main",
                    "sandbox_kind": "main",
                    "page_url": "https://demo.example/app",
                    "page_id": "p1"
                }}),
            )
            .unwrap();
        store
            .upsert_batch(
                &sid,
                "B11_interaction",
                "iframe:d1",
                &json!({"fields": {
                    "behavior_early_bound": true,
                    "behavior_events": [{"kind":"pointerdown","source":"iframe:d1"}],
                    "behavior_count": 1,
                    "rpa_source": "iframe:d1",
                    "sandbox_kind": "iframe",
                    "page_url": "https://demo.example/app",
                    "page_id": "p1"
                }}),
            )
            .unwrap();
        store
            .upsert_batch(
                &sid,
                "B11_interaction",
                "worker:d1",
                &json!({"fields": {
                    "behavior_early_bound": true,
                    "behavior_events": [{"kind":"worker_tick","source":"worker:d1"}],
                    "behavior_count": 1,
                    "rpa_source": "worker:d1",
                    "sandbox_kind": "worker",
                    "worker_rpa": true,
                    "page_url": "https://demo.example/app",
                    "page_id": "p1"
                }}),
            )
            .unwrap();
        let ev = store.build_evidence(&sid).unwrap();
        let conflicts = ev["source_conflicts"].as_array().cloned().unwrap_or_default();
        let bad: Vec<_> = conflicts
            .iter()
            .filter_map(|c| c.as_str())
            .filter(|s| {
                s.contains("behavior")
                    || s.contains("rpa_")
                    || s.contains("sandbox_kind")
                    || s.contains("page_url")
            })
            .collect();
        assert!(
            bad.is_empty(),
            "RPA surface-local keys must not be xsrc conflicts: {conflicts:?}"
        );
        // merged page fields prefer main RPA stream
        assert_eq!(ev["fields"]["rpa_source"], "main");
        assert_eq!(ev["fields"]["behavior_count"], 4);
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn analysis_storage_slims_and_prunes_history() {
        let db = std::env::temp_dir().join(format!("gr_an_slim_{}.sqlite", hex_now()));
        let _ = std::fs::remove_file(&db);
        let store = Store::open(&db).unwrap();
        let open = store.open_session(None, None, None).unwrap();
        let sid = open["session_id"].as_str().unwrap().to_string();
        // 5 revs; keep default 3 → oldest 2 pruned
        for i in 0..5 {
            let result = json!({
                "device": {"device_id": format!("dv_{i}"), "trust": {"materials": {"k": "v"}}},
                "hw_curve_webgl": [0.1, 0.2, 0.3],
                "diagnostics": {"field_utilization": {"x": i}, "analysis_quality": {"posture": "ok", "per_field": {"a": 1}}},
                "battle_log": (0..30).map(|j| json!({"j": j})).collect::<Vec<_>>(),
            });
            let rev = store.save_analysis(&sid, &result).unwrap();
            assert_eq!(rev, i + 1);
        }
        let hist = store.list_analyses(&sid).unwrap();
        assert!(
            hist.len() <= analysis_history_keep() as usize,
            "history len={} keep={}",
            hist.len(),
            analysis_history_keep()
        );
        let latest = store.latest_analysis(&sid).unwrap().unwrap();
        assert_eq!(latest["device"]["device_id"], "dv_4");
        assert!(latest.get("hw_curve_webgl").is_none());
        assert_eq!(latest["_storage"], "slim_v1");
        assert!(latest.pointer("/diagnostics/field_utilization").is_none());
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn empty_analyze_claim_is_safe() {
        let db = std::env::temp_dir().join(format!("gr_job_empty_{}.sqlite", hex_now()));
        let _ = std::fs::remove_file(&db);
        let store = Store::open(&db).unwrap();
        // No jobs: claim must return empty without error (read-only path on PG).
        let claimed = store
            .claim_due_analyze_jobs("w-empty", 4, ANALYZE_LOCK_MS)
            .unwrap();
        assert!(claimed.is_empty());
        let stats = store.analyze_queue_stats().unwrap();
        assert_eq!(stats["pending"], 0);
        assert_eq!(stats["claim_batch"], ANALYZE_CLAIM_BATCH);
        assert!(stats["idle_poll_ms_min"].as_u64().unwrap_or(0) >= 10);
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn retention_purge_batch_clamps_limit_and_is_bounded() {
        let db = std::env::temp_dir().join(format!("gr_purge_{}.sqlite", hex_now()));
        let _ = std::fs::remove_file(&db);
        let store = Store::open(&db).unwrap();
        let open = store.open_session(None, None, None).unwrap();
        let sid = open["session_id"].as_str().unwrap().to_string();
        store
            .upsert_batch(
                &sid,
                "B0_bootstrap",
                "main",
                &json!({"fields": {"os_family": "linux"}}),
            )
            .unwrap();
        store
            .save_analysis(&sid, &json!({"real_band": "watch"}))
            .unwrap();
        // limit=1 must clamp up to 10; future cutoff would delete this row but only in a batch.
        let out = store
            .retention_purge_batch(i64::MAX, 1, None, None, None)
            .unwrap();
        assert_eq!(out["ok"], true);
        assert_eq!(out["limit"], 10);
        assert!(out["deleted_analysis"].as_i64().unwrap_or(-1) <= 10);
        let out2 = store
            .retention_purge_batch(i64::MAX, 99999, None, None, None)
            .unwrap();
        assert_eq!(out2["limit"], 5000);
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn retention_purge_covers_ops_and_master_tables() {
        // iss/opus5 04-P1-6: observation_events / ops_* / api_idempotency /
        // soft_* / device_index_* / page_results must be bounded by retention.
        let db = std::env::temp_dir().join(format!("gr_purge_ext_{}.sqlite", hex_now()));
        let _ = std::fs::remove_file(&db);
        let store = Store::open(&db).unwrap();
        let open = store.open_session(None, None, None).unwrap();
        let sid = open["session_id"].as_str().unwrap().to_string();
        let old_ms = 1_000_000i64; // far past
        // Seed one old row into each newly-covered table.
        store
            .insert_observation_event(json!({
                "session_id": sid,
                "batch_id": "B0_bootstrap",
                "source": "main",
                "envelope": {"validated": {"source_kind": "fe"}},
                "created_ms": old_ms,
            }))
            .unwrap();
        store
            .insert_ops_client_event(json!({"code": "x", "ts_ms": old_ms}))
            .unwrap();
        store
            .insert_ops_server_event(json!({"code": "x", "ts_ms": old_ms}))
            .unwrap();
        store
            .put_api_idempotency("", "/v1/x", "k1", "h", 200, &json!({"ok": true}))
            .unwrap();
        let out = store
            .retention_purge_batch(
                i64::MAX - 1, // everything is "old"
                5000,
                None,
                Some(i64::MAX - 1),
                Some(i64::MAX - 1),
            )
            .unwrap();
        assert_eq!(out["ok"], true);
        assert_eq!(out["deleted_observation_events"].as_i64().unwrap_or(0), 1);
        assert_eq!(out["deleted_ops_client_events"].as_i64().unwrap_or(0), 1);
        assert_eq!(out["deleted_ops_server_events"].as_i64().unwrap_or(0), 1);
        // api_idempotency row was written with now_ms → also old under MAX cutoff.
        assert_eq!(out["deleted_api_idempotency"].as_i64().unwrap_or(0), 1);
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn dsar_erase_and_export_by_visitor_terminal() {
        // iss/opus5 05-S-5: selector cascade removes session + linked rows and
        // export returns the held data before erasure.
        let db = std::env::temp_dir().join(format!("gr_dsar_{}.sqlite", hex_now()));
        let _ = std::fs::remove_file(&db);
        let store = Store::open(&db).unwrap();
        let vt = "vt_dsar_test_1";
        let open = store.open_session(None, Some(vt.to_string()), None).unwrap();
        let sid = open["session_id"].as_str().unwrap().to_string();
        store
            .upsert_batch(
                &sid,
                "B0_bootstrap",
                "main",
                &json!({"fields": {"os_family": "linux"}}),
            )
            .unwrap();
        store
            .save_analysis(&sid, &json!({"real_band": "watch"}))
            .unwrap();
        // Export sees the session + analysis.
        let ex = store.subject_export("visitor_terminal_id", vt).unwrap();
        assert_eq!(ex["ok"], true);
        assert_eq!(ex["export"]["sessions_matched"].as_i64().unwrap_or(0), 1);
        assert!(
            !ex["export"]["analysis_results"]
                .as_array()
                .map(|a| a.is_empty())
                .unwrap_or(true)
        );
        // Erase cascades.
        let er = store.subject_erase("visitor_terminal_id", vt).unwrap();
        assert_eq!(er["ok"], true);
        assert_eq!(er["sessions_matched"].as_i64().unwrap_or(0), 1);
        assert!(er["deleted"]["sessions"].as_i64().unwrap_or(0) >= 1);
        // Export after erase is empty.
        let ex2 = store.subject_export("visitor_terminal_id", vt).unwrap();
        assert_eq!(ex2["export"]["sessions_matched"].as_i64().unwrap_or(1), 0);
        // Unknown kind rejected.
        assert!(store.subject_erase("email", "x@y.z").is_err());
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn dsar_erase_by_client_ip_matches_masked_form() {
        // S-4 masks client_ip to /24 at ingest; a DSAR selector in raw form
        // must still match the masked stored row (and vice versa).
        let db = std::env::temp_dir().join(format!("gr_dsar_ip_{}.sqlite", hex_now()));
        let _ = std::fs::remove_file(&db);
        let store = Store::open(&db).unwrap();
        let open = store.open_session(None, None, None).unwrap();
        let sid = open["session_id"].as_str().unwrap().to_string();
        store
            .upsert_batch(&sid, "B0", "main", &json!({"fields": {}}))
            .unwrap();
        // Persist a MASKED client_ip on an ops event (post-S-4 ingest form).
        store
            .insert_ops_server_event(json!({
                "session_id": sid,
                "code": "dsar_test",
                "client_ip": "203.0.113.0/24",
            }))
            .unwrap();
        // Raw selector must still match the masked stored row.
        let er = store.subject_erase("client_ip", "203.0.113.77").unwrap();
        assert!(
            er["sessions_matched"].as_i64().unwrap_or(0) >= 1,
            "raw selector must match masked stored row: {er}"
        );
        assert!(er["deleted"]["ops_server_events"].as_i64().unwrap_or(0) >= 1);
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn schedule_analyze_pull_earlier_keeps_milestone() {
        // PullEarlier: milestone 800ms not wiped by later PullEarlier 60s.
        let db = std::env::temp_dir().join(format!("gr_pull_{}.sqlite", hex_now()));
        let _ = std::fs::remove_file(&db);
        let store = Store::open(&db).unwrap();
        let o = store
            .open_cycle(
                None,
                Some("vt_pull_earlier".into()),
                Some(json!({"fe": "test"})),
            )
            .unwrap();
        let sid = o["cycle_id"].as_str().unwrap().to_string();
        store
            .schedule_analyze_merge(&sid, 800, AnalyzeDueMerge::PullEarlier)
            .unwrap();
        store
            .schedule_analyze_merge(&sid, 60_000, AnalyzeDueMerge::PullEarlier)
            .unwrap();
        let due: i64 = match &store.backend {
            Backend::Sqlite(s) => {
                let conn = s.conn.lock().unwrap();
                conn.query_row(
                    "SELECT due_ms FROM analyze_jobs WHERE session_id=?1",
                    params![sid],
                    |r| r.get(0),
                )
                .unwrap()
            }
            _ => panic!("expected sqlite"),
        };
        let now = now_ms();
        assert!(
            due <= now + 5_000,
            "due should stay near milestone (got due-now={})",
            due - now
        );
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn schedule_analyze_idle_reset_pushes_due_later() {
        // Idle re-arm after upload must reset quiet window (not stuck at first arm).
        let db = std::env::temp_dir().join(format!("gr_idle_{}.sqlite", hex_now()));
        let _ = std::fs::remove_file(&db);
        let store = Store::open(&db).unwrap();
        let o = store
            .open_cycle(
                None,
                Some("vt_idle_reset".into()),
                Some(json!({"fe": "test"})),
            )
            .unwrap();
        let sid = o["cycle_id"].as_str().unwrap().to_string();
        let t0 = now_ms();
        store
            .schedule_analyze_merge(&sid, 60_000, AnalyzeDueMerge::IdleReset)
            .unwrap();
        // Simulate ~10s later upload re-arm (force first due into past-imminent? no:
        // set existing due far out by first arm, then second arm later).
        // Manually age the due_ms backward so second arm is later than "now" window.
        {
            let conn = match &store.backend {
                Backend::Sqlite(s) => s.conn.lock().unwrap(),
                _ => panic!("sqlite"),
            };
            // Pretend first arm was 10s ago: due was t0+60s, still ~50s out (> imminent 10s)
            conn.execute(
                "UPDATE analyze_jobs SET due_ms=?1 WHERE session_id=?2",
                params![t0 + 50_000, sid],
            )
            .unwrap();
        }
        let t1 = now_ms();
        store
            .schedule_analyze_merge(&sid, 60_000, AnalyzeDueMerge::IdleReset)
            .unwrap();
        let due: i64 = match &store.backend {
            Backend::Sqlite(s) => {
                let conn = s.conn.lock().unwrap();
                conn.query_row(
                    "SELECT due_ms FROM analyze_jobs WHERE session_id=?1",
                    params![sid],
                    |r| r.get(0),
                )
                .unwrap()
            }
            _ => panic!("sqlite"),
        };
        // Must be ≈ t1+60s, not stuck at t0+50s
        assert!(
            due >= t1 + 55_000,
            "idle reset must push due later (due-t1={}, due-t0_arm={})",
            due - t1,
            due - (t0 + 50_000)
        );
        assert!(due <= t1 + 65_000, "due ≈ now+60s");
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn schedule_analyze_idle_does_not_delay_milestone() {
        let db = std::env::temp_dir().join(format!("gr_idle_ms_{}.sqlite", hex_now()));
        let _ = std::fs::remove_file(&db);
        let store = Store::open(&db).unwrap();
        let o = store
            .open_cycle(None, Some("vt_idle_ms".into()), Some(json!({})))
            .unwrap();
        let sid = o["cycle_id"].as_str().unwrap().to_string();
        let t0 = now_ms();
        store
            .schedule_analyze_merge(&sid, 800, AnalyzeDueMerge::PullEarlier)
            .unwrap();
        store
            .schedule_analyze_merge(&sid, 60_000, AnalyzeDueMerge::IdleReset)
            .unwrap();
        let due: i64 = match &store.backend {
            Backend::Sqlite(s) => {
                let conn = s.conn.lock().unwrap();
                conn.query_row(
                    "SELECT due_ms FROM analyze_jobs WHERE session_id=?1",
                    params![sid],
                    |r| r.get(0),
                )
                .unwrap()
            }
            _ => panic!("sqlite"),
        };
        assert!(
            due <= t0 + 5_000,
            "idle must not delay imminent milestone due (got {})",
            due - t0
        );
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn schedule_analyze_same_due_is_idempotent() {
        let db = std::env::temp_dir().join(format!("gr_job_idem_{}.sqlite", hex_now()));
        let _ = std::fs::remove_file(&db);
        let store = Store::open(&db).unwrap();
        let open = store.open_session(None, None, None).unwrap();
        let sid = open["session_id"].as_str().unwrap().to_string();
        store.schedule_analyze(&sid, 0).unwrap();
        store.schedule_analyze(&sid, 0).unwrap(); // same due bucket — no-op update path
        assert_eq!(store.pending_analyze_job_count().unwrap(), 1);
        let claimed = store
            .claim_due_analyze_jobs("w1", 4, ANALYZE_LOCK_MS)
            .unwrap();
        assert_eq!(claimed, vec![sid.clone()]);
        let _ = store.complete_analyze_job(&sid, "w1").unwrap();
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn analyze_job_debounce_claim_complete() {
        let db = std::env::temp_dir().join(format!("gr_job_{}.sqlite", hex_now()));
        let _ = std::fs::remove_file(&db);
        let store = Store::open(&db).unwrap();
        let open = store.open_session(None, None, None).unwrap();
        let sid = open["session_id"].as_str().unwrap().to_string();
        store
            .upsert_batch(
                &sid,
                "B0_bootstrap",
                "main",
                &json!({"fields": {"user_agent": "x"}}),
            )
            .unwrap();
        store.schedule_analyze(&sid, 0).unwrap();
        let claimed = store.claim_due_analyze_jobs("w1", 10, ANALYZE_LOCK_MS).unwrap();
        assert_eq!(claimed, vec![sid.clone()]);
        let claimed2 = store.claim_due_analyze_jobs("w2", 10, ANALYZE_LOCK_MS).unwrap();
        assert!(claimed2.is_empty());
        let still = store.complete_analyze_job(&sid, "w1").unwrap();
        assert!(!still);
        assert_eq!(store.pending_analyze_job_count().unwrap(), 0);
        store.schedule_analyze(&sid, 0).unwrap();
        let c = store.claim_due_analyze_jobs("w1", 5, ANALYZE_LOCK_MS).unwrap();
        assert_eq!(c.len(), 1);
        // PullEarlier would keep the earlier due; Replace pushes due into the future so
        // complete_analyze_job leaves the job pending (still=true).
        store
            .schedule_analyze_merge(&sid, 5000, AnalyzeDueMerge::Replace)
            .unwrap();
        let still2 = store.complete_analyze_job(&sid, "w1").unwrap();
        assert!(still2);
        assert_eq!(store.pending_analyze_job_count().unwrap(), 1);
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn gateway_fields_isolated_from_main_ua() {
        let db = std::env::temp_dir().join(format!("gr_gw_iso_{}.sqlite", hex_now()));
        let _ = std::fs::remove_file(&db);
        let store = Store::open(&db).unwrap();
        let open = store.open_session(None, None, None).unwrap();
        let sid = open["session_id"].as_str().unwrap().to_string();
        store
            .upsert_batch(
                &sid,
                "B8_gateway",
                "gateway",
                &json!({"fields": {"user_agent": "Googlebot/2.1 (+http://www.google.com/bot.html)"}}),
            )
            .unwrap();
        store
            .upsert_batch(
                &sid,
                "B0_bootstrap",
                "main",
                &json!({"fields": {
                    "user_agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120.0.0.0 Safari/537.36",
                    "os_family": "windows",
                    "form_class": "desktop",
                    "screen_width": 1920,
                    "screen_height": 1080,
                    "timezone": "Asia/Shanghai",
                    "hardware_concurrency": 8,
                    "webgl_unmasked_renderer": "ANGLE",
                    "automation": {"webdriver": false}
                }}),
            )
            .unwrap();
        let ev = store.build_evidence(&sid).unwrap();
        assert_eq!(
            ev["gateway_fields"]["user_agent"].as_str().unwrap(),
            "Googlebot/2.1 (+http://www.google.com/bot.html)",
            "gateway UA must not be overwritten by main"
        );
        assert!(
            ev["fields"]["user_agent"]
                .as_str()
                .unwrap()
                .contains("Chrome"),
            "main claim UA should be Chrome"
        );
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn expired_session_blocks_upsert_and_analyze() {
        let db = std::env::temp_dir().join(format!("gr_exp_{}.sqlite", hex_now()));
        let _ = std::fs::remove_file(&db);
        let store = Store::open(&db).unwrap();
        let open = store.open_session(None, None, None).unwrap();
        let sid = open["session_id"].as_str().unwrap().to_string();
        store
            .upsert_batch(
                &sid,
                "B0_bootstrap",
                "main",
                &json!({"fields": {"user_agent": "x"}}),
            )
            .unwrap();
        // complete cycle → identity window closed
        store.complete_cycle(&sid).unwrap();
        let w = store.session_window(&sid).unwrap();
        assert_eq!(w["active"], false);
        assert_eq!(w["expired_reason"], "cycle_complete");
        let err = store
            .upsert_batch(
                &sid,
                "B1_conflict",
                "main",
                &json!({"fields": {"webdriver": false}}),
            )
            .unwrap_err();
        assert!(matches!(err, StoreError::SessionExpired(_)), "{err:?}");
        // incomplete TTL: force created beyond 72h
        let open2 = store
            .open_cycle(None, Some("vt_ttl_test".into()), None)
            .unwrap();
        let sid2 = open2["cycle_id"].as_str().unwrap().to_string();
        let now = now_ms();
        store
            .force_session_times(&sid2, now - cycle_incomplete_ms() - 1000, now - 1000)
            .unwrap();
        let w2 = store.session_window(&sid2).unwrap();
        assert_eq!(w2["expired_reason"], "incomplete_ttl");
        assert_eq!(w2["active"], false);
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn fields_by_source_detects_worker_main_ua_conflict() {
        let db = std::env::temp_dir().join(format!("gr_xsrc_{}.sqlite", hex_now()));
        let _ = std::fs::remove_file(&db);
        let store = Store::open(&db).unwrap();
        let open = store.open_session(None, None, None).unwrap();
        let sid = open["session_id"].as_str().unwrap().to_string();
        store
            .upsert_batch(
                &sid,
                "B0_bootstrap",
                "main",
                &json!({"fields": {"user_agent": "Mozilla/5.0 Chrome/120.0.0.0"}}),
            )
            .unwrap();
        store
            .upsert_batch(
                &sid,
                "B7_worker",
                "worker",
                &json!({"fields": {"user_agent": "Mozilla/5.0 Firefox/121.0"}}),
            )
            .unwrap();
        let ev = store.build_evidence(&sid).unwrap();
        assert_eq!(
            ev["fields_by_source"]["main"]["user_agent"],
            "Mozilla/5.0 Chrome/120.0.0.0"
        );
        assert_eq!(
            ev["fields_by_source"]["worker"]["user_agent"],
            "Mozilla/5.0 Firefox/121.0"
        );
        let conflicts = ev["source_conflicts"].as_array().unwrap();
        assert!(
            conflicts
                .iter()
                .any(|c| c.as_str() == Some("source_conflict:user_agent")),
            "expected source_conflict:user_agent, got {conflicts:?}"
        );
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn postgres_roundtrip_if_available() {
        let dsn = gr_abi::env::get("TEST_DATABASE_URL").unwrap_or_else(|| {
            "postgresql://maxprobe:maxprobe@127.0.0.1:25432/greenv5".into()
        });
        let store = match Store::open_postgres(&dsn) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("skip postgres test: {e}");
                return;
            }
        };
        assert_eq!(store.backend_name(), "postgres");
        let open = store
            .open_session(None, None, Some(json!({"inject_path": "test"})))
            .unwrap();
        let sid = open["session_id"].as_str().unwrap().to_string();
        store
            .upsert_batch(
                &sid,
                "B0_bootstrap",
                "main",
                &json!({"fields": {"user_agent": "ChromePG", "os_family": "linux"}}),
            )
            .unwrap();
        store
            .upsert_batch(
                &sid,
                "B8_gateway",
                "gateway",
                &json!({"fields": {"user_agent": "Googlebot/2.1"}}),
            )
            .unwrap();
        let ev = store.build_evidence(&sid).unwrap();
        assert_eq!(ev["fields"]["user_agent"], "ChromePG");
        assert_eq!(ev["gateway_fields"]["user_agent"], "Googlebot/2.1");
        store.schedule_analyze(&sid, 0).unwrap();
        let claimed = store
            .claim_due_analyze_jobs("pg-w1", 5, ANALYZE_LOCK_MS)
            .unwrap();
        assert_eq!(claimed, vec![sid.clone()]);
        let claimed2 = store
            .claim_due_analyze_jobs("pg-w2", 5, ANALYZE_LOCK_MS)
            .unwrap();
        assert!(claimed2.is_empty(), "SKIP LOCKED must prevent double claim");
        let rev = store
            .save_analysis(&sid, &json!({"real_band": "watch"}))
            .unwrap();
        assert_eq!(rev, 1);
        let still = store.complete_analyze_job(&sid, "pg-w1").unwrap();
        assert!(!still);
        let qs = store.analyze_queue_stats().unwrap();
        assert_eq!(qs["backend"], "postgres");
        let latest = store.latest_analysis(&sid).unwrap().unwrap();
        assert_eq!(latest["real_band"], "watch");
    }

    #[test]
    fn device_index_sqlite_upsert_export_contract() {
        let db = std::env::temp_dir().join(format!("gr_didx_{}.sqlite", hex_now()));
        let _ = std::fs::remove_file(&db);
        let store = Store::open(&db).unwrap();
        let tenant = "tenant_sqlite_lab";
        let up = store
            .device_index_upsert(
                tenant,
                "dv_lab_1",
                &json!({"os_instance_hash": "os_a", "unit": "u1"}),
                &["bk_u1".into(), "bk_os_a".into()],
            )
            .unwrap();
        assert_eq!(up["ok"], true);
        let exp = store.device_index_export(tenant).unwrap();
        assert_eq!(exp["version"], "file_device_index_v1");
        assert_eq!(exp["tenant_id"], tenant);
        assert!(exp["devices"].get("dv_lab_1").is_some());
        assert!(exp["index"].get("bk_u1").is_some());
        let _ = std::fs::remove_file(&db);
    }

    /// Local lab PG (:25432 greenv5 by default) — single-region DeviceIndex path.
    /// Skip when GR_DATABASE_URL unset and default DSN unreachable.
    #[test]
    fn device_index_postgres_lab_upsert_export() {
        let dsn = gr_abi::env::get("DATABASE_URL").unwrap_or_else(|| {
            "postgresql://maxprobe:maxprobe@127.0.0.1:25432/greenv5".into()
        });
        let store = match Store::open_postgres(&dsn) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("skip pg device_index: {e}");
                return;
            }
        };
        let tenant = format!("lab_didx_{}", hex_now());
        let did = format!("dv_pg_{}", hex_now());
        let up = store
            .device_index_upsert(
                &tenant,
                &did,
                &json!({"residual_mean": 0.5, "unit_surface_id": "u_pg"}),
                &["bk_pg_unit".into()],
            )
            .unwrap();
        assert_eq!(up["ok"], true);
        assert_eq!(up["backend"], "postgres");
        let exp = store.device_index_export(&tenant).unwrap();
        assert_eq!(exp["version"], "file_device_index_v1");
        assert_eq!(exp["tenant_id"], tenant);
        assert!(
            exp["devices"].get(&did).is_some(),
            "export missing device: {exp}"
        );
        assert_eq!(exp["source"], "gr_store_postgres_lab");
        // tenant id is unique per run; rows are lab-scoped noise only
    }

    /// Shared multi-node rate-limit counter: atomic per-window increments and
    /// window-change reset. Skip unless GR_TEST_DATABASE_URL points at a lab PG.
    #[test]
    fn shared_rate_limit_window_if_available() {
        let Some(dsn) = gr_abi::env::get("TEST_DATABASE_URL") else {
            eprintln!("skip shared rate limit: GR_TEST_DATABASE_URL unset");
            return;
        };
        let store = match Store::open_postgres(&dsn) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("skip shared rate limit: {e}");
                return;
            }
        };
        let key = format!("rl_test_{}", hex_now());
        let w = now_ms() / 60_000;
        assert_eq!(store.bump_rate_limit_window(&key, w).unwrap(), 1);
        assert_eq!(store.bump_rate_limit_window(&key, w).unwrap(), 2);
        // Window rollover resets the counter.
        assert_eq!(store.bump_rate_limit_window(&key, w + 1).unwrap(), 1);
        // SQLite (single-process) intentionally reports the local marker error.
        let tmp = std::env::temp_dir().join(format!("gr_rl_local_{}.sqlite", hex_now()));
        let local = Store::open(&tmp).unwrap();
        assert!(local.bump_rate_limit_window(&key, w).is_err());
        let _ = std::fs::remove_file(&tmp);
    }

    /// Multi-process acceptance at the store level: two independent PgStore
    /// connections (each with its own dedicated DB thread, exactly like two
    /// processes) bump the same key/window concurrently. Returned counts must be
    /// a perfect 1..N sequence — any duplicate/lost update would mean the shared
    /// quota is not atomic across nodes. Skip unless GR_TEST_DATABASE_URL is set.
    #[test]
    fn shared_rate_limit_window_concurrent_if_available() {
        let Some(dsn) = gr_abi::env::get("TEST_DATABASE_URL") else {
            eprintln!("skip concurrent rate limit: GR_TEST_DATABASE_URL unset");
            return;
        };
        let a = match Store::open_postgres(&dsn) {
            Ok(s) => std::sync::Arc::new(s),
            Err(e) => {
                eprintln!("skip concurrent rate limit: {e}");
                return;
            }
        };
        let b = match Store::open_postgres(&dsn) {
            Ok(s) => std::sync::Arc::new(s),
            Err(e) => {
                eprintln!("skip concurrent rate limit: {e}");
                return;
            }
        };
        let key = format!("rl_con_{}", hex_now());
        let w = now_ms() / 60_000;
        let counts = std::sync::Arc::new(std::sync::Mutex::new(Vec::<i64>::new()));
        let mut handles = Vec::new();
        for i in 0..4 {
            let store = if i % 2 == 0 { a.clone() } else { b.clone() };
            let counts = counts.clone();
            let key = key.clone();
            handles.push(std::thread::spawn(move || {
                for _ in 0..25 {
                    let n = store.bump_rate_limit_window(&key, w).unwrap();
                    counts.lock().unwrap().push(n);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let mut v = counts.lock().unwrap().clone();
        assert_eq!(v.len(), 100, "all bumps must return");
        v.sort_unstable();
        for (i, n) in v.iter().enumerate() {
            assert_eq!(
                *n,
                (i as i64) + 1,
                "shared counter not atomic across connections at slot {i}"
            );
        }
        // Window rollover resets on the second connection too.
        assert_eq!(a.bump_rate_limit_window(&key, w + 1).unwrap(), 1);
    }


    #[test]
    fn cycle_cool_scoped_by_product_version() {
        let db = std::env::temp_dir().join(format!("gr_cycle_ver_{}.sqlite", hex_now()));
        let _ = std::fs::remove_file(&db);
        let store = Store::open(&db).unwrap();
        let vt = "vt_test_ver_cool_001".to_string();
        let o1 = store
            .open_cycle(
                None,
                Some(vt.clone()),
                Some(json!({"fe": "test", "product_version": "v-a"})),
            )
            .unwrap();
        let cid = o1["cycle_id"].as_str().unwrap().to_string();
        // Brain cool requires silicon (B10) — version scope is orthogonal.
        store
            .upsert_batch(
                &cid,
                "B10_hw_curves",
                "main",
                &json!({"fields": {
                    "hw_curve_webgl": [0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8],
                    "residual_mean": 0.26
                }}),
            )
            .unwrap();
        store
            .save_analysis(
                &cid,
                &json!({
                    "product_version": "v-a",
                    "analysis_terminal": true,
                    "real_band": "likely_real",
                    "product": {
                        "os": {"score": 0.8},
                        "device": {"device_id": "dh-1_vercooltest01", "device_tier": "dh"}
                    }
                }),
            )
            .unwrap();
        let done = store.complete_cycle(&cid).unwrap();
        assert_eq!(done["product_version"], "v-a");
        assert_eq!(done["cool_silicon_ok"], true, "silicon cool: {}", done);
        assert!(done["cool_until_ms"].as_i64().unwrap() > now_ms());
        // same version → cool
        let o2 = store
            .open_cycle(
                None,
                Some(vt.clone()),
                Some(json!({"fe": "test", "product_version": "v-a"})),
            )
            .unwrap();
        assert_eq!(o2["phase"], "cool", "same version should cool: {}", o2);
        assert_eq!(o2["skip_identity_probe"], true);
        // version bump → re-probe (not cool)
        let o3 = store
            .open_cycle(
                None,
                Some(vt.clone()),
                Some(json!({"fe": "test", "product_version": "v-b"})),
            )
            .unwrap();
        assert_ne!(o3["phase"], "cool", "version change must re-probe: {}", o3);
        assert_eq!(o3["skip_identity_probe"], false);
        assert!(
            o3["meta"]["cool_invalidated_reason"].as_str().is_some()
                || o3["phase"] == "new"
                || o3["phase"] == "active",
            "expected re-open after version change: {}",
            o3
        );
        assert_eq!(o3["force_identity_probe"], true);
        // Sticky complete cycle id under new product_version must be superseded (no user cookie clear).
        let o4 = store
            .open_cycle(
                Some(cid.clone()),
                Some(vt.clone()),
                Some(json!({"fe": "test", "product_version": "v-b"})),
            )
            .unwrap();
        assert_ne!(o4["session_id"].as_str().unwrap_or(""), cid, "must not reopen complete bag");
        assert_eq!(o4["skip_identity_probe"], false);
        assert_eq!(o4["client_hint_superseded"], true);
        assert_eq!(o4["force_identity_probe"], true);
        assert_eq!(
            o4["reprobe_reason"].as_str().unwrap_or(""),
            "cycle_complete_superseded"
        );
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn session_visitor_terminal_immutable_on_conflict() {
        let db = std::env::temp_dir().join(format!("gr_vt_immut_{}.sqlite", hex_now()));
        let _ = std::fs::remove_file(&db);
        let store = Store::open(&db).unwrap();
        let open = store
            .open_session(
                Some("sess_vt_lock".into()),
                Some("vt_original".into()),
                Some(json!({"inject_path": "nginx"})),
            )
            .unwrap();
        assert_eq!(open["visitor_terminal_id"], "vt_original");
        let again = store
            .open_session(
                Some("sess_vt_lock".into()),
                Some("vt_attacker_or_retry".into()),
                Some(json!({"inject_path": "unknown"})),
            )
            .unwrap();
        assert_eq!(
            again["visitor_terminal_id"], "vt_original",
            "ON CONFLICT must keep the first visitor_terminal_id: {}",
            again
        );
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn cycle_compatible_with_product_version_unit() {
        assert!(cycle_compatible_with_product_version(
            &json!({"product_version": "v1"}),
            "v1"
        ));
        assert!(!cycle_compatible_with_product_version(
            &json!({"product_version": "v1"}),
            "v2"
        ));
        assert!(!cycle_compatible_with_product_version(&json!({}), "v2"));
        assert!(cycle_compatible_with_product_version(
            &json!({"product_version": "v1"}),
            ""
        ));
    }

}
