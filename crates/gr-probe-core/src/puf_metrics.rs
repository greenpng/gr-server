//! A6: PUF dual-KPI scaffold (iss/74 §4-A6).
//!
//! Server-side report structure for device_id V-slot calibration. PUF
//! (Physically Unclonable Function) material — seed replays (B10x), WebGPU
//! atomic contention histograms (B92), GPU EU timing curves (B95) — is scored
//! on two independent KPIs before it may enter the device_id V slot:
//!
//! - uniqueness: cross-IP / cross-session collision rate in the lab real-machine
//!   matrix (a material whose digest repeats on many machines is not unique).
//! - reliability: same-VT repeat consistency rate (same machine must reproduce
//!   its material across repeats; drifting/unstable observations are unusable).
//!
//! ONLY materials passing both KPIs enter the V slot (`device_id_v_gate`).
//! Existing iss/67 data is the first baseline; thresholds stay
//! `lab_pending` until the real-machine calibration matrix lands (doc07 §2.1).
//!
//! Pure function of a sample *window* — no store dependency. The lab harness /
//! server analysis reads sessions and feeds the window; this module only scores.

use std::sync::OnceLock;

use serde_json::{json, Map, Value};

/// Kinds of PUF-capable material the framework knows.
pub const PUF_KINDS: &[&str] = &["seed_replay", "contention_hist", "eu_timing"];

/// Minimum distinct sessions for a uniqueness verdict (lab matrix floor).
pub const UNIQUENESS_MIN_SESSIONS: usize = 30;
/// Uniqueness pass threshold: collisions must be ≤2% of sampled sessions.
pub const UNIQUENESS_PASS_RATE: f64 = 0.98;
/// Minimum repeats for a reliability verdict.
pub const RELIABILITY_MIN_REPEATS: usize = 10;
/// Reliability pass threshold: same-VT repeat consistency ≥97%.
pub const RELIABILITY_PASS_RATE: f64 = 0.97;

/// A6 dual-KPI thresholds resolved from the lab calibration SSOT
/// (`spec/puf_calibration.json`). Lab recalibration edits only the spec file;
/// a missing/invalid file falls back to the built-in consts above.
#[derive(Debug, Clone, Copy)]
pub struct PufCalibration {
    pub uniqueness_min_sessions: usize,
    pub uniqueness_pass_rate: f64,
    pub reliability_min_repeats: usize,
    pub reliability_pass_rate: f64,
}

impl Default for PufCalibration {
    fn default() -> Self {
        PufCalibration {
            uniqueness_min_sessions: UNIQUENESS_MIN_SESSIONS,
            uniqueness_pass_rate: UNIQUENESS_PASS_RATE,
            reliability_min_repeats: RELIABILITY_MIN_REPEATS,
            reliability_pass_rate: RELIABILITY_PASS_RATE,
        }
    }
}

/// Parse `spec/puf_calibration.json` payload
/// `{uniqueness: {min_sessions, pass_rate}, reliability: {min_repeats,
/// pass_rate}}`. Malformed payload → `None` (callers fall back to defaults).
pub fn parse_puf_calibration(raw: &Value) -> Option<PufCalibration> {
    let u = raw.get("uniqueness")?;
    let r = raw.get("reliability")?;
    Some(PufCalibration {
        uniqueness_min_sessions: usize::try_from(u.get("min_sessions")?.as_u64()?).ok()?,
        uniqueness_pass_rate: u.get("pass_rate")?.as_f64()?,
        reliability_min_repeats: usize::try_from(r.get("min_repeats")?.as_u64()?).ok()?,
        reliability_pass_rate: r.get("pass_rate")?.as_f64()?,
    })
}

/// Loaded once per process from `spec/puf_calibration.json`.
fn puf_calibration() -> PufCalibration {
    static CAL: OnceLock<PufCalibration> = OnceLock::new();
    *CAL.get_or_init(|| {
        let spec_dir = crate::contracts::find_spec_dir();
        let path = spec_dir.join("puf_calibration.json");
        let raw: Value = match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(_) => return PufCalibration::default(),
            },
            Err(_) => return PufCalibration::default(),
        };
        parse_puf_calibration(&raw).unwrap_or_default()
    })
}

