//! Commercial-grade multi-path silicon fusion (iss/50 · same-SKU split).
//!
//! # Goal
//! Get as close as practical to **silicon-level** separability under the same
//! OS/driver, without violating product redlines:
//! - No residual-alone UV promise
//! - Lane-C remains cross-browser commercial-stable
//! - Lane-S / fusion fine maximises same-SKU split when materials exist
//!
//! # Channels fused
//! 1. WebGL residual_paths (float / rint / noderiv / ulp) — dual-lane
//! 2. EU / draw timing (DrawnApart-class)
//! 3. OfflineAudio curve
//! 4. WebGPU compute residual
//! 5. Canvas / CPU timing curves
//! 6. Anti-collision composite (ensemble + teaching params)
//!
//! # Output materials (for commercial_projection / device_segments)
//! - `hw_webgl_stable` — Lane-C (already produced upstream; we reaffirm)
//! - `hw_silicon_fine` — Lane-S fine digest (ulp + timing moments)
//! - `hw_silicon_fusion` — multi-channel weighted commercial secondary
//! - `hw_silicon_fusion_surface` — full diagnostics for ops

use crate::hw_anti_collision::{build_anti_collision_surface, complex_curve_features, TeachingParams};
use crate::trust::{residual_curve_entropy_ok, webgl_commercial_digest_for_engine};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub const HW_SILICON_FUSION_ALGO: &str = "hw_silicon_fusion_v2";

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

fn curve_std(xs: &[f64]) -> f64 {
    if xs.len() < 2 {
        return 0.0;
    }
    let n = xs.len() as f64;
    let m = xs.iter().sum::<f64>() / n;
    (xs.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / n).sqrt()
}

fn curve_mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        0.0
    } else {
        xs.iter().sum::<f64>() / xs.len() as f64
    }
}

fn digest_tag(tag: &str, parts: &[&str]) -> String {
    let mut h = Sha256::new();
    h.update(tag.as_bytes());
    for p in parts {
        h.update(p.as_bytes());
        h.update(b"|");
    }
    format!("{:x}", h.finalize())[..16].to_string()
}

fn digest_feats(tag: &str, feats: &[f64]) -> String {
    let mut h = Sha256::new();
    h.update(tag.as_bytes());
    for x in feats {
        h.update(format!("{x:.6}|").as_bytes());
    }
    format!("{:x}", h.finalize())[..16].to_string()
}

/// Path role for fusion weights.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum PathRole {
    Float,
    Rint,
    Noderiv,
    Ulp,
    Other,
}

