use gr_probe_core::device_segments::select_device_segments;
use gr_probe_core::model_key::{commercial_model_key_ready, model_key_from_fields};
use serde_json::{json, Value};

fn ar_of(seg: &Value) -> String {
    seg.pointer("/device_id_segments/dv0")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .split('-')
        .nth(6)
        .unwrap_or("?")
        .to_string()
}

fn notes_of(seg: &Value) -> Vec<String> {
    for path in [
        "/curve_selection_notes",
        "/notes",
        "/device_id_notes",
        "/segment_notes",
        "/mint_notes",
        "/slot_notes",
    ] {
        if let Some(a) = seg.pointer(path).and_then(|v| v.as_array()) {
            let n: Vec<String> = a
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .filter(|s| s.contains("ar_"))
                .collect();
            if !n.is_empty() {
                return n;
            }
        }
    }
    vec![]
}

fn main() {
    let curve8: Vec<f64> = (0..8).map(|i| 0.1 + i as f64 * 0.05).collect();
    let curve32: Vec<f64> = (0..32).map(|i| 0.1 + i as f64 * 0.01).collect();
    let curve36: Vec<f64> = (0..36)
        .map(|i| 0.2 + (i % 6) as f64 * 0.4 + (i / 6) as f64 * 0.02)
        .collect();
    let webgl: Vec<f64> = (0..16).map(|i| 0.2 + i as f64 * 0.01).collect();

    let scenarios: Vec<(&str, Value)> = vec![
        (
            "thin_adapter_only",
            json!({
                "webgpu_compute_ok": true,
                "webgpu_adapter_surface": "nvidia-discrete",
                "webgpu_limits_hash": "abc",
                "webgpu_features_hash": "def",
                "residual_mean": 0.26,
                "hw_curve_webgl": webgl,
            }),
        ),
        (
            "curve8_no_challenge",
            json!({
                "hw_curve_webgpu": curve8,
                "webgpu_compute_ok": true,
                "residual_mean": 0.26,
                "hw_curve_webgl": webgl,
            }),
        ),
        (
            "curve8_with_challenge",
            json!({
                "hw_curve_webgpu": curve8,
                "webgpu_compute_ok": true,
                "webgpu_challenge_seed_used": true,
                "residual_mean": 0.26,
                "hw_curve_webgl": webgl,
            }),
        ),
        (
            "curve32_with_challenge",
            json!({
                "hw_curve_webgpu": curve32,
                "webgpu_compute_ok": true,
                "webgpu_challenge_seed_used": true,
                "residual_mean": 0.26,
                "hw_curve_webgl": webgl,
            }),
        ),
        (
            "curve36_with_challenge",
            json!({
                "hw_curve_webgpu": curve36,
                "webgpu_compute_ok": true,
                "webgpu_challenge_seed_used": true,
                "residual_mean": 0.26,
                "hw_curve_webgl": webgl,
            }),
        ),
        (
            "curve32_timing_no_challenge",
            json!({
                "hw_curve_webgpu": curve32,
                "webgpu_eu_timing_curve": [1.0, 1.2, 1.1, 1.3],
                "residual_mean": 0.26,
                "hw_curve_webgl": webgl,
            }),
        ),
        (
            "digest_only",
            json!({
                "hw_webgpu_compute_digest": "deadbeefcafebabe",
                "webgpu_adapter_surface": "nvidia-discrete",
                "residual_mean": 0.26,
                "hw_curve_webgl": webgl,
            }),
        ),
        (
            "surface_only",
            json!({
                "webgpu_adapter_surface": "nvidia-discrete",
                "residual_mean": 0.26,
                "hw_curve_webgl": webgl,
            }),
        ),
        (
            "limits_features_only",
            json!({
                "webgpu_limits_hash": "limhash1",
                "webgpu_features_hash": "feathash1",
                "residual_mean": 0.26,
                "hw_curve_webgl": webgl,
            }),
        ),
        (
            "named_k_fields",
            json!({
                "webgl_unmasked_renderer": "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 Direct3D11 vs_5_0 ps_5_0, D3D11)",
                "webgl_unmasked_vendor": "Google Inc. (NVIDIA)",
                "gl_max_texture_size": 16384,
                "gl_max_renderbuffer": 16384,
                "residual_mean": 0.26,
                "hw_curve_webgl": webgl,
            }),
        ),
        (
            "class_t_only",
            json!({
                "gl_max_texture_size": 16384,
                "gl_max_renderbuffer": 16384,
                "webgl_unmasked_renderer": "Google SwiftShader",
                "residual_mean": 0.26,
                "hw_curve_webgl": webgl,
            }),
        ),
    ];

    println!("CRATE={}", env!("CARGO_PKG_NAME"));
    for (name, fields) in scenarios {
        let seg = select_device_segments(&fields, None);
        let ar = ar_of(&seg);
        let notes = notes_of(&seg);
        let did = seg
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let mk_info = model_key_from_fields(&fields);
        let k = mk_info
            .get("hw_model_key")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let k_ready = commercial_model_key_ready(&mk_info);
        if name == "thin_adapter_only" {
            if let Some(obj) = seg.as_object() {
                let keys: Vec<_> = obj.keys().collect();
                eprintln!("SEG_KEYS={keys:?}");
            }
        }
        println!(
            "SCENARIO={name}\tar={ar}\tk={k}\tk_ready={k_ready}\tdv0={did}\tnotes={}",
            notes.join("|")
        );
    }
}
