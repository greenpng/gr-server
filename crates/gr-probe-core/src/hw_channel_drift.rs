//! F3 — per-channel cross-observation drift job (iss/50 P1-2 · iss/54 F3).
//!
//! Given a **family** of observations (same host / session family / lab multi-tick),
//! compute pairwise cosine distance on curve LSH vectors → drift_p50 / drift_p95.
//!
//! Surfaces:
//! - `channel_drift_report` — single family batch
//! - `channel_drift_job` — multi-family batch (array of families or families map)
//! - `channel_drift_job_from_dir` — scan directory of `*.json` family files
//! - `channel_drift_from_fields` — product path when `family_observations` present
//!
//! Alerts:
//! - `drift_p95` high → material unstable
//! - drift near-zero on all pairs with n≥3 → possible replay template
//!
//! Does **not** invent materials. Offline/lab/ops job; no production TSDB required
//! when callers supply a JSON batch or directory export.

use crate::device_segments::curve_lsh_public;
use crate::hw_silicon_fusion::fuse_silicon_channels;
use serde_json::{json, Map, Value};
use std::path::Path;

pub const HW_CHANNEL_DRIFT_ALGO: &str = "hw_channel_drift_v1";
pub const HW_CHANNEL_DRIFT_JOB_ALGO: &str = "hw_channel_drift_job_v1";

// Include commercial soft candidates so dual-KPI can see cp/tz structure drift.
const DEFAULT_CHANNELS: &[&str] =
    &["wg", "wg_s", "au", "gp", "timing", "fusion", "cpu", "canvas"];

fn f64_vec(v: &Value) -> Vec<f64> {
    match v {
        Value::Array(a) => a
            .iter()
            .filter_map(|x| x.as_f64().or_else(|| x.as_i64().map(|i| i as f64)))
            .filter(|x| x.is_finite())
            .collect(),
        _ => Vec::new(),
    }
}

fn hist_from_lsh_obj(v: &Value) -> Option<Vec<f64>> {
    let h = v.get("hist16")?.as_array()?;
    let xs: Vec<f64> = h
        .iter()
        .filter_map(|x| x.as_u64().map(|u| u as f64).or_else(|| x.as_f64()))
        .collect();
    if xs.len() >= 8 {
        Some(xs)
    } else {
        None
    }
}

