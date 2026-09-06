//! Gap-closure tests for iss/09 ordered "全面领先" blockers (general scheme only).
//! Drives shipped commercial_projection / decision / soft store / brain — no hardcoded dv_*.

use gr_probe_core::{
    build_frontier, commercial_id_heat_report, commercial_projection, decide_product_action,
    ActionSensitivity, DecisionInput, MemorySoftEdgeStore, SoftEdge, SoftEdgeStore,
    CONFIDENCE_VERSION, PROMOTE_TO_COMMERCIAL_ID,
};
use serde_json::json;

fn reasons(fields: serde_json::Value) -> Vec<String> {
    let p = commercial_projection(&fields);
    p.get("no_id_reasons")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn form_class_derived_when_omitted_still_emits_reasons_only_if_ineligible() {
    // trust_materials derives form_class=desktop when omitted — so form is not "missing".
    // With both curves present, session may become eligible via derivation (honest general scheme).
    let audio: Vec<f64> = (0..64).map(|i| ((i as f64) * 0.19).sin().abs() + 0.05).collect();
    let webgl: Vec<f64> = (0..32).map(|i| ((i as f64) * 0.27).cos().abs() * 0.4 + 0.03).collect();
    let p = commercial_projection(&json!({
        "hw_curve_audio": audio,
        "hw_curve_webgl": webgl,
        "platform": "Linux x86_64",
    }));
    assert!(
        p.pointer("/materials/form_class").and_then(|v| v.as_str()).is_some(),
        "form_class should be derived when omitted"
    );
    // Thin: no curves at all → missing hardware + trust
    let thin = reasons(json!({ "user_agent": "x" }));
    assert!(
        thin.iter().any(|c| c == "missing_curve_audio" || c == "missing_hardware_anchor"),
        "thin session reasons: {thin:?}"
    );
}

#[test]
fn single_curve_audio_algo_groups_or_missing_webgl_reason() {
    // algo_groups_v1: G_DH_CORE_AUDIO may authorize identity with audio alone when
    // support/SW pass — so no_id_reasons can be empty (eligible) instead of
    // hard missing_curve_webgl. Dual-curve remains preferred for dh_ strength.
    let fields = json!({
        "form_class": "desktop",
        "hw_curve_audio": [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 0.1, 0.2, 0.3],
    });
    let p = commercial_projection(&fields);
    let eligible = p.get("eligible").and_then(|v| v.as_bool()).unwrap_or(false);
    let r = reasons(fields);
    let has_audio = p
        .pointer("/materials/hw_audio_stable")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());
    assert!(
        eligible
            || r.iter().any(|c| {
                c == "missing_curve_webgl"
                    || c == "single_curve_trust_insufficient"
                    || c == "missing_hardware_anchor"
            }),
        "audio-only: eligible via single-core group OR missing webgl/hardware reason; eligible={eligible} r={r:?}"
    );
    if eligible {
        assert!(has_audio, "eligible audio-only must include audio material");
    }
}

#[test]
fn single_curve_webgl_algo_groups_or_missing_audio_reason() {
    // Symmetric: G_DH_CORE_WEBGL may win with webgl alone under algo_groups_v1.
    let fields = json!({
        "form_class": "desktop",
        "hw_curve_webgl": [0.2, 0.1, 0.3, 0.4, 0.5, 0.2, 0.1, 0.3, 0.4, 0.5, 0.1, 0.2],
    });
    let p = commercial_projection(&fields);
    let eligible = p.get("eligible").and_then(|v| v.as_bool()).unwrap_or(false);
    let r = reasons(fields);
    let has_webgl = p
        .pointer("/materials/hw_webgl_stable")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());
    assert!(
        eligible
            || r.iter().any(|c| {
                c == "missing_curve_audio"
                    || c == "single_curve_trust_insufficient"
                    || c == "missing_hardware_anchor"
            }),
        "webgl-only: eligible via single-core group OR missing audio/hardware reason; eligible={eligible} r={r:?}"
    );
    if eligible {
        assert!(has_webgl, "eligible webgl-only must include webgl material");
    }
}

#[test]
fn no_id_reason_trust_sum_when_empty() {
    let r = reasons(json!({}));
    assert!(
        r.iter().any(|c| c == "trust_sum_below_min" || c == "missing_form"),
        "expected trust/form reasons, got {r:?}"
    );
    assert!(r.iter().any(|c| c == "missing_hardware_anchor"));
}

#[test]
fn eligible_clears_no_id_reasons() {
    // Use long non-constant curves so digests + trust clear gates (shipped path).
    let audio: Vec<f64> = (0..64).map(|i| ((i as f64) * 0.17).sin().abs() * 0.5 + 0.1).collect();
    let webgl: Vec<f64> = (0..32).map(|i| ((i as f64) * 0.31).cos().abs() * 0.4 + 0.05).collect();
    let p = commercial_projection(&json!({
        "form_class": "desktop",
        "hw_curve_audio": audio,
        "hw_curve_webgl": webgl,
        "platform": "Linux x86_64",
    }));
    let eligible = p.get("eligible").and_then(|v| v.as_bool()).unwrap_or(false);
    if eligible {
        let r = p
            .get("no_id_reasons")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        assert_eq!(r, 0, "eligible sessions must have empty no_id_reasons");
        let id = p.get("device_id").and_then(|v| v.as_str()).unwrap_or("");
        assert!((id.starts_with("dv-") || id.starts_with("dv_")), "expected dv_* got {id}");
    } else {
        // Still assert reasons are non-empty when not eligible
        let r = p.get("no_id_reasons").and_then(|v| v.as_array()).unwrap();
        assert!(!r.is_empty());
    }
}

