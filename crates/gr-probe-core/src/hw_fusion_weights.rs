//! F5 — silicon fusion weights: config load, offline fit, explicit adopt (iss/54).
//!
//! - **Defaults**: `spec/silicon_fusion_weights_v1.json`
//! - **Override**: `GR_SILICON_FUSION_WEIGHTS_PATH`
//! - **Fit**: supervised adjustment from labeled pairs `{same_host, fields_a, fields_b}`
//! - **Adopt**: explicit write only — never silent runtime mutate of process defaults
//!
//! Fit is a lightweight rank/vote adjuster (not full GBM). Output is a **candidate**
//! JSON; ops must call `adopt_fusion_weights` / set env path after review.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

pub const SILICON_FUSION_WEIGHTS_ALGO: &str = "silicon_fusion_weights_v1";
pub const SILICON_FUSION_WEIGHTS_FIT_ALGO: &str = "silicon_fusion_weights_fit_v1";

#[derive(Debug, Clone)]
pub struct FusionWeights {
    pub channels: HashMap<String, f64>,
    pub separability: HashMap<String, f64>,
    pub learned: bool,
    pub source: String,
}

impl Default for FusionWeights {
    fn default() -> Self {
        let mut channels = HashMap::new();
        channels.insert("lane_c_healthy".into(), 1.0);
        channels.insert("lane_c_dead".into(), 0.45);
        channels.insert("lane_s_healthy".into(), 1.25);
        channels.insert("lane_s_weak".into(), 0.7);
        channels.insert("timing".into(), 1.1);
        channels.insert("audio_healthy".into(), 0.95);
        channels.insert("audio_weak".into(), 0.55);
        channels.insert("webgpu_healthy".into(), 0.9);
        channels.insert("webgpu_weak".into(), 0.5);
        channels.insert("cpu".into(), 0.55);
        channels.insert("canvas".into(), 0.4);
        channels.insert("ensemble".into(), 0.85);
        channels.insert("anti_collision_composite".into(), 0.95);
        channels.insert("seed_ulp".into(), 0.35);
        channels.insert("seed_residual".into(), 0.25);
        let mut separability = HashMap::new();
        separability.insert("lane_s_entropy".into(), 0.28);
        separability.insert("timing".into(), 0.22);
        separability.insert("audio_ok".into(), 0.18);
        separability.insert("webgpu_ok".into(), 0.14);
        separability.insert("multi_fn_disagrees".into(), 0.12);
        separability.insert("n_paths_ok_ge3".into(), 0.06);
        Self {
            channels,
            separability,
            learned: false,
            source: "builtin_default".into(),
        }
    }
}

impl FusionWeights {
    pub fn get_ch(&self, k: &str, default: f64) -> f64 {
        self.channels.get(k).copied().unwrap_or(default)
    }
    pub fn get_sep(&self, k: &str, default: f64) -> f64 {
        self.separability.get(k).copied().unwrap_or(default)
    }
    pub fn to_json(&self) -> Value {
        // stable key order for human review: serde_json Map from BTree via intermediate
        let mut ch: Vec<(String, f64)> = self.channels.iter().map(|(k, v)| (k.clone(), *v)).collect();
        ch.sort_by(|a, b| a.0.cmp(&b.0));
        let mut sep: Vec<(String, f64)> =
            self.separability.iter().map(|(k, v)| (k.clone(), *v)).collect();
        sep.sort_by(|a, b| a.0.cmp(&b.0));
        let mut channels = serde_json::Map::new();
        for (k, v) in ch {
            channels.insert(k, json!(v));
        }
        let mut separability_boosts = serde_json::Map::new();
        for (k, v) in sep {
            separability_boosts.insert(k, json!(v));
        }
        json!({
            "algo": SILICON_FUSION_WEIGHTS_ALGO,
            "version": 1,
            "learned": self.learned,
            "source": self.source,
            "channels": channels,
            "separability_boosts": separability_boosts,
            "note": if self.learned {
                "learned candidate or adopted file — review before production promote"
            } else {
                "hand defaults (iss/54 F5); fit with labeled pairs then adopt explicitly"
            },
        })
    }
}

fn parse_weights(v: &Value, source: &str) -> FusionWeights {
    let mut w = FusionWeights::default();
    w.source = source.into();
    w.learned = v.get("learned").and_then(|x| x.as_bool()).unwrap_or(false);
    if let Some(obj) = v.get("channels").and_then(|x| x.as_object()) {
        for (k, val) in obj {
            if let Some(f) = val.as_f64() {
                w.channels.insert(k.clone(), f);
            }
        }
    }
    if let Some(obj) = v
        .get("separability_boosts")
        .or_else(|| v.get("separability"))
        .and_then(|x| x.as_object())
    {
        for (k, val) in obj {
            if let Some(f) = val.as_f64() {
                w.separability.insert(k.clone(), f);
            }
        }
    }
    w
}

