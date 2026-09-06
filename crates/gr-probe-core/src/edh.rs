//! Evidence-first Device Hypothesis (EDH) — research/identity architecture.
//!
//! **Not** force-associate `dh_`. Emit multi-layer evidence + spoof/channel scores
//! so fingerprint-browser surfaces and true silicon noise can be reasoned about
//! separately. Compatible with existing mint: EDH attaches alongside `device`.

use serde_json::{json, Map, Value};

fn s(fo: &Map<String, Value>, k: &str) -> String {
    fo.get(k)
        .and_then(|v| v.as_str())
        .map(|x| x.to_string())
        .unwrap_or_default()
}

fn f64_of(fo: &Map<String, Value>, k: &str) -> Option<f64> {
    fo.get(k).and_then(|v| {
        v.as_f64()
            .or_else(|| v.as_i64().map(|i| i as f64))
            .or_else(|| v.as_str().and_then(|t| t.parse().ok()))
    })
}

fn has_b10x_batch(evidence: &Value) -> bool {
    let check = |id: &str| id.starts_with("B10x_");
    if let Some(arr) = evidence.get("batches").and_then(|v| v.as_array()) {
        for b in arr {
            let id = b
                .get("batch_id")
                .or_else(|| b.get("pack_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if check(id) {
                return true;
            }
        }
    }
    // Only explicit B10x batch/pack markers — B10 multipath may already contain
    // noderiv path ids without any B10x pack having landed.
    if let Some(fo) = evidence.get("fields").and_then(|v| v.as_object()) {
        if fo.get("b10x_pack").and_then(|v| v.as_str()).is_some_and(|s| s.starts_with("B10x_")) {
            return true;
        }
        if fo.get("b10x_ok").and_then(|v| v.as_bool()) == Some(true)
            && fo.get("b10x_pack").is_some()
        {
            return true;
        }
    }
    false
}

/// Count B10x-prefixed batches in evidence.
pub fn count_b10x_batches(evidence: &Value) -> usize {
    let mut n = 0usize;
    if let Some(arr) = evidence.get("batches").and_then(|v| v.as_array()) {
        for b in arr {
            let id = b
                .get("batch_id")
                .or_else(|| b.get("pack_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if id.starts_with("B10x_") {
                n += 1;
            }
        }
    }
    n
}

pub fn evidence_has_b10x(evidence: &Value) -> bool {
    has_b10x_batch(evidence)
}

/// Silicon B10x packs that must land for measurement completeness (research gate).
/// Terminal cool requires these three; `B10x_silicon_deep` is preferred deepen (evaluate injects
/// after trio complete) — not hard-required so weak/no-context devices can still finalize.
pub fn b10x_silicon_required_packs() -> &'static [&'static str] {
    &[
        "B10x_silicon_ulp",
        "B10x_silicon_noderiv",
        "B10x_silicon_rint",
    ]
}

/// Preferred deepen packs (schedule after required trio; parallel classes where possible).
pub fn secondary_silicon_infra_packs() -> &'static [&'static str] {
    &[
        "B10x_silicon_deep", // gpu: fma/denorm/tex/interp
        "B47_sab_clock",     // cpu ∥
        "B18_webgpu",        // gpu: f32+f16 compute
        "B46_audio_deep",    // audio ∥ : dual-seed + convolver
    ]
}

/// Terminal attempt: success, honest failure, or final fields — NOT a bare `started` heartbeat.
pub fn b10x_fields_terminal(fo: &Map<String, Value>) -> bool {
    if fo.get("b10x_ok").and_then(|v| v.as_bool()) == Some(true) {
        return true;
    }
    let err = fo.get("b10x_err").and_then(|v| v.as_str()).unwrap_or("");
    let phase = fo.get("b10x_phase").and_then(|v| v.as_str()).unwrap_or("");
    if !err.is_empty() && err != "started" {
        return true;
    }
    if !phase.is_empty() && phase != "start" {
        return true;
    }
    if fo.get("residual_paths_n").and_then(|v| v.as_u64()).is_some() {
        return true;
    }
    if fo.get("residual_paths").and_then(|v| v.as_array()).is_some_and(|a| !a.is_empty()) {
        return true;
    }
    if fo.get("hw_curve_webgl").is_some() {
        return true;
    }
    // Explicit honest skip without curve
    if fo.get("b10x_ok").and_then(|v| v.as_bool()) == Some(false)
        && !err.is_empty()
        && err != "started"
    {
        return true;
    }
    false
}

fn batch_fields_map(batch: &Value) -> Option<Map<String, Value>> {
    batch
        .pointer("/payload/fields")
        .or_else(|| batch.get("fields"))
        .and_then(|v| v.as_object())
        .cloned()
}