/// "spec:puf_calibration.json" when the SSOT file was loaded, else the
/// built-in lab-pending marker (kept in the report for lab traceability).
fn calibration_source() -> &'static str {
    match crate::contracts::find_spec_dir()
        .join("puf_calibration.json")
        .try_exists()
        .unwrap_or(false)
    {
        true => "spec:puf_calibration.json",
        false => "lab_pending_until_real_machine_matrix",
    }
}

/// One observation of one PUF material.
#[derive(Debug, Clone)]
pub struct PufSample {
    pub kind: String,
    pub digest: String,
    pub session_id: String,
    pub vt_id: String,
    /// 0 = first observation of the VT; >0 = repeat round.
    pub round: u64,
    /// Observation usable for scoring (ok / not-skip / stable bit).
    pub stable: bool,
}

/// Extract PUF-capable material samples from one evidence fields map
/// (single-session; the lab/server harness aggregates them into a window).
pub fn puf_materials_from_fields(
    fields: &Map<String, Value>,
    session_id: &str,
    vt_id: &str,
) -> Vec<PufSample> {
    let mut out: Vec<PufSample> = Vec::new();

    // B10x seed replay: agreement bits + resettable digests (seed_replay_*).
    let replay_digest = str_nonempty(fields, "seed_residual_digest")
        .or_else(|| str_nonempty(fields, "seed_ulp_digest"))
        .unwrap_or_default();
    if fields.contains_key("seed_replay_agree_0p001") && !replay_digest.is_empty() {
        let agree = fields
            .get("seed_replay_agree_0p001")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        out.push(PufSample {
            kind: "seed_replay".into(),
            digest: replay_digest.clone(),
            session_id: session_id.to_string(),
            vt_id: vt_id.to_string(),
            round: fields
                .get("seed_replay_round")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
            stable: agree,
        });
    }

    // B92 WebGPU atomic contention histogram (when built).
    if let Some(hist) = fields.get("atomic_contention_hist16").and_then(|v| v.as_array()) {
        let digest = str_nonempty(fields, "atomic_contention_digest").unwrap_or_default();
        if !digest.is_empty() && !hist.is_empty() {
            let ok = fields
                .get("atomic_contention_ok")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            out.push(PufSample {
                kind: "contention_hist".into(),
                digest,
                session_id: session_id.to_string(),
                vt_id: vt_id.to_string(),
                round: fields
                    .get("atomic_contention_round")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                stable: ok,
            });
        }
    }

    // B95 GPU EU timing curve (when built).
    if let Some(curve) = fields.get("gpu_eu_curve").and_then(|v| v.as_array()) {
        let digest = str_nonempty(fields, "gpu_eu_digest").unwrap_or_default();
        if !digest.is_empty() && !curve.is_empty() {
            let ok = fields
                .get("gpu_eu_ok")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            out.push(PufSample {
                kind: "eu_timing".into(),
                digest,
                session_id: session_id.to_string(),
                vt_id: vt_id.to_string(),
                round: fields.get("gpu_eu_round").and_then(|v| v.as_u64()).unwrap_or(0),
                stable: ok,
            });
        }
    }

    out
}