/// Load weights once (process lifetime). Tests use `load_fusion_weights_fresh`.
pub fn load_fusion_weights() -> FusionWeights {
    static W: OnceLock<FusionWeights> = OnceLock::new();
    W.get_or_init(load_fusion_weights_fresh).clone()
}

pub fn load_fusion_weights_fresh() -> FusionWeights {
    let paths = [
        gr_abi::env::get("SILICON_FUSION_WEIGHTS_PATH").unwrap_or_default(),
        format!(
            "{}/../../spec/silicon_fusion_weights_v1.json",
            env!("CARGO_MANIFEST_DIR")
        ),
        "spec/silicon_fusion_weights_v1.json".into(),
    ];
    for p in paths {
        if p.trim().is_empty() {
            continue;
        }
        if let Ok(t) = std::fs::read_to_string(&p) {
            if let Ok(v) = serde_json::from_str::<Value>(&t) {
                return parse_weights(&v, &p);
            }
        }
    }
    FusionWeights::default()
}

/// Runtime honesty status for doctor / product / config snapshot.
pub fn fusion_weights_status() -> Value {
    let w = load_fusion_weights_fresh();
    json!({
        "algo": SILICON_FUSION_WEIGHTS_ALGO,
        "learned": w.learned,
        "source": w.source,
        "silent_adopt": false,
        "env_override": !gr_abi::env::get("SILICON_FUSION_WEIGHTS_PATH")
            .unwrap_or_default()
            .trim()
            .is_empty(),
        "channels": w.channels,
        "separability_boosts": w.separability,
        "note": "weights drive fusion blend only; never UV alone; adopt is explicit file write",
    })
}

fn digest_str(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other
            .get("digest")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        None => String::new(),
    }
}

