//! Guest-simulatable readiness: durable Soft file store, SLA 大盘, offline conf cal.
//! No production traffic; no hardcoded dv_* expectations.

use gr_probe_core::{
    aggregate_identity_sla, calibrate_offline, commercial_id_heat_report, commercial_projection,
    pairs_from_matrix_cells, SlaSessionRow, SoftEdge, SoftEdgeStore, FileSoftEdgeStore,
    PROMOTE_TO_COMMERCIAL_ID, RUNTIME_CONFIDENCE_VERSION,
};
use serde_json::json;
use std::path::PathBuf;

fn curves() -> (Vec<f64>, Vec<f64>) {
    let audio: Vec<f64> = (0..64)
        .map(|i| ((i as f64) * 0.17).sin().abs() + 0.1)
        .collect();
    let webgl: Vec<f64> = (0..32)
        .map(|i| ((i as f64) * 0.29).cos().abs() * 0.4 + 0.05)
        .collect();
    (audio, webgl)
}

fn tmp_dir(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "gr_guest_ready_{}_{}",
        name,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn file_soft_store_multi_worker_durable() {
    let dir = tmp_dir("soft");
    let w1 = FileSoftEdgeStore::with_fuse(&dir, 3, "guest_test").unwrap();
    let w2 = FileSoftEdgeStore::open(&dir).unwrap();
    w1.put_edge(
        "t",
        &SoftEdge {
            a_session: "a".into(),
            b_session: "b".into(),
            priority: "p1".into(),
            promote_to_commercial_id: true,
            confidence: 0.9,
            reason: "hw".into(),
        },
    )
    .unwrap();
    let listed = w2.list_edges("t").unwrap();
    assert_eq!(listed.len(), 1);
    assert!(!listed[0].promote_to_commercial_id);
    assert!(!PROMOTE_TO_COMMERCIAL_ID);

    let (a, w) = curves();
    let p = commercial_projection(&json!({
        "form_class": "desktop",
        "hw_curve_audio": a,
        "hw_curve_webgl": w,
        "platform": "Linux x86_64",
    }));
    if let Some(dv) = p.get("device_id").and_then(|v| v.as_str()) {
        w1.record_device_id_sighting("t", dv, "s1").unwrap();
        w2.record_device_id_sighting("t", dv, "s2").unwrap();
        let heat = commercial_id_heat_report(&w2, "t", dv);
        assert_eq!(heat["ok"], true);
        assert!(heat["heat"]["session_count"].as_i64().unwrap() >= 2);
        assert_eq!(heat["soft_promote"], false);
    }
}

#[test]
fn guest_sla_dashboard_from_projections() {
    let thin = commercial_projection(&json!({}));
    let (a, w) = curves();
    let rich = commercial_projection(&json!({
        "form_class": "desktop",
        "hw_curve_audio": a,
        "hw_curve_webgl": w,
        "platform": "Linux x86_64",
    }));
    let cells = vec![
        json!({
            "engine": "thin",
            "device_id": thin.get("device_id"),
            "eligible": thin.get("eligible"),
            "has_both_curves": thin.get("has_both_curves"),
            "materials_included": thin.get("materials_included"),
            "no_id_reasons": thin.get("no_id_reasons"),
            "soft_promote": false,
        }),
        json!({
            "engine": "camoufox",
            "device_id": rich.get("device_id"),
            "eligible": rich.get("eligible"),
            "has_both_curves": rich.get("has_both_curves"),
            "has_audio": true,
            "has_webgl": true,
            "hw_audio_stable": rich.pointer("/materials/hw_audio_stable"),
            "hw_webgl_stable": rich.pointer("/materials/hw_webgl_stable"),
            "materials_included": rich.get("materials_included"),
            "trust_sum": rich.get("trust_sum"),
            "no_id_reasons": rich.get("no_id_reasons").cloned().unwrap_or(json!([])),
            "soft_promote": false,
        }),
        json!({
            "engine": "librewolf_rfp",
            "device_id": null,
            "eligible": false,
            "has_audio": true,
            "has_webgl": false,
            "materials_included": ["form_class", "hw_audio_stable"],
            "no_id_reasons": ["missing_curve_webgl"],
            "soft_promote": false,
        }),
    ];
    let rows: Vec<SlaSessionRow> = cells.iter().map(SlaSessionRow::from_matrix_cell).collect();
    let rep = aggregate_identity_sla(&rows);
    assert_eq!(rep["n_sessions"], 3);
    assert_eq!(rep["soft_promote_violations"], 0);
    assert!(rep["gating"]["soft_promote_clean"].as_bool().unwrap());
    let hist = rep["no_id_reason_histogram"].as_object().unwrap();
    assert!(
        hist.contains_key("missing_curve_webgl") || hist.contains_key("missing_hardware_anchor") || !hist.is_empty()
            || rep["counts"]["has_device_id"].as_i64().unwrap() > 0,
        "histogram or ids: {rep}"
    );
}

#[test]
fn offline_conf_cal_from_matrix_pairs() {
    let (a, w) = curves();
    let rich = commercial_projection(&json!({
        "form_class": "desktop",
        "hw_curve_audio": a.clone(),
        "hw_curve_webgl": w.clone(),
        "platform": "Linux x86_64",
    }));
    let other_audio: Vec<f64> = (0..64).map(|i| ((i as f64) * 0.91).sin().abs() + 0.4).collect();
    let other_webgl: Vec<f64> = (0..32).map(|i| ((i as f64) * 1.1).cos().abs() * 0.7 + 0.2).collect();
    let peer = commercial_projection(&json!({
        "form_class": "desktop",
        "hw_curve_audio": other_audio,
        "hw_curve_webgl": other_webgl,
        "platform": "Linux x86_64",
    }));
    let cells = vec![
        json!({
            "device_id": rich.get("device_id"),
            "hw_audio_stable": rich.pointer("/materials/hw_audio_stable"),
            "hw_webgl_stable": rich.pointer("/materials/hw_webgl_stable"),
            "trust_sum": rich.get("trust_sum"),
        }),
        json!({
            "device_id": rich.get("device_id"),
            "hw_audio_stable": rich.pointer("/materials/hw_audio_stable"),
            "hw_webgl_stable": rich.pointer("/materials/hw_webgl_stable"),
            "trust_sum": rich.get("trust_sum"),
        }),
        json!({
            "device_id": peer.get("device_id"),
            "hw_audio_stable": peer.pointer("/materials/hw_audio_stable"),
            "hw_webgl_stable": peer.pointer("/materials/hw_webgl_stable"),
            "trust_sum": peer.get("trust_sum"),
        }),
    ];
    let pairs = pairs_from_matrix_cells(&cells);
    let report = calibrate_offline(&pairs);
    if pairs.is_empty() {
        assert_eq!(report["confidence_version"], RUNTIME_CONFIDENCE_VERSION);
    } else {
        assert_eq!(report["ok"], true);
        assert_eq!(report["runtime_default_remains"], RUNTIME_CONFIDENCE_VERSION);
        assert_eq!(report["silent_relabel_forbidden"], true);
    }
}
