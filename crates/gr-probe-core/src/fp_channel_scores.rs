//! Four-channel fingerprint / kernel-spoof observability (`fp_channel_scores_v1`).
//!
//! Aggregates surface conflict, channel invariance, replay hardness, and sandbox
//! layer consistency for lab/product observability. Does **not** change commercial
//! `dh_` mint rules.

use crate::stack_auth::StackAuth;
use serde_json::{json, Map, Value};

fn clamp01(x: f64) -> f64 {
    if x.is_nan() {
        0.0
    } else {
        x.clamp(0.0, 1.0)
    }
}

fn f_field(fo: &Map<String, Value>, key: &str) -> Option<f64> {
    fo.get(key).and_then(|v| {
        v.as_f64()
            .or_else(|| v.as_i64().map(|i| i as f64))
            .or_else(|| v.as_u64().map(|u| u as f64))
    })
}

fn bool_field(fo: &Map<String, Value>, key: &str) -> Option<bool> {
    fo.get(key).and_then(|v| v.as_bool())
}

fn str_field<'a>(fo: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    fo.get(key).and_then(|v| v.as_str()).filter(|s| !s.is_empty())
}

fn present(fo: &Map<String, Value>, key: &str) -> bool {
    match fo.get(key) {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(_) => true,
    }
}

fn push_hit(hits: &mut Vec<String>, tag: &str) {
    if !hits.iter().any(|h| h == tag) {
        hits.push(tag.into());
    }
}

fn dim(score: f64, hits: Vec<String>) -> Value {
    json!({
        "score": (score * 1000.0).round() / 1000.0,
        "hits": hits,
    })
}

