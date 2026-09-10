//! Hot (in-memory L1) probe materials + demotion helpers for cold JSON storage.
//!
//! # Multi-worker / single-host model
//!
//! | Tier | Where | Shared across workers? | Role |
//! |------|-------|------------------------|------|
//! | **L1 Hot** | process `HotProbeCache` | No (each worker own map) | demote tracking, ops peek, promote cache |
//! | **L2 Shared warm** | PG `probe_batches` | **Yes** | **analyze / build_evidence** source of truth |
//! | **L3 Cold** | PG `probe_cold` | **Yes** | idle archive + cross-query; TTL purge |
//!
//! Analysis **must** use L2 (already does via `build_evidence`). L1 is never
//! the sole source for multi-worker analyze — no Redis required for correctness.
//! On VT re-upload, workers **promote L3→L1** from shared cold (TTL-filtered).
//!
//! Hot key: `vtid` (primary) and optional `session_id` index.
//! Idle demotion: if `last_update_ms` older than `idle_ms`, drop L1 only.

use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// L1 hot idle before demote from process memory (aligns with session inactivity 30m).
/// Override: `GR_HOT_IDLE_MS`.
pub fn hot_idle_ms() -> i64 {
    let rt = crate::runtime_cfg::get_runtime_cfg().hot_idle_ms;
    if rt > 0 && crate::runtime_cfg::config_version() > 0 {
        return rt;
    }
    gr_abi::env::get("HOT_IDLE_MS")
        .and_then(|s| s.parse().ok())
        .filter(|&n| n >= 0)
        .unwrap_or(DEFAULT_HOT_IDLE_MS)
}

/// Default L1 hot idle (30 minutes) — same order as `SESSION_INACTIVITY_MS`.
pub const DEFAULT_HOT_IDLE_MS: i64 = 30 * 60 * 1000;

/// L3 cold retention (7 days). After this, cold rows are purged.
/// Override: `GR_COLD_TTL_MS`. Must be ≥ cycle incomplete for late re-promote.
pub fn cold_ttl_ms() -> i64 {
    let rt = crate::runtime_cfg::get_runtime_cfg().cold_ttl_ms;
    if rt > 0 && crate::runtime_cfg::config_version() > 0 {
        return rt;
    }
    gr_abi::env::get("COLD_TTL_MS")
        .and_then(|s| s.parse().ok())
        .filter(|&n| n >= 60_000)
        .unwrap_or(DEFAULT_COLD_TTL_MS)
}

/// Default cold TTL: 7 days.
pub const DEFAULT_COLD_TTL_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// L1 hot map VT cap (flood bound). 0 = unbounded. Runtime cfg (panel hot)
/// seeds from `GR_HOT_MAX_VTS`. Eviction picks the most-idle entries — L2/L3
/// are already durable and per-batch ingest arms exist, so silent L1 drop is
/// safe (the pre-cold sweep arm only tops up result-less sessions).
pub fn hot_max_vts() -> i64 {
    let rt = crate::runtime_cfg::get_runtime_cfg().hot_max_vts;
    if crate::runtime_cfg::config_version() > 0 && rt >= 0 {
        return rt;
    }
    gr_abi::env::get("HOT_MAX_VTS")
        .and_then(|s| s.trim().parse::<i64>().ok())
        .filter(|&n| n >= 0)
        .unwrap_or(8192)
}

/// How far back promote-from-cold looks for a VT (default = hot idle × 48 ≈ 24h).
pub fn cold_promote_window_ms() -> i64 {
    let rt = crate::runtime_cfg::get_runtime_cfg().cold_promote_window_ms;
    if rt > 0 && crate::runtime_cfg::config_version() > 0 {
        return rt;
    }
    gr_abi::env::get("COLD_PROMOTE_WINDOW_MS")
        .and_then(|s| s.parse().ok())
        .filter(|&n| n >= 0)
        .unwrap_or(DEFAULT_COLD_PROMOTE_WINDOW_MS)
}

pub const DEFAULT_COLD_PROMOTE_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;

