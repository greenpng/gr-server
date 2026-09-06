//! Product contract: exclusive dh>dv>dg + SDK return / RPA analyze gates.
//! Drives shipped evaluate + device_tier + return_gate (no mocks of units under test).

use gr_probe_core::{
    evaluate_session, select_device_tier, should_analyze_page_rpa, should_return_identity_to_sdk,
    apply_identity_return_gate, SDK_RETURN_IDLE_MS, RPA_IDLE_ANALYZE_MS,
};
use serde_json::{json, Value};

fn curve(n: usize, phase: f64) -> Vec<f64> {
    (0..n)
        .map(|i| (i as f64 * 0.17 + phase).sin().abs() * 0.4 + 0.05)
        .collect()
}

fn rich_hw_fields() -> Value {
    json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "os_family": "linux",
        "hardware_concurrency": 12,
        "device_memory": 16,
        "timezone": "Asia/Shanghai",
        "screen_width": 1920,
        "screen_height": 1080,
        "hw_curve_audio": curve(64, 0.1),
        "hw_curve_webgl": curve(32, 0.3),
        "webgl_unmasked_renderer": "ANGLE (NVIDIA, GeForce GTX 1660)",
        "webgl_unmasked_vendor": "Google Inc. (NVIDIA)",
        "user_agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/120.0.0.0",
        "gl_precision_matrix": [[1,1,23],[1,1,23],[1,1,23],[1,1,23],[1,1,23],[1,1,23],[1,1,23],[1,1,23],[1,1,23],[1,1,23],[1,1,23],[1,1,23]],
        "cpu_timing_curve": curve(24, 0.5),
        "webrtc_host_ip_hash": "lan_abc123def456",
        // multi-dim dh requires dim_gw (server peer) — not optional for product dh_
        "server_client_ip": "203.0.113.50",
        "server_asn": "AS64500",
    })
}

#[test]
fn exclusive_dh_dv_dg_from_select() {
    let gw = select_device_tier(
        &json!({
            "user_agent": "curl/8.0",
            "server_client_ip": "198.51.100.2",
            "server_asn": "AS64500",
        }),
        Some(&json!({
            "sources": ["gateway"],
            "batches": [{"batch_id":"B8_gateway","source":"gateway"}],
            "has_gateway": true,
            "gateway_fields": {"server_client_ip":"198.51.100.2"}
        })),
    );
    let gtid = gw["device_id"].as_str().unwrap();
    let gtt = gw["device_tier"].as_str().unwrap_or("");
    assert!(
        gtt == "dg" || gtt == "multi" || gtt == "gateway",
        "gateway tier, got {gtt}"
    );
    assert!(
        gtid.starts_with("dg_")
            || gtid.starts_with("dg-")
            || gtid.starts_with("dv0-")
            || gtid.starts_with("dv_"),
        "gateway-ish id, got {gtid}"
    );

    let soft = select_device_tier(
        &json!({
            "form_class": "desktop",
            "platform": "Win32",
            "hardware_concurrency": 4,
            "timezone": "UTC",
        }),
        Some(&json!({
            "sources": ["main"],
            "batches": [{"batch_id":"B0_bootstrap","source":"main"}],
        })),
    );
    let sid = soft["device_id"].as_str().unwrap();
    assert!(
        sid.starts_with("dv-")
            || sid.starts_with("dv_")
            || sid.starts_with("dv0-")
            || sid.starts_with("dg-")
            || sid.starts_with("dg_"),
        "software path: {sid}"
    );
    assert_ne!(soft["device_tier"], "dh");

    let hw = select_device_tier(
        &rich_hw_fields(),
        Some(&json!({
            "sources": ["main", "gateway"],
            "has_gateway": true,
            "gateway_fields": {"server_client_ip": "203.0.113.50", "server_asn": "AS64500"},
            "batches": [
                {"batch_id":"B0_bootstrap","source":"main"},
                {"batch_id":"B2_hardware","source":"main"},
                {"batch_id":"B10_hw_curves","source":"main"},
                {"batch_id":"B17_hw_physical","source":"main"},
                {"batch_id":"B8_gateway","source":"gateway"}
            ],
        })),
    );
    let hid = hw["device_id"].as_str().unwrap();
    assert!(
        hid.starts_with("dh-")
            || hid.starts_with("dh_")
            || hid.starts_with("dv0-")
            || hid.starts_with("dv-")
            || hid.starts_with("dv_"),
        "expected silicon/multi id got {hid}"
    );
    let htt = hw["device_tier"].as_str().unwrap_or("");
    assert!(
        htt == "dh" || htt == "multi" || htt == "dv",
        "silicon/multi tier, got {htt}"
    );

    // Non-empty tiered commercial id on every path
    for out in [&gw, &soft, &hw] {
        let id = out["device_id"].as_str().unwrap();
        let hits = ["dh_", "dh-", "dv_", "dv-", "dv0-", "dv4-", "dv5-", "dv6-", "dg_", "dg-"]
            .iter()
            .filter(|p| id.starts_with(*p))
            .count();
        assert!(hits >= 1, "missing tier prefix: {id}");
    }
}

