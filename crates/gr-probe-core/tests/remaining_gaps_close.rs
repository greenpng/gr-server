//! Close remaining SSOT open gaps: H04/H09/H15/D08/D21/D35 wiring + brain.

use gr_probe_core::neg_dict::{derive_neg_dict_hits, neg_dict_spoof_boost};
use gr_probe_core::{build_frontier, stack_auth_from_fields};
use serde_json::json;

#[test]
fn neg_dict_boosts_spoof_from_hits() {
    let clean = json!({
        "webgl_unmasked_renderer": "ANGLE (NVIDIA GeForce GTX 1050 Ti)",
        "residual_mean": 0.5001811906403189,
        "webdriver": false,
    });
    let dirty = json!({
        "webgl_unmasked_renderer": "Google SwiftShader",
        "residual_mean": 0.5002774107689955,
        "webdriver": true,
        "outer_zero": true,
        "neg_dict_hits": ["puppeteer", "cdc_prop"],
        "agent_automation_globals_n": 2,
    });
    let a0 = stack_auth_from_fields(&clean);
    let a1 = stack_auth_from_fields(&dirty);
    assert!(
        a1.spoof_score > a0.spoof_score + 0.2,
        "neg dict must raise spoof: clean={} dirty={} reasons={:?}",
        a0.spoof_score,
        a1.spoof_score,
        a1.reasons
    );
    assert!(a1.reasons.iter().any(|r| r.starts_with("neg:") || r.contains("neg_dict")));
    let hits = derive_neg_dict_hits(dirty.as_object().unwrap());
    assert!(neg_dict_spoof_boost(&hits) >= 0.3);
}

#[test]
fn remaining_materials_change_stack_reasons() {
    let fo = json!({
        "webgl_unmasked_renderer": "ANGLE (NVIDIA)",
        "residual_mean": 0.5001811906403189,
        "raster_edge_hash": "r1",
        "raster_edge_curve": [{"o":0,"mean":10.0}],
        "thermal_cpu_slope": 0.5,
        "thermal_bursts": [{"i":0,"cpu_ms":1.0}],
        "mem_alloc_ladder": [{"mb":1,"ok":true}],
        "mem_alloc_max_mb": 16,
        "ws_handshake_ms": 12.0,
        "websocket_present": true,
        "ws_constructor_native": true,
        "hid_surface_score": 3,
        "gamepad_count": 0,
    });
    let a = stack_auth_from_fields(&fo);
    let joined = a.reasons.join("|");
    for needle in [
        "raster",
        "thermal",
        "mem_pressure",
        "websocket",
        "hid_gamepad",
    ] {
        assert!(
            joined.contains(needle),
            "expected reason containing {needle}: {:?}",
            a.reasons
        );
    }
    assert!(a.vm_score >= 0.05 || a.reasons.iter().any(|r| r.contains("thermal")));
}

#[test]
fn frontier_schedules_remaining_gap_packs() {
    let evidence = json!({
        "session_id": "rem_gaps",
        "sources": ["main"],
        "batches": [
            {"batch_id":"B0_bootstrap","source":"main"},
            {"batch_id":"B1_conflict","source":"main"}
        ],
        "fields": {
            "form_class": "desktop",
            "platform": "Linux",
            "webdriver": false,
            "user_agent": "Mozilla/5.0 Chrome/120"
        },
        "has_gateway": false,
    });
    let f = build_frontier(&evidence, true, Some(true), 36).unwrap();
    let packs: Vec<String> = f
        .packs
        .iter()
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    let codes: Vec<_> = f.gaps.iter().map(|g| g.code.clone()).collect();
    let want = [
        "B36_raster_msaa",
        "B37_thermal_drift_lite",
        "B38_neg_dict",
        "B39_mem_pressure",
        "B40_websocket_fp",
        "B41_hid_gamepad",
    ];
    let pack_hit = want.iter().any(|w| packs.iter().any(|p| p == *w));
    let gap_hit = codes.iter().any(|c| {
        c.contains("raster")
            || c.contains("thermal")
            || c.contains("neg_dict")
            || c.contains("mem_pressure")
            || c.contains("websocket")
            || c.contains("hid")
    });
    assert!(
        pack_hit || gap_hit,
        "expected remaining gap packs/codes: packs={packs:?} codes={codes:?}"
    );
}
