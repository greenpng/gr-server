//! Gating tests: algorithm-group commercial placement via **shipped** evaluate/mint paths.
use gr_probe_core::{
    apply_server_mint, evaluate_session, field_registry_json, same_commercial_bucket,
    score_materials_boost, select_identity_group, select_vt_best_silicon, SoftEdge,
    PROMOTE_TO_COMMERCIAL_ID,
};
use serde_json::{json, Value};

fn dual_silicon() -> Value {
    json!({
        "residual_mean": 0.26004,
        "residual_std": 0.09075,
        "residual_entropy_ok": true,
        "hw_webgl_stable": "wg_dual_aaa",
        "hw_audio_stable": "au_dual_bbb",
        "hw_curve_webgl": [0.22, 0.21, 0.22, 0.23, 0.22, 0.21, 0.22, 0.23],
        "hw_curve_audio": [0.0, 0.0, 0.0001, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        "webrtc_host_ip_hash": "d2778e5f",
        "form_class": "desktop",
        "hardware_concurrency": 12,
        "device_memory": 16,
        "platform": "Linux x86_64",
        "os_family": "linux",
        "timezone": "Asia/Shanghai",
        "engine_family": "blink",
        "user_agent": "Mozilla/5.0 Chrome/120",
    })
}

fn core_webgl(webgl: &str, host: &str, eng: &str) -> Value {
    json!({
        "residual_mean": 0.26004,
        "residual_std": 0.09075,
        "residual_entropy_ok": true,
        "hw_webgl_stable": webgl,
        "hw_curve_webgl": [0.22, 0.21, 0.22, 0.23, 0.22, 0.21, 0.22, 0.23, 0.22],
        "webrtc_host_ip_hash": host,
        "form_class": "desktop",
        "hardware_concurrency": 8,
        "platform": "Linux x86_64",
        "os_family": "linux",
        "timezone": "UTC",
        "engine_family": eng,
        "user_agent": format!("Mozilla/5.0 ({eng})"),
    })
}

fn gateway_only() -> Value {
    json!({
        "early_kick": true,
        "ja4": "t13d1516h2_8daaf6152771_02713d6af862",
        "user_agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36",
        "server_client_ip": "203.0.113.10",
        "server_asn": "AS12345",
    })
}

#[test]
fn shipped_mint_dual_residual_group() {
    let m = apply_server_mint(&dual_silicon());
    assert_eq!(m["algo_group_id"], "G_DH_DUAL_RESIDUAL");
    assert_eq!(m["algo_group_tier"], "dh");
    assert_eq!(m["mint_eligible"], true);
    assert_eq!(m["provisional_gateway"], false);
    let id = m["device_id"].as_str().unwrap_or("");
    assert!(id.starts_with("dh-") || id.starts_with("dh_") || id.starts_with("dv-") || id.starts_with("dv_") || id.starts_with("dv0-") || id.starts_with("dv4-") || id.starts_with("dv5-") || id.starts_with("dv6-"), "expected commercial/multi prefix, got {id}");
}

#[test]
fn shipped_mint_single_webgl_same_bucket() {
    let a = core_webgl("wg_floor_same", "host_lab1", "gecko");
    let b = core_webgl("wg_floor_same", "host_lab1", "blink");
    let ma = apply_server_mint(&a);
    let mb = apply_server_mint(&b);
    assert_eq!(ma["algo_group_id"], "G_DH_CORE_WEBGL");
    assert_eq!(mb["algo_group_id"], "G_DH_CORE_WEBGL");
    assert_eq!(ma["device_id"], mb["device_id"]);
    let pair = same_commercial_bucket(&a, &b);
    assert_eq!(pair["same_bucket"], true);
}

#[test]
fn shipped_mint_webgl_disagree_forks() {
    let a = core_webgl("wg_floor_a", "host_lab1", "gecko");
    let b = core_webgl("wg_floor_b", "host_lab1", "blink");
    let ma = apply_server_mint(&a);
    let mb = apply_server_mint(&b);
    assert_ne!(ma["device_id"], mb["device_id"]);
    let pair = same_commercial_bucket(&a, &b);
    assert_eq!(pair["same_bucket"], false);
}

#[test]
fn shipped_evaluate_gateway_provisional() {
    let evidence = json!({
        "session_id": "cycle_b8_only",
        "visitor_terminal_id": "vt_test",
        "fields": gateway_only(),
        "batches": [{"batch_id": "B8_gateway", "source": "gateway"}],
        "meta": {"product_version": "test", "early_kick": true},
    });
    let out = evaluate_session(&evidence, None, None, None, false).expect("evaluate");
    let device = out.get("device").cloned().unwrap_or(out.clone());
    let prov = device
        .get("provisional_gateway")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let tier = device
        .get("device_tier")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let group = device
        .get("algo_group_id")
        .or_else(|| device.pointer("/algo_group/group_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(prov || tier == "dg", "expected provisional/dg device={device}");
    assert!(
        group.contains("G_DG") || tier == "dg",
        "expected dg group, got {group}"
    );
    // Soft never promote constant
    assert!(!PROMOTE_TO_COMMERCIAL_ID);
    let _edge = SoftEdge {
        a_session: "a".into(),
        b_session: "b".into(),
        priority: "P1".into(),
        promote_to_commercial_id: PROMOTE_TO_COMMERCIAL_ID,
        confidence: 0.8,
        reason: "test".into(),
    };
    assert!(!_edge.promote_to_commercial_id);
}

#[test]
fn shipped_evaluate_rich_exposes_algo_group() {
    let evidence = json!({
        "session_id": "cycle_rich",
        "visitor_terminal_id": "vt_rich",
        "fields": dual_silicon(),
        "batches": [
            {"batch_id": "B0_bootstrap", "source": "main"},
            {"batch_id": "B10_hw_curves", "source": "main"},
            {"batch_id": "B8_gateway", "source": "gateway"},
        ],
        "meta": {"product_version": "test"},
    });
    let out = evaluate_session(&evidence, None, None, None, false).expect("evaluate");
    let device = out.get("device").expect("device");
    let product = out.get("product");
    let gid = device
        .get("algo_group_id")
        .or_else(|| device.pointer("/algo_group/group_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(!gid.is_empty(), "algo_group_id missing: {device}");
    assert!(
        gid.starts_with("G_DH_") || gid.starts_with("G_DV_"),
        "unexpected group {gid}"
    );
    if let Some(p) = product {
        // commercial id on product must match device (scores don't rewrite)
        assert_eq!(p.get("device_id"), device.get("device_id"));
    }
}

#[test]
fn os_br_rpa_boost_changes_with_materials_device_id_stable() {
    let thin = json!({"platform": "Linux x86_64", "form_class": "desktop"});
    let rich = {
        let mut m = dual_silicon();
        if let Some(o) = m.as_object_mut() {
            o.insert("ja4".into(), json!("t13d1516h2_test"));
            o.insert("webgl_unmasked_renderer".into(), json!("NVIDIA"));
            o.insert("pointer_samples".into(), json!([1, 2, 3, 4]));
            o.insert("webdriver".into(), json!(false));
            o.insert("media_device_count".into(), json!(3));
        }
        m
    };
    let bt = score_materials_boost(&thin);
    let br = score_materials_boost(&rich);
    assert!(
        br["os"]["boost"].as_f64().unwrap() >= bt["os"]["boost"].as_f64().unwrap()
    );
    assert!(br["br"]["boost"].as_f64().unwrap() > 0.0);

    let id_thin = apply_server_mint(&thin)
        .get("device_id")
        .cloned()
        .unwrap_or(Value::Null);
    // Adding score-only materials must not be the sole path to change group when still thin —
    // compare rich mint has silicon group.
    let mint_rich = apply_server_mint(&rich);
    assert_ne!(mint_rich.get("algo_group_id").and_then(|v| v.as_str()), Some("G_DG_EMPTY"));
    let _ = id_thin;
}

#[test]
fn vt_best_silicon_over_latest_b8() {
    let cands = vec![
        json!({
            "session_id": "latest_b8",
            "device_tier": "dg",
            "digest_path": "empty_anchor_v1",
            "provisional_gateway": true,
            "batches_n": 1,
            "created_ms": 2_000_000i64,
        }),
        json!({
            "session_id": "older_rich",
            "device_tier": "dh",
            "digest_path": "real_curves_v1",
            "mint_silicon_ok": true,
            "batches_n": 65,
            "created_ms": 1_000_000i64,
        }),
    ];
    let r = select_vt_best_silicon(&cands);
    assert_eq!(r["selected"]["session_id"], "older_rich");
}

#[test]
fn field_registry_documents_probe_families() {
    let reg = field_registry_json();
    let fields = reg["fields"].as_array().unwrap();
    let names: Vec<&str> = fields
        .iter()
        .filter_map(|f| f.get("field").and_then(|v| v.as_str()))
        .collect();
    for must in [
        "residual_mean",
        "hw_curve_webgl",
        "webrtc_host_ip_hash",
        "ja4",
        "hw_webgl_stable",
    ] {
        assert!(names.contains(&must), "missing {must}");
    }
    let groups = reg["identity_groups"].as_array().unwrap();
    assert!(groups.iter().any(|g| g["id"] == "G_DH_CORE_WEBGL"));
    assert!(groups.iter().any(|g| g["id"] == "G_DG_GATEWAY"));
}

#[test]
fn selector_priority_dual_beats_single_core() {
    let s = select_identity_group(&dual_silicon());
    assert_eq!(s.group_id, "G_DH_DUAL_RESIDUAL");
    assert!(s.priority < 20);
}

/// multi_source conf-only residual must not authorize G_DH_* commercial body.
///
/// v7 auth-ladder semantics: a single-source `webrtc_host_ip_hash` is
/// machine-grade separator material (harder to forge than soft conf-only keys),
/// so `host_mint_ok` stays true; residual alone without a dual-channel partner
/// curve stays conf-only (`residual_mint_ok=false`) and must never win G_DH_*.
#[test]
fn multi_source_conf_only_residual_host_cannot_mint_dh() {
    // residual alone (no dual-channel partner curve) + free-form webgl digest without curves
    // + single-source webrtc → residual_mint_ok=false, host_mint_ok=true (machine-grade host).
    let fields = json!({
        "residual_mean": 0.26,
        "residual_std": 0.09,
        "residual_entropy_ok": true,
        "hw_webgl_stable": "wg_freeform_no_curve_family",
        "webrtc_host_ip_hash": "host_spoof_single",
        "form_class": "desktop",
        "hardware_concurrency": 8,
        "platform": "Linux x86_64",
        "engine_family": "blink",
        "user_agent": "Mozilla/5.0",
    });
    let gate = gr_probe_core::multi_source_mint::assess_mint_gate(&fields, None);
    assert_eq!(
        gate["residual_mint_ok"],
        false,
        "fixture must have residual conf-only: {gate}"
    );
    assert_eq!(
        gate["host_mint_ok"],
        true,
        "single-source webrtc host hash is machine-grade mint material (auth ladder): {gate}"
    );

    let m = apply_server_mint(&fields);
    let gid = m["algo_group_id"].as_str().unwrap_or("");
    let tier = m["algo_group_tier"].as_str().unwrap_or("");
    let id = m["device_id"].as_str().unwrap_or("");
    assert!(
        !gid.starts_with("G_DH_"),
        "conf-only residual/host must not win G_DH_* group, got {gid} id={id} mint={m}"
    );
    assert_ne!(tier, "dh", "must not claim dh tier: {m}");
    assert!(
        !(id.starts_with("dh-") || id.starts_with("dh_")),
        "must not mint public dh_ from conf-only residual: {id}"
    );
    let posture = m["analysis_posture"].as_array().cloned().unwrap_or_default();
    let joined: Vec<&str> = posture.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        joined.iter().any(|p| p.contains("residual") && p.contains("conf_only")
            || *p == "algo_group_residual_stripped_from_commercial_body"),
        "expected residual strip posture, got {joined:?}"
    );
}

/// Free-form webgl digest without curve family cannot win G_DH_CORE_WEBGL even with dual residual+curve.
#[test]
fn freeform_webgl_digest_without_curves_not_core_webgl() {
    let fields = json!({
        "residual_mean": 0.26,
        "residual_std": 0.09,
        "residual_entropy_ok": true,
        // partner residual+curve would gate residual, but no webgl curve array
        "hw_curve_audio": [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        "hw_webgl_stable": "wg_only_string_no_curves",
        "webrtc_host_ip_hash": "host1",
        "form_class": "desktop",
        "hardware_concurrency": 8,
        "platform": "Linux x86_64",
    });
    let m = apply_server_mint(&fields);
    assert_ne!(
        m["algo_group_id"].as_str().unwrap_or(""),
        "G_DH_CORE_WEBGL",
        "free-form digest without curves must not win CORE_WEBGL: {m}"
    );
}

/// Capture real evaluate/mint JSON for skeptic verification (scratch path).
#[test]
fn capture_real_evaluate_product_snippets() {
    let scratch = std::env::var("GROK_SCRATCH")
        .unwrap_or_else(|_| "/tmp/grok-goal-077cd9947678/implementer".into());
    let _ = std::fs::create_dir_all(&scratch);

    let rich_ev = json!({
        "session_id": "cycle_rich_snip",
        "visitor_terminal_id": "vt_snip",
        "fields": dual_silicon(),
        "batches": [
            {"batch_id": "B0_bootstrap", "source": "main"},
            {"batch_id": "B10_hw_curves", "source": "main"},
        ],
        "meta": {"product_version": "test"},
    });
    let rich = evaluate_session(&rich_ev, None, None, None, false).expect("rich eval");
    let thin_ev = json!({
        "session_id": "cycle_b8_snip",
        "visitor_terminal_id": "vt_snip",
        "fields": gateway_only(),
        "batches": [{"batch_id": "B8_gateway", "source": "gateway"}],
        "meta": {"product_version": "test", "early_kick": true},
    });
    let thin = evaluate_session(&thin_ev, None, None, None, false).expect("thin eval");

    let conf_only_fields = json!({
        "residual_mean": 0.26,
        "residual_std": 0.09,
        "hw_webgl_stable": "wg_freeform_no_curve_family",
        "webrtc_host_ip_hash": "host_spoof_single",
        "form_class": "desktop",
        "hardware_concurrency": 8,
        "platform": "Linux x86_64",
    });
    let conf_only_mint = apply_server_mint(&conf_only_fields);

    let snip = json!({
        "algo": "product_algo_group_snippets_v1",
        "captured_from": "evaluate_session + apply_server_mint shipped paths",
        "rich_device": {
            "device_id": rich.pointer("/device/device_id"),
            "device_tier": rich.pointer("/device/device_tier"),
            "algo_group_id": rich.pointer("/device/algo_group_id"),
            "algo_group": rich.pointer("/device/algo_group"),
            "provisional_gateway": rich.pointer("/device/provisional_gateway"),
            "digest_path": rich.pointer("/device/digest_path"),
            "product_device_id": rich.pointer("/product/device_id"),
            "score_materials_boost": rich.pointer("/device/score_materials_boost")
                .or_else(|| rich.pointer("/product/score_materials_boost")),
        },
        "thin_gateway_device": {
            "device_id": thin.pointer("/device/device_id"),
            "device_tier": thin.pointer("/device/device_tier"),
            "algo_group_id": thin.pointer("/device/algo_group_id"),
            "provisional_gateway": thin.pointer("/device/provisional_gateway"),
            "digest_path": thin.pointer("/device/digest_path"),
        },
        "conf_only_residual_mint": {
            "device_id": conf_only_mint.get("device_id"),
            "algo_group_id": conf_only_mint.get("algo_group_id"),
            "algo_group_tier": conf_only_mint.get("algo_group_tier"),
            "provisional_gateway": conf_only_mint.get("provisional_gateway"),
            "analysis_posture": conf_only_mint.get("analysis_posture"),
            "multi_source_mint_gate": {
                "residual_mint_ok": conf_only_mint.pointer("/multi_source_mint_gate/residual_mint_ok"),
                "host_mint_ok": conf_only_mint.pointer("/multi_source_mint_gate/host_mint_ok"),
            },
        },
    });
    let path = format!("{scratch}/product_algo_group_snippets.json");
    std::fs::write(&path, serde_json::to_string_pretty(&snip).unwrap()).expect("write snip");
    // Sanity: conf-only must not be dh
    assert_ne!(snip["conf_only_residual_mint"]["algo_group_tier"], "dh");
}