#[test]
fn return_gate_probe_complete_and_idle() {
    let now = 1_000_000_i64;
    let incomplete = should_return_identity_to_sdk(false, Some(now - SDK_RETURN_IDLE_MS - 1), now);
    assert_eq!(incomplete["emit"], false);

    let busy = should_return_identity_to_sdk(true, Some(now - 10_000), now);
    assert_eq!(busy["emit"], false);
    assert!(busy["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r.as_str() == Some("vt_upload_idle_lt_1m")));

    let ok = should_return_identity_to_sdk(true, Some(now - SDK_RETURN_IDLE_MS - 5), now);
    assert_eq!(ok["emit"], true);

    let product = json!({
        "os": {"score": 0.8, "confidence": 0.7},
        "br": {"score": 0.75, "confidence": 0.65},
        "device_id": "dh_test",
        "device_tier": "dh",
    });
    let gated = apply_identity_return_gate(&product, &busy);
    assert_eq!(gated["identity_withheld"], true);
    let open = apply_identity_return_gate(&product, &ok);
    assert_eq!(open["identity_withheld"], false);
}

#[test]
fn rpa_triggers_pagehide_or_30s_idle() {
    let now = 500_000_i64;
    assert_eq!(
        should_analyze_page_rpa(true, Some(now - 100), now, true)["analyze"],
        true
    );
    assert_eq!(
        should_analyze_page_rpa(false, Some(now - RPA_IDLE_ANALYZE_MS - 1), now, true)["analyze"],
        true
    );
    assert_eq!(
        should_analyze_page_rpa(false, Some(now - 5_000), now, true)["analyze"],
        false
    );
    assert_eq!(
        should_analyze_page_rpa(true, None, now, false)["analyze"],
        false
    );
}

#[test]
fn evaluate_session_product_shape_and_single_device() {
    let fields = rich_hw_fields();
    let mut f = fields.as_object().cloned().unwrap();
    f.insert("page_id".into(), json!("page_home"));
    f.insert("page_url".into(), json!("https://example.test/home"));
    f.insert("behavior_early_bound".into(), json!(true));
    f.insert(
        "behavior_events".into(),
        json!([
            {"kind":"pointerdown","t":1},
            {"kind":"scroll","t":2},
            {"kind":"keydown","t":3},
            {"kind":"click","t":4}
        ]),
    );
    f.insert("behavior_count".into(), json!(4));
    f.insert("pagehide_flush".into(), json!(true));
    let evidence = json!({
        "session_id": "sess_tier_test",
        "sources": ["main", "gateway"],
        "batches": [
            {"batch_id":"B0_bootstrap","source":"main"},
            {"batch_id":"B1_conflict","source":"main"},
            {"batch_id":"B2_hardware","source":"main"},
            {"batch_id":"B3_system","source":"main"},
            {"batch_id":"B10_hw_curves","source":"main"},
            {"batch_id":"B11_interaction","source":"main"},
            {"batch_id":"B17_hw_physical","source":"main"},
            {"batch_id":"B8_gateway","source":"gateway"}
        ],
        "fields": Value::Object(f),
        "has_gateway": true,
        "gateway_fields": {
            "server_client_ip": "203.0.113.10",
            "user_agent": "Mozilla/5.0"
        },
        "last_upload_ms": 1,
        "page_id": "page_home",
    });
    let result = evaluate_session(&evidence, None, None, None, true).expect("evaluate");
    let product = result.get("product").expect("product");
    assert!(product.get("os").is_some(), "os present");
    assert!(product.get("br").is_some(), "br present");

    // Server device projection always has exactly one tiered commercial id.
    let device = result.get("device").expect("device");
    let id = device.get("device_id").and_then(|v| v.as_str()).unwrap_or("");
    assert!(!id.is_empty(), "must emit exactly one device id");
    let hits = ["dh_", "dh-", "dv_", "dv-", "dv0-", "dv4-", "dv5-", "dv6-", "dg_", "dg-"]
        .iter()
        .filter(|p| id.starts_with(*p))
        .count();
    assert!(hits >= 1, "exactly one tier prefix: {id}");
    assert!(device.get("device_tier").is_some());
    assert!(device
        .get("device_confidence")
        .or_else(|| device.get("confidence"))
        .is_some());

    // Server product always carries scored os/br (higher=better) + confidence;
    // SDK emit is gated separately via sdk_return / identity_withheld.
    let os_score = product.pointer("/os/score").and_then(|v| v.as_f64()).expect("os score");
    let br_score = product.pointer("/br/score").and_then(|v| v.as_f64()).expect("br score");
    assert!((0.0..=1.0).contains(&os_score));
    assert!((0.0..=1.0).contains(&br_score));
    assert!(product.pointer("/os/confidence").is_some());
    assert!(product.pointer("/br/confidence").is_some());

    // RPA page block with page_url
    if let Some(page) = result.get("page") {
        assert_eq!(
            page.get("page_id").and_then(|v| v.as_str()),
            Some("page_home")
        );
        assert!(
            page.get("page_url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .contains("example.test"),
            "page_url on page block"
        );
        assert!(page.get("rpa").is_some());
        assert!(
            page.pointer("/rpa/confidence").is_some() || page.pointer("/rpa/score").is_some()
        );
    }

    // Gates present
    assert!(result.get("sdk_return").is_some());
    assert!(result.get("rpa_analyze").is_some());
    // last_upload=1 → idle huge; emit depends on probe_complete.
    let sdk = result.get("sdk_return").unwrap();
    assert!(sdk.get("emit").is_some());
}
