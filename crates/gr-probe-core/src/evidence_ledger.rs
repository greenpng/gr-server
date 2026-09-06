//! iss/opus5 03-P0-1: planned-vs-actual evidence ledger ("对方在躲" vs "没看清").
//!
//! The server issues a **signed** `route_plan` (with its pack set) inside every
//! analyze result; the FE executes those packs and uploads batches. This module
//! compares the *issued* pack set (the server-side expectation, registered when
//! the route plan was sealed) against the *actually received* batch ids.
//!
//! - High missing ratio on a session that is **still alive** → the client is
//!   deliberately withholding material, which is itself a strong signal:
//!   `evidence_withheld` replaces the old "insufficient" reward for evasion.
//! - A dead/closed session with missing packs → network drop or early leave;
//!   no withholding judgment (stays 没看清, not 在躲).
//!
//! Thresholds are tunable via env (debuted with conservative defaults):
//! - `GR_EW_MISS_RATIO` (default 0.5): missing-ratio floor to trigger.
//! - `GR_EW_MIN_EXPECTED` (default 3): fewer scheduled packs than this is
//!   never enough signal to accuse withholding.

use serde_json::{json, Value};

pub const EW_MISS_RATIO_DEFAULT: f64 = 0.5;
pub const EW_MIN_EXPECTED_DEFAULT: usize = 3;

fn env_f64(name: &str, default: f64, min: f64, max: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v >= min && *v <= max)
        .unwrap_or(default)
}

fn env_usize(name: &str, default: usize, min: usize, max: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|v| *v >= min && *v <= max)
        .unwrap_or(default)
}

pub fn ew_miss_ratio() -> f64 {
    env_f64("GR_EW_MISS_RATIO", EW_MISS_RATIO_DEFAULT, 0.1, 0.95)
}

pub fn ew_min_expected() -> usize {
    env_usize("GR_EW_MIN_EXPECTED", EW_MIN_EXPECTED_DEFAULT, 2, 200)
}

