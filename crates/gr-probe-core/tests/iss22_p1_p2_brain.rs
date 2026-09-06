//! iss/22 P1a–P1d + P2 control plane integration tests.
use gr_probe_core::brain::build_frontier;
use gr_probe_core::brain_directions::{
    mission_allowed_directions, rank_directions_for_missions, DIRECTIONS,
};
use gr_probe_core::brain_control::{
    apply_band_pack_budget, max_packs_for_band, policy_band_from_scores, sign_route_plan,
    unknown_bucket_from_belief, update_direction_priors,
};
use gr_probe_core::evaluate_session;
use serde_json::json;

#[test]
fn p1c_bio_input_direction_registered() {
    assert!(
        DIRECTIONS.iter().any(|d| d.id == "bio_input"),
        "bio_input direction must exist"
    );
    let bio = DIRECTIONS.iter().find(|d| d.id == "bio_input").unwrap();
    assert!(bio.yield_keys.iter().any(|k| *k == "input_mouse_entropy"));
    assert!(bio.packs.iter().any(|p| *p == "B11_interaction"));
}

#[test]
fn iss25_automation_control_direction_rpa_primary() {
    assert!(
        DIRECTIONS.iter().any(|d| d.id == "automation_control"),
        "automation_control direction must exist"
    );
    let d = DIRECTIONS
        .iter()
        .find(|x| x.id == "automation_control")
        .unwrap();
    assert!(d.axes.iter().any(|a| *a == "rpa"));
    assert!(d.packs.iter().any(|p| *p == "B12_anti_camouflage"));
    assert!(d.yield_keys.iter().any(|k| *k == "cdp_runtime_hint"));
    // webdriver evidence should prefer automation_control over pure bio absence
    let evidence = json!({
        "session_id": "auto_dir",
        "sources": ["main"],
        "batches": [{"batch_id":"B0_bootstrap","source":"main"}],
        "fields": {
            "form_class": "desktop",
            "platform": "Linux",
            "user_agent": "Mozilla/5.0 HeadlessChrome/120",
            "webdriver": true,
            "outer_zero": true,
        },
    });
    let ranked = rank_directions_for_missions(&evidence, None);
    let ids: Vec<&str> = ranked
        .iter()
        .filter_map(|r| r.get("direction_id").and_then(|v| v.as_str()))
        .collect();
    assert!(
        ids.iter().any(|id| *id == "automation_control"),
        "ranked directions must include automation_control: {ids:?}"
    );
}

#[test]
fn p1a_mission_filters_direction_rank() {
    let evidence = json!({
        "session_id": "m1",
        "sources": ["main"],
        "batches": [{"batch_id":"B0_bootstrap","source":"main"}],
        "fields": {
            "form_class": "desktop",
            "platform": "Linux",
            "user_agent": "Mozilla/5.0 Chrome/120",
            "webdriver": false,
        },
    });
    let missions = json!({
        "ordered": [{
            "id": "raise_rpa_clarity",
            "directions": ["bio_input", "browser_kernel"],
            "priority": 90
        }]
    });
    let allow = mission_allowed_directions(Some(&missions)).unwrap();
    assert!(allow.contains("bio_input"));
    assert!(allow.contains("browser_kernel"));

    let ranked = rank_directions_for_missions(&evidence, Some(&missions));
    assert!(!ranked.is_empty());
    for r in &ranked {
        let id = r.get("direction_id").and_then(|v| v.as_str()).unwrap();
        assert!(
            allow.contains(id),
            "ranked direction {id} outside mission allowlist"
        );
    }
}

#[test]
fn p1a_frontier_notes_mission_primary() {
    let evidence = json!({
        "session_id": "m2",
        "sources": ["main"],
        "batches": [
            {"batch_id":"B0_bootstrap","source":"main"},
            {"batch_id":"B1_conflict","source":"main"}
        ],
        "fields": {
            "form_class": "desktop",
            "platform": "Linux",
            "user_agent": "Mozilla/5.0 Chrome/120",
            "webdriver": false,
            "hardware_concurrency": 4,
        },
    });
    let plan = build_frontier(&evidence, false, None, 12).unwrap();
    assert!(
        plan.notes.iter().any(|n| n.starts_with("mission_primary=")),
        "notes={:?}",
        plan.notes
    );
    assert!(
        plan.notes
            .iter()
            .any(|n| n.contains("mission_filter") || n.contains("direction ")),
        "expected mission/direction notes: {:?}",
        plan.notes
    );
}

#[test]
fn p1d_band_caps_max_packs() {
    // Critical deepens verification (prefer_stop=false); cap is budget not freeze.
    assert_eq!(max_packs_for_band("critical"), 12);
    assert_eq!(max_packs_for_band("low"), 6);
    let band = policy_band_from_scores(Some(0.2), Some(0.2), Some(0.2));
    assert_eq!(band.get("band").and_then(|v| v.as_str()), Some("critical"));
    assert_eq!(
        band.get("prefer_stop").and_then(|v| v.as_bool()),
        Some(false),
        "high risk must not prefer_stop probe — demote scores only"
    );
    assert_eq!(
        band.get("probe_policy").and_then(|v| v.as_str()),
        Some("verify_deepen")
    );

    let mut packs = vec![
        json!({"pack_id":"B10_hw_curves","layer":"hard","effective_priority":10000}),
        json!({"pack_id":"B17_hw_physical","layer":"hard","effective_priority":9000}),
        json!({"pack_id":"B22_gpu_timer","layer":"hard","effective_priority":8000}),
        json!({"pack_id":"B33_caps_pressure","layer":"hard","effective_priority":7000}),
        json!({"pack_id":"B5_census","layer":"deep","effective_priority":50}),
        json!({"pack_id":"B21_census_volume","layer":"deep","effective_priority":40}),
    ];
    apply_band_pack_budget(&mut packs, &band);
    assert!(
        packs.len() <= 12,
        "critical band budget should keep verification packs, got {}",
        packs.len()
    );
    // All scheduled packs fit under critical cap of 12
    assert_eq!(packs.len(), 6);
    assert!(packs.iter().any(|p| p["pack_id"] == "B10_hw_curves"));
    assert!(packs.iter().any(|p| p["pack_id"] == "B17_hw_physical"));
}

