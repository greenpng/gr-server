//! SDK return policy for os / br / device_id.
//!
//! Emit to SDK only when:
//! 1) identity probe tasks for the vt/cycle are complete, and
//! 2) that vt has had **no uploads for ≥ 1 minute**.
//!
//! RPA is **not** gated here (page-scoped; close flush or 30s idle).

use serde_json::{json, Value};

/// Vt idle threshold before SDK may receive os/br/device (ms).
pub const SDK_RETURN_IDLE_MS: i64 = 60_000;

/// Overridable via admin config (process global); min 10s enforced by store clamp.
pub fn sdk_return_idle_ms() -> i64 {
    static V: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
    let x = V.load(std::sync::atomic::Ordering::Relaxed);
    if x >= 10_000 {
        x
    } else {
        SDK_RETURN_IDLE_MS
    }
}

pub fn set_sdk_return_idle_ms(ms: i64) {
    static V: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
    V.store(ms.max(10_000), std::sync::atomic::Ordering::Relaxed);
}

/// When true (default), commercial silicon + B10x done → analysis_terminal / cool.
pub fn complete_on_commercial_silicon() -> bool {
    static V: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(1);
    V.load(std::sync::atomic::Ordering::Relaxed) != 0
}

pub fn set_complete_on_commercial_silicon(on: bool) {
    static V: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(1);
    V.store(if on { 1 } else { 0 }, std::sync::atomic::Ordering::Relaxed);
}

/// Page RPA analyze after no RPA upload for this long (ms).
/// Contract: FE `rpa_monitor.js` `RPA_IDLE_MS` must equal this (iss/50 R2).
/// Flush reason codes: `idle_30s` (canonical); `idle_45s` accepted for legacy clients.
pub const RPA_IDLE_ANALYZE_MS: i64 = 30_000;

/// Pure decision: whether product os/br/device may be returned to the FE/SDK.
///
/// Emit when **any**:
/// 1) probe complete **and** vt idle ≥ 1m (legacy strict path)
/// 2) pagehide/final flush **and** min evidence (short-visit / close path)
/// 3) vt idle ≥ 1m **and** min evidence (stop-uploading without full complete)
pub fn should_return_identity_to_sdk(
    probe_complete: bool,
    last_upload_ms: Option<i64>,
    now_ms: i64,
) -> Value {
    should_return_identity_to_sdk_ex(probe_complete, last_upload_ms, now_ms, false, false)
}

/// Extended gate with pagehide + min-evidence flags (from evaluate).
pub fn should_return_identity_to_sdk_ex(
    probe_complete: bool,
    last_upload_ms: Option<i64>,
    now_ms: i64,
    pagehide_final: bool,
    has_min_evidence: bool,
) -> Value {
    let idle_ms = last_upload_ms.map(|t| (now_ms - t).max(0));
    let thr = sdk_return_idle_ms();
    let idle_ok = idle_ms.map(|i| i >= thr).unwrap_or(false);
    let mut reasons = Vec::new();
    let mut emit = false;
    if probe_complete && idle_ok {
        emit = true;
        reasons.push("probe_complete_and_vt_idle_1m".into());
    } else if pagehide_final && has_min_evidence {
        emit = true;
        reasons.push("pagehide_min_evidence".into());
    } else if idle_ok && has_min_evidence {
        emit = true;
        reasons.push("vt_idle_1m_min_evidence".into());
    } else {
        if !probe_complete {
            reasons.push("probe_incomplete".to_string());
        }
        if !idle_ok {
            reasons.push("vt_upload_idle_lt_1m".to_string());
        }
        if pagehide_final && !has_min_evidence {
            reasons.push("pagehide_without_min_evidence".into());
        }
    }
    json!({
        "emit": emit,
        "probe_complete": probe_complete,
        "pagehide_final": pagehide_final,
        "has_min_evidence": has_min_evidence,
        "last_upload_ms": last_upload_ms,
        "idle_ms": idle_ms,
        "idle_threshold_ms": thr,
        "reasons": reasons,
    })
}

