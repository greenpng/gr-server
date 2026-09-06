//! Probe expand + multi-source soft consistency + confidence on all product legs.
//! Drives shipped evaluate_session on real product path.

use gr_probe_core::evaluate_session;
use serde_json::json;

fn curve(n: usize, phase: f64) -> Vec<f64> {
    (0..n)
        .map(|i| (i as f64 * 0.17 + phase).sin().abs() * 0.4 + 0.05)
        .collect()
}

fn base_fields() -> serde_json::Map<String, serde_json::Value> {
    let v = json!({
        // OPUS5 closure: pin paid entitlement — the RPA leg needs a score.
        "rpa_analysis_enabled": true,
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "os_family": "linux",
        "hardware_concurrency": 8,
        "device_memory": 8,
        "screen_width": 1920,
        "screen_height": 1080,
        "timezone": "UTC",
        "timezone_offset_min": 0,
        "user_agent": "Mozilla/5.0 (X11; Linux x86_64) Chrome/120.0.0.0",
        "language": "en-US",
        "languages": ["en-US"],
        "vendor": "Google Inc.",
        "webdriver": false,
        "automation": {"webdriver": false, "playwright": false, "selenium": false, "cdc": false},
        "webgl_unmasked_renderer": "ANGLE (NVIDIA, GeForce GTX 1660)",
        "webgl_unmasked_vendor": "Google Inc. (NVIDIA)",
        "webgl2_support": true,
        "webgl_extensions_hash": "ext_abc123",
        "font_count": 12,
        "math_digest": "m_deadbeef",
        "canvas_hash": "c_abc",
        "residual_mean": 0.5001811906403189,
        "stack_class": "real_silicon",
        "hw_curve_audio": curve(64, 0.1),
        "hw_curve_webgl": curve(32, 0.3),
        "gl_precision_matrix": [[1,1,23],[1,1,23],[1,1,23],[1,1,23],[1,1,23],[1,1,23],[1,1,23],[1,1,23],[1,1,23],[1,1,23],[1,1,23],[1,1,23]],
        "cpu_timing_curve": curve(24, 0.5),
        "behavior_early_bound": true,
        "behavior_events": [
            {"kind":"pointerdown","t":1},
            {"kind":"scroll","t":2},
            {"kind":"keydown","t":3},
            {"kind":"click","t":4}
        ],
        "behavior_count": 4,
        "page_id": "p1",
        "page_url": "https://demo.example/app",
        "pagehide_flush": true,
    });
    v.as_object().unwrap().clone()
}

fn evidence_with(fields: serde_json::Value, conflicts: Vec<&str>) -> serde_json::Value {
    json!({
        "session_id": "sess_expand",
        "sources": ["main", "iframe", "worker", "gateway"],
        "batches": [
            {"batch_id":"B0_bootstrap","source":"main"},
            {"batch_id":"B1_conflict","source":"main"},
            {"batch_id":"B2_hardware","source":"main"},
            {"batch_id":"B3_system","source":"main"},
            {"batch_id":"B10_hw_curves","source":"main"},
            {"batch_id":"B11_interaction","source":"main"},
            {"batch_id":"B7_sandbox","source":"main"},
            {"batch_id":"B17_hw_physical","source":"main"},
            {"batch_id":"B8_gateway","source":"gateway"}
        ],
        "fields": fields,
        "source_conflicts": conflicts,
        "has_gateway": true,
        "gateway_fields": {"server_client_ip":"203.0.113.9","user_agent":"Mozilla/5.0"},
        "page_id": "p1",
        "last_upload_ms": 1,
    })
}

