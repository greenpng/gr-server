//! Commercial multi-path silicon fusion — same-SKU fleet split matrix.

use gr_probe_core::{
    commercial_projection, fuse_silicon_channels, select_device_segments,
};
use serde_json::json;

fn dead_hist() -> Vec<f64> {
    vec![
        0.03418, 0.04199, 0.03027, 0.03906, 0.02637, 0.0293, 0.03223, 0.03027, 0.03516, 0.03125,
        0.02832, 0.02051, 0.03223, 0.02539, 0.0332, 0.03223, 0.0293, 0.02344, 0.02637, 0.02734,
        0.02734, 0.03418, 0.0332, 0.0332, 0.02637, 0.02734, 0.02832, 0.04004, 0.02637, 0.04102,
        0.03418, 0.04004,
    ]
}

fn rint_like() -> Vec<f64> {
    (0..32)
        .map(|i| {
            let x = i as f64;
            ((x * 13.0).sin().abs() * 0.3 + (x * 0.07).cos().abs() * 0.15 + 0.08).max(0.02)
        })
        .collect()
}

fn ulp_like(seed: f64) -> Vec<f64> {
    (0..32)
        .map(|i| {
            let x = i as f64 * 0.23 + seed * 4.1;
            (x.sin().abs() * 0.55 + (x * 2.1).cos().abs() * 0.28 + seed * 0.12).max(0.04)
        })
        .collect()
}

fn machine(seed: f64) -> serde_json::Value {
    let d = dead_hist();
    // Prod-like class floor mean (ANGLE hist) — multipath / ULP / EU must still split
    let residual_mean = 0.2603390625_f64;
    let ulp = ulp_like(seed);
    let ulp_mean = ulp.iter().sum::<f64>() / ulp.len() as f64;
    let ulp_var = ulp.iter().map(|x| (x - ulp_mean) * (x - ulp_mean)).sum::<f64>() / ulp.len() as f64;
    json!({
        "form_class": "desktop",
        "platform": "Win32",
        "os_family": "windows",
        "hardware_concurrency": 16,
        "residual_mean": residual_mean,
        "residual_std": 0.008,
        "hw_curve_webgl": d.clone(),
        "residual_path_means": [residual_mean, residual_mean + 0.0003, residual_mean + 0.0001, ulp_mean],
        "residual_path_stds": [0.008, 0.012, 0.010, ulp_var.sqrt()],
        "residual_path_modes": ["float", "rint", "noderiv", "ulp"],
        "residual_paths": [
            {"path_id":"v3f_128_std","shader_mode":"float","ok":true,"entropy_ok":false,
             "mean": residual_mean, "std": 0.008, "curve": d.clone()},
            {"path_id":"rint_warm2_128","shader_mode":"rint","ok":true,"entropy_ok":true,
             "mean": residual_mean + 0.0003, "std": 0.012, "curve": rint_like()},
            {"path_id":"v3f_128_noderiv","shader_mode":"noderiv","ok":true,"entropy_ok":true,
             "mean": residual_mean + 0.0001, "std": 0.010, "curve": rint_like()},
            {"path_id":"ulp_anti_collision_128","shader_mode":"ulp","ok":true,"entropy_ok":true,
             "mean": ulp_mean, "std": ulp_var.sqrt(),
             "curve": ulp,
             "eu_timing_ms": (0..10).map(|i| 1.0 + seed * 2.0 + i as f64 * 0.08).collect::<Vec<_>>()
            },
        ],
        "eu_timing_ms": (0..10).map(|i| 1.0 + seed * 2.0 + i as f64 * 0.08).collect::<Vec<_>>(),
        "timing_quality_score": 0.85,
        "hw_curve_audio": (0..64).map(|i| ((i as f64) * 0.15 + seed).sin().abs() * 0.55 + 0.05).collect::<Vec<_>>(),
        "audio_sample_rate": if seed < 0.5 { 44100 } else { 48000 },
        "audio_base_latency": 0.005 + seed * 0.01,
        "audio_max_channel_count": if seed < 0.5 { 2 } else { 6 },
        "hw_curve_webgpu": (0..32).map(|i| ((i as f64) * 0.33 + seed * 1.7).cos().abs() * 0.4 + 0.04).collect::<Vec<_>>(),
        "cpu_timing_curve": (0..32).map(|i| ((i as f64) * 0.09 + seed).sin().abs() * 0.2 + 0.1).collect::<Vec<_>>(),
        "hw_curve_cpu": (0..32).map(|i| ((i as f64) * 0.09 + seed).sin().abs() * 0.2 + 0.1).collect::<Vec<_>>(),
        "gl_max_texture_size": 16384,
        "gl_max_renderbuffer": 16384,
        "webgl_extensions_hash": "angle_nv_3060_class",
        "webgl_depth_bits": 24,
        "webgl_samples": 0,
        "gl_max_varying_vectors": 30,
        "gl_high_float": [23, 127, 127],
        "webgl_unmasked_renderer": "ANGLE (NVIDIA, GeForce RTX 3060 Direct3D11)",
    })
}

