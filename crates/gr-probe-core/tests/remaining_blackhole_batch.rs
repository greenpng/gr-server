//! Remaining blackhole batch: wire_now + selective wire_if_easy must move stack/product.
#![recursion_limit = "256"]

use gr_probe_core::bot::score_bot;
use gr_probe_core::product_scores::{score_br, score_os};
use gr_probe_core::stack_auth::stack_auth_from_fields;
use gr_probe_core::xsrc::evaluate_xsrc;
use serde_json::json;

#[test]
fn remaining_batch_hits_and_reasons() {
    let fields = json!({
        "user_agent": "Mozilla/5.0 Chrome/150",
        "platform": "Linux x86_64",
        "form_class": "desktop",
        "webdriver": false,
        "challenge_seed_digest": "abcd",
        "permission_notification": "prompt",
        "permissions_prompt_n": 4,
        "canplay_av1": "probably",
        "canplay_hevc": "",
        "codec_empty_n": 2,
        "codec_maybe_n": 3,
        "codec_matrix_n": 16,
        "eme_clearkey": true,
        "font_token_checked": 12,
        "gpu_ns_poll_ms": 120,
        "gpu_ns_points": 6,
        "material_cross_reasons": ["a"],
        "material_consistency": 0.9,
        "material_keys": ["hw", "canvas", "audio"],
        "media_prefers_color_scheme": "dark",
        "media_prefers_reduced_motion": "no-preference",
        "orientation_angle": 0,
        "orientation_type": "landscape-primary",
        "perf_now_delta_n": 12,
        "sandbox_residual_mean": 0.5001,
        "main_residual_mean": 0.5002,
        "screen_pixel_depth": 24,
        "screen_color_depth": 24,
        "session_residual_repeat": 0.01,
        "thermal_cpu_cv": 0.1,
        "cookie_count": 3,
        "connection": "4g",
        "device_motion": false,
        "gl_max_cube_map": 16384,
        "highp_precision": 23,
        "native_checked": 20,
        "neg_dict_hash": "n1",
        "offscreen_available": true,
        "perf_dom_content_loaded_ms": 100,
        "perf_time_origin": 1.0,
        "roundtrip_slope": 0.01,
        "total_js_heap_size": 50_000_000,
        "used_js_heap_size": 10_000_000,
        "ws_protocol": "",
        "ws_ready_state": 1,
        "challenge_audio_freq": 440,
        "quic_aead_ok": true,
        "quic_fp": "q1",
    });
    let stack = stack_auth_from_fields(&fields);
    let rs = stack.reasons.join(",");
    for need in [
        "challenge_seed_digest_present",
        "permission_notification_present",
        "canplay_modern_codecs",
        "eme_clearkey_present",
        "sandbox_main_residual_ok",
        "screen_depth_present",
        "quic_aead_clienthello",
        "websocket_depth_present",
        "material_meta_present",
    ] {
        assert!(rs.contains(need), "missing {need} in {rs}");
    }

    let bot = score_bot(&fields, "balanced").unwrap();
    let truth = evaluate_xsrc(
        &json!({
            "fields": fields,
            "sources": ["main"],
            "batches": [{"batch_id":"B0_bootstrap","source":"main"}]
        }),
        None,
    )
    .unwrap();
    let br = score_br(&fields, &stack, &bot, &truth, &[]);
    let os = score_os(&fields, &stack, &truth, &[]);
    let hits: Vec<String> = br["field_hits"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(os["field_hits"].as_array().into_iter().flatten())
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    for need in [
        "challenge_seed_digest",
        "canplay_modern_codecs",
        "eme_clearkey",
        "material_keys",
        "sandbox_residual_mean",
        "screen_depth",
        "quic_aead",
        "websocket_depth",
        "heap_storage_runtime",
    ] {
        assert!(hits.iter().any(|h| h == need), "missing hit {need}: {hits:?}");
    }
}

#[test]
fn codec_mostly_empty_and_sandbox_mismatch() {
    let fields = json!({
        "user_agent": "Mozilla/5.0 Chrome/150",
        "platform": "Linux x86_64",
        "form_class": "desktop",
        "webdriver": false,
        "codec_empty_n": 14,
        "codec_matrix_n": 16,
        "main_residual_mean": 0.5,
        "sandbox_residual_mean": 0.55,
        "screen_color_depth": 8,
    });
    let stack = stack_auth_from_fields(&fields);
    assert!(stack.vm_score > 0.0 || stack.spoof_score > 0.0);
    let rs = stack.reasons.join(",");
    assert!(rs.contains("codec_mostly_empty"), "{rs}");
    assert!(rs.contains("sandbox_main_residual_mismatch"), "{rs}");
    assert!(rs.contains("screen_color_depth_low"), "{rs}");
}
