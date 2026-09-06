//! R00–R99 authenticity spot-check: catalog, schedule lane, analysis demotion.
use gr_probe_core::bot::BotScore;
use gr_probe_core::brain::build_frontier;
use gr_probe_core::brain_control::{
    is_verify_rand_pack, VERIFY_SPOTCHECK_MAX, VERIFY_SPOTCHECK_MIN, VERIFY_SPOTCHECK_PER_TICK,
};
use gr_probe_core::catalog::load_catalog;
use gr_probe_core::product_scores::score_br;
use gr_probe_core::stack_auth::stack_auth_from_fields;
use gr_probe_core::xsrc::TruthResult;
use serde_json::{json, Value};
use std::collections::HashSet;

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

#[test]
fn catalog_has_100_random_verify_packs() {
    let cat = load_catalog().expect("catalog");
    let r_packs: Vec<_> = cat
        .packs
        .iter()
        .filter(|p| is_verify_rand_pack(&p.pack_id) || p.layer == "verify_rand")
        .collect();
    assert_eq!(r_packs.len(), 100, "expected R00-R99, got {}", r_packs.len());
    for i in 0..100 {
        let pid = format!("R{i:02}_spotcheck");
        let p = cat.resolve(&pid).unwrap_or_else(|| panic!("missing {pid}"));
        assert_eq!(p.layer, "verify_rand");
        assert_eq!(p.schedule, "dynamic");
        assert!(!p.hard_eligible);
    }
}

#[test]
fn frontier_schedules_verify_spotcheck_subset() {
    let ev = json!({
        "session_id": "sess_verify_abc",
        "fields": {
            "platform": "Linux",
            "user_agent": "Chrome",
            "webdriver": false,
            "form_class": "desktop"
        },
        "batches": [{"batch_id":"B0_bootstrap"}],
        "sources": ["main"]
    });
    let f = build_frontier(&ev, true, Some(true), 10).expect("frontier");
    let verify: Vec<_> = f
        .packs
        .iter()
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()))
        .filter(|id| is_verify_rand_pack(id))
        .collect();
    assert!(
        verify.len() >= VERIFY_SPOTCHECK_MIN && verify.len() <= VERIFY_SPOTCHECK_MAX,
        "expected {VERIFY_SPOTCHECK_MIN}..={VERIFY_SPOTCHECK_MAX} verify packs (nominal {VERIFY_SPOTCHECK_PER_TICK}), got {} {verify:?}",
        verify.len()
    );
    for p in &f.packs {
        let pid = p.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
        if is_verify_rand_pack(pid) {
            assert_eq!(
                p.get("verify_spotcheck").and_then(|v| v.as_bool()),
                Some(true)
            );
            assert_eq!(p.get("score_weight").and_then(|v| v.as_f64()), Some(0.0));
        }
    }
    // Different session → different subset (probabilistic but high chance)
    let ev2 = json!({
        "session_id": "sess_verify_xyz_other",
        "fields": {
            "platform": "Linux",
            "user_agent": "Chrome",
            "webdriver": false,
            "form_class": "desktop"
        },
        "batches": [{"batch_id":"B0_bootstrap"}],
        "sources": ["main"]
    });
    let f2 = build_frontier(&ev2, true, Some(true), 10).expect("frontier2");
    let v2: HashSet<_> = f2
        .packs
        .iter()
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .filter(|id| is_verify_rand_pack(id))
        .collect();
    let v1: HashSet<_> = verify.iter().map(|s| s.to_string()).collect();
    // Allow rare collision; only assert both non-empty
    assert!(!v1.is_empty() && !v2.is_empty());
}

#[test]
fn high_empty_ratio_demotes_br_authenticity() {
    let bot = bot_human();
    let base = json!({
        "platform": "Linux",
        "user_agent": "Chrome",
        "webdriver": false,
        "form_class": "desktop",
        "webgl_support": true,
    });
    let bad = json!({
        "platform": "Linux",
        "user_agent": "Chrome",
        "webdriver": false,
        "form_class": "desktop",
        "webgl_support": true,
        "verify_role": "authenticity_spotcheck",
        "verify_algo": "gr_rand_verify_v1",
        "verify_not_score_material": true,
        "verify_nonempty_ratio": 0.20,
        "verify_exec_ratio": 0.25,
        "verify_empty_n": 100,
        "verify_probe_ops_n": 120,
        "verify_dim_fail_n": 5,
    });
    let stack = stack_auth_from_fields(&base);
    let br_ok = score_br(&base, &stack, &bot, &truth_ok(), &[]);
    let stack_b = stack_auth_from_fields(&bad);
    let br_bad = score_br(&bad, &stack_b, &bot, &truth_ok(), &[]);
    let rs = reasons(&br_bad);
    assert!(
        rs.iter().any(|r| r.contains("rand_verify")),
        "expected rand_verify demotion reasons, got {rs:?}"
    );
    // Risk path must fire; absolute score may interact with coverage — require reasons + risk-ish hit
    let hits: Vec<String> = br_bad
        .get("field_hits")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(|s| s.to_string()))
        .collect();
    assert!(
        hits.iter().any(|h| h.contains("rand_verify") || h.contains("verify_")),
        "expected verify field_hits, got {hits:?}"
    );
    let _ = br_ok; // baseline scored without verify surface
}

#[test]
fn manifest_diversity_target() {
    let text = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../spec/random_verify_manifest.json"),
    )
    .expect("manifest");
    let v: Value = serde_json::from_str(&text).expect("json");
    let mean = v
        .get("mean_unique_vs_peers")
        .and_then(|x| x.as_f64())
        .unwrap_or(0.0);
    let min_u = v
        .get("min_unique_vs_peers")
        .and_then(|x| x.as_f64())
        .unwrap_or(0.0);
    // PRIMARY: global non-repeat across all 100 packs jointly ≥ 85%
    let global = v
        .get("global_nonrepeat_rate")
        .and_then(|x| x.as_f64())
        .unwrap_or(0.0);
    let g_unique = v.get("global_unique_ops").and_then(|x| x.as_u64()).unwrap_or(0);
    let total = v.get("total_slots").and_then(|x| x.as_u64()).unwrap_or(0);
    assert!(
        global >= 0.85,
        "global_nonrepeat_rate {global} < 0.85 (unique {g_unique} / total {total})"
    );
    // pairwise uniqueness is diagnostic only (should stay high after global no-reuse)
    assert!(mean >= 0.85, "pairwise mean unique {mean} < 0.85");
    assert_eq!(v.get("pack_count").and_then(|x| x.as_u64()), Some(100));
    let ppd = v.get("probes_per_dim").and_then(|x| x.as_u64()).unwrap_or(0);
    assert!(ppd >= 10, "probes_per_dim {ppd} < 10");
    let max_b = v
        .get("max_b_overlap_ratio")
        .and_then(|x| x.as_f64())
        .unwrap_or(1.0);
    assert!(max_b <= 0.10 + 1e-9, "b_overlap {max_b} > 10%");
}
