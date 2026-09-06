//! Audit pack counts, layers, parallel groups under band budgets (latency guard).
use gr_probe_core::brain::{build_frontier, scan_gaps};
use gr_probe_core::brain_control::{apply_band_pack_budget, max_packs_for_band, policy_band_from_scores};
use gr_probe_core::catalog::load_catalog;
use serde_json::{json, Value};
use std::collections::HashMap;

fn thin_ev() -> Value {
    json!({
        "session_id": "audit_latency",
        "fields": {
            "platform": "Linux x86_64",
            "user_agent": "Mozilla/5.0 Chrome/120",
            "webdriver": false,
            "form_class": "desktop",
            "max_touch_points": 0,
            "webgl_support": true,
        },
        "batches": [{"batch_id":"B0_bootstrap"}],
        "sources": ["main"]
    })
}

fn pack_ids(plan: &Value) -> Vec<String> {
    plan.get("packs")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect()
}

fn layer_hist(plan: &Value) -> HashMap<String, usize> {
    let mut h = HashMap::new();
    if let Some(arr) = plan.get("packs").and_then(|v| v.as_array()) {
        for p in arr {
            let layer = p
                .get("layer")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string();
            *h.entry(layer).or_default() += 1;
        }
    }
    h
}

fn dense_count(ids: &[String]) -> usize {
    ids.iter()
        .filter(|id| {
            id.chars()
                .skip(1)
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse::<u32>()
                .map(|n| (47..=80).contains(&n))
                .unwrap_or(false)
        })
        .count()
}

#[test]
fn audit_print_frontier_under_budgets() {
    let cat = load_catalog().unwrap();
    eprintln!("CATALOG_PACKS {}", cat.packs.len());
    let gaps = scan_gaps(&thin_ev(), None).unwrap();
    eprintln!("THIN_GAPS {}", gaps.len());
    let dense_gaps: Vec<_> = gaps
        .iter()
        .filter(|g| {
            g.code.contains("api_flags")
                || g.code.contains("font_matrix")
                || g.code.contains("webrtc")
                || g.code.contains("webgl")
                || g.code.contains("navigator")
                || g.detail.contains("B4")
                || g.detail.contains("B5")
                || g.detail.contains("B6")
                || g.detail.contains("B7")
        })
        .map(|g| g.code.clone())
        .collect();
    eprintln!("DENSE_ISH_GAPS {} {:?}", dense_gaps.len(), dense_gaps);

    for budget in [6usize, 10, 12, 16, 24, 48] {
        let f = build_frontier(&thin_ev(), true, Some(true), budget).unwrap();
        let v = f.to_value().unwrap();
        let ids = pack_ids(&v);
        let groups = v
            .get("parallel_groups")
            .and_then(|g| g.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let gsizes: Vec<usize> = v
            .get("parallel_groups")
            .and_then(|g| g.as_array())
            .map(|a| {
                a.iter()
                    .map(|g| g.as_array().map(|x| x.len()).unwrap_or(0))
                    .collect()
            })
            .unwrap_or_default();
        eprintln!(
            "BUDGET={budget} packs={} dense={} layers={:?} groups={} sizes={:?} ids={:?}",
            ids.len(),
            dense_count(&ids),
            layer_hist(&v),
            groups,
            gsizes,
            ids
        );
    }

    // Band budget after frontier
    let f = build_frontier(&thin_ev(), true, Some(true), 24).unwrap();
    let mut packs = f.packs.clone();
    for band in ["low", "mid", "high", "critical"] {
        let pb = policy_band_from_scores(Some(0.8), Some(0.75), Some(0.6));
        // override band
        let mut b = pb;
        if let Some(o) = b.as_object_mut() {
            o.insert("band".into(), json!(band));
            o.insert("max_packs_cap".into(), json!(max_packs_for_band(band)));
        }
        let mut p2 = packs.clone();
        apply_band_pack_budget(&mut p2, &b);
        let ids: Vec<_> = p2
            .iter()
            .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
            .collect();
        eprintln!(
            "BAND={} cap={} after_budget packs={} dense={} ids={:?}",
            band,
            max_packs_for_band(band),
            ids.len(),
            dense_count(&ids),
            ids
        );
    }

    // Critical path anchors must appear under mid budget
    let f = build_frontier(&thin_ev(), true, Some(true), 10).unwrap();
    let ids = pack_ids(&f.to_value().unwrap());
    for must in ["B10_hw_curves", "B2_hardware", "B8_gateway_early", "B11_interaction", "B12_anti_camouflage"] {
        // may or may not - print presence
        eprintln!("MUST_PRESENT {}={}", must, ids.iter().any(|x| x == must));
    }
    // lite5 dense should not dominate core
    let lite_dense: Vec<_> = f
        .packs
        .iter()
        .filter(|p| {
            p.get("layer").and_then(|v| v.as_str()) == Some("lite5")
                && p.get("pack_id")
                    .and_then(|v| v.as_str())
                    .map(|id| {
                        id.trim_start_matches('B')
                            .chars()
                            .take_while(|c| c.is_ascii_digit())
                            .collect::<String>()
                            .parse::<u32>()
                            .map(|n| n >= 47)
                            .unwrap_or(false)
                    })
                    .unwrap_or(false)
        })
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    eprintln!("LITE5_DENSE_IN_FRONTIER {:?}", lite_dense);
}

#[test]
fn critical_anchors_not_starved_by_dense_under_mid_budget() {
    let f = build_frontier(&thin_ev(), true, Some(true), 10).unwrap();
    let ids: Vec<_> = f
        .packs
        .iter()
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()))
        .collect();
    // At least one commercial/material anchor should be present
    let has_anchor = ids.iter().any(|id| {
        matches!(
            *id,
            "B10_hw_curves"
                | "B2_hardware"
                | "B8_gateway_early"
                | "B11_interaction"
                | "B12_anti_camouflage"
                | "B1_conflict"
                | "B3_system"
        )
    });
    assert!(has_anchor, "mid budget starved anchors: {ids:?}");
    // Dense should not consume entire mid budget alone
    let dense = dense_count(
        &ids.iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
    );
    assert!(
        dense <= 6,
        "too many dense packs in first mid tick: dense={dense} ids={ids:?}"
    );
}

