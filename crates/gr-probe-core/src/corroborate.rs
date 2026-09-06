//! Multi-channel conflict-first fusion for product axes (v5 hedge architecture).
//!
//! Design rules:
//! 1. Never trust a single field, single FE source, or single analyzer alone.
//! 2. Any Contradict channel caps fused safety (<0.5) and blocks "real".
//! 3. Support is weighted by coverage; incomplete channels lower coverage, not invent agreement.
//! 4. Material vote: product-critical axes need ≥2 independent material families.

use serde_json::{Map, Value};

/// Single corroboration channel output.
#[derive(Debug, Clone)]
pub struct ChannelOut {
    pub id: &'static str,
    /// Higher = safer (polarity already flipped).
    pub safety: f64,
    pub coverage: f64,
    pub stance: Stance,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stance {
    Support,
    Neutral,
    Contradict,
}

fn clamp01(x: f64) -> f64 {
    if x.is_nan() {
        0.0
    } else {
        x.clamp(0.0, 1.0)
    }
}

fn has_nonempty(fields: &Map<String, Value>, key: &str) -> bool {
    match fields.get(key) {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(_) => true,
    }
}

fn f_field(fields: &Map<String, Value>, key: &str) -> Option<f64> {
    fields.get(key).and_then(|v| {
        v.as_f64()
            .or_else(|| v.as_i64().map(|i| i as f64))
            .or_else(|| v.as_u64().map(|u| u as f64))
    })
}

fn bool_field(fields: &Map<String, Value>, key: &str) -> Option<bool> {
    fields.get(key).and_then(|v| v.as_bool())
}

/// Conflict-first fusion: any Contradict channel caps safety and blocks "real" status.
/// Also enforces multi-channel: fewer than 2 non-empty channels cannot be "real".
pub fn fuse_channels(chs: &[ChannelOut], min_cov: f64) -> (f64, &'static str, Vec<String>) {
    let mut reasons = Vec::new();
    if chs.is_empty() {
        reasons.push("no_channels".into());
        return (0.35, "unknown", reasons);
    }
    let active: Vec<&ChannelOut> = chs
        .iter()
        .filter(|c| c.coverage >= 0.05 || c.stance != Stance::Neutral)
        .collect();
    let n_active = active.len().max(chs.len().min(1));
    let cov = chs.iter().map(|c| c.coverage).sum::<f64>() / chs.len() as f64;
    if cov < min_cov {
        reasons.push("insufficient_coverage".into());
        return (0.35_f64.min(cov), "unknown", reasons);
    }
    let has_conflict = chs.iter().any(|c| c.stance == Stance::Contradict);
    if has_conflict {
        let capped = chs.iter().map(|c| c.safety).fold(1.0_f64, f64::min);
        for c in chs.iter().filter(|c| c.stance == Stance::Contradict) {
            reasons.extend(c.reasons.iter().cloned());
            reasons.push(format!("conflict:{}", c.id));
        }
        // Conflict cannot average into high trust
        let safety = capped.min(0.49);
        return (safety, "suspect", reasons);
    }
    // Single active channel: never "real" (no single-source truth)
    if n_active < 2 {
        reasons.push("single_channel_no_real".into());
        let s = chs
            .iter()
            .map(|c| c.safety * (0.25 + 0.75 * c.coverage))
            .sum::<f64>()
            / chs
                .iter()
                .map(|c| 0.25 + 0.75 * c.coverage)
                .sum::<f64>()
                .max(1e-9);
        let safety = clamp01(s.min(0.65));
        let status = if safety >= 0.45 { "suspect" } else { "fake" };
        return (safety, status, reasons);
    }
    let mut wsum = 0.0;
    let mut s = 0.0;
    for c in chs {
        let w = 0.25 + 0.75 * c.coverage;
        s += c.safety * w;
        wsum += w;
        reasons.extend(c.reasons.iter().cloned());
    }
    let safety = clamp01(if wsum > 0.0 { s / wsum } else { 0.0 });
    let status = if safety >= 0.72 && cov >= 0.5 {
        "real"
    } else if safety >= 0.45 {
        "suspect"
    } else {
        "fake"
    };
    (safety, status, reasons)
}

/// Independent material family keys for multi-material hedge per axis.
/// Each family is a set of alternate keys; presence of any key in the set counts as 1 family.
pub fn material_families_for_axis(axis: &str) -> &'static [(&'static str, &'static [&'static str])] {
    match axis {
        "os" => &[
            ("platform_env", &["platform", "os_family", "ua_ch_platform", "timezone"]),
            ("hw_curves", &["hw_curve_webgl", "hw_curve_audio", "hw_curve_cpu", "cpu_timing_curve"]),
            ("gpu_physical", &["gl_precision_matrix", "gpu_wall_staircase", "gpu_slope_wall"]),
            ("census", &["font_bitmap_hash", "font_count", "css_supports_hash", "api_flags_hash"]),
            ("network", &["webrtc_host_ip_hash", "net_rtt", "dns_lookup_ms"]),
            ("stack", &["stack_class", "vm_score", "residual_mean"]),
        ],
        "br" => &[
            ("automation", &["webdriver", "automation", "chrome_runtime", "outer_zero"]),
            ("native", &["native_integrity_ratio", "native_function_toString", "errors_engine", "non_native_count"]),
            ("sandbox_xsrc", &[
                "sandbox_ok",
                "sandbox_sources_received",
                "multi_source_match_ratio",
                "cross_residual_delta",
                "layer_divergence_score",
                "sandbox_capability_score",
                "sandbox_capability_band",
                "sandbox_all_empty",
                "sandbox_under_two_kinds",
                "sandbox_thin_vs_main",
                "js_ok_sandbox_dead",
                "sandbox_blocked",
                "sandbox_ua_mismatch",
                "iframe_ua_mismatch",
                "sandbox_tostring_diverged",
            ]),
            ("canvas_audio", &["canvas_geometry_hash", "canvas_hash", "hw_curve_canvas", "hw_curve_audio", "math_digest"]),
            ("eme_media", &["eme_systems", "media_capabilities"]),
            ("census_br", &["api_flags_hash", "css_supports_hash", "media_query_hash"]),
            ("pohw", &["pohw_triad", "challenge_residual_mean", "challenge_alt_changed"]),
            ("agent_parity", &["agent_parity_hash", "agent_automation_globals_n", "agent_parity_matrix"]),
            ("storage_privacy", &["privacy_storage_score", "storage_quota", "local_storage", "cookie_enabled"]),
            ("dom_perf", &["dom_rect_hash", "perf_timeline_hash"]),
        ],
        "device_id" => &[
            ("hw_curves", &["hw_curve_webgl", "hw_curve_audio", "hw_curve_canvas"]),
            ("gpu_stair", &["gpu_wall_staircase", "gpu_bandwidth_ladder", "gl_precision_matrix", "gpu_r2_wall"]),
            ("pohw", &["pohw_triad", "challenge_residual_mean", "challenge_size_ladder"]),
            ("cpu", &["cpu_timing_curve", "hw_curve_cpu", "challenge_cpu_wall_ms"]),
            ("cross", &["cross_residual_delta", "material_vote_digest", "multi_source_match_ratio"]),
            ("webgpu", &["webgpu_adapter", "webgpu_limits"]),
            ("canvas_math", &["canvas_geometry_hash", "math_digest", "wasm_timing_ms"]),
            ("shader_ulp", &["shader_ulp_max", "mediump_diverges", "precision_matrix"]),
            ("caps_actual", &["actual_max_tex", "caps_claim_vs_actual", "claimed_max_tex"]),
            ("clock_phys", &["perf_now_resolution_ms", "raf_jitter_cv", "date_now_skew_ms"]),
            ("codec_eme", &["eme_systems", "codec_matrix", "canplay_matrix"]),
            ("gpu_ns", &["gpu_ns_staircase", "gpu_ns_median", "gpu_disjoint_rate"]),
            ("bw_h02", &["gpu_readback_ladder", "roundtrip_intercept_ms"]),
            ("cpu_cache", &["cpu_cache_ladder", "cpu_cache_knee_bytes"]),
        ],
        _ => &[
            ("generic", &["platform", "user_agent", "hardware_concurrency"]),
        ],
    }
}