/// Full timeout matrix for open/ops/FE alignment.
pub fn timeout_matrix_json() -> Value {
    json!({
        "schema": "gr_timeout_matrix_v1",
        "tiers": {
            "l1_hot_process": {
                "idle_ms": hot_idle_ms(),
                "default_ms": DEFAULT_HOT_IDLE_MS,
                "shared_across_workers": false,
                "note": "process memory only; demote drops L1; dual-write already on L2+L3"
            },
            "l2_shared_warm_batches": {
                "primary_for_analyze": true,
                "shared_across_workers": true,
                "cycle_incomplete_ms": crate::runtime_cfg::cycle_incomplete_ms(),
                "session_inactivity_ms": crate::SESSION_INACTIVITY_MS,
                "note": "probe_batches — build_evidence / analyze workers all read this"
            },
            "l3_cold": {
                "ttl_ms": cold_ttl_ms(),
                "default_ttl_ms": DEFAULT_COLD_TTL_MS,
                "promote_window_ms": cold_promote_window_ms(),
                "shared_across_workers": true,
                "note": "probe_cold compact+deflate; purged after ttl; promote→L1 on VT re-upload"
            }
        },
        "session_cycle": {
            "cycle_cool_ms": crate::runtime_cfg::cycle_cool_ms(),
            "cycle_incomplete_ms": crate::runtime_cfg::cycle_incomplete_ms(),
            "session_inactivity_ms": crate::runtime_cfg::session_inactivity_ms_rt(),
            "session_hard_max_ms": crate::runtime_cfg::session_hard_max_ms_rt(),
            "config_version": crate::runtime_cfg::config_version(),
            "session_ticket_ttl_ms": 24 * 60 * 60 * 1000,
            "challenge_seed_ttl_ms": 120_000
        },
        "browser_fe": {
            "cool_localstorage_key": "gr_probe_cool_until_v1",
            "cool_product_version_key": "gr_product_version_v1",
            "cool_until_from": "open.cool_until_ms (= cycle complete + cycle_cool_ms)",
            "cool_scoped_by_product_version": true,
            "cool_reprobe_on_version_change": "same product_version + cool → skip; version bump without data for new version → re-probe/upload/analyze",
            "on_cool_expired": "clear cool → full identity re-probe",
            "vt_localstorage_key": "gr_visitor_terminal_v1",
            "vt_cookie_max_age_s": 31536000,
            "rpa_idle_flush_ms": 30_000,
            "script_cache_bust": "query ?v= from boot version"
        },
        "analyze_poll": {
            "debounce_ms": crate::ANALYZE_DEBOUNCE_MS,
            "lock_ms": crate::ANALYZE_LOCK_MS,
            "claim_batch": crate::ANALYZE_CLAIM_BATCH,
            "idle_poll_ms_min": crate::ANALYZE_IDLE_POLL_MS_MIN,
            "idle_poll_ms_max": crate::ANALYZE_IDLE_POLL_MS_MAX,
            "note": "empty-queue exponential backoff 40→500ms; claim uses EXISTS then single UPDATE LIMIT n"
        },
        "write_amp": {
            "analysis_history_keep": crate::analysis_history_keep(),
            "session_touch_min_interval_ms": crate::session_touch_min_interval_ms(),
            "analysis_storage": "slim_v1 + optional z1 deflate in result_json",
            "probe_cold": "skip update when fields_json+payload_z unchanged",
            "probe_batches": "skip update when payload_json unchanged",
            "sessions": "throttle updated_ms touch; IP change always writes"
        },
        "align": {
            "hot_idle_eq_session_inactivity": hot_idle_ms() == crate::SESSION_INACTIVITY_MS
                || hot_idle_ms() == DEFAULT_HOT_IDLE_MS,
            "cold_ttl_ge_cycle_incomplete": cold_ttl_ms() >= crate::runtime_cfg::cycle_incomplete_ms(),
            "fe_cool_eq_cycle_cool": true
        }
    })
}