fn cosine_distance(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 1.0;
    }
    let mut dot = 0.0;
    let mut na = 0.0;
    let mut nb = 0.0;
    for i in 0..n {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na <= 1e-12 || nb <= 1e-12 {
        return 1.0;
    }
    let cos = (dot / (na.sqrt() * nb.sqrt())).clamp(-1.0, 1.0);
    (1.0 - cos).clamp(0.0, 2.0)
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn channel_vector(fields: &Value, channel: &str) -> Option<Vec<f64>> {
    let fo = fields.as_object()?;
    match channel {
        "cpu" | "cp" => {
            let c = fo
                .get("cpu_timing_curve")
                .or_else(|| fo.get("hw_curve_cpu"))
                .map(f64_vec)
                .unwrap_or_default();
            if c.len() >= 4 {
                hist_from_lsh_obj(&curve_lsh_public(&c))
            } else {
                None
            }
        }
        "canvas" | "of" | "cv" => {
            let c = fo
                .get("hw_curve_canvas")
                .or_else(|| fo.get("canvas_noise_curve"))
                .map(f64_vec)
                .unwrap_or_default();
            if c.len() >= 4 {
                hist_from_lsh_obj(&curve_lsh_public(&c))
            } else {
                None
            }
        }
        "wg" | "webgl" => {
            let c = fo
                .get("hw_curve_webgl")
                .or_else(|| fo.get("webgl_residual_curve"))
                .map(f64_vec)
                .unwrap_or_default();
            if c.len() >= 8 {
                hist_from_lsh_obj(&curve_lsh_public(&c))
            } else {
                None
            }
        }
        "wg_s" | "lane_s" => {
            let fusion = fuse_silicon_channels(fields);
            if let Some(arr) = fusion
                .get("lane_s_curve")
                .map(f64_vec)
                .filter(|c| c.len() >= 8)
            {
                return hist_from_lsh_obj(&curve_lsh_public(&arr));
            }
            None
        }
        "au" | "audio" => {
            let c = fo.get("hw_curve_audio").map(f64_vec).unwrap_or_default();
            if c.len() >= 8 {
                hist_from_lsh_obj(&curve_lsh_public(&c))
            } else {
                None
            }
        }
        "gp" | "webgpu" => {
            let c = fo
                .get("hw_curve_webgpu")
                .or_else(|| fo.get("webgpu_compute_curve"))
                .map(f64_vec)
                .unwrap_or_default();
            if c.len() >= 8 {
                hist_from_lsh_obj(&curve_lsh_public(&c))
            } else {
                None
            }
        }
        "timing" | "eu" => {
            // Prefer residual path timings, then top-level.
            let mut c = fo.get("eu_timing_ms").map(f64_vec).unwrap_or_default();
            if c.len() < 4 {
                if let Some(arr) = fo.get("residual_paths").and_then(|v| v.as_array()) {
                    for e in arr {
                        let t = e
                            .get("eu_timing_ms")
                            .or_else(|| e.get("timing_ms"))
                            .map(f64_vec)
                            .unwrap_or_default();
                        if t.len() >= 4 {
                            c = t;
                            break;
                        }
                    }
                }
            }
            if c.len() >= 4 {
                Some(c)
            } else {
                None
            }
        }
        "fusion" => {
            let fusion = fuse_silicon_channels(fields);
            let mut v = Vec::new();
            if let Some(s) = fusion.get("same_sku_separability").and_then(|x| x.as_f64()) {
                v.push(s);
            }
            if let Some(s) = fusion.pointer("/lane_c/std").and_then(|x| x.as_f64()) {
                v.push(s);
            }
            if let Some(s) = fusion.pointer("/lane_s/std").and_then(|x| x.as_f64()) {
                v.push(s);
            }
            if let Some(s) = fusion.get("n_paths_ok").and_then(|x| x.as_f64().or_else(|| x.as_u64().map(|u| u as f64))) {
                v.push(s / 10.0);
            }
            if v.len() >= 2 {
                Some(v)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn drift_one_channel(obs: &[Value], channel: &str) -> Value {
    let mut vecs: Vec<Vec<f64>> = Vec::new();
    for o in obs {
        let fields = o.get("fields").cloned().unwrap_or_else(|| o.clone());
        if let Some(v) = channel_vector(&fields, channel) {
            vecs.push(v);
        }
    }
    if vecs.len() < 2 {
        return json!({
            "channel": channel,
            "ok": false,
            "n_obs_with_channel": vecs.len(),
            "error": "need_at_least_two_observations_with_channel",
        });
    }
    let mut dists = Vec::new();
    for i in 0..vecs.len() {
        for j in (i + 1)..vecs.len() {
            dists.push(cosine_distance(&vecs[i], &vecs[j]));
        }
    }
    dists.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p50 = percentile(&dists, 0.5);
    let p95 = percentile(&dists, 0.95);
    let mean = dists.iter().sum::<f64>() / dists.len() as f64;
    let max_d = dists.last().copied().unwrap_or(0.0);
    let min_d = dists.first().copied().unwrap_or(0.0);
    let constant_zero = max_d < 1e-9 && vecs.len() >= 3;
    let unstable = p95 > 0.35;
    json!({
        "channel": channel,
        "ok": true,
        "n_obs_with_channel": vecs.len(),
        "n_pairs": dists.len(),
        "drift_mean": (mean * 10000.0).round() / 10000.0,
        "drift_p50": (p50 * 10000.0).round() / 10000.0,
        "drift_p95": (p95 * 10000.0).round() / 10000.0,
        "drift_min": (min_d * 10000.0).round() / 10000.0,
        "drift_max": (max_d * 10000.0).round() / 10000.0,
        "alert_unstable": unstable,
        "alert_constant_zero_replay": constant_zero,
        "note": if constant_zero {
            "drift≈0 across ≥3 obs — possible replay/template"
        } else if unstable {
            "drift_p95 high — material unstable for this family"
        } else {
            "ok"
        },
    })
}

fn extract_observations(batch: &Value) -> Vec<Value> {
    batch
        .get("observations")
        .or_else(|| batch.get("batch"))
        .or_else(|| batch.get("family_observations"))
        .and_then(|v| v.as_array())
        .cloned()
        .or_else(|| batch.as_array().cloned())
        .unwrap_or_default()
}

fn family_id_of(batch: &Value, fallback: &str) -> String {
    batch
        .get("family_id")
        .or_else(|| batch.get("host_id"))
        .or_else(|| batch.get("subject_ref"))
        .or_else(|| batch.get("session_family"))
        .and_then(|v| v.as_str())
        .unwrap_or(fallback)
        .to_string()
}

/// Single-family drift report.
/// Input: `{ "family_id": "...", "observations": [ fields | {fields} ] }` or bare array.
pub fn channel_drift_report(batch: &Value) -> Value {
    let family = family_id_of(batch, "family");
    let obs = extract_observations(batch);
    if obs.len() < 2 {
        return json!({
            "algo": HW_CHANNEL_DRIFT_ALGO,
            "ok": false,
            "error": "need_at_least_two_observations",
            "family_id": family,
        });
    }
    let mut per = Vec::new();
    let mut any_zero = false;
    let mut any_unstable = false;
    let mut n_ok_ch = 0usize;
    for ch in DEFAULT_CHANNELS {
        let r = drift_one_channel(&obs, ch);
        if r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            n_ok_ch += 1;
        }
        if r.get("alert_constant_zero_replay")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            any_zero = true;
        }
        if r.get("alert_unstable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            any_unstable = true;
        }
        per.push(r);
    }
    json!({
        "algo": HW_CHANNEL_DRIFT_ALGO,
        "ok": true,
        "family_id": family,
        "n_observations": obs.len(),
        "n_channels_ok": n_ok_ch,
        "channels": per,
        "family_alert_constant_zero_replay": any_zero,
        "family_alert_unstable": any_unstable,
        "drives_production_gate": drift_gate_enabled(),
        "note": "iss/54 F3 + iss/67 C2: set GR_CHANNEL_DRIFT_GATE=1 to allow production demote consumers",
    })
}

/// Feature flag: when true, drift alerts may drive demote/deepen (still not hard-block).
/// iss/73 P2: auto-enable when `GR_CHANNEL_DRIFT_AUTO=1` and reliability file says pass.
pub fn drift_gate_enabled() -> bool {
    match gr_abi::env::get("CHANNEL_DRIFT_GATE") {
        Some(v) => v == "1" || v == "true" || v == "on",
        None => {
            // Auto path: lab/ops write data/reliability_gate.json {"pass":true}
            if match gr_abi::env::get("CHANNEL_DRIFT_AUTO") {
                Some(v) => v == "1" || v == "true" || v == "on",
                None => false,
            } {
                reliability_gate_file_pass()
            } else {
                false
            }
        }
    }
}

/// Read reliability gate file produced by dual-session / same-SKU KPI scripts.
fn reliability_gate_file_pass() -> bool {
    let path = gr_abi::env::get("RELIABILITY_GATE_FILE")
        .unwrap_or_else(|| "data/reliability_gate.json".into());
    if let Ok(txt) = std::fs::read_to_string(path) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) {
            return v.get("pass").and_then(|x| x.as_bool()).unwrap_or(false)
                || v.get("reliability_pass")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(false);
        }
    }
    false
}

/// Compute reliability summary from same-VT family digests (PUF-style).
/// Returns inter-slot uniqueness proxy + intra agree rates when history present.
pub fn reliability_kpi_from_family(fields: &serde_json::Value) -> serde_json::Value {
    let gate = stability_gate_from_fields(fields);
    let stable = gate
        .get("stable")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let ok = gate.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    let n_ok = gate
        .get("n_channels_ok")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    // Recommend opening drift gate only with multi-session evidence
    let recommend_open = ok && stable && n_ok >= 3;
    serde_json::json!({
        "algo": "reliability_kpi_v1",
        "stability_gate": gate,
        "recommend_open_drift_gate": recommend_open,
        "drift_gate_enabled": drift_gate_enabled(),
        "note": "iss/73 P2: set GR_CHANNEL_DRIFT_GATE=1 or write data/reliability_gate.json {\"pass\":true}",
    })
}

// ─── Same-VT multi-session digest history (iss/69 U3 producer) ─────────────

const VT_FAMILY_SHARED_KEY: &str = "vt_family_v1";
const VT_FAMILY_RING: usize = 6;
const VT_FAMILY_TTL_MS: u64 = 72 * 3600 * 1000;
const VT_FAMILY_SLOTS: &[&str] = &["res", "wg", "au", "cp", "of", "tz"];

fn vf_now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Record this session's per-slot digests under the VT (device_id) key.
/// Ring-capped per slot; TTL-pruned; flushed to shared JSON (multi-worker safe).
pub fn observe_vt_family(device_id: &str, digests: &Value) {
    if device_id.is_empty() || digests.is_null() {
        return;
    }
    use crate::shared_governance::{as_object_mut, with_shared_json};
    if crate::shared_governance::shared_governance_dir().is_none() {
        return;
    }
    let t = vf_now_ms();
    let _ = with_shared_json(VT_FAMILY_SHARED_KEY, json!({}), |v| {
        let o = as_object_mut(v);
        let fam = o
            .entry(device_id.to_string())
            .or_insert(json!({}))
            .as_object_mut()
            .unwrap();
        let slots = fam
            .entry("slots")
            .or_insert(json!({}))
            .as_object_mut()
            .unwrap();
        if let Some(obj) = digests.as_object() {
            for (slot, dv) in obj {
                if !VT_FAMILY_SLOTS.contains(&slot.as_str()) {
                    continue;
                }
                let Some(s) = dv.as_str() else { continue };
                if s.is_empty() || s == "0" {
                    continue;
                }
                let ring = slots
                    .entry(slot.clone())
                    .or_insert(json!([]))
                    .as_array_mut()
                    .unwrap();
                ring.push(json!(s));
                if ring.len() > VT_FAMILY_RING {
                    let drop = ring.len() - VT_FAMILY_RING;
                    ring.drain(0..drop);
                }
            }
        }
        fam.insert("ts".into(), json!(t));
    });
}

/// Attach `family_digests` ({slot: [digest,...], ≥2 sessions}) for the VT onto
/// the fields so `stability_gate_from_fields` can compute digest-agree stability.
/// No-op when the store is disabled, empty, or the device has <2 sessions.
pub fn attach_vt_family_digests(fields: &mut Value, device_id: &str) {
    if device_id.is_empty() {
        return;
    }
    use crate::shared_governance::with_shared_json;
    if crate::shared_governance::shared_governance_dir().is_none() {
        return;
    }
    let t = vf_now_ms();
    let fam = with_shared_json(VT_FAMILY_SHARED_KEY, json!({}), |v| {
        v.get(device_id)
            .and_then(|x| x.as_object())
            .map(|o| {
                json!({
                    "slots": o.get("slots").and_then(|x| x.as_object()).cloned().unwrap_or_default(),
                    "ts": o.get("ts").and_then(|x| x.as_u64()).unwrap_or(0),
                })
            })
            .unwrap_or_default()
    })
    .unwrap_or_default();
    let stale = fam
        .get("ts")
        .and_then(|x| x.as_u64())
        .map(|ts| t.saturating_sub(ts) >= VT_FAMILY_TTL_MS)
        .unwrap_or(true);
    if stale {
        return;
    }
    let mut out = Map::new();
    if let Some(slots) = fam.get("slots").and_then(|x| x.as_object()) {
        for (slot, arr) in slots {
            let n = arr.as_array().map(|a| a.len()).unwrap_or(0);
            if n >= 2 {
                out.insert(slot.clone(), arr.clone());
            }
        }
    }
    if !out.is_empty() {
        if let Some(o) = fields.as_object_mut() {
            o.insert("family_digests".into(), json!(out));
        }
    }
}

/// Stability gate surface for pure-HW slots (iss/67 C2).
/// Two modes: `family_digests` (same-VT digest agree, producer: observe_vt_family)
/// or `family_observations`/`observations` (curve drift).
pub fn stability_gate_from_fields(fields: &Value) -> Value {
    // iss/69 U3: digest-agree stability from same-VT multi-session history.
    if let Some(fd) = fields.get("family_digests") {
        let mut slots = Map::new();
        let mut any = false;
        let mut all_stable = true;
        if let Some(obj) = fd.as_object() {
            for (slot, arr) in obj {
                let digs: Vec<&str> = arr
                    .as_array()
                    .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
                    .unwrap_or_default();
                let n = digs.len();
                if n < 2 {
                    continue;
                }
                any = true;
                use std::collections::HashMap;
                let mut counts: HashMap<&str, usize> = HashMap::new();
                for d in &digs {
                    *counts.entry(d).or_default() += 1;
                }
                let maxc = counts.values().copied().max().unwrap_or(0);
                let agree = maxc as f64 / n as f64;
                all_stable = all_stable && agree >= 0.8;
                slots.insert(
                    slot.clone(),
                    json!({"n": n, "agree": (agree * 10000.0).round() / 10000.0}),
                );
            }
        }
        if any {
            return json!({
                "algo": "hw_stability_gate_v1",
                "ok": true,
                "stable": all_stable,
                "unstable": !all_stable,
                "constant_zero_replay": false,
                "mode": "digest_agree_v1",
                "slots": slots,
                "drives_production_gate": drift_gate_enabled(),
                "hard_block": false,
                "note": "iss/67 C2: same-VT multi-session digest agree rate >= 0.8",
            });
        }
    }
    let mut body = fields.clone();
    if fields.get("family_observations").is_none() && fields.get("observations").is_none() {
        return serde_json::json!({
            "algo": HW_CHANNEL_DRIFT_ALGO,
            "ok": false,
            "stable": true,
            "drives_production_gate": drift_gate_enabled(),
            "error": "no_family_observations",
            "note": "attach family_observations/family_digests for same-VT multi-session stability",
        });
    }
    if body.get("family_id").is_none() {
        if let Some(o) = body.as_object_mut() {
            o.insert(
                "family_id".into(),
                fields
                    .get("visitor_terminal_id")
                    .cloned()
                    .unwrap_or(serde_json::json!("family")),
            );
        }
    }
    let rep = channel_drift_report(&body);
    let unstable = rep
        .get("family_alert_unstable")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let zero_replay = rep
        .get("family_alert_constant_zero_replay")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    serde_json::json!({
        "algo": "hw_stability_gate_v1",
        "ok": rep.get("ok").and_then(|v| v.as_bool()).unwrap_or(false),
        "stable": !unstable && !zero_replay,
        "unstable": unstable,
        "constant_zero_replay": zero_replay,
        "drives_production_gate": drift_gate_enabled(),
        "drift_report": rep,
        "hard_block": false,
    })
}

/// Multi-family job body:
/// - `{ "families": [ {family_id, observations}, ... ] }`
/// - `{ "families": { "id1": {observations}, ... } }`
/// - bare array of family objects
/// - single family object (falls through to report)
pub fn channel_drift_job(input: &Value) -> Value {
    let mut reports: Vec<Value> = Vec::new();

    if let Some(arr) = input.get("families").and_then(|v| v.as_array()) {
        for (i, fam) in arr.iter().enumerate() {
            let mut body = fam.clone();
            if body.get("family_id").is_none() {
                if let Some(o) = body.as_object_mut() {
                    o.insert("family_id".into(), json!(format!("family_{i}")));
                }
            }
            reports.push(channel_drift_report(&body));
        }
    } else if let Some(map) = input.get("families").and_then(|v| v.as_object()) {
        for (id, fam) in map {
            let body = if fam.is_object() {
                let mut o = fam.clone();
                if o.get("family_id").is_none() {
                    if let Some(m) = o.as_object_mut() {
                        m.insert("family_id".into(), json!(id));
                    }
                }
                o
            } else if fam.is_array() {
                json!({ "family_id": id, "observations": fam })
            } else {
                continue;
            };
            reports.push(channel_drift_report(&body));
        }
    } else if let Some(arr) = input.as_array() {
        // Heuristic: array of families if elements have observations; else single family obs list
        let looks_like_families = arr.iter().any(|x| {
            x.get("observations").is_some() || x.get("family_id").is_some() || x.get("batch").is_some()
        });
        if looks_like_families {
            for (i, fam) in arr.iter().enumerate() {
                let mut body = fam.clone();
                if body.get("family_id").is_none() && body.get("observations").is_some() {
                    if let Some(o) = body.as_object_mut() {
                        o.insert("family_id".into(), json!(format!("family_{i}")));
                    }
                }
                reports.push(channel_drift_report(&body));
            }
        } else {
            return channel_drift_report(input);
        }
    } else if input.get("observations").is_some() || input.get("batch").is_some() {
        return channel_drift_report(input);
    } else {
        return json!({
            "algo": HW_CHANNEL_DRIFT_JOB_ALGO,
            "ok": false,
            "error": "need_families_or_observations",
        });
    }

    let n_ok = reports
        .iter()
        .filter(|r| r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false))
        .count();
    let n_replay = reports
        .iter()
        .filter(|r| {
            r.get("family_alert_constant_zero_replay")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        })
        .count();
    let n_unstable = reports
        .iter()
        .filter(|r| {
            r.get("family_alert_unstable")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        })
        .count();

    json!({
        "algo": HW_CHANNEL_DRIFT_JOB_ALGO,
        "ok": !reports.is_empty(),
        "n_families": reports.len(),
        "n_families_ok": n_ok,
        "n_families_replay_alert": n_replay,
        "n_families_unstable_alert": n_unstable,
        "families": reports,
        "drives_production_gate": false,
        "note": "iss/54 F3 multi-family job — schedule against exported session-family JSON dir",
    })
}

