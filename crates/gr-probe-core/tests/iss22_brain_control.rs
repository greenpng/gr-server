//! iss/22 brain control plane P0 — belief / envelope / mission / battle_log / stop_reason.
use gr_probe_core::evaluate_session;
use serde_json::json;

fn thin_evidence() -> serde_json::Value {
    json!({
        "session_id": "iss22_ctrl",
        "sources": ["main"],
        "batches": [
            {"batch_id": "B0_bootstrap", "source": "main"},
            {"batch_id": "B1_conflict", "source": "main"}
        ],
        "fields": {
            "form_class": "desktop",
            "platform": "Linux x86_64",
            "user_agent": "Mozilla/5.0 (X11; Linux x86_64) Chrome/120.0.0.0 Safari/537.36",
            "hardware_concurrency": 8,
            "webdriver": false,
            "engine_claim": "blink",
            "engine_obs": "blink",
            "chrome_runtime": true,
        },
        "has_gateway": false,
    })
}

#[test]
fn evaluate_emits_belief_envelope_missions_battle_log() {
    let out = evaluate_session(&thin_evidence(), None, None, None, false).expect("eval");
    assert!(out.get("belief").is_some(), "belief first-class");
    assert_eq!(
        out.pointer("/belief/schema").and_then(|v| v.as_str()),
        Some("gr_belief_v1")
    );
    assert!(out.pointer("/belief/axes/os/status").is_some());
    assert!(out.pointer("/belief/axes/br/status").is_some());
    assert!(out.pointer("/belief/axes/rpa/status").is_some());
    assert!(out.pointer("/belief/axes/device").is_some());

    assert_eq!(
        out.pointer("/capability_envelope/schema")
            .and_then(|v| v.as_str()),
        Some("gr_envelope_v1")
    );
    assert!(out
        .pointer("/capability_envelope/capability_bitmap")
        .is_some());

    assert_eq!(
        out.pointer("/missions/schema").and_then(|v| v.as_str()),
        Some("gr_missions_v1")
    );
    assert!(out.pointer("/missions/primary").is_some());

    assert_eq!(
        out.pointer("/battle_log/schema").and_then(|v| v.as_str()),
        Some("gr_battle_log_v1")
    );
    assert!(out.pointer("/battle_log/packs_scheduled").is_some());
    assert!(out.pointer("/battle_log/stop_reason").is_some());

    assert!(out.get("stop_reason").and_then(|v| v.as_str()).is_some());
    assert_eq!(
        out.pointer("/policy_band/schema").and_then(|v| v.as_str()),
        Some("gr_policy_band_v1")
    );

    // route_plan enriched
    assert!(out.pointer("/route_plan/mission_id").is_some());
    assert!(out.pointer("/route_plan/stop_reason").is_some());
}

#[test]
fn form_ua_conflict_in_belief() {
    let mut ev = thin_evidence();
    if let Some(fo) = ev["fields"].as_object_mut() {
        fo.insert("mobile_ua_claim".into(), json!(true));
        fo.insert("form_class".into(), json!("desktop"));
    }
    let out = evaluate_session(&ev, None, None, None, false).unwrap();
    assert_eq!(
        out.pointer("/belief/axes/form/status").and_then(|v| v.as_str()),
        Some("conflict")
    );
}

#[test]
fn rpa_unknown_raises_rpa_mission() {
    let out = evaluate_session(&thin_evidence(), None, None, None, false).unwrap();
    // no behavior → rpa unknown → raise_rpa_clarity often primary or in ordered
    let primary = out
        .pointer("/missions/primary")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let ordered: Vec<_> = out
        .pointer("/missions/ordered")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|m| m.get("id").and_then(|v| v.as_str()))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        primary == "raise_rpa_clarity"
            || primary == "secure_device_anchor"
            || ordered.iter().any(|id| *id == "raise_rpa_clarity"),
        "primary={primary} ordered={ordered:?}"
    );
}

#[test]
fn unit_brain_control_module() {
    // re-export smoke
    let _ = gr_probe_core::build_belief;
    let _ = gr_probe_core::scan_capability_envelope;
    let _ = gr_probe_core::select_missions;
    let _ = gr_probe_core::classify_stop_reason;
    let _ = gr_probe_core::build_battle_log;
    let _ = gr_probe_core::policy_band_from_scores;
}