#[test]
fn commercial_fusion_splits_same_sku_fleet() {
    let m0 = machine(0.1);
    let m1 = machine(0.55);
    let m2 = machine(0.92);
    let f0 = fuse_silicon_channels(&m0);
    let f1 = fuse_silicon_channels(&m1);
    let f2 = fuse_silicon_channels(&m2);
    assert!(f0["same_sku_separability"].as_f64().unwrap() >= 0.5, "{f0}");
    assert_ne!(f0["hw_silicon_fusion"], f1["hw_silicon_fusion"]);
    assert_ne!(f1["hw_silicon_fusion"], f2["hw_silicon_fusion"]);
    assert_ne!(f0["hw_silicon_fine"], f2["hw_silicon_fine"]);

    let p0 = commercial_projection(&m0);
    let p1 = commercial_projection(&m1);
    // Fusion materials present
    let s = format!("{p0}");
    assert!(
        s.contains("hw_silicon_fusion") || s.contains("near_silicon") || s.contains("strong_machine"),
        "projection missing fusion surface: {p0}"
    );
    let _ = p1;
}

#[test]
fn multi_segment_body_prefers_fused_silicon_curve() {
    let m = machine(0.4);
    let seg = select_device_segments(&m, None);
    let id = seg["device_id"].as_str().unwrap_or("");
    assert!(id.starts_with("dv0-") || id.contains('-'), "id={id}");
    // should not be all zeros on wg/au when curves present
    assert!(!id.contains("-0-0-0-0-0-0-0-0-0-0"), "empty segment body: {id}");
}

#[test]
fn identical_machines_stable_fusion() {
    let a = machine(0.33);
    let b = a.clone();
    assert_eq!(
        fuse_silicon_channels(&a)["hw_silicon_fusion"],
        fuse_silicon_channels(&b)["hw_silicon_fusion"]
    );
}

/// Commercial segment body across a synthetic same-SKU fleet (n=8, shared
/// residual class floor): full dv0 ids must fork; the res slot is the Lane-C
/// 2-dec floor (shared by design — iss/73/doc07: uniqueness for same-floor
/// GPUs comes from wg/au/cp/… slots, not "res soup"), while the multipath
/// ULP/EU divergence still forks the Lane-S diagnostic sig.
#[test]
fn multipath_segment_res_forks_same_mean_fleet() {
    let seeds: [f64; 8] = [0.05, 0.18, 0.31, 0.44, 0.57, 0.70, 0.83, 0.96];
    let mut res_tokens = std::collections::BTreeSet::new();
    let mut full_ids = std::collections::BTreeSet::new();
    let mut lane_s_tokens = std::collections::BTreeSet::new();
    for s in seeds {
        let m = machine(s);
        let seg = select_device_segments(&m, None);
        let id = seg
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(id.starts_with("dv0-"), "bad id={id}");
        let parts: Vec<&str> = id.split('-').collect();
        // dv0-res-wg-au-cp-of-ar-cc-tz-oi-rtc → res is index 1
        let res = parts.get(1).copied().unwrap_or("0");
        assert_ne!(res, "0", "res missing for seed={s} id={id}");
        res_tokens.insert(res.to_string());
        full_ids.insert(id.to_string());
        if let Some(ls) = seg.get("lane_s_sig").and_then(|v| v.as_str()) {
            lane_s_tokens.insert(ls.to_string());
        }
    }
    // All 8 full IDs unique — commercial split must come from the body as a whole.
    assert_eq!(full_ids.len(), 8, "full dv0 collisions in fleet: {full_ids:?}");
    // res = shared Lane-C floor for the same-mean fleet (iss/73): stable, not a splitter.
    assert_eq!(
        res_tokens.len(),
        1,
        "res must stay the Lane-C 2-dec floor across a same-mean fleet: {res_tokens:?}"
    );
    // ULP/EU/advanced-path divergence still forks the Lane-S diagnostic sig.
    assert!(
        lane_s_tokens.len() >= 6,
        "lane_s multipath sig must fork ({}/8): {lane_s_tokens:?}",
        lane_s_tokens.len()
    );
}

/// Fusion + anti-collision + segments pipeline smoke on dead residual materials.
#[test]
fn multipath_pipeline_fusion_then_segments() {
    let m = machine(0.42);
    let fusion = fuse_silicon_channels(&m);
    assert!(
        fusion["same_sku_separability"].as_f64().unwrap_or(0.0) >= 0.5,
        "sep too low: {fusion}"
    );
    // Inject fusion digests into fields (product path does this via prefer_fused_curves)
    let mut fo = m.as_object().cloned().unwrap();
    if let Some(d) = fusion.get("hw_silicon_fusion").and_then(|v| v.as_str()) {
        fo.insert("hw_silicon_fusion".into(), json!(d));
    }
    if let Some(d) = fusion.get("hw_silicon_fine").and_then(|v| v.as_str()) {
        fo.insert("hw_silicon_fine".into(), json!(d));
    }
    let seg = select_device_segments(&serde_json::Value::Object(fo), None);
    let id = seg
        .pointer("/device_id_segments/dv0")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let res = id.split('-').nth(1).unwrap_or("0");
    assert_ne!(res, "0", "pipeline res empty: {id} fusion={fusion}");
    let notes = format!("{seg}");
    assert!(
        notes.contains("multipath") || notes.contains("res_includes"),
        "expected multipath note in segments: {notes}"
    );
}
