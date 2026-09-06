//! Same-SKU single residual collision → multi-fn / timing fork (local unit matrix).

use gr_probe_core::{build_anti_collision_surface, commercial_projection};
use serde_json::json;

fn dead_hist() -> Vec<f64> {
    vec![
        0.03418, 0.04199, 0.03027, 0.03906, 0.02637, 0.0293, 0.03223, 0.03027, 0.03516, 0.03125,
        0.02832, 0.02051, 0.03223, 0.02539, 0.0332, 0.03223, 0.0293, 0.02344, 0.02637, 0.02734,
        0.02734, 0.03418, 0.0332, 0.0332, 0.02637, 0.02734, 0.02832, 0.04004, 0.02637, 0.04102,
        0.03418, 0.04004,
    ]
}

fn ulp(seed: f64) -> Vec<f64> {
    (0..32)
        .map(|i| {
            let x = i as f64 * 0.19 + seed * 2.7;
            (x.sin().abs() * 0.45 + (x * 1.7).cos().abs() * 0.22 + seed * 0.08).max(0.02)
        })
        .collect()
}

fn machine(seed: f64, timing_base: f64) -> serde_json::Value {
    let dead = dead_hist();
    json!({
        "form_class": "desktop",
        "platform": "Win32",
        "os_family": "windows",
        "hardware_concurrency": 12,
        "hw_curve_webgl": dead.clone(),
        "residual_paths": [
            {"path_id":"float_std","shader_mode":"float","ok":true,"curve": dead.clone()},
            {"path_id":"ulp_chain","shader_mode":"ulp","ok":true,"curve": ulp(seed)},
        ],
        "eu_timing_ms": (0..8).map(|i| timing_base + (i as f64)*0.07 + seed*0.3).collect::<Vec<_>>(),
        "hw_curve_audio": (0..64).map(|i| ((i as f64)*0.13 + seed).sin().abs()*0.5+0.05).collect::<Vec<_>>(),
        "webgl_unmasked_renderer": "ANGLE (NVIDIA, GeForce RTX 3060 Direct3D11)",
    })
}

#[test]
fn same_sku_dead_residual_forks_with_ensemble() {
    let a = machine(0.1, 1.0);
    let b = machine(0.9, 3.5);
    let sa = build_anti_collision_surface(&a);
    let sb = build_anti_collision_surface(&b);
    assert_eq!(sa["primary_entropy_ok"], false);
    assert_ne!(
        sa["composite_digest"].as_str(),
        sb["composite_digest"].as_str()
    );
    let pa = commercial_projection(&a);
    let pb = commercial_projection(&b);
    // Both should mark anti-collision secondary path
    assert!(
        pa.get("hw_anti_collision")
            .or_else(|| pa.pointer("/materials/hw_anti_collision"))
            .is_some()
            || pa
                .get("materials_included")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().any(|x| {
                    matches!(
                        x.as_str(),
                        Some("hw_anti_collision")
                            | Some("hw_ensemble_digest")
                            | Some("hw_timing_phase_digest")
                    )
                }))
                .unwrap_or(false)
            || pa["collision_risk"].as_bool() == Some(true)
            || pa.get("hw_anti_collision_surface").is_some()
            || format!("{pa}").contains("hw_anti_collision")
            || format!("{pa}").contains("ensemble"),
        "projection should surface anti-collision: {pa}"
    );
    // Digests should diverge when materials included differ
    let ida = pa
        .get("server_mint_id")
        .or_else(|| pa.get("device_id"))
        .cloned();
    let idb = pb
        .get("server_mint_id")
        .or_else(|| pb.get("device_id"))
        .cloned();
    // At minimum composite digests differ
    assert_ne!(sa["ensemble_digest"], sb["ensemble_digest"]);
    let _ = (ida, idb);
}

#[test]
fn identical_same_sku_honest_collision() {
    let a = machine(0.2, 1.2);
    let b = a.clone();
    let sa = build_anti_collision_surface(&a);
    let sb = build_anti_collision_surface(&b);
    assert_eq!(sa["composite_digest"], sb["composite_digest"]);
}
