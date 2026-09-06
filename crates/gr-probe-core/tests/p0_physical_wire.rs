//! P0 SSOT gap closure: analyze wire + physical pack scheduling.
//! Drives shipped stack_auth / product_scores / build_frontier.

use gr_probe_core::corroborate::{fuse_channels, ChannelOut, Stance};
use gr_probe_core::{
    build_frontier, score_bot, score_br, score_os, stack_auth_from_fields,
};
use gr_probe_core::xsrc::TruthResult;
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

#[test]
fn p0a_field_presence_changes_stack_auth() {
    let thin = json!({
        "webgl_unmasked_renderer": "ANGLE (NVIDIA GeForce GTX 1050 Ti)",
        "residual_mean": 0.5001811906403189,
    });
    let a0 = stack_auth_from_fields(&thin);
    let mut fo = thin.as_object().cloned().unwrap();
    fo.insert("challenge_avg_cv".into(), json!(0.15));
    fo.insert("session_residual_cv".into(), json!(0.02));
    fo.insert("native_integrity_ratio".into(), json!(0.5));
    fo.insert(
        "gpu_wall_staircase".into(),
        json!([
            {"size":64,"iter":8,"wall_ms":1.0},
            {"size":128,"iter":16,"wall_ms":4.0},
            {"size":256,"iter":32,"wall_ms":20.0},
            {"size":512,"iter":64,"wall_ms":90.0}
        ]),
    );
    fo.insert("gpu_slope_wall".into(), json!(0.0005));
    fo.insert("gpu_r2_wall".into(), json!(0.91));
    fo.insert("gl_precision_matrix".into(), json!([[1, 1, 23]]));
    fo.insert("shader_ulp_max".into(), json!(12.0));
    fo.insert("actual_max_tex".into(), json!(4096));
    fo.insert("gl_max_texture_size".into(), json!(32768));
    fo.insert("perf_now_resolution_ms".into(), json!(0.1));
    fo.insert("raf_jitter_cv".into(), json!(0.4));
    let a1 = stack_auth_from_fields(&Value::Object(fo));
    assert!(
        a1.spoof_score > a0.spoof_score || a1.vm_score > a0.vm_score,
        "wired fields must change spoof/vm: thin={} rich_spoof={} rich_vm={} reasons={:?}",
        a0.spoof_score,
        a1.spoof_score,
        a1.vm_score,
        a1.reasons
    );
    assert!(
        a1.reasons.iter().any(|r| {
            r.contains("noise")
                || r.contains("native")
                || r.contains("gpu")
                || r.contains("caps")
                || r.contains("precision")
                || r.contains("clock")
                || r.contains("raf")
                || r.contains("challenge")
                || r.contains("staircase")
        }),
        "expected p0 reasons: {:?}",
        a1.reasons
    );
}

#[test]
fn p0a_score_os_reads_physical_materials() {
    let mut fo = Map::new();
    fo.insert("platform".into(), json!("Linux"));
    fo.insert("hw_curve_webgl".into(), json!([0.1, 0.2, 0.3, 0.4]));
    fo.insert("font_bitmap_hash".into(), json!("f1"));
    fo.insert("sandbox_ok".into(), json!(true));
    fo.insert("sandbox_sources_received".into(), json!(["iframe:1"]));
    fo.insert("multi_source_match_ratio".into(), json!(0.95));
    let base = Value::Object(fo.clone());
    let stack = stack_auth_from_fields(&base);
    let os0 = score_os(&base, &stack, &truth_ok(), &[]);

    fo.insert("gpu_wall_staircase".into(), json!([{"size":64,"iter":8,"wall_ms":1.0}]));
    fo.insert("gpu_slope_wall".into(), json!(1e-7));
    fo.insert("gl_precision_matrix".into(), json!([[1, 1, 23]]));
    fo.insert("shader_ulp_max".into(), json!(2.0));
    fo.insert("actual_max_tex".into(), json!(8192));
    fo.insert("gl_max_texture_size".into(), json!(8192));
    fo.insert("challenge_avg_cv".into(), json!(0.001));
    fo.insert("codec_matrix".into(), json!({"video/mp4": 2}));
    fo.insert("perf_now_resolution_ms".into(), json!(0.001));
    let rich = Value::Object(fo);
    let stack1 = stack_auth_from_fields(&rich);
    let os1 = score_os(&rich, &stack1, &truth_ok(), &[]);
    let hits = os1["field_hits"].as_array().cloned().unwrap_or_default();
    let hit_s: Vec<String> = hits
        .iter()
        .filter_map(|h| h.as_str().map(|s| s.to_string()))
        .collect();
    assert!(
        hit_s.iter().any(|h| h.contains("gpu")
            || h.contains("precision")
            || h.contains("clock")
            || h.contains("challenge")
            || h.contains("shader")
            || h.contains("caps")
            || h.contains("mat:")),
        "os hits must reflect physical materials: {hit_s:?} vs base={os0}"
    );
}

