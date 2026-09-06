//! Local-environment closures: conf_cal labeled corpus + what lab can prove without fleet CI.

use gr_probe_core::{
    adopt_ladder_calibration, calibrate_offline_by_ladder, pairs_from_json, LabeledPair,
};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

fn spec_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.join("spec")
}

/// conf 真标定：本机可跑离线灌数 + 分层 report；不能替代大规模人工 1−P_fp 生产 adopt。
#[test]
fn conf_cal_labeled_pairs_offline_pipeline() {
    let path = spec_dir().join("conf_cal_labeled_pairs_v1.json");
    let raw: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let pairs_v = raw
        .get("pairs")
        .cloned()
        .unwrap_or(raw.clone());
    let pairs: Vec<LabeledPair> = pairs_from_json(&pairs_v);
    assert!(
        pairs.len() >= 12,
        "need enough labeled pairs for offline strata, got {}",
        pairs.len()
    );
    let report = calibrate_offline_by_ladder(&pairs);
    assert_eq!(report.get("ok").and_then(|v| v.as_bool()), Some(true));
    // empty refuse — adopted=false, never silent relabel
    let empty = adopt_ladder_calibration(&[]);
    assert_eq!(empty.get("adopted").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(
        empty.get("silent_relabel_forbidden").and_then(|v| v.as_bool()),
        Some(true)
    );
    // explicit adopt with pairs
    let adopted = adopt_ladder_calibration(&pairs);
    assert_eq!(adopted.get("adopted").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        adopted
            .get("silent_relabel_forbidden")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
}

/// Fleet CI: fixture matrix is first-class; live multi-vendor remains optional.
#[test]
fn antidetect_fleet_ci_fixture_matrix_in_capability() {
    let path = spec_dir().join("self_capability.json");
    let raw: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let fixture = raw
        .pointer("/capabilities/antidetect_fleet_ci_fixture_matrix")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let text = raw.to_string();
    assert!(
        fixture || text.contains("antidetect_fleet_ci_fixture_matrix"),
        "self_capability must document fixture fleet CI: {raw}"
    );
    // hook_only may remain as historical key but must not be the only mode
    let hook = raw
        .pointer("/capabilities/antidetect_fleet_ci_hook_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    assert!(
        !hook || fixture,
        "if hook_only still true, fixture matrix must also be true"
    );
}