/// Pure decision: whether page RPA should be analyzed now.
pub fn should_analyze_page_rpa(
    pagehide_final: bool,
    last_rpa_upload_ms: Option<i64>,
    now_ms: i64,
    has_rpa_materials: bool,
) -> Value {
    if !has_rpa_materials {
        return json!({
            "analyze": false,
            "reasons": ["no_rpa_materials"],
        });
    }
    let idle_ms = last_rpa_upload_ms.map(|t| (now_ms - t).max(0));
    let idle_ok = idle_ms.map(|i| i >= RPA_IDLE_ANALYZE_MS).unwrap_or(false);
    let analyze = pagehide_final || idle_ok;
    let mut reasons = Vec::new();
    if pagehide_final {
        reasons.push("pagehide_final_flush".to_string());
    }
    if idle_ok {
        reasons.push("rpa_idle_ge_30s".to_string());
    }
    if !analyze {
        reasons.push("waiting_rpa_idle_or_close".to_string());
    }
    json!({
        "analyze": analyze,
        "pagehide_final": pagehide_final,
        "idle_ms": idle_ms,
        "idle_threshold_ms": RPA_IDLE_ANALYZE_MS,
        "reasons": reasons,
    })
}

/// Annotate product for SDK return policy.
///
/// Server-side analysis **keeps** os/br/device scores on `product` for continuous
/// re-analysis and brain routing. FE/SDK must only treat identity as final when
/// `sdk_return.emit == true` (and `identity_withheld == false`).
pub fn apply_identity_return_gate(product: &Value, gate: &Value) -> Value {
    let emit = gate.get("emit").and_then(|v| v.as_bool()).unwrap_or(false);
    let mut out = product.clone();
    if let Some(obj) = out.as_object_mut() {
        obj.insert("sdk_return".into(), gate.clone());
        // Slim projection always attached for SDK consumers (perf/security/stability).
        obj.insert(
            "sdk_projection".into(),
            sdk_slim_projection(product, emit),
        );
        if !emit {
            obj.insert("identity_withheld".into(), json!(true));
            obj.insert(
                "identity_withheld_reason".into(),
                gate.get("reasons").cloned().unwrap_or(json!([])),
            );
            // Explicit legs FE may return only after emit (document contract).
            obj.insert(
                "sdk_identity_legs".into(),
                json!([
                    "os",
                    "br",
                    "device_id",
                    "device_tier",
                    "device_confidence",
                    "identity",
                    "mint_posture",
                    "ephemeral",
                    "ephemeral_ttl_ms",
                    "effective_bits",
                    "bucket_heat",
                    "collision_risk",
                    "peer_similarity",
                    "homogenization",
                    "curve_descriptors"
                ]),
            );
        } else {
            obj.insert("identity_withheld".into(), json!(false));
        }
    }
    out
}

