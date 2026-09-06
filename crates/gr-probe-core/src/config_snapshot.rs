//! ConfigSnapshot — versioned runtime config (iss/49 P0).
//!
//! Not a full distributed config center; provides a **shipped** snapshot
//! contract (`config_version` + hash) that analyze/product can stamp and
//! that `gr doctor` can report.

use crate::GR_PRODUCT_VERSION;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const CONFIG_SNAPSHOT_ALGO: &str = "config_snapshot_v1";

/// Optional file override: `GR_CONFIG_SNAPSHOT_PATH` JSON merges into snapshot body
/// (remote push can write this file; process reloads on each call — no SaaS UI required).
fn file_overlay() -> Value {
    let path = gr_abi::env::get("CONFIG_SNAPSHOT_PATH").unwrap_or_default();
    if path.trim().is_empty() {
        return json!({});
    }
    match std::fs::read_to_string(&path) {
        Ok(t) => serde_json::from_str(&t).unwrap_or(json!({})),
        Err(_) => json!({ "config_file_error": true, "path": path }),
    }
}

/// Write remote-push JSON overlay file (SaaS / ops config center target).
/// Path: `GR_CONFIG_SNAPSHOT_PATH` or `path` argument.
pub fn push_config_overlay(overlay: &Value, path: Option<&str>) -> Value {
    let p = path
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            gr_abi::env::get("CONFIG_SNAPSHOT_PATH")
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(|| {
            let dir = gr_abi::env::get("DATA_DIR").unwrap_or_else(|| "data".into());
            format!("{dir}/gr_config_snapshot.json")
        });
    if let Some(parent) = std::path::Path::new(&p).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Merge with existing file so partial pushes work
    let mut base = file_overlay_at(&p);
    if let (Some(b), Some(o)) = (base.as_object_mut(), overlay.as_object()) {
        for (k, v) in o {
            b.insert(k.clone(), v.clone());
        }
        b.insert(
            "pushed_at_ms".into(),
            json!(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0)),
        );
    }
    match std::fs::write(&p, serde_json::to_string_pretty(&base).unwrap_or_else(|_| "{}".into())) {
        Ok(()) => {
            // Ensure subsequent current_config_snapshot sees this path
            if gr_abi::env::get("CONFIG_SNAPSHOT_PATH").unwrap_or_default().is_empty() {
                std::env::set_var("GR_CONFIG_SNAPSHOT_PATH", &p);
            }
            let snap = current_config_snapshot();
            json!({
                "ok": true,
                "path": p,
                "config_version": snap.get("config_version"),
                "config_hash": snap.get("config_hash"),
                "snapshot": snap,
                "algo": "config_push_v1",
            })
        }
        Err(e) => json!({"ok": false, "error": e.to_string(), "path": p}),
    }
}

fn file_overlay_at(path: &str) -> Value {
    match std::fs::read_to_string(path) {
        Ok(t) => serde_json::from_str(&t).unwrap_or(json!({})),
        Err(_) => json!({}),
    }
}

/// Build a stable config snapshot for this process.
pub fn current_config_snapshot() -> Value {
    let soft_v2 = gr_abi::env::get("SOFT_V2_READY").unwrap_or_else(|| "0".into());
    let sealed = gr_abi::env::get("REQUIRE_SEALED_INGEST").unwrap_or_else(|| "0".into());
    let result_token_set = !gr_abi::env::get("RESULT_TOKEN")
        .unwrap_or_default()
        .trim()
        .is_empty();
    let mut body = json!({
        "product_version": GR_PRODUCT_VERSION,
        "soft_v2_ready": soft_v2,
        "require_sealed_ingest": sealed,
        "result_token_configured": result_token_set,
        "rpa_idle_analyze_ms": crate::return_gate::RPA_IDLE_ANALYZE_MS,
        "side_join_algo": "side_join_v2_ip_port_ttl",
        "device_slot_scheme": "extended_curve_v2",
        "homogenization_algo": "homogenization_v1",
        "peer_similarity_algo": crate::peer_similarity::PEER_SIMILARITY_ALGO,
        "association_backend": gr_abi::env::get("ASSOCIATION_DATABASE_URL")
            .filter(|s| !s.is_empty())
            .map(|_| "postgres_url_configured")
            .unwrap_or("postgres_required").to_string(),
        // iss/54 F5/F6 honesty in config stamp
        "silicon_fusion_weights": crate::hw_fusion_weights::fusion_weights_status(),
        "lane_s_engine_norm": crate::hw_engine_norm::engine_norm_status(),
        "hw_channel_drift_algo": crate::hw_channel_drift::HW_CHANNEL_DRIFT_ALGO,
    });
    // Merge file overlay (SaaS remote push can drop JSON here)
    let overlay = file_overlay();
    if let (Some(base), Some(over)) = (body.as_object_mut(), overlay.as_object()) {
        for (k, v) in over {
            base.insert(k.clone(), v.clone());
        }
        base.insert("config_file_overlay".into(), json!(!over.is_empty()));
    }
    let canonical = serde_json::to_string(&body).unwrap_or_default();
    let mut h = Sha256::new();
    h.update(canonical.as_bytes());
    let digest = format!("{:x}", h.finalize());
    let version = format!(
        "cfg_{}_{}",
        GR_PRODUCT_VERSION.replace(|c: char| !c.is_ascii_alphanumeric(), "_"),
        &digest[..12.min(digest.len())]
    );
    json!({
        "algo": CONFIG_SNAPSHOT_ALGO,
        "config_version": version,
        "config_hash": digest,
        "snapshot": body,
        "note": "iss/49: process snapshot + optional GR_CONFIG_SNAPSHOT_PATH file pull (remote push target)",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_has_version_and_hash() {
        let s = current_config_snapshot();
        assert_eq!(s["algo"], CONFIG_SNAPSHOT_ALGO);
        assert!(s["config_version"].as_str().unwrap().starts_with("cfg_"));
        assert_eq!(s["config_hash"].as_str().unwrap().len(), 64);
        assert_eq!(
            s["snapshot"]["rpa_idle_analyze_ms"],
            crate::return_gate::RPA_IDLE_ANALYZE_MS
        );
    }
}
