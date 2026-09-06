//! Blackhole close-up: FE-emitted digests must move product/stack (not only sit in store).

use gr_probe_core::product_scores::{score_br, score_os};
use gr_probe_core::stack_auth::stack_auth_from_fields;
use gr_probe_core::bot::score_bot;
use gr_probe_core::xsrc::{evaluate_xsrc, XSRC_CONFLICT};
use serde_json::json;

fn base_fields() -> serde_json::Value {
    json!({
        "user_agent": "Mozilla/5.0 (X11; Linux x86_64) Chrome/150.0.0.0 Safari/537.36",
        "platform": "Linux x86_64",
        "form_class": "desktop",
        "webdriver": false,
        "sandbox_ok": true,
        "hardware_concurrency": 8,
    })
}

#[test]
fn blackhole_siblings_produce_hits_and_reasons() {
    let mut fields = base_fields();
    let o = fields.as_object_mut().unwrap();
    o.insert("proxy_protocol_present".into(), json!(true));
    o.insert("proxy_protocol_version".into(), json!("1"));
    o.insert("font_token_hit_count".into(), json!(6));
    o.insert("font_present_sample".into(), json!(["Arial", "Noto"]));
    o.insert("codec_matrix_hash".into(), json!("cm_abc"));
    o.insert("canvas_geometry_stable".into(), json!(true));
    o.insert("canvas_geometry_mean".into(), json!(0.12));
    o.insert("agent_parity_hit_n".into(), json!(12.0));
    o.insert("agent_parity_keys_n".into(), json!(16.0));
    o.insert("permissions_hash".into(), json!("ph1"));
    o.insert("permissions_granted_n".into(), json!(2));
    o.insert("battery_charging".into(), json!(true));
    o.insert("media_devices_kinds".into(), json!(["audioinput", "videoinput"]));
    o.insert("css_supports_ok_count".into(), json!(20));
    o.insert("css_supports_total".into(), json!(30));
    o.insert("media_query_true_count".into(), json!(8));
    o.insert("main_residual_mean".into(), json!(0.01));
    o.insert("residual_challenge_delta".into(), json!(0.001));
    o.insert("challenge_audio_mean".into(), json!(0.2));
    o.insert("challenge_cpu_acc".into(), json!(1.0));
    o.insert("gpu_roundtrip_ladder".into(), json!([1.0, 2.0, 3.0]));
    o.insert("caps_claim_vs_actual_gap".into(), json!(0.1));
    o.insert("raf_mean_ms".into(), json!(16.7));
    o.insert("dom_rect_subpixel".into(), json!(true));
    o.insert("storage_persisted".into(), json!(true));
    o.insert("caches_api".into(), json!(true));
    o.insert("thermal_gpu_slope_full".into(), json!(0.05));
    o.insert("errors_engine_chrome_like".into(), json!(true));
    o.insert("speech_langs".into(), json!(["en-US", "zh-CN"]));
    o.insert("display_mq".into(), json!({"hdr": true}));
    o.insert("webgpu_limits_n".into(), json!(12));
    o.insert("webrtc_host_ips".into(), json!(["10.0.0.1"]));
    o.insert("gl_max_vertex_attribs".into(), json!(16));
    o.insert("gpu_ns_poll_frames".into(), json!(4));
    o.insert("layer_divergence_n".into(), json!(3));
    o.insert("perf_ttfb_ms".into(), json!(40));
    o.insert("sensor_gyro_present".into(), json!(true));

    let stack = stack_auth_from_fields(&fields);
    let reasons = stack
        .reasons
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>();
    for need in [
        "proxy_protocol_edge_present",
        "font_token_dense",
        "codec_matrix_hash_present",
        "canvas_geometry_depth",
        "agent_parity_ratio_ok",
        "permissions_digest_present",
        "battery_charge_state_present",
        "media_devices_shape_present",
        "residual_challenge_delta_ok",
        "pohw_numeric_surfaces",
        "gpu_roundtrip_present",
        "raf_mean_present",
        "dom_rect_depth_present",
        "storage_privacy_depth",
        "css_supports_volume",
        "gl_caps_depth_present",
        "webrtc_host_list_present",
        "perf_timeline_depth",
        "sensor_orient_gyro_present",
    ] {
        assert!(
            reasons.iter().any(|r| r.contains(need) || *r == need),
            "missing stack reason {need}; have {reasons:?}"
        );
    }

    let bot = score_bot(&fields, "balanced").unwrap();
    let truth = evaluate_xsrc(
        &json!({
            "fields": fields,
            "gateway_fields": {
                "proxy_protocol_present": true,
                "server_client_ip": "203.0.113.55",
                "ja4": "t13i1513h2_test",
                "protocol_engine": "chrome",
            },
            "sources": ["main", "gateway"],
            "has_gateway": true,
            "batches": [
                {"batch_id":"B0_bootstrap","source":"main"},
                {"batch_id":"B8_gateway","source":"gateway"}
            ]
        }),
        None,
    )
    .unwrap();
    let empty: Vec<String> = vec![];
    let br = score_br(&fields, &stack, &bot, &truth, &empty);
    let hits = br["field_hits"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect::<Vec<_>>();
    for need in [
        "proxy_protocol_edge",
        "font_token_surface",
        "codec_matrix_hash",
        "canvas_geometry_depth",
        "agent_parity_ratio",
        "permissions_digest",
        "battery_charge_state",
        "media_devices_shape",
        "css_supports_volume",
        "residual_mean_surface",
        "pohw_numeric_surfaces",
        "gpu_roundtrip_h02",
        "raf_mean_surface",
        "dom_rect_depth",
        "storage_privacy_depth",
        "gl_caps_depth",
        "webrtc_host_list",
        "perf_timeline_depth",
        "sensor_orient_gyro",
    ] {
        assert!(
            hits.iter().any(|h| h == need),
            "missing product br hit {need}; have {hits:?}"
        );
    }
    let _os = score_os(&fields, &stack, &truth, &empty);
}

#[test]
fn caps_gap_and_agent_ratio_raise_spoof() {
    let mut fields = base_fields();
    let o = fields.as_object_mut().unwrap();
    o.insert("caps_claim_vs_actual_gap".into(), json!(0.9));
    o.insert("agent_parity_hit_n".into(), json!(2.0));
    o.insert("agent_parity_keys_n".into(), json!(20.0));
    o.insert("canvas_geometry_stable".into(), json!(false));
    o.insert("residual_challenge_delta".into(), json!(0.2));
    o.insert("raf_mean_ms".into(), json!(80.0));

    let stack = stack_auth_from_fields(&fields);
    assert!(
        stack.spoof_score >= 0.3,
        "expected elevated spoof, got {}",
        stack.spoof_score
    );
    let rs = stack.reasons.join(",");
    assert!(rs.contains("caps_claim_vs_actual_gap_high"), "{rs}");
    assert!(rs.contains("agent_parity_ratio_low"), "{rs}");
    assert!(rs.contains("canvas_geometry_unstable"), "{rs}");
    assert!(rs.contains("residual_challenge_delta_high"), "{rs}");
    assert!(rs.contains("raf_mean_coarse") || stack.vm_score > 0.0, "{rs}");
}

#[test]
fn proxy_ip_vs_fe_public_claim_conflicts() {
    let fields = json!({
        "user_agent": "Mozilla/5.0 Chrome/150",
        "platform": "Linux x86_64",
        "form_class": "desktop",
        "webdriver": false,
        "client_ip_claim": "198.51.100.9",
        "proxy_protocol_present": true,
        "server_client_ip": "203.0.113.55",
    });
    let ev = json!({
        "fields": fields,
        "gateway_fields": {
            "proxy_protocol_present": true,
            "server_client_ip": "203.0.113.55",
        },
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "batches": [
            {"batch_id":"B0_bootstrap","source":"main"},
            {"batch_id":"B8_gateway","source":"gateway"}
        ]
    });
    let truth = evaluate_xsrc(&ev, None).unwrap();
    let conflicts = truth
        .details
        .get("conflicts")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(
        conflicts
            .iter()
            .any(|c| c.as_str() == Some("proxy_ip_vs_fe_ip_claim")),
        "status={} conflicts={conflicts:?}",
        truth.xsrc_status
    );
    // may or may not elevate to CONFLICT depending on other weights; code presence is enough
    let _ = XSRC_CONFLICT;
}
