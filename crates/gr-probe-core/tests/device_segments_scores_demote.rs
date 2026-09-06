//! Device multi-segment mint + os/br/rpa demotion on shipped score paths.
use gr_probe_core::bot::score_bot;
use gr_probe_core::device_segments::{select_device_segments, SEGMENT_PREFIXES};
use gr_probe_core::product_scores::{score_br, score_os, score_rpa};
use gr_probe_core::stack_auth::stack_auth_from_fields;
use gr_probe_core::xsrc::TruthResult;
use serde_json::json;

fn truth_ok() -> TruthResult {
    TruthResult {
        xsrc_status: "ok".into(),
        real_band: "watch".into(),
        credibility: 0.8,
        fe_only: false,
        has_server_side: true,
        has_main_core: true,
        packages: json!([]),
        reasons: vec![],
        details: json!({}),
    }
}

fn clean_fields() -> serde_json::Value {
    json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "os_family": "linux",
        "architecture": "x86_64",
        "hardware_concurrency": 8,
        "timezone": "UTC",
        "residual_mean": 0.26,
        "hw_curve_webgl": [0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8],
        "hw_curve_audio": [0.0,0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8,0.9,0.1,0.2,0.3,0.4,0.5,0.6],
        "webdriver": false,
        "chrome_runtime": true,
        "user_agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        "behavior_count": 6,
        "behavior_events": [
            {"t":1,"kind":"mousemove","x":10.3,"y":20.7},
            {"t":2,"kind":"mousemove","x":15.1,"y":22.4},
            {"t":3,"kind":"click","x":40.6,"y":50.2}
        ],
    })
}

#[test]
fn multi_segment_ua_ip_not_in_body() {
    let mut f = clean_fields();
    f.as_object_mut()
        .unwrap()
        .insert("server_client_ip".into(), json!("198.51.100.9"));
    let out = select_device_segments(
        &f,
        Some(&json!({
            "sources": ["main", "gateway"],
            "has_gateway": true,
            "gateway_fields": {"server_client_ip": "198.51.100.9"},
        })),
    );
    for p in SEGMENT_PREFIXES {
        let s = out["device_id_segments"][p].as_str().unwrap();
        assert!(!s.contains("198.51.100"), "IP in {s}");
        assert!(!s.contains("Mozilla"), "UA in {s}");
        assert!(s.starts_with(&format!("{p}-")));
    }
}

#[test]
fn webdriver_and_headless_demote_br_and_rpa() {
    let clean = clean_fields();
    let dirty = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "os_family": "linux",
        "architecture": "x86_64",
        "hardware_concurrency": 8,
        "timezone": "UTC",
        "residual_mean": 0.26,
        "hw_curve_webgl": [0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8],
        "webdriver": true,
        "agent_has_webdriver": true,
        "agent_has_dom_automation": true,
        "chrome_runtime": false,
        "user_agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome/120.0.0.0 Safari/537.36",
        "behavior_count": 0,
        "behavior_events": [],
    });
    let stack_c = stack_auth_from_fields(&clean);
    let stack_d = stack_auth_from_fields(&dirty);
    let bot_c = score_bot(&clean, "balanced").unwrap();
    let bot_d = score_bot(&dirty, "balanced").unwrap();
    let br_c = score_br(&clean, &stack_c, &bot_c, &truth_ok(), &[]);
    let br_d = score_br(&dirty, &stack_d, &bot_d, &truth_ok(), &[]);
    let rpa_c = score_rpa(&clean, &bot_c, Some("p1"));
    let rpa_d = score_rpa(&dirty, &bot_d, Some("p1"));
    assert!(
        br_d["score"].as_f64().unwrap() < br_c["score"].as_f64().unwrap(),
        "automation/headless must demote br: clean={} dirty={}",
        br_c["score"],
        br_d["score"]
    );
    assert!(
        rpa_d["score"].as_f64().unwrap() < rpa_c["score"].as_f64().unwrap(),
        "automation must demote rpa: clean={} dirty={}",
        rpa_c["score"],
        rpa_d["score"]
    );
}

#[test]
fn soft_stack_vm_demotes_os() {
    let clean = clean_fields();
    let soft = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "os_family": "linux",
        "architecture": "x86_64",
        "hardware_concurrency": 8,
        "timezone": "UTC",
        "residual_mean": 0.01,
        "residual_soft_like": true,
        "webgl_unmasked_renderer": "Google SwiftShader",
        "vm_score": 0.8,
        "webdriver": false,
    });
    let stack_c = stack_auth_from_fields(&clean);
    let stack_s = stack_auth_from_fields(&soft);
    let os_c = score_os(&clean, &stack_c, &truth_ok(), &[]);
    let os_s = score_os(&soft, &stack_s, &truth_ok(), &[]);
    assert!(
        os_s["score"].as_f64().unwrap() < os_c["score"].as_f64().unwrap(),
        "soft/VM must demote os: clean={} soft={}",
        os_c["score"],
        os_s["score"]
    );
}