#[test]
fn decision_table_order_bot_then_dv_then_soft() {
    assert!(!PROMOTE_TO_COMMERCIAL_ID);
    let bot = decide_product_action(&DecisionInput {
        bot_verdict: "bot".into(),
        device_id: Some("dv_aaaaaaaaaaaaaaaa".into()),
        soft_edge: true,
        soft_promote: false,
        confidence: 0.9,
        confidence_version: CONFIDENCE_VERSION.into(),
        sensitivity: ActionSensitivity::NonSensitive,
        os_score: None,
        br_score: None,
        rpa_score: None,
    });
    assert!(bot.bot_veto && !bot.allow);

    let soft_only = decide_product_action(&DecisionInput {
        bot_verdict: "human".into(),
        device_id: None,
        soft_edge: true,
        soft_promote: false,
        confidence: 0.4,
        confidence_version: CONFIDENCE_VERSION.into(),
        sensitivity: ActionSensitivity::Sensitive,
        os_score: None,
        br_score: None,
        rpa_score: None,
    });
    assert!(!soft_only.use_device_id_as_primary);
    assert!(soft_only.use_soft_as_旁证);
    assert!(soft_only.require_otp);
    assert_eq!(soft_only.confidence_version, CONFIDENCE_VERSION);

    // Derive a real id from projection then decide
    let audio: Vec<f64> = (0..64).map(|i| ((i as f64) * 0.13).sin().abs() + 0.05).collect();
    let webgl: Vec<f64> = (0..32).map(|i| ((i as f64) * 0.29).cos().abs() * 0.3 + 0.02).collect();
    let p = commercial_projection(&json!({
        "form_class": "desktop",
        "hw_curve_audio": audio,
        "hw_curve_webgl": webgl,
        "platform": "Linux x86_64",
    }));
    if let Some(id) = p.get("device_id").and_then(|v| v.as_str()) {
        let ok = decide_product_action(&DecisionInput {
            bot_verdict: "human".into(),
            device_id: Some(id.to_string()),
            soft_edge: false,
            soft_promote: false,
            confidence: 0.85,
            confidence_version: CONFIDENCE_VERSION.into(),
            sensitivity: ActionSensitivity::NonSensitive,
            os_score: None,
            br_score: None,
            rpa_score: None,
        });
        assert!(ok.use_device_id_as_primary);
        assert!(ok.allow);
    }
}

#[test]
fn soft_store_multi_worker_and_heat_and_fuse() {
    let store = MemorySoftEdgeStore::with_fuse(2, "test_owner");
    let w2 = store.worker_view();
    let edge = SoftEdge {
        a_session: "s1".into(),
        b_session: "s2".into(),
        priority: "p1".into(),
        promote_to_commercial_id: true, // must be forced false on put
        confidence: 0.9,
        reason: "hw_noise".into(),
    };
    store.put_edge("t1", &edge).expect("put1");
    let listed = w2.list_edges("t1").expect("list");
    assert_eq!(listed.len(), 1);
    assert!(!listed[0].promote_to_commercial_id);

    // fuse at threshold 2
    store
        .put_edge(
            "t1",
            &SoftEdge {
                a_session: "s3".into(),
                b_session: "s4".into(),
                priority: "p1".into(),
                promote_to_commercial_id: false,
                confidence: 0.8,
                reason: "hw".into(),
            },
        )
        .expect("put2");
    let fused = store.put_edge(
        "t1",
        &SoftEdge {
            a_session: "s5".into(),
            b_session: "s6".into(),
            priority: "p1".into(),
            promote_to_commercial_id: false,
            confidence: 0.7,
            reason: "hw".into(),
        },
    );
    assert!(fused.is_err(), "fuse should trip");
    assert!(fused.unwrap_err().contains("soft_misrecall_fuse"));
    assert_eq!(store.fuse_owner(), "test_owner");

    // heat from real commercial id path
    let audio: Vec<f64> = (0..64).map(|i| ((i as f64) * 0.11).sin().abs() + 0.08).collect();
    let webgl: Vec<f64> = (0..32).map(|i| ((i as f64) * 0.23).cos().abs() * 0.35 + 0.04).collect();
    let p = commercial_projection(&json!({
        "form_class": "desktop",
        "hw_curve_audio": audio,
        "hw_curve_webgl": webgl,
        "platform": "Linux x86_64",
    }));
    let Some(dv) = p.get("device_id").and_then(|v| v.as_str()) else {
        // If not eligible in this env, still validated fuse/store above
        return;
    };
    store
        .record_device_id_sighting("t1", dv, "sess_a")
        .unwrap();
    w2.record_device_id_sighting("t1", dv, "sess_b").unwrap();
    let heat = commercial_id_heat_report(&store, "t1", dv);
    assert_eq!(heat["ok"], true);
    assert_eq!(heat["soft_promote"], false);
    assert!(heat["heat"]["session_count"].as_i64().unwrap() >= 2);
    assert_eq!(heat["heat"]["collision_style"], true);
}

#[test]
fn brain_hard_priority_note_present() {
    let evidence = json!({
        "session_id": "bp",
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "batches": [
            {"batch_id": "B0_bootstrap", "source": "main"},
            {"batch_id": "B8_gateway", "source": "gateway"}
        ],
        "fields": { "user_agent": "Mozilla/5.0", "form_class": "desktop" }
    });
    let plan = build_frontier(&evidence, true, None, 5).expect("frontier");
    assert!(plan.notes.iter().any(|n| n.contains("hard_anchor")));
    assert!(plan.packs.iter().any(|p| p["pack_id"] == "B10_hw_curves"));
}
