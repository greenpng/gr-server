//! Lab goals: same-machine multi-browser device_id consistency; abnormal polarity.
//! Field-driven only — no host/env gold tables.

use gr_probe_core::evaluate::evaluate_session;
use serde_json::{json, Value};

fn curves() -> Value {
    json!([0.12, 0.18, 0.25, 0.31, 0.28, 0.22, 0.19, 0.27, 0.33, 0.29, 0.21, 0.17, 0.24, 0.3, 0.26, 0.2])
}

fn machine(ua: &str, platform: &str, extra: Value) -> Value {
    let mut fields = json!({
        "form_class": "desktop",
        "os_family": "linux",
        "platform": platform,
        "user_agent": ua,
        "hardware_concurrency": 8,
        "device_memory": 16,
        "timezone": "Asia/Shanghai",
        "screen_width": 1920,
        "screen_height": 1080,
        "webgl_unmasked_renderer": "ANGLE (Intel, Intel(R) UHD Graphics 620 Direct3D11 vs_5_0 ps_5_0)",
        "webgl_unmasked_vendor": "Google Inc. (Intel)",
        "hw_curve_audio": curves(),
        "hw_curve_webgl": curves(),
        "vm_score": 0.05,
        "stack_class": "native",
        "spoof_score": 0.05,
        "residual_mean": 0.42,
        "residual_soft_like": false,
        "software_renderer_heuristic": false,
        "webdriver": false,
        "automation": {"webdriver": false},
        // OPUS5 closure: pin paid entitlement so the RPA axis scores.
        "rpa_analysis_enabled": true,
        "chrome_runtime": true,
        "plugins_length": 5,
        "outer_zero": false,
        "behavior_early_bound": true,
        "behavior_count": 20,
        "behavior_events": [
            {"t":1,"type":"pointermove","kind":"pointermove"},
            {"t":2,"type":"click","kind":"click"},
            {"t":3,"type":"scroll","kind":"scroll"},
            {"t":4,"type":"keydown","kind":"keydown"}
        ],
        "pagehide_flush": true
    });
    if let (Some(fo), Some(ex)) = (fields.as_object_mut(), extra.as_object()) {
        for (k, v) in ex {
            fo.insert(k.clone(), v.clone());
        }
    }
    json!({
        "session_id": "lab_goal",
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "batches": [
            {"batch_id":"B0_bootstrap","source":"main"},
            {"batch_id":"B1_conflict","source":"main"},
            {"batch_id":"B2_hardware","source":"main"},
            {"batch_id":"B10_hw_curves","source":"main"},
            {"batch_id":"B11_interaction","source":"main"},
            {"batch_id":"B8_gateway","source":"gateway"}
        ],
        "fields": fields,
        "gateway_fields": {"user_agent": ua}
    })
}

#[test]
fn same_machine_chrome_firefox_same_device_id() {
    let chrome = machine(
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/120.0.0.0 Safari/537.36",
        "Linux x86_64",
        json!({}),
    );
    let firefox = machine(
        "Mozilla/5.0 (X11; Linux x86_64; rv:121.0) Gecko/20100101 Firefox/121.0",
        "Linux x86_64",
        json!({"chrome_runtime": false}),
    );
    let c = evaluate_session(&chrome, None, None, None, true).unwrap();
    let f = evaluate_session(&firefox, None, None, None, true).unwrap();
    let cd = c["product"]["device_id"].as_str().expect("chrome device_id");
    let fd = f["product"]["device_id"].as_str().expect("firefox device_id");
    // Commercial id may be dh_ (full multi-dim) or dv_ (partial) — same residual materials
    // must still share one machine-stable id across browsers (not browser-surface split).
    assert!(
        cd.starts_with("dh-")
            || cd.starts_with("dh_")
            || cd.starts_with("dv-")
            || cd.starts_with("dv_")
            || cd.starts_with("dv0-")
            || cd.starts_with("dv4-")
            || cd.starts_with("dv5-")
            || cd.starts_with("dv6-")
            || cd.starts_with("dg_")
            || cd.starts_with("dg-"),
        "expected commercial/multi prefix, got {cd}"
    );
    assert_eq!(
        cd, fd,
        "same machine materials must yield same commercial device_id across browsers"
    );
}

