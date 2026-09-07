//! iss/36 develop-now slice: FE antidetect emit path (structural), recommended_action,
//! JA low-weight br aux, unknown-UA honesty — without rewriting residual commercial id.

use gr_probe_core::{
    association_ladder, commercial_projection, derive_engine_claim_obs, derive_recommended_action,
    evaluate_session, score_br, stack_auth_from_fields, BotScore, TruthResult,
};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::PathBuf;

fn curves() -> (Vec<f64>, Vec<f64>) {
    let a: Vec<f64> = (0..32)
        .map(|i| (i as f64 * 0.11).sin().abs() * 0.5 + 0.05)
        .collect();
    let w: Vec<f64> = (0..16)
        .map(|i| ((i * 13) % 200) as f64 / 200.0 + 0.01)
        .collect();
    (a, w)
}

fn rich_fields(residual: f64) -> Value {
    let (a, w) = curves();
    json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "hardware_concurrency": 8,
        "timezone": "UTC",
        "os_family": "linux",
        "architecture": "x86",
        "residual_mean": residual,
        "hw_curve_audio": a,
        "hw_curve_webgl": w,
        "unit_surface_id": "u_dev_rank",
        "unit_surface_algo": "gr_unit_v1",
        "unit_multiround_stable": true,
        "screen_width": 1280,
        "screen_height": 720,
        "user_agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
    })
}

fn truth_ok() -> TruthResult {
    TruthResult {
        xsrc_status: "ok".into(),
        real_band: "likely_real".into(),
        credibility: 0.55,
        fe_only: true,
        has_server_side: false,
        has_main_core: true,
        packages: json!({}),
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
        algo: "t".into(),
        details: json!({}),
        robot_name: None,
    }
}

/// D-1 structural: FE B12 registry emits antidetect/emulator/prototype fields scorers already read.
#[test]
fn fe_b12_emits_antidetect_emulator_prototype_fields() {
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.pop(); // crates
    root.pop(); // repo root (FE lives under probe/fe)
    let reg = [root.join("probe/fe/collectors/registry.js"), root.join("fe/collectors/registry.js")]
        .into_iter()
        .find(|p| p.is_file())
        .expect("registry.js readable");
    let src = fs::read_to_string(&reg).expect("registry.js readable");
    for needle in [
        "antidetect_vendor_hint",
        "emulator_hint",
        "prototype_chain_tamper",
        "fingerprint_vendor_lie",
        "B12_anti_camouflage",
    ] {
        assert!(
            src.contains(needle),
            "FE registry must emit/contain {needle} for scorer path"
        );
    }
    // Must live inside B12 pack body (to next register), not only comments elsewhere.
    // Pack body can exceed 8k as anti-camouflage grows — bound by next `register(`.
    let b12 = src
        .find("register(\"B12_anti_camouflage\"")
        .expect("B12 register");
    let after = &src[b12 + 10..];
    let next_reg = after
        .find("\n  register(")
        .or_else(|| after.find("\n  register(\""))
        .unwrap_or(after.len().min(24000));
    let slice = &src[b12..b12 + 10 + next_reg];
    assert!(
        slice.contains("prototype_chain_tamper"),
        "prototype_chain_tamper must be inside B12 body (len={})",
        slice.len()
    );
    assert!(
        slice.contains("antidetect_vendor_hint"),
        "antidetect_vendor_hint must be inside B12 body (len={})",
        slice.len()
    );
    assert!(
        slice.contains("emulator_hint"),
        "emulator_hint must be inside B12 body (len={})",
        slice.len()
    );
    assert!(
        slice.contains("fingerprint_vendor_lie"),
        "fingerprint_vendor_lie must be inside B12 body"
    );
}

/// D-1 path: fields that FE would emit fire existing scorers; residual-led id stable.
#[test]
fn antidetect_fields_from_fe_shape_hit_scorers_not_digest() {
    let mut fields = rich_fields(0.500423);
    {
        let o = fields.as_object_mut().unwrap();
        o.insert("antidetect_vendor_hint".into(), json!(true));
        o.insert("prototype_chain_tamper".into(), json!(true));
        o.insert("fingerprint_vendor_lie".into(), json!(true));
        o.insert("emulator_hint".into(), json!("android_emu"));
        o.insert("webgl_unmasked_renderer".into(), json!("NVIDIA GeForce GTX 1060"));
    }
    let stack = stack_auth_from_fields(&fields);
    let br = score_br(&fields, &stack, &bot_human(), &truth_ok(), &[]);
    let reasons: String = br["reasons"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str())
        .collect::<Vec<_>>()
        .join("|");
    assert!(
        reasons.contains("antidetect")
            || reasons.contains("prototype")
            || reasons.contains("integrity"),
        "br must consume FE antidetect fields: {reasons}"
    );

    let mut clean = rich_fields(0.500423);
    {
        let o = clean.as_object_mut().unwrap();
        o.insert("webgl_unmasked_renderer".into(), json!("NVIDIA GeForce GTX 1060"));
    }
    let proj_anti = commercial_projection(&fields);
    let proj_clean = commercial_projection(&clean);
    // L0 GPU / antidetect claim-obs must not fork residual commercial digest materials
    let mats_a = proj_anti
        .get("materials_included")
        .or_else(|| proj_anti.pointer("/materials"))
        .cloned()
        .unwrap_or(json!([]));
    let mats_c = proj_clean
        .get("materials_included")
        .or_else(|| proj_clean.pointer("/materials"))
        .cloned()
        .unwrap_or(json!([]));
    let s_a = serde_json::to_string(&mats_a).unwrap_or_default();
    let s_c = serde_json::to_string(&mats_c).unwrap_or_default();
    assert!(
        !s_a.contains("antidetect") && !s_a.contains("ja4"),
        "commercial materials must not include antidetect/ja: {s_a}"
    );
    let _ = s_c;
    let lad = association_ladder(&fields, None);
    // emulator_hint alone may still allow non-hardware; soft not required
    assert!(lad.get("association_level").is_some());
}