/// Pack ids the server issued/expected. Honors both `pack_id` and `batch_id`
/// keys (both are used across the brain/schedule paths).
pub fn expected_pack_ids(route_plan: &Value) -> Vec<String> {
    route_plan
        .get("packs")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    p.get("pack_id")
                        .or_else(|| p.get("batch_id"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Received batch ids from the canonical (store-built) evidence.
pub fn received_batch_ids(evidence: &Value) -> Vec<String> {
    evidence
        .get("batches")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|b| {
                    b.get("batch_id")
                        .or_else(|| b.get("pack_id"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Raw transparency ledger: how many of the issued packs actually landed.
/// Always attached to the route plan (and covered by the route seal) so
/// "why was this field not executed" is answerable from the sealed set.
pub fn missing_ledger(expected: &[String], received: &[String]) -> Value {
    let recv: std::collections::HashSet<&str> = received.iter().map(|s| s.as_str()).collect();
    let missing: Vec<String> = expected
        .iter()
        .filter(|e| !recv.contains(e.as_str()))
        .cloned()
        .collect();
    let ratio = if expected.is_empty() {
        0.0
    } else {
        (missing.len() as f64) / (expected.len() as f64)
    };
    json!({
        "expected_n": expected.len(),
        "received_n": received.len(),
        "missing_n": missing.len(),
        "missing_ratio": (ratio * 1000.0).round() / 1000.0,
        "missing_packs": missing,
    })
}

/// Pure withholding judgment.
///
/// Returns `Some(...)` only when the session is alive **and** the missing
/// ratio is at/above `miss_ratio` **and** the server issued at least
/// `min_expected` packs (enough signal to accuse). Otherwise `None` — the
/// caller keeps the base band (e.g. "insufficient" for honest shortages).
pub fn assess_evidence_withheld(
    expected: &[String],
    received: &[String],
    session_alive: bool,
    min_expected: usize,
    miss_ratio: f64,
) -> Option<Value> {
    if !session_alive || expected.len() < min_expected.max(1) {
        return None;
    }
    let ledger = missing_ledger(expected, received);
    let ratio = ledger
        .get("missing_ratio")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    if ratio < miss_ratio {
        return None;
    }
    Some(json!({
        "evidence_withheld": true,
        "rule": "planned_vs_actual_ledger",
        "algo": "evidence_ledger_v1",
        "session_alive": true,
        "missing_ratio": ratio,
        "min_expected": min_expected,
        "threshold": miss_ratio,
        "expected_n": ledger.get("expected_n"),
        "received_n": ledger.get("received_n"),
        "missing_packs": ledger.get("missing_packs"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(packs: &[&str]) -> Value {
        let arr: Vec<Value> = packs
            .iter()
            .map(|p| json!({"pack_id": p, "schedule": "static"}))
            .collect();
        json!({"packs": arr})
    }

    #[test]
    fn expected_pack_ids_both_keys() {
        let v = json!({"packs": [
            {"pack_id": "B0_bootstrap"},
            {"batch_id": "B10_hw_curves"},
            {"pack_id": "", "batch_id": ""},
            {"other": 1}
        ]});
        assert_eq!(
            expected_pack_ids(&v),
            vec!["B0_bootstrap", "B10_hw_curves"]
        );
    }

    #[test]
    fn received_ids_from_evidence() {
        let ev = json!({"batches": [
            {"batch_id": "B0_bootstrap"},
            {"pack_id": "B5_census"},
            {"batch_id": "B10_hw_curves"}
        ]});
        assert_eq!(
            received_batch_ids(&ev),
            vec!["B0_bootstrap", "B5_census", "B10_hw_curves"]
        );
    }

    #[test]
    fn withheld_when_alive_and_high_missing() {
        let exp = expected_pack_ids(&route(&["A", "B", "C", "D"]));
        let recv = vec!["A".to_string()];
        let out = assess_evidence_withheld(&exp, &recv, true, 3, 0.5).expect("should trigger");
        assert_eq!(out["evidence_withheld"], true);
        assert_eq!(out["missing_ratio"], 0.75);
        assert_eq!(out["missing_packs"], json!(["B", "C", "D"]));
    }

    #[test]
    fn not_withheld_when_dead_session() {
        let exp = expected_pack_ids(&route(&["A", "B", "C", "D"]));
        let recv = vec!["A".to_string()];
        // Dead session = network drop or early leave — never accuse withholding.
        assert!(assess_evidence_withheld(&exp, &recv, false, 3, 0.5).is_none());
    }

    #[test]
    fn not_withheld_below_threshold() {
        let exp = expected_pack_ids(&route(&["A", "B", "C", "D"]));
        let recv = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        assert!(assess_evidence_withheld(&exp, &recv, true, 3, 0.5).is_none());
    }

    #[test]
    fn not_withheld_when_too_few_expected() {
        let exp = expected_pack_ids(&route(&["A", "B"]));
        let recv = vec![];
        // 1/2 missing but only 2 packs were ever asked — insufficient signal.
        assert!(assess_evidence_withheld(&exp, &recv, true, 3, 0.5).is_none());
    }

    #[test]
    fn expired_env_overrides_are_ignored() {
        std::env::set_var("GR_EW_MISS_RATIO", "0.8");
        std::env::set_var("GR_EW_MIN_EXPECTED", "77");
        assert_eq!(ew_miss_ratio(), 0.8);
        assert_eq!(ew_min_expected(), 77);
        // Out-of-range values fall back to defaults.
        std::env::set_var("GR_EW_MISS_RATIO", "7.0");
        std::env::set_var("GR_EW_MIN_EXPECTED", "1");
        assert_eq!(ew_miss_ratio(), EW_MISS_RATIO_DEFAULT);
        assert_eq!(ew_min_expected(), EW_MIN_EXPECTED_DEFAULT);
        std::env::remove_var("GR_EW_MISS_RATIO");
        std::env::remove_var("GR_EW_MIN_EXPECTED");
    }
}
