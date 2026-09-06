//! Offline confidence calibration (guest-simulatable).
//!
//! Hot path stays `heuristic_v0`. This module only produces a **report** (and optional
//! calibrated version tag) from **labeled pairs** — never silently renames heuristic
//! scores as `1−P_fp` without labels.

use serde_json::{json, Value};

/// Default runtime confidence version (evaluate/decision).
pub const RUNTIME_CONFIDENCE_VERSION: &str = "heuristic_v0";
/// Emitted only when offline calibrator runs on non-empty labeled pairs.
pub const CALIBRATED_CONFIDENCE_VERSION: &str = "calibrated_v0_guest";

/// FS tau calibration snapshot (iss/60 L3) — optional adopt, never silent.
static FS_TAU_ADOPTED: std::sync::Mutex<Option<(f64, f64)>> = std::sync::Mutex::new(None);

/// Explicit adopt switch — **never silent**.
/// Order: env `GR_CONF_ADOPT` (1/true/yes) → `product_policy.confidence_adopt`.
pub fn confidence_adopt_requested() -> bool {
    if let Some(s) = gr_abi::env::get("CONF_ADOPT") {
        let t = s.trim().to_ascii_lowercase();
        if matches!(t.as_str(), "1" | "true" | "yes" | "on") {
            return true;
        }
        if matches!(t.as_str(), "0" | "false" | "no" | "off") {
            return false;
        }
    }
    crate::policy::load_product_policy()
        .get("confidence_adopt")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn adopt_max_ece() -> f64 {
    if let Some(s) = gr_abi::env::get("CONF_ADOPT_MAX_ECE") {
        if let Ok(v) = s.parse::<f64>() {
            return v.clamp(0.01, 0.9);
        }
    }
    crate::policy::load_product_policy()
        .get("confidence_adopt_max_ece")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.35)
}

fn adopt_min_pairs() -> usize {
    if let Some(s) = gr_abi::env::get("CONF_ADOPT_MIN_PAIRS") {
        if let Ok(v) = s.parse::<usize>() {
            return v.max(1);
        }
    }
    crate::policy::load_product_policy()
        .get("confidence_adopt_min_pairs")
        .and_then(|v| v.as_u64())
        .map(|u| u as usize)
        .unwrap_or(8)
}

/// Active confidence version for evaluate hot path.
/// Only returns calibrated when: **explicit adopt requested** + labeled pairs pass ECE gate.
/// Otherwise always `heuristic_v0`. Never silent-relabels.
pub fn active_confidence_version() -> String {
    if !confidence_adopt_requested() {
        return RUNTIME_CONFIDENCE_VERSION.to_string();
    }
    let decision = adopt_decision_report();
    if decision.get("adopted").and_then(|v| v.as_bool()) == Some(true) {
        CALIBRATED_CONFIDENCE_VERSION.to_string()
    } else {
        RUNTIME_CONFIDENCE_VERSION.to_string()
    }
}

/// Full adopt decision (for ops / evaluate annotation).
pub fn adopt_decision_report() -> Value {
    let requested = confidence_adopt_requested();
    if !requested {
        return json!({
            "requested": false,
            "adopted": false,
            "runtime_confidence_version": RUNTIME_CONFIDENCE_VERSION,
            "silent_relabel_forbidden": true,
            "note": "adopt not requested — stay heuristic_v0",
        });
    }
    let pairs = load_spec_labeled_pairs();
    let min_n = adopt_min_pairs();
    if pairs.len() < min_n {
        return json!({
            "requested": true,
            "adopted": false,
            "runtime_confidence_version": RUNTIME_CONFIDENCE_VERSION,
            "silent_relabel_forbidden": true,
            "reason": "insufficient_labeled_pairs",
            "n_pairs": pairs.len(),
            "min_pairs": min_n,
            "note": "explicit adopt requested but labeled set too small — refuse",
        });
    }
    let report = calibrate_offline(&pairs);
    let ece = report.get("ece_like").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let max_ece = adopt_max_ece();
    if report.get("ok").and_then(|v| v.as_bool()) != Some(true) || ece > max_ece {
        return json!({
            "requested": true,
            "adopted": false,
            "runtime_confidence_version": RUNTIME_CONFIDENCE_VERSION,
            "silent_relabel_forbidden": true,
            "reason": "ece_gate_failed",
            "ece_like": ece,
            "max_ece": max_ece,
            "n_pairs": pairs.len(),
            "note": "explicit adopt requested but ECE above gate — refuse silent-quality claim",
        });
    }
    json!({
        "requested": true,
        "adopted": true,
        "runtime_confidence_version": CALIBRATED_CONFIDENCE_VERSION,
        "runtime_default_if_disabled": RUNTIME_CONFIDENCE_VERSION,
        "silent_relabel_forbidden": true,
        "ece_like": ece,
        "max_ece": max_ece,
        "n_pairs": pairs.len(),
        "note": "explicit adopt ON — version tagged calibrated_v0_guest; still not 1-P_fp marketing",
        "report_subset": {
            "n_same_host": report.get("n_same_host"),
            "n_diff_host": report.get("n_diff_host"),
            "ece_like": report.get("ece_like"),
        },
    })
}

