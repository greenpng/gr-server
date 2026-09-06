//! Multi-channel hedge / vote fusion on shipped product_scores + corroborate paths.
//! Proves: conflict demotes; multi-support raises; single material cannot be real;
//! frontier schedules multi-direction hedge packs + staged sandbox.

use gr_probe_core::corroborate::{
    count_material_families, fuse_axis_hedge, fuse_channels, ChannelOut, Stance,
};
use gr_probe_core::xsrc::TruthResult;
use gr_probe_core::{
    build_frontier, score_bot, score_br, score_os, stack_auth_from_fields,
};
use serde_json::{json, Map, Value};

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

fn rich_os_fields() -> Map<String, Value> {
    let mut fo = Map::new();
    fo.insert("platform".into(), json!("Linux x86_64"));
    fo.insert("os_family".into(), json!("linux"));
    fo.insert("timezone".into(), json!("UTC"));
    fo.insert("hw_curve_webgl".into(), json!([0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8]));
    fo.insert("hw_curve_audio".into(), json!([0.1, 0.2, 0.3, 0.4]));
    fo.insert("gl_precision_matrix".into(), json!([[1, 1, 23]]));
    fo.insert("gpu_wall_staircase".into(), json!([{"size":64,"iter":8,"wall_ms":1.2}]));
    fo.insert("font_bitmap_hash".into(), json!("fb_abc"));
    fo.insert("api_flags_hash".into(), json!("af_def"));
    fo.insert("webrtc_host_ip_hash".into(), json!("wh_1"));
    fo.insert("sandbox_ok".into(), json!(true));
    fo.insert("sandbox_sources_received".into(), json!(["iframe:d1", "worker:w1"]));
    fo.insert("multi_source_match_ratio".into(), json!(0.95));
    fo.insert("material_vote_digest".into(), json!("mv_1"));
    fo.insert("material_count".into(), json!(3));
    fo.insert("stack_class".into(), json!("native"));
    fo.insert("hardware_concurrency".into(), json!(8));
    fo.insert("device_memory".into(), json!(8));
    fo
}

#[test]
fn fuse_conflict_cannot_average_to_real() {
    let chs = vec![
        ChannelOut {
            id: "ch_env",
            safety: 0.95,
            coverage: 0.9,
            stance: Stance::Support,
            reasons: vec![],
        },
        ChannelOut {
            id: "ch_xsrc",
            safety: 0.15,
            coverage: 0.85,
            stance: Stance::Contradict,
            reasons: vec!["source_conflict:user_agent".into()],
        },
    ];
    let (safety, status, rs) = fuse_channels(&chs, 0.2);
    assert!(safety < 0.5, "got {safety}");
    assert_eq!(status, "suspect");
    assert!(rs.iter().any(|r| r.contains("conflict")));
}

#[test]
fn multi_support_channels_can_reach_high_safety() {
    let chs = vec![
        ChannelOut {
            id: "ch_env",
            safety: 0.9,
            coverage: 0.85,
            stance: Stance::Support,
            reasons: vec![],
        },
        ChannelOut {
            id: "ch_vote",
            safety: 0.86,
            coverage: 0.8,
            stance: Stance::Support,
            reasons: vec![],
        },
        ChannelOut {
            id: "ch_xsrc",
            safety: 0.84,
            coverage: 0.75,
            stance: Stance::Support,
            reasons: vec![],
        },
    ];
    let (safety, status, _) = fuse_channels(&chs, 0.2);
    assert!(safety >= 0.72, "got {safety}");
    assert_eq!(status, "real");
}

#[test]
fn score_os_conflict_demotes_via_shipped_hedge() {
    let mut fo = rich_os_fields();
    fo.insert("material_cross_conflict".into(), json!(true));
    let fields = Value::Object(fo.clone());
    let stack = stack_auth_from_fields(&fields);
    let truth = truth_ok();
    let os = score_os(&fields, &stack, &truth, &[]);
    let score = os["score"].as_f64().unwrap();
    let status = os["status"].as_str().unwrap();
    assert!(score < 0.5, "conflict must demote score, got {score} status={status} {os}");
    assert_ne!(status, "real");
    assert!(
        os.get("hedge_channels").and_then(|v| v.as_array()).map(|a| a.len() >= 3).unwrap_or(false),
        "must expose multi-channel diag: {os}"
    );
    assert_eq!(os["fusion_algo"], "gr_axis_hedge_v1");
}