#[derive(Debug, Clone)]
pub struct HotProbeEntry {
    pub visitor_terminal_id: String,
    pub session_id: String,
    pub last_update_ms: i64,
    /// batch_id → compact field map + meta
    pub batches: HashMap<String, Value>,
    pub client_ip: Option<String>,
}

#[derive(Debug, Default)]
pub struct HotProbeCache {
    by_vt: Mutex<HashMap<String, HotProbeEntry>>,
    by_session: Mutex<HashMap<String, String>>, // session_id → vtid
}

impl HotProbeCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert_batch(
        &self,
        visitor_terminal_id: &str,
        session_id: &str,
        batch_id: &str,
        payload: &Value,
        client_ip: Option<&str>,
    ) {
        let ts = now_ms();
        let mut by_vt = self.by_vt.lock().unwrap();
        let entry = by_vt
            .entry(visitor_terminal_id.to_string())
            .or_insert_with(|| HotProbeEntry {
                visitor_terminal_id: visitor_terminal_id.to_string(),
                session_id: session_id.to_string(),
                last_update_ms: ts,
                batches: HashMap::new(),
                client_ip: client_ip.map(|s| s.to_string()),
            });
        entry.session_id = session_id.to_string();
        entry.last_update_ms = ts;
        if let Some(ip) = client_ip {
            if !ip.is_empty() {
                entry.client_ip = Some(ip.to_string());
            }
        }
        // Store compact: prefer fields object for hot queries
        let compact = compact_probe_payload(payload);
        entry.batches.insert(batch_id.to_string(), compact);
        // Flood bound: cap distinct VTs in L1, evicting the most-idle first.
        // Eviction is silent — every batch ingest already armed its session and
        // L2/L3 hold the durable copies (no data loss, only cache pressure off).
        let cap = hot_max_vts() as usize;
        if cap > 0 && by_vt.len() > cap {
            let mut idle: Vec<(String, i64)> = by_vt
                .iter()
                .map(|(k, e)| (k.clone(), e.last_update_ms))
                .collect();
            idle.sort_by_key(|(_, t)| *t); // oldest activity first
            let over = by_vt.len() - cap;
            let mut victims: Vec<String> = Vec::with_capacity(over.min(64).max(0));
            for (k, _) in idle.into_iter().take(over.min(64)) {
                victims.push(k);
            }
            let mut by_s = self.by_session.lock().unwrap();
            for k in victims {
                if k == visitor_terminal_id {
                    continue; // never evict the entry we just touched
                }
                if let Some(e) = by_vt.remove(&k) {
                    by_s.remove(&e.session_id);
                }
            }
            drop(by_s);
        }
        drop(by_vt);
        let mut by_s = self.by_session.lock().unwrap();
        by_s.insert(session_id.to_string(), visitor_terminal_id.to_string());
    }

    pub fn get_by_vt(&self, vtid: &str) -> Option<HotProbeEntry> {
        self.by_vt.lock().unwrap().get(vtid).cloned()
    }

    pub fn get_by_session(&self, session_id: &str) -> Option<HotProbeEntry> {
        let vt = self.by_session.lock().unwrap().get(session_id)?.clone();
        self.get_by_vt(&vt)
    }

    pub fn touch(&self, vtid: &str) {
        if let Some(e) = self.by_vt.lock().unwrap().get_mut(vtid) {
            e.last_update_ms = now_ms();
        }
    }

    /// Peek entries idle longer than `idle_ms` without removing (pre-cold analyze arm).
    pub fn list_idle(&self, idle_ms: i64) -> Vec<HotProbeEntry> {
        let ts = now_ms();
        self.by_vt
            .lock()
            .unwrap()
            .values()
            .filter(|e| ts.saturating_sub(e.last_update_ms) >= idle_ms)
            .cloned()
            .collect()
    }

    /// Entries idle longer than `idle_ms` — remove from hot and return for cold write.
    pub fn demote_idle(&self, idle_ms: i64) -> Vec<HotProbeEntry> {
        let ts = now_ms();
        let mut by_vt = self.by_vt.lock().unwrap();
        let mut by_s = self.by_session.lock().unwrap();
        let mut out = Vec::new();
        let stale: Vec<String> = by_vt
            .iter()
            .filter(|(_, e)| ts.saturating_sub(e.last_update_ms) >= idle_ms)
            .map(|(k, _)| k.clone())
            .collect();
        for k in stale {
            if let Some(e) = by_vt.remove(&k) {
                by_s.remove(&e.session_id);
                out.push(e);
            }
        }
        out
    }

    /// Demote exactly the given VT keys (caller pre-selected them via
    /// `list_idle`). Used by the capped sweep: when a flood leaves more idle
    /// entries than the arm budget allows, only the armed subset demotes —
    /// the rest stays in L1 for the next sweep instead of being dropped
    /// un-armed.
    pub fn demote_vts(&self, vtids: &[String]) -> Vec<HotProbeEntry> {
        let mut by_vt = self.by_vt.lock().unwrap();
        let mut by_s = self.by_session.lock().unwrap();
        let mut out = Vec::new();
        for k in vtids {
            if let Some(e) = by_vt.remove(k) {
                by_s.remove(&e.session_id);
                out.push(e);
            }
        }
        out
    }

    pub fn len_hot(&self) -> usize {
        self.by_vt.lock().unwrap().len()
    }

    /// True if VT is present in L1 and not idle past `idle_ms`.
    pub fn is_hot(&self, vtid: &str, idle_ms: i64) -> bool {
        let ts = now_ms();
        self.by_vt
            .lock()
            .unwrap()
            .get(vtid)
            .map(|e| ts.saturating_sub(e.last_update_ms) < idle_ms)
            .unwrap_or(false)
    }

    /// Promote a compact (or full) payload into L1 for an existing/new VT.
    pub fn promote_batch(
        &self,
        visitor_terminal_id: &str,
        session_id: &str,
        batch_id: &str,
        payload: &Value,
        client_ip: Option<&str>,
    ) {
        self.upsert_batch(visitor_terminal_id, session_id, batch_id, payload, client_ip);
    }
}