#[derive(Debug, Clone)]
pub struct LabeledPair {
    /// Same physical host / same hard materials expected true.
    pub same_host: bool,
    /// Model score in [0,1] (e.g. heuristic conf or soft cosine).
    pub score: f64,
    pub label_source: String,
    /// Optional association ladder level (hardware|env|profile|gateway) for stratified cal.
    pub association_level: Option<String>,
}

/// Reliability-style buckets + ECE-like mean absolute calibration error.
pub fn calibrate_offline(pairs: &[LabeledPair]) -> Value {
    if pairs.is_empty() {
        return json!({
            "ok": false,
            "error": "empty_pairs",
            "confidence_version": RUNTIME_CONFIDENCE_VERSION,
            "note": "no labels → do not emit calibrated version; keep heuristic_v0",
            "silent_relabel_forbidden": true,
        });
    }

    // 10 buckets of width 0.1
    let mut bucket_n = [0i64; 10];
    let mut bucket_pos = [0i64; 10];
    let mut bucket_score_sum = [0.0f64; 10];

    for p in pairs {
        let s = p.score.clamp(0.0, 0.9999);
        let b = (s * 10.0).floor() as usize;
        let b = b.min(9);
        bucket_n[b] += 1;
        bucket_score_sum[b] += s;
        if p.same_host {
            bucket_pos[b] += 1;
        }
    }

    let mut buckets = Vec::new();
    let mut ece = 0.0;
    let n_tot = pairs.len() as f64;
    for i in 0..10 {
        let n = bucket_n[i] as f64;
        if n < 1.0 {
            buckets.push(json!({
                "lo": i as f64 / 10.0,
                "hi": (i + 1) as f64 / 10.0,
                "n": 0,
                "mean_score": null,
                "empirical_same_host_rate": null,
            }));
            continue;
        }
        let mean_score = bucket_score_sum[i] / n;
        let emp = bucket_pos[i] as f64 / n;
        ece += (n / n_tot) * (mean_score - emp).abs();
        buckets.push(json!({
            "lo": i as f64 / 10.0,
            "hi": (i + 1) as f64 / 10.0,
            "n": bucket_n[i],
            "mean_score": (mean_score * 10000.0).round() / 10000.0,
            "empirical_same_host_rate": (emp * 10000.0).round() / 10000.0,
        }));
    }

    let n_same = pairs.iter().filter(|p| p.same_host).count();
    json!({
        "ok": true,
        "n_pairs": pairs.len(),
        "n_same_host": n_same,
        "n_diff_host": pairs.len() - n_same,
        "ece_like": (ece * 10000.0).round() / 10000.0,
        "buckets": buckets,
        "confidence_version_if_adopted": CALIBRATED_CONFIDENCE_VERSION,
        "runtime_default_remains": RUNTIME_CONFIDENCE_VERSION,
        "note": "Guest offline calibration only — not production ECE quality claim",
        "silent_relabel_forbidden": true,
    })
}