/// True when this silicon pack has a terminal (success or honest-fail) attempt.
pub fn silicon_pack_attempt_complete(evidence: &Value, pack_id: &str) -> bool {
    if let Some(arr) = evidence.get("batches").and_then(|v| v.as_array()) {
        for b in arr {
            let id = b
                .get("batch_id")
                .or_else(|| b.get("pack_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if id != pack_id {
                continue;
            }
            if let Some(fo) = batch_fields_map(b) {
                if b10x_fields_terminal(&fo) {
                    return true;
                }
                // Batch row exists but only started heartbeat — not complete yet.
                let err = fo.get("b10x_err").and_then(|v| v.as_str()).unwrap_or("");
                let phase = fo.get("b10x_phase").and_then(|v| v.as_str()).unwrap_or("");
                if err == "started" || phase == "start" {
                    return false;
                }
            }
            // Batch present without phase markers (legacy / force dry-run) counts as attempt.
            return true;
        }
    }
    // Merged fields path: pack marker + terminal err/ok (when batch merge lagging).
    if let Some(fo) = evidence.get("fields").and_then(|v| v.as_object()) {
        let marked = fo
            .get("b10x_pack")
            .and_then(|v| v.as_str())
            .is_some_and(|s| s == pack_id);
        if marked && b10x_fields_terminal(fo) {
            return true;
        }
    }
    false
}

/// Missing silicon B10x packs (no terminal attempt yet).
/// Honest failure (`b10x_ok:false` + `b10x_err`) counts as landed; bare `started` does not.
pub fn missing_b10x_silicon(evidence: &Value) -> Vec<String> {
    b10x_silicon_required_packs()
        .iter()
        .filter(|p| !silicon_pack_attempt_complete(evidence, p))
        .map(|s| (*s).to_string())
        .collect()
}

fn spoof_signals(fo: &Map<String, Value>) -> (f64, Vec<String>) {
    let mut reasons = Vec::new();
    let mut score = 0.0f64;
    let renderer = s(fo, "webgl_unmasked_renderer").to_lowercase();
    let vendor = s(fo, "webgl_unmasked_vendor").to_lowercase();
    let eng = s(fo, "engine_family").to_lowercase();
    let residual_ok = fo.get("residual_ok").and_then(|v| v.as_bool()) == Some(true)
        || f64_of(fo, "residual_std").map(|x| x > 0.0).unwrap_or(false);
    let mean = f64_of(fo, "residual_mean").unwrap_or(0.0);

    // Apple GPU label on Linux desktop residual that looks like discrete NVIDIA/AMD path
    if renderer.contains("apple") || vendor.contains("apple") {
        let plat = s(fo, "platform").to_lowercase() + &s(fo, "user_agent").to_lowercase();
        if plat.contains("linux") || plat.contains("x11") {
            score += 0.35;
            reasons.push("gpu_label_apple_on_linux".into());
        }
    }
    // ANGLE/NVIDIA residual-ish mean with soft Apple label already covered;
    // concurrency spoof: very low cores with high residual entropy desktop
    if let Some(hc) = f64_of(fo, "hardware_concurrency") {
        if hc > 0.0 && hc <= 4.0 && residual_ok && mean > 0.15 {
            score += 0.12;
            reasons.push("low_concurrency_vs_desktop_residual".into());
        }
    }
    // WebGPU absent + unmasked blocked often fingerprint stack
    if s(fo, "webgpu_adapter_skip") == "no_gpu" && eng == "webkit" {
        score += 0.08;
        reasons.push("webgpu_absent_webkit".into());
    }
    if fo.get("webgl_unmask_blocked").and_then(|v| v.as_bool()) == Some(true) {
        score += 0.1;
        reasons.push("webgl_unmask_blocked".into());
    }
    // Client Hints skip on blink is normal; on "desktop chrome-like" webkit is odd
    if s(fo, "ua_ch_high_entropy_skip").contains("no_userAgentData") && eng == "webkit" {
        score += 0.05;
        reasons.push("ua_ch_absent_webkit".into());
    }
    // Nest lite residual polluting commercial keys historically → spoof/channel noise
    if s(fo, "residual_algo").contains("nest_webgl_hist_lite") {
        score += 0.15;
        reasons.push("nest_lite_residual_on_session_view".into());
    }
    score = score.min(1.0);
    (score, reasons)
}

/// Build EDH object for attach into analyze result.
pub fn build_edh(evidence: &Value, device: &Value) -> Value {
    let fo = evidence
        .get("fields")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let eng = s(&fo, "engine_family");
    let profile = s(&fo, "probe_profile");
    let residual_mean = f64_of(&fo, "residual_mean");
    let residual_std = f64_of(&fo, "residual_std");
    let paths_n = fo
        .get("residual_paths_n")
        .and_then(|v| v.as_u64())
        .or_else(|| {
            fo.get("residual_paths")
                .and_then(|v| v.as_array())
                .map(|a| a.len() as u64)
        })
        .unwrap_or(0);
    let b10x_n = count_b10x_batches(evidence);
    let b10x_ok = evidence_has_b10x(evidence);
    let missing_silicon = missing_b10x_silicon(evidence);
    let (spoof, spoof_reasons) = spoof_signals(&fo);

    let device_id = device
        .get("device_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let digest_path = device
        .get("digest_path")
        .or_else(|| device.pointer("/trust/digest_path"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let hw_webgl = device
        .pointer("/trust/materials/hw_webgl_stable")
        .or_else(|| device.pointer("/materials/hw_webgl_stable"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let hw_audio = device
        .pointer("/trust/materials/hw_audio_stable")
        .or_else(|| device.pointer("/materials/hw_audio_stable"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Noise bucket: commercial stable digests + residual quanta (not force-link)
    let noise_bucket = format!(
        "nb_{}_{}_{}",
        if hw_webgl.is_empty() {
            "none"
        } else {
            &hw_webgl[hw_webgl.len().saturating_sub(12)..]
        },
        residual_mean
            .map(|m| format!("{:.3}", m))
            .unwrap_or_else(|| "na".into()),
        residual_std
            .map(|s| format!("{:.3}", s))
            .unwrap_or_else(|| "na".into())
    );

    let surface_bits = [
        s(&fo, "user_agent"),
        s(&fo, "platform"),
        eng.clone(),
        s(&fo, "webgl_unmasked_renderer"),
        f64_of(&fo, "hardware_concurrency")
            .map(|c| format!("hc{}", c as i64))
            .unwrap_or_default(),
    ]
    .join("|");
    let surface_hash = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        surface_bits.hash(&mut h);
        format!("sf_{:016x}", h.finish())
    };

    let channel_id = if eng.is_empty() {
        "ch_unknown".into()
    } else {
        format!("ch_{}_{}", eng, if profile.is_empty() { "def" } else { &profile })
    };

    // Hypotheses (scores sum not required to 1)
    let mut hypotheses = Vec::new();
    let measurement_ready = b10x_ok && paths_n >= 3 && residual_mean.is_some();
    let spoof_hi = spoof >= 0.35;
    let same_silicon_base = if measurement_ready && !spoof_hi {
        0.55
    } else if residual_mean.is_some() && !spoof_hi {
        0.35
    } else {
        0.15
    };
    hypotheses.push(json!({
        "id": "H_same_silicon",
        "score": same_silicon_base,
        "note": "curve/noise cluster only — not force dh merge",
    }));
    hypotheses.push(json!({
        "id": "H_engine_channel",
        "score": if eng == "webkit" { 0.45 } else { 0.25 },
        "note": "same host different browser measurement channel",
    }));
    hypotheses.push(json!({
        "id": "H_spoof_surface",
        "score": spoof,
        "reasons": spoof_reasons,
    }));
    hypotheses.push(json!({
        "id": "H_virtual_gpu",
        "score": if s(&fo, "gl_stack_class").contains("software") { 0.6 } else { 0.1 },
    }));

    let top = hypotheses
        .iter()
        .max_by(|a, b| {
            let sa = a.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let sb = b.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
            sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
        })
        .cloned()
        .unwrap_or(json!({"id": "H_unknown", "score": 0.0}));

    json!({
        "algo": "evidence_device_hypothesis_v1",
        "policy": "no_force_dh_associate; probe_deep_upload_all; analyze_joint_hw_sw",
        "compatible_device_id": device_id,
        "digest_path": digest_path,
        "layers": {
            "noise": {
                "noise_bucket": noise_bucket,
                "hw_webgl_stable": hw_webgl,
                "hw_audio_stable": hw_audio,
                "residual_mean": residual_mean,
                "residual_std": residual_std,
                "residual_paths_n": paths_n,
                "residual_algo": s(&fo, "residual_algo"),
            },
            "surface": {
                "surface_id": surface_hash,
                "engine_family": eng,
                "probe_profile": profile,
                "webgl_unmasked_renderer": s(&fo, "webgl_unmasked_renderer"),
                "hardware_concurrency": f64_of(&fo, "hardware_concurrency"),
                "device_memory": f64_of(&fo, "device_memory"),
            },
            "channel": {
                "channel_id": channel_id,
                "gl_stack_class": s(&fo, "gl_stack_class"),
            },
            "measurement": {
                "b10x_batches_n": b10x_n,
                "b10x_landed": b10x_ok,
                "missing_b10x_silicon": missing_silicon,
                "webgpu_skip": s(&fo, "webgpu_adapter_skip"),
            }
        },
        "spoof": {
            "likelihood": spoof,
            "reasons": spoof_reasons,
        },
        "hypotheses": hypotheses,
        "top_hypothesis": top,
        "research_gate": {
            "b10x_required": true,
            "b10x_complete": missing_b10x_silicon(evidence).is_empty(),
            "note": "complete when each silicon pack has terminal attempt (success or honest fail); started heartbeat alone is not enough",
        }
    })
}