#[test]
fn automation_and_crawler_lower_br_than_human_same_machine() {
    let human = evaluate_session(
        &machine(
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/120.0.0.0 Safari/537.36",
            "Linux x86_64",
            json!({}),
        ),
        None,
        None,
        None,
        true,
    )
    .unwrap();
    let auto = evaluate_session(
        &machine(
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 HeadlessChrome/120 Safari/537.36",
            "Linux x86_64",
            json!({
                "webdriver": true,
                "automation": {"webdriver": true, "playwright": true},
                "outer_zero": true,
                "plugins_length": 0,
                "behavior_count": 2,
                "behavior_events": [{"t":1,"type":"click"},{"t":2,"type":"click"}]
            }),
        ),
        None,
        None,
        None,
        true,
    )
    .unwrap();
    let crawl = evaluate_session(
        &machine(
            "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)",
            "Linux x86_64",
            json!({
                "behavior_early_bound": false,
                "behavior_count": 0,
                "behavior_events": [],
                "plugins_length": 0,
                "chrome_runtime": false
            }),
        ),
        None,
        None,
        None,
        true,
    )
    .unwrap();
    let hb = human["product"]["br"]["score"].as_f64().unwrap();
    let ab = auto["product"]["br"]["score"].as_f64().unwrap();
    let cb = crawl["product"]["br"]["score"].as_f64().unwrap();
    assert!(ab < hb, "automation br {ab} should be < human {hb}");
    assert!(cb < hb, "crawler br {cb} should be < human {hb}");
    assert!(
        auto["product"]["rpa"]["score"].as_f64().unwrap()
            < human["product"]["rpa"]["score"].as_f64().unwrap()
    );
}

#[test]
fn real_band_demoted_when_product_marks_abnormal() {
    // VM-heavy + multi-source present would historically allow confirmed_real via xsrc alone.
    let mut ev = machine(
        "Mozilla/5.0 Chrome/120",
        "Linux x86_64",
        json!({
            "vm_score": 0.92,
            "stack_class": "vm",
            "residual_soft_like": true,
            "software_renderer_heuristic": true
        }),
    );
    // Ensure gateway/main present for xsrc path
    if let Some(obj) = ev.as_object_mut() {
        obj.insert("has_gateway".into(), json!(true));
        obj.insert(
            "gateway_fields".into(),
            json!({"user_agent": "Mozilla/5.0 Chrome/120"}),
        );
    }
    let out = evaluate_session(&ev, None, None, None, true).unwrap();
    assert!(
        out["product"]["os"]["score"].as_f64().unwrap() < 0.45,
        "os should be abnormal"
    );
    let band = out["real_band"].as_str().unwrap_or("");
    assert_ne!(
        band, "confirmed_real",
        "real_band must not stay confirmed_real when os product is abnormal: {band}"
    );
    assert!(
        matches!(band, "watch" | "likely_bot" | "likely_real" | "insufficient" | "crawler"),
        "unexpected band {band}"
    );
}

#[test]
fn real_band_demoted_on_abnormal_product_evidence() {
    // Selenium-like chrome without runtime + empty plugins + mild vm → not confirmed_real.
    // (Pure coverage_cap suspect without abnormal markers may still keep multi-source band.)
    let mut ev = machine(
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        "Linux x86_64",
        json!({
            "webdriver": false,
            "plugins_length": 0,
            "chrome_runtime": false,
            "languages": ["en-US", "en"],
            "language": "en-US",
            "cookie_enabled": true,
            "screen_width": 1920,
            "screen_height": 1080,
            "form_class": "desktop",
            "outer_zero": false,
            "vm_score": 0.72,
            "stack_class": "vm"
        }),
    );
    if let Some(obj) = ev.as_object_mut() {
        obj.insert("has_gateway".into(), json!(true));
        obj.insert(
            "gateway_fields".into(),
            json!({
                "user_agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"
            }),
        );
    }
    let out = evaluate_session(&ev, None, None, None, true).unwrap();
    let band = out["real_band"].as_str().unwrap_or("");
    assert_ne!(
        band, "confirmed_real",
        "abnormal os/br evidence must demote real_band, got {band}; product={}",
        out["product"]
    );
}

#[test]
fn vm_and_source_conflict_cap_safety_not_real() {
    let vm = evaluate_session(
        &machine(
            "Mozilla/5.0 Chrome/120",
            "Linux x86_64",
            json!({
                "vm_score": 0.92,
                "stack_class": "vm",
                "residual_soft_like": true,
                "software_renderer_heuristic": true
            }),
        ),
        None,
        None,
        None,
        true,
    )
    .unwrap();
    assert!(vm["product"]["os"]["score"].as_f64().unwrap() < 0.5);
    assert_ne!(vm["product"]["os"]["status"].as_str(), Some("real"));

    let mut conflict = machine(
        "Mozilla/5.0 Chrome/120",
        "Linux x86_64",
        json!({}),
    );
    conflict.as_object_mut().unwrap().insert(
        "source_conflicts".into(),
        json!(["source_conflict:user_agent"]),
    );
    conflict.as_object_mut().unwrap().insert(
        "gateway_fields".into(),
        json!({"user_agent": "GatewayBot/1.0"}),
    );
    let c = evaluate_session(&conflict, None, None, None, true).unwrap();
    let br = c["product"]["br"]["score"].as_f64().unwrap();
    let st = c["product"]["br"]["status"].as_str().unwrap();
    assert!(br < 0.72, "conflict must not allow high br: {br}");
    assert_ne!(st, "real", "conflict must not be real status");
    assert!(
        c["product"]["br"]["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r.as_str().unwrap_or("").contains("conflict")
                || r.as_str().unwrap_or("").contains("source_conflict")),
        "reasons={:?}",
        c["product"]["br"]["reasons"]
    );
}
