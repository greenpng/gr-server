//! Env-capable gaps must not stay "intentionally open": H09 full, GPU-ns fields,
//! protocol edge inject, demo-method digests (errors/speech/display/audio).

use gr_probe_core::{
    build_frontier, headers_map_from_pairs, inject_protocol_from_headers, scan_gaps,
    stack_auth_from_fields,
};
use serde_json::json;

#[test]
fn protocol_edge_injects_edge_browser_ja4() {
    let mut gw = serde_json::Map::new();
    let headers = headers_map_from_pairs(&[
        ("X-TLS-JA4".into(), "ed_t13d1516h2_aabbccdd".into()),
        ("X-HTTP2-Fingerprint".into(), "edge-h2-lab".into()),
        ("X-TLS-ALPN".into(), "h2".into()),
    ]);
    inject_protocol_from_headers(&mut gw, &headers);
    assert_eq!(
        gw.get("ja4").and_then(|v| v.as_str()).unwrap(),
        "ed_t13d1516h2_aabbccdd"
    );
    assert_eq!(
        gw.get("protocol_engine").and_then(|v| v.as_str()).unwrap(),
        "edge"
    );
    assert_eq!(
        gw.get("protocol_fp_source")
            .and_then(|v| v.as_str())
            .unwrap(),
        "edge_header"
    );
    assert!(gw.get("h2_fingerprint").is_some());
}

#[test]
fn stack_auth_consumes_gpu_ns_thermal_full_audio_deep() {
    let fields = json!({
        "webgl_unmasked_renderer": "ANGLE (NVIDIA GeForce GTX 1050 Ti)",
        "residual_mean": 0.5001811906403189,
        "gpu_ns_staircase": [{"size":64,"iter":8,"gpu_ns":1200000}],
        "gpu_ns_median": 1200000.0,
        "gpu_ns_points": 8,
        "gpu_slope_ns": 0.5,
        "gpu_ns_wall_ratio_median": 0.35,
        "gpu_ns_hardware_path": true,
        "timer_query_available": true,
        "thermal_cpu_slope_full": 0.12,
        "thermal_bursts_full": [{"i":0,"cpu_ms":1.0}],
        "errors_engine_hash": "abc123",
        "speech_voices_hash": "sp01",
        "speech_voices_count": 12,
        "display_mq_hash": "dm01",
        "hdr_likely": false,
        "audio_deep_hash": "ad01",
        "audio_deep_moments": {"mean": 0.01, "rms": 0.02},
        "audio_deep_peak_bins": [0.1, 0.2],
        "ja4": "ed_t13d1516h2_x",
        "protocol_engine": "edge"
    });
    let a = stack_auth_from_fields(&fields);
    let joined = a.reasons.join("|");
    assert!(
        joined.contains("gpu_ns") || joined.contains("hardware"),
        "reasons={}",
        joined
    );
    assert!(
        joined.contains("thermal_full") || joined.contains("thermal"),
        "reasons={}",
        joined
    );
    assert!(
        joined.contains("audio_deep")
            || joined.contains("errors_engine")
            || joined.contains("speech")
            || joined.contains("display"),
        "reasons={}",
        joined
    );
}

#[test]
fn frontier_schedules_h09_full_gpu_ns_audio_demo_methods() {
    let evidence = json!({
        "session_id": "env_capable",
        "sources": ["main", "gateway"],
        "batches": [
            {"batch_id":"B0_bootstrap","source":"main"},
            {"batch_id":"B8_gateway","source":"gateway"},
            {"batch_id":"B37_thermal_drift_lite","source":"main"},
            {"batch_id":"B22_gpu_timer","source":"main"}
        ],
        "fields": {
            "form_class": "desktop",
            "platform": "Linux",
            "webdriver": false,
            "user_agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/138.0.0.0 Safari/537.36 Edg/138.0.0.0",
            "gpu_wall_staircase": [{"size":32,"iter":4,"wall_ms":1.2},{"size":64,"iter":8,"wall_ms":2.1}],
            "timer_query_available": true,
            "gpu_ns_points": 0,
            "thermal_cpu_slope": 0.05,
            "thermal_bursts": [{"i":0,"cpu_ms":1.0}],
            "behavior_count": 10,
            "page_dwell_ms": 25000,
            "webgl_unmasked_renderer": "ANGLE (NVIDIA GeForce GTX 1050 Ti Direct3D11)"
        },
        "gateway_fields": {},
        "has_gateway": true
    });
    let gaps = scan_gaps(&evidence, None).expect("scan_gaps");
    let codes: Vec<String> = gaps.iter().map(|g| g.code.clone()).collect();
    let code_s = codes.join(",");
    assert!(
        codes.iter().any(|c| {
            c.contains("thermal_full")
                || c.contains("gpu_ns")
                || c.contains("audio_deep")
                || c.contains("errors_engine")
                || c.contains("display_hdr")
                || c.contains("speech")
        }),
        "expected env-capable gap codes, got {code_s}"
    );

    let f = build_frontier(&evidence, true, Some(true), 40).unwrap();
    let packs: Vec<String> = f
        .packs
        .iter()
        .filter_map(|p| {
            p.get("pack_id")
                .or_else(|| p.get("id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect();
    let ps = packs.join(",");
    let pack_hit = packs.iter().any(|p| {
        p.contains("B42")
            || p.contains("B22")
            || p.contains("B46")
            || p.contains("B43")
            || p.contains("B44")
            || p.contains("B45")
    });
    let gap_hit = f.gaps.iter().any(|g| {
        g.code.contains("thermal_full")
            || g.code.contains("gpu_ns")
            || g.code.contains("audio_deep")
            || g.code.contains("errors")
            || g.code.contains("display")
            || g.code.contains("speech")
    });
    assert!(
        pack_hit || gap_hit,
        "expected B42/B22/B46/demo packs or gaps: packs={ps} codes={code_s}"
    );
}
