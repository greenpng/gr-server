//! Write-amplification controls for multi-worker probe/analyze IO.
//!
//! Prod 178 symptoms: analysis_results TOAST ~85KB/rev, probe_cold always rewritten,
//! sessions.updated_ms touched every batch (~1M updates / 30k sessions).

use serde_json::{json, Value};
#[cfg(test)]
use serde_json::Map;

/// Keep at most this many analysis revs per session (oldest pruned after insert).
pub fn analysis_history_keep() -> i64 {
    gr_abi::env::get("ANALYSIS_HISTORY_KEEP")
        .and_then(|s| s.parse().ok())
        .unwrap_or(3)
        .clamp(1, 32)
}

/// Min interval between session `updated_ms` touches (ms). IP changes always write.
pub fn session_touch_min_interval_ms() -> i64 {
    gr_abi::env::get("SESSION_TOUCH_MIN_INTERVAL_MS")
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000)
        .clamp(0, 60_000)
}

/// Keys dropped entirely from stored analysis JSON (recomputable / ops-redundant).
const DROP_TOP: &[&str] = &[
    "field_algorithm_utilization",
    "field_utilization_detail",
    "matrix_rows",
    "probe_fields",
    "evidence_raw",
    "raw_fields",
];

/// Nested keys stripped under any object (curves / hist dumps already in probe_batches).
const DROP_ANY: &[&str] = &[
    "hw_curve_webgl",
    "hw_curve_audio",
    "hw_curve_canvas",
    "hw_curve_cpu",
    "residual_hist",
    "challenge_hist",
    "audio_deep_curve",
    "payload_full",
    "fields_dump",
];

/// Cap array lengths under these keys.
const CAP_ARRAYS: &[(&str, usize)] = &[
    ("battle_log", 16),
    ("direction_priors", 24),
    ("open_verification_gaps", 32),
    ("id_warnings", 32),
    ("analysis_posture", 48),
    ("eligibility_reasons", 48),
    ("no_id_reasons", 32),
    ("tier_reasons", 32),
    ("materials_included", 64),
    ("soft_extras_included", 32),
    ("corroboration", 32),
];

/// Slim evaluate/analysis JSON for durable storage (TOAST / WAL reduction).
///
/// Preserves ops-critical identity shape (`device`, `product`, `trust` digests,
/// collision posture) while dropping raw curves, huge diagnostics, and long logs.
pub fn slim_analysis_result_for_storage(result: &Value) -> Value {
    let mut v = result.clone();
    if let Some(obj) = v.as_object_mut() {
        for k in DROP_TOP {
            obj.remove(*k);
        }
        // device.trust.materials stays (short digest strings) — needed by ops materials.
        // Drop nested evaluate bloat under diagnostics if present.
        if let Some(diag) = obj.get_mut("diagnostics").and_then(|d| d.as_object_mut()) {
            for k in [
                "field_utilization",
                "field_algorithm_utilization",
                "utilization_by_axis",
                "matrix_coverage",
                "scorer_consumption",
                "raw",
            ] {
                diag.remove(k);
            }
            // Keep compact analysis_quality summary; drop heavy subtrees.
            if let Some(aq) = diag.get_mut("analysis_quality").and_then(|a| a.as_object_mut()) {
                for k in ["per_field", "axis_detail", "mutual_info", "samples"] {
                    aq.remove(k);
                }
            }
        }
        // brain / control plane: keep belief summary, trim logs
        if let Some(brain) = obj.get_mut("brain").and_then(|b| b.as_object_mut()) {
            for k in ["route_plan_full", "pack_catalog", "yield_table"] {
                brain.remove(k);
            }
        }
    }
    slim_walk(&mut v, 0);
    // Mark storage transform for readers / debugging.
    if let Some(obj) = v.as_object_mut() {
        obj.insert("_storage".into(), json!("slim_v1"));
    }
    v
}

fn slim_walk(v: &mut Value, depth: usize) {
    if depth > 12 {
        return;
    }
    match v {
        Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for k in keys {
                if DROP_ANY.iter().any(|d| *d == k) {
                    map.remove(&k);
                    continue;
                }
                if let Some(cap) = CAP_ARRAYS.iter().find(|(n, _)| *n == k).map(|(_, c)| *c) {
                    if let Some(Value::Array(arr)) = map.get_mut(&k) {
                        if arr.len() > cap {
                            arr.truncate(cap);
                        }
                    }
                }
                // Truncate huge string blobs (base64, dumps)
                if let Some(Value::String(s)) = map.get_mut(&k) {
                    if s.len() > 4096 {
                        let keep = 512;
                        *s = format!("{}…[trunc:{}]", &s[..keep], s.len());
                    }
                }
                if let Some(child) = map.get_mut(&k) {
                    slim_walk(child, depth + 1);
                }
            }
        }
        Value::Array(arr) => {
            // Soft cap on anonymous large arrays
            if arr.len() > 64 && depth > 2 {
                arr.truncate(64);
            }
            for item in arr.iter_mut() {
                slim_walk(item, depth + 1);
            }
        }
        _ => {}
    }
}

/// Optional 32-byte AES-256-GCM key from env (`GR_ANALYSIS_AT_REST_KEY` or
/// `GR_ANALYSIS_AT_REST_KEY`). Accepts 64-char hex or standard base64 of 32 bytes.
fn analysis_at_rest_key() -> Option<[u8; 32]> {
    let raw = gr_abi::env::get("ANALYSIS_AT_REST_KEY")?;
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    if t.len() == 64 && t.chars().all(|c| c.is_ascii_hexdigit()) {
        let mut out = [0u8; 32];
        for i in 0..32 {
            out[i] = u8::from_str_radix(&t[i * 2..i * 2 + 2], 16).ok()?;
        }
        return Some(out);
    }
    use base64::Engine;
    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(t) {
        if bytes.len() == 32 {
            let mut out = [0u8; 32];
            out.copy_from_slice(&bytes);
            return Some(out);
        }
    }
    None
}

