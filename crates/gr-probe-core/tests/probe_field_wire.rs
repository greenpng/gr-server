//! Prove high-value probe fields change shipped OS/BR/RPA scores and reasons.
use gr_probe_core::bot::BotScore;
use gr_probe_core::policy_band_from_scores;
use gr_probe_core::product_scores::{score_br, score_os, score_rpa};
use gr_probe_core::stack_auth::stack_auth_from_fields;
use gr_probe_core::xsrc::TruthResult;
use serde_json::{json, Value};

fn truth_ok() -> TruthResult {
    TruthResult {
        xsrc_status: "consistent".into(),
        real_band: "watch".into(),
        credibility: 0.55,
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

fn reasons(block: &Value) -> Vec<String> {
    block
        .get("reasons")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(|s| s.to_string()))
        .collect()
}

fn hits(block: &Value) -> Vec<String> {
    block
        .get("field_hits")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(|s| s.to_string()))
        .collect()
}

fn base_fields() -> Value {
    json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "os_family": "linux",
        "hardware_concurrency": 8,
        "timezone": "UTC",
        "user_agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/120.0.0.0 Safari/537.36",
        "webdriver": false,
        "webgl_support": true,
    })
}

#[test]
fn product_sub_mismatch_is_consumed_by_score_br() {
    let mut ok = base_fields();
    ok.as_object_mut()
        .unwrap()
        .insert("product_sub".into(), json!("20030107"));
    let stack_ok = stack_auth_from_fields(&ok);
    let br_ok = score_br(&ok, &stack_ok, &bot_human(), &truth_ok(), &[]);

    let mut bad = base_fields();
    bad.as_object_mut()
        .unwrap()
        .insert("product_sub".into(), json!("20100101"));
    let stack_bad = stack_auth_from_fields(&bad);
    let br_bad = score_br(&bad, &stack_bad, &bot_human(), &truth_ok(), &[]);
    let rs = reasons(&br_bad);
    assert!(
        rs.iter().any(|r| r.contains("product_sub")),
        "product_sub must reach score_br reasons={rs:?}"
    );
    assert!(
        hits(&br_bad).iter().any(|h| h == "product_sub"),
        "product_sub field_hits missing {:?}",
        hits(&br_bad)
    );
    assert!(
        br_ok.get("score") != br_bad.get("score") || reasons(&br_ok) != rs,
        "engine-consistent product_sub must not score the same as a mismatch"
    );
}

#[test]
fn residual_std_and_canvas_noise_change_os_score() {
    let stack = stack_auth_from_fields(&base_fields());
    let control = score_os(&base_fields(), &stack, &truth_ok(), &[]);
    let mut rich = base_fields();
    rich.as_object_mut().unwrap().insert("residual_std".into(), json!(0.09));
    rich.as_object_mut()
        .unwrap()
        .insert("canvas_noise_hash".into(), json!("cn_deadbeef"));
    rich.as_object_mut()
        .unwrap()
        .insert("audio_noise_energy".into(), json!(173.5));
    let treated = score_os(&rich, &stack, &truth_ok(), &[]);
    let rs = reasons(&treated);
    assert!(
        rs.iter().any(|r| r.contains("residual_std") || r.contains("canvas_noise")),
        "expected residual/canvas reasons, got {rs:?}"
    );
    assert!(
        hits(&treated).iter().any(|h| h == "residual_std" || h == "canvas_noise_hash"),
        "expected field_hits, got {:?}",
        hits(&treated)
    );
    // Materials must not leave score identical to empty control in all dimensions
    assert!(
        control.get("score") != treated.get("score")
            || reasons(&control) != rs
            || hits(&control) != hits(&treated),
        "control and treated OS must differ"
    );
}

#[test]
fn permission_shape_and_agent_parity_change_br_score() {
    let stack = stack_auth_from_fields(&base_fields());
    let bot = bot_human();
    let control = score_br(&base_fields(), &stack, &bot, &truth_ok(), &[]);
    let mut rich = base_fields();
    {
        let o = rich.as_object_mut().unwrap();
        o.insert("permissions_geolocation".into(), json!("prompt"));
        o.insert("geolocation_permission".into(), json!("prompt"));
        o.insert("permission_states".into(), json!("geolocation=prompt|notifications=denied"));
        o.insert("permissions_granted_n".into(), json!(0));
        o.insert("permissions_prompt_n".into(), json!(2));
        o.insert("permissions_denied_n".into(), json!(1));
        o.insert("agent_has_webdriver".into(), json!(true));
        o.insert("agent_has_selenium".into(), json!(true));
        o.insert("agent_parity_matrix".into(), json!({"webdriver": true}));
    }
    let treated = score_br(&rich, &stack, &bot, &truth_ok(), &[]);
    let rs = reasons(&treated);
    assert!(
        rs.iter().any(|r| r.contains("permission_geo") || r.contains("agent_has_webdriver")),
        "expected permission/agent reasons: {rs:?}"
    );
    let c_score = control["score"].as_f64().unwrap_or(1.0);
    let t_score = treated["score"].as_f64().unwrap_or(1.0);
    assert!(
        t_score < c_score,
        "agent automation surface must demote br score: control={c_score} treated={t_score}"
    );
}