/// Count independent material families present for an axis (hedge breadth).
pub fn count_material_families(axis: &str, fields: &Map<String, Value>) -> (usize, Vec<String>) {
    let mut present = Vec::new();
    for (fam, keys) in material_families_for_axis(axis) {
        if keys.iter().any(|k| has_nonempty(fields, k)) {
            present.push((*fam).to_string());
        }
    }
    (present.len(), present)
}

/// Material-vote channel: ≥2 families → support; 1 family → neutral/capped; 0 → thin.
pub fn material_vote_channel(axis: &str, fields: &Map<String, Value>) -> ChannelOut {
    let (n, fams) = count_material_families(axis, fields);
    let mut reasons = vec![format!("material_families={n}")];
    if !fams.is_empty() {
        reasons.push(format!("families={}", fams.join(",")));
    }
    if n >= 3 {
        ChannelOut {
            id: "ch_material_vote",
            safety: 0.88,
            coverage: (0.45 + 0.12 * n as f64).min(0.95),
            stance: Stance::Support,
            reasons,
        }
    } else if n >= 2 {
        ChannelOut {
            id: "ch_material_vote",
            safety: 0.78,
            coverage: 0.55 + 0.1 * (n as f64 - 2.0),
            stance: Stance::Support,
            reasons,
        }
    } else if n == 1 {
        reasons.push("single_material_family".into());
        ChannelOut {
            id: "ch_material_vote",
            safety: 0.55,
            coverage: 0.35,
            stance: Stance::Neutral,
            reasons,
        }
    } else {
        reasons.push("no_material_families".into());
        ChannelOut {
            id: "ch_material_vote",
            safety: 0.4,
            coverage: 0.1,
            stance: Stance::Neutral,
            reasons,
        }
    }
}