/// Scan a directory of `*.json` family batch files and run the job.
pub fn channel_drift_job_from_dir(dir: &Path) -> Value {
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) => {
            return json!({
                "algo": HW_CHANNEL_DRIFT_JOB_ALGO,
                "ok": false,
                "error": format!("read_dir: {e}"),
                "dir": dir.display().to_string(),
            })
        }
    };
    let mut families = Vec::new();
    let mut errors = Vec::new();
    for ent in rd.flatten() {
        let p = ent.path();
        if p.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match std::fs::read_to_string(&p) {
            Ok(t) => match serde_json::from_str::<Value>(&t) {
                Ok(mut v) => {
                    if v.get("family_id").is_none() {
                        if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                            if let Some(o) = v.as_object_mut() {
                                o.insert("family_id".into(), json!(stem));
                            }
                        }
                    }
                    // Single fields file → wrap as 1-obs (will fail min-2, skip noise)
                    if v.get("observations").is_none()
                        && v.get("batch").is_none()
                        && !v.is_array()
                        && (v.get("hw_curve_webgl").is_some()
                            || v.get("residual_paths").is_some()
                            || v.get("fields").is_some())
                    {
                        // allow multi-tick via sibling naming: family_id from stem
                        // leave as single-obs family; report will mark need_at_least_two
                        let fields = v.get("fields").cloned().unwrap_or(v.clone());
                        v = json!({
                            "family_id": p.file_stem().and_then(|s| s.to_str()).unwrap_or("f"),
                            "observations": [fields],
                        });
                    }
                    families.push(v);
                }
                Err(e) => errors.push(json!({"path": p.display().to_string(), "error": e.to_string()})),
            },
            Err(e) => errors.push(json!({"path": p.display().to_string(), "error": e.to_string()})),
        }
    }
    let mut job = channel_drift_job(&json!({ "families": families }));
    if let Some(o) = job.as_object_mut() {
        o.insert("dir".into(), json!(dir.display().to_string()));
        o.insert("n_files_errors".into(), json!(errors.len()));
        if !errors.is_empty() {
            o.insert("file_errors".into(), json!(errors));
        }
    }
    job
}

