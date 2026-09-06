//! Composite soft association (`composite_association_v1`).
//!
//! Weighted fusion of HW / precision / SW / host / nest / channel features with a
//! veto layer. Outputs continuity bands and soft `likely_same_machine` — never
//! silently force-merges different commercial `wg_` floors into one public `dh_`.

use crate::fp_channel_scores::build_fp_channel_scores;
use crate::link_or_mint::binder_obs_from_fields;
use crate::stack_auth::stack_auth_from_fields;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

pub const COMPOSITE_ASSOCIATION_ALGO: &str = "composite_association_v1";

#[derive(Debug, Clone, Default)]
pub struct AssocFeatures {
    pub tenant_id: Option<String>,
    pub soft_class: bool,
    pub hw_webgl_stable: Option<String>,
    pub hw_audio_stable: Option<String>,
    pub residual_class: Option<String>,
    pub b10x_dual_ok: bool,
    pub agree_001: Option<f64>,
    pub agree_0001: Option<f64>,
    pub agree_00001: Option<f64>,
    pub hw_webgpu_compute_digest: Option<String>,
    pub eu_timing_bucket: Option<String>,
    pub form_class: Option<String>,
    pub cores_class: Option<String>,
    pub architecture: Option<String>,
    pub timezone: Option<String>,
    pub audio_deep_phase: Option<String>,
    pub webrtc_host_ip_hash: Option<String>,
    pub os_instance_hash: Option<String>,
    pub nest_comparable: Option<bool>,
    pub nest_agree_001: Option<bool>,
    pub sandbox_consistency: Option<f64>,
    pub engine_family: Option<String>,
    pub os_family: Option<String>,
}