#[test]
fn b10x_silicon_reserved_not_truncated_by_dense() {
    // Fill core to near cap, then ensure B10x_silicon_* still kept (short-visit residual).
    let band = json!({"band": "mid", "max_packs_cap": 8});
    let mut packs = vec![
        json!({"pack_id":"B0_bootstrap","layer":"hard","effective_priority":10000}),
        json!({"pack_id":"B2_hardware","layer":"hard","effective_priority":9000}),
        json!({"pack_id":"B3_system","layer":"hard","effective_priority":8900}),
        json!({"pack_id":"B10_hw_curves","layer":"hard","effective_priority":8800}),
        json!({"pack_id":"B1_conflict","layer":"hard","effective_priority":8700}),
        json!({"pack_id":"B12_anti_camouflage","layer":"hard","effective_priority":8600}),
        json!({"pack_id":"B8_gateway_early","layer":"hard","effective_priority":8500}),
        // dense deepen that used to crowd out B10x under naive cap
        json!({"pack_id":"B50_dense_a","layer":"deep","effective_priority":500}),
        json!({"pack_id":"B51_dense_b","layer":"deep","effective_priority":490}),
        json!({"pack_id":"B52_dense_c","layer":"deep","effective_priority":480}),
        json!({
            "pack_id":"B10x_silicon_ulp",
            "layer":"hard",
            "priority": 1090,
            "reason": "edh_b10x_silicon_must_land",
            "hard_eligible": true
        }),
        json!({
            "pack_id":"B10x_silicon_noderiv",
            "layer":"hard",
            "priority": 1089,
            "reason": "edh_b10x_silicon_must_land",
            "hard_eligible": true
        }),
        json!({
            "pack_id":"B10x_silicon_rint",
            "layer":"hard",
            "priority": 1088,
            "reason": "edh_b10x_silicon_must_land",
            "hard_eligible": true
        }),
    ];
    apply_band_pack_budget(&mut packs, &band);
    assert!(
        packs.iter().any(|p| p["pack_id"] == "B10x_silicon_ulp"),
        "B10x ulp must not be truncated: {:?}",
        packs
    );
    assert!(
        packs.iter().any(|p| p["pack_id"] == "B10x_silicon_noderiv"),
        "B10x noderiv must not be truncated"
    );
    assert!(
        packs.iter().any(|p| p["pack_id"] == "B10x_silicon_rint"),
        "B10x rint must not be truncated"
    );
}

#[test]
fn p2_route_seal_and_learning_and_unknown() {
    let route = json!({
        "session_id": "s1",
        "plan_version": 2,
        "mission_id": "raise_rpa_clarity",
        "packs": [{"pack_id":"B11_interaction"}],
    });
    let seal = sign_route_plan(&route, "s1", 5);
    assert_eq!(seal.get("evidence_rev").and_then(|v| v.as_i64()), Some(5));
    assert!(seal.get("signed").is_some());

    let priors = update_direction_priors(None, &["bio_input".into()], true);
    assert_eq!(
        priors.pointer("/bio_input/n").and_then(|v| v.as_f64()),
        Some(1.0)
    );
    let priors2 = update_direction_priors(Some(&priors), &["bio_input".into()], false);
    assert_eq!(
        priors2.pointer("/bio_input/n").and_then(|v| v.as_f64()),
        Some(2.0)
    );

    let belief = json!({
        "unknown_flags": ["no_webgl", "no_ua_ch"],
        "adversary_tags": [],
        "axes": {"engine": {"status": "unknown"}}
    });
    let env = json!({
        "decision": "declare_out_of_envelope",
        "capability_bitmap": {"webgl": false}
    });
    let ub = unknown_bucket_from_belief(&belief, &env).unwrap();
    assert_eq!(ub.get("out_of_envelope").and_then(|v| v.as_bool()), Some(true));
}

#[test]
fn evaluate_emits_route_seal_and_control_persist() {
    let ev = json!({
        "session_id": "iss22_p12",
        "sources": ["main"],
        "batches": [
            {"batch_id":"B0_bootstrap","source":"main"},
            {"batch_id":"B1_conflict","source":"main"}
        ],
        "fields": {
            "form_class": "desktop",
            "platform": "Linux x86_64",
            "user_agent": "Mozilla/5.0 Chrome/120",
            "hardware_concurrency": 8,
            "webdriver": false,
            "engine_claim": "blink",
            "engine_obs": "blink",
        },
        "evidence_rev": 2,
    });
    let out = evaluate_session(&ev, None, None, None, false).unwrap();
    assert!(out.pointer("/route_plan/route_seal").is_some());
    assert!(out.pointer("/route_plan/evidence_rev").is_some());
    assert!(out.pointer("/route_plan/mission_id").is_some());
    assert!(out.get("control_persist").is_some());
    assert!(out.get("direction_priors").is_some());
    // missions should prefer device or rpa for thin evidence
    let primary = out
        .pointer("/missions/primary")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        !primary.is_empty() && primary != "null",
        "primary mission empty"
    );
}