/// Extract queryable fields subset from a probe payload (small JSON for cold jsonb).
///
/// Full payload is still retained in `probe_cold.payload_z` (deflate) and
/// `probe_batches.payload_json`. This subset is for cross-filter indexes only.
pub fn compact_probe_payload(payload: &Value) -> Value {
    let mut m = Map::new();
    if let Some(bid) = payload.get("batch_id") {
        m.insert("batch_id".into(), bid.clone());
    }
    if let Some(src) = payload.get("source") {
        m.insert("source".into(), src.clone());
    }
    if let Some(fields) = payload.get("fields").and_then(|f| f.as_object()) {
        let keep = [
            "user_agent",
            "server_client_ip",
            "server_client_ip_source",
            "server_country",
            "server_asn",
            "os_family",
            "platform",
            "screen_width",
            "screen_height",
            "hardware_concurrency",
            "device_memory",
            "webrtc_host_ip_hash",
            "webrtc_srflx_ip_hash",
            "webrtc_host_count",
            "os_instance_hash",
            "os_instance_source",
            "canvas_hash",
            "webgl_renderer",
            "webgl_vendor",
            "webgl_unmasked_renderer",
            "language",
            "timezone",
            "page_id",
            "page_url",
            "robot_name",
            "early_kick",
            "micro_kick",
            "b8_path_kind",
            "ja3",
            "ja4",
            "tls_ja4",
            "h2_fingerprint",
            "h2_fingerprint_hash",
            "webdriver",
            "bot",
            "engine_claim",
            "engine_obs",
            "visitor_terminal_id",
            "device_id",
            "browser_surface_id",
            // Residual / commercial curve digests (cold index for collision audit)
            "residual_algo",
            "residual_mean",
            "residual_std",
            "residual_available",
            "form_class",
            "architecture",
            "unit_surface_id",
            "unit_surface_algo",
            "unit_multiround_stable",
            "hw_noise_algo",
            "audio_noise_energy",
            "audio_noise_sr",
            "audio_sample_rate",
        ];
        // Also keep short string digests of curves (not full float arrays — cold index cap).
        // Full curves live in compressed payload_full; index keeps residual + precomputed hashes.
        let mut fo = Map::new();
        for k in keep {
            if let Some(v) = fields.get(k) {
                // Skip oversized string blobs in the query index layer
                if let Some(s) = v.as_str() {
                    if s.len() > 512 {
                        continue;
                    }
                }
                fo.insert(k.to_string(), v.clone());
            }
        }
        // Compact curve fingerprints for cold index (full curves stay in payload_full).
        // Prefer FE-provided digests; else store length + coarse checksum of first/last bins.
        for (src, dst) in [
            ("hw_curve_webgl", "hw_curve_webgl_fp"),
            ("hw_curve_audio", "hw_curve_audio_fp"),
            ("hw_curve_canvas", "hw_curve_canvas_fp"),
            ("hw_curve_cpu", "hw_curve_cpu_fp"),
        ] {
            if fo.contains_key(dst) {
                continue;
            }
            if let Some(fp) = fields.get(dst) {
                if fp.is_string() || fp.is_number() {
                    fo.insert(dst.to_string(), fp.clone());
                    continue;
                }
            }
            if let Some(arr) = fields.get(src).and_then(|v| v.as_array()) {
                if arr.is_empty() {
                    continue;
                }
                let n = arr.len();
                let mut acc: u64 = n as u64;
                for (i, v) in arr.iter().enumerate() {
                    if i >= 8 && i + 8 < n {
                        continue; // sample head+tail only
                    }
                    if let Some(f) = v.as_f64() {
                        acc = acc
                            .wrapping_mul(131)
                            .wrapping_add((f * 1e6).round() as i64 as u64);
                    }
                }
                fo.insert(dst.to_string(), json!(format!("cfp_{n}_{acc:x}")));
            }
        }
        // Also copy any *ip* / *asn* / *country* / *ja* / residual / curve-fp scalar keys
        for (k, v) in fields {
            if fo.contains_key(k) {
                continue;
            }
            let kl = k.to_ascii_lowercase();
            if kl.contains("ip")
                || kl.contains("asn")
                || kl.contains("country")
                || kl.contains("ja3")
                || kl.contains("ja4")
                || kl.ends_with("_hash")
                || kl.starts_with("residual_")
                || kl.ends_with("_fp")
                || kl == "form_class"
                || kl == "os_instance_source"
            {
                if v.is_string() || v.is_number() || v.is_boolean() {
                    if let Some(s) = v.as_str() {
                        if s.len() > 512 {
                            continue;
                        }
                    }
                    fo.insert(k.clone(), v.clone());
                }
            }
        }
        // Cap total queryable field keys (raised: residual + curve fps need room)
        if fo.len() > 96 {
            let keys: Vec<String> = fo.keys().cloned().collect();
            for k in keys.into_iter().skip(96) {
                fo.remove(&k);
            }
        }
        m.insert("fields".into(), Value::Object(fo));
    }
    // gateway-style top-level early flag
    if let Some(early) = payload.get("early") {
        m.insert("early".into(), early.clone());
    }
    Value::Object(m)
}