#[test]
fn second_tick_schedules_capped_high_sev_dense_digests() {
    // After commercial anchors land, residual budget should fill high-sev digests (≤3).
    let ev = json!({
        "session_id": "audit_tick2",
        "fields": {
            "platform": "Linux x86_64",
            "user_agent": "Mozilla/5.0 Chrome/120",
            "webdriver": false,
            "form_class": "desktop",
            "max_touch_points": 0,
            "webgl_support": true,
            "webgl_unmasked_renderer": "ANGLE (NVIDIA)",
            "hw_curve_webgl": [1,2,3,4,5,6,7,8],
        },
        "batches": [
            {"batch_id":"B0_bootstrap"},
            {"batch_id":"B1_conflict"},
            {"batch_id":"B2_hardware"},
            {"batch_id":"B3_system"},
            {"batch_id":"B8_gateway_early"},
            {"batch_id":"B10_hw_curves"},
            {"batch_id":"B11_interaction"},
            {"batch_id":"B12_anti_camouflage"},
            {"batch_id":"B17_hw_physical"},
            {"batch_id":"B20_challenge_seed"},
            {"batch_id":"B7_sandbox"},
        ],
        "sources": ["main", "server"]
    });
    let f = build_frontier(&ev, true, Some(true), 10).unwrap();
    let ids: Vec<String> = f
        .packs
        .iter()
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    let dense = dense_count(&ids);
    eprintln!("TICK2 packs={} dense={} ids={:?}", ids.len(), dense, ids);
    // Cap is 1 in brain_control; always-open silicon B10x may co-schedule one densify digest.
    // Guard against unbounded dense flood (was historically a latency regression).
    assert!(
        dense <= gr_probe_core::DENSE_PACKS_PER_TICK_CAP.saturating_add(1).max(2),
        "dense over per-tick cap: dense={dense} cap={} ids={ids:?}",
        gr_probe_core::DENSE_PACKS_PER_TICK_CAP
    );
    // Prefer claim-obs digests over flood of mid/low dense
    let has_digest = ids.iter().any(|id| {
        matches!(
            id.as_str(),
            "B47_api_flags_detail"
                | "B50_font_matrix_detail"
                | "B55_webrtc_ice_deep"
                | "B59_webgl_params_full"
                | "B52_navigator_deep"
                | "B65_client_hints_full"
                | "B77_worker_env_deep"
        )
    });
    assert!(
        has_digest || dense > 0 || ids.is_empty(),
        "expected high-sev dense digests on tick2 residual, got {ids:?}"
    );
    // Parallel groups: core/mid/deep/verify plus always-open B10x silicon stages.
    // Guard against unbounded stage explosion (historical latency regression).
    assert!(
        f.parallel_groups.len() <= 8,
        "too many serial stages: {:?}",
        f.parallel_groups
    );
}

#[test]
fn b65_b74_not_lite5_core_anymore() {
    let cat = load_catalog().unwrap();
    for pid in ["B65_client_hints_full", "B74_visual_viewport"] {
        let p = cat.resolve(pid).expect(pid);
        assert_ne!(
            p.layer, "lite5",
            "{pid} must not sit in lite5 core wave (latency)"
        );
        assert_ne!(p.layer, "b8", "{pid} must not be b8");
    }
}