/// Offline fit: for each labeled pair, if digests of channels
/// **agree when same_host** / **disagree when !same_host**, boost those channel weights.
/// Produces candidate weights — does **not** auto-adopt into runtime.
pub fn fit_fusion_weights_from_pairs(pairs: &[Value]) -> Value {
    let mut base = load_fusion_weights_fresh();
    if pairs.is_empty() {
        return json!({
            "algo": SILICON_FUSION_WEIGHTS_FIT_ALGO,
            "ok": false,
            "error": "empty_pairs",
            "weights": base.to_json(),
            "adopt": false,
            "note": "need labeled pairs [{same_host, fields_a, fields_b}]",
        });
    }
    let mut vote: HashMap<String, f64> = HashMap::new();
    let mut sep_vote: HashMap<String, f64> = HashMap::new();
    let mut n_used = 0usize;
    let mut n_same = 0usize;
    let mut n_diff = 0usize;
    for p in pairs {
        let same = p
            .get("same_host")
            .or_else(|| p.get("same_device"))
            .and_then(|v| v.as_bool());
        let Some(same) = same else { continue };
        if same {
            n_same += 1;
        } else {
            n_diff += 1;
        }
        let a = p
            .get("fields_a")
            .or_else(|| p.get("a"))
            .cloned()
            .unwrap_or(json!({}));
        let b = p
            .get("fields_b")
            .or_else(|| p.get("b"))
            .cloned()
            .unwrap_or(json!({}));
        let a = a.get("fields").cloned().unwrap_or(a);
        let b = b.get("fields").cloned().unwrap_or(b);
        let fa = crate::hw_silicon_fusion::fuse_silicon_channels(&a);
        let fb = crate::hw_silicon_fusion::fuse_silicon_channels(&b);
        n_used += 1;
        let checks = [
            (
                "lane_s_healthy",
                fa.get("hw_silicon_fine"),
                fb.get("hw_silicon_fine"),
            ),
            (
                "timing",
                fa.pointer("/channels/timing"),
                fb.pointer("/channels/timing"),
            ),
            (
                "audio_healthy",
                fa.pointer("/channels/audio"),
                fb.pointer("/channels/audio"),
            ),
            (
                "webgpu_healthy",
                fa.pointer("/channels/webgpu"),
                fb.pointer("/channels/webgpu"),
            ),
            (
                "lane_c_healthy",
                fa.pointer("/lane_c/digest"),
                fb.pointer("/lane_c/digest"),
            ),
            (
                "ensemble",
                fa.get("hw_silicon_fusion"),
                fb.get("hw_silicon_fusion"),
            ),
            (
                "anti_collision_composite",
                fa.pointer("/anti_collision/composite_digest")
                    .or_else(|| fa.get("anti_collision_digest")),
                fb.pointer("/anti_collision/composite_digest")
                    .or_else(|| fb.get("anti_collision_digest")),
            ),
        ];
        for (name, va, vb) in checks {
            let sa = digest_str(va);
            let sb = digest_str(vb);
            if sa.is_empty() || sb.is_empty() {
                continue;
            }
            let differ = sa != sb;
            let good = if same { !differ } else { differ };
            *vote.entry(name.into()).or_insert(0.0) += if good { 1.0 } else { -0.5 };
        }
        // Separability boosts: when !same_host and fine digests differ → boost lane_s_entropy etc.
        let fine_diff = digest_str(fa.get("hw_silicon_fine"))
            != digest_str(fb.get("hw_silicon_fine"))
            && !digest_str(fa.get("hw_silicon_fine")).is_empty();
        if !same && fine_diff {
            *sep_vote.entry("lane_s_entropy".into()).or_insert(0.0) += 1.0;
        }
        if !same
            && digest_str(fa.pointer("/channels/timing"))
                != digest_str(fb.pointer("/channels/timing"))
        {
            *sep_vote.entry("timing".into()).or_insert(0.0) += 1.0;
        }
        if same && !fine_diff && !digest_str(fa.get("hw_silicon_fine")).is_empty() {
            // stable same-host → mild boost multi path agreement signal
            *sep_vote.entry("n_paths_ok_ge3".into()).or_insert(0.0) += 0.5;
        }
    }
    if n_used == 0 {
        return json!({
            "algo": SILICON_FUSION_WEIGHTS_FIT_ALGO,
            "ok": false,
            "error": "no_usable_pairs",
            "weights": base.to_json(),
            "adopt": false,
        });
    }
    for (k, v) in &vote {
        let cur = base.get_ch(k, 1.0);
        let delta = (v / n_used as f64) * 0.18;
        let nw = (cur + delta).clamp(0.2, 2.0);
        base.channels.insert(k.clone(), (nw * 10000.0).round() / 10000.0);
    }
    for (k, v) in &sep_vote {
        let cur = base.get_sep(k, 0.1);
        let delta = (v / n_used as f64) * 0.05;
        let nw = (cur + delta).clamp(0.02, 0.5);
        base.separability
            .insert(k.clone(), (nw * 10000.0).round() / 10000.0);
    }
    base.learned = true;
    base.source = "offline_fit_v1".into();
    json!({
        "algo": SILICON_FUSION_WEIGHTS_FIT_ALGO,
        "ok": true,
        "n_pairs": pairs.len(),
        "n_used": n_used,
        "n_same_host": n_same,
        "n_diff_host": n_diff,
        "votes": vote,
        "separability_votes": sep_vote,
        "weights": base.to_json(),
        "adopt": false,
        "silent_adopt": false,
        "note": "candidate only — call adopt_fusion_weights(path) or write JSON + GR_SILICON_FUSION_WEIGHTS_PATH; never silent",
    })
}

/// Write candidate weights JSON to path (does not change process OnceLock).
pub fn write_fusion_weights_candidate(weights: &Value, path: &Path) -> Value {
    let body = if weights.get("channels").is_some() {
        weights.clone()
    } else if let Some(w) = weights.get("weights") {
        w.clone()
    } else {
        return json!({"ok": false, "error": "no_weights_object"});
    };
    let mut out = body;
    if let Some(o) = out.as_object_mut() {
        o.insert("algo".into(), json!(SILICON_FUSION_WEIGHTS_ALGO));
        o.insert(
            "written_at_ms".into(),
            json!(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0)),
        );
        if o.get("learned").is_none() {
            o.insert("learned".into(), json!(true));
        }
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(
        path,
        serde_json::to_string_pretty(&out).unwrap_or_else(|_| "{}".into()),
    ) {
        Ok(()) => json!({
            "ok": true,
            "path": path.display().to_string(),
            "algo": "fusion_weights_write_v1",
            "adopted_into_process": false,
            "note": "file written; set GR_SILICON_FUSION_WEIGHTS_PATH and restart or call reload path on next process",
            "weights": out,
        }),
        Err(e) => json!({"ok": false, "error": e.to_string(), "path": path.display().to_string()}),
    }
}