#[test]
fn score_os_multi_material_support_without_conflict() {
    let fo = rich_os_fields();
    let (n, fams) = count_material_families("os", &fo);
    assert!(n >= 2, "need multi-family fixture: n={n} fams={fams:?}");
    let fields = Value::Object(fo);
    let stack = stack_auth_from_fields(&fields);
    let truth = truth_ok();
    let os = score_os(&fields, &stack, &truth, &[]);
    let score = os["score"].as_f64().unwrap();
    let mat_n = os["material_family_count"].as_u64().unwrap_or(0);
    assert!(mat_n >= 2, "material families exposed: {os}");
    assert!(score >= 0.45, "support multi-material should not collapse: {score} {os}");
    // hits should show multi-family mat: tags
    let hits = os["field_hits"].as_array().cloned().unwrap_or_default();
    let mat_hits = hits
        .iter()
        .filter(|h| h.as_str().unwrap_or("").starts_with("mat:"))
        .count();
    assert!(mat_hits >= 2, "multi-hit materials: {hits:?}");
}

#[test]
fn score_br_single_material_not_real() {
    let fields = json!({
        "webdriver": false,
        "user_agent": "Mozilla/5.0 Chrome/120",
        "platform": "Linux",
    });
    let stack = stack_auth_from_fields(&fields);
    let bot = score_bot(&fields, "balanced").unwrap();
    let truth = truth_ok();
    let br = score_br(&fields, &stack, &bot, &truth, &[]);
    assert_ne!(
        br["status"].as_str(),
        Some("real"),
        "single-field browser path cannot be real: {br}"
    );
    assert!(
        br.get("material_family_count").and_then(|v| v.as_u64()).unwrap_or(99) < 2
            || br["status"] != "real",
        "{br}"
    );
}

#[test]
fn fuse_axis_hedge_entry_conflict_and_support() {
    let mut fo = rich_os_fields();
    let (s_ok, st_ok, _, diag) = fuse_axis_hedge(
        "os",
        &fo,
        0.88,
        0.75,
        vec!["env_ok".into()],
        false,
        &[],
        false,
    );
    assert!(diag.len() >= 4, "channels: {diag:?}");
    assert!(s_ok >= 0.5, "support: {s_ok} {st_ok}");

    fo.insert("material_cross_conflict".into(), json!(true));
    let (s_bad, st_bad, rs, _) = fuse_axis_hedge(
        "os",
        &fo,
        0.88,
        0.75,
        vec!["env_ok".into()],
        false,
        &[],
        false,
    );
    assert!(s_bad < 0.5, "conflict: {s_bad}");
    assert_ne!(st_bad, "real");
    assert!(rs.iter().any(|r| r.contains("conflict") || r.contains("material")));
}

#[test]
fn frontier_schedules_hedge_directions_and_sandbox_stage() {
    let evidence = json!({
        "session_id": "hedge_sess",
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
    let f = build_frontier(&evidence, true, Some(true), 20).expect("frontier");
    let packs: Vec<String> = f
        .packs
        .iter()
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    // Multi-direction specialized packs (not empty flat list)
    let hedge_like = packs.iter().any(|p| {
        p == "B23_native_canvas_hedge"
            || p == "B24_material_crosscheck"
            || p == "B20_challenge_seed"
            || p == "B22_gpu_timer"
            || p == "B21_census_volume"
            || p == "B17_hw_physical"
            || p == "B10_hw_curves"
    });
    assert!(hedge_like, "expected multi-direction packs: {packs:?}");
    let codes: Vec<_> = f.gaps.iter().map(|g| g.code.clone()).collect();
    assert!(
        codes.iter().any(|c| c.contains("hedge")
            || c.contains("pohw")
            || c.contains("challenge")
            || c.contains("gpu")
            || c.contains("census")
            || c.contains("native")),
        "gaps: {codes:?}"
    );
    let route = f.to_route_plan().unwrap();
    let sp = &route["sandbox_plan"];
    let kinds = sp["kinds"].as_array().map(|a| a.len()).unwrap_or(99);
    let maxc = sp["max_concurrent"].as_i64().unwrap_or(99);
    assert!(kinds <= 2, "first wave sandbox kinds<=2: {sp}");
    assert!(maxc <= 2, "first wave max_concurrent<=2: {sp}");
    let ranking = route
        .get("direction_ranking")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(ranking.len() >= 5, "direction ranking: {ranking:?}");
}
