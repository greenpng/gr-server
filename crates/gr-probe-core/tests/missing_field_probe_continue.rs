//! Missing fields demote related axes; never prefer_stop probe pipeline.
use gr_probe_core::bot::BotScore;
use gr_probe_core::product_scores::{score_br, score_os, score_rpa};
use gr_probe_core::stack_auth::stack_auth_from_fields;
use gr_probe_core::xsrc::TruthResult;
use gr_probe_core::policy_band_from_scores;
use serde_json::json;

fn truth_ok() -> TruthResult {
    TruthResult {
        xsrc_status: "consistent".into(),
        real_band: "watch".into(),
        credibility: 0.5,
        fe_only: false,
        has_server_side: true,
        has_main_core: true,
        packages: json!([]),
        reasons: vec![],
        details: json!({}),
    }
}

fn bot_human() -> BotScore {
    BotScore {
        score: 10,
        verdict: "human".into(),
        flags: vec![],
        family: "none".into(),
        algo: "balanced".into(),
        details: json!({}),
        robot_name: None,
    }
}

#[test]
fn policy_critical_does_not_prefer_stop() {
    let band = policy_band_from_scores(Some(0.15), Some(0.15), Some(0.1));
    assert_eq!(band["band"], "critical");
    assert_eq!(band["prefer_stop"], false);
    assert_eq!(band["probe_policy"], "verify_deepen");
}

#[test]
fn webrtc_no_rtc_demotes_os_not_as_bot() {
    let fields = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "os_family": "linux",
        "hardware_concurrency": 8,
        "timezone": "UTC",
        "hw_curve_webgl": [0.2,0.2,0.2,0.2,0.2,0.2,0.2,0.2,0.2,0.2,0.2,0.2,0.2,0.2,0.2,0.2,0.2,0.2,0.2,0.2,0.2,0.2,0.2,0.2,0.2,0.2,0.2,0.2,0.2,0.2,0.2,0.2],
        "webrtc_probe_failed": "no_rtc",
        "webrtc_host_count": 0,
        "webrtc_missing": true,
        "engine_family": "webkit",
    });
    let stack = stack_auth_from_fields(&fields);
    let os = score_os(&fields, &stack, &truth_ok(), &[]);
    let reasons: Vec<String> = os["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    assert!(
        reasons.iter().any(|r| r.contains("webrtc_platform_unavailable")),
        "expected platform unavailable reason: {reasons:?}"
    );
    assert!(os["score"].as_f64().unwrap_or(0.0) > 0.2);
}

#[test]
fn antidetect_demotes_rpa_and_br() {
    let fields = json!({
        "form_class": "desktop",
        "user_agent": "Mozilla/5.0 Chrome/120.0.0.0",
        "webdriver": false,
        "behavior_early_bound": true,
        "behavior_count": 12,
        "behavior_events": [
            {"kind":"mousemove"},{"kind":"click"},{"kind":"keydown"},{"kind":"scroll"}
        ],
        "behavior_type_diversity_n": 4,
        "antidetect_vendor_hint": true,
        "fingerprint_vendor_lie": true,
        "spoof_score": 0.5,
    });
    let stack = stack_auth_from_fields(&fields);
    let bot = bot_human();
    let br = score_br(&fields, &stack, &bot, &truth_ok(), &[]);
    let rpa = score_rpa(&fields, &bot, Some("p1"));
    let br_rs: Vec<_> = br["reasons"].as_array().unwrap().iter().filter_map(|v| v.as_str()).collect();
    let rpa_rs: Vec<_> = rpa["reasons"].as_array().unwrap().iter().filter_map(|v| v.as_str()).collect();
    assert!(br_rs.iter().any(|r| r.contains("antidetect")), "br: {br_rs:?}");
    assert!(rpa_rs.iter().any(|r| r.contains("antidetect_rpa_env")), "rpa: {rpa_rs:?}");
    assert!(rpa["score"].as_f64().unwrap() < 0.7);
}
