//! L4 business policy thresholds (G-ARCH-13). Not constitution — configurable.
//!
//! L0/L1 redlines stay in trust/soft constants. This file only gates OTP / friction.

use serde_json::{json, Value};
use std::sync::OnceLock;

pub const DEFAULT_POLICY_ID: &str = "site_default_v1";

static POLICY: OnceLock<Value> = OnceLock::new();

fn default_policy() -> Value {
    json!({
        "policy_id": DEFAULT_POLICY_ID,
        "bot_block_verdicts": ["bot", "crawler", "automation"],
        "otp_if": {
            "br_score_lt": 0.55,
            "os_score_lt": 0.50,
            "rpa_score_lt": 0.45,
            "device_confidence_lt": 0.55
        },
        "allow_anonymous_if_no_device_id": true
    })
}

/// Load policy from `spec/product_policy.json` when present; else defaults.
pub fn load_product_policy() -> &'static Value {
    POLICY.get_or_init(|| {
        let candidates = [
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spec/product_policy.json"),
            std::path::PathBuf::from("spec/product_policy.json"),
        ];
        for p in candidates {
            if let Ok(s) = std::fs::read_to_string(&p) {
                if let Ok(v) = serde_json::from_str::<Value>(&s) {
                    return v;
                }
            }
        }
        default_policy()
    })
}

pub fn otp_thresholds(policy: &Value) -> (f64, f64, f64, f64) {
    let o = policy.get("otp_if").cloned().unwrap_or(json!({}));
    (
        o.get("br_score_lt").and_then(|v| v.as_f64()).unwrap_or(0.55),
        o.get("os_score_lt").and_then(|v| v.as_f64()).unwrap_or(0.50),
        o.get("rpa_score_lt").and_then(|v| v.as_f64()).unwrap_or(0.45),
        o.get("device_confidence_lt")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.55),
    )
}

/// Multi-source conflict demotion weights (policy + optional env override for lab calibrate).
#[derive(Debug, Clone, Copy)]
pub struct MultiSourceConflictWeights {
    pub os_base: f64,
    pub os_per: f64,
    pub os_max_n: usize,
    pub br_base: f64,
    pub br_per: f64,
    pub br_max_n: usize,
    pub rpa_base: f64,
    pub rpa_mismatch_add: f64,
    pub rpa_match_ratio_lt: f64,
    pub single_source_pressure_lt: f64,
    pub single_source_scale: f64,
}

fn env_f64(name: &str) -> Option<f64> {
    std::env::var(name).ok().and_then(|s| s.parse().ok())
}

pub fn multi_source_conflict_weights() -> MultiSourceConflictWeights {
    let p = load_product_policy();
    let m = p
        .get("multi_source_conflict")
        .cloned()
        .unwrap_or(json!({}));
    MultiSourceConflictWeights {
        os_base: env_f64("GR_MSC_OS_BASE")
            .or_else(|| m.get("os_base").and_then(|v| v.as_f64()))
            .unwrap_or(0.08),
        os_per: env_f64("GR_MSC_OS_PER")
            .or_else(|| m.get("os_per").and_then(|v| v.as_f64()))
            .unwrap_or(0.04),
        os_max_n: m
            .get("os_max_n")
            .and_then(|v| v.as_u64())
            .unwrap_or(4) as usize,
        br_base: env_f64("GR_MSC_BR_BASE")
            .or_else(|| m.get("br_base").and_then(|v| v.as_f64()))
            .unwrap_or(0.10),
        br_per: env_f64("GR_MSC_BR_PER")
            .or_else(|| m.get("br_per").and_then(|v| v.as_f64()))
            .unwrap_or(0.05),
        br_max_n: m
            .get("br_max_n")
            .and_then(|v| v.as_u64())
            .unwrap_or(4) as usize,
        rpa_base: env_f64("GR_MSC_RPA_BASE")
            .or_else(|| m.get("rpa_base").and_then(|v| v.as_f64()))
            .unwrap_or(0.06),
        rpa_mismatch_add: env_f64("GR_MSC_RPA_MISMATCH")
            .or_else(|| m.get("rpa_mismatch_add").and_then(|v| v.as_f64()))
            .unwrap_or(0.08),
        rpa_match_ratio_lt: m
            .get("rpa_match_ratio_lt")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.50),
        single_source_pressure_lt: m
            .get("single_source_pressure_lt")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.25),
        single_source_scale: m
            .get("single_source_scale")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.05),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_loads() {
        let p = load_product_policy();
        assert_eq!(p["policy_id"], DEFAULT_POLICY_ID);
        let (br, os, rpa, conf) = otp_thresholds(p);
        assert!(br > 0.0 && os > 0.0 && rpa > 0.0 && conf > 0.0);
    }
}
