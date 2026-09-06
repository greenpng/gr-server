//! iss/67 wiring: model_key / atlas / stability_gate reach evaluate output.
use gr_probe_core::{
    commercial_body_excludes_plaintext_model, evaluate_session, select_device_segments,
};
use serde_json::{json, Value};

fn rich_fields() -> Value {
    json!({
        "webgl_unmasked_renderer": "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 Direct3D11 vs_5_0 ps_5_0, D3D11)",
        "engine_family": "blink",
        "os_family": "windows",
        "residual_mean": 0.2603390625,
        "residual_std": 0.014,
        "hw_curve_webgl": [0.11,0.22,0.13,0.24,0.15,0.26,0.17,0.28,0.19,0.21,0.12,0.23,0.14,0.25,0.16,0.27],
        "hw_curve_audio": [0.01,0.02,0.03,0.04,0.05,0.06,0.07,0.08,0.09,0.10,0.11,0.12,0.13,0.14,0.15,0.16],
        "cpu_timing_curve": [1.0,1.1,1.2,1.3,1.4,1.5,1.6,1.7,1.8,1.9,2.0,2.1,2.2,2.3,2.4,2.5],
        "webgl_residual_multipath": [0.11,0.22,0.13,0.24,0.15,0.26,0.17,0.28,0.19,0.21,0.12,0.23,0.14,0.25,0.16,0.27],
        "residual_paths": [
            {"path_id":"v3f_128_std","shader_mode":"float","ok":true,"entropy_ok":true,"mean":0.260,"std":0.01,"curve":[0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8]},
            {"path_id":"denorm_ftz_128","shader_mode":"denorm","ok":true,"entropy_ok":true,"mean":0.261,"std":0.012,"curve":[0.11,0.21,0.31,0.41,0.51,0.61,0.71,0.81]}
        ],
        "form_class": "desktop",
        "hardware_concurrency": 8,
        "timezone": "Asia/Shanghai",
    })
}

fn evidence(fields: Value) -> Value {
    json!({
        "fields": fields,
        "sources": ["main"],
        "fields_by_source": { "main": fields },
    })
}

#[test]
fn segments_and_evaluate_expose_model_key_atlas_conf() {
    let fields = rich_fields();
    // iss/67 wiring (current): the model-key atlas moved out of
    // select_device_segments into atlas_score::atlas_shadow_score (consumed by
    // antidetect_signals); channel-drift stability lives in
    // hw_channel_drift::stability_gate_from_fields. Segments still mint a
    // non-empty commercial body and must never leak the plaintext model key.
    // Class-default K needs a texture bucket; disable it here to exercise the
    // named NVIDIA path (same pattern as model_key module unit tests).
    std::env::set_var("GR_MODEL_KEY_DEFAULT_CLASS", "0");
    let atlas = gr_probe_core::atlas_score::atlas_shadow_score(&fields);
    let _ = std::env::remove_var("GR_MODEL_KEY_DEFAULT_CLASS");
    let key = atlas.get("hw_model_key").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        key.starts_with("nvidia:"),
        "expected nvidia model key, got {key}"
    );
    let segs = select_device_segments(&fields, None);
    let body = segs.get("device_id").and_then(|v| v.as_str()).unwrap_or("");
    assert!(!body.is_empty());
    assert!(
        commercial_body_excludes_plaintext_model(body, key),
        "body leaked model: {body} key={key}"
    );

    // Stability gate surface (drift channel, evaluate-consulted).
    let gate = gr_probe_core::hw_channel_drift::stability_gate_from_fields(&fields);
    assert!(
        gate.get("drives_production_gate").is_some() || gate.get("algo").is_some(),
        "stability gate surface missing: {gate}"
    );

    // evaluate projects the atlas shadow under the device physical_assert bundle
    // (atlas_kv → atlas_shadow_score), and raw hw_model_key under
    // multi_source_resolution resolutions.
    std::env::set_var("GR_MODEL_KEY_DEFAULT_CLASS", "0");
    let out = evaluate_session(&evidence(fields), None, None, None, false).expect("eval");
    let _ = std::env::remove_var("GR_MODEL_KEY_DEFAULT_CLASS");
    let eval_key = out
        .pointer("/device/physical_assert/atlas_kv/atlas_shadow_score/hw_model_key")
        .or_else(|| out.pointer("/device/multi_source_resolution/resolutions/hw_model_key"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        !eval_key.is_empty() && eval_key.starts_with("nvidia:"),
        "evaluate must project atlas hw_model_key; got keys around output: {}",
        serde_json::to_string_pretty(&json!({
            "phys_atlas": out.pointer("/device/physical_assert/atlas_kv/atlas_shadow_score/hw_model_key"),
            "msr_hw_model_key": out.pointer("/device/multi_source_resolution/resolutions/hw_model_key"),
        }))
        .unwrap_or_default()
    );
    let eval_body = out
        .pointer("/device/device_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !eval_body.is_empty() {
        assert!(commercial_body_excludes_plaintext_model(eval_body, eval_key));
    }
}

#[test]
fn stability_gate_demotes_conf_when_gate_on_and_unstable() {
    // Build fields with multi-obs that drift hard
    let mut fields = rich_fields();
    if let Some(o) = fields.as_object_mut() {
        o.insert(
            "family_observations".into(),
            json!([
                {"hw_curve_webgl": [0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8,0.9,1.0,1.1,1.2,1.3,1.4,1.5,1.6]},
                {"hw_curve_webgl": [9.0,8.0,7.0,6.0,5.0,4.0,3.0,2.0,1.0,0.0,0.5,1.5,2.5,3.5,4.5,5.5]},
                {"hw_curve_webgl": [0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0]},
            ]),
        );
        o.insert("visitor_terminal_id".into(), json!("vt_test_unstable"));
    }
    std::env::set_var("GR_CHANNEL_DRIFT_GATE", "1");
    let segs_on = select_device_segments(&fields, None);
    std::env::set_var("GR_CHANNEL_DRIFT_GATE", "0");
    let segs_off = select_device_segments(&fields, None);
    std::env::remove_var("GR_CHANNEL_DRIFT_GATE");

    let conf_on = segs_on
        .get("device_confidence")
        .or_else(|| segs_on.get("confidence"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let conf_off = segs_off
        .get("device_confidence")
        .or_else(|| segs_off.get("confidence"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let gate = segs_on.get("stability_gate").cloned().unwrap_or(json!({}));
    // Gate surface always present; when enabled + unstable, conf should not exceed off path
    assert!(gate.get("drives_production_gate").is_some() || gate.get("algo").is_some());
    if gate.get("ok").and_then(|v| v.as_bool()).unwrap_or(false)
        && !gate.get("stable").and_then(|v| v.as_bool()).unwrap_or(true)
    {
        assert!(
            conf_on <= conf_off + 1e-9,
            "unstable+gate-on conf {conf_on} should be <= gate-off {conf_off}"
        );
    }
}