#[test]
fn product_legs_have_confidence_and_exclusive_device() {
    let mut fo = base_fields();
    fo.insert("sandbox_ok".into(), json!(true));
    fo.insert("sandbox_blocked".into(), json!(false));
    fo.insert("multi_source_match_ratio".into(), json!(0.95));
    fo.insert("sandbox_sources_received".into(), json!(["iframe:d1", "worker:d1"]));
    let out = evaluate_session(
        &evidence_with(serde_json::Value::Object(fo), vec![]),
        None,
        None,
        None,
        true,
    )
    .unwrap();
    let product = out.get("product").unwrap();
    for leg in ["os", "br", "rpa"] {
        let conf = product
            .pointer(&format!("/{leg}/confidence"))
            .and_then(|v| v.as_f64());
        assert!(conf.is_some(), "{leg} confidence missing");
        assert!(
            conf.unwrap() > 0.0 && conf.unwrap() <= 1.0,
            "{leg} conf={conf:?}"
        );
        assert!(product.pointer(&format!("/{leg}/score")).and_then(|v| v.as_f64()).is_some());
    }
    let dev = out.get("device").unwrap();
    let id = dev.get("device_id").and_then(|v| v.as_str()).unwrap_or("");
    let hits = ["dh_", "dh-", "dv_", "dv-", "dv0-", "dv4-", "dv5-", "dv6-", "dg_", "dg-"]
        .iter()
        .filter(|p| id.starts_with(*p))
        .count();
    assert!(hits >= 1, "exclusive device: {id}");
    assert!(dev
        .get("device_confidence")
        .or_else(|| dev.get("confidence"))
        .and_then(|v| v.as_f64())
        .is_some());
    // scores higher-is-better quality with sandbox ok
    let br = product.pointer("/br/score").and_then(|v| v.as_f64()).unwrap();
    assert!(br >= 0.45, "healthy br expected, got {br}");
}

#[test]
fn sandbox_fully_blocked_demotes_br() {
    let mut ok_f = base_fields();
    ok_f.insert("sandbox_ok".into(), json!(true));
    ok_f.insert("sandbox_blocked".into(), json!(false));
    ok_f.insert("js_ok_sandbox_dead".into(), json!(false));
    ok_f.insert("multi_source_match_ratio".into(), json!(0.95));
    ok_f.insert("sandbox_sources_received".into(), json!(["iframe:d1"]));

    let mut dead_f = base_fields();
    dead_f.insert("sandbox_ok".into(), json!(false));
    dead_f.insert("sandbox_blocked".into(), json!(true));
    dead_f.insert("js_ok_sandbox_dead".into(), json!(true));
    dead_f.insert("multi_source_match_ratio".into(), json!(1.0));
    dead_f.insert("sandbox_sources_received".into(), json!([]));

    let ok = evaluate_session(
        &evidence_with(serde_json::Value::Object(ok_f), vec![]),
        None,
        None,
        None,
        true,
    )
    .unwrap();
    let dead = evaluate_session(
        &evidence_with(serde_json::Value::Object(dead_f), vec![]),
        None,
        None,
        None,
        true,
    )
    .unwrap();
    let br_ok = ok.pointer("/product/br/score").and_then(|v| v.as_f64()).unwrap();
    let br_dead = dead.pointer("/product/br/score").and_then(|v| v.as_f64()).unwrap();
    assert!(
        br_dead < br_ok - 0.05,
        "sandbox dead must demote br: ok={br_ok} dead={br_dead}"
    );
    let reasons = dead
        .pointer("/product/br/reasons")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(
        reasons
            .iter()
            .any(|r| r.as_str().unwrap_or("").contains("sandbox")),
        "br reasons must cite sandbox: {reasons:?}"
    );
}

#[test]
fn multi_source_rpa_variance_still_not_collapsing_os_br() {
    let mut fo = base_fields();
    fo.insert("sandbox_ok".into(), json!(true));
    fo.insert("multi_source_match_ratio".into(), json!(0.9));
    fo.insert("rpa_source".into(), json!("main"));
    fo.insert("sandbox_kind".into(), json!("main"));
    // Empty identity conflicts (soft compare already filtered RPA keys)
    let out = evaluate_session(
        &evidence_with(serde_json::Value::Object(fo), vec![]),
        None,
        None,
        None,
        true,
    )
    .unwrap();
    let os = out.pointer("/product/os/score").and_then(|v| v.as_f64()).unwrap();
    let br = out.pointer("/product/br/score").and_then(|v| v.as_f64()).unwrap();
    assert!(os >= 0.45, "os={os}");
    assert!(br >= 0.45, "br={br}");
    let os_r = out
        .pointer("/product/os/reasons")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(
        !os_r
            .iter()
            .any(|r| r.as_str().unwrap_or("").contains("source_conflict:behavior")),
        "{os_r:?}"
    );
}