fn s_field(fo: &Map<String, Value>, k: &str) -> Option<String> {
    fo.get(k)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn f_field(fo: &Map<String, Value>, k: &str) -> Option<f64> {
    fo.get(k)
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
}

fn b_field(fo: &Map<String, Value>, k: &str) -> Option<bool> {
    fo.get(k).and_then(|v| v.as_bool())
}

fn eu_timing_bucket_from(fo: &Map<String, Value>) -> Option<String> {
    let ms = f_field(fo, "eu_timing_mean_ms")
        .or_else(|| f_field(fo, "eu_timing_ms"))
        .or_else(|| {
            fo.get("eu_timing_ms")
                .and_then(|v| v.as_array())
                .and_then(|a| {
                    let nums: Vec<f64> = a
                        .iter()
                        .filter_map(|x| x.as_f64().or_else(|| x.as_i64().map(|i| i as f64)))
                        .collect();
                    if nums.is_empty() {
                        None
                    } else {
                        Some(nums.iter().sum::<f64>() / nums.len() as f64)
                    }
                })
        })?;
    // Coarse ms buckets — second silicon source, not commercial floor.
    let b = if ms < 1.0 {
        "eu_lt1"
    } else if ms < 5.0 {
        "eu_1_5"
    } else if ms < 20.0 {
        "eu_5_20"
    } else if ms < 80.0 {
        "eu_20_80"
    } else {
        "eu_ge80"
    };
    Some(b.into())
}

fn nested_f(fo: &Map<String, Value>, path: &[&str]) -> Option<f64> {
    let mut cur: Option<&Value> = fo.get(path[0]);
    for key in &path[1..] {
        cur = cur.and_then(|v| v.get(*key));
    }
    cur.and_then(|v| v.as_f64())
}

fn precision_from_fields(fo: &Map<String, Value>) -> (Option<f64>, Option<f64>, Option<f64>) {
    // Prefer pair/self multipath summaries already attached on fields/product.
    let p001 = f_field(fo, "point_agree_0p001")
        .or_else(|| nested_f(fo, &["residual_precision", "point_agree_0p001"]))
        .or_else(|| nested_f(fo, &["near_host_cluster", "point_agree_0p001"]));
    let p0001 = f_field(fo, "point_agree_0p0001")
        .or_else(|| nested_f(fo, &["residual_precision", "point_agree_0p0001"]))
        .or_else(|| nested_f(fo, &["near_host_cluster", "point_agree_0p0001"]));
    let p00001 = f_field(fo, "point_agree_0p00001")
        .or_else(|| nested_f(fo, &["residual_precision", "point_agree_0p00001"]))
        .or_else(|| nested_f(fo, &["near_host_cluster", "point_agree_0p00001"]));
    (p001, p0001, p00001)
}

/// Extract association feature groups from a merged fields object.
pub fn extract_assoc_features(fields: &Value) -> AssocFeatures {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let stack = stack_auth_from_fields(fields);
    let binder = binder_obs_from_fields(fields);
    let soft = binder.soft_class || stack.soft_stack;

    let dual = fo
        .get("b10x_dual_ok")
        .and_then(|v| v.as_bool())
        .unwrap_or_else(|| {
            binder.hw_webgl_stable.as_ref().is_some_and(|s| !s.is_empty())
                && binder.hw_audio_stable.as_ref().is_some_and(|s| !s.is_empty())
        });

    let (agree_001, agree_0001, agree_00001) = precision_from_fields(&fo);

    let channel = build_fp_channel_scores(fields, &stack);
    let sandbox = channel
        .pointer("/sandbox_consistency/score")
        .or_else(|| channel.get("sandbox_consistency"))
        .and_then(|v| {
            v.as_f64()
                .or_else(|| v.get("score").and_then(|s| s.as_f64()))
        });

    let nest_comparable = b_field(&fo, "nest_vs_main_comparable").or_else(|| {
        if fo.get("nest_residual_mean").is_some() && fo.get("residual_mean").is_some() {
            Some(true)
        } else {
            None
        }
    });
    let nest_agree = b_field(&fo, "nest_vs_main_agree_0p001");

    let audio_phase = s_field(&fo, "audio_deep_phase")
        .or_else(|| s_field(&fo, "audio_phase_class"))
        .or_else(|| {
            f_field(&fo, "audio_phase")
                .map(|p| format!("ph_{:.3}", (p * 1000.0).round() / 1000.0))
        });

    AssocFeatures {
        tenant_id: binder.tenant_id.clone(),
        soft_class: soft,
        hw_webgl_stable: binder
            .hw_webgl_stable
            .or_else(|| s_field(&fo, "hw_webgl_stable")),
        hw_audio_stable: binder
            .hw_audio_stable
            .or_else(|| s_field(&fo, "hw_audio_stable")),
        residual_class: binder.residual_class.clone(),
        b10x_dual_ok: dual,
        agree_001,
        agree_0001,
        agree_00001,
        hw_webgpu_compute_digest: s_field(&fo, "hw_webgpu_compute_digest"),
        eu_timing_bucket: eu_timing_bucket_from(&fo),
        form_class: s_field(&fo, "form_class"),
        cores_class: binder.cores_class.clone().or_else(|| s_field(&fo, "cores_class")),
        architecture: s_field(&fo, "architecture"),
        timezone: s_field(&fo, "timezone")
            .or_else(|| s_field(&fo, "timezone_class"))
            .or_else(|| {
                f_field(&fo, "timezone_offset")
                    .map(|z| format!("tz_{}", z as i64))
            }),
        audio_deep_phase: audio_phase,
        webrtc_host_ip_hash: binder.webrtc_host_ip_hash.clone(),
        os_instance_hash: binder.os_instance_hash.clone(),
        nest_comparable,
        nest_agree_001: nest_agree,
        sandbox_consistency: sandbox,
        engine_family: s_field(&fo, "engine_family").or(Some(binder.family.clone())),
        os_family: binder.os_family.clone(),
    }
}

#[derive(Debug, Clone)]
struct Weights {
    hw_silicon: f64,
    hw_precision: f64,
    hw_alt: f64,
    sw_stable: f64,
    host: f64,
    nest: f64,
    channel: f64,
    soft_ref_max: f64,
    band_confirmed: f64,
    band_likely: f64,
    band_weak: f64,
    demote_dh_below: f64,
    public_link_min: f64,
}

impl Default for Weights {
    fn default() -> Self {
        Self {
            hw_silicon: 0.28,
            hw_precision: 0.12,
            hw_alt: 0.08,
            sw_stable: 0.16,
            host: 0.18,
            nest: 0.10,
            channel: 0.08,
            soft_ref_max: 0.05,
            band_confirmed: 0.82,
            band_likely: 0.62,
            band_weak: 0.40,
            demote_dh_below: 0.38,
            public_link_min: 0.60,
        }
    }
}

fn load_weights() -> Weights {
    static CACHED: OnceLock<Weights> = OnceLock::new();
    CACHED
        .get_or_init(|| {
            let paths = [
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spec/association_weights_v1.json"),
                PathBuf::from("spec/association_weights_v1.json"),
            ];
            for p in &paths {
                if let Ok(raw) = fs::read_to_string(p) {
                    if let Ok(v) = serde_json::from_str::<Value>(&raw) {
                        return parse_weights(&v);
                    }
                }
            }
            Weights::default()
        })
        .clone()
}

fn parse_weights(v: &Value) -> Weights {
    let mut w = Weights::default();
    let g = |k: &str| {
        v.pointer(&format!("/groups/{k}/weight"))
            .and_then(|x| x.as_f64())
    };
    if let Some(x) = g("HW_silicon") {
        w.hw_silicon = x;
    }
    if let Some(x) = g("HW_precision") {
        w.hw_precision = x;
    }
    if let Some(x) = g("HW_alt") {
        w.hw_alt = x;
    }
    if let Some(x) = g("SW_stable") {
        w.sw_stable = x;
    }
    if let Some(x) = g("Host") {
        w.host = x;
    }
    if let Some(x) = g("Nest") {
        w.nest = x;
    }
    if let Some(x) = g("Channel") {
        w.channel = x;
    }
    if let Some(x) = v
        .pointer("/groups/Soft_ref/max_contribution")
        .and_then(|x| x.as_f64())
    {
        w.soft_ref_max = x;
    }
    if let Some(x) = v.pointer("/bands/confirmed").and_then(|x| x.as_f64()) {
        w.band_confirmed = x;
    }
    if let Some(x) = v.pointer("/bands/likely").and_then(|x| x.as_f64()) {
        w.band_likely = x;
    }
    if let Some(x) = v.pointer("/bands/weak").and_then(|x| x.as_f64()) {
        w.band_weak = x;
    }
    if let Some(x) = v.get("demote_dh_below").and_then(|x| x.as_f64()) {
        w.demote_dh_below = x;
    }
    if let Some(x) = v.get("public_link_min").and_then(|x| x.as_f64()) {
        w.public_link_min = x;
    }
    w
}

fn opt_eq(a: &Option<String>, b: &Option<String>) -> Option<bool> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x == y),
        _ => None,
    }
}