/// Build `fp_channel_scores_v1` from session fields + stack auth.
pub fn build_fp_channel_scores(fields: &Value, stack: &StackAuth) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();

    // --- surface_conflict (high = more contradiction) ---
    let mut sc_hits = Vec::new();
    let mut sc = 0.0;
    if stack.gpu_label_untrusted {
        sc += 0.35;
        push_hit(&mut sc_hits, "gpu_label_untrusted");
    }
    if stack.spoof_score >= 0.45 {
        sc += 0.2;
        push_hit(&mut sc_hits, "spoof_score_mid");
    } else if stack.spoof_score >= 0.25 {
        sc += 0.1;
        push_hit(&mut sc_hits, "spoof_score_low");
    }
    for r in &stack.reasons {
        if r.contains("gpu_label_vs_") {
            sc += 0.15;
            push_hit(&mut sc_hits, r);
        }
    }
    let eng_claim = str_field(&fo, "engine_claim").unwrap_or("unknown");
    let eng_obs = str_field(&fo, "engine_obs").unwrap_or("unknown");
    if eng_claim != "unknown" && eng_obs != "unknown" && eng_claim != eng_obs {
        sc += 0.25;
        push_hit(&mut sc_hits, "engine_claim_obs_mismatch");
    }
    if bool_field(&fo, "mobile_ua_claim").unwrap_or(false)
        && str_field(&fo, "form_class") == Some("desktop")
    {
        sc += 0.15;
        push_hit(&mut sc_hits, "ua_form_spoof_suspect");
    }
    // concurrency vs worker / throughput mismatch (if both present)
    if let (Some(hc), Some(wt)) = (
        f_field(&fo, "hardware_concurrency"),
        f_field(&fo, "worker_throughput")
            .or_else(|| f_field(&fo, "cpu_worker_ops_per_ms")),
    ) {
        if hc > 0.0 && wt > 0.0 {
            let expected = hc * 0.15;
            if wt < expected * 0.25 || wt > expected * 8.0 {
                sc += 0.12;
                push_hit(&mut sc_hits, "concurrency_vs_throughput");
            }
        }
    }
    if present(&fo, "webrtc_host_ip_hash") || present(&fo, "webrtc_host_ips") {
        if bool_field(&fo, "webrtc_host_vs_ua_mismatch").unwrap_or(false)
            || bool_field(&fo, "webrtc_form_mismatch").unwrap_or(false)
        {
            sc += 0.18;
            push_hit(&mut sc_hits, "webrtc_host_vs_ua_form");
        } else if !sc_hits.iter().any(|h| h.starts_with("webrtc")) {
            push_hit(&mut sc_hits, "webrtc_present_thin");
        }
    }
    if sc_hits.is_empty() && !present(&fo, "webgl_unmasked_renderer") {
        push_hit(&mut sc_hits, "surface_thin");
    }
    let surface_conflict = clamp01(sc);

    // --- channel_invariance (high = more invariant / stable) ---
    let mut ci_hits = Vec::new();
    let mut ci = 0.55; // neutral baseline when materials thin
    // Only score nest vs main when FE marks scales comparable (hist-lite ≠ residual).
    let nest_comparable = bool_field(&fo, "nest_vs_main_comparable").unwrap_or_else(|| {
        match (
            f_field(&fo, "nest_residual_mean"),
            f_field(&fo, "residual_mean"),
        ) {
            (Some(n), Some(m)) => (m >= 0.15 && n >= 0.15) || (m < 0.12 && n < 0.12),
            _ => true,
        }
    });
    if nest_comparable {
        if let Some(agree) = bool_field(&fo, "nest_vs_main_agree_0p001") {
            if agree {
                ci += 0.25;
                push_hit(&mut ci_hits, "nest_vs_main_agree");
            } else {
                ci -= 0.3;
                push_hit(&mut ci_hits, "nest_vs_main_disagree");
            }
        }
    } else if present(&fo, "nest_residual_mean") {
        push_hit(&mut ci_hits, "nest_vs_main_incomparable");
    }
    if let Some(r) = f_field(&fo, "multi_source_match_ratio") {
        if r >= 0.85 {
            ci += 0.15;
            push_hit(&mut ci_hits, "multi_source_high");
        } else if r < 0.5 {
            ci -= 0.2;
            push_hit(&mut ci_hits, "multi_source_low");
        }
    }
    // noderiv/rint path agreement among residual_paths
    if let Some(paths) = fo.get("residual_paths").and_then(|v| v.as_array()) {
        let mut means = Vec::new();
        for p in paths {
            let mode = p.get("shader_mode").and_then(|v| v.as_str()).unwrap_or("");
            let pid = p.get("path_id").and_then(|v| v.as_str()).unwrap_or("");
            let ok = p.get("ok").and_then(|v| v.as_bool()).unwrap_or(false)
                || p.get("entropy_ok").and_then(|v| v.as_bool()).unwrap_or(false);
            if !ok {
                continue;
            }
            if mode == "noderiv"
                || mode == "rint"
                || pid.contains("noderiv_")
                || pid.contains("rint_")
            {
                if let Some(m) = p.get("mean").and_then(|v| v.as_f64()) {
                    means.push(m);
                }
            }
        }
        if means.len() >= 2 {
            let mn = means.iter().cloned().fold(f64::INFINITY, f64::min);
            let mx = means.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            if (mx - mn).abs() < 0.001 {
                ci += 0.2;
                push_hit(&mut ci_hits, "noderiv_rint_path_agree");
            } else if (mx - mn).abs() > 0.002 {
                ci -= 0.15;
                push_hit(&mut ci_hits, "noderiv_rint_path_diverge");
            } else {
                push_hit(&mut ci_hits, "noderiv_rint_path_near");
            }
        } else if !means.is_empty() {
            push_hit(&mut ci_hits, "noderiv_rint_path_thin");
        }
    }
    if let Some(lane) = fo
        .get("residual_select")
        .and_then(|rs| {
            rs.get("lane_c")
                .and_then(|c| c.get("chosen_path_id"))
                .or_else(|| rs.get("chosen_path_id"))
        })
        .and_then(|v| v.as_str())
    {
        if lane.contains("noderiv") || lane.contains("rint") {
            ci += 0.05;
            push_hit(&mut ci_hits, "lane_c_stable_mode");
        } else if lane.contains("ulp") {
            ci -= 0.1;
            push_hit(&mut ci_hits, "lane_c_ulp_leak");
        } else if lane.contains("unk") || lane.contains("float") || lane.contains("warm4_v3f") {
            ci -= 0.05;
            push_hit(&mut ci_hits, "lane_c_float_warm");
        }
    }
    if ci_hits.is_empty() {
        push_hit(&mut ci_hits, "channel_thin");
    }
    let channel_invariance = clamp01(ci);

    // --- replay_hardness (high = coarse stable + fine diverge / anti-forge) ---
    let mut rh_hits = Vec::new();
    let mut rh = 0.4;
    let seed_res = present(&fo, "seed_residual_digest") || present(&fo, "challenge_hist_digest");
    let seed_ulp = present(&fo, "seed_ulp_digest");
    if seed_res {
        rh += 0.1;
        push_hit(&mut rh_hits, "seed_residual_present");
    }
    if let Some(agree) = bool_field(&fo, "seed_replay_agree_0p001") {
        if agree {
            rh += 0.15;
            push_hit(&mut rh_hits, "seed_coarse_agree");
        } else {
            rh -= 0.2;
            push_hit(&mut rh_hits, "seed_coarse_disagree");
        }
    }
    if let Some(ulp_agree) = bool_field(&fo, "seed_ulp_agree") {
        if bool_field(&fo, "seed_replay_agree_0p001").unwrap_or(false) && !ulp_agree {
            rh += 0.35;
            push_hit(&mut rh_hits, "coarse_ok_ulp_diverge");
        } else if ulp_agree && seed_ulp {
            rh += 0.1;
            push_hit(&mut rh_hits, "seed_ulp_stable");
        } else if !ulp_agree {
            push_hit(&mut rh_hits, "seed_ulp_diverge");
        }
    } else if seed_ulp {
        push_hit(&mut rh_hits, "seed_ulp_present");
    }
    if bool_field(&fo, "noise_suspect").unwrap_or(false) {
        rh -= 0.15;
        push_hit(&mut rh_hits, "challenge_noise_suspect");
    }
    if let Some(cv) = f_field(&fo, "challenge_avg_cv") {
        if cv > 0.08 {
            rh -= 0.1;
            push_hit(&mut rh_hits, "challenge_cv_high");
        } else if cv < 0.01 && present(&fo, "challenge_residual_mean") {
            rh += 0.05;
            push_hit(&mut rh_hits, "challenge_cv_tight");
        }
    }
    if !seed_res && !present(&fo, "challenge_residual_mean") {
        push_hit(&mut rh_hits, "replay_thin");
        rh -= 0.05;
    }
    let replay_hardness = clamp01(rh);

    // --- sandbox_consistency (high = layers agree / consistent) ---
    let mut sb_hits = Vec::new();
    let mut sb = 0.55;
    if let Some(r) = f_field(&fo, "multi_source_match_ratio") {
        sb = r;
        push_hit(&mut sb_hits, "multi_source_match_ratio");
    }
    if bool_field(&fo, "iframe_ua_mismatch").unwrap_or(false)
        || bool_field(&fo, "sandbox_ua_mismatch").unwrap_or(false)
    {
        sb -= 0.25;
        push_hit(&mut sb_hits, "iframe_ua_mismatch");
    }
    if bool_field(&fo, "iframe_platform_mismatch").unwrap_or(false)
        || bool_field(&fo, "sandbox_platform_mismatch").unwrap_or(false)
    {
        sb -= 0.15;
        push_hit(&mut sb_hits, "iframe_platform_mismatch");
    }
    if bool_field(&fo, "sandbox_hw_mismatch").unwrap_or(false) {
        sb -= 0.15;
        push_hit(&mut sb_hits, "sandbox_hw_mismatch");
    }
    if bool_field(&fo, "sandbox_tostring_diverged").unwrap_or(false) {
        sb -= 0.1;
        push_hit(&mut sb_hits, "sandbox_tostring_diverged");
    }
    if let Some(cap) = f_field(&fo, "sandbox_capability_score") {
        if cap < 0.2 {
            sb -= 0.2;
            push_hit(&mut sb_hits, "sandbox_capability_dead");
        } else if cap >= 0.8 {
            sb += 0.1;
            push_hit(&mut sb_hits, "sandbox_capability_ok");
        }
    }
    if present(&fo, "nest_residual_mean") {
        push_hit(&mut sb_hits, "nest_residual_mean");
        if nest_comparable && bool_field(&fo, "nest_vs_main_agree_0p001") == Some(false) {
            sb -= 0.2;
            push_hit(&mut sb_hits, "nest_residual_disagree");
        }
    }
    if present(&fo, "nest_engine_family") {
        push_hit(&mut sb_hits, "nest_engine_family");
        if let (Some(main_e), Some(nest_e)) = (
            str_field(&fo, "engine_family").or_else(|| str_field(&fo, "engine_obs")),
            str_field(&fo, "nest_engine_family"),
        ) {
            if main_e != nest_e && nest_e != "unknown" && main_e != "unknown" {
                sb -= 0.15;
                push_hit(&mut sb_hits, "nest_engine_mismatch");
            }
        }
    }
    if sb_hits.is_empty() {
        push_hit(&mut sb_hits, "sandbox_thin");
    }
    let sandbox_consistency = clamp01(sb);

    let surface_high = surface_conflict >= 0.55;
    let channel_low = channel_invariance < 0.45;
    let sandbox_low = sandbox_consistency < 0.45;
    let replay_soft = replay_hardness < 0.4;
    let fp_suspect = surface_high && (channel_low || sandbox_low || replay_soft);

    let note = if fp_suspect {
        "fp_channel_suspect: high surface conflict with weak invariance/sandbox/replay"
    } else if surface_conflict < 0.2 && sc_hits.iter().any(|h| h.contains("thin")) {
        "fp_channel_scores: thin surface materials; scores informational"
    } else {
        "fp_channel_scores_v1: observability only; does not alter dh_ mint"
    };

    json!({
        "schema": "fp_channel_scores_v1",
        "surface_conflict": dim(surface_conflict, sc_hits),
        "channel_invariance": dim(channel_invariance, ci_hits),
        "replay_hardness": dim(replay_hardness, rh_hits),
        "sandbox_consistency": dim(sandbox_consistency, sb_hits),
        "fp_suspect": fp_suspect,
        "note": note,
    })
}

