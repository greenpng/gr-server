//! Hardware single-curve collision hardening (iss/50 · prod-178 dead residual).
//!
//! # Problem
//! Same GPU SKU + same OS + same driver often yields **identical** WebGL residual
//! class fingerprints (ANGLE hist templates). A single residual digest cannot be
//! silicon UV — product redline: 硅级同 SKU 硬身份物理不可达.
//!
//! # Industry / open-source references (design inputs)
//! - Cao et al. WebGL FP / UNIGL (USENIX SEC'19): FP ops drive cross-device deltas
//! - DrawnApart: GPU execution-unit **timing** as secondary silicon channel
//! - FingerprintJS / ThumbmarkJS: multi-signal fusion, never single-hash UV
//! - Existing in-repo: dual-lane residual (lane_c stable / lane_s ulp), eu_timing
//!
//! # Solution (this module)
//! 1. **Multi-function ensemble**: fuse float/rint/ulp residual_paths digests
//! 2. **Teaching params**: adaptive quanta by entropy class (dead/mid/high)
//! 3. **Timing phase**: eu_timing / draw-latency digests when residual collides
//! 4. **Complex moments**: cross-path correlation features (not just mean hash)
//! 5. Honest `collision_risk` + never promote dead residual alone to UV
//!
//! Commercial digest may absorb ensemble/timing **only as secondary anchors**
//! when residual is low-entropy or multi-path disagrees — not as fake silicon UV.

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

pub const HW_ANTI_COLLISION_ALGO: &str = "hw_anti_collision_v1";

/// Adaptive quanta / teaching parameters by residual quality class.
#[derive(Debug, Clone, Copy)]
pub struct TeachingParams {
    /// Residual magnitude quanta for ensemble (smaller = finer).
    pub mag_quanta: f64,
    /// Mean quanta for path means.
    pub mean_quanta: f64,
    /// Timing ms quanta.
    pub timing_ms_quanta: f64,
    /// Whether residual alone may claim machine-tier uniqueness.
    pub residual_alone_uv_ok: bool,
    pub class: &'static str,
}

impl TeachingParams {
    pub fn from_entropy(entropy_ok: bool, std: f64, uniq_hint: usize) -> Self {
        if !entropy_ok || std < 0.010 {
            // Dead class template — refuse residual-alone UV; coarse ensemble for cluster
            TeachingParams {
                mag_quanta: 0.05,
                mean_quanta: 0.01,
                timing_ms_quanta: 0.5,
                residual_alone_uv_ok: false,
                class: "dead",
            }
        } else if std < 0.04 || uniq_hint < 12 {
            TeachingParams {
                mag_quanta: 0.03,
                mean_quanta: 0.005,
                timing_ms_quanta: 0.25,
                residual_alone_uv_ok: false,
                class: "mid",
            }
        } else {
            // High structure — still no residual-alone UV promise; finer for ensemble split
            TeachingParams {
                mag_quanta: 0.015,
                mean_quanta: 0.002,
                timing_ms_quanta: 0.1,
                residual_alone_uv_ok: false,
                class: "high",
            }
        }
    }
}

fn f64_vec(v: &Value) -> Vec<f64> {
    match v {
        Value::Array(a) => a
            .iter()
            .filter_map(|x| x.as_f64().or_else(|| x.as_i64().map(|i| i as f64)))
            .filter(|x| x.is_finite())
            .collect(),
        Value::String(s) => {
            // sparse "a,b,c" fallback
            s.split(',')
                .filter_map(|p| p.trim().parse::<f64>().ok())
                .filter(|x| x.is_finite())
                .collect()
        }
        _ => Vec::new(),
    }
}

fn curve_std(xs: &[f64]) -> f64 {
    if xs.len() < 2 {
        return 0.0;
    }
    let n = xs.len() as f64;
    let mean = xs.iter().sum::<f64>() / n;
    let var = xs.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n;
    var.sqrt()
}

fn curve_mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    xs.iter().sum::<f64>() / xs.len() as f64
}