/// Build labeled pairs from guest matrix-style cells using commercial digests.
/// same_host = same (audio_stable, webgl_stable) digests when both have device_id;
/// diff when digests differ or one has no id with different materials.
pub fn pairs_from_matrix_cells(cells: &[Value]) -> Vec<LabeledPair> {
    let mut pairs = Vec::new();
    for (i, a) in cells.iter().enumerate() {
        for b in cells.iter().skip(i + 1) {
            let da = a.get("hw_audio_stable").and_then(|v| v.as_str()).unwrap_or("");
            let wa = a.get("hw_webgl_stable").and_then(|v| v.as_str()).unwrap_or("");
            let db = b.get("hw_audio_stable").and_then(|v| v.as_str()).unwrap_or("");
            let wb = b.get("hw_webgl_stable").and_then(|v| v.as_str()).unwrap_or("");
            let ida = a.get("device_id").and_then(|v| v.as_str()).unwrap_or("");
            let idb = b.get("device_id").and_then(|v| v.as_str()).unwrap_or("");
            // Only pair when both have digest materials for a clear label
            if da.is_empty() || db.is_empty() {
                continue;
            }
            let same = da == db && wa == wb && !ida.is_empty() && ida == idb;
            let diff = (da != db || wa != wb) && (!ida.is_empty() || !idb.is_empty());
            if !same && !diff {
                continue;
            }
            // score: heuristic stand-in from trust_sum if present, else 1.0 same / 0.2 diff
            let sa = a.get("trust_sum").and_then(|v| v.as_f64()).unwrap_or(if same { 0.85 } else { 0.25 });
            let sb = b.get("trust_sum").and_then(|v| v.as_f64()).unwrap_or(if same { 0.85 } else { 0.25 });
            let score = ((sa + sb) / 2.0).clamp(0.0, 1.0);
            pairs.push(LabeledPair {
                same_host: same,
                score,
                label_source: "guest_matrix_digest_pair".into(),
                association_level: a
                    .get("association_level")
                    .and_then(|v| v.as_str())
                    .or_else(|| b.get("association_level").and_then(|v| v.as_str()))
                    .map(|s| s.to_string()),
            });
        }
    }
    pairs
}

/// Build labeled pairs from association customer labels (iss/50 P2).
/// Expects label objects with `outcome` + optional `detail.score` / `detail.same_host`.
/// Outcome mapping: legit|human|same_host|true → same_host=true;
/// fraud|bot|diff_host|false → same_host=false.
pub fn pairs_from_customer_labels(labels: &[Value]) -> Vec<LabeledPair> {
    let mut pairs = Vec::new();
    for lab in labels {
        let outcome = lab
            .get("outcome")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let detail = lab.get("detail").cloned().unwrap_or(json!({}));
        let same = detail
            .get("same_host")
            .and_then(|v| v.as_bool())
            .or_else(|| {
                if matches!(
                    outcome.as_str(),
                    "legit" | "human" | "same_host" | "true" | "ok" | "pass"
                ) {
                    Some(true)
                } else if matches!(
                    outcome.as_str(),
                    "fraud" | "bot" | "diff_host" | "false" | "fail" | "reject"
                ) {
                    Some(false)
                } else {
                    None
                }
            });
        let Some(same_host) = same else {
            continue;
        };
        let score = detail
            .get("score")
            .and_then(|v| v.as_f64())
            .or_else(|| detail.get("confidence").and_then(|v| v.as_f64()))
            .or_else(|| detail.get("model_score").and_then(|v| v.as_f64()))
            .unwrap_or(if same_host { 0.8 } else { 0.25 })
            .clamp(0.0, 1.0);
        pairs.push(LabeledPair {
            same_host,
            score,
            label_source: lab
                .get("label_source")
                .and_then(|v| v.as_str())
                .unwrap_or("customer_label")
                .to_string(),
            association_level: detail
                .get("association_level")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        });
    }
    pairs
}

/// ECE report from customer label list (assoc_labels dump).
pub fn ece_from_customer_labels(labels: &[Value]) -> Value {
    let pairs = pairs_from_customer_labels(labels);
    let report = calibrate_offline(&pairs);
    json!({
        "algo": "customer_label_ece_v1",
        "n_labels_in": labels.len(),
        "n_pairs": pairs.len(),
        "calibrate": report,
        "requires_customer_labels_for_production": pairs.is_empty(),
        "note": if pairs.is_empty() {
            "no usable labels — keep heuristic_v0"
        } else {
            "lab/customer labels used for offline ECE; runtime remains heuristic until adopt"
        },
    })
}