/// Multi-source consistency channel (soft class, not strict field equality).
pub fn multi_source_channel(fields: &Map<String, Value>) -> ChannelOut {
    let mut reasons = Vec::new();
    let ratio = f_field(fields, "multi_source_match_ratio");
    let sandbox_blocked = bool_field(fields, "sandbox_blocked").unwrap_or(false);
    let sandbox_ok = bool_field(fields, "sandbox_ok").unwrap_or(false);
    let has_sources = has_nonempty(fields, "sandbox_sources_received")
        || has_nonempty(fields, "sources_received");
    let cross_match = bool_field(fields, "cross_residual_match");
    let cross_delta = f_field(fields, "cross_residual_delta");
    let material_conflict = bool_field(fields, "material_cross_conflict").unwrap_or(false);

    if material_conflict {
        reasons.push("material_cross_conflict".into());
        return ChannelOut {
            id: "ch_xsrc_multisource",
            safety: 0.22,
            coverage: 0.7,
            stance: Stance::Contradict,
            reasons,
        };
    }
    if sandbox_blocked && !has_sources {
        reasons.push("sandbox_fully_blocked".into());
        return ChannelOut {
            id: "ch_xsrc_multisource",
            safety: 0.25,
            coverage: 0.65,
            stance: Stance::Contradict,
            reasons,
        };
    }
    if let Some(d) = cross_delta {
        if d > 1e-3 && cross_match == Some(false) {
            reasons.push(format!("cross_residual_delta={d}"));
            return ChannelOut {
                id: "ch_xsrc_multisource",
                safety: 0.28,
                coverage: 0.7,
                stance: Stance::Contradict,
                reasons,
            };
        }
    }
    if let Some(r) = ratio {
        if r < 0.5 {
            reasons.push(format!("multi_source_match={r:.2}"));
            return ChannelOut {
                id: "ch_xsrc_multisource",
                safety: 0.3,
                coverage: 0.65,
                stance: Stance::Contradict,
                reasons,
            };
        }
        if r >= 0.85 && (sandbox_ok || has_sources) {
            reasons.push(format!("multi_source_match={r:.2}"));
            return ChannelOut {
                id: "ch_xsrc_multisource",
                safety: 0.86,
                coverage: 0.75,
                stance: Stance::Support,
                reasons,
            };
        }
        if r >= 0.7 {
            reasons.push(format!("multi_source_match={r:.2}"));
            return ChannelOut {
                id: "ch_xsrc_multisource",
                safety: 0.7,
                coverage: 0.55,
                stance: Stance::Support,
                reasons,
            };
        }
    }
    if sandbox_ok || has_sources {
        reasons.push("sandbox_sources_present".into());
        return ChannelOut {
            id: "ch_xsrc_multisource",
            safety: 0.72,
            coverage: 0.5,
            stance: Stance::Support,
            reasons,
        };
    }
    reasons.push("multi_source_partial".into());
    ChannelOut {
        id: "ch_xsrc_multisource",
        safety: 0.5,
        coverage: 0.25,
        stance: Stance::Neutral,
        reasons,
    }
}