#[test]
fn conflict_still_cannot_be_real() {
    let chs = vec![
        ChannelOut {
            id: "ch_env",
            safety: 0.95,
            coverage: 0.9,
            stance: Stance::Support,
            reasons: vec![],
        },
        ChannelOut {
            id: "ch_x",
            safety: 0.1,
            coverage: 0.8,
            stance: Stance::Contradict,
            reasons: vec!["x".into()],
        },
    ];
    let (s, st, _) = fuse_channels(&chs, 0.2);
    assert!(s < 0.5);
    assert_ne!(st, "real");
}

#[test]
fn frontier_schedules_physical_p0_packs() {
    let evidence = json!({
        "session_id": "p0_phys",
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
            "hardware_concurrency": 8
        },
        "has_gateway": false,
    });
    let f = build_frontier(&evidence, true, Some(true), 24).expect("frontier");
    let packs: Vec<String> = f
        .packs
        .iter()
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    let physical = packs.iter().any(|p| {
        matches!(
            p.as_str(),
            "B22_gpu_timer"
                | "B31_shader_numeric"
                | "B33_caps_pressure"
                | "B25_clock_raf"
                | "B19_eme_media"
                | "B17_hw_physical"
                | "B20_challenge_seed"
        )
    });
    assert!(physical, "expected physical/H-class packs: {packs:?}");
    let codes: Vec<_> = f.gaps.iter().map(|g| g.code.clone()).collect();
    assert!(
        codes.iter().any(|c| {
            c.contains("shader")
                || c.contains("caps")
                || c.contains("clock")
                || c.contains("gpu")
                || c.contains("eme")
                || c.contains("pohw")
                || c.contains("hedge")
                || c.contains("challenge")
        }),
        "gaps: {codes:?}"
    );
    let route = f.to_route_plan().unwrap();
    let ranking = route
        .get("direction_ranking")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    assert!(ranking >= 5, "direction ranking len={ranking}");
    // Sandbox first wave still small
    if let Some(sp) = route.get("sandbox_plan") {
        let maxc = sp["max_concurrent"].as_i64().unwrap_or(99);
        assert!(maxc <= 2, "sandbox stage small: {sp}");
    }
}

#[test]
fn score_br_native_and_conflict_still_hedged() {
    let fields = json!({
        "webdriver": false,
        "user_agent": "Mozilla/5.0 Chrome/120",
        "native_integrity_ratio": 0.95,
        "canvas_geometry_hash": "abc",
        "sandbox_ok": true,
        "sandbox_sources_received": ["iframe:1"],
        "multi_source_match_ratio": 0.9,
        "api_flags_hash": "af",
    });
    let stack = stack_auth_from_fields(&fields);
    let bot = score_bot(&fields, "balanced").unwrap();
    let br = score_br(&fields, &stack, &bot, &truth_ok(), &[]);
    assert!(br.get("material_family_count").is_some() || br.get("hedge_channels").is_some());
    // Explicit source conflict still demotes
    let br2 = score_br(
        &fields,
        &stack,
        &bot,
        &truth_ok(),
        &["source_conflict:user_agent".into()],
    );
    assert_ne!(br2["status"].as_str(), Some("real"));
    assert!(br2["score"].as_f64().unwrap() < 0.5);
}