/// Parse pairs from JSON array of {same_host:bool, score:f64, association_level?:str}.
pub fn pairs_from_json(v: &Value) -> Vec<LabeledPair> {
    v.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|p| {
                    Some(LabeledPair {
                        same_host: p.get("same_host")?.as_bool()?,
                        score: p.get("score")?.as_f64()?,
                        label_source: p
                            .get("label_source")
                            .and_then(|x| x.as_str())
                            .unwrap_or("json")
                            .to_string(),
                        association_level: p
                            .get("association_level")
                            .and_then(|x| x.as_str())
                            .map(|s| s.to_string()),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Load labeled pairs from `spec/conf_cal_labeled_pairs_v1.json` when present.
/// Used only for **reference ECE annotation** on evaluate — never changes runtime conf version.
pub fn load_spec_labeled_pairs() -> Vec<LabeledPair> {
    let candidates = [
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spec/conf_cal_labeled_pairs_v1.json"),
        std::path::PathBuf::from("spec/conf_cal_labeled_pairs_v1.json"),
    ];
    for p in &candidates {
        if let Ok(raw) = std::fs::read_to_string(p) {
            if let Ok(v) = serde_json::from_str::<Value>(&raw) {
                // Accept either { "pairs": [...] } or bare array.
                let arr = v
                    .get("pairs")
                    .cloned()
                    .unwrap_or_else(|| v.clone());
                let pairs = pairs_from_json(&arr);
                if !pairs.is_empty() {
                    return pairs;
                }
            }
        }
    }
    Vec::new()
}

/// Offline calibration snapshot for product surface (reference only).
/// Always reports `runtime_default_remains=heuristic_v0` and `silent_relabel_forbidden=true`.
pub fn confidence_calibration_ref() -> Value {
    let pairs = load_spec_labeled_pairs();
    if pairs.is_empty() {
        return json!({
            "ok": false,
            "runtime_default_remains": RUNTIME_CONFIDENCE_VERSION,
            "silent_relabel_forbidden": true,
            "note": "no labeled pairs in spec — conf stays heuristic_v0",
        });
    }
    let report = calibrate_offline(&pairs);
    json!({
        "ok": report.get("ok").and_then(|v| v.as_bool()).unwrap_or(false),
        "runtime_default_remains": RUNTIME_CONFIDENCE_VERSION,
        "silent_relabel_forbidden": true,
        "ece_like": report.get("ece_like"),
        "n_pairs": report.get("n_pairs"),
        "confidence_version_if_adopted": CALIBRATED_CONFIDENCE_VERSION,
        "note": "reference only — product conf not auto-renamed to calibrated",
        "report_subset": {
            "n_same_host": report.get("n_same_host"),
            "n_diff_host": report.get("n_diff_host"),
            "ece_like": report.get("ece_like"),
        },
    })
}

/// Explicit adopt of offline ladder calibration (never silently renames runtime conf).
/// Returns a report with `adopted: true` and suggested version — caller must opt in.
pub fn adopt_ladder_calibration(pairs: &[LabeledPair]) -> Value {
    let report = calibrate_offline_by_ladder(pairs);
    if report.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return json!({
            "adopted": false,
            "runtime_confidence_version": RUNTIME_CONFIDENCE_VERSION,
            "report": report,
            "note": "refuse adopt without non-empty labeled pairs",
            "silent_relabel_forbidden": true,
        });
    }
    json!({
        "adopted": true,
        "runtime_confidence_version_unchanged": RUNTIME_CONFIDENCE_VERSION,
        "confidence_version_if_adopted": CALIBRATED_CONFIDENCE_VERSION,
        "report": report,
        "note": "explicit adopt only — evaluate default remains heuristic_v0 until product wires this version",
        "silent_relabel_forbidden": true,
        "algo": "ladder_conf_adopt_v1",
    })
}

/// Ladder-stratified offline calibration (iss/32 X-14).
/// Runs overall `calibrate_offline` plus per-`association_level` buckets when labels present.
pub fn calibrate_offline_by_ladder(pairs: &[LabeledPair]) -> Value {
    let overall = calibrate_offline(pairs);
    if pairs.is_empty() {
        return json!({
            "ok": false,
            "error": "empty_pairs",
            "confidence_version": RUNTIME_CONFIDENCE_VERSION,
            "ladder_strata": {},
            "note": "no labels → do not emit calibrated version; keep heuristic_v0",
            "silent_relabel_forbidden": true,
            "algo": "ladder_conf_cal_v1",
        });
    }
    let mut by_level: std::collections::HashMap<String, Vec<LabeledPair>> =
        std::collections::HashMap::new();
    for p in pairs {
        let lvl = p
            .association_level
            .clone()
            .unwrap_or_else(|| "unspecified".into());
        by_level.entry(lvl).or_default().push(p.clone());
    }
    let mut strata = serde_json::Map::new();
    for (lvl, ps) in &by_level {
        let r = calibrate_offline(ps);
        strata.insert(
            lvl.clone(),
            json!({
                "n_pairs": ps.len(),
                "report": r,
            }),
        );
    }
    json!({
        "ok": true,
        "algo": "ladder_conf_cal_v1",
        "overall": overall,
        "ladder_strata": strata,
        "levels_present": by_level.keys().cloned().collect::<Vec<_>>(),
        "confidence_version_if_adopted": CALIBRATED_CONFIDENCE_VERSION,
        "runtime_default_remains": RUNTIME_CONFIDENCE_VERSION,
        "silent_relabel_forbidden": true,
        "note": "Stratified by association_level — not production 1−P_fp without large corpus",
    })
}

// ─── FS merge/split threshold calibration (iss/60 L3) ─────────────────────

/// Offline: search tau_merge/tau_split on pairs with `score` = FS pair score.
/// LabeledPair.score here is interpreted as **FS log-likelihood score** (not [0,1] conf)
/// when `label_source` contains `fs_score`; otherwise treats score as conf and maps.
pub fn calibrate_fs_thresholds(pairs: &[LabeledPair]) -> Value {
    if pairs.len() < 4 {
        return json!({
            "ok": false,
            "error": "need_ge4_pairs",
            "silent_relabel_forbidden": true,
            "note": "FS tau remains posture defaults until enough labeled pairs",
        });
    }
    // Grid search tau_merge in [1.0, 8.0], tau_split in [-2.0, 2.0]
    let mut best = (4.5_f64, 0.5_f64, 0.0_f64); // merge, split, f1
    for mi in 0..15 {
        let tau_m = 1.0 + mi as f64 * 0.5;
        for si in 0..9 {
            let tau_s = -2.0 + si as f64 * 0.5;
            if tau_s >= tau_m {
                continue;
            }
            let mut tp = 0usize;
            let mut fp = 0usize;
            let mut fn_ = 0usize;
            for p in pairs {
                let sc = p.score;
                let pred_merge = sc >= tau_m;
                if pred_merge && p.same_host {
                    tp += 1;
                } else if pred_merge && !p.same_host {
                    fp += 1;
                } else if !pred_merge && p.same_host {
                    fn_ += 1;
                }
            }
            let prec = if tp + fp == 0 {
                0.0
            } else {
                tp as f64 / (tp + fp) as f64
            };
            let rec = if tp + fn_ == 0 {
                0.0
            } else {
                tp as f64 / (tp + fn_) as f64
            };
            let f1 = if prec + rec == 0.0 {
                0.0
            } else {
                2.0 * prec * rec / (prec + rec)
            };
            if f1 > best.2 {
                best = (tau_m, tau_s, f1);
            }
        }
    }
    // ECE-like on merge decisions
    let ece_rep = calibrate_offline(pairs);
    json!({
        "ok": true,
        "algo": "fs_threshold_cal_v1",
        "tau_merge_suggested": best.0,
        "tau_split_suggested": best.1,
        "f1_at_suggested": (best.2 * 10000.0).round() / 10000.0,
        "n_pairs": pairs.len(),
        "ece_report": ece_rep,
        "adopted": false,
        "silent_relabel_forbidden": true,
        "note": "Call adopt_fs_thresholds() only with explicit ops/env; never auto-hotpath",
    })
}

/// Explicit adopt of FS thresholds (env GR_FS_TAU_ADOPT=1 + valid calibration).
pub fn adopt_fs_thresholds(tau_merge: f64, tau_split: f64) -> Value {
    if tau_split >= tau_merge {
        return json!({"ok": false, "error": "tau_split_ge_merge"});
    }
    let explicit = matches!(
        gr_abi::env::get("FS_TAU_ADOPT")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    );
    if !explicit {
        return json!({
            "ok": false,
            "adopted": false,
            "error": "set_GR_FS_TAU_ADOPT=1",
            "silent_relabel_forbidden": true,
        });
    }
    let mut g = FS_TAU_ADOPTED.lock().unwrap_or_else(|e| e.into_inner());
    *g = Some((tau_merge, tau_split));
    json!({
        "ok": true,
        "adopted": true,
        "tau_merge": tau_merge,
        "tau_split": tau_split,
        "silent_relabel_forbidden": true,
    })
}

/// Runtime FS taus if adopted, else None (caller uses MergePosture defaults).
pub fn adopted_fs_taus() -> Option<(f64, f64)> {
    let g = FS_TAU_ADOPTED.lock().unwrap_or_else(|e| e.into_inner());
    *g
}

pub fn clear_fs_tau_adopt_for_tests() {
    let mut g = FS_TAU_ADOPTED.lock().unwrap_or_else(|e| e.into_inner());
    *g = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_pairs_keep_heuristic() {
        let r = calibrate_offline(&[]);
        assert_eq!(r["ok"], false);
        assert_eq!(r["confidence_version"], RUNTIME_CONFIDENCE_VERSION);
        assert_eq!(r["silent_relabel_forbidden"], true);
    }

    #[test]
    fn labeled_pairs_emit_ece_and_cal_tag() {
        let pairs = vec![
            LabeledPair {
                same_host: true,
                score: 0.9,
                label_source: "t".into(),
                association_level: Some("hardware".into()),
            },
            LabeledPair {
                same_host: true,
                score: 0.85,
                label_source: "t".into(),
                association_level: Some("hardware".into()),
            },
            LabeledPair {
                same_host: false,
                score: 0.2,
                label_source: "t".into(),
                association_level: Some("env".into()),
            },
            LabeledPair {
                same_host: false,
                score: 0.15,
                label_source: "t".into(),
                association_level: Some("env".into()),
            },
        ];
        let r = calibrate_offline(&pairs);
        assert_eq!(r["ok"], true);
        assert_eq!(r["n_pairs"], 4);
        assert!(r["ece_like"].as_f64().is_some());
        assert_eq!(r["runtime_default_remains"], RUNTIME_CONFIDENCE_VERSION);
        assert_eq!(r["confidence_version_if_adopted"], CALIBRATED_CONFIDENCE_VERSION);

        let lad = calibrate_offline_by_ladder(&pairs);
        assert_eq!(lad["ok"], true);
        assert!(lad["ladder_strata"].as_object().is_some_and(|m| m.len() >= 2));
        let empty = calibrate_offline_by_ladder(&[]);
        assert_eq!(empty["ok"], false);
        assert_eq!(empty["silent_relabel_forbidden"], true);

        let rref = confidence_calibration_ref();
        // Spec pairs may or may not load in unit test cwd; either way must not silent-relabel.
        assert_eq!(rref["silent_relabel_forbidden"], true);
        assert_eq!(rref["runtime_default_remains"], RUNTIME_CONFIDENCE_VERSION);

        // Without explicit adopt, always heuristic.
        std::env::remove_var("GR_CONF_ADOPT");
        assert_eq!(active_confidence_version(), RUNTIME_CONFIDENCE_VERSION);
        let d = adopt_decision_report();
        assert_eq!(d["requested"], false);
        assert_eq!(d["adopted"], false);
        assert_eq!(d["silent_relabel_forbidden"], true);
    }
}