fn group_hw_silicon(a: &AssocFeatures, b: &AssocFeatures) -> (f64, Map<String, Value>) {
    let mut parts = Map::new();
    let mut scores = Vec::new();
    match opt_eq(&a.hw_webgl_stable, &b.hw_webgl_stable) {
        Some(true) => {
            scores.push(1.0);
            parts.insert("hw_webgl_stable".into(), json!("agree"));
        }
        Some(false) => {
            scores.push(0.0);
            parts.insert("hw_webgl_stable".into(), json!("disagree"));
        }
        None => {
            parts.insert("hw_webgl_stable".into(), json!("absent"));
        }
    }
    match opt_eq(&a.hw_audio_stable, &b.hw_audio_stable) {
        Some(true) => {
            scores.push(1.0);
            parts.insert("hw_audio_stable".into(), json!("agree"));
        }
        Some(false) => {
            scores.push(0.15);
            parts.insert("hw_audio_stable".into(), json!("disagree"));
        }
        None => {
            parts.insert("hw_audio_stable".into(), json!("absent"));
        }
    }
    match opt_eq(&a.residual_class, &b.residual_class) {
        Some(true) => {
            scores.push(0.9);
            parts.insert("residual_class".into(), json!("agree"));
        }
        Some(false) => {
            scores.push(0.0);
            parts.insert("residual_class".into(), json!("disagree"));
        }
        None => {
            parts.insert("residual_class".into(), json!("absent"));
        }
    }
    if a.b10x_dual_ok && b.b10x_dual_ok {
        scores.push(0.85);
        parts.insert("b10x_dual_ok".into(), json!("both"));
    } else {
        parts.insert("b10x_dual_ok".into(), json!("partial_or_absent"));
    }
    let s = if scores.is_empty() {
        0.0
    } else {
        scores.iter().sum::<f64>() / scores.len() as f64
    };
    parts.insert("score".into(), json!((s * 1000.0).round() / 1000.0));
    (s, parts)
}

fn group_hw_precision(a: &AssocFeatures, b: &AssocFeatures) -> (f64, Map<String, Value>, bool) {
    // Pair precision: when both sides carry multipath agree ratios, fuse them.
    // Floor conflict: if both have residual_class and they disagree → floor_conflict.
    let floor_conflict = matches!(
        (&a.residual_class, &b.residual_class),
        (Some(x), Some(y)) if x != y && x != "rm_none" && y != "rm_none"
    ) || matches!(
        (&a.hw_webgl_stable, &b.hw_webgl_stable),
        (Some(x), Some(y)) if x != y
    );

    let mut parts = Map::new();
    let avg = |x: Option<f64>, y: Option<f64>| match (x, y) {
        (Some(a), Some(b)) => Some((a + b) * 0.5),
        (Some(a), None) | (None, Some(a)) => Some(a),
        _ => None,
    };
    let r001 = avg(a.agree_001, b.agree_001);
    let r0001 = avg(a.agree_0001, b.agree_0001);
    let r00001 = avg(a.agree_00001, b.agree_00001);
    if let Some(v) = r001 {
        parts.insert("point_agree_0p001".into(), json!((v * 1000.0).round() / 1000.0));
    }
    if let Some(v) = r0001 {
        parts.insert("point_agree_0p0001".into(), json!((v * 1000.0).round() / 1000.0));
    }
    if let Some(v) = r00001 {
        parts.insert("point_agree_0p00001".into(), json!((v * 1000.0).round() / 1000.0));
    }

    // Soft weighted: L0 floor base, L1/L2 boost only when floor not conflicting.
    let mut s = 0.0;
    if let Some(v) = r001 {
        s += v * 0.45;
    }
    if !floor_conflict {
        if let Some(v) = r0001 {
            s += v * 0.30;
        }
        if let Some(v) = r00001 {
            s += v * 0.25;
        }
    } else {
        // Fine precision cannot cover floor conflict — ignore L1/L2 for this group.
        parts.insert("floor_blocks_fine".into(), json!(true));
    }
    if r001.is_none() && r0001.is_none() && r00001.is_none() {
        // No multipath numbers: residual_class agree as weak L0 proxy.
        s = match opt_eq(&a.residual_class, &b.residual_class) {
            Some(true) => 0.55,
            Some(false) => 0.0,
            None => 0.0,
        };
    }
    s = s.clamp(0.0, 1.0);
    parts.insert("score".into(), json!((s * 1000.0).round() / 1000.0));
    parts.insert("floor_conflict".into(), json!(floor_conflict));
    (s, parts, floor_conflict)
}

fn group_hw_alt(a: &AssocFeatures, b: &AssocFeatures) -> (f64, Map<String, Value>) {
    let mut parts = Map::new();
    let mut scores = Vec::new();
    match opt_eq(&a.hw_webgpu_compute_digest, &b.hw_webgpu_compute_digest) {
        Some(true) => {
            scores.push(1.0);
            parts.insert("hw_webgpu_compute_digest".into(), json!("agree"));
        }
        Some(false) => {
            scores.push(0.2);
            parts.insert("hw_webgpu_compute_digest".into(), json!("disagree"));
        }
        None => {
            parts.insert("hw_webgpu_compute_digest".into(), json!("absent"));
        }
    }
    match opt_eq(&a.eu_timing_bucket, &b.eu_timing_bucket) {
        Some(true) => {
            scores.push(0.75);
            parts.insert("eu_timing_bucket".into(), json!("agree"));
        }
        Some(false) => {
            scores.push(0.25);
            parts.insert("eu_timing_bucket".into(), json!("disagree"));
        }
        None => {
            parts.insert("eu_timing_bucket".into(), json!("absent"));
        }
    }
    let s = if scores.is_empty() {
        0.0
    } else {
        scores.iter().sum::<f64>() / scores.len() as f64
    };
    parts.insert("score".into(), json!((s * 1000.0).round() / 1000.0));
    (s, parts)
}