fn classify_path(entry: &Value) -> PathRole {
    let pid = entry
        .get("path_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mode = entry
        .get("shader_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    // iss/54 advanced silicon probes → treat as ULP-class Lane-S
    if mode == "ulp"
        || mode == "silicon_ulp"
        || mode == "fma_pair"
        || mode == "fma"
        || mode == "denorm"
        || mode == "ftz"
        || mode == "tex_lerp"
        || mode == "tex"
        || mode == "interp"
        || mode == "interp_diverge"
        || pid.contains("ulp")
        || pid.contains("fma_pair")
        || pid.contains("denorm")
        || pid.contains("tex_lerp")
        || pid.contains("interp_diverge")
    {
        PathRole::Ulp
    } else if mode == "rint" || mode == "int" || pid.contains("rint") {
        PathRole::Rint
    } else if mode == "noderiv" || pid.contains("noderiv") {
        PathRole::Noderiv
    } else if mode == "float" || pid.contains("v3f") || pid.contains("warm") || pid.contains("std")
    {
        PathRole::Float
    } else {
        PathRole::Other
    }
}

fn path_ok(entry: &Value, curve: &[f64]) -> bool {
    if curve.len() < 8 {
        return false;
    }
    if entry.get("ok").and_then(|v| v.as_bool()).unwrap_or(false)
        || entry
            .get("entropy_ok")
            .and_then(|v| v.as_bool())
            .unwrap_or_else(|| residual_curve_entropy_ok(curve))
        || residual_curve_entropy_ok(curve)
    {
        return true;
    }
    // ULP / deep silicon paths with EU timing: accept lower residual entropy when
    // timing_samples land (old GPU residual often flatter; timing still separates).
    let role = classify_path(entry);
    if role == PathRole::Ulp {
        let has_timing = entry
            .get("eu_timing_ms")
            .or_else(|| entry.get("timing_ms"))
            .and_then(|v| v.as_array())
            .map(|a| a.len() >= 4)
            .unwrap_or(false);
        let std = curve_std(curve);
        if has_timing && std > 0.005 {
            return true;
        }
        // flat residual but multi-mode deep path marked ok by FE
        if entry.get("shader_mode").and_then(|v| v.as_str()).is_some_and(|m| {
            matches!(
                m,
                "ulp" | "fma_pair" | "fma" | "denorm" | "tex_lerp" | "tex" | "interp"
            )
        }) && std > 0.008
        {
            return true;
        }
    }
    false
}

/// Weighted average of curves (resampled to max len by padding last).
fn blend_curves(curves: &[(f64, Vec<f64>)]) -> Vec<f64> {
    if curves.is_empty() {
        return Vec::new();
    }
    let n = curves.iter().map(|(_, c)| c.len()).max().unwrap_or(0);
    if n == 0 {
        return Vec::new();
    }
    let mut acc = vec![0.0; n];
    let mut wsum = vec![0.0; n];
    for (w, c) in curves {
        if *w <= 0.0 || c.is_empty() {
            continue;
        }
        for i in 0..n {
            let v = c.get(i).copied().unwrap_or_else(|| *c.last().unwrap_or(&0.0));
            if v.is_finite() {
                acc[i] += w * v;
                wsum[i] += w;
            }
        }
    }
    acc.iter()
        .zip(wsum.iter())
        .map(|(a, w)| if *w > 1e-12 { a / w } else { 0.0 })
        .collect()
}

/// Collect residual_paths; also accept residual_paths from nested residual_select.
fn all_residual_paths(fo: &Map<String, Value>) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(a) = fo.get("residual_paths").and_then(|v| v.as_array()) {
        out.extend(a.iter().cloned());
    }
    // B10x may land as residual_paths_b10x / residual_paths_extra
    for k in [
        "residual_paths_b10x",
        "residual_paths_extra",
        "residual_paths_merged",
    ] {
        if let Some(a) = fo.get(k).and_then(|v| v.as_array()) {
            out.extend(a.iter().cloned());
        }
    }
    out
}

/// Commercial multi-path silicon fusion surface + material digests.
pub fn fuse_silicon_channels(fields: &Value) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();
    // F6: robust engine detect (engine_family / UA / brands) before Lane-S norm
    let engine_owned = {
        let d = crate::hw_engine_norm::detect_engine_from_fields(fields);
        if d != "unknown" {
            d
        } else {
            fo.get("engine_family")
                .or_else(|| fo.get("protocol_engine"))
                .and_then(|v| v.as_str())
                .map(|s| crate::hw_engine_norm::normalize_engine_key(s))
                .unwrap_or_else(|| "unknown".into())
        }
    };
    let engine = Some(engine_owned.as_str());

    let paths = all_residual_paths(&fo);
    let mut by_role: HashMap<PathRole, Vec<(f64, Vec<f64>, String)>> = HashMap::new();
    let mut n_ok = 0usize;
    let mut timing_curves: Vec<Vec<f64>> = Vec::new();

    for entry in &paths {
        let curve = entry
            .get("curve")
            .or_else(|| entry.get("values"))
            .map(f64_vec)
            .unwrap_or_default();
        let role = classify_path(entry);
        let ok = path_ok(entry, &curve);
        if ok {
            n_ok += 1;
        }
        // weights: prefer healthy high-std paths within role
        let std = curve_std(&curve);
        let mut w = if ok { 1.0 } else { 0.25 };
        w *= 1.0 + (std * 8.0).clamp(0.0_f64, 2.0_f64);
        // iss/54 F4: down-weight noisy timing-bearing paths
        let tq = entry
            .get("timing_quality_score")
            .and_then(|v| v.as_f64())
            .or_else(|| {
                match entry.get("timing_quality").and_then(|v| v.as_str()) {
                    Some("high") => Some(1.0),
                    Some("med") => Some(0.7),
                    Some("low") => Some(0.35),
                    Some("reject") => Some(0.1),
                    _ => None,
                }
            });
        if let Some(q) = tq {
            if entry.get("eu_timing_ms").is_some() {
                w *= q.clamp(0.1, 1.0);
            }
        }
        if role == PathRole::Ulp {
            w *= 1.35; // silicon-separating
        } else if role == PathRole::Rint || role == PathRole::Noderiv {
            w *= 1.15; // commercial-stable
        }
        let pid = entry
            .get("path_id")
            .and_then(|v| v.as_str())
            .unwrap_or("p")
            .to_string();
        if curve.len() >= 8 {
            by_role
                .entry(role)
                .or_default()
                .push((w, curve, pid));
        }
        if let Some(t) = entry
            .get("eu_timing_ms")
            .or_else(|| entry.get("timing_ms"))
            .map(f64_vec)
            .filter(|c| c.len() >= 4)
        {
            timing_curves.push(t);
        }
    }
    // top-level eu_timing
    if let Some(t) = fo
        .get("eu_timing_ms")
        .map(f64_vec)
        .filter(|c| c.len() >= 4)
    {
        timing_curves.push(t);
    }

    // --- Lane-C blend: rint > noderiv > float ---
    let mut lane_c_parts: Vec<(f64, Vec<f64>)> = Vec::new();
    for role in [PathRole::Rint, PathRole::Noderiv, PathRole::Float, PathRole::Other] {
        if let Some(v) = by_role.get(&role) {
            for (w, c, _) in v {
                let rw = match role {
                    PathRole::Rint => *w * 1.4,
                    PathRole::Noderiv => *w * 1.3,
                    PathRole::Float => *w,
                    _ => *w * 0.8,
                };
                lane_c_parts.push((rw, c.clone()));
            }
        }
    }
    // primary hw_curve_webgl fallback
    if lane_c_parts.is_empty() {
        let c = fo
            .get("hw_curve_webgl")
            .or_else(|| fo.get("webgl_residual_curve"))
            .map(f64_vec)
            .unwrap_or_default();
        if c.len() >= 8 {
            lane_c_parts.push((1.0, c));
        }
    }
    let lane_c_curve = blend_curves(&lane_c_parts);

    // --- Lane-S blend: ulp only (+ high-std float if no ulp) ---
    let mut lane_s_parts: Vec<(f64, Vec<f64>)> = Vec::new();
    if let Some(v) = by_role.get(&PathRole::Ulp) {
        for (w, c, _) in v {
            lane_s_parts.push((*w * 1.5, c.clone()));
        }
    }
    if lane_s_parts.is_empty() {
        // fallback: highest-std float path as weak silicon
        if let Some(v) = by_role.get(&PathRole::Float) {
            let mut best: Option<(f64, Vec<f64>)> = None;
            for (w, c, _) in v {
                let s = curve_std(c);
                if best.as_ref().map(|(bs, _)| s > *bs).unwrap_or(true) {
                    best = Some((s, c.clone()));
                }
                let _ = w;
            }
            if let Some((s, c)) = best {
                if s > 0.02 {
                    lane_s_parts.push((0.6, c));
                }
            }
        }
    }
    let lane_s_raw = blend_curves(&lane_s_parts);
    // F6: engine-family normalize Lane-S before fine digest
    let (lane_s_curve, engine_norm_meta) =
        crate::hw_engine_norm::normalize_lane_s_curve(&lane_s_raw, engine);

    let lane_c_entropy = residual_curve_entropy_ok(&lane_c_curve);
    let lane_s_entropy = residual_curve_entropy_ok(&lane_s_curve);
    let params = TeachingParams::from_entropy(
        lane_s_entropy || lane_c_entropy,
        curve_std(if lane_s_entropy {
            &lane_s_curve
        } else {
            &lane_c_curve
        }),
        16,
    );
    let wcfg = crate::hw_fusion_weights::load_fusion_weights();

    // Digests
    let webgl_stable = webgl_commercial_digest_for_engine(&lane_c_curve, engine);
    let silicon_fine = if lane_s_curve.len() >= 8 {
        let mut feats = complex_curve_features(&lane_s_curve, &params);
        // salt fine feats with engine lsh_salt for F6 separation without host merge
        if let Some(salt) = engine_norm_meta.get("lsh_salt").and_then(|v| v.as_str()) {
            let mut h = 0u64;
            for b in salt.bytes() {
                h = h.wrapping_mul(131).wrapping_add(b as u64);
            }
            feats.push((h % 1000) as f64 / 1000.0);
        }
        Some(format!("sf_{}", digest_feats("silicon_fine_v2|", &feats)))
    } else {
        None
    };

    // Timing fusion
    let timing_blend = {
        let parts: Vec<(f64, Vec<f64>)> = timing_curves
            .iter()
            .map(|c| (1.0, c.clone()))
            .collect();
        blend_curves(&parts)
    };
    let timing_digest = if timing_blend.len() >= 4 {
        let tp = TeachingParams {
            mag_quanta: params.timing_ms_quanta,
            ..params
        };
        let feats = complex_curve_features(&timing_blend, &tp);
        Some(format!("tm_{}", digest_feats("eu_timing_fuse|", &feats)))
    } else {
        None
    };

    // Other channels
    let audio = fo.get("hw_curve_audio").map(f64_vec).unwrap_or_default();
    let audio_ok = audio.len() >= 8 && residual_curve_entropy_ok(&audio);
    let audio_dig = if audio.len() >= 8 {
        let feats = complex_curve_features(&audio, &params);
        Some(format!("au_{}", digest_feats("audio_fuse|", &feats)))
    } else {
        None
    };

    let webgpu = fo
        .get("hw_curve_webgpu")
        .or_else(|| fo.get("webgpu_compute_curve"))
        .map(f64_vec)
        .unwrap_or_default();
    let webgpu_ok = webgpu.len() >= 8 && residual_curve_entropy_ok(&webgpu);
    let webgpu_dig = if webgpu.len() >= 8 {
        let feats = complex_curve_features(&webgpu, &params);
        Some(format!("gp_{}", digest_feats("webgpu_fuse|", &feats)))
    } else {
        None
    };

    // CPU silicon channel (independent of B10 GPU residual). Used heavily when
    // WebGPU/Lane-S weak — never mixes B34 cache ladder into commercial mint.
    let cpu = fo
        .get("hw_curve_cpu")
        .or_else(|| fo.get("cpu_timing_curve"))
        .map(f64_vec)
        .unwrap_or_default();
    let cpu_ok = cpu.len() >= 8 && residual_curve_entropy_ok(&cpu);
    let cpu_dig = if cpu.len() >= 8 {
        let mut feats = complex_curve_features(&cpu, &params);
        // Optional B34 knee is diagnostic conf salt only — never sole mint material.
        if let Some(knee) = fo
            .get("cpu_cache_knee_bytes")
            .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
        {
            if knee > 0.0 && knee.is_finite() {
                feats.push(knee.log2());
            }
        }
        Some(format!("cp_{}", digest_feats("cpu_fuse|", &feats)))
    } else {
        None
    };

    let canvas = fo.get("hw_curve_canvas").map(f64_vec).unwrap_or_default();
    let canvas_dig = if canvas.len() >= 8 {
        let feats = complex_curve_features(&canvas, &params);
        Some(format!("cv_{}", digest_feats("canvas_fuse|", &feats)))
    } else {
        None
    };

    // Anti-collision surface (ensemble of residual_paths)
    let ac = build_anti_collision_surface(fields);

    // Weighted multi-channel fusion digest — weights from silicon_fusion_weights_v1 (F5)
    let mut fuse_parts: Vec<(f64, String)> = Vec::new();
    if let Some(ref d) = webgl_stable {
        fuse_parts.push((
            if lane_c_entropy {
                wcfg.get_ch("lane_c_healthy", 1.0)
            } else {
                wcfg.get_ch("lane_c_dead", 0.45)
            },
            d.clone(),
        ));
    }
    if let Some(ref d) = silicon_fine {
        fuse_parts.push((
            if lane_s_entropy {
                wcfg.get_ch("lane_s_healthy", 1.25)
            } else {
                wcfg.get_ch("lane_s_weak", 0.7)
            },
            d.clone(),
        ));
    }
    if let Some(ref d) = timing_digest {
        fuse_parts.push((wcfg.get_ch("timing", 1.1), d.clone()));
    }
    if let Some(ref d) = audio_dig {
        fuse_parts.push((
            if audio_ok {
                wcfg.get_ch("audio_healthy", 0.95)
            } else {
                wcfg.get_ch("audio_weak", 0.55)
            },
            d.clone(),
        ));
    }
    if let Some(ref d) = webgpu_dig {
        fuse_parts.push((
            if webgpu_ok {
                wcfg.get_ch("webgpu_healthy", 0.9)
            } else {
                wcfg.get_ch("webgpu_weak", 0.5)
            },
            d.clone(),
        ));
    }
    if let Some(ref d) = cpu_dig {
        // Boost CPU channel when GPU silicon weak (old / no WebGPU stacks).
        let cpu_w = if !webgpu_ok && !lane_s_entropy {
            wcfg.get_ch("cpu_when_gpu_weak", 0.95)
        } else if cpu_ok {
            wcfg.get_ch("cpu", 0.7)
        } else {
            wcfg.get_ch("cpu_weak", 0.45)
        };
        fuse_parts.push((cpu_w, d.clone()));
    }
    if let Some(ref d) = canvas_dig {
        fuse_parts.push((wcfg.get_ch("canvas", 0.4), d.clone()));
    }
    if let Some(e) = ac.get("ensemble_digest").and_then(|v| v.as_str()) {
        fuse_parts.push((wcfg.get_ch("ensemble", 0.85), e.to_string()));
    }
    if let Some(c) = ac.get("composite_digest").and_then(|v| v.as_str()) {
        fuse_parts.push((
            wcfg.get_ch("anti_collision_composite", 0.95),
            c.to_string(),
        ));
    }
    if let Some(s) = fo.get("seed_ulp_digest").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
        fuse_parts.push((wcfg.get_ch("seed_ulp", 0.35), s.to_string()));
    }
    if let Some(s) = fo
        .get("seed_residual_digest")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        fuse_parts.push((wcfg.get_ch("seed_residual", 0.25), s.to_string()));
    }

    // Sort by channel tag for stability then weight into hash
    fuse_parts.sort_by(|a, b| a.1.cmp(&b.1));
    let fusion_digest = if fuse_parts.is_empty() {
        None
    } else {
        let mut h = Sha256::new();
        h.update(b"hw_silicon_fusion_v2|");
        h.update(params.class.as_bytes());
        for (w, d) in &fuse_parts {
            h.update(format!("{w:.2}=").as_bytes());
            h.update(d.as_bytes());
            h.update(b"|");
        }
        Some(format!("hsf_{}", &format!("{:x}", h.finalize())[..16]))
    };

    // Same-SKU separability score [0,1]: how much fine structure we have beyond class residual
    let mut sep = 0.0;
    if lane_s_entropy {
        sep += wcfg.get_sep("lane_s_entropy", 0.28);
    }
    if timing_digest.is_some() {
        sep += wcfg.get_sep("timing", 0.22);
    }
    if audio_ok {
        sep += wcfg.get_sep("audio_ok", 0.18);
    }
    if webgpu_ok {
        sep += wcfg.get_sep("webgpu_ok", 0.14);
    }
    if cpu_ok {
        sep += wcfg.get_sep("cpu_ok", 0.10);
    }
    if ac
        .get("multi_fn_disagrees")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        sep += wcfg.get_sep("multi_fn_disagrees", 0.12);
    }
    if n_ok >= 3 {
        sep += wcfg.get_sep("n_paths_ok_ge3", 0.06);
    }
    // Timing-bearing ULP paths count toward separability even when residual_entropy weak
    if timing_digest.is_some() && !lane_s_entropy {
        sep += wcfg.get_sep("timing_without_lane_s", 0.08);
    }
    let same_sku_separability = (sep as f64).clamp(0.0_f64, 1.0_f64);

    // Grade for product/ops
    let grade = if same_sku_separability >= 0.75 && lane_s_entropy {
        "near_silicon"
    } else if same_sku_separability >= 0.5 {
        "strong_machine"
    } else if lane_c_entropy || audio_ok {
        "config_plus"
    } else {
        "class_template"
    };

    let n_roles = by_role.len();
    let commercial_secondary_ok = fusion_digest.is_some()
        && (same_sku_separability >= 0.35
            || !lane_c_entropy
            || ac
                .get("commercial_secondary_ok")
                .and_then(|v| v.as_bool())
                .unwrap_or(false));

    json!({
        "algo": HW_SILICON_FUSION_ALGO,
        "grade": grade,
        "same_sku_separability": (same_sku_separability * 10000.0).round() / 10000.0,
        "teaching_class": params.class,
        "n_paths": paths.len(),
        "n_paths_ok": n_ok,
        "n_shader_roles": n_roles,
        "lane_c": {
            "curve_len": lane_c_curve.len(),
            "entropy_ok": lane_c_entropy,
            "mean": (curve_mean(&lane_c_curve) * 1e6).round() / 1e6,
            "std": (curve_std(&lane_c_curve) * 1e6).round() / 1e6,
            "digest": webgl_stable,
            "curve_lsh": if lane_c_curve.len() >= 8 {
                crate::device_segments::curve_lsh_public(&lane_c_curve)
            } else {
                Value::Null
            },
        },
        "lane_s": {
            "curve_len": lane_s_curve.len(),
            "entropy_ok": lane_s_entropy,
            "mean": (curve_mean(&lane_s_curve) * 1e6).round() / 1e6,
            "std": (curve_std(&lane_s_curve) * 1e6).round() / 1e6,
            "digest": silicon_fine,
            "curve_lsh": if lane_s_curve.len() >= 8 {
                crate::device_segments::curve_lsh_public(&lane_s_curve)
            } else {
                Value::Null
            },
        },
        "channels": {
            "timing": timing_digest,
            "audio": audio_dig,
            "webgpu": webgpu_dig,
            "cpu": cpu_dig,
            "canvas": canvas_dig,
            "anti_collision_ensemble": ac.get("ensemble_digest").cloned().unwrap_or(Value::Null),
        },
        "hw_silicon_fine": silicon_fine,
        "hw_silicon_fusion": fusion_digest,
        "hw_cpu_silicon": cpu_dig,
        "cpu_silicon_ok": cpu_ok,
        "lane_c_curve": if lane_c_curve.len() >= 8 { json!(lane_c_curve) } else { Value::Null },
        "lane_s_curve": if lane_s_curve.len() >= 8 { json!(lane_s_curve) } else { Value::Null },
        "engine_norm": engine_norm_meta,
        "fusion_weights_source": wcfg.source,
        "fusion_weights_learned": wcfg.learned,
        "commercial_secondary_ok": commercial_secondary_ok,
        "promote_to_silicon_uv": false,
        "note": "Lane-C=cross-browser stable; Lane-S+fusion=near-silicon same-SKU split; never residual-alone UV",
    })
}

