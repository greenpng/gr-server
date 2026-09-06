//! iss/opus5 04-P0-3: default slim projection for analysis responses.
//!
//! Measured on lab data: a full evaluate result averages ~0.5 MB uncompressed
//! (58% internal state — diagnostics 17%, brain 17%, coverage 7%), and the
//! analyze envelope additionally duplicated `device`/`product`/`diagnostics`
//! at top level, yielding ~1.9 MB responses. The browser/SDK control loop
//! only needs route/coverage/terminal keys plus identity conclusions, so the
//! default response is an allowlist projection; `verbose=1` with an ops
//! credential returns the full envelope.

use serde_json::{json, Map, Value};

/// Coverage keys the FE self-heal / terminal detection actually reads.
const COVERAGE_KEEP: &[&str] = &[
    "has_b10",
    "received_batch_ids",
    "coverage_complete",
    "brain_schedule_final",
    "final_analysis_ok",
    "analysis_terminal",
    "probe_complete",
    "identity_complete",
    "missing_batches",
    "b10x_missing",
    "stop_probe",
    "halt_uploads",
];

/// Top-level result keys required by the FE control loop and SDK consumers.
/// Everything not listed here (diagnostics, selection, static_wave, task_gaps,
/// control_persist, battle_log, device_hypothesis, edh, strategy, belief, …)
/// is internal state and stays out of the default response.
const RESULT_KEEP: &[&str] = &[
    "route_plan",
    "coverage",
    "cycle_probe_status",
    "identity_coverage",
    "analysis_rev",
    "rev",
    "plan_epoch",
    "plan_version",
    "real_band",
    "bot_verdict",
    "analysis_terminal",
    "stop_probe",
    "halt_uploads",
    "cycle_complete",
    "cycle_closes",
    "brain_schedule_final",
    "final_analysis_ok",
    "skip_session_probe",
    "b10x_missing",
    "b10_present",
    "probe_complete",
    "session_ticket",
    "sdk_return",
    "rpa_analyze",
    "link",
    "page",
    "product_version",
    "session_id",
    "tenant_id",
    "analyze_reuse",
    "analyze_dirty",
    "product_action",
];

/// Device conclusion fields (identity answer, not mint internals).
const DEVICE_KEEP: &[&str] = &[
    "device_id",
    "device_tier",
    "digest_path",
    "confidence",
    "device_confidence",
    "commercial_identity_final",
    "peer_similarity",
    "homogenization",
];

/// Product conclusion fields (merchant-facing answer after return gate).
const PRODUCT_KEEP: &[&str] = &[
    "device_id",
    "real_band",
    "sdk_return",
    "sdk_projection",
    "recommended_action",
    "product_action",
];

fn pick(src: &Value, keys: &[&str]) -> Value {
    let mut out = Map::new();
    if let Some(o) = src.as_object() {
        for k in keys {
            if let Some(v) = o.get(*k) {
                out.insert((*k).to_string(), v.clone());
            }
        }
    }
    Value::Object(out)
}

/// Slimmed `coverage` block: control flags only, drops open_verification_gaps /
/// direction_ranking / dag_explain (≈85% of the block).
pub fn slim_coverage(cov: &Value) -> Value {
    pick(cov, COVERAGE_KEEP)
}

/// Slimmed `device` block: identity conclusion only (drops link_or_mint,
/// multi_source_mint_gate, trust materials, …).
pub fn slim_device(device: &Value) -> Value {
    pick(device, DEVICE_KEEP)
}

/// Slimmed `product` block: gated merchant conclusion only.
pub fn slim_product(product: &Value) -> Value {
    pick(product, PRODUCT_KEEP)
}