fn group_sw(a: &AssocFeatures, b: &AssocFeatures) -> (f64, Map<String, Value>) {
    let mut parts = Map::new();
    let keys: [(&str, &Option<String>); 5] = [
        ("form_class", &a.form_class),
        ("cores_class", &a.cores_class),
        ("architecture", &a.architecture),
        ("timezone", &a.timezone),
        ("audio_deep_phase", &a.audio_deep_phase),
    ];
    let b_vals = [
        &b.form_class,
        &b.cores_class,
        &b.architecture,
        &b.timezone,
        &b.audio_deep_phase,
    ];
    let mut scores = Vec::new();
    for (i, (name, av)) in keys.iter().enumerate() {
        match opt_eq(av, b_vals[i]) {
            Some(true) => {
                scores.push(1.0);
                parts.insert((*name).into(), json!("agree"));
            }
            Some(false) => {
                scores.push(0.2);
                parts.insert((*name).into(), json!("disagree"));
            }
            None => {
                parts.insert((*name).into(), json!("absent"));
            }
        }
    }
    let s = if scores.is_empty() {
        0.0
    } else {
        scores.iter().sum::<f64>() / scores.len() as f64
    };
    parts.insert("score".into(), json!((s * 1000.0).round() / 1000.0));
    (s, parts)
}

fn group_host(a: &AssocFeatures, b: &AssocFeatures) -> (f64, Map<String, Value>) {
    let mut parts = Map::new();
    // Prefer webrtc host over noisy os_instance.
    let rtc = opt_eq(&a.webrtc_host_ip_hash, &b.webrtc_host_ip_hash);
    let osi = opt_eq(&a.os_instance_hash, &b.os_instance_hash);
    let s: f64 = match (rtc, osi) {
        (Some(true), _) => {
            parts.insert("prefer".into(), json!("webrtc_host_ip_hash"));
            parts.insert("webrtc_host_ip_hash".into(), json!("agree"));
            1.0_f64
        }
        (Some(false), Some(true)) => {
            parts.insert("prefer".into(), json!("os_instance_hash"));
            parts.insert("webrtc_host_ip_hash".into(), json!("disagree"));
            parts.insert("os_instance_hash".into(), json!("agree"));
            0.55_f64
        }
        (Some(false), Some(false)) => {
            parts.insert("webrtc_host_ip_hash".into(), json!("disagree"));
            parts.insert("os_instance_hash".into(), json!("disagree"));
            0.0_f64
        }
        (Some(false), None) => {
            parts.insert("webrtc_host_ip_hash".into(), json!("disagree"));
            0.05_f64
        }
        (None, Some(true)) => {
            parts.insert("prefer".into(), json!("os_instance_hash"));
            parts.insert("os_instance_hash".into(), json!("agree"));
            0.75_f64
        }
        (None, Some(false)) => {
            parts.insert("os_instance_hash".into(), json!("disagree"));
            0.1_f64
        }
        (None, None) => 0.0_f64,
    };
    parts.insert("score".into(), json!((s * 1000.0).round() / 1000.0));
    (s, parts)
}

fn group_nest(a: &AssocFeatures, b: &AssocFeatures) -> (f64, Map<String, Value>) {
    let mut parts = Map::new();
    // Pair: both sides' nest agree flags — and/or self-consistency.
    let score_side = |f: &AssocFeatures| -> Option<f64> {
        match (f.nest_comparable, f.nest_agree_001) {
            (Some(true), Some(true)) => Some(1.0),
            (Some(true), Some(false)) => Some(0.15),
            (Some(false), _) => Some(0.4),
            _ => None,
        }
    };
    let sa = score_side(a);
    let sb = score_side(b);
    let s: f64 = match (sa, sb) {
        (Some(x), Some(y)) => (x + y) * 0.5,
        (Some(x), None) | (None, Some(x)) => x,
        _ => 0.0,
    };
    parts.insert("a".into(), json!(sa));
    parts.insert("b".into(), json!(sb));
    parts.insert("score".into(), json!((s * 1000.0).round() / 1000.0));
    (s, parts)
}

fn group_channel(a: &AssocFeatures, b: &AssocFeatures) -> (f64, Map<String, Value>) {
    let mut parts = Map::new();
    let s = match (a.sandbox_consistency, b.sandbox_consistency) {
        (Some(x), Some(y)) => {
            // High when both consistent and close.
            let mean = (x + y) * 0.5;
            let close = 1.0 - (x - y).abs().min(1.0);
            mean * 0.7 + close * 0.3
        }
        (Some(x), None) | (None, Some(x)) => x,
        _ => 0.0,
    };
    parts.insert("sandbox_consistency".into(), json!((s * 1000.0).round() / 1000.0));
    parts.insert("score".into(), json!((s * 1000.0).round() / 1000.0));
    (s.clamp(0.0, 1.0), parts)
}

