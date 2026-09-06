//! Research redlines for cross-browser **device_id** (iss research 2026-07-30).
//!
//! Drives shipped SSOT: `field_product_matrix.commercial_device_id` and
//! `trust::webgl_commercial_digest` / commercial_projection.
//! Proves L0 forgeable signals stay out of commercial digest and residual anchors stay in.

use gr_probe_core::{
    commercial_device_id_from_fields, commercial_projection, load_field_product_matrix,
    webgl_commercial_digest, webgl_peak_signature,
};
use serde_json::json;
use std::collections::HashSet;

#[test]
fn never_digest_includes_research_l0_machine_forbidden() {
    let m = load_field_product_matrix().expect("matrix");
    let never: HashSet<&str> = m
        .commercial_device_id
        .never_digest
        .iter()
        .map(|s| s.as_str())
        .collect();
    // Research L0: must never be commercial machine join keys
    for k in [
        "user_agent",
        "webgl_unmasked_renderer",
        "webgl_unmasked_vendor",
        "server_client_ip",
        "ja3_hash",
        "ja4",
        "tls_ja4",
        "screen_width",
        "screen_height",
    ] {
        assert!(
            never.contains(k),
            "never_digest must include research-forbidden {k}; got {never:?}"
        );
    }
}

#[test]
fn digest_order_prefers_residual_not_labels() {
    let m = load_field_product_matrix().expect("matrix");
    let order = &m.commercial_device_id.digest_order;
    assert!(
        order.iter().any(|s| s == "hw_webgl_stable" || s == "hw_audio_stable"),
        "digest_order must include residual anchors: {order:?}"
    );
    for bad in [
        "user_agent",
        "webgl_unmasked_renderer",
        "server_client_ip",
        "ja3_hash",
        "ja4",
    ] {
        assert!(
            !order.iter().any(|s| s == bad),
            "digest_order must not include {bad}"
        );
    }
    let anchors = &m.commercial_device_id.hardware_anchors;
    assert!(
        anchors.iter().any(|s| s.contains("webgl") || s.contains("audio")),
        "hardware_anchors must reference residual curves: {anchors:?}"
    );
}

#[test]
fn commercial_webgl_ignores_bin_layout_keeps_peak_sig_separate() {
    // Same mass, different bin layout (WebKit vs ANGLE class) → same commercial digest
    let mut a = vec![0.02_f64; 32];
    a[2] = 0.16;
    a[8] = 0.12;
    a[14] = 0.10;
    let s: f64 = a.iter().sum();
    for x in &mut a {
        *x /= s;
    }
    let mut b = vec![0.02_f64; 32];
    b[5] = 0.16;
    b[19] = 0.12;
    b[27] = 0.10;
    let s2: f64 = b.iter().sum();
    for x in &mut b {
        *x /= s2;
    }
    let ca = webgl_commercial_digest(&a).expect("ca");
    let cb = webgl_commercial_digest(&b).expect("cb");
    assert_eq!(
        ca, cb,
        "commercial webgl must be layout-invariant (mag-rank): {ca} vs {cb}"
    );
    // Peak signature is allowed to differ (conf / engine-class)
    let pa = webgl_peak_signature(&a);
    let pb = webgl_peak_signature(&b);
    assert!(pa.is_some() && pb.is_some());
}

