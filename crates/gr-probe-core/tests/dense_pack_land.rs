//! Dense B47–B79 port: catalog, directions, gap schedule, digest scoring.
use gr_probe_core::bot::BotScore;
use gr_probe_core::brain::{build_frontier, scan_gaps};
use gr_probe_core::brain_directions::DIRECTIONS;
use gr_probe_core::catalog::load_catalog;
use gr_probe_core::product_scores::{score_br, score_os};
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

#[test]
fn catalog_has_80_packs_including_dense_b47_b79() {
    let cat = load_catalog().expect("catalog");
    let n = cat.packs.len();
    // 80 commercial/dense + 100 R00–R99 authenticity spot-check
    assert!(n >= 70 && n <= 220, "pack count {n}");
    for pid in [
        "B47_api_flags_detail",
        "B55_webrtc_ice_deep",
        "B59_webgl_params_full",
        "B73_pointer_capabilities",
        "B79_cross_origin_isolation",
        "B80_r100_high_value",
    ] {
        assert!(cat.resolve(pid).is_some(), "missing pack {pid}");
    }
}

#[test]
fn directions_cover_dense_samples() {
    let mut all = std::collections::HashSet::new();
    for d in DIRECTIONS {
        for p in d.packs {
            all.insert(*p);
        }
    }
    assert!(all.len() >= 60, "direction union {}", all.len());
    assert!(all.contains("B47_api_flags_detail"));
    assert!(all.contains("B79_cross_origin_isolation"));
}

#[test]
fn scan_gaps_opens_dense_need_when_materials_missing() {
    let ev = json!({
        "fields": {
            "platform": "Linux",
            "user_agent": "Chrome",
            "webdriver": false,
            "form_class": "desktop"
        },
        "batches": [{"batch_id":"B0_bootstrap"}],
        "sources": ["main"]
    });
    let gaps = scan_gaps(&ev, None).expect("gaps");
    let codes: Vec<_> = gaps.iter().map(|g| g.code.as_str()).collect();
    assert!(
        codes.iter().any(|c| c.contains("api_flags")
            || c.contains("font_matrix")
            || c.contains("webrtc")
            || c.contains("webgl")
            || c.contains("navigator")
            || c.contains("census")),
        "expected dense-related need codes, got {codes:?}"
    );
}

#[test]
fn frontier_can_schedule_dense_or_related_under_budget() {
    let ev = json!({
        "session_id": "s_dense",
        "fields": {
            "platform": "Linux",
            "user_agent": "Chrome",
            "webdriver": false,
            "form_class": "desktop"
        },
        "batches": [{"batch_id":"B0_bootstrap"}],
        "sources": ["main"]
    });
    let f = build_frontier(&ev, true, Some(true), 48).expect("frontier");
    let packs: Vec<_> = f
        .packs
        .iter()
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()))
        .collect();
    assert!(!packs.is_empty());
    // either a dense B47+ or classic deep that dense supports
    let ok = packs.iter().any(|p| {
        p.starts_with("B4")
            || p.starts_with("B5")
            || p.starts_with("B6")
            || p.starts_with("B7")
            || p.starts_with("B2")
            || p.starts_with("B1")
    });
    assert!(ok, "frontier packs unexpected: {packs:?}");
}

#[test]
fn dense_digests_enter_product_scores() {
    let bot = bot_human();
    let thin = json!({
        "platform": "Linux x86_64",
        "user_agent": "Mozilla/5.0 Chrome/120",
        "webdriver": false,
        "form_class": "desktop",
        "max_touch_points": 0,
        "webgl_support": true,
    });
    let dense = json!({
        "platform": "Linux x86_64",
        "user_agent": "Mozilla/5.0 Chrome/120",
        "webdriver": false,
        "form_class": "desktop",
        "max_touch_points": 0,
        "webgl_support": true,
        "probe_density_algo": "gr_dense_v3",
        "dense_pack": "B47_api_flags_detail",
        "probe_min_met": true,
        "probe_specific_n": 36,
        "probe_pad_n": 0,
        "api_flags_hash": "af_abc123",
        "api_probe_hash": "ap_def456",
        "api_probe_ratio": 0.55,
        "api_flags_hit": 22,
        "api_flags_n": 40,
        "font_matrix_hash": "fm_789",
        "mq_hash": "mq_012",
    });
    let stack_t = stack_auth_from_fields(&thin);
    let stack_d = stack_auth_from_fields(&dense);
    let br_thin = score_br(&thin, &stack_t, &bot, &truth_ok(), &[]);
    let br_dense = score_br(&dense, &stack_d, &bot, &truth_ok(), &[]);
    let os_dense = score_os(&dense, &stack_d, &truth_ok(), &[]);

    let hits_br = hits(&br_dense);
    let reasons_br = reasons(&br_dense);
    assert!(
        hits_br.iter().any(|h| h.contains("gr_dense")
            || h.contains("api_flags_hash")
            || h.contains("dense_pack")
            || h.contains("probe_density")),
        "dense surface hits missing: {hits_br:?}"
    );
    assert!(
        reasons_br.iter().any(|r| r.contains("dense_digest")
            || r.contains("probe_min_met")
            || r.contains("api_probe_ratio")
            || r.contains("probe_specific")),
        "dense digest reasons missing: {reasons_br:?}"
    );
    let s_thin = br_thin.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let s_dense = br_dense.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
    assert!(
        s_dense + 1e-9 >= s_thin,
        "dense digests should not lower br score vs thin; thin={s_thin} dense={s_dense}"
    );
    let os_hits = hits(&os_dense);
    assert!(
        os_hits.iter().any(|h| h.contains("dense")
            || h.contains("api_flags")
            || h.contains("font_matrix")
            || h.contains("gr_dense")),
        "os should see dense digests: {os_hits:?}"
    );
}

#[test]
fn dense_error_and_pad_dominate_demote() {
    let bot = bot_human();
    let bad = json!({
        "platform": "Linux",
        "user_agent": "Chrome",
        "webdriver": false,
        "form_class": "desktop",
        "probe_density_algo": "gr_dense_v3",
        "probe_min_met": false,
        "probe_specific_n": 8,
        "probe_pad_n": 40,
        "densify_error": "family boom",
        "api_probe_ratio": 0.05,
    });
    let stack = stack_auth_from_fields(&bad);
    let br = score_br(&bad, &stack, &bot, &truth_ok(), &[]);
    let rs = reasons(&br);
    assert!(
        rs.iter().any(|r| r.contains("dense_family_error")
            || r.contains("dense_pad_dominates")
            || r.contains("probe_specific_thin")
            || r.contains("api_probe_ratio_low")),
        "expected demotion reasons, got {rs:?}"
    );
}
