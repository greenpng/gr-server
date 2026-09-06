//! Dump iss/36 D-1..D-4 shipped-path evidence for scratch.
use gr_probe_core::{
    commercial_projection, derive_engine_claim_obs, evaluate_session, score_br,
    stack_auth_from_fields, BotScore, TruthResult,
};
use serde_json::{json, Map, Value};
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let scratch = env::var("SCRATCH").unwrap_or_else(|_| "/tmp/out".into());
    let (a, w) = (
        (0..32).map(|i| (i as f64 * 0.11).sin().abs() * 0.5 + 0.05).collect::<Vec<_>>(),
        (0..16).map(|i| ((i * 13) % 200) as f64 / 200.0 + 0.01).collect::<Vec<_>>(),
    );
    // Path A: JA family mismatch only (empty conflicts) — proves score_br low_weight tags.
    let ja_fields = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "hardware_concurrency": 8,
        "timezone": "UTC",
        "os_family": "linux",
        "residual_mean": 0.500423,
        "hw_curve_audio": a,
        "hw_curve_webgl": w,
        "unit_surface_id": "u_ev",
        "unit_surface_algo": "gr_unit_v1",
        "unit_multiround_stable": true,
        "user_agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/120.0.0.0 Safari/537.36",
        "engine_claim": "blink",
        "engine_obs": "blink",
        "chrome_runtime": true,
        "sandbox_ok": true,
        "canvas_hash": "cafebabe",
        "fonts_hash": "deadbeef",
        "languages": ["en-US", "en"],
        "language": "en-US",
        "webgl_max_texture": 16384,
        "protocol_engine": "firefox",
        "ja4": "ff_t13d1516h2_lab",
        "antidetect_vendor_hint": true,
        "prototype_chain_tamper": true,
        "fingerprint_vendor_lie": true,
        "emulator_hint": "android_emu",
    });
    // Path B: soft env for recommended_action / association ladder product surface.
    let soft_fields = {
        let mut f = ja_fields.clone();
        let o = f.as_object_mut().unwrap();
        o.insert("residual_soft_like".into(), json!(true));
        o.insert("soft_stack".into(), json!(true));
        o.insert("os_instance_hash".into(), json!("vm1"));
        o.insert("webgl_unmasked_renderer".into(), json!("SwiftShader"));
        f
    };
    let stack = stack_auth_from_fields(&ja_fields);
    let truth = TruthResult {
        xsrc_status: "ok".into(),
        real_band: "unknown".into(),
        credibility: 0.4,
        fe_only: false,
        has_server_side: true,
        has_main_core: true,
        packages: json!({}),
        reasons: vec![],
        details: json!({}),
    };
    let bot = BotScore {
        score: 15,
        verdict: "human".into(),
        flags: vec![],
        family: "none".into(),
        algo: "t".into(),
        robot_name: None,
        details: json!({}),
    };
    // Empty conflicts — must hit ja_*_low_weight reasons from score_br.
    let br = score_br(&ja_fields, &stack, &bot, &truth, &[]);
    let br_reasons: Vec<String> = br
        .get("reasons")
        .and_then(|r| r.as_array())
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(|s| s.to_string()))
        .collect();
    let has_proto_lw = br_reasons
        .iter()
        .any(|s| s.contains("ja_protocol_vs_ua_claim_low_weight"));
    let has_ja4_lw = br_reasons
        .iter()
        .any(|s| s.contains("ja4_vs_ua_claim_low_weight"));
    let proj = commercial_projection(&ja_fields);
    let mats = proj.get("materials_included").cloned().unwrap_or(json!([]));
    let evidence = json!({
        "session_id": "dump_dev_rank",
        "fields": soft_fields,
        "batches": [{"batch_id": "B12_anti_camouflage", "ok": true}],
    });
    let out = evaluate_session(&evidence, None, None, None, false).expect("eval");
    let product = out.get("product").cloned().unwrap_or(json!({}));
    let fo: Map<String, Value> = {
        let mut fake = ja_fields.clone();
        fake.as_object_mut().unwrap().insert(
            "user_agent".into(),
            json!("TotallyFakeBrowser/0.0 (CloudPhoneX)"),
        );
        fake.as_object().cloned().unwrap_or_default()
    };
    let (claim, _) = derive_engine_claim_obs(&fo);

    // env-only (no soft_stack) recommended_action contract
    let env_ra = gr_probe_core::derive_recommended_action(
        &json!({
            "os": {"score": 0.72},
            "br": {"score": 0.70},
            "rpa": {"score": 0.75},
            "association_level": "env",
        }),
        &json!({
            "association_level": "env",
            "soft_stack": false,
            "collision_risk": false,
            "digest_path": "unit_v1",
        }),
        "human",
        Some("coverage_complete"),
        Some(true),
    );

    let fe = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../probe/fe/collectors/registry.js"),
    )
    .unwrap_or_default();

    let doc = json!({
        "fe_b12_emit_fields": {
            "antidetect_vendor_hint": fe.contains("antidetect_vendor_hint"),
            "emulator_hint": fe.contains("emulator_hint"),
            "prototype_chain_tamper": fe.contains("prototype_chain_tamper"),
            "fingerprint_vendor_lie": fe.contains("fingerprint_vendor_lie"),
        },
        "br_reasons_sample": br_reasons,
        "br_score": br.get("score"),
        "ja_low_weight_reason_hit": has_proto_lw || has_ja4_lw,
        "ja_protocol_vs_ua_claim_low_weight": has_proto_lw,
        "ja4_vs_ua_claim_low_weight": has_ja4_lw,
        "source_conflicts_empty": true,
        "commercial_materials": mats,
        "materials_exclude_ja_antidetect": {
            "no_ja": !format!("{mats}").contains("ja4"),
            "no_antidetect": !format!("{mats}").contains("antidetect"),
        },
        "product_recommended_action": product.get("recommended_action"),
        "product_degradation_gate": product.get("degradation_gate"),
        "product_redlines": product.get("product_redlines"),
        "association_level": product.get("association_level"),
        "env_only_recommended_action": env_ra,
        "battle_log_schema": out.pointer("/battle_log/schema"),
        "stop_reason": out.get("stop_reason"),
        "unknown_ua_engine_claim": claim,
        "residual_led": true,
    });
    let path = PathBuf::from(&scratch).join("next_dev_slice_evidence.json");
    fs::create_dir_all(&scratch).ok();
    fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
    println!("wrote {}", path.display());
    println!("{}", serde_json::to_string_pretty(&doc).unwrap());
}