/// SDK slim projection (default return surface).
///
/// **Decision**: slim whitelist by default — better performance (small JSON),
/// security (no raw evidence/curves/PII), stability (versioned contract).
/// Full product remains server-side for brain/ops; SDK uses `sdk_projection`.
pub fn sdk_slim_projection(product: &Value, emit_identity: bool) -> Value {
    let p = product.as_object();
    let score_only = |axis: &str| -> Value {
        product
            .get(axis)
            .map(|a| {
                json!({
                    "score": a.get("score"),
                    "status": a.get("status"),
                    "coverage": a.get("coverage"),
                    "environment_flags": a.get("environment_flags"),
                })
            })
            .unwrap_or(Value::Null)
    };
    let mut slim = json!({
        "algo": "sdk_slim_projection_v1",
        "emit_identity": emit_identity,
        "product_version": product.get("product_version"),
        // Site-owner allowlisted cookies captured server-side at open/ingest
        // (business-identifier binding). Present only when configured + sent.
        "cookie_fields": product.get("cookie_fields"),
        "peer_similarity": product.get("peer_similarity"),
        "homogenization": product.get("homogenization"),
        "environment_homogeneity": product
            .get("environment_homogeneity")
            .cloned()
            .or_else(|| {
                product
                    .pointer("/homogenization/environment_homogeneity")
                    .cloned()
            }),
        "rpa_bands": product.get("rpa_bands"),
        "behavior_self_sim": product.get("behavior_self_sim").map(|b| json!({
            "algo": b.get("algo"),
            "dim": b.get("dim"),
            "profile_digest": b.get("profile_digest"),
            "vec32": b.get("vec32"),
            "promote_to_commercial_id": false,
        })),
        "shadow_score": product.get("shadow_score").map(|s| json!({
            "algo": s.get("algo"),
            "shadow_risk": s.get("shadow_risk"),
            "shadow_safety": s.get("shadow_safety"),
            "drives_production_gate": false,
        })),
        "canary_model": product.get("canary_model").map(|c| json!({
            "algo": c.get("algo"),
            "signals_agree": c.get("signals_agree"),
            "delta_shadow_minus_prod": c.get("delta_shadow_minus_prod"),
            "bucket_hint": c.get("bucket_hint"),
            "traffic_split_active": false,
            "drives_production_gate": false,
        })),
        "claim_obs_graph": product.get("claim_obs_graph").map(|g| json!({
            "algo": g.get("algo"),
            "n_conflict": g.get("n_conflict"),
            "engine_claim": g.get("engine_claim"),
            "engine_obs": g.get("engine_obs"),
        })),
        "ja4h_lite": product.get("ja4h_lite").map(|j| json!({
            "algo": j.get("algo"),
            "present": j.get("present"),
            "ja4h_lite": j.get("ja4h_lite"),
            "commercial_mint": false,
        })),
        "config_version": product
            .get("config_version")
            .cloned()
            .or_else(|| product.pointer("/config_snapshot/config_version").cloned()),
        "environment_flags": product.get("environment_flags")
            .cloned()
            .or_else(|| product.pointer("/os/environment_flags").cloned()),
        "association_level": product.get("association_level"),
        "association_ladder": product.get("association_ladder").map(|l| json!({
            "association_level": l.get("association_level"),
            "claims_host_silicon": l.get("claims_host_silicon"),
            "continuity_band": l.get("continuity_band"),
            "host_separator": l.get("host_separator"),
        })),
        "curve_descriptors": product.get("curve_descriptors"),
        "os": score_only("os"),
        "br": score_only("br"),
        "rpa": score_only("rpa"),
        "privacy": {
            "silent_probe_no_permission_request": product
                .pointer("/product_redlines/silent_probe_no_permission_request")
                .cloned()
                .unwrap_or(json!(true)),
        },
        "note": "slim whitelist for SDK; full product stays on V5 for ops/brain",
    });
    if emit_identity {
        if let Some(obj) = slim.as_object_mut() {
            obj.insert("device_id".into(), product.get("device_id").cloned().unwrap_or(Value::Null));
            // iss/58: confidence estimate (1-FPP) + mint posture — not UV SLA
            obj.insert(
                "identity".into(),
                product.get("identity").cloned().unwrap_or(json!({
                    "confidence_kind": "calibrated_estimate_not_uv_sla",
                })),
            );
            obj.insert(
                "mint_posture".into(),
                product.get("mint_posture").cloned().unwrap_or(json!("stable")),
            );
            obj.insert(
                "ephemeral".into(),
                product.get("ephemeral").cloned().unwrap_or(json!(false)),
            );
            obj.insert(
                "ephemeral_ttl_ms".into(),
                product.get("ephemeral_ttl_ms").cloned().unwrap_or(json!(0)),
            );
            obj.insert(
                "collision_risk".into(),
                product.get("collision_risk").cloned().unwrap_or(json!(false)),
            );
            obj.insert(
                "effective_bits".into(),
                product.get("effective_bits").cloned().unwrap_or(Value::Null),
            );
            obj.insert(
                "bucket_heat".into(),
                product.get("bucket_heat").cloned().unwrap_or(Value::Null),
            );
            obj.insert(
                "device_id_segments".into(),
                product.get("device_id_segments").cloned().unwrap_or(Value::Null),
            );
            obj.insert(
                "device_id_reserved_compare".into(),
                product
                    .get("device_id_reserved_compare")
                    .cloned()
                    .unwrap_or(Value::Null),
            );
            obj.insert(
                "device_tier".into(),
                product.get("device_tier").cloned().unwrap_or(Value::Null),
            );
            // multi-segment mint (device_segments_v1): required by lab matrix / SDK
            // commercial readers that only look at product (not device.*).
            let multi = product
                .get("multi_segment")
                .cloned()
                .or_else(|| {
                    let tier = product
                        .get("device_tier")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let did = product
                        .get("device_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if tier == "multi"
                        || did.starts_with("dv0-")
                        || did.starts_with("dv4-")
                        || did.starts_with("dv5-")
                        || did.starts_with("dv6-")
                    {
                        Some(json!(true))
                    } else {
                        Some(json!(false))
                    }
                })
                .unwrap_or(json!(false));
            obj.insert("multi_segment".into(), multi);
            obj.insert(
                "device_algo_group".into(),
                product
                    .get("device_algo_group")
                    .or_else(|| product.get("algo_group"))
                    .cloned()
                    .unwrap_or(Value::Null),
            );
            obj.insert(
                "digest_path".into(),
                product.get("digest_path").cloned().unwrap_or(Value::Null),
            );
            obj.insert(
                "device_confidence".into(),
                product
                    .get("device_confidence")
                    .or_else(|| product.get("confidence"))
                    .cloned()
                    .unwrap_or(Value::Null),
            );
            obj.insert(
                "conf_ceiling".into(),
                product.get("conf_ceiling").cloned().unwrap_or(Value::Null),
            );
            obj.insert(
                "slot_scheme".into(),
                product.get("slot_scheme").cloned().unwrap_or(json!("extended_curve_v2")),
            );
            obj.insert(
                "browser_surface_id".into(),
                product.get("browser_surface_id").cloned().unwrap_or(Value::Null),
            );
        }
    } else {
        if let Some(obj) = slim.as_object_mut() {
            obj.insert("identity_withheld".into(), json!(true));
        }
    }
    let _ = p;
    slim
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_emit_until_complete_and_idle() {
        let g = should_return_identity_to_sdk(false, Some(1_000), 70_000);
        assert_eq!(g["emit"], false);
        let g2 = should_return_identity_to_sdk(true, Some(1_000), 30_000);
        assert_eq!(g2["emit"], false);
        let g3 = should_return_identity_to_sdk(true, Some(1_000), 70_000);
        assert_eq!(g3["emit"], true);
        let g4 = should_return_identity_to_sdk_ex(false, Some(1_000), 1_500, true, true);
        assert_eq!(g4["emit"], true);
    }

    #[test]
    fn rpa_triggers_on_close_or_30s() {
        let a = should_analyze_page_rpa(true, Some(0), 1000, true);
        assert_eq!(a["analyze"], true);
        let b = should_analyze_page_rpa(false, Some(0), 31_000, true);
        assert_eq!(b["analyze"], true);
        let c = should_analyze_page_rpa(false, Some(0), 10_000, true);
        assert_eq!(c["analyze"], false);
        let d = should_analyze_page_rpa(true, None, 0, false);
        assert_eq!(d["analyze"], false);
    }

    #[test]
    fn sdk_projection_exposes_homogenization_and_curve_descriptors() {
        let product = json!({
            "product_version": "test",
            "peer_similarity": {"algo": "peer_similarity_v1", "score": 0.7, "severity": "high"},
            "homogenization": {
                "algo": "homogenization_v1",
                "severity": "high",
                "score": 0.6,
                "window_ms": 300000,
                "basis": ["hot_bucket_n=40"]
            },
            "curve_descriptors": {"algo": "curve_descriptors_v1", "slots": {}},
            "os": {"score": 0.8, "status": "ok"},
            "br": {"score": 0.7, "status": "ok"},
            "rpa": {"score": 0.5, "status": "ok"},
            "device_id": "dv0-a-b-c-d-e-f-g-h-i-j",
            "device_confidence": 0.7,
            "conf_ceiling": 0.95,
        });
        let slim = sdk_slim_projection(&product, true);
        assert_eq!(slim["algo"], "sdk_slim_projection_v1");
        assert_eq!(slim["homogenization"]["algo"], "homogenization_v1");
        assert_eq!(slim["homogenization"]["severity"], "high");
        assert_eq!(slim["peer_similarity"]["score"], 0.7);
        assert_eq!(slim["curve_descriptors"]["algo"], "curve_descriptors_v1");
        assert_eq!(slim["device_id"], "dv0-a-b-c-d-e-f-g-h-i-j");
        assert_eq!(slim["conf_ceiling"], 0.95);
    }

    #[test]
    fn write_product_shape_evidence_for_goal() {
        use crate::device_segments::select_device_segments;
        use crate::soft_v2::{homogenization_product_signal, SoftConfig};
        let fields = json!({
            "residual_mean": 0.261,
            "hw_curve_webgl": [0.11,0.12,0.13,0.14,0.15,0.16,0.17,0.18],
            "hw_curve_audio": [0.21,0.22,0.23,0.24,0.25,0.26,0.27,0.28],
            "hw_curve_cpu": [0.01,0.02,0.03,0.04,0.05,0.06,0.07,0.08],
            "os_instance_hash": "oi_probe_x",
            "webrtc_host_ip_hash": "rtc_probe_y",
            "bucket_member_count": 48,
            "env_class": "residential",
            "h2_priority_fingerprint": "0",
            "h3_pseudo_order": ":method,:path",
            "cpu_cache_ladder": [1,2,4,8],
            "gl_governor_active": true,
            "governed_webgl_renderer": "ANGLE shared pool",
        });
        let tier = select_device_segments(&fields, Some(&json!({"sources":["main"]})));
        let homo = homogenization_product_signal(
            "residential",
            &json!({"bucket_member_count": 48}),
            SoftConfig::default().hot_bucket_threshold,
            SoftConfig::default().p1_window_ms,
        );
        let product = json!({
            "device_id": tier.get("device_id"),
            "device_id_reserved_compare": tier.get("device_id_reserved_compare"),
            "device_confidence": tier.get("device_confidence"),
            "conf_ceiling": tier.get("conf_ceiling"),
            "curve_descriptors": tier.get("curve_descriptors"),
            "slot_scheme": tier.get("slot_scheme"),
            "peer_similarity": {"algo":"peer_similarity_v1","score":0.55,"severity":"medium","window_sec":900,"promote_to_commercial_id":false},
            "homogenization": homo,
            "os": {"score": 0.72, "status": "ok", "coverage": 0.6},
            "br": {"score": 0.68, "status": "ok", "coverage": 0.5},
            "rpa": {"score": 0.5, "status": "ok", "coverage": 0.4},
            "product_version": "goal_verify",
        });
        let gate = json!({"emit": true, "reasons": ["goal_verify"]});
        let gated = apply_identity_return_gate(&product, &gate);
        let slim = gated.get("sdk_projection").cloned().unwrap();
        // Soft never promote
        assert_eq!(slim["homogenization"]["promote_to_commercial_id"], false);
        assert!(slim.get("curve_descriptors").is_some());
        assert!(slim.get("peer_similarity").is_some());
        // Fixed names not in device body
        let did = tier["device_id"].as_str().unwrap();
        assert!(!did.contains("residential"));
        assert!(!did.contains("ANGLE"));
        // reserved compare may differ when extended curves present
        assert!(tier.get("device_id_reserved_compare").is_some());
        if let Some(path) = gr_abi::env::get("PRODUCT_SHAPE_OUT") {
            let out = json!({
                "device_id": did,
                "device_id_reserved_compare": tier.get("device_id_reserved_compare"),
                "conf_ceiling": tier.get("conf_ceiling"),
                "material_slots_addressable": tier.get("material_slots_addressable"),
                "sdk_projection": slim,
                "homogenization": gated.get("homogenization"),
                "protocol_notes": {
                    "h2_priority_diagnostic": true,
                    "b34_not_in_device_body": !did.contains("cache"),
                    "gl_governor_isolated": true,
                },
            });
            std::fs::write(&path, serde_json::to_string_pretty(&out).unwrap()).unwrap();
        }
    }
}
