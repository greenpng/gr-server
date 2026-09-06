//! iss/21 verified debts — contract tests (U-1/U-3/T-BIO/T-BR/T-COV + §15.4).
use gr_probe_core::bot::score_bot;
use gr_probe_core::brain::{build_frontier, scan_gaps};
use gr_probe_core::product_scores::{score_br, score_os, score_rpa};
use gr_probe_core::protocol_edge::{brand_to_engine_family, derive_engine_claim_obs};
use gr_probe_core::stack_auth::stack_auth_from_fields;
use gr_probe_core::xsrc::TruthResult;
use serde_json::{json, Map};

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

#[test]
fn u3_chrome_shell_without_runtime_not_hard_penalized() {
    // WebView / stripped Chromium shell: Chrome UA, no chrome.runtime, no webdriver
    let fields = json!({
        "user_agent": "Mozilla/5.0 (Linux; Android 13; Pixel) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Mobile Safari/537.36",
        "chrome_runtime": false,
        "webdriver": false,
        "form_class": "mobile",
        "platform": "Linux armv8l",
    });
    let stack = stack_auth_from_fields(&fields);
    let bot = score_bot(&fields, "balanced").unwrap();
    let br = score_br(&fields, &stack, &bot, &truth_ok(), &[]);
    let reasons: Vec<_> = br["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r.as_str())
        .collect();
    assert!(
        !reasons.iter().any(|r| r.contains("chrome_ua_without_runtime_headless")),
        "must not hard-risk plain Chrome shell: {reasons:?}"
    );
    // claim-only note is OK
    let _ = reasons;
}

#[test]
fn u3_headless_chrome_without_runtime_raises_br_risk() {
    let fields = json!({
        "user_agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome/120.0.0.0 Safari/537.36",
        "chrome_runtime": false,
        "webdriver": true,
        "form_class": "desktop",
        "platform": "Linux x86_64",
    });
    let stack = stack_auth_from_fields(&fields);
    let bot = score_bot(&fields, "balanced").unwrap();
    let br = score_br(&fields, &stack, &bot, &truth_ok(), &[]);
    let reasons: Vec<_> = br["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r.as_str())
        .collect();
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("chrome_ua_without_runtime") || r.contains("webdriver")),
        "headless path must still surface control signals: {reasons:?}"
    );
    assert!(br["score"].as_f64().unwrap() < 0.7);
}