/// Explicit adopt: write weights file and set env so **this process** fresh-load path sees it.
/// Does not mutate OnceLock already initialized (document restart for long-lived service).
pub fn adopt_fusion_weights(weights: &Value, path: &Path) -> Value {
    let wr = write_fusion_weights_candidate(weights, path);
    if !wr.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        return wr;
    }
    std::env::set_var("GR_SILICON_FUSION_WEIGHTS_PATH", path.display().to_string());
    // Also push into config overlay for doctor visibility
    let _ = crate::config_snapshot::push_config_overlay(
        &json!({
            "silicon_fusion_weights_path": path.display().to_string(),
            "silicon_fusion_weights_learned": true,
            "silicon_fusion_weights_adopted": true,
        }),
        None,
    );
    let fresh = load_fusion_weights_fresh();
    json!({
        "ok": true,
        "algo": "fusion_weights_adopt_v1",
        "path": path.display().to_string(),
        "env_set": true,
        "fresh_load": fresh.to_json(),
        "process_once_lock_note": "long-lived processes that already called load_fusion_weights() keep old Until restart",
        "silent_adopt": false,
        "note": "explicit adopt complete; restart service workers to pick OnceLock",
    })
}

/// Fit then optionally write/adopt in one ops call.
/// `mode`: "fit" | "write" | "adopt"
pub fn fusion_weights_ops(pairs: &[Value], mode: &str, out_path: Option<&Path>) -> Value {
    let fit = fit_fusion_weights_from_pairs(pairs);
    if !fit.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        return fit;
    }
    match mode {
        "fit" | "" => fit,
        "write" => {
            let path = out_path
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| Path::new("data/silicon_fusion_weights_candidate.json").to_path_buf());
            let w = fit.get("weights").cloned().unwrap_or(json!({}));
            let mut out = write_fusion_weights_candidate(&w, &path);
            if let Some(o) = out.as_object_mut() {
                o.insert("fit".into(), fit);
            }
            out
        }
        "adopt" => {
            let path = out_path
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| Path::new("data/silicon_fusion_weights_adopted.json").to_path_buf());
            let w = fit.get("weights").cloned().unwrap_or(json!({}));
            let mut out = adopt_fusion_weights(&w, &path);
            if let Some(o) = out.as_object_mut() {
                o.insert("fit".into(), fit);
            }
            out
        }
        other => json!({
            "ok": false,
            "error": format!("unknown_mode:{other}"),
            "fit": fit,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fields(seed: f64) -> Value {
        let c: Vec<f64> = (0..32)
            .map(|i| ((i as f64) * 0.19 + seed).sin().abs() * 0.5 + 0.05)
            .collect();
        json!({
            "engine_family": "blink",
            "hw_curve_webgl": c.clone(),
            "hw_curve_audio": (0..64).map(|i| ((i as f64)*0.1+seed).cos().abs()*0.3).collect::<Vec<_>>(),
            "eu_timing_ms": (0..8).map(|i| 1.0+seed*2.0+i as f64*0.05).collect::<Vec<_>>(),
            "residual_paths": [
                {"path_id":"ulp","shader_mode":"ulp","ok":true,"curve": c.clone()},
                {"path_id":"rint","shader_mode":"rint","ok":true,"curve": c},
            ],
        })
    }

    #[test]
    fn default_weights_load() {
        let w = load_fusion_weights_fresh();
        assert!(w.get_ch("lane_s_healthy", 0.0) > 1.0);
    }

    #[test]
    fn fit_empty_refuses() {
        let r = fit_fusion_weights_from_pairs(&[]);
        assert_eq!(r["ok"], false);
    }

    #[test]
    fn fit_with_pairs_ok() {
        let pairs = vec![
            json!({"same_host": true, "fields_a": fields(0.1), "fields_b": fields(0.1)}),
            json!({"same_host": false, "fields_a": fields(0.1), "fields_b": fields(0.9)}),
            json!({"same_host": false, "fields_a": fields(0.2), "fields_b": fields(0.8)}),
        ];
        let r = fit_fusion_weights_from_pairs(&pairs);
        assert_eq!(r["ok"], true);
        assert_eq!(r["adopt"], false);
        assert_eq!(r["weights"]["learned"], true);
    }

    #[test]
    fn write_candidate_file() {
        let pairs = vec![json!({"same_host": false, "a": fields(0.1), "b": fields(0.9)})];
        let fit = fit_fusion_weights_from_pairs(&pairs);
        let path = std::env::temp_dir().join(format!("gr_fw_{}.json", std::process::id()));
        let w = write_fusion_weights_candidate(fit.get("weights").unwrap(), &path);
        assert_eq!(w["ok"], true);
        assert!(path.exists());
        let _ = std::fs::remove_file(&path);
    }
}