/// Project a full evaluate result into the default browser/SDK shape.
pub fn slim_analyze_result(result: &Value) -> Value {
    let mut out = pick(result, RESULT_KEEP);
    if let Some(o) = out.as_object_mut() {
        if let Some(cov) = result.get("coverage") {
            o.insert("coverage".into(), slim_coverage(cov));
        }
        // brain: FE only falls back to brain.route_plan / brain.coverage when
        // the top-level keys are missing; both are kept above, so the slim
        // form carries only the small stop_reason marker.
        if let Some(brain) = result.get("brain") {
            let mut b = Map::new();
            if let Some(sr) = brain.get("stop_reason") {
                b.insert("stop_reason".into(), sr.clone());
            }
            o.insert("brain".into(), Value::Object(b));
        }
        if let Some(d) = result.get("device") {
            o.insert("device".into(), slim_device(d));
        }
        if let Some(p) = result.get("product") {
            o.insert("product".into(), slim_product(p));
        }
        o.insert("response_projection".into(), json!("slim_v1"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slim_drops_internal_state_keeps_control_keys() {
        let full = json!({
            "route_plan": {"packs": [], "stop_probe": false},
            "coverage": {"has_b10": true, "received_batch_ids": ["B0_bootstrap"], "open_verification_gaps": [{"huge": "x".repeat(1000)}]},
            "diagnostics": {"analysis_quality": {"blob": "y".repeat(5000)}},
            "brain": {"route_plan": {"packs": []}, "coverage": {"direction_ranking": [1,2,3]}, "stop_reason": "done", "packs": [{"p": 1}]},
            "device": {"device_id": "dv0-x", "device_tier": "dv", "link_or_mint": {"candidates": [1,2,3]}, "trust": {"materials": [0.1]}},
            "product": {"device_id": "dv0-x", "sdk_return": {"emit": true}, "field_utilization": {"big": "z".repeat(2000)}},
            "selection": {"candidates": [1]},
            "analysis_rev": 7,
            "real_band": "confirmed_real",
            "session_ticket": {"ticket": "st_abc"},
        });
        let slim = slim_analyze_result(&full);
        // Control keys survive
        assert!(slim.get("route_plan").is_some());
        assert_eq!(slim.get("analysis_rev").and_then(|v| v.as_i64()), Some(7));
        assert_eq!(
            slim.get("real_band").and_then(|v| v.as_str()),
            Some("confirmed_real")
        );
        assert!(slim.get("session_ticket").is_some());
        // Coverage flags survive, heavy gaps dropped
        assert_eq!(
            slim.pointer("/coverage/has_b10").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert!(slim.pointer("/coverage/received_batch_ids").is_some());
        assert!(slim.pointer("/coverage/open_verification_gaps").is_none());
        // Internal state dropped
        assert!(slim.get("diagnostics").is_none());
        assert!(slim.get("selection").is_none());
        assert!(slim.pointer("/brain/route_plan").is_none());
        assert_eq!(
            slim.pointer("/brain/stop_reason").and_then(|v| v.as_str()),
            Some("done")
        );
        // Device conclusion kept, mint internals dropped
        assert_eq!(
            slim.pointer("/device/device_id").and_then(|v| v.as_str()),
            Some("dv0-x")
        );
        assert!(slim.pointer("/device/link_or_mint").is_none());
        assert!(slim.pointer("/product/field_utilization").is_none());
        assert!(slim.pointer("/product/sdk_return").is_some());
        // Size sanity: the 8KB+ fixture must shrink dramatically
        let full_len = serde_json::to_string(&full).unwrap().len();
        let slim_len = serde_json::to_string(&slim).unwrap().len();
        assert!(
            slim_len * 10 < full_len,
            "slim {slim_len} should be << full {full_len}"
        );
    }

    #[test]
    fn slim_tolerates_missing_blocks() {
        let slim = slim_analyze_result(&json!({"rev": 1}));
        assert_eq!(slim.get("rev").and_then(|v| v.as_i64()), Some(1));
        assert!(slim.get("device").is_none());
        assert_eq!(
            slim.get("response_projection").and_then(|v| v.as_str()),
            Some("slim_v1")
        );
    }
}