fn soft_ref_bonus(a: &AssocFeatures, b: &AssocFeatures, cap: f64) -> (f64, Map<String, Value>) {
    let mut parts = Map::new();
    let mut bonus: f64 = 0.0;
    if opt_eq(&a.engine_family, &b.engine_family) == Some(true) {
        bonus += 0.02;
        parts.insert("engine_family".into(), json!("agree"));
    } else if opt_eq(&a.engine_family, &b.engine_family) == Some(false) {
        parts.insert("engine_family".into(), json!("disagree"));
    }
    if opt_eq(&a.os_family, &b.os_family) == Some(true) {
        bonus += 0.02;
        parts.insert("os_family".into(), json!("agree"));
    }
    (bonus.clamp(0.0, cap), parts)
}

/// Compare two feature sets → composite association result JSON.
pub fn compare_assoc_features(a: &AssocFeatures, b: &AssocFeatures) -> Value {
    let w = load_weights();
    let mut veto_reasons: Vec<String> = Vec::new();

    if let (Some(ta), Some(tb)) = (&a.tenant_id, &b.tenant_id) {
        if ta != tb {
            veto_reasons.push("tenant_mismatch".into());
        }
    }

    let wg_disagree = matches!(
        (&a.hw_webgl_stable, &b.hw_webgl_stable),
        (Some(x), Some(y)) if x != y
    );
    let residual_disagree = matches!(
        (&a.residual_class, &b.residual_class),
        (Some(x), Some(y)) if x != y && x != "rm_none" && y != "rm_none"
    );

    // Soft OS fork veto / hard cap.
    let soft_os_fork = (a.soft_class || b.soft_class)
        && matches!(
            (&a.os_instance_hash, &b.os_instance_hash),
            (Some(x), Some(y)) if x != y
        );
    if soft_os_fork {
        veto_reasons.push("soft_os_fork".into());
    }

    let empty_a = a.hw_webgl_stable.is_none()
        && a.hw_audio_stable.is_none()
        && a.residual_class
            .as_ref()
            .map(|r| r.is_empty() || r == "rm_none")
            .unwrap_or(true);
    let empty_b = b.hw_webgl_stable.is_none()
        && b.hw_audio_stable.is_none()
        && b.residual_class
            .as_ref()
            .map(|r| r.is_empty() || r == "rm_none")
            .unwrap_or(true);
    if empty_a || empty_b {
        veto_reasons.push("empty_anchor".into());
    }

    if wg_disagree || residual_disagree {
        // Not a full veto of soft continuity — blocks public_device_link only.
        veto_reasons.push("commercial_floor_mismatch".into());
    }

    let (hw_s, hw_parts) = group_hw_silicon(a, b);
    let (prec_s, prec_parts, floor_conflict) = group_hw_precision(a, b);
    let (alt_s, alt_parts) = group_hw_alt(a, b);
    let (sw_s, sw_parts) = group_sw(a, b);
    let (host_s, host_parts) = group_host(a, b);
    let (nest_s, nest_parts) = group_nest(a, b);
    let (ch_s, ch_parts) = group_channel(a, b);
    let (soft_b, soft_parts) = soft_ref_bonus(a, b, w.soft_ref_max);

    // Present-group renormalization: only count groups with material.
    let mut weighted = 0.0;
    let mut wsum = 0.0;
    let mut push = |score: f64, weight: f64, present: bool| {
        if present && weight > 0.0 {
            weighted += score * weight;
            wsum += weight;
        }
    };
    push(hw_s, w.hw_silicon, hw_s > 0.0 || wg_disagree || residual_disagree);
    push(
        prec_s,
        w.hw_precision,
        prec_parts.get("point_agree_0p001").is_some()
            || prec_parts.get("floor_conflict").and_then(|v| v.as_bool()) == Some(true)
            || a.residual_class.is_some()
            || b.residual_class.is_some(),
    );
    push(
        alt_s,
        w.hw_alt,
        a.hw_webgpu_compute_digest.is_some()
            || b.hw_webgpu_compute_digest.is_some()
            || a.eu_timing_bucket.is_some()
            || b.eu_timing_bucket.is_some(),
    );
    push(
        sw_s,
        w.sw_stable,
        a.form_class.is_some()
            || b.form_class.is_some()
            || a.cores_class.is_some()
            || b.cores_class.is_some(),
    );
    push(
        host_s,
        w.host,
        a.webrtc_host_ip_hash.is_some()
            || b.webrtc_host_ip_hash.is_some()
            || a.os_instance_hash.is_some()
            || b.os_instance_hash.is_some(),
    );
    push(
        nest_s,
        w.nest,
        a.nest_agree_001.is_some()
            || b.nest_agree_001.is_some()
            || a.nest_comparable.is_some()
            || b.nest_comparable.is_some(),
    );
    push(
        ch_s,
        w.channel,
        a.sandbox_consistency.is_some() || b.sandbox_consistency.is_some(),
    );

    let mut assoc = if wsum > 0.0 { weighted / wsum } else { 0.0 };
    assoc = (assoc + soft_b).clamp(0.0, 1.0);

    let hard_veto = veto_reasons
        .iter()
        .any(|r| r == "tenant_mismatch" || r == "soft_os_fork");
    if hard_veto {
        assoc = assoc.min(0.15);
    }
    if veto_reasons.iter().any(|r| r == "empty_anchor") {
        assoc = assoc.min(0.25);
    }
    // Floor conflict: fine precision already ignored in group; also cap public path.
    if floor_conflict {
        // Keep soft score from host/SW but never treat as confirmed public link.
        assoc = assoc.min(0.72);
    }

    let public_device_link = !hard_veto
        && !wg_disagree
        && !residual_disagree
        && !veto_reasons.iter().any(|r| r == "empty_anchor")
        && assoc >= w.public_link_min
        && host_s >= 0.5
        && hw_s >= 0.5;

    let likely_same_machine = !hard_veto
        && !veto_reasons.iter().any(|r| r == "empty_anchor")
        && host_s >= 0.7
        && sw_s >= 0.55
        && assoc >= w.band_likely
        && (wg_disagree || residual_disagree || public_device_link);

    let band = if hard_veto || veto_reasons.iter().any(|r| r == "tenant_mismatch") {
        "vetoed"
    } else if public_device_link && assoc >= w.band_confirmed {
        "confirmed"
    } else if likely_same_machine || (assoc >= w.band_likely && !wg_disagree) {
        "likely"
    } else if assoc >= w.band_weak {
        "weak"
    } else {
        "distinct"
    };

    let demote_dh = assoc < w.demote_dh_below
        || hard_veto
        || veto_reasons.iter().any(|r| r == "empty_anchor");

    json!({
        "algo": COMPOSITE_ASSOCIATION_ALGO,
        "assoc_score": (assoc * 10000.0).round() / 10000.0,
        "continuity_band": band,
        "public_device_link": public_device_link,
        "likely_same_machine": likely_same_machine,
        "demote_dh_recommended": demote_dh,
        "veto_reasons": veto_reasons,
        "wg_disagree": wg_disagree,
        "residual_disagree": residual_disagree,
        "floor_conflict": floor_conflict,
        "breakdown": {
            "HW_silicon": hw_parts,
            "HW_precision": prec_parts,
            "HW_alt": alt_parts,
            "SW_stable": sw_parts,
            "Host": host_parts,
            "Nest": nest_parts,
            "Channel": ch_parts,
            "Soft_ref": soft_parts,
        },
        "weights_algo": "association_weights_v1",
    })
}

