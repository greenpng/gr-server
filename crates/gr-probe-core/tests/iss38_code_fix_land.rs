//! iss/38 P0–P2 landing tests.
//!
//! Fast loop (do NOT stack many --test binaries):
//!   cargo check -p gr-core
//!   cargo test -p gr-core --test iss38_code_fix_land -- --test-threads=1
//!   cargo test -p gr-core --test iss38_code_fix_land p0_ip -- --exact

use gr_probe_core::{
    activate_hypotheses, aggregate_unknown_buckets, classify_platform_honesty,
    commercial_projection, elevated_packs_for_fields, evaluate_rule_samples_shadow,
    hypothesis_coverage_json, score_os, self_capability_json, stack_auth_from_fields, H_AD, H_CDP,
    H_EMU, H_VM, TruthResult,
};
use serde_json::{json, Value};

fn truth() -> TruthResult {
    TruthResult {
        xsrc_status: "ok".into(),
        real_band: "likely_real".into(),
        credibility: 0.5,
        fe_only: true,
        has_server_side: true,
        has_main_core: true,
        packages: json!({}),
        reasons: vec![],
        details: json!({}),
    }
}

fn reasons(block: &Value) -> Vec<String> {
    block
        .get("reasons")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(|s| s.to_string()))
        .collect()
}

fn field_hits(block: &Value) -> Vec<String> {
    block
        .get("field_hits")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(|s| s.to_string()))
        .collect()
}

#[test]
fn p0_ip_asn_reference_aux_on_score_os() {
    let fields = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "hardware_concurrency": 8,
        "server_client_ip": "203.0.113.9",
        "server_asn": "AS64500",
        "user_agent": "Mozilla/5.0 (X11; Linux x86_64) Chrome/120.0.0.0",
    });
    let stack = stack_auth_from_fields(&fields);
    let os = score_os(&fields, &stack, &truth(), &[]);
    let rs = reasons(&os);
    let hs = field_hits(&os);
    assert!(
        rs.iter().any(|r| r.contains("net_egress_aux")),
        "expected net_egress_aux reasons, got {rs:?}"
    );
    assert!(
        hs.iter().any(|h| h == "server_asn" || h == "server_client_ip"),
        "expected asn/ip field_hits, got {hs:?}"
    );

    let soft = json!({
        "form_class": "mobile",
        "mobile_ua_claim": true,
        "residual_soft_like": true,
        "soft_stack": true,
        "server_asn": "AS1",
        "user_agent": "Mozilla/5.0 (Linux; Android 12) Mobile",
        "hardware_concurrency": 8,
    });
    let rs2 = reasons(&score_os(&soft, &stack_auth_from_fields(&soft), &truth(), &[]));
    assert!(
        rs2.iter().any(|r| r.contains("net_egress_aux_soft_mobile")),
        "soft mobile egress aux missing: {rs2:?}"
    );
}

#[test]
fn p0_ua_form_aux_from_user_agent_string() {
    let fields = json!({
        "form_class": "desktop",
        "user_agent": "Mozilla/5.0 (Linux; Android 13; Pixel) AppleWebKit/537.36 Mobile Safari/537.36",
        "hardware_concurrency": 8,
        "platform": "Linux x86_64",
    });
    let rs = reasons(&score_os(
        &fields,
        &stack_auth_from_fields(&fields),
        &truth(),
        &[],
    ));
    assert!(
        rs.iter()
            .any(|r| r.contains("mobile_ua_claim_vs_desktop_form") || r.contains("ua_form_aux")),
        "UA/form aux missing: {rs:?}"
    );
    assert!(!commercial_projection(&fields).to_string().contains("Android 13"));
}

#[test]
fn p1_emulator_form_cross_lie() {
    let fields = json!({
        "form_class": "mobile",
        "mobile_ua_claim": true,
        "residual_soft_like": true,
        "emulator_hint": "android_emu",
        "sensor_accel_present": false,
        "sensor_gyro_present": false,
        "max_touch_points": 0,
        "hardware_concurrency": 8,
        "user_agent": "Mozilla/5.0 (Linux; Android 12; sdk_gphone) Mobile",
    });
    let rs = reasons(&score_os(
        &fields,
        &stack_auth_from_fields(&fields),
        &truth(),
        &[],
    ));
    assert!(
        rs.iter().any(|r| r.contains("emulator_form_cross_lie")),
        "joint emulator cross missing: {rs:?}"
    );
}

#[test]
fn p1_brain_hypotheses_elevate_packs() {
    let fields = json!({
        "residual_soft_like": true,
        "antidetect_vendor_hint": true,
        "webdriver": true,
        "emulator_hint": "android_emu",
    });
    let hyps = activate_hypotheses(&fields, &[]);
    assert!(hyps.iter().any(|h| h == H_VM), "{hyps:?}");
    assert!(hyps.iter().any(|h| h == H_AD), "{hyps:?}");
    assert!(hyps.iter().any(|h| h == H_CDP || h == H_EMU), "{hyps:?}");
    let (_h, packs) = elevated_packs_for_fields(&fields, &[]);
    assert!(packs.iter().any(|p| p == "B12_anti_camouflage"), "{packs:?}");
    assert_eq!(hypothesis_coverage_json(&fields, &[])["algo"], "brain_hypotheses_v1");
}

#[test]
fn p2_platform_honesty_webview_never_refuse() {
    let h = classify_platform_honesty(&json!({
        "user_agent": "Mozilla/5.0 (Linux; Android 10; wv) AppleWebKit/537.36 Version/4.0 Chrome/91.0.4472.120 Mobile Safari/537.36",
        "form_class": "mobile",
        "hardware_concurrency": 2,
    }));
    assert_eq!(h["kind"], "webview");
    assert_eq!(h["refuse_probe"], false);
}

#[test]
fn p2_worker_throughput_weak_os() {
    let fields = json!({
        "form_class": "desktop",
        "hardware_concurrency": 8,
        "worker_throughput_vs_cores": 0.1,
        "user_agent": "Mozilla/5.0 Chrome/120",
    });
    let rs = reasons(&score_os(
        &fields,
        &stack_auth_from_fields(&fields),
        &truth(),
        &[],
    ));
    assert!(
        rs.iter().any(|r| r.contains("worker_throughput_vs_cores_low")),
        "{rs:?}"
    );
}

#[test]
fn p2_rule_sample_shadow_and_unknown_hub() {
    let shadow = evaluate_rule_samples_shadow(&json!({
        "antidetect_vendor_hint": true,
        "webdriver": true,
    }));
    assert_eq!(shadow["loaded"], true);
    assert!(!shadow["matched"].as_array().unwrap().is_empty());
    let hub = aggregate_unknown_buckets(&[
        json!({"session_id":"s1","unknown_bucket":{"codes":["out_of_envelope"],"present":true,"tags":["thin"]}}),
        json!({"session_id":"s2","unknown_bucket":{"code":"out_of_envelope","reason":"thin"}}),
    ]);
    assert_eq!(hub["sessions_with_bucket"], 2);
    assert_eq!(
        self_capability_json()["capabilities"]["reference_aux_ua_ip_ja"],
        true
    );
}
