//! Field utilization + mutual verification + confidence completeness + re-probe gaps.
//! Drives shipped evaluate_session / analysis_quality / product scores / matrix / brain.

use gr_probe_core::{
    build_field_utilization_map, build_mutual_verification, commercial_projection,
    evaluate_session, load_field_product_matrix, open_verification_gaps, scan_gaps,
    stack_auth_from_fields, BotScore, TruthResult,
};
use serde_json::{json, Value};

fn audio_curve() -> Vec<f64> {
    (0..32)
        .map(|i| (i as f64 * 0.11).sin().abs() * 0.5 + 0.05)
        .collect()
}
fn webgl_curve() -> Vec<f64> {
    (0..16)
        .map(|i| ((i * 13) % 200) as f64 / 200.0 + 0.01)
        .collect()
}

fn thin_fields() -> Value {
    json!({
        "user_agent": "Mozilla/5.0 (compatible; thin)",
        "form_class": "desktop",
        "server_client_ip": "198.51.100.7",
        "server_asn": "AS64496",
        "server_country": "US",
    })
}

fn rich_fields() -> Value {
    json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "hardware_concurrency": 12,
        "timezone": "Asia/Shanghai",
        "os_family": "linux",
        "architecture": "x86",
        "webgl_unmasked_renderer": "ANGLE (NVIDIA Corporation, NVIDIA GeForce GTX 1050 Ti/PCIe/SSE2, OpenGL 4.5.0)",
        "residual_mean": 0.500181,
        "residual_soft_like": false,
        "residual_available": true,
        "hw_curve_audio": audio_curve(),
        "hw_curve_webgl": webgl_curve(),
        "webrtc_host_ip_hash": "lan_host_rich",
        "unit_surface_id": "unit_rich_abc",
        "unit_surface_algo": "gr_unit_v1",
        "unit_multiround_stable": true,
        "multi_seed_n": 8,
        "webgl2_support": true,
        "webgl_max_texture": 16384,
        "webgl_extensions_hash": "ext_abc",
        "vm_score": 0.05,
        "stack_class": "real_silicon",
        "behavior_early_bound": true,
        "behavior_count": 24,
        "behavior_events": [
            {"kind": "mousemove"}, {"kind": "click"}, {"kind": "scroll"}, {"kind": "keydown"}
        ],
        "ja4": "t13d1516h2_8daaf6152771_b0da82dd1658",
        "screen_width": 1920,
        "screen_height": 1080,
        "server_client_ip": "203.0.113.10",
        "server_asn": "AS64500",
        "server_country": "CN",
        "user_agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/120.0.0.0 Safari/537.36",
        "chrome_runtime": true,
    })
}

fn claim_obs_soft() -> Value {
    json!({
        "form_class": "desktop",
        "platform": "Win32",
        "hardware_concurrency": 8,
        "timezone": "UTC",
        "os_family": "windows",
        "webgl_unmasked_renderer": "ANGLE (NVIDIA, NVIDIA GeForce RTX 4090 Direct3D11 vs_5_0 ps_5_0)",
        "residual_mean": 0.500423,
        "residual_soft_like": true,
        "soft_stack": true,
        "residual_available": true,
        "webgl2_support": false,
        "webgl_max_texture": 4096,
        "hw_curve_webgl": webgl_curve(),
        "hw_curve_audio": audio_curve(),
        "spoof_score": 0.5,
        "screen_width": 1366,
        "screen_height": 768,
    })
}

fn evidence(fields: Value, sources: &[&str], has_gw: bool) -> Value {
    let batches: Vec<Value> = sources
        .iter()
        .map(|s| {
            json!({
                "batch_id": if *s == "gateway" { "B8_gateway" } else { "B10_hw_curves" },
                "source": s,
            })
        })
        .collect();
    let mut ev = json!({
        "sources": sources,
        "batches": batches,
        "has_gateway": has_gw,
        "fields": fields,
        "session_id": "field_quality_sess",
        "page_id": "pq1",
    });
    if has_gw {
        ev.as_object_mut().unwrap().insert(
            "gateway_fields".into(),
            json!({
                "server_client_ip": "203.0.113.10",
                "server_asn": "AS64500",
                "server_country": "CN",
            }),
        );
    }
    ev
}

