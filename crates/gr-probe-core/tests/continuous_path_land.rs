//! Continuous path (post mid-term): antidetect/emulator claim-obs, UA-merge ban,
//! profile-boundary KPI, tenant pepper index, ladder conf adopt.

use gr_probe_core::{
    adopt_ladder_calibration, association_ladder, commercial_projection, evaluate_session,
    link_or_mint_pair, report_profile_boundary_kpi, score_br, score_os, score_rpa,
    stack_auth_from_fields, FileDeviceIndex, LabeledPair, BotScore, TruthResult,
    RUNTIME_CONFIDENCE_VERSION, CALIBRATED_CONFIDENCE_VERSION,
};
use serde_json::{json, Value};

fn curves() -> (Vec<f64>, Vec<f64>) {
    let a: Vec<f64> = (0..32)
        .map(|i| (i as f64 * 0.11).sin().abs() * 0.5 + 0.05)
        .collect();
    let w: Vec<f64> = (0..16)
        .map(|i| ((i * 13) % 200) as f64 / 200.0 + 0.01)
        .collect();
    (a, w)
}

fn base_fields(residual: f64, unit: &str) -> Value {
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
        "unit_surface_id": unit,
        "unit_surface_algo": "gr_unit_v1",
        "unit_multiround_stable": true,
        "screen_width": 1280,
        "screen_height": 720,
    })
}