/// Deflate-compress JSON text for cold blob storage (compact on disk).
pub fn compress_json_payload(payload: &Value) -> Result<Vec<u8>, String> {
    use flate2::write::DeflateEncoder;
    use flate2::Compression;
    use std::io::Write;
    let plain = serde_json::to_vec(payload).map_err(|e| e.to_string())?;
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&plain).map_err(|e| e.to_string())?;
    enc.finish().map_err(|e| e.to_string())
}

pub fn decompress_json_payload(data: &[u8]) -> Result<Value, String> {
    use flate2::read::DeflateDecoder;
    use std::io::Read;
    const MAX: u64 = 2 * 1024 * 1024;
    let mut dec = DeflateDecoder::new(data).take(MAX.saturating_add(1));
    let mut plain = Vec::new();
    dec.read_to_end(&mut plain).map_err(|e| e.to_string())?;
    if plain.len() as u64 > MAX {
        return Err("decompressed payload exceeds limit".into());
    }
    serde_json::from_slice(&plain).map_err(|e| e.to_string())
}

/// Build a cold document row shape for PG insert.
pub fn cold_doc_from_batch(
    session_id: &str,
    visitor_terminal_id: &str,
    batch_id: &str,
    source: &str,
    payload: &Value,
    client_ip: Option<&str>,
    created_ms: i64,
) -> Result<Value, String> {
    let fields = compact_probe_payload(payload);
    let blob = compress_json_payload(payload)?;
    Ok(json!({
        "session_id": session_id,
        "visitor_terminal_id": visitor_terminal_id,
        "batch_id": batch_id,
        "source": source,
        "client_ip": client_ip,
        "fields_json": fields,
        "payload_z": blob,
        "created_ms": created_ms,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hot_upsert_and_demote() {
        let cache = HotProbeCache::new();
        cache.upsert_batch(
            "vt_1",
            "cycle_1",
            "B0_bootstrap",
            &json!({"fields": {"user_agent": "ua", "server_client_ip": "1.2.3.4"}}),
            Some("1.2.3.4"),
        );
        assert_eq!(cache.len_hot(), 1);
        let e = cache.get_by_vt("vt_1").unwrap();
        assert_eq!(e.client_ip.as_deref(), Some("1.2.3.4"));
        assert!(e.batches.contains_key("B0_bootstrap"));
        // Force stale
        {
            let mut g = cache.by_vt.lock().unwrap();
            g.get_mut("vt_1").unwrap().last_update_ms = now_ms() - 60_000;
        }
        let demoted = cache.demote_idle(30_000);
        assert_eq!(demoted.len(), 1);
        assert_eq!(cache.len_hot(), 0);
    }

    #[test]
    fn compress_roundtrip() {
        let v = json!({"fields": {"a": 1, "b": "x".repeat(200)}});
        let c = compress_json_payload(&v).unwrap();
        assert!(c.len() < serde_json::to_vec(&v).unwrap().len());
        let back = decompress_json_payload(&c).unwrap();
        assert_eq!(back["fields"]["a"], 1);
    }

    #[test]
    fn compact_keeps_ip() {
        let p = json!({"fields": {
            "server_client_ip": "203.0.113.1",
            "server_client_ip_source": "cf",
            "user_agent": "Mozilla",
            "ja4": "t13d",
            "noise_big": "x".repeat(5000)
        }});
        let c = compact_probe_payload(&p);
        assert_eq!(c["fields"]["server_client_ip"], "203.0.113.1");
        assert_eq!(c["fields"]["server_client_ip_source"], "cf");
        assert_eq!(c["fields"]["ja4"], "t13d");
        assert!(c["fields"].get("noise_big").is_none());
    }

    #[test]
    fn compact_keeps_residual_and_curve_fp() {
        let p = json!({"fields": {
            "residual_algo": "gr_webgl_residual_std_v3f",
            "residual_mean": 0.211,
            "residual_std": 0.045,
            "os_instance_hash": "osi_abc",
            "webrtc_host_ip_hash": "lan_1",
            "hw_curve_webgl": [0.01, 0.02, 0.03, 0.04, 0.05, 0.06, 0.07, 0.08,
                               0.09, 0.10, 0.11, 0.12, 0.13, 0.14, 0.15, 0.16],
            "hw_curve_audio": [0.1, 0.2, 0.3, 0.4],
            "form_class": "desktop",
        }});
        let c = compact_probe_payload(&p);
        assert_eq!(c["fields"]["residual_algo"], "gr_webgl_residual_std_v3f");
        assert_eq!(c["fields"]["residual_mean"], 0.211);
        assert_eq!(c["fields"]["os_instance_hash"], "osi_abc");
        assert_eq!(c["fields"]["form_class"], "desktop");
        assert!(
            c["fields"]["hw_curve_webgl_fp"]
                .as_str()
                .unwrap_or("")
                .starts_with("cfp_"),
            "webgl curve fingerprint: {}",
            c["fields"]
        );
        assert!(
            c["fields"]["hw_curve_audio_fp"]
                .as_str()
                .unwrap_or("")
                .starts_with("cfp_"),
            "audio curve fingerprint: {}",
            c["fields"]
        );
        // Full float arrays must not bloat the index layer
        assert!(c["fields"].get("hw_curve_webgl").is_none());
    }

    #[test]
    fn compress_roundtrip_preserves_ip_field() {
        let v = json!({"fields": {
            "server_client_ip": "198.51.100.7",
            "server_client_ip_source": "cf",
            "screen_width": 1440,
            "blob": "y".repeat(800)
        }});
        let z = compress_json_payload(&v).unwrap();
        let back = decompress_json_payload(&z).unwrap();
        assert_eq!(back["fields"]["server_client_ip"], "198.51.100.7");
        assert_eq!(back["fields"]["screen_width"], 1440);
        assert!(z.len() < serde_json::to_vec(&v).unwrap().len());
    }

    #[test]
    fn timeout_matrix_has_tiers() {
        let m = timeout_matrix_json();
        assert_eq!(m["schema"], "gr_timeout_matrix_v1");
        assert!(m["tiers"]["l1_hot_process"]["idle_ms"].as_i64().unwrap() > 0);
        assert!(m["tiers"]["l3_cold"]["ttl_ms"].as_i64().unwrap() >= DEFAULT_COLD_TTL_MS);
        assert_eq!(m["tiers"]["l2_shared_warm_batches"]["primary_for_analyze"], true);
        assert_eq!(m["tiers"]["l1_hot_process"]["shared_across_workers"], false);
        assert_eq!(m["tiers"]["l2_shared_warm_batches"]["shared_across_workers"], true);
    }

    #[test]
    fn is_hot_respects_idle() {
        let cache = HotProbeCache::new();
        cache.upsert_batch("vt_h", "c1", "B0_bootstrap", &json!({"fields": {}}), None);
        assert!(cache.is_hot("vt_h", 60_000));
        {
            let mut g = cache.by_vt.lock().unwrap();
            g.get_mut("vt_h").unwrap().last_update_ms = now_ms() - 120_000;
        }
        assert!(!cache.is_hot("vt_h", 60_000));
    }

    #[test]
    fn hot_cap_evicts_most_idle_keeps_recent() {
        // Cap via env seed (config_version()==0 in unit tests → env path).
        std::env::set_var("GR_HOT_MAX_VTS", "3");
        let cache = HotProbeCache::new();
        // Distinct ages so the victim order is deterministic (equal-ms stamps
        // would fall back to HashMap iteration order).
        let ages_ms = [3_600_000, 2_400_000, 1_200_000];
        for i in 0..4 {
            cache.upsert_batch(
                &format!("vt_cap{i}"),
                &format!("c_cap{i}"),
                "B0_bootstrap",
                &json!({"fields": {"n": i}}),
                None,
            );
            if i < 3 {
                let mut g = cache.by_vt.lock().unwrap();
                g.get_mut(&format!("vt_cap{i}"))
                    .unwrap()
                    .last_update_ms = now_ms() - ages_ms[i];
            }
        }
        assert_eq!(cache.len_hot(), 3, "cap enforced");
        assert!(
            cache.get_by_vt("vt_cap0").is_none(),
            "most-idle evicted first"
        );
        assert!(cache.get_by_vt("vt_cap2").is_some(), "less-idle survives");
        assert!(cache.get_by_vt("vt_cap3").is_some(), "just-touched survives");
        assert!(
            cache.get_by_session("c_cap0").is_none(),
            "session index dropped with evicted entry"
        );
        std::env::remove_var("GR_HOT_MAX_VTS");
    }
}
