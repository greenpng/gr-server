//! Behavior self-similarity profile (iss/50 R7): fixed vec32 from page RPA aggregates.
//!
//! Not a full biometric model. Produces a compact, comparable vector so SDK can
//! compare subject sessions offline. ECE-style calibration uses [`crate::conf_cal`]
//! when labels exist (spec pairs or customer label webhook).

use serde_json::{json, Map, Value};

pub const BEHAVIOR_SELF_SIM_ALGO: &str = "behavior_self_sim_v1";
pub const BEHAVIOR_VEC32_DIM: usize = 32;

fn f(m: &Map<String, Value>, keys: &[&str]) -> f64 {
    for k in keys {
        if let Some(v) = m.get(*k).and_then(|x| x.as_f64().or_else(|| x.as_i64().map(|i| i as f64))) {
            if v.is_finite() {
                return v;
            }
        }
    }
    0.0
}

fn clamp01(x: f64) -> f64 {
    x.clamp(0.0, 1.0)
}

/// Build a 32-dim behavior profile from rpa_features_v2 / flat kinematics.
pub fn behavior_profile_vec32(fields: &Value) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let feat = fo
        .get("rpa_features_v2")
        .and_then(|v| v.get("features"))
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_else(|| fo.clone());

    let mut v = [0.0_f64; BEHAVIOR_VEC32_DIM];
    // 0..7 kinematics
    v[0] = clamp01(f(&feat, &["input_mouse_entropy"]));
    v[1] = clamp01(1.0 - f(&feat, &["integer_coord_ratio"]).min(1.0));
    v[2] = clamp01(f(&feat, &["key_dwell_stddev"]) / 40.0);
    v[3] = clamp01(f(&feat, &["pre_action_move_count"]) / 12.0);
    v[4] = clamp01(f(&feat, &["event_order_score"]));
    v[5] = clamp01(f(&feat, &["scroll_burst_n"]) / 8.0);
    v[6] = clamp01(f(&feat, &["input_velocity_cv"]).min(2.0) / 2.0);
    v[7] = clamp01(f(&feat, &["path_straightness"]));
    // 8..15 volume / diversity
    v[8] = clamp01(f(&feat, &["n_events", "behavior_count"]) / 80.0);
    v[9] = clamp01(f(&feat, &["type_diversity_n", "behavior_type_diversity_n"]) / 12.0);
    v[10] = clamp01(f(&feat, &["pointer_n"]) / 60.0);
    v[11] = clamp01(f(&feat, &["key_n"]) / 40.0);
    v[12] = clamp01(f(&feat, &["scroll_n"]) / 30.0);
    v[13] = clamp01(f(&feat, &["ttfi_ms"]) / 5000.0);
    v[14] = clamp01(f(&feat, &["click_interval_cv"]).min(2.0) / 2.0);
    v[15] = clamp01(f(&feat, &["path_length_px"]) / 4000.0);
    // 16..23 quality / flags (Map has no .pointer — nest get)
    let sq = fo
        .get("rpa_features_v2")
        .and_then(|v| v.get("segment"))
        .and_then(|v| v.get("sample_quality"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    v[16] = match sq {
        "adequate" => 1.0,
        "thin" => 0.5,
        _ => 0.15,
    };
    v[17] = if feat
        .get("sensitive_action_zero_move")
        .and_then(|x| x.as_bool())
        .unwrap_or(false)
    {
        1.0
    } else {
        0.0
    };
    v[18] = if feat
        .get("sensitive_action_seen")
        .and_then(|x| x.as_bool())
        .unwrap_or(false)
    {
        1.0
    } else {
        0.0
    };
    // rpa bands if present
    v[19] = fo
        .get("rpa_bands")
        .and_then(|v| v.get("human_likeness"))
        .or_else(|| fo.get("human_likeness"))
        .and_then(|x| x.as_f64())
        .unwrap_or(0.45);
    v[20] = fo
        .get("rpa_bands")
        .and_then(|v| v.get("automation_evidence"))
        .or_else(|| fo.get("automation_evidence"))
        .and_then(|x| x.as_f64())
        .unwrap_or(0.25);
    // 21..31 reserved / derived harmonics
    for i in 21..BEHAVIOR_VEC32_DIM {
        let a = v[i % 16];
        let b = v[(i * 3) % 16];
        v[i] = clamp01((a * 0.6 + b * 0.4).sin().abs());
    }

    let rounded: Vec<f64> = v
        .iter()
        .map(|x| (x * 10000.0).round() / 10000.0)
        .collect();
    let digest = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        for x in &rounded {
            h.update(format!("{x:.4}").as_bytes());
            h.update(b"|");
        }
        format!("{:x}", h.finalize())[..16].to_string()
    };

    json!({
        "algo": BEHAVIOR_SELF_SIM_ALGO,
        "dim": BEHAVIOR_VEC32_DIM,
        "vec32": rounded,
        "profile_digest": digest,
        "sdk_use": "compare cosine(vec32_a, vec32_b) per subject_ref offline; not commercial device id",
        "promote_to_commercial_id": false,
    })
}

/// Cosine similarity of two vec32 arrays (SDK / offline).
pub fn cosine_vec32(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len().min(b.len()).min(BEHAVIOR_VEC32_DIM);
    if n == 0 {
        return 0.0;
    }
    let mut dot = 0.0;
    let mut na = 0.0;
    let mut nb = 0.0;
    for i in 0..n {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na <= 1e-12 || nb <= 1e-12 {
        return 0.0;
    }
    (dot / (na.sqrt() * nb.sqrt())).clamp(-1.0, 1.0)
}

/// Offline ECE-style calibration report when labels exist (iss/50 P2 path).
/// Uses existing conf_cal machinery; does not invent production labels.
pub fn offline_ece_from_spec_or_json(pairs_json: Option<&Value>) -> Value {
    let pairs = if let Some(v) = pairs_json {
        crate::conf_cal::pairs_from_json(v)
    } else {
        crate::conf_cal::load_spec_labeled_pairs()
    };
    let report = crate::conf_cal::calibrate_offline(&pairs);
    json!({
        "algo": "behavior_ece_offline_v1",
        "n_pairs": pairs.len(),
        "calibrate": report,
        "note": "customer labels via /v1/assoc/label → webhook; this path is offline/spec until labels exist",
        "requires_customer_labels_for_production": true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn vec32_dim_and_cosine() {
        let f = json!({
            "rpa_features_v2": {
                "segment": {"sample_quality": "adequate"},
                "features": {
                    "input_mouse_entropy": 0.5,
                    "integer_coord_ratio": 0.4,
                    "n_events": 40,
                    "type_diversity_n": 6
                }
            }
        });
        let p = behavior_profile_vec32(&f);
        let v = p["vec32"].as_array().unwrap();
        assert_eq!(v.len(), 32);
        let a: Vec<f64> = v.iter().filter_map(|x| x.as_f64()).collect();
        assert!((cosine_vec32(&a, &a) - 1.0).abs() < 1e-6);
        assert_eq!(p["promote_to_commercial_id"], false);
    }
}