/// Complex multi-seed curve transform (server-side) for ensemble features.
/// Mixes polynomial, folding, and cross-lag products — not a simple mean hash.
pub fn complex_curve_features(curve: &[f64], params: &TeachingParams) -> Vec<f64> {
    if curve.len() < 4 {
        return Vec::new();
    }
    let n = curve.len();
    let mut out = Vec::with_capacity(24);
    // order stats
    let mut sorted: Vec<f64> = curve.iter().copied().filter(|x| x.is_finite()).collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let q = |p: f64| {
        let i = ((sorted.len() as f64 - 1.0) * p).round() as usize;
        sorted[i.min(sorted.len() - 1)]
    };
    out.push(q(0.1));
    out.push(q(0.25));
    out.push(q(0.5));
    out.push(q(0.75));
    out.push(q(0.9));
    // moments
    let mean = curve_mean(curve);
    let std = curve_std(curve);
    out.push(mean);
    out.push(std);
    // lag-1 autocorr (shape, not just mass)
    if n >= 4 && std > 1e-12 {
        let mut num = 0.0;
        for i in 1..n {
            num += (curve[i] - mean) * (curve[i - 1] - mean);
        }
        out.push(num / ((n - 1) as f64 * std * std));
    } else {
        out.push(0.0);
    }
    // energy in odd vs even bins (ANGLE templates often balance; silicon ULP unbalance)
    let mut odd = 0.0;
    let mut even = 0.0;
    for (i, v) in curve.iter().enumerate() {
        if i % 2 == 0 {
            even += v.abs();
        } else {
            odd += v.abs();
        }
    }
    let den = (odd + even).max(1e-12);
    out.push(odd / den);
    // top-k mag ranks
    let mut mags: Vec<f64> = curve.iter().map(|v| v.abs()).collect();
    mags.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    for i in 0..6.min(mags.len()) {
        out.push(mags[i]);
    }
    // folding nonlinear (amplify micro structure)
    let mut fold = 0.0;
    for (i, v) in curve.iter().enumerate() {
        let t = (i as f64 + 1.0) * 0.17;
        fold += (v * t).sin().abs() * 0.6 + (v * v * 0.5).tanh().abs() * 0.4;
    }
    out.push(fold / n as f64);
    // quantize for stable digest material
    out.iter()
        .map(|x| {
            let q = params.mag_quanta.max(1e-6);
            (x / q).round() * q
        })
        .collect()
}

fn digest_of_features(tag: &str, feats: &[f64]) -> String {
    let mut h = Sha256::new();
    h.update(tag.as_bytes());
    for x in feats {
        h.update(format!("{x:.6}|").as_bytes());
    }
    format!("{:x}", h.finalize())[..16].to_string()
}