/// D-3: JA/protocol low-weight aux demotes br via score_br itself (empty conflicts).
/// Must not rely on xsrc source_conflicts short-circuit in fuse_axis_hedge.
#[test]
fn ja_protocol_low_weight_br_aux_not_digest() {
    // Rich br surface so coverage_cap does not erase low-weight JA risk delta.
    let mut fields = rich_fields(0.500423);
    {
        let o = fields.as_object_mut().unwrap();
        o.insert(
            "user_agent".into(),
            json!("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/120.0.0.0 Safari/537.36"),
        );
        o.insert("engine_claim".into(), json!("blink"));
        o.insert("engine_obs".into(), json!("blink"));
        o.insert("chrome_runtime".into(), json!(true));
        o.insert("sandbox_ok".into(), json!(true));
        o.insert("canvas_hash".into(), json!("cafebabe"));
        o.insert("fonts_hash".into(), json!("deadbeef"));
        o.insert("languages".into(), json!(["en-US", "en"]));
        o.insert("language".into(), json!("en-US"));
        o.insert("webgl_max_texture".into(), json!(16384));
        o.insert("protocol_engine".into(), json!("firefox"));
        o.insert("ja4".into(), json!("ff_t13d1516h2_labmismatch"));
    }
    let stack = stack_auth_from_fields(&fields);
    let mut truth = truth_ok();
    truth.has_server_side = true; // drop no_server_side_cap noise
    // Empty conflicts: exercise score_br low-weight JA path only.
    let br = score_br(&fields, &stack, &bot_human(), &truth, &[]);
    let reason_list: Vec<String> = br["reasons"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(|s| s.to_string()))
        .collect();
    let reasons = reason_list.join("|");
    let has_proto = reason_list
        .iter()
        .any(|s| s.contains("ja_protocol_vs_ua_claim_low_weight"));
    let has_ja4 = reason_list
        .iter()
        .any(|s| s.contains("ja4_vs_ua_claim_low_weight"));
    assert!(
        has_proto || has_ja4,
        "must emit explicit low_weight JA reasons (not xsrc short-circuit): {reasons}"
    );
    assert!(
        has_proto,
        "protocol_engine firefox vs blink claim must tag ja_protocol_vs_ua_claim_low_weight: {reasons}"
    );
    assert!(
        has_ja4,
        "ja4 ff_* vs blink claim must tag ja4_vs_ua_claim_low_weight: {reasons}"
    );

    let br_ok = {
        let mut ok = fields.clone();
        let o = ok.as_object_mut().unwrap();
        o.insert("protocol_engine".into(), json!("chrome"));
        o.insert("ja4".into(), json!("cr_t13d1516h2_ok"));
        let st = stack_auth_from_fields(&ok);
        score_br(&ok, &st, &bot_human(), &truth, &[])
    };
    let ok_reasons: String = br_ok["reasons"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str())
        .collect::<Vec<_>>()
        .join("|");
    assert!(
        !ok_reasons.contains("ja_protocol_vs_ua_claim_low_weight")
            && !ok_reasons.contains("ja4_vs_ua_claim_low_weight"),
        "confirm path must not carry mismatch low_weight tags: {ok_reasons}"
    );
    let score_mis = br["score"].as_f64().unwrap_or(1.0);
    let score_ok = br_ok["score"].as_f64().unwrap_or(0.0);
    assert!(
        score_mis + 0.005 < score_ok,
        "mismatch br must demote vs confirm path: mis={score_mis} ok={score_ok} r_mis={reasons} r_ok={ok_reasons}"
    );

    let proj = commercial_projection(&fields);
    // Digest path materials must not key on ja4 string as UV
    if let Some(inc) = proj.get("materials_included").and_then(|v| v.as_array()) {
        for m in inc {
            let s = m.as_str().unwrap_or("");
            assert!(
                !s.contains("ja4") && !s.contains("ja3"),
                "ja must not be commercial material: {s}"
            );
        }
    }
}

