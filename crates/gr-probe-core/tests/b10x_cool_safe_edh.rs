//! Cool-safe B10x scheduling + honest-failure EDH gate.

use gr_probe_core::brain::build_frontier;
use gr_probe_core::edh::{
    build_edh, missing_b10x_silicon, silicon_pack_attempt_complete,
};
use gr_probe_core::evaluate::evaluate_session;
use serde_json::json;

fn evidence_b10_no_b10x() -> serde_json::Value {
    json!({
        "session_id": "sess_b10x_cool_001",
        "batches": [
            {"batch_id": "B10_hw_curves", "source": "main"},
            {"batch_id": "B0_bootstrap", "source": "main"},
            {"batch_id": "B11_interaction", "source": "main"}
        ],
        "fields": {
            "form_class": "desktop",
            "hw_curve_webgl": [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8],
            "hw_curve_audio": [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8],
            "residual_mean": 0.3,
            "residual_std": 0.05,
            "engine_family": "blink",
            "timezone": "Asia/Shanghai"
        },
        "meta": {"skip_session_probe": true}
    })
}

#[test]
fn cool_down_with_b10_schedules_silicon_b10x() {
    let ev = evidence_b10_no_b10x();
    let plan = build_frontier(&ev, true, None, 12).expect("frontier");
    let ids: Vec<_> = plan
        .packs
        .iter()
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()))
        .collect();
    assert!(
        ids.iter().any(|p| p.starts_with("B10x_silicon_")),
        "expected silicon B10x on cool, got {ids:?}"
    );
    assert!(
        plan.notes.iter().any(|n| n.contains("cool_safe_b10x")),
        "notes={:?}",
        plan.notes
    );
}

#[test]
fn started_heartbeat_not_terminal_still_missing() {
    let ev = json!({
        "batches": [{
            "batch_id": "B10x_silicon_noderiv",
            "source": "main",
            "payload": {"fields": {
                "b10x_pack": "B10x_silicon_noderiv",
                "b10x_ok": false,
                "b10x_err": "started",
                "b10x_phase": "start"
            }}
        }],
        "fields": {}
    });
    assert!(!silicon_pack_attempt_complete(&ev, "B10x_silicon_noderiv"));
    let miss = missing_b10x_silicon(&ev);
    assert!(
        miss.iter().any(|m| m == "B10x_silicon_noderiv"),
        "started-only must remain missing, miss={miss:?}"
    );
}

#[test]
fn honest_fail_batch_completes_silicon_pack() {
    let ev = json!({
        "batches": [
            {"batch_id": "B10_hw_curves", "source": "main"},
            {
                "batch_id": "B10x_silicon_ulp",
                "source": "main",
                "payload": {"fields": {
                    "b10x_pack": "B10x_silicon_ulp",
                    "b10x_ok": false,
                    "b10x_err": "no_curve",
                    "b10x_phase": "done",
                    "residual_paths_n": 0
                }}
            },
            {
                "batch_id": "B10x_silicon_noderiv",
                "source": "main",
                "payload": {"fields": {
                    "b10x_pack": "B10x_silicon_noderiv",
                    "b10x_ok": true,
                    "b10x_phase": "done",
                    "residual_paths_n": 3,
                    "hw_curve_webgl": [0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8]
                }}
            },
            {
                "batch_id": "B10x_silicon_rint",
                "source": "main",
                "payload": {"fields": {
                    "b10x_pack": "B10x_silicon_rint",
                    "b10x_ok": false,
                    "b10x_err": "multipath_timeout",
                    "b10x_phase": "done",
                    "residual_paths_n": 0
                }}
            }
        ],
        "fields": {
            "hw_curve_webgl": [0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8],
            "form_class": "desktop"
        }
    });
    assert!(missing_b10x_silicon(&ev).is_empty(), "miss={:?}", missing_b10x_silicon(&ev));
    let edh = build_edh(&ev, &json!({"device_id": "dh_test"}));
    assert_eq!(edh["research_gate"]["b10x_complete"], json!(true));
}

#[test]
fn evaluate_cool_route_keeps_b10x_must_land() {
    let mut ev = evidence_b10_no_b10x();
    // Rich enough for evaluate
    if let Some(fo) = ev.get_mut("fields").and_then(|f| f.as_object_mut()) {
        fo.insert("user_agent".into(), json!("Mozilla/5.0 Chrome/120"));
        fo.insert("os_family".into(), json!("linux"));
        fo.insert("platform".into(), json!("Linux x86_64"));
        fo.insert("webgl_unmasked_renderer".into(), json!("ANGLE (NVIDIA)"));
    }
    let out = evaluate_session(&ev, None, None, None, true).expect("eval");
    let rp = &out["route_plan"];
    assert_eq!(rp["b10x_must_land"], json!(true), "route={rp}");
    let packs = rp["packs"].as_array().cloned().unwrap_or_default();
    let has_b10x = packs.iter().any(|p| {
        p.get("pack_id")
            .or_else(|| p.get("batch_id"))
            .and_then(|v| v.as_str())
            .is_some_and(|s| s.starts_with("B10x_silicon_"))
    });
    assert!(has_b10x, "evaluate route must include silicon B10x, packs={packs:?}");
    assert_eq!(out["stop_probe"], json!(false));
}