/// Merge fusion materials into commercial materials map.
pub fn silicon_fusion_materials(fields: &Value) -> Map<String, Value> {
    let surf = fuse_silicon_channels(fields);
    let mut m = Map::new();
    if let Some(d) = surf.pointer("/lane_c/digest").and_then(|v| v.as_str()) {
        // reinforce / fill stable when missing
        m.insert("hw_webgl_stable_fused".into(), json!(d));
    }
    if let Some(d) = surf.get("hw_silicon_fine").and_then(|v| v.as_str()) {
        m.insert("hw_silicon_fine".into(), json!(d));
    }
    if let Some(d) = surf.get("hw_silicon_fusion").and_then(|v| v.as_str()) {
        m.insert("hw_silicon_fusion".into(), json!(d));
    }
    if let Some(d) = surf.pointer("/channels/timing").and_then(|v| v.as_str()) {
        m.insert("hw_timing_phase_digest".into(), json!(d));
    }
    if let Some(d) = surf
        .get("hw_cpu_silicon")
        .or_else(|| surf.pointer("/channels/cpu"))
        .and_then(|v| v.as_str())
    {
        m.insert("hw_cpu_silicon".into(), json!(d));
    }
    if surf.get("cpu_silicon_ok").and_then(|v| v.as_bool()) == Some(true) {
        m.insert("hw_cpu_silicon_ok".into(), json!(true));
    }
    if surf
        .get("commercial_secondary_ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        m.insert("hw_silicon_fusion_secondary_ok".into(), json!(true));
    }
    m.insert(
        "hw_silicon_grade".into(),
        surf.get("grade").cloned().unwrap_or(json!("class_template")),
    );
    m.insert(
        "same_sku_separability".into(),
        surf.get("same_sku_separability")
            .cloned()
            .unwrap_or(json!(0.0)),
    );
    m.insert("hw_silicon_fusion_surface".into(), surf);
    m
}