/// Pair association from raw fields.
pub fn composite_associate(fields_a: &Value, fields_b: &Value) -> Value {
    let a = extract_assoc_features(fields_a);
    let b = extract_assoc_features(fields_b);
    compare_assoc_features(&a, &b)
}

/// Single-observation continuity readiness (self nest/channel/dual materials).
/// Used by evaluate for cautious dh→dv demote — not for cross-session link.
pub fn self_association_readiness(fields: &Value) -> Value {
    let f = extract_assoc_features(fields);
    let mut score = 0.0;
    let mut n = 0.0;
    let mut reasons: Vec<String> = Vec::new();

    if f.b10x_dual_ok
        || (f.hw_webgl_stable.is_some() && f.hw_audio_stable.is_some())
    {
        score += 0.35;
        n += 0.35;
        reasons.push("dual_hw".into());
    } else if f.hw_webgl_stable.is_some() || f.hw_audio_stable.is_some() {
        score += 0.15;
        n += 0.35;
        reasons.push("partial_hw".into());
    } else {
        n += 0.35;
        reasons.push("no_hw_anchor".into());
    }

    if f.webrtc_host_ip_hash.is_some() || f.os_instance_hash.is_some() {
        score += 0.25;
        n += 0.25;
        reasons.push("host_separator".into());
    } else {
        n += 0.25;
        reasons.push("no_host_separator".into());
    }

    match (f.nest_comparable, f.nest_agree_001) {
        (Some(true), Some(true)) => {
            score += 0.2;
            n += 0.2;
            reasons.push("nest_agree".into());
        }
        (Some(true), Some(false)) => {
            score += 0.02;
            n += 0.2;
            reasons.push("nest_disagree".into());
        }
        _ => {
            n += 0.2;
        }
    }

    if let Some(sc) = f.sandbox_consistency {
        score += sc * 0.12;
        n += 0.12;
    } else {
        n += 0.12;
    }

    if f.form_class.is_some() {
        score += 0.08;
        n += 0.08;
    } else {
        n += 0.08;
    }

    let assoc = if n > 0.0 { (score / n).clamp(0.0, 1.0) } else { 0.0 };
    let w = load_weights();
    // Dual silicon + host separator already qualify commercial dh via device_tier.
    // Nest residual drift (GPU context lost, multi-browser VRAM pressure, sandbox timing)
    // must NOT force dh→dv alone — that was splitting same-machine blink/gecko into
    // dv_* vs dh_* with identical materials (lab v5.8.81 7-browser wave).
    let dual_hw = f.b10x_dual_ok
        || (f.hw_webgl_stable.is_some() && f.hw_audio_stable.is_some());
    let host_sep = f.webrtc_host_ip_hash.is_some() || f.os_instance_hash.is_some();
    let nest_disagree =
        matches!(f.nest_agree_001, Some(false)) && f.nest_comparable == Some(true);
    // nest_disagree demotes only when silicon or host sep is incomplete / soft class.
    let nest_demote = nest_disagree && !(dual_hw && host_sep && !f.soft_class);
    let demote = assoc < w.demote_dh_below
        || (f.soft_class && f.webrtc_host_ip_hash.is_none() && f.os_instance_hash.is_none())
        || nest_demote;

    let band = if demote && assoc < w.band_weak {
        "weak"
    } else if assoc >= w.band_confirmed {
        "confirmed"
    } else if assoc >= w.band_likely {
        "likely"
    } else if assoc >= w.band_weak {
        "weak"
    } else {
        "distinct"
    };

    json!({
        "algo": COMPOSITE_ASSOCIATION_ALGO,
        "mode": "self_readiness",
        "assoc_score": (assoc * 10000.0).round() / 10000.0,
        "continuity_band": band,
        "demote_dh_recommended": demote,
        "public_device_link": false,
        "likely_same_machine": false,
        "reasons": reasons,
        "nest_disagree": nest_disagree,
        "nest_demote_applied": nest_demote,
        "features": {
            "hw_webgl_stable": f.hw_webgl_stable,
            "hw_audio_stable": f.hw_audio_stable,
            "residual_class": f.residual_class,
            "b10x_dual_ok": f.b10x_dual_ok,
            "webrtc_host": f.webrtc_host_ip_hash.is_some(),
            "os_instance": f.os_instance_hash.is_some(),
            "nest_agree_001": f.nest_agree_001,
            "sandbox_consistency": f.sandbox_consistency,
            "soft_class": f.soft_class,
            "dual_hw": dual_hw,
            "host_sep": host_sep,
        },
        "weights_algo": "association_weights_v1",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base_fields() -> Value {
        json!({
            "form_class": "desktop",
            "hw_webgl_stable": "wg_aaaa1111",
            "hw_audio_stable": "aa_bbbb2222",
            "residual_class": "rm_0.260",
            "residual_mean": 0.26,
            "webrtc_host_ip_hash": "host_same_99",
            "cores_class": "c2",
            "architecture": "x86_64",
            "timezone": "Asia/Shanghai",
            "engine_family": "blink",
            "b10x_dual_ok": true,
            "nest_vs_main_comparable": true,
            "nest_vs_main_agree_0p001": true,
            "nest_residual_mean": 0.26,
            "point_agree_0p001": 0.95,
            "point_agree_0p0001": 0.88,
            "point_agree_0p00001": 0.80,
        })
    }

    #[test]
    fn same_wg_dual_hw_host_confirmed() {
        let a = base_fields();
        let mut b = base_fields();
        b.as_object_mut()
            .unwrap()
            .insert("engine_family".into(), json!("gecko"));
        let out = composite_associate(&a, &b);
        assert_eq!(out["algo"], COMPOSITE_ASSOCIATION_ALGO);
        assert!(out["assoc_score"].as_f64().unwrap() >= 0.82);
        assert_eq!(out["public_device_link"], true);
        assert!(matches!(
            out["continuity_band"].as_str(),
            Some("confirmed") | Some("likely")
        ));
    }

    #[test]
    fn different_wg_same_host_likely_same_machine_no_public_link() {
        let a = base_fields();
        let mut b = base_fields();
        {
            let o = b.as_object_mut().unwrap();
            o.insert("hw_webgl_stable".into(), json!("wg_ffff9999"));
            o.insert("residual_class".into(), json!("rm_0.410"));
            o.insert("engine_family".into(), json!("webkit"));
            o.insert("point_agree_0p001".into(), json!(0.2));
            o.insert("point_agree_0p0001".into(), json!(0.1));
            o.insert("point_agree_0p00001".into(), json!(0.05));
        }
        let out = composite_associate(&a, &b);
        assert_eq!(out["public_device_link"], false);
        assert_eq!(out["wg_disagree"], true);
        assert_eq!(out["likely_same_machine"], true);
        assert!(
            out["continuity_band"].as_str() == Some("likely")
                || out["assoc_score"].as_f64().unwrap() >= 0.55
        );
    }

    #[test]
    fn fine_precision_cannot_override_floor_conflict() {
        let mut a = base_fields();
        let mut b = base_fields();
        {
            let o = a.as_object_mut().unwrap();
            o.insert("hw_webgl_stable".into(), json!("wg_floor_a"));
            o.insert("residual_class".into(), json!("rm_0.100"));
            o.insert("point_agree_0p001".into(), json!(0.1));
            o.insert("point_agree_0p0001".into(), json!(0.99));
            o.insert("point_agree_0p00001".into(), json!(0.99));
            // Different host → not likely_same_machine; focus on floor rule.
            o.insert("webrtc_host_ip_hash".into(), json!("host_a"));
        }
        {
            let o = b.as_object_mut().unwrap();
            o.insert("hw_webgl_stable".into(), json!("wg_floor_b"));
            o.insert("residual_class".into(), json!("rm_0.900"));
            o.insert("point_agree_0p001".into(), json!(0.1));
            o.insert("point_agree_0p0001".into(), json!(0.99));
            o.insert("point_agree_0p00001".into(), json!(0.99));
            o.insert("webrtc_host_ip_hash".into(), json!("host_b"));
        }
        let out = composite_associate(&a, &b);
        assert_eq!(out["public_device_link"], false);
        assert_eq!(out["floor_conflict"], true);
        assert_eq!(
            out["breakdown"]["HW_precision"]["floor_blocks_fine"],
            true
        );
        // Soft score must not reach confirmed solely via 1e-5 agree.
        assert_ne!(out["continuity_band"], "confirmed");
    }

    #[test]
    fn soft_different_os_veto_or_cap() {
        let mut a = base_fields();
        let mut b = base_fields();
        {
            let o = a.as_object_mut().unwrap();
            o.insert("soft_stack".into(), json!(true));
            o.insert("residual_soft_like".into(), json!(true));
            o.insert("os_instance_hash".into(), json!("os_vm_1"));
            o.insert("webrtc_host_ip_hash".into(), Value::Null);
        }
        {
            let o = b.as_object_mut().unwrap();
            o.insert("soft_stack".into(), json!(true));
            o.insert("residual_soft_like".into(), json!(true));
            o.insert("os_instance_hash".into(), json!("os_vm_2"));
            o.insert("webrtc_host_ip_hash".into(), Value::Null);
        }
        let out = composite_associate(&a, &b);
        assert!(
            out["continuity_band"] == "vetoed"
                || out["assoc_score"].as_f64().unwrap() <= 0.25
                || out["public_device_link"] == false
        );
        assert_eq!(out["public_device_link"], false);
        let reasons = out["veto_reasons"].as_array().unwrap();
        assert!(reasons.iter().any(|r| r.as_str() == Some("soft_os_fork")));
    }

    #[test]
    fn self_readiness_extracts_features() {
        let f = base_fields();
        let out = self_association_readiness(&f);
        assert_eq!(out["mode"], "self_readiness");
        assert!(out["assoc_score"].as_f64().unwrap() >= 0.6);
        assert_eq!(out["demote_dh_recommended"], false);
        let feats = extract_assoc_features(&f);
        assert_eq!(feats.hw_webgl_stable.as_deref(), Some("wg_aaaa1111"));
        assert_eq!(feats.nest_agree_001, Some(true));
    }

    /// Lab acceptance shape: Blink+Gecko share floor → public link OK;
    /// WebKit different floor → likely_same_machine, distinct public dh_.
    #[test]
    fn lab_blink_gecko_webkit_assoc_shape() {
        let blink = json!({
            "form_class": "desktop",
            "hw_webgl_stable": "wg_c2feaaaa",
            "hw_audio_stable": "aa_labdual1",
            "residual_class": "rm_0.260",
            "webrtc_host_ip_hash": "host_lab_machine",
            "cores_class": "c2",
            "architecture": "x86_64",
            "timezone": "Asia/Shanghai",
            "engine_family": "blink",
            "b10x_dual_ok": true,
            "nest_vs_main_comparable": true,
            "nest_vs_main_agree_0p001": true,
            "point_agree_0p001": 0.92,
            "point_agree_0p0001": 0.85,
        });
        let mut gecko = blink.clone();
        gecko.as_object_mut().unwrap().insert("engine_family".into(), json!("gecko"));
        let mut webkit = blink.clone();
        {
            let o = webkit.as_object_mut().unwrap();
            o.insert("engine_family".into(), json!("webkit"));
            o.insert("hw_webgl_stable".into(), json!("wg_f029bbbb"));
            o.insert("residual_class".into(), json!("rm_0.410"));
            o.insert("point_agree_0p001".into(), json!(0.25));
        }
        let bg = composite_associate(&blink, &gecko);
        assert_eq!(bg["public_device_link"], true);
        assert!(bg["assoc_score"].as_f64().unwrap() >= 0.75);

        let bw = composite_associate(&blink, &webkit);
        assert_eq!(bw["public_device_link"], false);
        assert_eq!(bw["likely_same_machine"], true);
        assert!(bw["assoc_score"].as_f64().unwrap() >= 0.55);

        let thin = self_association_readiness(&json!({"form_class":"desktop"}));
        assert!(thin["assoc_score"].as_f64().unwrap() < 0.5);
        // Thin must not claim confirmed continuity.
        assert_ne!(thin["continuity_band"], "confirmed");
    }

    /// nest residual disagree alone must NOT demote when dual silicon + host sep present
    /// (same commercial floor across browsers under GPU/nest noise).
    #[test]
    fn nest_disagree_keeps_dh_when_dual_hw_and_host_sep() {
        let f = json!({
            "form_class": "desktop",
            "hw_webgl_stable": "wg_c2fe5b5a7983c515",
            "hw_audio_stable": "cc_1fca5a5d56887fa6",
            "residual_class": "rm_0.260",
            "b10x_dual_ok": true,
            "webrtc_host_ip_hash": "host_lab_1",
            "os_instance_hash": "osi_lab_1",
            "nest_vs_main_comparable": true,
            "nest_vs_main_agree_0p001": false,
            "engine_family": "blink",
        });
        let out = self_association_readiness(&f);
        assert_eq!(out["nest_disagree"], true);
        assert_eq!(out["nest_demote_applied"], false);
        assert_eq!(
            out["demote_dh_recommended"], false,
            "dual_hw+host must keep dh; got {out}"
        );
        assert!(out["assoc_score"].as_f64().unwrap() >= 0.5, "{out}");
    }

    /// nest disagree + missing host sep still demotes (incomplete machine uniqueness).
    #[test]
    fn nest_disagree_demotes_without_host_sep() {
        let f = json!({
            "form_class": "desktop",
            "hw_webgl_stable": "wg_c2fe5b5a7983c515",
            "hw_audio_stable": "cc_1fca5a5d56887fa6",
            "b10x_dual_ok": true,
            "nest_vs_main_comparable": true,
            "nest_vs_main_agree_0p001": false,
        });
        let out = self_association_readiness(&f);
        assert_eq!(out["nest_disagree"], true);
        assert_eq!(out["nest_demote_applied"], true);
        assert_eq!(out["demote_dh_recommended"], true, "{out}");
    }
}
