//! RPA layered verdict bands (iss/50 R5/R6): human_likeness vs automation_evidence.
//!
//! Separates "looks human" aggregate from "automation evidence" so count-alone
//! rules cannot invent human (iss/47).

use serde_json::{json, Map, Value};

pub const RPA_BANDS_ALGO: &str = "rpa_bands_v1";

fn f(m: &Map<String, Value>, k: &str) -> Option<f64> {
    m.get(k).and_then(|v| v.as_f64()).or_else(|| {
        m.get("features")
            .and_then(|f| f.get(k))
            .and_then(|v| v.as_f64())
    })
}

/// Derive dual-axis RPA bands from rpa_features_v2 or flat kinematics fields.
pub fn rpa_behavior_bands(fields: &Value) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let v2 = fo
        .get("rpa_features_v2")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let feat = v2
        .get("features")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_else(|| fo.clone());

    let mut human = 0.45_f64;
    let mut auto = 0.25_f64;
    let mut reasons: Vec<String> = Vec::new();

    if let Some(ent) = f(&feat, "input_mouse_entropy") {
        if ent >= 0.35 {
            human += 0.12;
            reasons.push(format!("entropy_human={ent:.2}"));
        } else if ent < 0.12 {
            auto += 0.14;
            reasons.push(format!("entropy_low={ent:.2}"));
        }
    }
    if let Some(icr) = f(&feat, "integer_coord_ratio") {
        if icr >= 0.995 {
            auto += 0.10;
            reasons.push(format!("integer_coord_pure={icr:.3}"));
        } else if icr <= 0.75 {
            human += 0.08;
            reasons.push(format!("integer_coord_humanish={icr:.2}"));
        }
    }
    if let Some(dwell) = f(&feat, "key_dwell_stddev") {
        if dwell >= 12.0 {
            human += 0.08;
        } else if dwell > 0.0 && dwell < 3.0 {
            auto += 0.10;
            reasons.push(format!("key_dwell_uniform={dwell:.1}"));
        }
    }
    if let Some(pre) = f(&feat, "pre_action_move_count") {
        if pre <= 0.0 {
            auto += 0.12;
            reasons.push("pre_action_move_zero".into());
        } else if pre >= 3.0 {
            human += 0.05;
        }
    }
    if feat
        .get("sensitive_action_zero_move")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        auto += 0.15;
        reasons.push("sensitive_action_zero_move".into());
    }
    // Count alone must not invent human
    let n = f(&feat, "n_events").or_else(|| f(&fo, "behavior_count")).unwrap_or(0.0);
    if n >= 20.0 && reasons.iter().all(|r| !r.contains("entropy") && !r.contains("integer")) {
        // slight conf only
        human = (human + 0.02).min(0.55);
        reasons.push("count_not_safety".into());
    }

    human = human.clamp(0.0, 0.99);
    auto = auto.clamp(0.0, 0.99);
    let human_band = if human >= 0.72 {
        "likely_human"
    } else if human >= 0.5 {
        "mixed"
    } else {
        "weak_human_evidence"
    };
    let auto_band = if auto >= 0.65 {
        "likely_automation"
    } else if auto >= 0.4 {
        "automation_suspect"
    } else {
        "low_automation_evidence"
    };

    json!({
        "algo": RPA_BANDS_ALGO,
        "human_likeness": (human * 10000.0).round() / 10000.0,
        "human_likeness_band": human_band,
        "automation_evidence": (auto * 10000.0).round() / 10000.0,
        "automation_evidence_band": auto_band,
        "reasons": reasons,
        "count_not_safety": true,
        "note": "iss/50 R5/R6 dual-axis; fuse with page rpa score, never sole ban",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pure_integer_raises_automation() {
        let f = json!({
            "rpa_features_v2": {
                "features": {
                    "integer_coord_ratio": 1.0,
                    "input_mouse_entropy": 0.05,
                    "n_events": 40
                }
            }
        });
        let b = rpa_behavior_bands(&f);
        assert!(b["automation_evidence"].as_f64().unwrap() > 0.4);
        assert_eq!(b["count_not_safety"], true);
    }
}
