//! Hardware-noise soft association: same host across different IPs/kernels.
//! Soft never promotes to commercial device_id.

use gr_probe_core::soft_v2::{
    cosine_similarity, extract_hw_curves, hw_noise_similarity, soft_pair_decision,
    PROMOTE_TO_COMMERCIAL_ID,
};
use serde_json::json;

fn curve_a() -> Vec<f64> {
    (0..32).map(|i| (i as f64 * 0.07).sin() + 0.1).collect()
}
fn curve_b_similar() -> Vec<f64> {
    // Same shape + small noise (same chip, different browser render path)
    curve_a()
        .into_iter()
        .enumerate()
        .map(|(i, v)| v + (i as f64 * 0.001).cos() * 0.02)
        .collect()
}
fn curve_other_machine() -> Vec<f64> {
    (0..32).map(|i| (i as f64 * 0.31).cos() * 2.0 + 1.5).collect()
}

#[test]
fn cosine_detects_similar_curves() {
    let a = curve_a();
    let b = curve_b_similar();
    let c = curve_other_machine();
    let s_ab = cosine_similarity(&a, &b).unwrap();
    let s_ac = cosine_similarity(&a, &c).unwrap();
    assert!(s_ab > 0.95, "similar curves {s_ab}");
    assert!(s_ab > s_ac, "other machine less similar {s_ab} vs {s_ac}");
}

#[test]
fn soft_edge_cross_ip_via_hw_noise_never_promotes() {
    let audio = curve_a();
    let canvas: Vec<f64> = (0..16).map(|i| (i as f64 + 1.0) / 16.0).collect();
    let cpu: Vec<f64> = (0..24).map(|i| 1.0 + (i as f64 * 0.05).sin() * 0.1).collect();

    let ev_a = json!({
        "session_id": "s_chrome_proxy1",
        "ts_ms": 1_000_000,
        "fields": {
            "os_family": "windows", // spoofed
            "hardware_concurrency": 4,
            "timezone": "America/New_York",
            "screen_width": 1920,
            "screen_height": 1080,
            "form_class": "desktop",
            "server_client_ip": "198.51.100.10",
            "hw_curve_audio": audio,
            "hw_curve_canvas": canvas,
            "hw_curve_cpu": cpu,
        }
    });
    let audio2 = curve_b_similar();
    let canvas2: Vec<f64> = canvas.iter().map(|v| v * 1.01).collect();
    let cpu2: Vec<f64> = cpu.iter().enumerate().map(|(i,v)| v + i as f64 * 0.0005).collect();
    let ev_b = json!({
        "session_id": "s_firefox_proxy2",
        "ts_ms": 1_000_500,
        "fields": {
            "os_family": "macos", // different spoof
            "hardware_concurrency": 8,
            "timezone": "Europe/London",
            "screen_width": 1366,
            "screen_height": 768,
            "form_class": "desktop",
            "server_client_ip": "203.0.113.55", // different proxy IP
            "hw_curve_audio": audio2,
            "hw_curve_canvas": canvas2,
            "hw_curve_cpu": cpu2,
        }
    });
    let out = soft_pair_decision(&ev_a, &ev_b, None);
    assert_eq!(out["promote_to_commercial_id"], false);
    assert!(!PROMOTE_TO_COMMERCIAL_ID);
    assert_eq!(out["server_ip_differs"], true);
    assert_eq!(out["hw_noise"]["eligible"], true, "{out}");
    let edge = out.get("edge").expect("edge");
    assert!(!edge.is_null(), "expected soft edge: {out}");
    assert_eq!(edge["promote_to_commercial_id"], false);
    let reason = edge["reason"].as_str().unwrap_or("");
    assert!(
        reason.contains("hw_noise"),
        "reason should be hw_noise*: {reason}"
    );
}

#[test]
fn different_machine_noise_no_soft_edge() {
    let ev_a = json!({
        "session_id": "m1",
        "ts_ms": 1,
        "fields": {
            "server_client_ip": "1.1.1.1",
            "hw_curve_audio": curve_a(),
            "hw_curve_canvas": (0..16).map(|i| i as f64).collect::<Vec<_>>(),
            "hw_curve_cpu": (0..24).map(|i| 1.0).collect::<Vec<_>>(),
        }
    });
    let ev_b = json!({
        "session_id": "m2",
        "ts_ms": 2,
        "fields": {
            "server_client_ip": "2.2.2.2",
            "hw_curve_audio": curve_other_machine(),
            "hw_curve_canvas": (0..16).map(|i| (15 - i) as f64).collect::<Vec<_>>(),
            "hw_curve_cpu": (0..24).map(|i| 3.0 + i as f64).collect::<Vec<_>>(),
        }
    });
    let out = soft_pair_decision(&ev_a, &ev_b, None);
    assert_eq!(out["hw_noise"]["eligible"], false, "{out}");
    // No hs2 either → no edge
    assert!(out.get("edge").map(|e| e.is_null()).unwrap_or(true), "{out}");
}

#[test]
fn extract_nested_or_flat_curves() {
    let f = json!({
        "hw_curve_audio": [0.1, 0.2, 0.3, 0.4, 0.5],
        "hw_noise_curves": { "canvas": [1.0, 2.0, 3.0, 4.0, 5.0] }
    });
    let c = extract_hw_curves(&f);
    assert_eq!(c.audio.len(), 5);
    assert_eq!(c.canvas.len(), 5);
    let sim = hw_noise_similarity(&c, &c);
    // only canvas+audio present, both identical → eligible if cos high
    assert!(sim["scores"]["audio"].as_f64().unwrap() > 0.99);
}