/// Prefer fused lane curves for multi-segment body when present.
/// Never shrink/overwrite a healthy longer primary residual with a short fuse.
pub fn prefer_fused_curves(fields: &mut Map<String, Value>) {
    let surf = fuse_silicon_channels(&Value::Object(fields.clone()));
    let cur = fields.get("hw_curve_webgl").map(f64_vec).unwrap_or_default();
    let cur_ok = residual_curve_entropy_ok(&cur);
    if let Some(Value::Array(c)) = surf.get("lane_c_curve") {
        if c.len() >= 8 {
            fields.insert("hw_curve_webgl_fused_c".into(), Value::Array(c.clone()));
            // Only fill empty / dead primary — never replace healthy raw multipath
            if cur.len() < 8 || !cur_ok {
                let fused: Vec<f64> = c.iter().filter_map(|v| v.as_f64()).collect();
                if fused.len() > cur.len() || !cur_ok {
                    fields.insert("hw_curve_webgl".into(), Value::Array(c.clone()));
                }
            }
        }
    }
    if let Some(Value::Array(c)) = surf.get("lane_s_curve") {
        if c.len() >= 8 {
            fields.insert("hw_curve_webgl_fused_s".into(), Value::Array(c.clone()));
            // silicon lane is additive (same-SKU); only promote to primary when dead
            fields.insert("hw_curve_webgl_silicon".into(), Value::Array(c.clone()));
            if !cur_ok && c.len() >= 8 {
                // optional: keep silicon as alternate body material via pick_curves key order
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dead() -> Vec<f64> {
        vec![
            0.03418, 0.04199, 0.03027, 0.03906, 0.02637, 0.0293, 0.03223, 0.03027, 0.03516,
            0.03125, 0.02832, 0.02051, 0.03223, 0.02539, 0.0332, 0.03223, 0.0293, 0.02344,
            0.02637, 0.02734, 0.02734, 0.03418, 0.0332, 0.0332, 0.02637, 0.02734, 0.02832,
            0.04004, 0.02637, 0.04102, 0.03418, 0.04004,
        ]
    }
    fn ulp(s: f64) -> Vec<f64> {
        (0..32)
            .map(|i| {
                let x = i as f64 * 0.21 + s * 3.1;
                (x.sin().abs() * 0.5 + (x * 1.9).cos().abs() * 0.25 + s * 0.1).max(0.03)
            })
            .collect()
    }
    fn healthy() -> Vec<f64> {
        (0..32)
            .map(|i| {
                let x = i as f64;
                (x * 0.37).sin().abs() * 0.4 + (x * 0.11).cos().abs() * 0.2 + 0.05
            })
            .collect()
    }

    #[test]
    fn multi_path_fusion_splits_same_sku() {
        let d = dead();
        let a = json!({
            "hw_curve_webgl": d.clone(),
            "residual_paths": [
                {"path_id":"float_std","shader_mode":"float","ok":true,"curve": d.clone()},
                {"path_id":"rint_w","shader_mode":"rint","ok":true,"curve": healthy()},
                {"path_id":"ulp_a","shader_mode":"ulp","ok":true,"curve": ulp(0.2)},
            ],
            "eu_timing_ms": [1.1,1.3,1.0,1.5,1.2,1.4,1.1,1.3],
            "hw_curve_audio": (0..64).map(|i| (i as f64 * 0.17).sin().abs()*0.6+0.05).collect::<Vec<_>>(),
            "hw_curve_webgpu": (0..32).map(|i| (i as f64 * 0.29).cos().abs()*0.35+0.04).collect::<Vec<_>>(),
        });
        let b = json!({
            "hw_curve_webgl": d.clone(),
            "residual_paths": [
                {"path_id":"float_std","shader_mode":"float","ok":true,"curve": d.clone()},
                {"path_id":"rint_w","shader_mode":"rint","ok":true,"curve": healthy()},
                {"path_id":"ulp_a","shader_mode":"ulp","ok":true,"curve": ulp(0.85)},
            ],
            "eu_timing_ms": [2.8,3.1,2.7,3.4,2.9,3.0,2.8,3.2],
            "hw_curve_audio": (0..64).map(|i| (i as f64 * 0.41).cos().abs()*0.55+0.05).collect::<Vec<_>>(),
            "hw_curve_webgpu": (0..32).map(|i| (i as f64 * 0.47).sin().abs()*0.4+0.03).collect::<Vec<_>>(),
        });
        let fa = fuse_silicon_channels(&a);
        let fb = fuse_silicon_channels(&b);
        assert_ne!(fa["hw_silicon_fusion"], fb["hw_silicon_fusion"]);
        assert_ne!(fa["hw_silicon_fine"], fb["hw_silicon_fine"]);
        assert!(fa["same_sku_separability"].as_f64().unwrap() >= 0.4);
        assert_eq!(fa["promote_to_silicon_uv"], false);
    }

    #[test]
    fn identical_paths_same_fusion() {
        let d = dead();
        let f = json!({
            "hw_curve_webgl": d.clone(),
            "residual_paths": [
                {"path_id":"float_std","shader_mode":"float","ok":true,"curve": d},
            ],
        });
        let a = fuse_silicon_channels(&f);
        let b = fuse_silicon_channels(&f);
        assert_eq!(a["hw_silicon_fusion"], b["hw_silicon_fusion"]);
        assert_eq!(a["grade"], "class_template");
    }
}