#[test]
fn gpu_ns_timer_unavailable_demotes_os_with_reason() {
    let stack = stack_auth_from_fields(&base_fields());
    let mut fields = base_fields();
    {
        let o = fields.as_object_mut().unwrap();
        o.insert("gpu_timer_query_available".into(), json!(true));
        o.insert("gpu_ns_readback_ok".into(), json!(false));
        o.insert("h01_points".into(), json!(6));
        o.insert("gpu_ns_async_frames".into(), json!(12));
    }
    let os = score_os(&fields, &stack, &truth_ok(), &[]);
    let rs = reasons(&os);
    assert!(
        rs.iter().any(|r| r.contains("gpu_ns_timer_available_no_readback")),
        "expected no-readback reason: {rs:?}"
    );
    assert!(hits(&os).iter().any(|h| h == "gpu_ns_async_h01" || h == "h01_points"));
}

#[test]
fn caps_actual_far_below_claim_demotes_os() {
    let stack = stack_auth_from_fields(&base_fields());
    let mut fields = base_fields();
    {
        let o = fields.as_object_mut().unwrap();
        o.insert("caps_claimed_max_tex".into(), json!(16384));
        o.insert("caps_actual_max_tex".into(), json!(1024));
        o.insert("caps_ok_n".into(), json!(2));
    }
    let os = score_os(&fields, &stack, &truth_ok(), &[]);
    let rs = reasons(&os);
    assert!(
        rs.iter().any(|r| r.contains("caps_actual_far_below_claim")),
        "expected caps claim gap: {rs:?}"
    );
}

#[test]
fn pohw_and_clock_materials_hit_os() {
    let stack = stack_auth_from_fields(&base_fields());
    let mut fields = base_fields();
    {
        let o = fields.as_object_mut().unwrap();
        o.insert("pohw_triad_ok".into(), json!(true));
        o.insert("challenge_cv".into(), json!(0.02));
        o.insert("clock_resolution_ms".into(), json!(0.01));
        o.insert("raf_cv".into(), json!(0.05));
    }
    let os = score_os(&fields, &stack, &truth_ok(), &[]);
    let rs = reasons(&os);
    assert!(rs.iter().any(|r| r.contains("pohw_triad_ok")), "{rs:?}");
    assert!(rs.iter().any(|r| r.contains("challenge_cv")), "{rs:?}");
    assert!(hits(&os).iter().any(|h| h == "clock_raf_h08" || h == "pohw_challenge"));
}

#[test]
fn agent_parity_flats_demote_rpa() {
    let bot = bot_human();
    let mut fields = base_fields();
    {
        let o = fields.as_object_mut().unwrap();
        o.insert("behavior_early_bound".into(), json!(true));
        o.insert("behavior_count".into(), json!(20));
        o.insert(
            "behavior_events".into(),
            json!([
                {"kind": "mousemove"},
                {"kind": "click"},
                {"kind": "keydown"},
                {"kind": "scroll"}
            ]),
        );
        o.insert("behavior_type_diversity_n".into(), json!(4));
        o.insert("agent_has_puppeteer".into(), json!(true));
        o.insert("agent_automation_globals_n".into(), json!(2));
        o.insert("agent_parity_hit_ratio".into(), json!(0.25));
    }
    let rpa = score_rpa(&fields, &bot, Some("page1"));
    let rs = reasons(&rpa);
    assert!(
        rs.iter().any(|r| r.contains("agent_parity_automation_surface")),
        "expected rpa agent parity reason: {rs:?}"
    );
}

#[test]
fn critical_policy_band_never_prefer_stop() {
    let band = policy_band_from_scores(Some(0.1), Some(0.1), Some(0.05));
    assert_eq!(band["band"], "critical");
    assert_eq!(band["prefer_stop"], false);
    assert_eq!(band["probe_policy"], "verify_deepen");
}