#[test]
fn t_bio1_pre_action_zero_move_raises_rpa_risk() {
    let human = json!({
        "behavior_early_bound": true,
        "behavior_count": 8,
        "behavior_events": [
            {"t":1,"kind":"mousemove","x":10.3,"y":20.7},
            {"t":2,"kind":"mousemove","x":15.1,"y":22.4},
            {"t":3,"kind":"mousemove","x":40.6,"y":50.2},
            {"t":4,"kind":"mousemove","x":80.2,"y":90.8},
            {"t":5,"kind":"click","x":81.1,"y":91.3}
        ],
        "webdriver": false,
        "page_id": "p1",
    });
    let scripted = json!({
        "behavior_early_bound": true,
        "behavior_count": 8,
        "behavior_events": [
            {"t":1,"kind":"click","x":100.0,"y":200.0},
            {"t":2,"kind":"click","x":100.0,"y":200.0},
            {"t":3,"kind":"click","x":100.0,"y":200.0},
            {"t":4,"kind":"click","x":100.0,"y":200.0},
            {"t":5,"kind":"click","x":100.0,"y":200.0}
        ],
        "pre_action_move_count": 0,
        "integer_coord_ratio": 1.0,
        "input_mouse_entropy": 0.05,
        "webdriver": false,
        "page_id": "p1",
    });
    let bot = score_bot(&human, "balanced").unwrap();
    let r_human = score_rpa(&human, &bot, Some("p1"));
    let r_script = score_rpa(&scripted, &bot, Some("p1"));
    let sh = r_human["score"].as_f64().unwrap();
    let ss = r_script["score"].as_f64().unwrap();
    assert!(
        ss < sh,
        "scripted zero-move / integer coords must score lower rpa: script={ss} human={sh}"
    );
    let reasons: Vec<_> = r_script["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r.as_str())
        .collect();
    assert!(
        reasons.iter().any(|r| r.contains("pre_action_move") || r.contains("integer_coord") || r.contains("input_mouse")),
        "expected kinematics reasons: {reasons:?}"
    );
}

#[test]
fn t_br1_spoof_beats_static_webdriver_alone_ordering() {
    // With only webdriver, br risk exists; with high spoof + cross conflict, risk should be high even without webdriver.
    let only_wd = json!({
        "user_agent": "Mozilla/5.0 Chrome/120",
        "webdriver": true,
        "automation": {"webdriver": true},
        "form_class": "desktop",
        "platform": "Linux",
    });
    let incoherence = json!({
        "user_agent": "Mozilla/5.0 Chrome/120",
        "webdriver": false,
        "automation": {"webdriver": false},
        "spoof_score": 0.8,
        "material_cross_conflict": true,
        "form_class": "desktop",
        "platform": "Linux",
    });
    let bot_wd = score_bot(&only_wd, "balanced").unwrap();
    let bot_in = score_bot(&incoherence, "balanced").unwrap();
    let stack_wd = stack_auth_from_fields(&only_wd);
    let stack_in = stack_auth_from_fields(&incoherence);
    let br_wd = score_br(&only_wd, &stack_wd, &bot_wd, &truth_ok(), &[]);
    let br_in = score_br(&incoherence, &stack_in, &bot_in, &truth_ok(), &[]);
    // incoherence path must be competitive (not ignored vs static webdriver)
    assert!(
        br_in["score"].as_f64().unwrap() <= br_wd["score"].as_f64().unwrap() + 0.15,
        "consistency signals should matter: incoherence={} wd={}",
        br_in["score"],
        br_wd["score"]
    );
}

#[test]
fn t_os1_soft_stack_primary_over_string_renderer() {
    let string_only = json!({
        "software_renderer_heuristic": true,
        "webgl_unmasked_renderer": "CustomSoftGPU",
        "form_class": "desktop",
        "platform": "Linux",
    });
    let phys = json!({
        "software_renderer_heuristic": true,
        "webgl_unmasked_renderer": "SwiftShader",
        "residual_soft_like": true,
        "stack_class": "soft_render",
        "vm_score": 0.7,
        "form_class": "desktop",
        "platform": "Linux",
        "hw_curve_webgl": null,
    });
    let s0 = stack_auth_from_fields(&string_only);
    let s1 = stack_auth_from_fields(&phys);
    let os0 = score_os(&string_only, &s0, &truth_ok(), &[]);
    let os1 = score_os(&phys, &s1, &truth_ok(), &[]);
    // physical soft path should be at least as harsh as string-only
    assert!(
        os1["score"].as_f64().unwrap() <= os0["score"].as_f64().unwrap() + 0.05,
        "phys soft should not be softer than string-only: phys={} string={}",
        os1["score"],
        os0["score"]
    );
}

#[test]
fn t_cov1_unknown_env_missing_webgl_audio_still_scores() {
    // Minimal unknown shell: no WebGL, no Audio, no UA-CH — must still analyze
    let fields = json!({
        "form_class": "desktop",
        "platform": "Unknown",
        "user_agent": "MyWeirdShell/1.0",
        "hardware_concurrency": 2,
        "webdriver": false,
        "behavior_early_bound": true,
        "behavior_count": 2,
        "behavior_events": [{"t":1,"kind":"click","x":1,"y":2}],
        "page_id": "unknown_shell",
    });
    let stack = stack_auth_from_fields(&fields);
    let bot = score_bot(&fields, "balanced").unwrap();
    let os = score_os(&fields, &stack, &truth_ok(), &[]);
    let br = score_br(&fields, &stack, &bot, &truth_ok(), &[]);
    let rpa = score_rpa(&fields, &bot, Some("unknown_shell"));
    assert!(os.get("score").and_then(|v| v.as_f64()).is_some());
    assert!(br.get("score").and_then(|v| v.as_f64()).is_some());
    assert!(rpa.get("score").and_then(|v| v.as_f64()).is_some());
    // No panic path — status may be unknown/suspect but must exist
    assert!(os.get("status").is_some());
    assert!(br.get("status").is_some());
    assert!(rpa.get("status").is_some());
}

#[test]
fn t_emu_mobile_ua_claim_vs_desktop_form() {
    let fields = json!({
        "form_class": "desktop",
        "mobile_ua_claim": true,
        "mobile_ua_signals": true,
        "user_agent": "Mozilla/5.0 (iPhone; CPU iPhone OS 16_0 like Mac OS X)",
        "platform": "Linux x86_64",
        "max_touch_points": 0,
    });
    let stack = stack_auth_from_fields(&fields);
    let os = score_os(&fields, &stack, &truth_ok(), &[]);
    let reasons: Vec<_> = os["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r.as_str())
        .collect();
    assert!(
        reasons.iter().any(|r| r.contains("mobile_ua_claim_vs_desktop_form")),
        "expected form claim-obs: {reasons:?}"
    );
}

#[test]
fn t_bio1_extended_kinematics_materials() {
    let fields = json!({
        "behavior_early_bound": true,
        "behavior_count": 12,
        "behavior_events": [
            {"t":100,"kind":"mousemove","x":10.2,"y":11.3},
            {"t":120,"kind":"mousemove","x":40.5,"y":50.1},
            {"t":140,"kind":"mousemove","x":90.7,"y":80.2},
            {"t":200,"kind":"scroll"},
            {"t":210,"kind":"wheel"},
            {"t":220,"kind":"wheel"},
            {"t":230,"kind":"wheel"},
            {"t":240,"kind":"wheel"},
            {"t":300,"kind":"click","x":91.1,"y":81.4}
        ],
        "ttfi_ms": 350.0,
        "event_order_score": 0.9,
        "scroll_burst_n": 2,
        "input_velocity_cv": 0.55,
        "path_length_px": 200.0,
        "input_mouse_entropy": 0.5,
        "integer_coord_ratio": 0.4,
        "pre_action_move_count": 3,
        "webdriver": false,
        "page_id": "p_kin",
    });
    let bot = score_bot(&fields, "balanced").unwrap();
    let rpa = score_rpa(&fields, &bot, Some("p_kin"));
    let hits: Vec<_> = rpa["field_hits"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|h| h.as_str())
        .collect();
    for need in [
        "ttfi_ms",
        "event_order_score",
        "scroll_burst_n",
        "input_velocity_cv",
        "path_length_px",
        "input_mouse_entropy",
        "pre_action_move_count",
    ] {
        assert!(hits.iter().any(|h| h.contains(need)), "missing hit {need}: {hits:?}");
    }
    assert!(rpa["score"].as_f64().unwrap() > 0.35);
}

#[test]
fn t_bio2_sensitive_zero_move_joint_rpa_br() {
    let fields = json!({
        "behavior_early_bound": true,
        "behavior_count": 3,
        "behavior_events": [
            {"t":1,"kind":"keydown"},
            {"t":2,"kind":"click","x":10.0,"y":10.0}
        ],
        "pre_action_move_count": 0,
        "sensitive_action_zero_move": true,
        "sensitive_action_seen": true,
        "webdriver": false,
        "user_agent": "Mozilla/5.0 Chrome/120",
        "form_class": "desktop",
        "page_id": "p_form",
    });
    let stack = stack_auth_from_fields(&fields);
    let bot = score_bot(&fields, "balanced").unwrap();
    let rpa = score_rpa(&fields, &bot, Some("p_form"));
    let br = score_br(&fields, &stack, &bot, &truth_ok(), &[]);
    let rr: Vec<_> = rpa["reasons"].as_array().unwrap().iter().filter_map(|r| r.as_str()).collect();
    let br_r: Vec<_> = br["reasons"].as_array().unwrap().iter().filter_map(|r| r.as_str()).collect();
    assert!(rr.iter().any(|r| r.contains("sensitive_action_zero_move")), "{rr:?}");
    assert!(br_r.iter().any(|r| r.contains("sensitive_action_zero_move")), "{br_r:?}");
}

#[test]
fn t_eng1_engine_family_claim_obs() {
    assert_eq!(brand_to_engine_family("firefox"), "gecko");
    assert_eq!(brand_to_engine_family("chrome"), "blink");
    assert_eq!(brand_to_engine_family("safari"), "webkit");
    assert_eq!(brand_to_engine_family("edge"), "blink");
    assert_eq!(brand_to_engine_family("unknown_tls13"), "unknown");

    let mut fo = Map::new();
    fo.insert(
        "user_agent".into(),
        json!("Mozilla/5.0 (X11; Linux) Chrome/120 Safari/537.36"),
    );
    fo.insert("chrome_runtime".into(), json!(true));
    let (claim, obs) = derive_engine_claim_obs(&fo);
    assert_eq!(claim, "blink");
    assert_eq!(obs, "blink");

    // mismatch: claims gecko, obs blink
    let bad = json!({
        "user_agent": "Mozilla/5.0 Firefox/120.0",
        "engine_claim": "gecko",
        "engine_obs": "blink",
        "form_class": "desktop",
        "webdriver": false,
    });
    let stack = stack_auth_from_fields(&bad);
    let bot = score_bot(&bad, "balanced").unwrap();
    let br = score_br(&bad, &stack, &bot, &truth_ok(), &[]);
    let reasons: Vec<_> = br["reasons"].as_array().unwrap().iter().filter_map(|r| r.as_str()).collect();
    assert!(
        reasons.iter().any(|r| r.contains("engine_claim_obs_mismatch")),
        "{reasons:?}"
    );
}

#[test]
fn t_brain1_bio_cdp_budget_floor() {
    let evidence = json!({
        "session_id": "bio_thin",
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
            "hardware_concurrency": 4
            // no behavior kinematics, no cdp_runtime_hint
        },
        "has_gateway": false,
    });
    let gaps = scan_gaps(&evidence, None).expect("gaps");
    let codes: Vec<_> = gaps.iter().map(|g| g.code.as_str()).collect();
    assert!(
        codes.iter().any(|c| *c == "need_rpa_bind" || *c == "need_rpa_kinematics" || *c == "need_cdp_probe"),
        "expected bio/cdp gaps: {codes:?}"
    );
    let plan = build_frontier(&evidence, false, None, 8).expect("frontier");
    let packs: Vec<_> = plan
        .packs
        .iter()
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()))
        .collect();
    assert!(
        packs.iter().any(|p| *p == "B11_interaction"),
        "B11 must be scheduled under bio floor: {packs:?}"
    );
    assert!(
        packs.iter().any(|p| *p == "B12_anti_camouflage"),
        "B12 must be scheduled under cdp floor: {packs:?}"
    );
    assert!(
        plan.notes.iter().any(|n| n.contains("bio_cdp_budget_floor")
            || n.contains("bio_budget_floor")
            || n.contains("cdp_budget_floor")),
        "notes should mention budget floor: {:?}",
        plan.notes
    );
}

#[test]
fn t_bio_matrix_rpa_material_at_least_12() {
    use gr_probe_core::product_matrix::load_field_product_matrix;
    let m = load_field_product_matrix().expect("matrix");
    let n = m
        .fields
        .iter()
        .filter(|f| f.axes.get("rpa").map(|r| r.as_str()) == Some("material"))
        .count();
    assert!(n >= 12, "rpa material count {n} < 12 (iss/21 T-BIO-1)");
}