/// Matrix multi-axis roles: residual/unit/L0/control/JA* — never_digest ≠ global ban.
#[test]
fn field_map_multi_axis_roles_no_global_discard() {
    let m = load_field_product_matrix().expect("matrix");
    for f in [
        "residual_soft_like",
        "unit_surface_id",
        "webgl_unmasked_renderer",
        "webdriver",
        "ja4",
        "webgl2_support",
        "hw_curve_webgl",
    ] {
        assert!(m.by_field.contains_key(f), "missing field {f}");
    }
    let l0 = &m.by_field["webgl_unmasked_renderer"];
    assert!(l0.axes.get("os").is_some() || l0.axes.get("br").is_some());
    assert!(m
        .commercial_device_id
        .never_digest
        .iter()
        .any(|x| x.contains("webgl_unmasked") || x == "webgl_unmasked_renderer"));
    let ja4 = &m.by_field["ja4"];
    assert!(ja4.axes.get("br").is_some());
    let wd = &m.by_field["webdriver"];
    assert_eq!(wd.axes.get("rpa").map(|s| s.as_str()), Some("veto"));

    // Utilization map from shipped function
    let util = build_field_utilization_map(&rich_fields());
    assert!(util["present_count"].as_u64().unwrap_or(0) >= 5);
    let ndp = util["never_digest_policy"].as_str().unwrap();
    assert!(
        ndp.contains("reference_aux") || ndp.contains("never global"),
        "never_digest_policy should allow reference_aux: {ndp}"
    );
    assert!(util.get("by_axis").is_some());
}