/// Build env/authenticity channel from pre-scored safety + stack signals.
pub fn env_channel(
    id: &'static str,
    safety: f64,
    coverage: f64,
    reasons: Vec<String>,
    contradict_env: bool,
) -> ChannelOut {
    let stance = if contradict_env {
        Stance::Contradict
    } else if safety >= 0.55 && coverage >= 0.2 {
        Stance::Support
    } else if safety < 0.4 {
        Stance::Contradict
    } else {
        Stance::Neutral
    };
    ChannelOut {
        id,
        safety: clamp01(safety),
        coverage: clamp01(coverage),
        stance,
        reasons,
    }
}

/// Completeness channel: thin completeness → Neutral/capped, never invents Support alone for real.
pub fn completeness_channel(coverage: f64, material_n: usize) -> ChannelOut {
    let mut reasons = vec![format!("axis_coverage={coverage:.2}"), format!("materials={material_n}")];
    if coverage >= 0.55 && material_n >= 2 {
        ChannelOut {
            id: "ch_completeness",
            safety: 0.8,
            coverage: coverage.min(0.95),
            stance: Stance::Support,
            reasons,
        }
    } else if coverage >= 0.3 {
        reasons.push("completeness_partial".into());
        ChannelOut {
            id: "ch_completeness",
            safety: 0.55 + 0.2 * coverage,
            coverage: coverage.min(0.7),
            stance: Stance::Neutral,
            reasons,
        }
    } else {
        reasons.push("completeness_thin".into());
        ChannelOut {
            id: "ch_completeness",
            safety: 0.4,
            coverage: coverage.max(0.08),
            stance: Stance::Neutral,
            reasons,
        }
    }
}

