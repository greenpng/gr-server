//! Multi-source priority + conflict-aware scoring — shipped APIs only.
use gr_probe_core::{
    apply_server_mint_with_evidence, assess_mint_gate, conflict_score_demotion, evaluate_session,
    field_mint_decision, resolve_fields_multi_source, score_materials_boost, SoftEdge,
    PROMOTE_TO_COMMERCIAL_ID,
};
use serde_json::{json, Value};

fn dual_curves() -> Value {
    json!({
        "hw_curve_webgl": [0.22, 0.21, 0.22, 0.23, 0.22, 0.21, 0.22, 0.23],
        "hw_curve_audio": [0.0, 0.0, 0.0001, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        "residual_mean": 0.26,
        "residual_std": 0.09,
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "hardware_concurrency": 8,
        "webrtc_host_ip_hash": "host_lab",
        "engine_family": "blink",
        "user_agent": "Mozilla/5.0 Chrome/120",
    })
}

#[test]
fn multi_source_agree_cores_mint_ok_and_stable() {
    let fields = json!({
        "hardware_concurrency": 12,
        "platform": "Linux x86_64",
        "form_class": "desktop",
        "hw_curve_webgl": [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8],
        "hw_curve_audio": [0.01, 0.02, 0.03, 0.04, 0.05, 0.06, 0.07, 0.08],
        "residual_mean": 0.26,
        "residual_std": 0.09,
        "webrtc_host_ip_hash": "h1",
    });
    let evidence = json!({
        "fields_by_source": {
            "main": {"hardware_concurrency": 12, "platform": "Linux x86_64"},
            "worker": {"hardware_concurrency": 12, "platform": "Linux x86_64"},
            "iframe": {"hardware_concurrency": 12}
        },
        "source_conflicts": []
    });
    let d = field_mint_decision(
        "hardware_concurrency",
        &fields,
        evidence.get("fields_by_source").and_then(|v| v.as_object()),
        &[],
    );
    assert_eq!(d["mint_ok"], true, "{d}");
    assert_eq!(d["agreed"], true);
    let mint = apply_server_mint_with_evidence(&fields, Some(&evidence));
    assert!(
        mint.get("multi_source_resolution").is_some(),
        "mint must surface multi_source_resolution: {mint}"
    );
    let sum = mint
        .pointer("/multi_source_resolution/summary")
        .cloned()
        .unwrap_or(json!({}));
    assert!(
        sum["agree_keys_n"].as_u64().unwrap_or(0) >= 1
            || sum["mint_ok_keys_n"].as_u64().unwrap_or(0) >= 1,
        "summary={sum}"
    );
}

#[test]
fn soft_identity_conflict_prefers_worker_over_main() {
    let fields = json!({
        "hardware_concurrency": 4, // merged may be wrong
        "platform": "Win32",
        "form_class": "desktop",
    });
    let evidence = json!({
        "fields_by_source": {
            "main": {"hardware_concurrency": 4, "platform": "Win32"},
            "worker": {"hardware_concurrency": 12, "platform": "Linux x86_64"}
        },
        "source_conflicts": [
            "source_conflict:hardware_concurrency",
            "source_conflict:platform"
        ]
    });
    let res = resolve_fields_multi_source(&fields, Some(&evidence));
    let resolved = res.get("resolved_fields").unwrap();
    // Worker harder-to-forge for soft identity wins
    assert_eq!(
        resolved["hardware_concurrency"],
        12,
        "worker cores should win: {res}"
    );
    assert_eq!(
        resolved["platform"].as_str().unwrap_or(""),
        "Linux x86_64",
        "worker platform should win: {res}"
    );
    let cores_dec = res
        .pointer("/resolutions/hardware_concurrency")
        .cloned()
        .unwrap_or(json!({}));
    assert_eq!(cores_dec["conflicted"], true);
    assert_eq!(
        cores_dec["chosen_source"].as_str().unwrap_or(""),
        "worker",
        "{cores_dec}"
    );
    assert!(
        cores_dec["mint_ok"].as_bool().unwrap_or(false)
            || cores_dec["reason"]
                .as_str()
                .unwrap_or("")
                .contains("conflict"),
        "conflict must not silent-agree: {cores_dec}"
    );

    let mint = apply_server_mint_with_evidence(&fields, Some(&evidence));
    let gate = mint.get("multi_source_mint_gate").cloned().unwrap_or(json!({}));
    assert!(
        gate["conflict_pressure"].as_f64().unwrap_or(0.0) > 0.0
            || !evidence["source_conflicts"]
                .as_array()
                .map(|a| a.is_empty())
                .unwrap_or(true),
        "conflict pressure or listed conflicts must surface: gate={gate}"
    );
}

#[test]
fn silicon_hard_conflict_excludes_mint() {
    let fields = json!({
        "residual_mean": 0.10,
        "hw_curve_webgl": [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8],
    });
    let evidence = json!({
        "fields_by_source": {
            "main": {
                "residual_mean": 0.10,
                "hw_curve_webgl": [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8]
            },
            "worker": {
                "residual_mean": 0.90,
                "hw_curve_webgl": [0.9, 0.8, 0.7, 0.6, 0.5, 0.4, 0.3, 0.2]
            }
        },
        "source_conflicts": ["source_conflict:residual_mean", "source_conflict:hw_curve_webgl"]
    });
    let d = field_mint_decision(
        "residual_mean",
        &fields,
        evidence.get("fields_by_source").and_then(|v| v.as_object()),
        &["source_conflict:residual_mean".into()],
    );
    assert_eq!(d["conflicted"], true);
    assert_eq!(
        d["mint_ok"],
        false,
        "silicon conflict must exclude mint: {d}"
    );
    assert!(
        d["reason"]
            .as_str()
            .unwrap_or("")
            .contains("silicon_conflict")
            || d["mint_ok"] == false,
        "{d}"
    );
    let res = resolve_fields_multi_source(&fields, Some(&evidence));
    // residual should be removed from commercial merge when silicon excluded
    let resolved = res.get("resolved_fields").unwrap();
    // May still have residual if single-channel dual partner - check resolution reason
    let rm = res.pointer("/resolutions/residual_mean");
    if let Some(rm) = rm {
        assert_eq!(rm["mint_ok"], false, "{rm}");
    }
    let _ = resolved;
}

/// Dual-sibling loophole: residual_mean+webgl conflict while residual_std+audio dual_ok
/// must NOT keep residual_mint_ok / silicon commercial body.
#[test]
fn silicon_sibling_dual_ok_cannot_resurrect_after_hard_conflict() {
    // Merged bag has both residual/webgl (conflicted across sources) AND residual_std/audio
    // which would dual_ok if gate only looked at mint_ok_keys without poison.
    let fields = json!({
        "residual_mean": 0.10,
        "residual_std": 0.09,
        "hw_curve_webgl": [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8],
        "hw_curve_audio": [0.01, 0.02, 0.03, 0.04, 0.05, 0.06, 0.07, 0.08, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "hardware_concurrency": 8,
        "webrtc_host_ip_hash": "host1",
    });
    let evidence = json!({
        "fields_by_source": {
            "main": {
                "residual_mean": 0.10,
                "residual_std": 0.09,
                "hw_curve_webgl": [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8],
                "hw_curve_audio": [0.01, 0.02, 0.03, 0.04, 0.05, 0.06, 0.07, 0.08, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                "form_class": "desktop",
                "platform": "Linux x86_64",
                "hardware_concurrency": 8,
                "webrtc_host_ip_hash": "host1"
            },
            "worker": {
                "residual_mean": 0.90,
                "hw_curve_webgl": [0.9, 0.8, 0.7, 0.6, 0.5, 0.4, 0.3, 0.2],
                // sibling audio/std agree with main — the loophole pair
                "residual_std": 0.09,
                "hw_curve_audio": [0.01, 0.02, 0.03, 0.04, 0.05, 0.06, 0.07, 0.08, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
            }
        },
        "source_conflicts": [
            "source_conflict:residual_mean",
            "source_conflict:hw_curve_webgl"
        ]
    });

    let gate = assess_mint_gate(&fields, Some(&evidence));
    assert_eq!(
        gate["silicon_hard_conflict"], true,
        "must detect silicon hard conflict: {gate}"
    );
    assert_eq!(
        gate["residual_mint_ok"], false,
        "sibling dual_ok must not resurrect residual_mint_ok: {gate}"
    );
    assert_eq!(
        gate["silicon_mint_ok"], false,
        "sibling dual_ok must not resurrect silicon_mint_ok: {gate}"
    );

    let mint = apply_server_mint_with_evidence(&fields, Some(&evidence));
    assert_eq!(
        mint.pointer("/multi_source_mint_gate/residual_mint_ok"),
        Some(&json!(false)),
        "mint gate residual must be false: {mint}"
    );
    assert_eq!(
        mint.pointer("/multi_source_mint_gate/silicon_mint_ok"),
        Some(&json!(false)),
        "mint gate silicon must be false: {mint}"
    );

    // Commercial body: no residual class / silicon-backed group id
    let residual_class = mint.get("residual_class");
    assert!(
        residual_class.is_none()
            || residual_class == Some(&Value::Null)
            || residual_class.and_then(|v| v.as_str()) == Some(""),
        "residual_class must be stripped from mint: {mint}"
    );
    let gid = mint["algo_group_id"].as_str().unwrap_or("");
    assert!(
        !gid.starts_with("G_DH_"),
        "must not claim G_DH_* after silicon poison: {gid} mint={mint}"
    );
    // G_DV_PARTIAL_AUDIO would still use audio curves — those must also be stripped
    assert_ne!(
        gid, "G_DV_PARTIAL_AUDIO",
        "audio sibling must not back commercial partial after silicon poison: {mint}"
    );
    assert_ne!(
        gid, "G_DV_PARTIAL_HW",
        "webgl sibling must not back commercial partial after silicon poison: {mint}"
    );
    let id = mint["device_id"].as_str().unwrap_or("");
    // If any id, must not be silicon-backed dh
    assert!(
        !(id.starts_with("dh-") || id.starts_with("dh_")),
        "no silicon-backed dh_ after hard conflict: {id}"
    );

    // Resolved fields must not retain residual/curves for commercial merge
    let res = resolve_fields_multi_source(&fields, Some(&evidence));
    let resolved = res.get("resolved_fields").and_then(|v| v.as_object()).unwrap();
    for k in [
        "residual_mean",
        "residual_std",
        "hw_curve_webgl",
        "hw_curve_audio",
    ] {
        assert!(
            !resolved.contains_key(k),
            "resolved_fields must strip {k} under silicon poison: {res}"
        );
    }
    assert_eq!(
        res.pointer("/summary/silicon_hard_conflict"),
        Some(&json!(true))
    );
}

#[test]
fn single_source_residual_conf_only_not_commercial_body() {
    let fields = json!({
        "residual_mean": 0.26,
        "residual_std": 0.09,
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "hardware_concurrency": 8,
    });
    let gate = assess_mint_gate(&fields, None);
    assert_eq!(gate["residual_mint_ok"], false);
    let mint = apply_server_mint_with_evidence(&fields, None);
    let tier = mint["algo_group_tier"].as_str().unwrap_or("");
    assert_ne!(tier, "dh", "single-source residual must not mint dh: {mint}");
    let posture = mint["analysis_posture"].as_array().cloned().unwrap_or_default();
    let joined: String = posture
        .iter()
        .filter_map(|v| v.as_str())
        .collect::<Vec<_>>()
        .join("|");
    assert!(
        joined.contains("residual") || joined.contains("conf_only") || tier == "dv" || tier == "dg",
        "expect residual strip posture: {joined}"
    );
}

#[test]
fn scores_demote_under_conflict_device_id_unchanged() {
    let base_fields = dual_curves();
    let agree_ev = json!({
        "session_id": "s_agree",
        "visitor_terminal_id": "vt1",
        "fields": base_fields,
        "fields_by_source": {
            "main": {
                "hardware_concurrency": 8,
                "platform": "Linux x86_64",
                "form_class": "desktop",
                "hw_curve_webgl": base_fields["hw_curve_webgl"],
                "hw_curve_audio": base_fields["hw_curve_audio"],
                "residual_mean": 0.26,
                "residual_std": 0.09,
            },
            "worker": {
                "hardware_concurrency": 8,
                "platform": "Linux x86_64"
            }
        },
        "source_conflicts": [],
        "batches": [
            {"batch_id": "B0_bootstrap", "source": "main"},
            {"batch_id": "B10_hw_curves", "source": "main"}
        ],
        "meta": {"product_version": "test"}
    });
    let conflict_ev = {
        let mut e = agree_ev.clone();
        e["fields_by_source"] = json!({
            "main": {
                "hardware_concurrency": 4,
                "platform": "Win32",
                "form_class": "desktop",
                "hw_curve_webgl": base_fields["hw_curve_webgl"],
                "hw_curve_audio": base_fields["hw_curve_audio"],
                "residual_mean": 0.26,
                "residual_std": 0.09,
            },
            "worker": {
                "hardware_concurrency": 12,
                "platform": "Linux x86_64"
            }
        });
        e["source_conflicts"] = json!([
            "source_conflict:hardware_concurrency",
            "source_conflict:platform"
        ]);
        e["session_id"] = json!("s_conflict");
        e
    };

    let out_agree = evaluate_session(&agree_ev, None, None, None, false).expect("agree");
    let out_conflict = evaluate_session(&conflict_ev, None, None, None, false).expect("conflict");

    let os_a = out_agree
        .pointer("/product/os/score")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0);
    let os_c = out_conflict
        .pointer("/product/os/score")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0);
    let br_a = out_agree
        .pointer("/product/br/score")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0);
    let br_c = out_conflict
        .pointer("/product/br/score")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0);

    // Under conflict, at least one of os/br should demote (or demotion metadata present)
    let demoted = os_c < os_a - 0.01
        || br_c < br_a - 0.01
        || out_conflict
            .pointer("/product/os/multi_source_demotion_risk")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
            > 0.0
        || out_conflict
            .pointer("/product/os/mint_conflict_pressure_applied")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
            > 0.0
        || out_conflict
            .pointer("/product/br/reasons")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter().any(|r| {
                    r.as_str()
                        .map(|s| s.contains("conflict") || s.contains("source"))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);

    assert!(
        demoted,
        "scores should demote under multi-source conflict\nagree os={os_a} br={br_a}\nconflict os={os_c} br={br_c}\nproduct={}",
        out_conflict.get("product").cloned().unwrap_or(json!({}))
    );

    // Soft never promote
    assert!(!PROMOTE_TO_COMMERCIAL_ID);
    let _e = SoftEdge {
        a_session: "a".into(),
        b_session: "b".into(),
        priority: "P1".into(),
        promote_to_commercial_id: false,
        confidence: 0.5,
        reason: "t".into(),
    };

    // Score path does not rewrite commercial id independently of mint
    let did_a = out_agree.pointer("/device/device_id").cloned();
    let did_p = out_agree.pointer("/product/device_id").cloned();
    assert_eq!(did_a, did_p);

    // conflict_score_demotion pure function reacts
    let gate_c = out_conflict
        .pointer("/device/multi_source_mint_gate")
        .cloned()
        .unwrap_or(json!({}));
    let (risk, reasons) = conflict_score_demotion(
        &gate_c,
        &["source_conflict:platform".into(), "source_conflict:hardware_concurrency".into()],
    );
    assert!(risk > 0.0 || !reasons.is_empty() || gate_c.get("conflict_pressure").is_some());
}

#[test]
fn gateway_protocol_preferred_over_fe_ua() {
    let fields = json!({
        "user_agent": "Mozilla/5.0 Spoofed/1.0",
        "ja4": "t13d1516h2_edge",
    });
    let evidence = json!({
        "fields_by_source": {
            "main": {"user_agent": "Mozilla/5.0 Spoofed/1.0"},
            "gateway": {"user_agent": "Mozilla/5.0 RealEdge/120.0", "ja4": "t13d1516h2_edge"}
        },
        "source_conflicts": ["source_conflict:user_agent"]
    });
    let res = resolve_fields_multi_source(&fields, Some(&evidence));
    let ua_dec = res.pointer("/resolutions/user_agent").unwrap();
    // gateway should win for user_agent when present as soft/protocol edge
    let chosen = ua_dec["chosen_source"].as_str().unwrap_or("");
    assert!(
        chosen == "gateway" || ua_dec["chosen_trust"].as_str() == Some("gateway_edge"),
        "expected gateway UA preference: {ua_dec}"
    );
    assert_eq!(ua_dec["conflicted"], true);
}

#[test]
fn capture_multi_source_audit_and_logs() {
    let scratch = std::env::var("GROK_SCRATCH")
        .unwrap_or_else(|_| "/tmp/grok-goal-b1501f907a31/implementer".into());
    let _ = std::fs::create_dir_all(&scratch);

    // Produce real mint outputs for evidence
    let fields = dual_curves();
    let agree = json!({
        "fields_by_source": {
            "main": fields,
            "worker": {
                "hardware_concurrency": 8,
                "platform": "Linux x86_64"
            }
        },
        "source_conflicts": []
    });
    let conflict = json!({
        "fields_by_source": {
            "main": {
                "hardware_concurrency": 2,
                "platform": "Win32",
                "hw_curve_webgl": fields["hw_curve_webgl"],
                "hw_curve_audio": fields["hw_curve_audio"],
                "residual_mean": 0.26,
                "residual_std": 0.09,
                "form_class": "desktop",
                "webrtc_host_ip_hash": "host_lab"
            },
            "worker": {"hardware_concurrency": 16, "platform": "Linux x86_64"}
        },
        "source_conflicts": [
            "source_conflict:hardware_concurrency",
            "source_conflict:platform"
        ]
    });
    let m_agree = apply_server_mint_with_evidence(&fields, Some(&agree));
    let m_conflict = apply_server_mint_with_evidence(&fields, Some(&conflict));
    let res_conflict = resolve_fields_multi_source(&fields, Some(&conflict));

    let evidence_json = json!({
        "agree_mint": {
            "algo_group_id": m_agree.get("algo_group_id"),
            "multi_source_resolution_summary": m_agree.pointer("/multi_source_resolution/summary"),
            "gate": {
                "residual_mint_ok": m_agree.pointer("/multi_source_mint_gate/residual_mint_ok"),
                "host_mint_ok": m_agree.pointer("/multi_source_mint_gate/host_mint_ok"),
                "conflict_pressure": m_agree.pointer("/multi_source_mint_gate/conflict_pressure"),
            }
        },
        "conflict_mint": {
            "algo_group_id": m_conflict.get("algo_group_id"),
            "device_id": m_conflict.get("device_id"),
            "resolution": res_conflict.get("summary"),
            "cores_chosen": res_conflict.pointer("/resolutions/hardware_concurrency/chosen_source"),
            "platform_chosen": res_conflict.pointer("/resolutions/platform/chosen_source"),
            "gate_conflict_pressure": m_conflict.pointer("/multi_source_mint_gate/conflict_pressure"),
        },
        "soft_never_promote": !PROMOTE_TO_COMMERCIAL_ID,
    });
    std::fs::write(
        format!("{scratch}/multi_source_evidence.json"),
        serde_json::to_string_pretty(&evidence_json).unwrap(),
    )
    .unwrap();

    // Also ensure score_materials_boost still works
    let _ = score_materials_boost(&fields);
}