/// Claim-obs multi-signal: L0 does not redefine residual device id; os/br move.
#[test]
fn mutual_verification_claim_obs_and_residual_led_device() {
    let fields = claim_obs_soft();
    let stack = stack_auth_from_fields(&fields);
    let truth = TruthResult {
        xsrc_status: "ok".into(),
        real_band: "unknown".into(),
        credibility: 0.4,
        fe_only: true,
        has_server_side: false,
        has_main_core: true,
        packages: json!({}),
        reasons: vec![],
        details: json!({}),
    };
    let bot = BotScore {
        score: 10,
        verdict: "human".into(),
        flags: vec![],
        family: "none".into(),
        algo: "t".into(),
        details: json!({}),
        robot_name: None,
    };
    let os = gr_probe_core::score_os(&fields, &stack, &truth, &[]);
    let br = gr_probe_core::score_br(&fields, &stack, &bot, &truth, &[]);
    let rpa = gr_probe_core::score_rpa(&fields, &bot, Some("p"));
    let device = json!({
        "device_id": commercial_projection(&fields).get("device_id").cloned().unwrap_or(json!(null)),
        "collision_risk": true,
    });
    let mv = build_mutual_verification(&fields, &stack, &os, &br, &rpa, &device, &truth, &[]);
    assert!(
        mv["contradict_count"].as_u64().unwrap_or(0) >= 1,
        "claim-obs must produce contradict: {mv}"
    );
    let stance = mv["fusion_stance"].as_str().unwrap_or("");
    assert!(
        stance.contains("contradict") || stance.contains("mixed") || stance.contains("thin"),
        "fusion stance reflects verification: {stance}"
    );
    // Residual-led: same residual materials with different L0 → **same** commercial device_id
    let mut honest = fields.clone();
    honest.as_object_mut().unwrap().insert(
        "webgl_unmasked_renderer".into(),
        json!("SwiftShader"),
    );
    let id_a = commercial_projection(&fields)
        .get("device_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let id_b = commercial_projection(&honest)
        .get("device_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    assert!(!id_a.is_empty() && !id_b.is_empty(), "both soft residual must emit ids");
    assert_eq!(
        id_a, id_b,
        "L0 GPU string flip must not redefine residual-led commercial id: {id_a} vs {id_b}"
    );
}

/// Residual-rich consistent session: no false score_* contradicts from low-weight GPU context.
#[test]
fn honest_rich_fusion_no_false_score_contradict() {
    let fields = rich_fields();
    let stack = stack_auth_from_fields(&fields);
    let truth = TruthResult {
        xsrc_status: "ok".into(),
        real_band: "likely_real".into(),
        credibility: 0.8,
        fe_only: false,
        has_server_side: true,
        has_main_core: true,
        packages: json!({}),
        reasons: vec![],
        details: json!({}),
    };
    let bot = BotScore {
        score: 5,
        verdict: "human".into(),
        flags: vec![],
        family: "none".into(),
        algo: "t".into(),
        details: json!({}),
        robot_name: None,
    };
    let os = gr_probe_core::score_os(&fields, &stack, &truth, &[]);
    let br = gr_probe_core::score_br(&fields, &stack, &bot, &truth, &[]);
    let rpa = gr_probe_core::score_rpa(&fields, &bot, Some("p"));
    let proj = commercial_projection(&fields);
    let device = json!({
        "device_id": proj.get("device_id").cloned().unwrap_or(json!(null)),
        "collision_risk": false,
    });
    let mv = build_mutual_verification(&fields, &stack, &os, &br, &rpa, &device, &truth, &[]);

    // No hard score_* contradict from gpu_label_claim_context_low_weight alone
    let score_contradicts: Vec<String> = mv
        .get("contradicts")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter(|c| {
            c.get("channel")
                .and_then(|v| v.as_str())
                .is_some_and(|ch| ch.starts_with("score_"))
                && c.get("stance").and_then(|v| v.as_str()) == Some("contradict")
        })
        .filter_map(|c| c.get("signal").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    assert!(
        !score_contradicts
            .iter()
            .any(|s| s.contains("gpu_label_claim_context_low_weight")),
        "low-weight GPU context must not be hard contradict: {score_contradicts:?} mv={mv}"
    );
    let stance = mv["fusion_stance"].as_str().unwrap_or("");
    assert!(
        matches!(
            stance,
            "multi_confirm" | "partial_confirm" | "mixed_verify"
        ) && stance != "multi_contradict",
        "honest residual-rich fusion should confirm, not multi_contradict: {stance} mv={mv}"
    );
    // Prefer multi_confirm / partial_confirm; mixed only if real hard contradicts exist
    if stance == "mixed_verify" {
        // mixed allowed only with real hard signals — not context-only
        assert!(
            score_contradicts.is_empty()
                || score_contradicts.iter().any(|s| {
                    s.contains("vs_residual")
                        || s.contains("incoherent")
                        || s.contains("collusion")
                        || s.contains("capability")
                }),
            "mixed_verify without hard score contradicts is a false demotion: {score_contradicts:?}"
        );
    }
    // Strong path: residual+unit consistent → multi_confirm or partial_confirm
    let n_hard = mv["contradict_count"].as_u64().unwrap_or(99);
    if n_hard == 0 {
        assert!(
            stance == "multi_confirm" || stance == "partial_confirm",
            "zero hard contradict → multi_confirm|partial_confirm, got {stance}"
        );
    }

    // L0-only flip on rich residual path: same commercial id
    let mut spoof_l0 = fields.clone();
    spoof_l0.as_object_mut().unwrap().insert(
        "webgl_unmasked_renderer".into(),
        json!("Apple M1, or similar"),
    );
    let id_a = commercial_projection(&fields)
        .get("device_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let id_b = commercial_projection(&spoof_l0)
        .get("device_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    assert!(!id_a.is_empty() && !id_b.is_empty());
    assert_eq!(
        id_a, id_b,
        "rich residual-led id must not split on L0-only spoof: {id_a} vs {id_b}"
    );
}

/// Thin vs rich materials: confidence / posture observably different.
#[test]
fn confidence_completeness_thin_vs_rich() {
    let thin_ev = evidence(thin_fields(), &["gateway"], true);
    let rich_ev = evidence(rich_fields(), &["main", "gateway"], true);
    let thin_out = evaluate_session(&thin_ev, None, None, None, true).expect("thin");
    let rich_out = evaluate_session(&rich_ev, None, None, None, true).expect("rich");
    let tp = thin_out.get("product").cloned().unwrap_or(thin_out.clone());
    let rp = rich_out.get("product").cloned().unwrap_or(rich_out.clone());

    // Product four axes present
    for ax in ["os", "br", "rpa"] {
        assert!(tp.get(ax).is_some(), "thin missing {ax}");
        assert!(rp.get(ax).is_some(), "rich missing {ax}");
    }

    let conf = |p: &Value, ax: &str| -> f64 {
        p.pointer(&format!("/{ax}/confidence"))
            .and_then(|v| v.as_f64())
            .or_else(|| {
                p.pointer(&format!("/{ax}/confidence_report/confidence"))
                    .and_then(|v| v.as_f64())
            })
            .unwrap_or(0.0)
    };
    let thin_os = conf(&tp, "os");
    let rich_os = conf(&rp, "os");
    let thin_br = conf(&tp, "br");
    let rich_br = conf(&rp, "br");
    assert!(
        rich_os + 0.02 >= thin_os || rich_br + 0.02 >= thin_br,
        "rich should not have lower conf than thin on both os/br: thin_os={thin_os} rich_os={rich_os} thin_br={thin_br} rich_br={rich_br}"
    );
    // Stronger assertion: at least one axis clearly higher for rich
    assert!(
        rich_os > thin_os + 0.03 || rich_br > thin_br + 0.03
            || rp.pointer("/device_confidence")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
                > tp.pointer("/device_confidence")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0)
                    + 0.03,
        "rich materials must raise conf vs thin: thin={tp} rich={rp}"
    );

    // Posture present on rich
    let posture = rp
        .pointer("/os/confidence_posture")
        .or_else(|| rp.pointer("/os/confidence_report/posture"))
        .and_then(|v| v.as_str())
        .unwrap_or("missing");
    assert_ne!(posture, "missing", "confidence posture required on product axis");

    // Thin should not claim sufficient on all axes silently
    let thin_posture = tp
        .pointer("/os/confidence_posture")
        .or_else(|| tp.pointer("/os/confidence_report/posture"))
        .and_then(|v| v.as_str())
        .unwrap_or("thin");
    assert!(
        thin_posture == "thin" || thin_posture == "partial" || thin_os < 0.55,
        "thin session should not look fully sufficient: posture={thin_posture} conf={thin_os}"
    );
}

/// Re-probe: soft / claim-obs / missing unit → task-oriented gaps; GPU digs demoted.
#[test]
fn reprobe_task_targeted_open_gaps() {
    let fields = json!({
        "form_class": "desktop",
        "residual_soft_like": true,
        "soft_stack": true,
        "stack_class": "soft_render",
        "residual_mean": 0.500423,
        "hw_curve_webgl": webgl_curve(),
        "hw_curve_audio": audio_curve(),
        "webgl_unmasked_renderer": "GeForce GTX 980",
        "spoof_score": 0.55,
    });
    let ev = json!({
        "sources": ["main"],
        "batches": [{"batch_id": "B0_bootstrap", "source": "main"}],
        "fields": fields,
        "session_id": "reprobe1",
    });
    let gaps = scan_gaps(&ev, None).expect("gaps");
    let codes: Vec<_> = gaps.iter().map(|g| g.code.as_str()).collect();
    assert!(
        codes.iter().any(|c| c.contains("unit")
            || c.contains("soft_stack")
            || c.contains("claim_obs")
            || c.contains("os_instance")
            || c.contains("capability")),
        "task-oriented gaps required: {codes:?}"
    );
    for g in &gaps {
        if g.code.contains("gpu_staircase") || g.code.contains("gpu_ns") {
            assert_eq!(g.severity, "low", "soft demotes GPU host dig: {:?}", g);
        }
    }

    let open = open_verification_gaps(&ev);
    assert!(open["open_count"].as_u64().unwrap_or(0) > 0);
    assert!(open
        .get("re_probe_priority")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty()));

    // evaluate surfaces open gaps on product/diagnostics
    let out = evaluate_session(&ev, None, None, None, true).expect("eval");
    let product = out.get("product").cloned().unwrap_or(out.clone());
    assert!(
        product.get("open_verification_gaps").is_some()
            || out
                .get("diagnostics")
                .and_then(|d| d.get("open_verification_gaps"))
                .is_some()
            || out.get("task_gaps").is_some(),
        "open gaps must be listable from analysis output"
    );
    assert!(
        product.get("mutual_verification").is_some()
            || product.get("analysis_quality").is_some()
            || out
                .get("diagnostics")
                .and_then(|d| d.get("mutual_verification"))
                .is_some(),
        "mutual verification / analysis quality required on product path"
    );
}

/// Full product path: field utilization + ops telemetry + four axes + sub-algorithms.
#[test]
fn evaluate_product_exposes_quality_and_ops() {
    let mut f = claim_obs_soft();
    f.as_object_mut()
        .unwrap()
        .insert("webdriver".into(), json!(true));
    f.as_object_mut()
        .unwrap()
        .insert("headless_likely".into(), json!(true));
    // This test exercises the full product path incl. RPA reasons; pin RPA plan
    // explicit so it does not depend on the signed-license default (free → stub).
    f.as_object_mut()
        .unwrap()
        .insert("rpa_analysis_enabled".into(), json!(true));
    let out = evaluate_session(
        &evidence(f, &["main", "gateway"], true),
        None,
        None,
        None,
        true,
    )
    .expect("eval");
    let product = out.get("product").cloned().unwrap_or(out.clone());
    assert!(product.get("device_id").is_some());
    assert!(product.get("os").is_some());
    assert!(product.get("br").is_some());
    assert!(product.get("rpa").is_some());
    assert!(product.get("sub_algorithms").is_some() || product.get("mutual_verification").is_some());
    let util = product
        .get("field_utilization")
        .or_else(|| {
            out.get("diagnostics")
                .and_then(|d| d.get("field_utilization"))
        })
        .cloned();
    assert!(util.is_some(), "field utilization on product path");
    let ops = product
        .get("ops_fusion_telemetry")
        .or_else(|| {
            out.get("diagnostics")
                .and_then(|d| d.get("ops_fusion_telemetry"))
        })
        .cloned()
        .unwrap_or(json!({}));
    assert!(ops.get("collision_risk").is_some() || product.get("collision_risk").is_some());
    // Control plane should hit rpa reasons
    let rpa_rs = product
        .pointer("/rpa/reasons")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join("|")
        })
        .unwrap_or_default();
    assert!(
        rpa_rs.contains("webdriver")
            || rpa_rs.contains("headless")
            || rpa_rs.contains("farm")
            || rpa_rs.contains("control"),
        "rpa control-plane reasons: {rpa_rs}"
    );
}

/// A2/A3/A5 integrity channels: weak contradicts surface, never hard device digest.
#[test]
fn integrity_channels_b8_sandbox_fe_weak_contradicts() {
    let mut fields = rich_fields();
    let fo = fields.as_object_mut().unwrap();
    // A2: FE claims B8 convergence, server observed zero edge evidence
    fo.insert("post_converge".into(), json!(true));
    fo.insert("b8_converge_without_edge".into(), json!(true));
    fo.insert("b8_edge_seen".into(), json!(false));
    // A3: sandbox residual algorithm diverges from main
    fo.insert("sandbox_residual_algo_match".into(), json!(false));
    // A5: FE bundle drifted mid-session + build epoch mismatch
    fo.insert("fe_code_version".into(), json!("7.0.0"));
    fo.insert("fe_version_changed".into(), json!(true));
    fo.insert("fe_build_mismatch".into(), json!(true));

    let stack = stack_auth_from_fields(&fields);
    let truth = TruthResult {
        xsrc_status: "ok".into(),
        real_band: "unknown".into(),
        credibility: 0.4,
        fe_only: true,
        has_server_side: false,
        has_main_core: true,
        packages: json!({}),
        reasons: vec![],
        details: json!({}),
    };
    let bot = BotScore {
        score: 10,
        verdict: "human".into(),
        flags: vec![],
        family: "none".into(),
        algo: "t".into(),
        details: json!({}),
        robot_name: None,
    };
    let os = gr_probe_core::score_os(&fields, &stack, &truth, &[]);
    let br = gr_probe_core::score_br(&fields, &stack, &bot, &truth, &[]);
    let rpa = gr_probe_core::score_rpa(&fields, &bot, Some("p"));
    let proj = commercial_projection(&fields);
    let device = json!({
        "device_id": proj.get("device_id").cloned().unwrap_or(json!(null)),
        "collision_risk": false,
    });
    let mv = build_mutual_verification(&fields, &stack, &os, &br, &rpa, &device, &truth, &[]);
    let contradicts = mv["contradicts"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let wanted = [
        ("channel_integrity", "b8_converge_without_edge"),
        ("sandbox_residual", "nest_algo_diverges_main"),
        ("fe_integrity", "fe_code_version_changed_mid_session"),
        ("fe_integrity", "fe_build_impl_epoch_mismatch"),
    ];
    for (ch, sig) in wanted {
        let hit = contradicts.iter().any(|c| {
            c.get("channel").and_then(|v| v.as_str()) == Some(ch)
                && c.get("signal").and_then(|v| v.as_str()) == Some(sig)
        });
        assert!(hit, "missing contradict channel {ch}/{sig}: {mv}");
        assert!(
            contradicts.iter().any(|c| {
                c.get("channel").and_then(|v| v.as_str()) == Some(ch)
                    && c.get("signal").and_then(|v| v.as_str()) == Some(sig)
                    && c.get("stance").and_then(|v| v.as_str()) == Some("contradict_weak")
                    && c.get("device_digest").and_then(|v| v.as_bool()) == Some(false)
            }),
            "{ch}/{sig} must be contradict_weak + never device digest: {mv}"
        );
    }
    assert!(
        mv["fusion_stance"].as_str().unwrap_or("") != "hard_conflict",
        "soft integrity contradicts must not escalate to hard_conflict: {mv}"
    );
}

/// A2/A3 confirm path: edge observed + matched sandbox algo → confirms, no weak contradicts.
#[test]
fn integrity_channels_confirm_when_consistent() {
    let mut fields = rich_fields();
    let fo = fields.as_object_mut().unwrap();
    fo.insert("b8_edge_seen".into(), json!(true));
    fo.insert("b8_converge_without_edge".into(), json!(false));
    fo.insert("sandbox_residual_algo_match".into(), json!(true));
    fo.insert("fe_code_version".into(), json!("7.0.0"));

    let stack = stack_auth_from_fields(&fields);
    let truth = TruthResult {
        xsrc_status: "ok".into(),
        real_band: "likely_real".into(),
        credibility: 0.8,
        fe_only: false,
        has_server_side: true,
        has_main_core: true,
        packages: json!({}),
        reasons: vec![],
        details: json!({}),
    };
    let bot = BotScore {
        score: 5,
        verdict: "human".into(),
        flags: vec![],
        family: "none".into(),
        algo: "t".into(),
        details: json!({}),
        robot_name: None,
    };
    let os = gr_probe_core::score_os(&fields, &stack, &truth, &[]);
    let br = gr_probe_core::score_br(&fields, &stack, &bot, &truth, &[]);
    let rpa = gr_probe_core::score_rpa(&fields, &bot, Some("p"));
    let proj = commercial_projection(&fields);
    let device = json!({
        "device_id": proj.get("device_id").cloned().unwrap_or(json!(null)),
        "collision_risk": false,
    });
    let mv = build_mutual_verification(&fields, &stack, &os, &br, &rpa, &device, &truth, &[]);
    let confirms = mv["confirms"].as_array().cloned().unwrap_or_default();
    assert!(
        confirms.iter().any(|c| {
            c.get("channel").and_then(|v| v.as_str()) == Some("channel_integrity")
                && c.get("signal").and_then(|v| v.as_str()) == Some("b8_edge_observed")
        }),
        "edge-observed confirm required: {mv}"
    );
    assert!(
        confirms.iter().any(|c| {
            c.get("channel").and_then(|v| v.as_str()) == Some("sandbox_residual")
                && c.get("signal").and_then(|v| v.as_str()) == Some("nest_algo_matches_main")
        }),
        "sandbox algo match confirm required: {mv}"
    );
    let contradicts = mv["contradicts"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        !contradicts.iter().any(|c| {
            matches!(
                c.get("channel").and_then(|v| v.as_str()),
                Some("channel_integrity") | Some("sandbox_residual") | Some("fe_integrity")
            )
        }),
        "consistent session must not produce integrity contradicts: {mv}"
    );
}

/// Utilization policy layer: group-level consistency channels surface in mutual
/// verification (soft only, never device digest), and the field-utilization map
/// registers policy fields that have no product-matrix row (nothing dropped).
#[test]
fn utilization_policy_channels_and_registration() {
    let mut fields = rich_fields();
    let fo = fields.as_object_mut().unwrap();
    // Policy-registered fields (spec/field_utilization_policy.json):
    fo.insert("canplay_av1".into(), json!(true));
    fo.insert("canplay_webm".into(), json!(false));
    fo.insert("canvas_w_emoji".into(), json!(11));
    fo.insert("canvas_w_cjk".into(), json!(13));
    fo.insert("canvas_w_base".into(), json!(17));
    fo.insert("window_screen_delta_h".into(), json!(8));
    fo.insert("pointer_fine".into(), json!(true));
    fo.insert("touch_event".into(), json!(true));
    fo.insert("pack_failed_n".into(), json!(1));
    fo.insert("webgpu_f16_ok".into(), json!(true));
    fo.insert("webgpu_compute_ok".into(), json!(true));

    // 1) Mutual verification surfaces policy channels as soft signals.
    let stack = stack_auth_from_fields(&fields);
    let truth = TruthResult {
        xsrc_status: "ok".into(),
        real_band: "likely_real".into(),
        credibility: 0.8,
        fe_only: false,
        has_server_side: true,
        has_main_core: true,
        packages: json!({}),
        reasons: vec![],
        details: json!({}),
    };
    let bot = BotScore {
        score: 5,
        verdict: "human".into(),
        flags: vec![],
        family: "none".into(),
        algo: "t".into(),
        details: json!({}),
        robot_name: None,
    };
    let os = gr_probe_core::score_os(&fields, &stack, &truth, &[]);
    let br = gr_probe_core::score_br(&fields, &stack, &bot, &truth, &[]);
    let rpa = gr_probe_core::score_rpa(&fields, &bot, Some("p"));
    let proj = commercial_projection(&fields);
    let device = json!({
        "device_id": proj.get("device_id").cloned().unwrap_or(json!(null)),
        "collision_risk": false,
    });
    let mv = build_mutual_verification(&fields, &stack, &os, &br, &rpa, &device, &truth, &[]);
    let contradicts = mv["contradicts"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let confirms = mv["confirms"].as_array().cloned().unwrap_or_default();
    for sig in [
        "av1_without_webm_incoherent",
        "pointer_fine_with_touch_stream",
        "pack_health_failed",
    ] {
        let hit = contradicts.iter().any(|c| {
            c.get("signal").and_then(|v| v.as_str()) == Some(sig)
                && c.get("stance").and_then(|v| v.as_str()) == Some("contradict_weak")
                && c.get("device_digest").and_then(|v| v.as_bool()) == Some(false)
        });
        assert!(hit, "policy contradict {sig} must be contradict_weak + no digest: {mv}");
    }
    for sig in ["webgpu_f16_compute_agree", "typographic_width_matrix"] {
        let hit = confirms.iter().any(|c| {
            c.get("signal").and_then(|v| v.as_str()) == Some(sig)
                && c.get("device_digest").and_then(|v| v.as_bool()) != Some(true)
        });
        assert!(hit, "policy confirm {sig} required: {mv}");
    }

    // 2) Field-utilization map registers policy fields explicitly.
    let util = build_field_utilization_map(&fields);
    assert!(
        util.get("policy_registered_present")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            >= 6,
        "policy fields registered into utilization map: {util}"
    );
    let by_axis = util.get("by_axis").cloned().unwrap_or(json!({}));
    let device_present = by_axis
        .pointer("/device_id/present")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(
        device_present
            .iter()
            .any(|v| v.get("field").and_then(|f| f.as_str()) == Some("canvas_w_emoji")),
        "canvas typography policy field visible on device axis: {device_present:?}"
    );
    assert!(
        util["utilization_policy"]["policy_present_by_channel"]["canvas_typography"]
            .as_u64()
            .unwrap_or(0)
            >= 3,
        "channel presence summary counts policy fields: {}",
        util["utilization_policy"]
    );
}

#[test]
fn b86_b91_policy_fields_registered_per_channel() {
    // iss/74: six new packs (B86-B91) — every emitted field must land in the
    // utilization policy map under its consumption channel, never dropped.
    let mut fields = rich_fields();
    let fo = fields.as_object_mut().unwrap();
    let inserts: Vec<(&str, serde_json::Value)> = vec![
        ("os_kernel_hint", json!("linux")),
        ("build_id_raw", json!("20200101000000")),
        ("linux_metric_font_arimo", json!(true)),
        ("engine_error_9set_digest", json!("aabbccddeeff")),
        (
            "engine_math_roundoff_profile",
            json!(["1.0", "0.5", "nan", "inf", "2.0", "0.25"]),
        ),
        ("engine_realm_ua_align_ok", json!(true)),
        ("engine_canvas_farbling_delta", json!(0)),
        ("wasm_bi_f32_mul_ns", json!(2.4)),
        ("wasm_bi_i64_div_ns", json!(7.8)),
        ("wasm_simd_f32x4_ns", json!(1.2)),
        ("wasm_scalar_vs_simd_ratio", json!(2.0)),
        ("wasm_throughput_digest", json!("1234567890ab")),
        ("audio_known_lock_sum", json!(0.42)),
        ("audio_channel_vs_copy_delta", json!(0.0)),
        ("audio_silent_osc_unique_bins", json!(1)),
        ("emoji_raster_hash", json!("1234567890ab")),
        ("emoji_tofu_count", json!(0)),
        ("mac_dot_font_measurable", json!(true)),
        ("font_fallback_chain_digest", json!("1234567890ab")),
        ("storage_quota_bytes", json!(268435456000u64)),
        ("storage_quota_grid_class", json!("128g_512g")),
        ("storage_quota_prev_match", json!(false)),
    ];
    for (k, v) in inserts {
        fo.insert(k.into(), v);
    }
    let util = build_field_utilization_map(&fields);
    let by_axis = util.get("by_axis").cloned().unwrap_or(json!({}));
    let os_present = by_axis
        .pointer("/os/present")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let dev_present = by_axis
        .pointer("/device_id/present")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    for f in [
        "os_kernel_hint",
        "wasm_bi_f32_mul_ns",
        "wasm_scalar_vs_simd_ratio",
        "audio_known_lock_sum",
        "storage_quota_grid_class",
        "engine_error_9set_digest",
    ] {
        assert!(
            os_present.iter().any(|v| v.get("field").and_then(|x| x.as_str()) == Some(f)),
            "iss/74 field {f} must be visible on os axis: {os_present:?}"
        );
    }
    let emoji_in = dev_present
        .iter()
        .any(|v| v.get("field").and_then(|x| x.as_str()) == Some("emoji_raster_hash"));
    assert!(emoji_in, "emoji raster (canvas_typography) on device axis: {dev_present:?}");
    let channels = util["utilization_policy"]["policy_present_by_channel"].clone();
    for (ch, min) in [
        ("env_fingerprint", 3u64),
        ("math_wasm", 6u64),
        ("media_codec", 4u64),
        ("canvas_typography", 3u64),
    ] {
        assert!(
            channels.get(ch).and_then(|v| v.as_u64()).unwrap_or(0) >= min,
            "iss/74 channel {ch} must count >= {min}: {channels}"
        );
    }
}

/// A1+A2 on the full product path: utilization registry v2 (level × recipe)
/// lands in analysis_quality; curve-morph summary + same-surface coherence
/// land in material_participation.
#[test]
fn a1_registry_v2_and_a2_curve_morph_on_product_path() {
    let mut f = rich_fields();
    let fo = f.as_object_mut().unwrap();
    // B86-B91 deep fields (L2/L3/L4 consumption levels)
    fo.insert("wasm_bi_f32_mul_ns".into(), json!(2.4));
    fo.insert("wasm_bi_i64_div_ns".into(), json!(7.8));
    fo.insert("wasm_simd_f32x4_ns".into(), json!(1.2));
    fo.insert("os_kernel_hint".into(), json!("linux"));
    fo.insert("engine_error_9set_digest".into(), json!("aabbccddeeff"));
    fo.insert("storage_quota_grid_class".into(), json!("128g_512g"));
    fo.insert("audio_known_lock_sum".into(), json!(0.42));
    // Same-surface deep audio curve (morph-coherent with hw_curve_audio)
    let hw_audio: Vec<f64> = (0..32).map(|i| 0.15 + i as f64 * 0.001).collect();
    let deep_audio: Vec<f64> = hw_audio.iter().map(|x| x * 1.005).collect();
    fo.insert("hw_curve_audio".into(), json!(hw_audio));
    fo.insert("audio_deep_curve".into(), json!(deep_audio));

    let out = evaluate_session(&evidence(f, &["main", "gateway"], true), None, None, None, true)
        .expect("eval");
    let product = out.get("product").cloned().unwrap_or(out.clone());
    let aq = product
        .get("analysis_quality")
        .or_else(|| out.get("diagnostics").and_then(|d| d.get("analysis_quality")))
        .cloned()
        .unwrap_or(json!({}));
    // A1: utilization_levels with L4 (wasm) + recipes
    let levels = aq.get("utilization_levels").cloned().unwrap_or(json!({}));
    assert!(
        levels
            .get("present_by_level")
            .and_then(|v| v.get("L4"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            >= 3,
        "wasm deep fields registered at L4: {levels}"
    );
    let recipes = levels
        .get("recipes_present")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    for r in ["wasm_throughput_kernel", "os_engine_gecko_surface", "residual_stats"] {
        assert!(recipes.iter().any(|x| x.as_str() == Some(r)), "recipe {r}: {recipes:?}");
    }
    // A2: curve_morph slots + same-surface coherence on material participation
    let mp = product
        .get("material_participation")
        .or_else(|| out.get("diagnostics").and_then(|d| d.get("material_participation")))
        .cloned()
        .unwrap_or(json!({}));
    let morph = mp.get("curve_morph").cloned().unwrap_or(json!({}));
    assert!(
        morph
            .get("morphs")
            .and_then(|v| v.get("curve_morph_webgl"))
            .is_some(),
        "webgl morph slot missing: {morph}"
    );
    assert!(
        morph
            .get("morphs")
            .and_then(|v| v.get("curve_morph_audio"))
            .is_some(),
        "audio morph slot missing: {morph}"
    );
    assert_eq!(
        morph.pointer("/coherence/audio_deep_vs_hw").and_then(|v| v.as_str()),
        Some("agree"),
        "deep vs shallow audio curves must agree on shape buckets: {morph}"
    );
    assert_eq!(
        mp.get("audio_morph_coherent").and_then(|v| v.as_bool()),
        Some(true),
        "morph coherence flag: {mp}"
    );
}