/// Full multi-channel fuse for a product axis.
///
/// Channels: env materials, material vote, multi-source, completeness (+ optional xsrc conflict).
/// Returns (safety, status, reasons, channel_diag).
pub fn fuse_axis_hedge(
    axis: &str,
    fields: &Map<String, Value>,
    env_safety: f64,
    env_coverage: f64,
    env_reasons: Vec<String>,
    env_contradict: bool,
    source_conflicts: &[String],
    xsrc_conflict: bool,
) -> (f64, &'static str, Vec<String>, Vec<Value>) {
    let (mat_n, mat_fams) = count_material_families(axis, fields);
    let mut channels = vec![
        env_channel("ch_env_materials", env_safety, env_coverage, env_reasons, env_contradict),
        material_vote_channel(axis, fields),
        multi_source_channel(fields),
        completeness_channel(env_coverage, mat_n),
    ];
    if xsrc_conflict || !source_conflicts.is_empty() {
        let mut reasons = source_conflicts.to_vec();
        if xsrc_conflict {
            reasons.push("xsrc_status_conflict".into());
        }
        channels.push(ChannelOut {
            id: "ch_xsrc_claim_obs",
            safety: 0.2,
            coverage: 0.75,
            stance: Stance::Contradict,
            reasons,
        });
    }
    // Hard rule: single material family cannot be real even if env looks clean
    let min_cov = match axis {
        "br" => 0.12,
        "os" => 0.15,
        _ => 0.15,
    };
    let (mut safety, mut status, mut reasons) = fuse_channels(&channels, min_cov);
    if mat_n < 2 && status == "real" {
        status = "suspect";
        safety = safety.min(0.65);
        reasons.push("hedge_require_ge2_material_families".into());
    }
    if mat_n >= 2 {
        reasons.push(format!("hedge_materials_ok={}", mat_fams.join("+")));
    }
    let diag: Vec<Value> = channels
        .iter()
        .map(|c| {
            serde_json::json!({
                "id": c.id,
                "safety": (c.safety * 10000.0).round() / 10000.0,
                "coverage": (c.coverage * 10000.0).round() / 10000.0,
                "stance": match c.stance {
                    Stance::Support => "support",
                    Stance::Neutral => "neutral",
                    Stance::Contradict => "contradict",
                },
                "reasons": c.reasons,
            })
        })
        .collect();
    (safety, status, reasons, diag)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn conflict_cannot_average_to_real() {
        let chs = vec![
            ChannelOut {
                id: "ch_env",
                safety: 0.9,
                coverage: 0.8,
                stance: Stance::Support,
                reasons: vec![],
            },
            ChannelOut {
                id: "ch_xsrc",
                safety: 0.2,
                coverage: 0.8,
                stance: Stance::Contradict,
                reasons: vec!["ua_vs_gateway".into()],
            },
        ];
        let (safety, status, rs) = fuse_channels(&chs, 0.2);
        assert!(safety < 0.5, "conflict must cap safety, got {safety}");
        assert_eq!(status, "suspect");
        assert!(rs.iter().any(|r| r.contains("conflict")));
    }

    #[test]
    fn high_safety_without_conflict_can_be_real() {
        let chs = vec![
            ChannelOut {
                id: "ch_env",
                safety: 0.88,
                coverage: 0.85,
                stance: Stance::Support,
                reasons: vec![],
            },
            ChannelOut {
                id: "ch_xsrc",
                safety: 0.82,
                coverage: 0.75,
                stance: Stance::Support,
                reasons: vec![],
            },
        ];
        let (safety, status, _) = fuse_channels(&chs, 0.2);
        assert!(safety >= 0.72, "high safety without conflict: {safety}");
        assert_eq!(status, "real");
    }

    #[test]
    fn single_channel_cannot_be_real() {
        let chs = vec![ChannelOut {
            id: "ch_only",
            safety: 0.95,
            coverage: 0.9,
            stance: Stance::Support,
            reasons: vec![],
        }];
        let (safety, status, rs) = fuse_channels(&chs, 0.1);
        assert_ne!(status, "real");
        assert!(safety <= 0.65);
        assert!(rs.iter().any(|r| r.contains("single_channel")));
    }

    #[test]
    fn material_vote_needs_two_families() {
        let mut fo = Map::new();
        fo.insert("webdriver".into(), json!(false));
        let (n1, _) = count_material_families("br", &fo);
        assert_eq!(n1, 1);
        let ch1 = material_vote_channel("br", &fo);
        assert_eq!(ch1.stance, Stance::Neutral);

        fo.insert("sandbox_ok".into(), json!(true));
        fo.insert("sandbox_sources_received".into(), json!(["iframe:1"]));
        fo.insert("canvas_geometry_hash".into(), json!("abc"));
        let (n2, fams) = count_material_families("br", &fo);
        assert!(n2 >= 2, "fams={fams:?}");
        let ch2 = material_vote_channel("br", &fo);
        assert_eq!(ch2.stance, Stance::Support);
    }

    #[test]
    fn fuse_axis_hedge_conflict_demotes() {
        let mut fo = Map::new();
        fo.insert("hw_curve_webgl".into(), json!([0.1, 0.2, 0.3, 0.4]));
        fo.insert("font_bitmap_hash".into(), json!("f1"));
        fo.insert("sandbox_ok".into(), json!(true));
        fo.insert("sandbox_sources_received".into(), json!(["iframe:a"]));
        fo.insert("multi_source_match_ratio".into(), json!(0.95));
        let (s_ok, st_ok, _, _) = fuse_axis_hedge(
            "os",
            &fo,
            0.88,
            0.7,
            vec!["clean".into()],
            false,
            &[],
            false,
        );
        assert!(s_ok >= 0.55, "support path: s={s_ok} st={st_ok}");

        fo.insert("material_cross_conflict".into(), json!(true));
        let (s_bad, st_bad, rs, diag) = fuse_axis_hedge(
            "os",
            &fo,
            0.88,
            0.7,
            vec!["clean".into()],
            false,
            &[],
            false,
        );
        assert!(s_bad < 0.5, "conflict demote: {s_bad}");
        assert_ne!(st_bad, "real");
        assert!(rs.iter().any(|r| r.contains("conflict") || r.contains("material")));
        assert!(diag.len() >= 3);
    }

    #[test]
    fn single_material_blocks_real_even_if_env_high() {
        let mut fo = Map::new();
        // only one os family (platform)
        fo.insert("platform".into(), json!("Linux"));
        let (s, st, rs, _) = fuse_axis_hedge(
            "os",
            &fo,
            0.95,
            0.8,
            vec!["looks_good".into()],
            false,
            &[],
            false,
        );
        assert_ne!(st, "real", "single material cannot be real: s={s} rs={rs:?}");
    }
}
