//! Follow-on SSOT open items: B26 agent parity, B27 storage, B35 dom/perf,
//! B15 layer divergence, B18 webgpu deep — analysis + brain schedule.

use gr_probe_core::{build_frontier, score_bot, score_br, score_os, stack_auth_from_fields};
use gr_probe_core::xsrc::TruthResult;
use serde_json::json;

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
fn agent_parity_automation_globals_raise_spoof() {
    let clean = json!({
        "webgl_unmasked_renderer": "ANGLE (NVIDIA GeForce GTX 1050 Ti)",
        "residual_mean": 0.5001811906403189,
        "agent_parity_hash": "abc",
        "agent_automation_globals_n": 0,
    });
    let dirty = json!({
        "webgl_unmasked_renderer": "ANGLE (NVIDIA GeForce GTX 1050 Ti)",
        "residual_mean": 0.5001811906403189,
        "agent_parity_hash": "def",
        "agent_automation_globals_n": 3,
    });
    let a0 = stack_auth_from_fields(&clean);
    let a1 = stack_auth_from_fields(&dirty);
    assert!(
        a1.spoof_score > a0.spoof_score,
        "automation globals must raise spoof: {} vs {} reasons {:?}",
        a1.spoof_score,
        a0.spoof_score,
        a1.reasons
    );
    assert!(a1.reasons.iter().any(|r| r.contains("agent_parity")));
}

#[test]
fn layer_divergence_and_storage_hit_scores() {
    let fields = json!({
        "platform": "Linux",
        "user_agent": "Mozilla/5.0 Chrome/120",
        "webdriver": false,
        "hw_curve_webgl": [0.1, 0.2, 0.3, 0.4],
        "sandbox_ok": true,
        "sandbox_sources_received": ["iframe:1"],
        "multi_source_match_ratio": 0.95,
        "native_integrity_ratio": 0.95,
        "canvas_geometry_hash": "c1",
        "api_flags_hash": "a1",
        "agent_parity_hash": "ap",
        "agent_automation_globals_n": 0,
        "privacy_storage_score": 5,
        "storage_quota": 1_000_000,
        "local_storage": true,
        "cookie_enabled": true,
        "dom_rect_hash": "dr",
        "perf_timeline_hash": "pt",
        "layer_divergence_score": 0.4,
        "webgpu_limits_hash": "wl",
        "webgpu_dual_adapter_diff": false,
    });
    let stack = stack_auth_from_fields(&fields);
    assert!(
        stack.reasons.iter().any(|r| r.contains("layer_divergence")),
        "reasons {:?}",
        stack.reasons
    );
    assert!(stack.spoof_score >= 0.1, "low layer match raises spoof");
    let bot = score_bot(&fields, "balanced").unwrap();
    let br = score_br(&fields, &stack, &bot, &truth_ok(), &[]);
    let hits: Vec<_> = br["field_hits"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|h| h.as_str())
        .collect();
    assert!(
        hits.iter().any(|h| h.contains("agent")
            || h.contains("storage")
            || h.contains("dom")
            || h.contains("layer")
            || h.contains("mat:")),
        "hits={hits:?}"
    );
    let os = score_os(&fields, &stack, &truth_ok(), &[]);
    assert!(os.get("score").is_some());
}

#[test]
fn frontier_schedules_b26_b27_b35() {
    let evidence = json!({
        "session_id": "followon_thin",
        "sources": ["main"],
        "batches": [
            {"batch_id":"B0_bootstrap","source":"main"},
            {"batch_id":"B1_conflict","source":"main"}
        ],
        "fields": {
            "form_class": "desktop",
            "platform": "Linux",
            "webdriver": false,
            "user_agent": "Mozilla/5.0 Chrome/120",
            "hardware_concurrency": 4
        },
        "has_gateway": false,
    });
    let f = build_frontier(&evidence, true, Some(true), 32).unwrap();
    let packs: Vec<String> = f
        .packs
        .iter()
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    let codes: Vec<_> = f.gaps.iter().map(|g| g.code.clone()).collect();
    assert!(
        packs.iter().any(|p| {
            matches!(
                p.as_str(),
                "B26_agent_parity"
                    | "B27_storage_privacy"
                    | "B35_dom_perf"
                    | "B15_cross_curves"
                    | "B18_webgpu"
            )
        }) || codes.iter().any(|c| {
            c.contains("agent")
                || c.contains("storage")
                || c.contains("dom")
                || c.contains("layer")
                || c.contains("webgpu")
        }),
        "packs={packs:?} codes={codes:?}"
    );
}
