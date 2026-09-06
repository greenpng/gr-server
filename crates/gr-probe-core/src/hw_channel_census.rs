//! Per-channel hardware fingerprint census — commercial comparison SSOT.
//!
//! Honest roles (same OS/driver, multi-machine):
//! | Channel | Same-SKU split | Cross-browser | Class collision | Commercial role |
//! |---------|----------------|---------------|-----------------|-----------------|
//! | WebGL float hist | weak | medium | high (ANGLE dead) | Lane-C primary when healthy |
//! | WebGL rint/noderiv | weak-med | strong | med | Lane-C prefer (stable) |
//! | WebGL ulp chain | strong | weak | low | Lane-S silicon fine |
//! | EU / draw timing | strong | weak | low | Lane-S secondary |
//! | OfflineAudio | med-strong | med (coarse) | med | Primary dual anchor |
//! | WebGPU compute | strong* | weak | low | Conf + fusion; *if available |
//! | Canvas noise | weak | weak | high | Conf / dead-residual fill |
//! | CPU timing | weak-med | weak | high | Extended segment only |
//! | Host sep (webrtc/oi) | machine env | n/a | low | Separator not silicon |
//! | GPU model string | none | n/a | total | **never commercial body** |
//!
//! *WebGPU may be absent (no adapter / insecure context).

use crate::hw_anti_collision::build_anti_collision_surface;
use crate::hw_silicon_fusion::fuse_silicon_channels;
use crate::trust::residual_curve_entropy_ok;
use serde_json::{json, Map, Value};

pub const HW_CHANNEL_CENSUS_ALGO: &str = "hw_channel_census_v1";

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

fn channel_row(
    id: &str,
    present: bool,
    entropy_ok: bool,
    len: usize,
    std: f64,
    same_sku: &str,
    cross_browser: &str,
    class_collision: &str,
    commercial_role: &str,
    field: &str,
    pack: &str,
    notes: &str,
) -> Value {
    json!({
        "id": id,
        "present": present,
        "entropy_ok": entropy_ok,
        "curve_len": len,
        "std": (std * 1e6).round() / 1e6,
        "same_sku_split": same_sku,
        "cross_browser_stable": cross_browser,
        "class_collision_risk": class_collision,
        "commercial_role": commercial_role,
        "field": field,
        "pack": pack,
        "notes": notes,
        "quality_score": quality_score(present, entropy_ok, std, same_sku),
    })
}

fn quality_score(present: bool, entropy_ok: bool, std: f64, same_sku: &str) -> f64 {
    if !present {
        return 0.0;
    }
    let mut s = 0.25;
    if entropy_ok {
        s += 0.35;
    }
    s += (std * 5.0).clamp(0.0, 0.2);
    s += match same_sku {
        "strong" => 0.25,
        "med" | "med-strong" => 0.15,
        "weak-med" => 0.08,
        "weak" => 0.03,
        _ => 0.0,
    };
    s.clamp(0.0, 1.0)
}