/// Extract path curves from residual_paths array grouped by shader mode / path role.
fn collect_path_curves(fo: &Map<String, Value>) -> Vec<(String, Vec<f64>)> {
    let mut out = Vec::new();
    let Some(paths) = fo.get("residual_paths").and_then(|v| v.as_array()) else {
        // fallback primary
        if let Some(c) = fo
            .get("hw_curve_webgl")
            .map(f64_vec)
            .filter(|c| c.len() >= 8)
        {
            out.push(("primary".into(), c));
        }
        return out;
    };
    for p in paths {
        let ok = p.get("ok").and_then(|v| v.as_bool()).unwrap_or(false)
            || p.get("entropy_ok").and_then(|v| v.as_bool()).unwrap_or(false);
        let curve = p
            .get("curve")
            .or_else(|| p.get("values"))
            .map(f64_vec)
            .unwrap_or_default();
        if curve.len() < 8 {
            continue;
        }
        if !ok && !crate::trust::residual_curve_entropy_ok(&curve) {
            // still keep for ensemble disagreement signals
        }
        let mode = p
            .get("shader_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let pid = p
            .get("path_id")
            .and_then(|v| v.as_str())
            .unwrap_or("path");
        let lane = if mode == "ulp" || pid.contains("ulp") {
            "ulp"
        } else if mode == "rint" || pid.contains("rint") {
            "rint"
        } else if mode == "noderiv" || pid.contains("noderiv") {
            "noderiv"
        } else {
            "float"
        };
        out.push((format!("{lane}:{pid}"), curve));
    }
    out
}

fn eu_timing_curve(fo: &Map<String, Value>) -> Vec<f64> {
    if let Some(a) = fo
        .get("residual_select")
        .and_then(|v| v.get("lane_s"))
        .and_then(|v| v.get("eu_timing_ms"))
        .map(f64_vec)
        .filter(|c| c.len() >= 4)
    {
        return a;
    }
    for k in ["eu_timing_ms", "hw_eu_timing_ms", "draw_timing_ms"] {
        let c = fo.get(k).map(f64_vec).unwrap_or_default();
        if c.len() >= 4 {
            return c;
        }
    }
    // residual_paths timing
    if let Some(paths) = fo.get("residual_paths").and_then(|v| v.as_array()) {
        for p in paths {
            if let Some(t) = p.get("eu_timing_ms").map(f64_vec).filter(|c| c.len() >= 4) {
                return t;
            }
            if let Some(t) = p.get("timing_ms").map(f64_vec).filter(|c| c.len() >= 4) {
                return t;
            }
        }
    }
    Vec::new()
}

/// Build full anti-collision surface for product / commercial secondary materials.
pub fn build_anti_collision_surface(fields: &Value) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let primary = fo
        .get("hw_curve_webgl")
        .or_else(|| fo.get("webgl_residual_curve"))
        .map(f64_vec)
        .unwrap_or_default();
    let entropy_ok = if primary.len() >= 4 {
        crate::trust::residual_curve_entropy_ok(&primary)
    } else {
        false
    };
    let std = curve_std(&primary);
    let uniq = {
        let mut s: Vec<f64> = primary.clone();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mut u = 1usize;
        for w in s.windows(2) {
            if (w[1] - w[0]).abs() > 1e-4 {
                u += 1;
            }
        }
        if s.is_empty() {
            0
        } else {
            u
        }
    };
    let params = TeachingParams::from_entropy(entropy_ok, std, uniq);
    let paths = collect_path_curves(&fo);
    let n_paths = paths.len();
    let n_modes = {
        let mut m = std::collections::HashSet::new();
        for (k, _) in &paths {
            m.insert(k.split(':').next().unwrap_or("").to_string());
        }
        m.len()
    };

    // Per-path complex features → ensemble
    let mut path_digests: Vec<String> = Vec::new();
    let mut mode_feats: Map<String, Value> = Map::new();
    for (label, curve) in &paths {
        let feats = complex_curve_features(curve, &params);
        if feats.is_empty() {
            continue;
        }
        let dig = digest_of_features(&format!("path|{label}|"), &feats);
        path_digests.push(format!("{label}={dig}"));
        let mode = label.split(':').next().unwrap_or("x");
        mode_feats
            .entry(mode.to_string())
            .or_insert_with(|| json!([]));
        if let Some(Value::Array(a)) = mode_feats.get_mut(mode) {
            a.push(json!(dig));
        }
    }
    path_digests.sort();
    let ensemble_digest = if path_digests.is_empty() {
        if primary.len() >= 4 {
            let feats = complex_curve_features(&primary, &params);
            Some(format!(
                "ens_{}",
                digest_of_features("primary|", &feats)
            ))
        } else {
            None
        }
    } else {
        let mut h = Sha256::new();
        h.update(b"ensemble_v1|");
        h.update(params.class.as_bytes());
        for d in &path_digests {
            h.update(d.as_bytes());
            h.update(b"|");
        }
        Some(format!("ens_{}", &format!("{:x}", h.finalize())[..16]))
    };

    // Cross-path disagreement: same SKU often agrees on float hist, disagrees on ulp
    let path_means: Vec<f64> = paths
        .iter()
        .map(|(_, c)| (curve_mean(c) / params.mean_quanta).round() * params.mean_quanta)
        .collect();
    let mean_spread = if path_means.len() >= 2 {
        let mn = path_means.iter().cloned().fold(f64::INFINITY, f64::min);
        let mx = path_means.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        mx - mn
    } else {
        0.0
    };
    let multi_fn_disagrees = n_modes >= 2 && mean_spread > params.mean_quanta * 2.0;

    // Timing phase (DrawnApart-inspired)
    let timing = eu_timing_curve(&fo);
    let timing_digest = if timing.len() >= 4 {
        let tparams = TeachingParams {
            mag_quanta: params.timing_ms_quanta,
            ..params
        };
        let feats = complex_curve_features(&timing, &tparams);
        Some(format!(
            "tm_{}",
            digest_of_features("eu_timing|", &feats)
        ))
    } else {
        None
    };

    // Secondary materials always considered for fork under dead residual
    let audio = fo.get("hw_curve_audio").map(f64_vec).unwrap_or_default();
    let audio_dig = if audio.len() >= 8 {
        let feats = complex_curve_features(&audio, &params);
        Some(format!(
            "au_{}",
            digest_of_features("audio_ac|", &feats)
        ))
    } else {
        None
    };

    // Composite anti-collision id material (secondary commercial when residual weak)
    let mut composite_parts: Vec<String> = Vec::new();
    if let Some(ref e) = ensemble_digest {
        composite_parts.push(e.clone());
    }
    if multi_fn_disagrees {
        composite_parts.push(format!("dis_{:.4}", mean_spread));
    }
    if let Some(ref t) = timing_digest {
        composite_parts.push(t.clone());
    }
    if !entropy_ok {
        if let Some(ref a) = audio_dig {
            composite_parts.push(a.clone());
        }
    }
    let composite = if composite_parts.is_empty() {
        None
    } else {
        let mut h = Sha256::new();
        h.update(b"hw_ac_composite_v1|");
        for p in &composite_parts {
            h.update(p.as_bytes());
            h.update(b"|");
        }
        Some(format!("ac_{}", &format!("{:x}", h.finalize())[..16]))
    };

    // Strategy recommendation
    let strategy = if entropy_ok && n_modes >= 2 && multi_fn_disagrees {
        "ensemble_multi_fn_split"
    } else if !entropy_ok && timing_digest.is_some() {
        "dead_residual_timing_secondary"
    } else if !entropy_ok && audio_dig.is_some() {
        "dead_residual_multi_material"
    } else if !entropy_ok {
        "dead_residual_need_host_sep"
    } else if n_paths <= 1 {
        "single_path_class_risk"
    } else {
        "ensemble_stable_class"
    };

    let single_curve_collision_prone = !entropy_ok
        || (n_modes <= 1 && !timing_digest.is_some() && !multi_fn_disagrees);

    json!({
        "algo": HW_ANTI_COLLISION_ALGO,
        "teaching": {
            "class": params.class,
            "mag_quanta": params.mag_quanta,
            "mean_quanta": params.mean_quanta,
            "timing_ms_quanta": params.timing_ms_quanta,
            "residual_alone_uv_ok": params.residual_alone_uv_ok,
        },
        "primary_entropy_ok": entropy_ok,
        "primary_std": (std * 1e6).round() / 1e6,
        "n_paths": n_paths,
        "n_shader_modes": n_modes,
        "mean_spread": (mean_spread * 1e6).round() / 1e6,
        "multi_fn_disagrees": multi_fn_disagrees,
        "ensemble_digest": ensemble_digest,
        "timing_digest": timing_digest,
        "audio_ac_digest": audio_dig,
        "composite_digest": composite,
        "path_digests_n": path_digests.len(),
        "mode_feats": mode_feats,
        "strategy": strategy,
        "single_curve_collision_prone": single_curve_collision_prone,
        "commercial_secondary_ok": composite.is_some() && (!entropy_ok || multi_fn_disagrees || timing_digest.is_some()),
        "promote_to_silicon_uv": false,
        "note": "multi-fn ensemble + adaptive quanta + timing; never residual-alone silicon UV",
        "refs": [
            "UNIGL/USENIX19 float-discrepancy",
            "DrawnApart EU timing",
            "FingerprintJS multi-signal fusion",
            "in-repo dual-lane residual + eu_timing"
        ],
    })
}