/// Append `fp_channel_suspect` to belief adversary_tags when scores flag it.
pub fn apply_fp_channel_adversary_tag(belief: &mut Value, scores: &Value) {
    let suspect = scores
        .get("fp_suspect")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !suspect {
        return;
    }
    let Some(obj) = belief.as_object_mut() else {
        return;
    };
    let tags = obj
        .entry("adversary_tags")
        .or_insert_with(|| json!([]));
    if let Some(arr) = tags.as_array_mut() {
        if !arr.iter().any(|t| t.as_str() == Some("fp_channel_suspect")) {
            arr.push(json!("fp_channel_suspect"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stack_auth::stack_auth_from_fields;

    #[test]
    fn smoke_four_dims_and_suspect() {
        let fields = json!({
            "engine_claim": "blink",
            "engine_obs": "gecko",
            "form_class": "desktop",
            "mobile_ua_claim": true,
            "nest_vs_main_agree_0p001": false,
            "multi_source_match_ratio": 0.2,
            "iframe_ua_mismatch": true,
            "seed_residual_digest": "sr_abc",
            "seed_ulp_digest": "su_def",
            "seed_replay_agree_0p001": true,
            "seed_ulp_agree": false,
            "residual_paths": [
                {"path_id": "noderiv_hard_warm4", "ok": true, "entropy_ok": true, "shader_mode": "noderiv", "mean": 0.26},
                {"path_id": "noderiv_hard_warm2", "ok": true, "entropy_ok": true, "shader_mode": "noderiv", "mean": 0.2605}
            ],
            "webgl_unmasked_renderer": "Apple GPU"
        });
        let stack = stack_auth_from_fields(&fields);
        let scores = build_fp_channel_scores(&fields, &stack);
        assert_eq!(scores["schema"], "fp_channel_scores_v1");
        assert!(scores["surface_conflict"]["score"].as_f64().unwrap() >= 0.3);
        assert!(scores["replay_hardness"]["score"].as_f64().unwrap() >= 0.5);
        assert!(scores.get("channel_invariance").is_some());
        assert!(scores.get("sandbox_consistency").is_some());
        // high surface + low sandbox/channel → suspect
        let mut belief = json!({"adversary_tags": []});
        apply_fp_channel_adversary_tag(&mut belief, &scores);
        if scores["fp_suspect"].as_bool().unwrap_or(false) {
            assert!(belief["adversary_tags"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t.as_str() == Some("fp_channel_suspect")));
        }
    }

    #[test]
    fn seeded_replay_fields_parsed() {
        let fields = json!({
            "seed_residual_digest": "sr_1",
            "seed_ulp_digest": "su_1",
            "seed_replay_agree_0p001": true,
            "seed_ulp_agree": false,
            "challenge_avg_cv": 0.001,
            "challenge_residual_mean": 0.26,
        });
        let stack = stack_auth_from_fields(&json!({}));
        let scores = build_fp_channel_scores(&fields, &stack);
        let hits = scores["replay_hardness"]["hits"].as_array().unwrap();
        assert!(hits.iter().any(|h| h.as_str() == Some("coarse_ok_ulp_diverge")));
        assert!(scores["replay_hardness"]["score"].as_f64().unwrap() >= 0.6);
    }
}