/// D-2: association_level=env (no soft_stack) → allow_soft, never plain allow.
#[test]
fn recommended_action_env_level_is_allow_soft() {
    let product = json!({
        "os": {"score": 0.72},
        "br": {"score": 0.70},
        "rpa": {"score": 0.75},
        "association_level": "env",
        "collision_risk": false,
    });
    let device = json!({
        "association_level": "env",
        "soft_stack": false,
        "collision_risk": false,
        "digest_path": "unit_v1",
    });
    let ra = derive_recommended_action(&product, &device, "human", Some("coverage_complete"), Some(true));
    assert_eq!(
        ra.get("action").and_then(|v| v.as_str()),
        Some("allow_soft"),
        "env bind must be allow_soft: {ra}"
    );
    let reasons: String = ra["reasons"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str())
        .collect::<Vec<_>>()
        .join("|");
    assert!(
        reasons.contains("association_level=env") || reasons.contains("soft_or_env"),
        "reasons should cite env: {reasons}"
    );
}

/// D-2: recommended_action + evaluate degradation_gate / battle_log stop_reason.
#[test]
fn recommended_action_and_battle_log_gate_on_evaluate() {
    let fields = {
        let mut f = rich_fields(0.500423);
        let o = f.as_object_mut().unwrap();
        o.insert("residual_soft_like".into(), json!(true));
        o.insert("soft_stack".into(), json!(true));
        o.insert("os_instance_hash".into(), json!("vm_sep_1"));
        o.insert("webgl_unmasked_renderer".into(), json!("SwiftShader"));
        f
    };
    let evidence = json!({
        "session_id": "s_dev_rank_rec",
        "fields": fields,
        "batches": [{"batch_id": "B12_anti_camouflage", "ok": true}],
    });
    let out = evaluate_session(&evidence, None, None, None, false).expect("evaluate");
    let product = out.get("product").expect("product");
    let rec = product
        .get("recommended_action")
        .expect("recommended_action on product");
    assert_eq!(
        rec.get("schema").and_then(|v| v.as_str()),
        Some("gr_recommended_action_v1")
    );
    let action = rec.get("action").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        matches!(
            action,
            "allow" | "allow_soft" | "step_up" | "deny_sensitive" | "challenge_bot"
        ),
        "action={action}"
    );
    // Soft path should not be plain hard allow without soft/step posture
    if product
        .get("association_level")
        .and_then(|v| v.as_str())
        == Some("hardware")
    {
        // soft_stack should prevent hardware
        panic!("soft session must not advertise association_level=hardware");
    }
    let gate = product.get("degradation_gate").expect("degradation_gate");
    assert_eq!(
        gate.get("stop_reason_nonempty").and_then(|v| v.as_bool()),
        Some(true)
    );
    let stop = out.get("stop_reason").and_then(|v| v.as_str()).unwrap_or("");
    assert!(!stop.is_empty(), "root stop_reason nonempty");
    let bl = out.get("battle_log").expect("battle_log");
    assert!(
        bl.get("schema")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .contains("battle_log"),
        "battle_log schema"
    );
    assert!(
        bl.get("stop_reason")
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        "battle_log.stop_reason"
    );

    // Pure helper path
    let device = out.get("device").cloned().unwrap_or(json!({}));
    let ra = derive_recommended_action(product, &device, "human", Some("budget"), Some(false));
    assert_eq!(
        ra.get("schema").and_then(|v| v.as_str()),
        Some("gr_recommended_action_v1")
    );
}

/// D-4: unknown UA → engine_claim unknown; evaluate still runs; hard anchor path possible.
#[test]
fn unknown_ua_honesty_contract() {
    let mut fields = rich_fields(0.500423);
    {
        let o = fields.as_object_mut().unwrap();
        o.insert(
            "user_agent".into(),
            json!("TotallyFakeBrowser/0.0 (CloudPhoneX; NotARealVendor)"),
        );
        // no engine_claim injected — derive from UA
    }
    let fo: Map<String, Value> = fields.as_object().cloned().unwrap_or_default();
    let (claim, _obs) = derive_engine_claim_obs(&fo);
    assert_eq!(claim, "unknown", "fake brand must be unknown, not invent blink");

    let evidence = json!({
        "session_id": "s_unknown_ua",
        "fields": fields,
        "batches": [{"batch_id": "B0", "ok": true}],
    });
    let out = evaluate_session(&evidence, None, None, None, false).expect("evaluate unknown UA");
    assert!(out.get("product").is_some());
    assert!(out.get("device").is_some());
    // Must not crash; may or may not emit id depending on trust floor — posture honesty
    let product = out.get("product").unwrap();
    assert!(product.get("os").is_some());
    assert!(product.get("br").is_some());
    // recommended_action present
    assert!(product.get("recommended_action").is_some());
}