/// Full census of every HW fingerprint channel for one observation.
pub fn hw_channel_census(fields: &Value) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let mut channels: Vec<Value> = Vec::new();

    // --- residual path roles ---
    let paths = fo
        .get("residual_paths")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut n_float = 0usize;
    let mut n_rint = 0usize;
    let mut n_noderiv = 0usize;
    let mut n_ulp = 0usize;
    let mut ulp_std_max = 0.0f64;
    let mut float_std_max = 0.0f64;
    let mut timing_n = 0usize;
    for p in &paths {
        let pid = p.get("path_id").and_then(|v| v.as_str()).unwrap_or("");
        let mode = p.get("shader_mode").and_then(|v| v.as_str()).unwrap_or("");
        let c = p.get("curve").map(f64_vec).unwrap_or_default();
        let st = curve_std(&c);
        let ok = p.get("ok").and_then(|v| v.as_bool()).unwrap_or(false)
            || residual_curve_entropy_ok(&c);
        if mode == "ulp" || pid.contains("ulp") {
            n_ulp += 1;
            if ok {
                ulp_std_max = ulp_std_max.max(st);
            }
        } else if mode == "rint" || pid.contains("rint") {
            n_rint += 1;
        } else if mode == "noderiv" || pid.contains("noderiv") {
            n_noderiv += 1;
        } else {
            n_float += 1;
            if ok {
                float_std_max = float_std_max.max(st);
            }
        }
        if p.get("eu_timing_ms")
            .and_then(|v| v.as_array())
            .map(|a| a.len() >= 4)
            .unwrap_or(false)
        {
            timing_n += 1;
        }
    }

    let primary = fo
        .get("hw_curve_webgl")
        .map(f64_vec)
        .unwrap_or_default();
    let primary_ok = residual_curve_entropy_ok(&primary);

    channels.push(channel_row(
        "webgl_float_hist",
        !primary.is_empty() || n_float > 0,
        primary_ok || float_std_max > 0.01,
        primary.len().max(if n_float > 0 { 32 } else { 0 }),
        float_std_max.max(curve_std(&primary)),
        "weak",
        "medium",
        "high",
        "lane_c_primary_if_healthy",
        "hw_curve_webgl / residual_paths float",
        "B10_hw_curves",
        "ANGLE dead hist → class template; need multipath+ulp",
    ));
    channels.push(channel_row(
        "webgl_rint_noderiv",
        n_rint + n_noderiv > 0,
        n_rint + n_noderiv > 0,
        if n_rint + n_noderiv > 0 { 32 } else { 0 },
        0.0,
        "weak-med",
        "strong",
        "med",
        "lane_c_preferred_stable",
        "residual_paths rint/noderiv",
        "B10 + B10x_silicon_rint/noderiv",
        "Cross-engine commercial stability (Blink/Gecko)",
    ));
    channels.push(channel_row(
        "webgl_ulp_chain",
        n_ulp > 0,
        ulp_std_max > 0.01,
        if n_ulp > 0 { 32 } else { 0 },
        ulp_std_max,
        "strong",
        "weak",
        "low",
        "lane_s_silicon_fine",
        "residual_paths ulp",
        "B10 multipath + B10x_silicon_ulp",
        "UNIGL-class FP chain; best same-SKU separator in-browser",
    ));

    let timing = fo.get("eu_timing_ms").map(f64_vec).unwrap_or_default();
    let timing_ok = timing.len() >= 4 && curve_std(&timing) > 1e-6;
    channels.push(channel_row(
        "eu_draw_timing",
        timing_ok || timing_n > 0,
        timing_ok,
        timing.len(),
        curve_std(&timing),
        "strong",
        "weak",
        "low",
        "lane_s_secondary",
        "eu_timing_ms",
        "ulp path timing_samples",
        "DrawnApart-inspired; noisy under thermal/load",
    ));

    let audio = fo.get("hw_curve_audio").map(f64_vec).unwrap_or_default();
    let audio_ok = residual_curve_entropy_ok(&audio);
    channels.push(channel_row(
        "offline_audio",
        audio.len() >= 8,
        audio_ok,
        audio.len(),
        curve_std(&audio),
        "med-strong",
        "medium",
        "med",
        "commercial_dual_anchor",
        "hw_curve_audio",
        "B10_hw_curves / B46_audio_deep",
        "Coarse digest commercial; fine conf-only (engine micro-diff)",
    ));

    let webgpu = fo
        .get("hw_curve_webgpu")
        .or_else(|| fo.get("webgpu_compute_curve"))
        .map(f64_vec)
        .unwrap_or_default();
    let webgpu_ok = residual_curve_entropy_ok(&webgpu);
    channels.push(channel_row(
        "webgpu_compute",
        webgpu.len() >= 8,
        webgpu_ok,
        webgpu.len(),
        curve_std(&webgpu),
        "strong",
        "weak",
        "low",
        "fusion_conf_extended_ar",
        "hw_curve_webgpu",
        "B18_webgpu",
        "Absent on many clients; when present excellent silicon complement",
    ));

    let canvas = fo.get("hw_curve_canvas").map(f64_vec).unwrap_or_default();
    channels.push(channel_row(
        "canvas_noise",
        canvas.len() >= 8,
        residual_curve_entropy_ok(&canvas),
        canvas.len(),
        curve_std(&canvas),
        "weak",
        "weak",
        "high",
        "conf_dead_residual_fill",
        "hw_curve_canvas",
        "B10_hw_curves",
        "Browser rasterizer dominated; not silicon UV",
    ));

    let cpu = fo.get("hw_curve_cpu").map(f64_vec).unwrap_or_default();
    channels.push(channel_row(
        "cpu_timing",
        cpu.len() >= 8,
        residual_curve_entropy_ok(&cpu),
        cpu.len(),
        curve_std(&cpu),
        "weak-med",
        "weak",
        "high",
        "extended_segment_cp",
        "hw_curve_cpu",
        "B10 / B34 conf",
        "Load/thermal sensitive; B34 ladder never mint",
    ));

    let webrtc = fo
        .get("webrtc_host_ip_hash")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .is_some();
    let oi = fo
        .get("os_instance_hash")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .is_some();
    channels.push(channel_row(
        "host_separator",
        webrtc || oi,
        webrtc || oi,
        if webrtc || oi { 1 } else { 0 },
        0.0,
        "machine_env",
        "n/a",
        "low",
        "separator_not_silicon",
        "webrtc_host_ip_hash / os_instance_hash",
        "B10 / gateway",
        "Splits same residual class across hosts; not GPU die id",
    ));

    channels.push(channel_row(
        "gpu_model_string",
        fo.get("webgl_unmasked_renderer")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .is_some(),
        false,
        0,
        0.0,
        "none",
        "n/a",
        "total",
        "never_commercial_body",
        "webgl_unmasked_renderer",
        "B2 / static",
        "Spoofable label; claim-obs only",
    ));

    let fusion = fuse_silicon_channels(fields);
    let ac = build_anti_collision_surface(fields);

    let present_n = channels
        .iter()
        .filter(|c| c.get("present").and_then(|v| v.as_bool()).unwrap_or(false))
        .count();
    let strong_n = channels
        .iter()
        .filter(|c| {
            c.get("present").and_then(|v| v.as_bool()).unwrap_or(false)
                && matches!(
                    c.get("same_sku_split").and_then(|v| v.as_str()),
                    Some("strong") | Some("med-strong") | Some("machine_env")
                )
        })
        .count();
    let mean_q: f64 = {
        let qs: Vec<f64> = channels
            .iter()
            .filter_map(|c| c.get("quality_score").and_then(|v| v.as_f64()))
            .collect();
        if qs.is_empty() {
            0.0
        } else {
            qs.iter().sum::<f64>() / qs.len() as f64
        }
    };

    let recommendation = if strong_n >= 3 {
        "near_silicon_fleet_ready"
    } else if strong_n >= 2 {
        "strong_machine_need_more_lane_s"
    } else if present_n >= 3 {
        "config_plus_run_b10x_ulp_webgpu"
    } else {
        "insufficient_force_b10_b10x_b18"
    };

    json!({
        "algo": HW_CHANNEL_CENSUS_ALGO,
        "n_channels": channels.len(),
        "n_present": present_n,
        "n_strong_same_sku": strong_n,
        "mean_quality": (mean_q * 10000.0).round() / 10000.0,
        "residual_paths_n": paths.len(),
        "path_role_counts": {
            "float": n_float,
            "rint": n_rint,
            "noderiv": n_noderiv,
            "ulp": n_ulp,
            "timing_attached": timing_n,
        },
        "channels": channels,
        "fusion": {
            "grade": fusion.get("grade"),
            "same_sku_separability": fusion.get("same_sku_separability"),
            "hw_silicon_fusion": fusion.get("hw_silicon_fusion"),
            "hw_silicon_fine": fusion.get("hw_silicon_fine"),
        },
        "anti_collision": {
            "strategy": ac.get("strategy"),
            "composite": ac.get("composite_digest"),
            "single_curve_collision_prone": ac.get("single_curve_collision_prone"),
        },
        "recommendation": recommendation,
        "promote_to_silicon_uv": false,
        "comparison_note": "strong same-SKU = ulp+timing+audio/webgpu; lane_c alone = class risk under ANGLE",
    })
}