/// Materials map entries to fold into commercial_projection when secondary anchors help.
pub fn anti_collision_materials(fields: &Value) -> Map<String, Value> {
    let surf = build_anti_collision_surface(fields);
    let mut m = Map::new();
    if let Some(e) = surf.get("ensemble_digest").and_then(|v| v.as_str()) {
        m.insert("hw_ensemble_digest".into(), json!(e));
    }
    if let Some(t) = surf.get("timing_digest").and_then(|v| v.as_str()) {
        m.insert("hw_timing_phase_digest".into(), json!(t));
    }
    if let Some(c) = surf.get("composite_digest").and_then(|v| v.as_str()) {
        m.insert("hw_anti_collision".into(), json!(c));
    }
    if surf
        .get("commercial_secondary_ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        m.insert("hw_anti_collision_secondary_ok".into(), json!(true));
    }
    if surf
        .get("single_curve_collision_prone")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        m.insert("hw_single_curve_collision_prone".into(), json!(true));
    }
    m.insert("hw_anti_collision_surface".into(), surf);
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dead_hist() -> Vec<f64> {
        vec![
            0.03418, 0.04199, 0.03027, 0.03906, 0.02637, 0.0293, 0.03223, 0.03027, 0.03516,
            0.03125, 0.02832, 0.02051, 0.03223, 0.02539, 0.0332, 0.03223, 0.0293, 0.02344,
            0.02637, 0.02734, 0.02734, 0.03418, 0.0332, 0.0332, 0.02637, 0.02734, 0.02832,
            0.04004, 0.02637, 0.04102, 0.03418, 0.04004,
        ]
    }

    fn ulp_like(seed: f64) -> Vec<f64> {
        (0..32)
            .map(|i| {
                let x = i as f64 * 0.17 + seed;
                (x.sin().abs() * 0.4 + (x * 1.3).cos().abs() * 0.2 + seed * 0.05).max(0.01)
            })
            .collect()
    }

    #[test]
    fn same_dead_residual_splits_on_ulp_and_timing() {
        let dead = dead_hist();
        let a = json!({
            "hw_curve_webgl": dead.clone(),
            "residual_paths": [
                {"path_id":"float_a","shader_mode":"float","ok":true,"curve": dead.clone()},
                {"path_id":"ulp_a","shader_mode":"ulp","ok":true,"curve": ulp_like(0.1)},
            ],
            "eu_timing_ms": [1.2, 1.5, 1.1, 1.8, 1.3, 1.4, 1.6, 1.2],
            "hw_curve_audio": (0..64).map(|i| (i as f64 * 0.11).sin().abs()).collect::<Vec<_>>(),
        });
        let b = json!({
            "hw_curve_webgl": dead.clone(),
            "residual_paths": [
                {"path_id":"float_a","shader_mode":"float","ok":true,"curve": dead.clone()},
                {"path_id":"ulp_a","shader_mode":"ulp","ok":true,"curve": ulp_like(0.7)},
            ],
            "eu_timing_ms": [3.2, 3.5, 2.9, 3.8, 3.1, 3.0, 3.4, 3.2],
            "hw_curve_audio": (0..64).map(|i| (i as f64 * 0.41).cos().abs()).collect::<Vec<_>>(),
        });
        let sa = build_anti_collision_surface(&a);
        let sb = build_anti_collision_surface(&b);
        assert_eq!(sa["primary_entropy_ok"], false);
        assert_eq!(sa["promote_to_silicon_uv"], false);
        assert_ne!(
            sa["composite_digest"], sb["composite_digest"],
            "same dead residual must fork via ensemble/timing/audio: {sa} vs {sb}"
        );
        assert_ne!(sa["ensemble_digest"], sb["ensemble_digest"]);
    }

    #[test]
    fn identical_materials_collide_honestly() {
        let dead = dead_hist();
        let f = json!({
            "hw_curve_webgl": dead.clone(),
            "residual_paths": [
                {"path_id":"float_a","shader_mode":"float","ok":true,"curve": dead},
            ],
        });
        let s = build_anti_collision_surface(&f);
        assert_eq!(s["single_curve_collision_prone"], true);
        assert_eq!(s["strategy"], "dead_residual_need_host_sep");
    }

    #[test]
    fn complex_features_dim() {
        let c: Vec<f64> = (0..32).map(|i| (i as f64 * 0.2).sin().abs() + 0.05).collect();
        let p = TeachingParams::from_entropy(true, 0.08, 16);
        let f = complex_curve_features(&c, &p);
        assert!(f.len() >= 12);
    }
}
