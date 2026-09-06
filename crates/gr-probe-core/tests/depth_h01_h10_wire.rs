//! N2/N3: H01 GPU-ns depth metrics + H10 codec/EME object depth must move stack/product.

use gr_probe_core::bot::score_bot;
use gr_probe_core::product_scores::{score_br, score_os};
use gr_probe_core::stack_auth::stack_auth_from_fields;
use gr_probe_core::xsrc::evaluate_xsrc;
use serde_json::json;

#[test]
fn gpu_ns_depth_strong_lowers_risk_signals() {
    let fields = json!({
        "user_agent": "Mozilla/5.0 Chrome/150",
        "platform": "Linux x86_64",
        "form_class": "desktop",
        "webdriver": false,
        "gpu_ns_points": 8,
        "gpu_ns_hardware_path": true,
        "gpu_r2_ns": 0.94,
        "gpu_ns_monotonic_ratio": 0.9,
        "gpu_ns_depth_score": 0.85,
        "gpu_ns_wall_ratio_median": 0.2,
        "gpu_ns_staircase": [{"size":32,"gpu_ns":1000},{"size":64,"gpu_ns":2000}],
        "gpu_ns_median": 1500.0,
    });
    let stack = stack_auth_from_fields(&fields);
    let rs = stack.reasons.join(",");
    assert!(
        rs.contains("gpu_r2_ns_strong") || rs.contains("gpu_ns_depth_strong") || rs.contains("gpu_ns_monotonic"),
        "expected depth reasons, got {rs}"
    );
}

#[test]
fn gpu_ns_depth_weak_raises_spoof() {
    let fields = json!({
        "user_agent": "Mozilla/5.0 Chrome/150",
        "platform": "Linux x86_64",
        "form_class": "desktop",
        "webdriver": false,
        "gpu_ns_points": 6,
        "gpu_ns_hardware_path": true,
        "gpu_r2_ns": 0.1,
        "gpu_ns_monotonic_ratio": 0.2,
        "gpu_ns_depth_score": 0.2,
        "gpu_ns_staircase": [1,2,3,4,5,6],
        "gpu_ns_median": 100.0,
    });
    let stack = stack_auth_from_fields(&fields);
    assert!(
        stack.spoof_score >= 0.1,
        "weak depth should raise spoof, got {}",
        stack.spoof_score
    );
    let rs = stack.reasons.join(",");
    assert!(
        rs.contains("gpu_r2_ns_low")
            || rs.contains("gpu_ns_non_monotonic")
            || rs.contains("gpu_ns_depth_weak"),
        "{rs}"
    );
}

#[test]
fn codec_eme_depth_virt_hint_and_rich() {
    let empty = json!({
        "user_agent": "Mozilla/5.0 Chrome/150",
        "platform": "Linux x86_64",
        "form_class": "desktop",
        "webdriver": false,
        "codec_matrix": {"a":0,"b":0,"c":0,"d":0,"e":0,"f":0,"g":0,"h":0},
        "codec_matrix_n": 16,
        "codec_support_score": 0,
        "codec_virt_hint": true,
        "codec_probably_n": 0,
        "eme_systems": {
            "com.widevine.alpha": "unsupported",
            "com.microsoft.playready": "unsupported",
            "com.apple.fps.1_0": "unsupported",
            "org.w3.clearkey": "unsupported"
        },
        "eme_supported_n": 0,
        "eme_unsupported_n": 4,
    });
    let stack = stack_auth_from_fields(&empty);
    assert!(stack.vm_score >= 0.1, "virt codec should raise vm {}", stack.vm_score);
    let rs = stack.reasons.join(",");
    assert!(
        rs.contains("codec_matrix_empty_virt_hint") || rs.contains("eme_all_unsupported"),
        "{rs}"
    );

    let rich = json!({
        "user_agent": "Mozilla/5.0 Chrome/150",
        "platform": "Linux x86_64",
        "form_class": "desktop",
        "webdriver": false,
        "canplay_matrix": {"video/mp4": "probably", "video/webm_vp9": "probably"},
        "codec_matrix": {"video/mp4": 2, "video/webm_vp9": 2, "audio/mp4_aac": 2},
        "codec_matrix_n": 16,
        "codec_support_score": 12,
        "codec_probably_n": 6,
        "codec_virt_hint": false,
        "eme_systems": {"com.widevine.alpha": "supported", "org.w3.clearkey": "supported"},
        "eme_supported_n": 2,
        "eme_unsupported_n": 2,
        "eme_widevine": true,
    });
    let stack2 = stack_auth_from_fields(&rich);
    let rs2 = stack2.reasons.join(",");
    assert!(rs2.contains("codec_matrix_rich") || rs2.contains("eme_widevine"), "{rs2}");
    assert!(rs2.contains("codec_probably_dense") || rs2.contains("eme_supported"));

    let bot = score_bot(&rich, "balanced").unwrap();
    let truth = evaluate_xsrc(
        &json!({
            "fields": rich,
            "sources": ["main"],
            "batches": [{"batch_id":"B19_eme_media","source":"main"}]
        }),
        None,
    )
    .unwrap();
    // H10 materials feed OS axis (device/env stack), not only br
    let os = score_os(&rich, &stack2, &truth, &[]);
    let hits = os["field_hits"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect::<Vec<_>>();
    assert!(
        hits.iter().any(|h| h == "codec_eme_depth" || h == "eme_widevine" || h == "eme_supported"),
        "os hits={hits:?}"
    );
    let _br = score_br(&rich, &stack2, &bot, &truth, &[]);
}