/// X-8/X-9: antidetect/emulator move os/br (rpa if control); not residual-led id; association ≤ env.
#[test]
fn antidetect_emulator_claim_obs_not_digest() {
    let mut fields = base_fields(0.500423, "u_anti");
    {
        let o = fields.as_object_mut().unwrap();
        o.insert("residual_soft_like".into(), json!(true));
        o.insert("soft_stack".into(), json!(true));
        o.insert("webgl_unmasked_renderer".into(), json!("NVIDIA GeForce RTX 4090"));
        o.insert("antidetect_vendor_hint".into(), json!(true));
        o.insert("fingerprint_vendor_lie".into(), json!(true));
        o.insert("prototype_chain_tamper".into(), json!(true));
        o.insert("emulator_hint".into(), json!("android_emu"));
        o.insert("stack_class".into(), json!("emulator"));
        o.insert("webdriver".into(), json!(true));
        o.insert("os_instance_hash".into(), json!("emu_os_1"));
    }
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
        score: 20,
        verdict: "human".into(),
        flags: vec![],
        family: "none".into(),
        algo: "t".into(),
        details: json!({}),
        robot_name: None,
    };
    let os = score_os(&fields, &stack, &truth, &[]);
    let br = score_br(&fields, &stack, &bot, &truth, &[]);
    let rpa = score_rpa(&fields, &bot, Some("p"));
    let os_j: String = os["reasons"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str())
        .collect::<Vec<_>>()
        .join("|");
    let br_j: String = br["reasons"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str())
        .collect::<Vec<_>>()
        .join("|");
    let rpa_j: String = rpa["reasons"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str())
        .collect::<Vec<_>>()
        .join("|");
    assert!(
        os_j.contains("antidetect") || os_j.contains("emulator") || os_j.contains("soft"),
        "os claim-obs: {os_j}"
    );
    assert!(
        br_j.contains("antidetect") || br_j.contains("integrity") || br_j.contains("prototype"),
        "br integrity: {br_j}"
    );
    assert!(
        rpa_j.contains("webdriver") || rpa_j.contains("emulator") || rpa_j.contains("farm"),
        "rpa control: {rpa_j}"
    );

    let lad = association_ladder(&fields, Some(&json!({"device_tier":"dv"})));
    assert_ne!(lad["association_level"], "hardware");
    assert!(
        matches!(
            lad["association_level"].as_str(),
            Some("env") | Some("profile") | Some("gateway")
        ),
        "emulator/soft ≤ env: {lad}"
    );

    // Residual-led: same residual with/without antidetect flags → same commercial id body path
    let mut clean = fields.clone();
    clean.as_object_mut().unwrap().remove("antidetect_vendor_hint");
    clean.as_object_mut().unwrap().remove("fingerprint_vendor_lie");
    clean.as_object_mut().unwrap().remove("prototype_chain_tamper");
    let id_a = commercial_projection(&fields)
        .get("device_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let id_b = commercial_projection(&clean)
        .get("device_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    // Both soft same residual — antidetect flags must not redefine digest materials
    assert!(!id_a.is_empty() && !id_b.is_empty());
    assert_eq!(
        id_a, id_b,
        "antidetect flags must not split residual-led commercial id: {id_a} vs {id_b}"
    );
}

/// X-11: same UA / L0 only, different residual → no auto-link.
#[test]
fn link_rejects_ua_l0_only_without_residual_match() {
    let mut a = base_fields(0.500111, "unit_a");
    let mut b = base_fields(0.500999, "unit_b");
    {
        let oa = a.as_object_mut().unwrap();
        oa.insert(
            "user_agent".into(),
            json!("Mozilla/5.0 (X11; Linux) Chrome/120.0.0.0"),
        );
        oa.insert(
            "webgl_unmasked_renderer".into(),
            json!("ANGLE (NVIDIA, GeForce GTX 1050 Ti)"),
        );
        oa.insert("residual_soft_like".into(), json!(false));
    }
    {
        let ob = b.as_object_mut().unwrap();
        ob.insert(
            "user_agent".into(),
            json!("Mozilla/5.0 (X11; Linux) Chrome/120.0.0.0"),
        );
        ob.insert(
            "webgl_unmasked_renderer".into(),
            json!("ANGLE (NVIDIA, GeForce GTX 1050 Ti)"),
        );
        ob.insert("residual_soft_like".into(), json!(false));
    }
    let pair = link_or_mint_pair(&a, &b);
    // residual_mean 0.500111 vs 0.500999 can share commercial residual class after
    // floor quanta (rm_0.500) — same_server_mint_id may be true. Product contract:
    // UA/L0 alone must not create **public** commercial same-device hard link.
    assert_eq!(
        pair["public_device_link"].as_bool().unwrap_or(true),
        false,
        "different unit surfaces / residual micro-diff must not public-link: {pair}"
    );
    assert!(
        pair["score"].as_f64().unwrap_or(1.0) < 0.95
            || pair["action"]
                .as_str()
                .unwrap_or("")
                .contains("soft_associate")
            || pair["action"].as_str().unwrap_or("").contains("mint"),
        "must not exclusive hard-link on UA/L0 alone: {pair}"
    );

    // Soft never hardware (X-17)
    let mut soft = base_fields(0.500423, "u_soft");
    soft.as_object_mut()
        .unwrap()
        .insert("soft_stack".into(), json!(true));
    soft.as_object_mut()
        .unwrap()
        .insert("residual_soft_like".into(), json!(true));
    soft.as_object_mut()
        .unwrap()
        .insert("os_instance_hash".into(), json!("os1"));
    let lad = association_ladder(&soft, Some(&json!({"device_tier":"dv"})));
    assert_ne!(lad["association_level"], "hardware");
}

/// X-20 profile boundary: same config may share; per-profile noise splits.
#[test]
fn profile_boundary_kpi_same_config_vs_noise() {
    let same: Vec<Value> = (0..3)
        .map(|i| {
            let mut f = base_fields(0.500423, "unit_same_cfg");
            let o = f.as_object_mut().unwrap();
            o.insert("soft_stack".into(), json!(true));
            o.insert("residual_soft_like".into(), json!(true));
            o.insert("pool_label".into(), json!(format!("same_{i}")));
            // no os separator → class collide
            f
        })
        .collect();
    let noise: Vec<Value> = (0..3)
        .map(|i| {
            let mut f = base_fields(0.500100 + i as f64 * 0.0002, &format!("unit_prof_{i}"));
            let o = f.as_object_mut().unwrap();
            o.insert("soft_stack".into(), json!(true));
            o.insert("residual_soft_like".into(), json!(true));
            o.insert("os_instance_hash".into(), json!(format!("os_prof_{i}")));
            o.insert("pool_label".into(), json!(format!("noise_{i}")));
            f
        })
        .collect();
    let rep = report_profile_boundary_kpi(&same, &noise);
    assert_eq!(rep["ok"], true);
    assert_eq!(rep["sla"]["global_unique_promised"], false);
    assert!(
        rep["boundary"]["pass"].as_bool().unwrap_or(false)
            || (rep["boundary"]["same_config_may_share_class_id"]
                .as_bool()
                .unwrap_or(false)
                && rep["boundary"]["per_profile_must_split"]
                    .as_bool()
                    .unwrap_or(false)),
        "profile boundary matrix: {rep}"
    );
}

/// Tenant pepper: different tenants → different commercial ids; same tenant link.
#[test]
fn tenant_pepper_index_isolates_and_reopens() {
    let dir = std::env::temp_dir().join(format!("gr_pepper_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path_a = dir.join("tenant_a.json");
    let path_b = dir.join("tenant_b.json");

    let mut fields = base_fields(0.500423, "unit_t");
    {
        let o = fields.as_object_mut().unwrap();
        o.insert("soft_stack".into(), json!(true));
        o.insert("residual_soft_like".into(), json!(true));
        o.insert("os_instance_hash".into(), json!("os_shared"));
    }

    let id_a = {
        let mut idx = FileDeviceIndex::open_for_tenant(&path_a, "tenant_alpha").unwrap();
        let r = idx.link_or_mint(&fields).unwrap();
        assert_eq!(r["tenant_id"], "tenant_alpha");
        r["device_id"].as_str().unwrap().to_string()
    };
    let id_b = {
        let mut idx = FileDeviceIndex::open_for_tenant(&path_b, "tenant_beta").unwrap();
        let r = idx.link_or_mint(&fields).unwrap();
        assert_eq!(r["tenant_id"], "tenant_beta");
        r["device_id"].as_str().unwrap().to_string()
    };
    assert_ne!(
        id_a, id_b,
        "tenant pepper must namespace commercial ids: {id_a} vs {id_b}"
    );

    // Same tenant reopen → link (FS v2 resolver reports link_fs; legacy
    // in-memory-only path reports link — both are valid link actions).
    {
        let mut idx = FileDeviceIndex::open_for_tenant(&path_a, "tenant_alpha").unwrap();
        let r2 = idx.link_or_mint(&fields).unwrap();
        let act = r2["action"].as_str().unwrap_or("");
        assert!(
            act == "link" || act == "link_fs",
            "same-tenant reopen must link, got action={act}: {r2}"
        );
        assert_eq!(r2["device_id"].as_str().unwrap(), id_a);
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// Ladder conf adopt: empty refuse; labeled adopt explicit; runtime stays heuristic.
#[test]
fn ladder_conf_adopt_explicit_only() {
    let refuse = adopt_ladder_calibration(&[]);
    assert_eq!(refuse["adopted"], false);
    assert_eq!(refuse["runtime_confidence_version"], RUNTIME_CONFIDENCE_VERSION);
    assert_eq!(refuse["silent_relabel_forbidden"], true);

    let pairs = vec![
        LabeledPair {
            same_host: true,
            score: 0.9,
            label_source: "lab".into(),
            association_level: Some("hardware".into()),
        },
        LabeledPair {
            same_host: false,
            score: 0.2,
            label_source: "lab".into(),
            association_level: Some("env".into()),
        },
    ];
    let ad = adopt_ladder_calibration(&pairs);
    assert_eq!(ad["adopted"], true);
    assert_eq!(
        ad["runtime_confidence_version_unchanged"],
        RUNTIME_CONFIDENCE_VERSION
    );
    assert_eq!(
        ad["confidence_version_if_adopted"],
        CALIBRATED_CONFIDENCE_VERSION
    );
    assert!(ad["report"]["ok"].as_bool().unwrap_or(false));
}

/// Product path surfaces continuous redlines + evaluate antidetect.
#[test]
fn evaluate_continuous_product_surface() {
    let mut fields = base_fields(0.500423, "u_ev");
    {
        let o = fields.as_object_mut().unwrap();
        o.insert("soft_stack".into(), json!(true));
        o.insert("residual_soft_like".into(), json!(true));
        o.insert("antidetect_vendor_hint".into(), json!("camoufox"));
        o.insert("emulator_hint".into(), json!(false));
        o.insert("os_instance_hash".into(), json!("os_e"));
        o.insert("webgl_unmasked_renderer".into(), json!("SwiftShader"));
    }
    let out = evaluate_session(
        &json!({
            "sources":["main"],
            "batches":[{"batch_id":"B10_hw_curves","source":"main"}],
            "fields": fields,
            "session_id":"c",
        }),
        None,
        None,
        None,
        true,
    )
    .expect("eval");
    let product = out.get("product").cloned().unwrap_or(out.clone());
    assert_ne!(
        product
            .get("association_level")
            .or_else(|| out.pointer("/device/association_level"))
            .and_then(|v| v.as_str()),
        Some("hardware")
    );
    assert_eq!(
        product
            .pointer("/product_redlines/no_pure_fe_global_uv_promise")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    let os_rs = product
        .pointer("/os/reasons")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join("|")
        })
        .unwrap_or_default();
    assert!(
        os_rs.contains("antidetect") || os_rs.contains("soft") || os_rs.contains("claim"),
        "os reasons: {os_rs}"
    );
}
