//! ARCH-S4 / G-ARCH-12,14: conflict-first corroboration must cap product scores.

use gr_probe_core::corroborate::{fuse_channels, ChannelOut, Stance};
use gr_probe_core::evaluate::evaluate_session;
use gr_probe_core::product_scores::{score_br, score_os};
use gr_probe_core::stack_auth::stack_auth_from_fields;
use gr_probe_core::xsrc::{evaluate_xsrc, TruthResult, XSRC_CONFLICT};
use serde_json::json;

#[test]
fn fuse_conflict_cannot_average_to_real() {
    let chs = vec![
        ChannelOut {
            id: "ch_env",
            safety: 0.9,
            coverage: 0.8,
            stance: Stance::Support,
            reasons: vec![],
        },
        ChannelOut {
            id: "ch_xsrc",
            safety: 0.2,
            coverage: 0.8,
            stance: Stance::Contradict,
            reasons: vec!["source_conflict:user_agent".into()],
        },
    ];
    let (safety, status, reasons) = fuse_channels(&chs, 0.2);
    assert!(safety < 0.5, "conflict must cap safety, got {safety}");
    assert_ne!(status, "real");
    assert_eq!(status, "suspect");
    assert!(reasons.iter().any(|r| r.contains("conflict")));
}

#[test]
fn fuse_high_safety_without_conflict_can_be_real() {
    let chs = vec![
        ChannelOut {
            id: "ch_env",
            safety: 0.88,
            coverage: 0.85,
            stance: Stance::Support,
            reasons: vec![],
        },
        ChannelOut {
            id: "ch_xsrc",
            safety: 0.82,
            coverage: 0.75,
            stance: Stance::Support,
            reasons: vec![],
        },
    ];
    let (safety, status, _) = fuse_channels(&chs, 0.2);
    assert!(safety >= 0.72, "high safety without conflict: {safety}");
    assert_eq!(status, "real");
}

#[test]
fn source_conflicts_cap_os_br_away_from_real() {
    let fields = json!({
        "vm_score": 0.05,
        "stack_class": "native",
        "hardware_concurrency": 8,
        "device_memory": 16,
        "hw_curve_webgl": [0.1, 0.2, 0.3, 0.4],
        "form_class": "desktop",
        "user_agent": "Mozilla/5.0 Chrome/120.0.0.0",
        "platform": "Win32",
        "languages": ["en-US"],
        "webdriver": false,
        "automation": {"webdriver": false}
    });
    let stack = stack_auth_from_fields(&fields);
    let mut ev = json!({
        "fields": fields,
        "sources": ["main", "worker", "gateway"],
        "has_gateway": true,
        "source_conflicts": ["source_conflict:user_agent"],
        "fields_by_source": {
            "main": {"user_agent": "Mozilla/5.0 Chrome/120.0.0.0"},
            "worker": {"user_agent": "Mozilla/5.0 Firefox/121.0"}
        }
    });
    let truth = evaluate_xsrc(&ev, None).unwrap();
    let conflicts = vec!["source_conflict:user_agent".into()];
    let os = score_os(&fields, &stack, &truth, &conflicts);
    let br = score_br(&fields, &stack, &gr_probe_core::score_bot(&fields, "balanced").unwrap(), &truth, &conflicts);
    assert_ne!(os["status"].as_str(), Some("real"), "os must not be real under conflict");
    assert_ne!(br["status"].as_str(), Some("real"), "br must not be real under conflict");
    assert!(os["score"].as_f64().unwrap() < 0.5);
    assert!(br["score"].as_f64().unwrap() < 0.5);

    // evaluate_session end-to-end
    let out = evaluate_session(&ev, None, None, None, true).unwrap();
    assert_ne!(out["product"]["os"]["status"].as_str(), Some("real"));
    assert_ne!(out["product"]["br"]["status"].as_str(), Some("real"));

    // xsrc_status conflict alone should also cap
    ev.as_object_mut().unwrap().remove("source_conflicts");
    let truth_conflict = TruthResult {
        xsrc_status: XSRC_CONFLICT.into(),
        real_band: "watch".into(),
        credibility: 0.3,
        fe_only: false,
        has_server_side: true,
        has_main_core: true,
        packages: json!([]),
        reasons: vec!["ua_vs_gateway".into()],
        details: json!({}),
    };
    let os2 = score_os(&fields, &stack, &truth_conflict, &[]);
    assert_ne!(os2["status"].as_str(), Some("real"));
}

#[test]
fn clean_evidence_without_conflict_can_still_be_real() {
    let ev = json!({
        "session_id": "s_test",
        "fields": {
            "vm_score": 0.05,
            "stack_class": "native",
            "hardware_concurrency": 8,
            "device_memory": 16,
            "hw_curve_webgl": [0.1, 0.2, 0.3, 0.4],
            "form_class": "desktop",
            "user_agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120.0.0.0 Safari/537.36",
            "platform": "Win32",
            "languages": ["en-US"],
            "webdriver": false,
            "automation": {"webdriver": false},
            "behavior_early_bound": true,
            "behavior_count": 12,
            "behavior_events": [{"t": 1, "type": "pointer"}, {"t": 2, "type": "scroll"}]
        },
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "gateway_fields": {"user_agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120.0.0.0 Safari/537.36"},
        "source_conflicts": [],
        "batches": [
            {"batch_id": "B0_bootstrap", "source": "main"},
            {"batch_id": "B8_gateway", "source": "gateway"}
        ]
    });
    let out = evaluate_session(&ev, None, None, None, true).unwrap();
    let os_status = out["product"]["os"]["status"].as_str().unwrap_or("");
    let br_status = out["product"]["br"]["status"].as_str().unwrap_or("");
    assert!(
        os_status == "real" || os_status == "suspect",
        "clean os should be real or suspect, got {os_status}"
    );
    assert!(
        out["product"]["os"]["score"].as_f64().unwrap() > 0.45,
        "clean os score should be reasonable"
    );
    assert!(
        br_status == "real" || br_status == "suspect",
        "clean br should be real or suspect, got {br_status}"
    );
}