fn encrypt_payload(plain: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, String> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?;
    let mut nonce_bytes = [0u8; 12];
    getrandom::getrandom(&mut nonce_bytes).map_err(|e| e.to_string())?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plain)
        .map_err(|e| format!("aes-gcm encrypt: {e}"))?;
    let mut out = Vec::with_capacity(12 + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(out)
}

fn decrypt_payload(blob: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, String> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};
    if blob.len() < 13 {
        return Err("e1 ciphertext too short".into());
    }
    let (nonce_bytes, ct) = blob.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?;
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher
        .decrypt(nonce, ct)
        .map_err(|e| format!("aes-gcm decrypt: {e}"))
}

/// Encode slim analysis for TEXT column:
/// - With `GR_ANALYSIS_AT_REST_KEY`: `e1:` + base64(nonce||aes-gcm(deflate-or-json))
/// - Else large payloads: `z1:` + base64(deflate(json))
/// - Small payloads: plain JSON
pub fn encode_analysis_result_json(slim: &Value) -> Result<String, String> {
    let plain = serde_json::to_vec(slim).map_err(|e| e.to_string())?;
    use base64::Engine;
    if let Some(key) = analysis_at_rest_key() {
        let z = if plain.len() < 256 {
            plain.clone()
        } else {
            crate::compress_json_payload(slim).unwrap_or_else(|_| plain.clone())
        };
        let enc = encrypt_payload(&z, &key)?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&enc);
        return Ok(format!("e1:{b64}"));
    }
    if plain.len() < 2048 {
        return String::from_utf8(plain).map_err(|e| e.to_string());
    }
    let z = crate::compress_json_payload(slim)?;
    if z.len() + 32 >= plain.len() {
        return String::from_utf8(plain).map_err(|e| e.to_string());
    }
    let b64 = base64::engine::general_purpose::STANDARD.encode(&z);
    Ok(format!("z1:{b64}"))
}

pub fn decode_analysis_result_json(s: &str) -> Result<Value, String> {
    use base64::Engine;
    if let Some(rest) = s.strip_prefix("e1:") {
        let key = analysis_at_rest_key()
            .ok_or_else(|| "e1: payload requires GR_ANALYSIS_AT_REST_KEY".to_string())?;
        let raw = base64::engine::general_purpose::STANDARD
            .decode(rest)
            .map_err(|e| e.to_string())?;
        let plain = decrypt_payload(&raw, &key)?;
        if let Ok(v) = crate::decompress_json_payload(&plain) {
            return Ok(v);
        }
        return serde_json::from_slice(&plain).map_err(|e| e.to_string());
    }
    if let Some(rest) = s.strip_prefix("z1:") {
        let raw = base64::engine::general_purpose::STANDARD
            .decode(rest)
            .map_err(|e| e.to_string())?;
        return crate::decompress_json_payload(&raw);
    }
    serde_json::from_str(s).map_err(|e| e.to_string())
}

/// Public storage mode note for ops / panel.
pub fn analysis_storage_mode() -> &'static str {
    if analysis_at_rest_key().is_some() {
        "e1:aes-256-gcm+deflate"
    } else {
        "z1:deflate+base64 (set GR_ANALYSIS_AT_REST_KEY for AES-256-GCM)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn slim_drops_curves_and_caps_arrays() {
        let raw = json!({
            "device": {
                "device_id": "dv_abc",
                "trust": {
                    "materials_included": ["form_class", "hw_webgl_stable"],
                    "materials": {"hw_webgl_stable": "wg_1"}
                }
            },
            "diagnostics": {
                "field_utilization": {"huge": true},
                "analysis_quality": {"posture": "ok", "per_field": {"a": 1}}
            },
            "battle_log": (0..40).map(|i| json!({"i": i})).collect::<Vec<_>>(),
            "hw_curve_webgl": [0.1, 0.2, 0.3],
        });
        let slim = slim_analysis_result_for_storage(&raw);
        assert!(slim.pointer("/device/device_id").is_some());
        assert!(slim.pointer("/device/trust/materials/hw_webgl_stable").is_some());
        assert!(slim.get("hw_curve_webgl").is_none());
        assert!(slim.pointer("/diagnostics/field_utilization").is_none());
        assert!(slim.pointer("/diagnostics/analysis_quality/per_field").is_none());
        assert_eq!(slim["battle_log"].as_array().unwrap().len(), 16);
        assert_eq!(slim["_storage"], "slim_v1");
    }

    #[test]
    fn encode_decode_roundtrip_plain_and_z() {
        let small = json!({"a": 1});
        let e = encode_analysis_result_json(&small).unwrap();
        assert!(!e.starts_with("z1:"));
        assert_eq!(decode_analysis_result_json(&e).unwrap()["a"], 1);

        let mut big_map = Map::new();
        for i in 0..200 {
            big_map.insert(format!("k{i}"), json!("x".repeat(64)));
        }
        let big = Value::Object(big_map);
        let e2 = encode_analysis_result_json(&big).unwrap();
        // may or may not compress depending on entropy; must round-trip
        let back = decode_analysis_result_json(&e2).unwrap();
        assert_eq!(back.as_object().unwrap().len(), 200);
    }
}