fn str_nonempty(fields: &Map<String, Value>, k: &str) -> Option<String> {
    fields
        .get(k)
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// One kind's dual-KPI verdicts.
struct KindReport {
    kind: String,
    samples_n: usize,
    sessions_n: usize,
    vts_n: usize,
    collision_sessions: usize,
    uniqueness_rate: f64,
    uniqueness_status: &'static str,
    repeats_n: usize,
    stable_repeats: usize,
    reliability_rate: f64,
    reliability_status: &'static str,
    kpi: &'static str,
}

fn score_kind(kind: &str, samples: &[PufSample], cal: &PufCalibration) -> KindReport {
    // Uniqueness: digest collisions across different VTs.
    let mut seen: Vec<(&str, &str)> = Vec::new(); // (vt, digest)
    let mut sessions_n = 0usize;
    for s in samples {
        if seen.iter().any(|(vt, d)| *vt == s.vt_id && *d == s.digest) {
            continue;
        }
        seen.push((s.vt_id.as_str(), s.digest.as_str()));
        sessions_n += 1;
    }
    // Cross-VT collisions only: same-VT repeats (round > 0) intentionally
    // reproduce the digest and must NOT count against uniqueness.
    let mut digest_vts: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for s in samples {
        let v = digest_vts.entry(s.digest.clone()).or_default();
        if !v.contains(&s.vt_id) {
            v.push(s.vt_id.clone());
        }
    }
    let collision_sessions = digest_vts
        .values()
        .map(|vts| vts.len().saturating_sub(1))
        .sum::<usize>();
    let uniqueness_rate = if sessions_n > 0 {
        (sessions_n.saturating_sub(collision_sessions)) as f64 / sessions_n as f64
    } else {
        0.0
    };
    let uniqueness_status = if sessions_n < cal.uniqueness_min_sessions {
        "lab_pending"
    } else if uniqueness_rate >= cal.uniqueness_pass_rate {
        "pass"
    } else {
        "fail"
    };

    // Reliability: repeat rounds (round > 0) of the same VT must reproduce.
    let mut repeats_n = 0usize;
    let mut stable_repeats = 0usize;
    for s in samples {
        if s.round > 0 {
            repeats_n += 1;
            if s.stable {
                stable_repeats += 1;
            }
        }
    }
    let reliability_rate = if repeats_n > 0 {
        stable_repeats as f64 / repeats_n as f64
    } else {
        0.0
    };
    let reliability_status = if repeats_n < cal.reliability_min_repeats {
        "lab_pending"
    } else if reliability_rate >= cal.reliability_pass_rate {
        "pass"
    } else {
        "fail"
    };

    let kpi = match (uniqueness_status, reliability_status) {
        ("pass", "pass") => "pass",
        ("fail", _) | (_, "fail") => "suspend",
        _ => "pending_lab",
    };

    KindReport {
        kind: kind.to_string(),
        samples_n: samples.len(),
        sessions_n,
        vts_n: samples
            .iter()
            .map(|s| s.vt_id.as_str())
            .fold(Vec::new(), |mut v: Vec<&str>, vt| {
                if !v.contains(&vt) {
                    v.push(vt);
                }
                v
            })
            .len(),
        collision_sessions,
        uniqueness_rate: (uniqueness_rate * 10000.0).round() / 10000.0,
        uniqueness_status,
        repeats_n,
        stable_repeats,
        reliability_rate: (reliability_rate * 10000.0).round() / 10000.0,
        reliability_status,
        kpi,
    }
}

/// Build the `puf_metrics` dual-KPI report over a sample window.
/// Window shape: `{"samples": [{"kind","digest","session_id","vt_id","round","stable"}, …]}`.
pub fn build_puf_metrics(window: &Value) -> Value {
    let cal = puf_calibration();
    let mut samples: Vec<PufSample> = Vec::new();
    if let Some(arr) = window.get("samples").and_then(|v| v.as_array()) {
        for s in arr {
            let kind = s.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let digest = s.get("digest").and_then(|v| v.as_str()).unwrap_or("");
            if kind.is_empty() || digest.is_empty() {
                continue;
            }
            samples.push(PufSample {
                kind: kind.to_string(),
                digest: digest.to_string(),
                session_id: s.get("session_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                vt_id: s.get("vt_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                round: s.get("round").and_then(|v| v.as_u64()).unwrap_or(0),
                stable: s.get("stable").and_then(|v| v.as_bool()).unwrap_or(false),
            });
        }
    }

    let mut materials = Map::new();
    let mut eligible: Vec<String> = Vec::new();
    for kind in PUF_KINDS {
        let ks: Vec<PufSample> = samples
            .iter()
            .filter(|s| s.kind == *kind)
            .cloned()
            .collect();
        if ks.is_empty() {
            continue;
        }
        let r = score_kind(kind, &ks, &cal);
        if r.kpi == "pass" {
            eligible.push(kind.to_string());
        }
        materials.insert(
            r.kind.clone(),
            json!({
                "samples_n": r.samples_n,
                "sessions_n": r.sessions_n,
                "vts_n": r.vts_n,
                "collision_sessions": r.collision_sessions,
                "uniqueness": {
                    "rate": r.uniqueness_rate,
                    "threshold": cal.uniqueness_pass_rate,
                    "min_sessions": cal.uniqueness_min_sessions,
                    "status": r.uniqueness_status,
                },
                "reliability": {
                    "rate": r.reliability_rate,
                    "threshold": cal.reliability_pass_rate,
                    "min_repeats": cal.reliability_min_repeats,
                    "repeats_n": r.repeats_n,
                    "stable_repeats": r.stable_repeats,
                    "status": r.reliability_status,
                },
                "kpi": r.kpi,
            }),
        );
    }

    json!({
        "algo": "puf_metrics_v1",
        "baseline": "iss67_first_baseline",
        "calibration_source": calibration_source(),
        "device_id_v_slot": {
            "dual_kpi_required": true,
            "eligible_materials": eligible,
        },
        "materials": materials,
    })
}

/// V-slot gate: kind names that pass BOTH KPIs in this window.
pub fn device_id_v_gate(window: &Value) -> Vec<String> {
    let report = build_puf_metrics(window);
    report
        .get("device_id_v_slot")
        .and_then(|v| v.get("eligible_materials"))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample(kind: &str, digest: &str, vt: &str, round: u64, stable: bool) -> Value {
        json!({
            "kind": kind,
            "digest": digest,
            "session_id": format!("s_{vt}_r{round}"),
            "vt_id": vt,
            "round": round,
            "stable": stable,
        })
    }

    #[test]
    fn dual_kpi_pass_enters_v_slot() {
        let mut samples = Vec::new();
        for i in 0..40 {
            samples.push(sample("contention_hist", &format!("d{i:04x}"), &format!("vt{i}"), 0, true));
        }
        for i in 0..12 {
            samples.push(sample("contention_hist", &format!("d{i:04x}"), &format!("vt{i}"), i as u64 + 1, true));
        }
        let w = json!({"samples": samples});
        let r = build_puf_metrics(&w);
        let m = &r["materials"]["contention_hist"];
        assert_eq!(m["uniqueness"]["status"], "pass", "{m}");
        assert_eq!(m["reliability"]["status"], "pass", "{m}");
        assert_eq!(m["kpi"], "pass", "{m}");
        assert_eq!(device_id_v_gate(&w), vec!["contention_hist"]);
    }

    #[test]
    fn collision_suspends_material() {
        // 40 sessions but only 8 distinct digests → heavy collision.
        let mut samples = Vec::new();
        for i in 0..40 {
            samples.push(sample("seed_replay", &format!("d{:02x}", i % 8), &format!("vt{i}"), 0, true));
        }
        let w = json!({"samples": samples});
        let r = build_puf_metrics(&w);
        let m = &r["materials"]["seed_replay"];
        assert_eq!(m["uniqueness"]["status"], "fail", "{m}");
        assert_eq!(m["kpi"], "suspend", "{m}");
        assert!(device_id_v_gate(&w).is_empty());
    }

    #[test]
    fn reliability_fail_suspends() {
        let mut samples = Vec::new();
        for i in 0..40 {
            samples.push(sample("eu_timing", &format!("d{i:04x}"), &format!("vt{i}"), 0, true));
        }
        // 12 repeats, 4 of them unstable (drift) → below 97%.
        for i in 0..12 {
            samples.push(sample("eu_timing", &format!("d{i:04x}"), &format!("vt{i}"), i as u64 + 1, i >= 8));
        }
        let w = json!({"samples": samples});
        let r = build_puf_metrics(&w);
        let m = &r["materials"]["eu_timing"];
        assert_eq!(m["uniqueness"]["status"], "pass", "{m}");
        assert_eq!(m["reliability"]["status"], "fail", "{m}");
        assert_eq!(m["kpi"], "suspend", "{m}");
        assert!(device_id_v_gate(&w).is_empty());
    }

    #[test]
    fn insufficient_samples_lab_pending() {
        let w = json!({"samples": [sample("contention_hist", "d0001", "vt1", 0, true)]});
        let r = build_puf_metrics(&w);
        let m = &r["materials"]["contention_hist"];
        assert_eq!(m["uniqueness"]["status"], "lab_pending", "{m}");
        assert_eq!(m["reliability"]["status"], "lab_pending", "{m}");
        assert_eq!(m["kpi"], "pending_lab", "{m}");
        assert!(device_id_v_gate(&w).is_empty());
    }

    #[test]
    fn empty_window_no_materials() {
        let r = build_puf_metrics(&json!({}));
        assert_eq!(r["materials"].as_object().unwrap().len(), 0);
        assert!(device_id_v_gate(&json!({})).is_empty());
    }

    #[test]
    fn parse_calibration_valid_payload() {
        let raw = json!({
            "uniqueness": {"min_sessions": 25, "pass_rate": 0.99},
            "reliability": {"min_repeats": 12, "pass_rate": 0.95},
        });
        let cal = parse_puf_calibration(&raw).expect("valid cal");
        assert_eq!(cal.uniqueness_min_sessions, 25);
        assert_eq!(cal.uniqueness_pass_rate, 0.99);
        assert_eq!(cal.reliability_min_repeats, 12);
        assert_eq!(cal.reliability_pass_rate, 0.95);
    }

    #[test]
    fn parse_calibration_malformed_falls_back() {
        assert!(parse_puf_calibration(&json!({})).is_none());
        assert!(parse_puf_calibration(&json!({"uniqueness": {}, "reliability": {}})).is_none());
        assert!(parse_puf_calibration(&json!({"uniqueness": {"min_sessions": "x", "pass_rate": 0.9}, "reliability": {"min_repeats": 1, "pass_rate": 0.9}})).is_none());
    }

    #[test]
    fn parse_calibration_lenient_thresholds_apply() {
        // Lab-tuned thresholds change verdict boundaries without code edits:
        // min_repeats 2 + min_sessions 3 keeps the dual-KPI shape but passes
        // a tiny window that the built-in defaults would call lab_pending.
        let raw = json!({
            "uniqueness": {"min_sessions": 3, "pass_rate": 0.5},
            "reliability": {"min_repeats": 2, "pass_rate": 0.5},
        });
        let cal = parse_puf_calibration(&raw).unwrap();
        let samples: Vec<Value> = vec![
            sample("contention_hist", "d0001", "vt1", 0, true),
            sample("contention_hist", "d0002", "vt2", 0, true),
            sample("contention_hist", "d0003", "vt3", 0, true),
            sample("contention_hist", "d0001", "vt1", 1, true),
            sample("contention_hist", "d0002", "vt2", 1, true),
        ];
        // Build the same window through score_kind with the lenient table.
        let mut samples_vec: Vec<PufSample> = Vec::new();
        for s in &samples {
            samples_vec.push(PufSample {
                kind: s["kind"].as_str().unwrap().to_string(),
                digest: s["digest"].as_str().unwrap().to_string(),
                session_id: s["session_id"].as_str().unwrap().to_string(),
                vt_id: s["vt_id"].as_str().unwrap().to_string(),
                round: s["round"].as_u64().unwrap(),
                stable: s["stable"].as_bool().unwrap(),
            });
        }
        let r = score_kind("contention_hist", &samples_vec, &cal);
        assert_eq!(r.uniqueness_status, "pass");
        assert_eq!(r.reliability_status, "pass");
        assert_eq!(r.kpi, "pass");
    }

    #[test]
    fn extracts_materials_from_fields() {
        let fo = json!({
            "seed_replay_agree_0p001": true,
            "seed_residual_digest": "aabbccdd",
            "atomic_contention_hist16": [1,2,3],
            "atomic_contention_digest": "fedc",
            "atomic_contention_ok": true,
        });
        let m = puf_materials_from_fields(fo.as_object().unwrap(), "s1", "vt1");
        assert_eq!(m.len(), 2);
        assert!(m.iter().any(|x| x.kind == "seed_replay" && x.stable));
        assert!(m.iter().any(|x| x.kind == "contention_hist" && x.digest == "fedc"));
    }
}