/// Lab-shaped: L0 renderer strings fork (1050 Ti vs spoofed 980 vs Apple GPU label)
/// while residual mass matches → same commercial id (L2 over L0).
#[test]
fn lab_shaped_l0_renderer_fork_same_l2_residual_id() {
    let residual: Vec<f64> = (0..32)
        .map(|i| (i as f64 * 0.17 + 0.3).sin().abs() * 0.25 + 0.04)
        .collect();
    let audio: Vec<f64> = (0..64)
        .map(|i| (i as f64 * 0.11).sin().abs() * 0.35 + 0.05)
        .collect();
    let labels = [
        "ANGLE (NVIDIA Corporation, NVIDIA GeForce GTX 1050 Ti/PCIe/SSE2, OpenGL 4.5.0)",
        "NVIDIA GeForce GTX 980, or similar",
        "Apple GPU",
    ];
    let mut ids = Vec::new();
    for (i, lab) in labels.iter().enumerate() {
        let f = json!({
            "form_class": "desktop",
            "platform": "Linux x86_64",
            "os_family": "linux",
            "hardware_concurrency": 12,
            "timezone": "Asia/Singapore",
            "architecture": "x86",
            "screen_width": 1920,
            "screen_height": 1080,
            "hw_curve_webgl": residual,
            "hw_curve_audio": audio,
            "webgl_unmasked_renderer": lab,
            "user_agent": format!("Browser/{}", i),
            "server_client_ip": format!("1.1.1.{}", i + 1),
            "ja3_hash": format!("ja3_{i}"),
        });
        ids.push(commercial_device_id_from_fields(&f).expect("id"));
    }
    assert!(
        ids.iter().all(|x| x == &ids[0]),
        "L0 label/IP/JA3 fork must not split L2 residual commercial id: {ids:?}"
    );
    // Soft SwiftShader residual class must not claim same path as real residual id
    let soft = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "os_family": "linux",
        "hardware_concurrency": 12,
        "timezone": "Asia/Singapore",
        "webgl_unmasked_renderer": "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero)))",
        "residual_soft_like": true,
        "soft_stack": true,
        "stack_class": "soft_render",
        "hw_curve_webgl": residual,
        "hw_curve_audio": audio,
        "server_client_ip": "127.0.0.1",
    });
    let soft_id = commercial_device_id_from_fields(&soft);
    // Soft commercial may be None (no host separator) or dv_ not equal to real residual id
    if let Some(sid) = soft_id {
        assert_ne!(sid, ids[0], "soft path must not share commercial id with real residual bag");
        assert!((sid.starts_with("dv-") || sid.starts_with("dv_")), "soft commercial if any is dv_ not dh body alone: {sid}");
    }
}

#[test]
fn commercial_id_ignores_ja3_and_renderer_string_changes() {
    let webgl: Vec<f64> = (0..32)
        .map(|i| (i as f64 * 0.19).sin().abs() * 0.3 + 0.05)
        .collect();
    let audio: Vec<f64> = (0..64)
        .map(|i| (i as f64 * 0.11).sin().abs() * 0.4 + 0.05)
        .collect();
    let base = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "os_family": "linux",
        "hardware_concurrency": 12,
        "timezone": "UTC",
        "architecture": "x86",
        "hw_curve_webgl": webgl,
        "hw_curve_audio": audio,
        "server_client_ip": "1.1.1.1",
        "webgl_unmasked_renderer": "ANGLE (NVIDIA GeForce GTX 1050 Ti)",
        "user_agent": "Chrome/120",
        "ja3_hash": "deadbeef",
        "ja4": "t13d1516h2_xxx",
    });
    let mut spoofed = base.clone();
    if let Some(o) = spoofed.as_object_mut() {
        o.insert(
            "webgl_unmasked_renderer".into(),
            json!("NVIDIA GeForce GTX 980, or similar"),
        );
        o.insert("user_agent".into(), json!("Firefox/121"));
        o.insert("ja3_hash".into(), json!("cafebabe"));
        o.insert("ja4".into(), json!("t13d1715h1_yyy"));
        o.insert("server_client_ip".into(), json!("9.9.9.9"));
    }
    let id_a = commercial_device_id_from_fields(&base).expect("id_a");
    let id_b = commercial_device_id_from_fields(&spoofed).expect("id_b");
    assert_eq!(
        id_a, id_b,
        "commercial id must ignore renderer/UA/JA3/IP spoof surface"
    );
    let p = commercial_projection(&base);
    let included = p["materials_included"].as_array().unwrap();
    for bad in ["user_agent", "webgl_unmasked_renderer", "server_client_ip", "ja3_hash", "ja4"] {
        assert!(
            !included.iter().any(|v| v.as_str() == Some(bad)),
            "{bad} must not be materials_included"
        );
    }
    assert!(
        included.iter().any(|v| v.as_str() == Some("hw_webgl_stable")),
        "webgl residual must be included when present"
    );
}