/// Attach census into materials map for commercial_projection / ops.
pub fn channel_census_materials(fields: &Value) -> Map<String, Value> {
    let c = hw_channel_census(fields);
    let mut m = Map::new();
    m.insert("hw_channel_census".into(), c.clone());
    if let Some(r) = c.get("recommendation").cloned() {
        m.insert("hw_channel_recommendation".into(), r);
    }
    if let Some(g) = c.pointer("/fusion/grade").cloned() {
        m.insert("hw_channel_fusion_grade".into(), g);
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn census_flags_missing_ulp() {
        let f = json!({
            "hw_curve_webgl": (0..32).map(|i| 0.03 + (i as f64)*0.0001).collect::<Vec<_>>(),
            "hw_curve_audio": (0..64).map(|i| ((i as f64)*0.1).sin().abs()).collect::<Vec<_>>(),
            "residual_paths": [
                {"path_id":"float","shader_mode":"float","ok":true,"curve": (0..32).map(|i| 0.03+(i as f64)*0.0001).collect::<Vec<_>>()},
            ],
        });
        let c = hw_channel_census(&f);
        assert!(c["n_present"].as_u64().unwrap() >= 2);
        assert_eq!(c["path_role_counts"]["ulp"], 0);
        assert!(
            c["recommendation"]
                .as_str()
                .unwrap()
                .contains("ulp")
                || c["recommendation"]
                    .as_str()
                    .unwrap()
                    .contains("insufficient")
                || c["recommendation"]
                    .as_str()
                    .unwrap()
                    .contains("config")
        );
    }

    #[test]
    fn census_near_silicon_with_full_stack() {
        let ulp: Vec<f64> = (0..32)
            .map(|i| ((i as f64) * 0.31).sin().abs() * 0.5 + 0.05)
            .collect();
        let f = json!({
            "hw_curve_webgl": ulp.clone(),
            "hw_curve_audio": (0..64).map(|i| ((i as f64)*0.17).sin().abs()*0.6+0.05).collect::<Vec<_>>(),
            "hw_curve_webgpu": (0..32).map(|i| ((i as f64)*0.41).cos().abs()*0.4+0.04).collect::<Vec<_>>(),
            "eu_timing_ms": [1.2,1.5,1.1,1.8,1.3,1.4,1.6,1.2],
            "residual_paths": [
                {"path_id":"float","shader_mode":"float","ok":true,"curve": ulp.clone()},
                {"path_id":"ulp","shader_mode":"ulp","ok":true,"curve": ulp.clone(), "eu_timing_ms": [1.2,1.5,1.1,1.8]},
            ],
            "webrtc_host_ip_hash": "abc123",
        });
        let c = hw_channel_census(&f);
        assert!(c["n_strong_same_sku"].as_u64().unwrap() >= 2);
        assert!(c["fusion"]["same_sku_separability"].as_f64().unwrap() > 0.4);
    }
}