/// Product path: if fields carry `family_observations` (≥2), compute drift; else status-only.
pub fn channel_drift_from_fields(fields: &Value) -> Value {
    let fo = fields.as_object();
    let fam_obs = fo
        .and_then(|m| {
            m.get("family_observations")
                .or_else(|| m.get("session_family_observations"))
                .or_else(|| m.get("multi_obs"))
        })
        .and_then(|v| v.as_array())
        .cloned();
    if let Some(obs) = fam_obs {
        if obs.len() >= 2 {
            let family = fo
                .and_then(|m| {
                    m.get("family_id")
                        .or_else(|| m.get("host_id"))
                        .or_else(|| m.get("subject_ref"))
                        .and_then(|v| v.as_str())
                })
                .unwrap_or("fields_family");
            return channel_drift_report(&json!({
                "family_id": family,
                "observations": obs,
            }));
        }
    }
    json!({
        "algo": HW_CHANNEL_DRIFT_ALGO,
        "ok": false,
        "pending_multi_obs": true,
        "drives_production_gate": false,
        "note": "attach family_observations[≥2] on fields, or run channel_drift_job / CLI",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    fn obs(seed: f64) -> Value {
        let c: Vec<f64> = (0..32)
            .map(|i| ((i as f64) * 0.17 + seed).sin().abs() * 0.4 + 0.05)
            .collect();
        json!({
            "hw_curve_webgl": c.clone(),
            "hw_curve_audio": (0..64).map(|i| ((i as f64)*0.11+seed).cos().abs()*0.3).collect::<Vec<_>>(),
            "eu_timing_ms": (0..8).map(|i| 1.0+seed+i as f64*0.05).collect::<Vec<_>>(),
            "residual_paths": [
                {"path_id":"ulp","shader_mode":"ulp","ok":true,"curve": c}
            ],
        })
    }

    #[test]
    fn drift_detects_variation() {
        let batch = json!({
            "family_id": "host_a",
            "observations": [obs(0.1), obs(0.5), obs(0.9)],
        });
        let r = channel_drift_report(&batch);
        assert_eq!(r["ok"], true);
        let ch = r["channels"].as_array().unwrap();
        assert!(ch.len() >= 3);
    }

    #[test]
    fn identical_obs_near_zero_drift() {
        let o = obs(0.2);
        let batch = json!({
            "family_id": "replay",
            "observations": [o.clone(), o.clone(), o.clone()],
        });
        let r = channel_drift_report(&batch);
        assert_eq!(r["ok"], true);
        assert_eq!(r["family_alert_constant_zero_replay"], true);
    }

    #[test]
    fn multi_family_job() {
        let job = channel_drift_job(&json!({
            "families": [
                {"family_id": "a", "observations": [obs(0.1), obs(0.2), obs(0.3)]},
                {"family_id": "b", "observations": [obs(0.5), obs(0.6)]},
            ]
        }));
        assert_eq!(job["ok"], true);
        assert_eq!(job["n_families"], 2);
    }

    #[test]
    fn from_fields_family_observations() {
        let fields = json!({
            "family_id": "h1",
            "family_observations": [obs(0.1), obs(0.2), obs(0.3)],
        });
        let r = channel_drift_from_fields(&fields);
        assert_eq!(r["ok"], true);
        assert_eq!(r["family_id"], "h1");
    }

    #[test]
    fn job_from_dir_reads_json() {
        let dir = std::env::temp_dir().join(format!("gr_drift_job_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fam_x.json");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            "{}",
            json!({
                "family_id": "fam_x",
                "observations": [obs(0.1), obs(0.4), obs(0.7)],
            })
        )
        .unwrap();
        let r = channel_drift_job_from_dir(&dir);
        assert_eq!(r["ok"], true);
        assert!(r["n_families"].as_u64().unwrap_or(0) >= 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn digest_gate_unstable_on_poor_agree() {
        let f = json!({
            "family_digests": {
                "res": ["d1", "d1", "d1", "d1"],
                "wg": ["d9", "d9", "d8"],
            }
        });
        let r = stability_gate_from_fields(&f);
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["stable"], false, "wg 2/3 agree < 0.8 → unstable: {r}");
        assert_eq!(r["mode"], "digest_agree_v1");
        assert_eq!(r["hard_block"], false);
    }

    #[test]
    fn digest_gate_unstable_when_digests_flap() {
        let f = json!({
            "family_digests": {
                "res": ["d1", "d2", "d3", "d4"],
            }
        });
        let r = stability_gate_from_fields(&f);
        assert_eq!(r["ok"], true);
        assert_eq!(r["stable"], false, "{r}");
        assert_eq!(r["slots"]["res"]["agree"], 0.25, "{r}");
    }

    #[test]
    fn vt_family_store_roundtrip() {
        let _guard = crate::shared_governance::ISS58_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "gr_vt_family_test_{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        crate::shared_governance::set_shared_governance_dir_for_tests(Some(dir.clone()));
        let vt = "dv0-aaaaaaaaaaaaaaaa";
        observe_vt_family(vt, &json!({"res": "r1", "wg": "w1", "cp": "c1"}));
        observe_vt_family(vt, &json!({"res": "r1", "wg": "w2", "cp": "c2"}));
        observe_vt_family(vt, &json!({"res": "r1", "wg": "w2", "cp": "c2"}));
        let mut fields = json!({"engine_family": "blink", "os_family": "windows"});
        attach_vt_family_digests(&mut fields, vt);
        assert!(
            fields.get("family_digests").is_some(),
            "family_digests should attach after 3 sessions: {fields}"
        );
        let g = stability_gate_from_fields(&fields);
        assert_eq!(g["ok"], true, "{g}");
        // res 3/3 agree (stable), wg 2/3 agree (unstable) → overall unstable
        assert_eq!(g["stable"], false, "{g}");
        assert_eq!(g["slots"]["res"]["agree"], 1.0, "{g}");
        crate::shared_governance::clear_thread_shared_governance_override();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
