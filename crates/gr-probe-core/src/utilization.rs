//! Field-utilization policy layer (all-fields-utilized).
//!
//! Every FE-emitted field is registered in `spec/field_utilization_policy.json`
//! with a consumption group/channel — the probe stream is never "slimmed";
//! each field participates through one of the group-level consistency
//! checkers below, or through the utilization map visibility registration.
//!
//! This module provides:
//! 1. policy loading + field→group lookup,
//! 2. `merge_policy_into_utilization` — registers every present policy field
//!    into the field-utilization map (visibility: no field is invisible),
//! 3. `run_channel_checks` — group-level consistency checkers producing
//!    confirm / contradict_weak signals for mutual verification
//!    (never hard device-digest roles; `device_digest:false` everywhere),
//! 4. `automation_deep_hits` / `pack_health_marks` aggregates consumed by
//!    brain gaps (probe_pack_incomplete) and rpa/br scoring surfaces.
//!
//! Group → checker mapping (see spec/field_utilization_policy.json groups):
//! silicon_rt / media_codec / env_fingerprint / proto_surface /
//! canvas_typography / automation_deep / math_wasm / residual_lane /
//! fe_integrity / sandbox_deep / verify_deep / rpa_behavior_deep /
//! pack_health / diag.

use crate::contracts::find_spec_dir;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::OnceLock;

const POLICY_FILE: &str = "field_utilization_policy.json";

#[derive(Clone, Debug)]
pub struct FieldPolicy {
    pub group: String,
    pub role: String,
    pub channel: String,
    /// Consumption-depth level (A1 registry v2): L0 raw presence, L1 digest,
    /// L2 fingerprint facet, L3 numeric behavioral, L4 deep numeric, L5 resolved.
    pub level: u8,
    /// Algorithm slot that consumes this field (A1 recipe: field → algorithm).
    pub recipe: String,
}

#[derive(Clone, Debug)]
pub struct GroupMeta {
    pub role: String,
    pub axes: Vec<String>,
    pub channel: String,
}

#[derive(Debug)]
pub struct UtilizationPolicy {
    pub groups: HashMap<String, GroupMeta>,
    pub fields: HashMap<String, FieldPolicy>,
    pub group_order: Vec<String>,
}

fn parse_policy(raw: &Value) -> Result<UtilizationPolicy, String> {
    let mut groups = HashMap::new();
    if let Some(g) = raw.get("groups").and_then(|v| v.as_object()) {
        for (name, meta) in g {
            let axes = meta
                .get("axes")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default();
            groups.insert(
                name.clone(),
                GroupMeta {
                    role: meta
                        .get("role")
                        .and_then(|v| v.as_str())
                        .unwrap_or("diag")
                        .to_string(),
                    axes,
                    channel: meta
                        .get("channel")
                        .and_then(|v| v.as_str())
                        .unwrap_or("diag")
                        .to_string(),
                },
            );
        }
    }
    let mut fields = HashMap::new();
    if let Some(f) = raw.get("fields").and_then(|v| v.as_object()) {
        for (k, meta) in f {
            let group = meta
                .get("group")
                .and_then(|v| v.as_str())
                .unwrap_or("diag_other")
                .to_string();
            let level = match meta
                .get("level")
                .and_then(|v| v.as_str())
                .and_then(|s| s.strip_prefix('L').and_then(|d| d.parse::<u8>().ok()))
                .or_else(|| meta.get("level").and_then(|v| v.as_u64()).map(|u| u as u8))
            {
                Some(l) if l <= 5 => l,
                _ => 1,
            };
            fields.insert(
                k.clone(),
                FieldPolicy {
                    role: meta
                        .get("role")
                        .and_then(|v| v.as_str())
                        .unwrap_or("diag")
                        .to_string(),
                    channel: meta
                        .get("channel")
                        .and_then(|v| v.as_str())
                        .unwrap_or("diag")
                        .to_string(),
                    group,
                    level,
                    recipe: meta
                        .get("recipe")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                },
            );
        }
    }
    let group_order = raw
        .get("group_order")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();
    Ok(UtilizationPolicy {
        groups,
        fields,
        group_order,
    })
}

static POLICY: OnceLock<Result<UtilizationPolicy, String>> = OnceLock::new();

/// Load the field-utilization policy (spec/field_utilization_policy.json).
/// Fail-open only on IO/parse error (analysis continues without the layer).
pub fn utilization_policy() -> &'static Result<UtilizationPolicy, String> {
    POLICY.get_or_init(|| {
        let dir = find_spec_dir();
        let path = dir.join(POLICY_FILE);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("field policy {}: {e}", path.display()))?;
        let raw: Value =
            serde_json::from_str(&text).map_err(|e| format!("field policy parse: {e}"))?;
        parse_policy(&raw)
    })
}

fn policy_fields() -> &'static HashMap<String, FieldPolicy> {
    static EMPTY: std::sync::OnceLock<HashMap<String, FieldPolicy>> = std::sync::OnceLock::new();
    match utilization_policy() {
        Ok(p) => &p.fields,
        Err(_) => EMPTY.get_or_init(|| HashMap::new()),
    }
}

/// Group-level checker registry: which policy groups have active checkers.
pub const CHECKED_GROUPS: [&str; 12] = [
    "silicon_rt",
    "media_codec",
    "env_fingerprint",
    "proto_surface",
    "canvas_typography",
    "automation_deep",
    "math_wasm",
    "residual_lane",
    "fe_integrity",
    "sandbox_deep",
    "verify_deep",
    "rpa_behavior_deep",
];

/// Deep automation hit keys (B1 + R packs). Presence or boolean-true counts.
const AUTOMATION_HIT_KEYS: [&str; 26] = [
    "webdriver_evaluate",
    "webdriver_get",
    "webdriver_script_fn",
    "webdriver_hit",
    "webdriverio",
    "selenium",
    "selenium_hit",
    "selenium_doc",
    "selenium_win",
    "selenium_unwrapped",
    "chromedriver_evaluate",
    "chromedriver_script",
    "cypress",
    "cypress_hit",
    "puppeteer",
    "puppeteer_eval",
    "puppeteer_global",
    "pw_init",
    "pw_manual",
    "phantom",
    "phantom_hit",
    "phantomjs",
    "testcafe",
    "nightmare",
    "fxdriver",
    "dom_automation_controller",
];

/// Count deep automation hits in raw fields (br/rpa consumption).
pub fn automation_deep_hits(fo: &Map<String, Value>) -> u32 {
    let mut hits = 0u32;
    for k in AUTOMATION_HIT_KEYS {
        match fo.get(k) {
            Some(Value::Bool(true)) => hits += 1,
            Some(Value::String(s)) if !s.is_empty() => hits += 1,
            Some(Value::Number(n)) if n.as_f64().unwrap_or(0.0) > 0.0 => hits += 1,
            _ => {}
        }
    }
    hits
}

/// Pack-health marks aggregated by evidence_merge for the session:
/// `pack_failed_n` / `pack_incomplete_n` / `pack_ok_n` / `pack_health`.
pub fn pack_health_marks(fo: &Map<String, Value>) -> (u32, u32) {
    let failed = fo
        .get("pack_failed_n")
        .and_then(|v| v.as_u64().or_else(|| v.as_i64().map(|i| i as u64)))
        .unwrap_or(0) as u32;
    let incomplete = fo
        .get("pack_incomplete_n")
        .and_then(|v| v.as_u64().or_else(|| v.as_i64().map(|i| i as u64)))
        .unwrap_or(0) as u32;
    (failed, incomplete)
}

/// Register every present policy-registered field into the utilization map.
/// `by_axis` is the existing per-axis present/missing structure; policy fields
/// land under their declared axes with role/channel visibility (no drop).
pub fn merge_policy_into_utilization(
    fo: &Map<String, Value>,
    present_rows: &mut Vec<Value>,
    by_axis: &mut Map<String, Value>,
) -> usize {
    let fields = policy_fields();
    let mut registered = 0usize;
    let mut per_channel: Map<String, Value> = Map::new();
    let mut per_level: Map<String, Value> = Map::new();
    let mut per_recipe: Map<String, Value> = Map::new();
    for (k, pol) in fields {
        if let Some(_v) = fo.get(k) {
            registered += 1;
            if let Some(meta) = policy_group_meta(&pol.group) {
                for ax in &meta.axes {
                    if let Some(bucket) = by_axis.get_mut(ax).and_then(|b| b.as_object_mut()) {
                        if let Some(arr) = bucket.get_mut("present").and_then(|a| a.as_array_mut()) {
                            arr.push(json!({
                                "field": k,
                                "role": pol.role,
                                "util": "policy_consistency",
                                "trust_tier": "policy",
                                "channel": pol.channel,
                                "level": format!("L{}", pol.level),
                                "recipe": pol.recipe,
                            }));
                        }
                    }
                }
            }
            let n = per_channel
                .get(&pol.channel)
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            per_channel.insert(
                pol.channel.clone(),
                json!(n + 1),
            );
            let lk = format!("L{}", pol.level);
            let ln = per_level.get(&lk).and_then(|v| v.as_u64()).unwrap_or(0);
            per_level.insert(lk, json!(ln + 1));
            if !pol.recipe.is_empty() {
                let rn = per_recipe
                    .get(&pol.recipe)
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                per_recipe.insert(pol.recipe.clone(), json!(rn + 1));
            }
        }
    }
    present_rows.push(json!({
        "field": "__utilization_policy__",
        "registered_present": registered,
        "by_channel": per_channel,
        "by_level": per_level,
        "by_recipe": per_recipe,
        "note": "FE fields without a product-matrix row are still consumed via consistency channels (field_utilization_policy.json) — nothing is dropped",
    }));
    registered
}

/// A1 registry-v2 presence view: level × recipe consumption of present fields.
/// Levels: L0 raw, L1 digest, L2 fingerprint facet, L3 numeric behavioral,
/// L4 deep numeric, L5 resolved. Recipes name the consuming algorithm slot.
pub fn utilization_level_presence(fo: &Map<String, Value>) -> Value {
    let fields = policy_fields();
    let mut per_level: Map<String, Value> = Map::new();
    let mut recipes: Vec<String> = Vec::new();
    for (k, pol) in fields {
        if fo.contains_key(k) {
            let lk = format!("L{}", pol.level);
            let n = per_level
                .get(&lk)
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            per_level.insert(lk, json!(n + 1));
            if !pol.recipe.is_empty() && !recipes.iter().any(|r| r == &pol.recipe) {
                recipes.push(pol.recipe.clone());
            }
        }
    }
    json!({
        "algo": "utilization_registry_v2",
        "present_by_level": per_level,
        "recipes_present": recipes,
    })
}

fn policy_group_meta(group: &str) -> Option<&'static GroupMeta> {
    match utilization_policy() {
        Ok(p) => p.groups.get(group),
        Err(_) => None,
    }
}

fn bool_any(fo: &Map<String, Value>, keys: &[&str]) -> bool {
    keys.iter().any(|k| match fo.get(*k) {
        Some(Value::Bool(true)) => true,
        Some(Value::String(s)) if !s.is_empty() => true,
        _ => false,
    })
}

fn f64_field(fo: &Map<String, Value>, key: &str) -> Option<f64> {
    fo.get(key)
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)).or_else(|| v.as_u64().map(|u| u as f64)))
}

/// Group-level consistency checkers. Every signal is soft:
/// `contradict_weak` / `confirm` / `confirm_weak`, `device_digest:false`.
pub fn run_channel_checks(fo: &Map<String, Value>) -> (Vec<Value>, Vec<Value>) {
    let mut confirms: Vec<Value> = Vec::new();
    let mut contradicts: Vec<Value> = Vec::new();

    // ── silicon_rt: real-time silicon channels cross-check ──
    {
        let f16_ok = fo.get("webgpu_f16_ok").and_then(|v| v.as_bool()).unwrap_or(false);
        let comp_ok = fo.get("webgpu_compute_ok").and_then(|v| v.as_bool()).unwrap_or(false);
        if f16_ok && comp_ok {
            confirms.push(json!({
                "channel": "silicon_rt",
                "axes": ["device_id", "os"],
                "signal": "webgpu_f16_compute_agree",
                "stance": "confirm",
                "device_digest": false,
                "note": "f16 + compute pipelines both observed — coherent real-time silicon",
            }));
        }
        if fo.get("webgpu_f16_error").is_some()
            && comp_ok
            && fo.get("webgpu_f16_ok").and_then(|v| v.as_bool()) == Some(false)
        {
            contradicts.push(json!({
                "channel": "silicon_rt",
                "axes": ["os"],
                "signal": "webgpu_f16_error_vs_compute_ok",
                "stance": "contradict_weak",
                "device_digest": false,
                "note": "f16 pipeline error while compute pipeline succeeded — inconsistent GPU surface",
            }));
        }
        let thermal_seen = bool_any(fo, &["thermal_cpu_slope_full", "thermal_bursts_full", "thermal_gpu_slope_full"]);
        let thermal_prog = fo.get("thermal_progressive").and_then(|v| v.as_bool()).unwrap_or(false);
        if thermal_seen && (comp_ok || f16_ok) {
            confirms.push(json!({
                "channel": "silicon_rt",
                "axes": ["device_id"],
                "signal": "thermal_observed_with_gpu_silicon",
                "stance": "confirm_weak",
                "device_digest": false,
                "note": "thermal drift channel observed alongside GPU silicon probes",
            }));
        }
        if thermal_prog && fo.get("thermal_cpu_slope_full").is_none() {
            contradicts.push(json!({
                "channel": "silicon_rt",
                "axes": ["os"],
                "signal": "thermal_progressive_without_slope",
                "stance": "contradict_weak",
                "device_digest": false,
                "note": "progressive thermal mode without slope material — truncated probe stream",
            }));
        }
        if fo.get("sab_clock_ok").and_then(|v| v.as_bool()) == Some(true)
            && (fo.get("raf_hz_mean_est").is_some() || fo.get("raf_hz_est").is_some())
        {
            confirms.push(json!({
                "channel": "silicon_rt",
                "axes": ["device_id"],
                "signal": "clock_channels_agree",
                "stance": "confirm_weak",
                "device_digest": false,
                "note": "SAB clock + RAF cadence both observed",
            }));
        }
        if fo.get("sab_clock_ok").and_then(|v| v.as_bool()) == Some(false)
            && (f16_ok || comp_ok)
        {
            contradicts.push(json!({
                "channel": "silicon_rt",
                "axes": ["os"],
                "signal": "sab_clock_failed_while_gpu_silicon_ok",
                "stance": "contradict_weak",
                "device_digest": false,
                "note": "SAB clock unavailable while GPU pipelines succeeded — sandbox/worker surface differs",
            }));
        }
    }

    // ── media_codec: capability coherence ──
    {
        let av1 = fo.get("canplay_av1").and_then(|v| v.as_bool()).unwrap_or(false);
        let webm = fo.get("canplay_webm").and_then(|v| v.as_bool()).unwrap_or(false);
        if av1 && !webm {
            contradicts.push(json!({
                "channel": "media_codec",
                "axes": ["os"],
                "signal": "av1_without_webm_incoherent",
                "stance": "contradict_weak",
                "device_digest": false,
                "note": "AV1 canplay without WebM — unusual capability matrix (patched/proxied media surface)",
            }));
        }
        if bool_any(fo, &["eme_widevine", "eme_clearkey", "eme_systems"]) {
            confirms.push(json!({
                "channel": "media_codec",
                "axes": ["os"],
                "signal": "eme_system_present",
                "stance": "confirm_weak",
                "device_digest": false,
            }));
        }
        if fo.get("codec_virt_hint").and_then(|v| v.as_bool()).unwrap_or(false) {
            contradicts.push(json!({
                "channel": "media_codec",
                "axes": ["os"],
                "signal": "codec_virt_hint",
                "stance": "contradict_weak",
                "device_digest": false,
                "note": "codec matrix hints virtualized environment",
            }));
        }
    }

    // ── env_fingerprint: environment multi-source coherence ──
    {
        let intl = fo.get("intl_locale").and_then(|v| v.as_str()).unwrap_or("");
        let langs = fo
            .get("languages")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        if !intl.is_empty() && !langs.is_empty() {
            let il = intl.get(0..2.min(intl.len())).unwrap_or(intl).to_ascii_lowercase();
            if langs.to_ascii_lowercase().contains(&il) {
                confirms.push(json!({
                    "channel": "env_fingerprint",
                    "axes": ["os"],
                    "signal": "intl_language_matches_languages",
                    "stance": "confirm_weak",
                    "device_digest": false,
                }));
            } else {
                contradicts.push(json!({
                    "channel": "env_fingerprint",
                    "axes": ["os"],
                    "signal": "intl_language_mismatch_languages",
                    "stance": "contradict_weak",
                    "device_digest": false,
                    "note": "Intl primary language not present in navigator.languages",
                }));
            }
        }
        let d_h = f64_field(fo, "window_screen_delta_h");
        let d_w = f64_field(fo, "window_screen_delta_w");
        if let (Some(dw), Some(dh)) = (d_w, d_h) {
            if dw.abs() > 40.0 || dh.abs() > 40.0 {
                let rfp = fo
                    .get("screen_fp_protection_suspect")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if rfp {
                    confirms.push(json!({
                        "channel": "env_fingerprint",
                        "axes": ["os"],
                        "signal": "screen_delta_with_rfp_signature",
                        "stance": "confirm_weak",
                        "device_digest": false,
                        "note": "viewport delta coherent with RFP-style masking (privacy tool, not machine)",
                    }));
                } else {
                    contradicts.push(json!({
                        "channel": "env_fingerprint",
                        "axes": ["os"],
                        "signal": "large_screen_delta_without_rfp",
                        "stance": "contradict_weak",
                        "device_digest": false,
                        "note": "large outer/inner viewport delta without RFP signature",
                    }));
                }
            }
        }
    }

    // ── proto_surface: transport coherence ──
    {
        if fo.get("ws_constructor_native").and_then(|v| v.as_bool()) == Some(false) {
            contradicts.push(json!({
                "channel": "proto_surface",
                "axes": ["br"],
                "signal": "ws_constructor_non_native",
                "stance": "contradict_weak",
                "device_digest": false,
                "note": "WebSocket constructor non-native — shell/patch surface",
            }));
        }
        if fo.get("rtc_can_trickle").and_then(|v| v.as_bool()) == Some(false)
            && bool_any(fo, &["webrtc_host_count", "ice_has_host"])
        {
            contradicts.push(json!({
                "channel": "proto_surface",
                "axes": ["br"],
                "signal": "rtc_trickle_disabled_with_ice",
                "stance": "contradict_weak",
                "device_digest": false,
            }));
        }
        if fo.get("ice_has_srflx").and_then(|v| v.as_bool()).unwrap_or(false)
            && fo.get("ice_has_host").and_then(|v| v.as_bool()) == Some(false)
        {
            confirms.push(json!({
                "channel": "proto_surface",
                "axes": ["os"],
                "signal": "ice_srflx_only_nat_shape",
                "stance": "confirm_weak",
                "device_digest": false,
                "note": "SRFLX-only ICE — NAT'd network surface observed",
            }));
        }
    }

    // ── canvas_typography: canvas geometry / fonts coherence ──
    {
        let has_emoji_w = fo.get("canvas_w_emoji").is_some();
        let has_cjk_w = fo.get("canvas_w_cjk").is_some();
        let has_base_w = fo.get("canvas_w_base").is_some();
        if has_emoji_w && has_cjk_w && has_base_w {
            confirms.push(json!({
                "channel": "canvas_typography",
                "axes": ["device_id"],
                "signal": "typographic_width_matrix",
                "stance": "confirm_weak",
                "device_digest": false,
                "note": "emoji/CJK/base canvas text widths all measured",
            }));
        }
        let cjk_zero = f64_field(fo, "canvas_w_cjk") == Some(0.0);
        if cjk_zero && bool_any(fo, &["font_count", "fonts_present", "font_matrix_hash"]) {
            contradicts.push(json!({
                "channel": "canvas_typography",
                "axes": ["device_id"],
                "signal": "cjk_width_zero_with_fonts",
                "stance": "contradict_weak",
                "device_digest": false,
                "note": "CJK canvas width collapsed while fonts exist — suspicious text renderer",
            }));
        }
        if fo.get("text_width").is_some() && fo.get("dom_rect_subpixel").and_then(|v| v.as_bool()).unwrap_or(false) {
            confirms.push(json!({
                "channel": "canvas_typography",
                "axes": ["device_id"],
                "signal": "subpixel_layout_observed",
                "stance": "confirm_weak",
                "device_digest": false,
            }));
        }
    }

    // ── automation_deep: hit aggregate → br/rpa ──
    {
        let hits = automation_deep_hits(fo);
        if hits > 0 {
            contradicts.push(json!({
                "channel": "automation_deep",
                "axes": ["br", "rpa"],
                "signal": "automation_deep_hits",
                "stance": "contradict_weak",
                "device_digest": false,
                "detail": json!({"hits": hits}),
                "note": "deep automation surface hits detected (webdriver/selenium/cypress/… variants)",
            }));
        }
    }

    // ── math_wasm: JS engine coherence ──
    {
        let wasm_simd = fo.get("wasm_simd").and_then(|v| v.as_bool()).unwrap_or(false)
            || fo.get("wasm_simd_sig").is_some();
        if wasm_simd && fo.get("math_digest").is_some() {
            confirms.push(json!({
                "channel": "math_wasm",
                "axes": ["os"],
                "signal": "wasm_simd_with_math_digest",
                "stance": "confirm",
                "device_digest": false,
                "note": "SIMD wasm + math digest observed — coherent JS engine surface",
            }));
        }
        if fo.get("jit_lowbits_silicon").and_then(|v| v.as_bool()).unwrap_or(false)
            && fo.get("hardwareAcceleration").and_then(|v| v.as_bool()) == Some(false)
        {
            contradicts.push(json!({
                "channel": "math_wasm",
                "axes": ["os"],
                "signal": "jit_silicon_without_hw_accel",
                "stance": "contradict_weak",
                "device_digest": false,
                "note": "JIT lowbit silicon observed while hardware acceleration disabled",
            }));
        }
        if fo.get("webgpu_adapter").is_some() && fo.get("webgpu_adapter_ok").and_then(|v| v.as_bool()).unwrap_or(true) {
            confirms.push(json!({
                "channel": "math_wasm",
                "axes": ["device_id"],
                "signal": "webgpu_adapter_surface",
                "stance": "confirm_weak",
                "device_digest": false,
                "note": "WebGPU adapter descriptor observed — coherent GPU surface",
            }));
        }
    }

    // ── residual_lane: residual multi-lane / seed coherence ──
    {
        let main_r = f64_field(fo, "main_residual_mean");
        let nest_r = f64_field(fo, "nest_residual_mean");
        if let (Some(a), Some(b)) = (main_r, nest_r) {
            if (a - b).abs() < 1e-3 {
                confirms.push(json!({
                    "channel": "residual_lane",
                    "axes": ["device_id"],
                    "signal": "main_nest_residual_agree",
                    "stance": "confirm",
                    "device_digest": false,
                    "note": "main and sandbox residual means agree within 1e-3 (A3)",
                }));
            } else {
                contradicts.push(json!({
                    "channel": "residual_lane",
                    "axes": ["os"],
                    "signal": "main_nest_residual_diverge",
                    "stance": "contradict_weak",
                    "device_digest": false,
                    "note": "main vs sandbox residual means diverge beyond 1e-3",
                }));
            }
        }
        if fo.get("seed_ulp_agree").and_then(|v| v.as_bool()) == Some(false) {
            contradicts.push(json!({
                "channel": "residual_lane",
                "axes": ["device_id"],
                "signal": "seed_ulp_digress",
                "stance": "contradict_weak",
                "device_digest": false,
                "note": "seed ULP replay disagrees across draws",
            }));
        }
        if let (Some(lane), Some(main)) = (
            fo.get("residual_algo_lane_s").and_then(|v| v.as_str()),
            fo.get("residual_algo").and_then(|v| v.as_str()),
        ) {
            if lane == main {
                confirms.push(json!({
                    "channel": "residual_lane",
                    "axes": ["device_id"],
                    "signal": "lane_s_algo_matches_main",
                    "stance": "confirm_weak",
                    "device_digest": false,
                }));
            }
        }
        if bool_any(fo, &["seed_residual_digest", "seed_replay_agree_0p001", "seed_ulp_digest"]) {
            confirms.push(json!({
                "channel": "residual_lane",
                "axes": ["device_id"],
                "signal": "seed_replay_materials",
                "stance": "confirm_weak",
                "device_digest": false,
            }));
        }
    }

    // ── fe_integrity: FE impl/loader/cohort stamp coherence (A5 extension) ──
    {
        let hard = fo.get("fe_hard_impl").and_then(|v| v.as_str()).unwrap_or("");
        let lite = fo.get("fe_lite_impl").and_then(|v| v.as_str()).unwrap_or("");
        if !hard.is_empty() && !lite.is_empty() && hard != lite {
            contradicts.push(json!({
                "channel": "fe_integrity",
                "axes": ["br", "os"],
                "signal": "fe_build_variant_split",
                "stance": "contradict_weak",
                "device_digest": false,
                "note": "FE hard and lite build impl stamps differ — mixed bundle surface",
            }));
        }
        let loader = fo.get("fe_loader_impl").and_then(|v| v.as_str()).unwrap_or("");
        let epoch = fo.get("fe_impl_version").and_then(|v| v.as_str()).unwrap_or("");
        if !loader.is_empty() && !epoch.is_empty() && loader != epoch {
            contradicts.push(json!({
                "channel": "fe_integrity",
                "axes": ["br"],
                "signal": "fe_loader_epoch_drift",
                "stance": "contradict_weak",
                "device_digest": false,
                "note": "FE loader impl stamp disagrees with the FE epoch",
            }));
        }
    }

    // ── sandbox_deep: realm coherence (A3 extension) ──
    {
        for (key, sig) in [
            ("sandbox_hw_mismatch", "sandbox_hw_mismatch"),
            ("sandbox_webdriver_mismatch", "sandbox_webdriver_mismatch"),
            ("sandbox_platform_mismatch", "sandbox_platform_mismatch"),
        ] {
            if fo.get(key).and_then(|v| v.as_bool()).unwrap_or(false) {
                contradicts.push(json!({
                    "channel": "sandbox_deep",
                    "axes": ["device_id", "os"],
                    "signal": sig,
                    "stance": "contradict_weak",
                    "device_digest": false,
                    "note": "sandbox realm diverges from main surface",
                }));
            }
        }
        let nest_eng = fo.get("nest_engine_family").and_then(|v| v.as_str()).unwrap_or("");
        let main_eng = fo
            .get("engine_family")
            .or_else(|| fo.get("residual_probe_engine"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !nest_eng.is_empty() && !main_eng.is_empty() && nest_eng != main_eng {
            contradicts.push(json!({
                "channel": "sandbox_deep",
                "axes": ["os"],
                "signal": "nest_engine_diverges_main",
                "stance": "contradict_weak",
                "device_digest": false,
            }));
        }
    }

    // ── verify_deep: random spotcheck aggregate coherence ──
    {
        if f64_field(fo, "verify_dim_fail").unwrap_or(0.0) > 0.0
            || f64_field(fo, "verify_dim_fail_n").unwrap_or(0.0) > 0.0
        {
            contradicts.push(json!({
                "channel": "verify_deep",
                "axes": ["br"],
                "signal": "verify_dim_failed",
                "stance": "contradict_weak",
                "device_digest": false,
                "note": "random spotcheck dimension failures — thin/scripted probe execution",
            }));
        }
        if let Some(ratio) = f64_field(fo, "verify_b_overlap_ratio") {
            if ratio < 0.5 {
                contradicts.push(json!({
                    "channel": "verify_deep",
                    "axes": ["br"],
                    "signal": "verify_low_overlap",
                    "stance": "contradict_weak",
                    "device_digest": false,
                    "note": "spotcheck pack overlap below 0.5 — inconsistent execution surface",
                }));
            }
        }
        if fo.get("verify_empty_ops").and_then(|v| v.as_bool()).unwrap_or(false) {
            contradicts.push(json!({
                "channel": "verify_deep",
                "axes": ["br"],
                "signal": "verify_empty_ops",
                "stance": "contradict_weak",
                "device_digest": false,
                "note": "spotcheck pack ran zero probes",
            }));
        }
    }

    // ── rpa_behavior_deep: kinematics coherence ──
    {
        if fo.get("rpa_coalesced_stats").is_some() || fo.get("events_with_coalesced").is_some() {
            confirms.push(json!({
                "channel": "rpa_behavior_deep",
                "axes": ["rpa"],
                "signal": "coalesced_pointer_stream",
                "stance": "confirm_weak",
                "device_digest": false,
                "note": "coalesced pointer events observed — real input device surface",
            }));
        }
        let fine = fo.get("pointer_fine").and_then(|v| v.as_bool()).unwrap_or(false)
            || fo.get("css_pointer_fine").and_then(|v| v.as_bool()).unwrap_or(false);
        let touch_heavy = bool_any(fo, &["touch_event", "touch_support", "max_touch_rpa"]);
        if fine && touch_heavy {
            contradicts.push(json!({
                "channel": "rpa_behavior_deep",
                "axes": ["rpa"],
                "signal": "pointer_fine_with_touch_stream",
                "stance": "contradict_weak",
                "device_digest": false,
                "note": "fine-pointer environment reporting touch-event stream",
            }));
        }
        if fo.get("input_modality").and_then(|v| v.as_str()).is_some() {
            confirms.push(json!({
                "channel": "rpa_behavior_deep",
                "axes": ["rpa"],
                "signal": "input_modality_observed",
                "stance": "confirm_weak",
                "device_digest": false,
            }));
        }
    }

    // ── pack_health: consolidated marks (consumed also by brain gaps) ──
    {
        let (failed, incomplete) = pack_health_marks(fo);
        if failed > 0 {
            contradicts.push(json!({
                "channel": "pack_health",
                "axes": ["xsrc"],
                "signal": "pack_health_failed",
                "stance": "contradict_weak",
                "device_digest": false,
                "detail": json!({"failed": failed, "incomplete": incomplete}),
                "note": "probe packs with explicit failure marks",
            }));
        }
        if incomplete > 0 {
            contradicts.push(json!({
                "channel": "pack_health",
                "axes": ["xsrc"],
                "signal": "pack_incomplete",
                "stance": "contradict_weak",
                "device_digest": false,
                "detail": json!({"incomplete": incomplete}),
                "note": "probe packs reported incomplete (skip/error without data)",
            }));
        }
    }

    (confirms, contradicts)
}

/// Present policy-channel summary for diagnostics (counts per channel).
pub fn channel_presence_summary(fo: &Map<String, Value>) -> Value {
    let fields = policy_fields();
    let mut per_channel: Map<String, Value> = Map::new();
    let mut per_level: Map<String, Value> = Map::new();
    for (k, pol) in fields {
        if fo.contains_key(k) {
            let n = per_channel
                .get(&pol.channel)
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            per_channel.insert(pol.channel.clone(), json!(n + 1));
            let lk = format!("L{}", pol.level);
            let ln = per_level.get(&lk).and_then(|v| v.as_u64()).unwrap_or(0);
            per_level.insert(lk, json!(ln + 1));
        }
    }
    json!({
        "policy_present_by_channel": per_channel,
        "policy_present_by_level": per_level,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn policy_ok() -> bool {
        utilization_policy().is_ok()
    }

    #[test]
    fn policy_spec_loads_and_covers_all_orphan_groups() {
        assert!(
            policy_ok(),
            "field_utilization_policy.json must load (run from repo root or set GR_SPEC_DIR): {:?}",
            utilization_policy().as_ref().err()
        );
    }

    #[test]
    fn automation_deep_hits_counts_variants() {
        let fo = json!({
            "webdriver_evaluate": true,
            "selenium_hit": true,
            "cypress": false,
            "phantomjs": "quit",
            "pi": 3
        });
        let fo = fo.as_object().unwrap();
        assert_eq!(automation_deep_hits(fo), 3);
    }

    #[test]
    fn silicon_rt_channels_weak_only() {
        let fo = json!({
            "webgpu_f16_ok": true,
            "webgpu_compute_ok": true,
            "thermal_cpu_slope_full": [1.0],
            "sab_clock_ok": true,
            "raf_hz_mean_est": 60.0,
        });
        let (c, x) = run_channel_checks(fo.as_object().unwrap());
        assert!(c.iter().any(|v| v["channel"] == "silicon_rt"));
        assert!(x.is_empty());
        // All confirms device_digest:false
        for v in &c {
            if v["signal"] == "webgpu_f16_compute_agree" {
                assert_eq!(v["device_digest"], json!(false));
            }
        }
    }

    #[test]
    fn media_codec_incoherent_av1_without_webm() {
        let fo = json!({"canplay_av1": true, "canplay_webm": false});
        let (_, x) = run_channel_checks(fo.as_object().unwrap());
        assert!(
            x.iter().any(|v| v["signal"] == "av1_without_webm_incoherent"),
            "{x:?}"
        );
    }

    #[test]
    fn residual_lane_agree_and_seed_digress() {
        let fo = json!({
            "main_residual_mean": 0.500423,
            "nest_residual_mean": 0.500423,
            "seed_ulp_agree": false,
        });
        let (c, x) = run_channel_checks(fo.as_object().unwrap());
        assert!(c.iter().any(|v| v["signal"] == "main_nest_residual_agree"));
        assert!(x.iter().any(|v| v["signal"] == "seed_ulp_digress"));
    }

    #[test]
    fn fe_integrity_build_variant_split() {
        let fo = json!({"fe_hard_impl": "a", "fe_lite_impl": "b"});
        let (_, x) = run_channel_checks(fo.as_object().unwrap());
        assert!(x.iter().any(|v| v["signal"] == "fe_build_variant_split"));
    }

    #[test]
    fn pack_health_via_marks() {
        let fo = json!({"pack_failed_n": 2, "pack_incomplete_n": 1});
        let (_, x) = run_channel_checks(fo.as_object().unwrap());
        assert!(x.iter().any(|v| v["signal"] == "pack_health_failed"));
        assert!(x.iter().any(|v| v["signal"] == "pack_incomplete"));
    }

    #[test]
    fn rpa_pointer_fine_with_touch_stream() {
        let fo = json!({"pointer_fine": true, "touch_event": true});
        let (_, x) = run_channel_checks(fo.as_object().unwrap());
        assert!(x.iter().any(|v| v["signal"] == "pointer_fine_with_touch_stream"));
    }

    #[test]
    fn utilization_registration_counts_present_policy_fields() {
        let fo = json!({
            "window_screen_delta_h": 12,
            "canvas_w_emoji": 12,
            "canvas_w_cjk": 14,
            "canvas_w_base": 16,
        });
        let mut present = Vec::new();
        let mut by_axis = Map::new();
        for ax in ["device_id", "os", "br", "rpa"] {
            by_axis.insert(ax.into(), json!({"present": [], "missing_material": []}));
        }
        let n = merge_policy_into_utilization(fo.as_object().unwrap(), &mut present, &mut by_axis);
        assert!(n >= 4, "registered present policy fields: {n}");
        let sentinel = present
            .iter()
            .find(|v| v["field"] == "__utilization_policy__")
            .expect("sentinel row");
        assert!(sentinel["registered_present"].as_u64().unwrap() >= 4);
        let device_present = by_axis["device_id"]["present"].as_array().unwrap();
        assert!(
            device_present.iter().any(|v| v["field"] == "canvas_w_emoji"),
            "canvas_typography policy field lands on device_id axis: {device_present:?}"
        );
    }

    #[test]
    fn a1_levels_and_recipes_parsed_from_policy() {
        let Ok(p) = utilization_policy() else {
            panic!("utilization policy must load");
        };
        // B86–B91 fields carry level + recipe (A1 three-layer registry)
        let wasm = p.fields.get("wasm_bi_f32_mul_ns").expect("wasm field");
        assert_eq!(wasm.level, 4, "deep numeric wasm → L4");
        assert_eq!(wasm.recipe, "wasm_throughput_kernel");
        let gecko = p.fields.get("os_kernel_hint").expect("gecko field");
        assert_eq!(gecko.level, 2, "fingerprint facet → L2");
        assert_eq!(gecko.recipe, "os_engine_gecko_surface");
        let storage = p.fields.get("storage_quota_grid_class").expect("storage");
        assert_eq!(storage.level, 3);
        assert_eq!(storage.recipe, "storage_quota");
        // curve/residual slots added with morph/stats recipes
        let wg = p.fields.get("hw_curve_webgl").expect("curve slot");
        assert_eq!(wg.level, 3);
        assert_eq!(wg.recipe, "curve_morph_webgl");
        let res = p.fields.get("residual_mean").expect("residual slot");
        assert_eq!(res.recipe, "residual_stats");
        // legacy fields default to L1 with empty recipe (backward compatible)
        let legacy = p.fields.get("canvas_2d_ctor").expect("legacy field");
        assert_eq!(legacy.level, 1);
        assert_eq!(legacy.recipe, "");
    }

    #[test]
    fn a1_channel_summary_reports_by_level_and_recipe() {
        let fo = json!({
            "wasm_bi_f32_mul_ns": 1.2,
            "wasm_bi_i64_div_ns": 2.1,
            "os_kernel_hint": "6.8",
            "hw_curve_webgl": [0.1, 0.2, 0.3, 0.4],
            "canvas_2d_ctor": true,
            "residual_mean": 0.21,
        });
        let s = channel_presence_summary(fo.as_object().unwrap());
        assert_eq!(s["policy_present_by_level"]["L4"], 2, "{s}");
        assert_eq!(s["policy_present_by_level"]["L2"], 1, "{s}");
        assert_eq!(s["policy_present_by_level"]["L3"], 2, "{s}");
        assert_eq!(s["policy_present_by_level"]["L1"], 1, "{s}");
        let lv = utilization_level_presence(fo.as_object().unwrap());
        let recipes = lv["recipes_present"].as_array().unwrap();
        assert!(
            recipes.contains(&json!("wasm_throughput_kernel"))
                && recipes.contains(&json!("curve_morph_webgl"))
                && recipes.contains(&json!("residual_stats")),
            "recipes: {recipes:?}"
        );
    }

    #[test]
    fn a1_merge_rows_carry_level_and_recipe() {
        let fo = json!({"wasm_bi_f32_mul_ns": 1.2, "os_kernel_hint": "6.8"});
        let mut present = Vec::new();
        let mut by_axis = Map::new();
        for ax in ["device_id", "os", "br", "rpa"] {
            by_axis.insert(ax.into(), json!({"present": [], "missing_material": []}));
        }
        merge_policy_into_utilization(fo.as_object().unwrap(), &mut present, &mut by_axis);
        let sentinel = present
            .iter()
            .find(|v| v["field"] == "__utilization_policy__")
            .expect("sentinel");
        assert_eq!(sentinel["by_level"]["L4"], 1, "{sentinel}");
        assert_eq!(sentinel["by_level"]["L2"], 1, "{sentinel}");
        assert_eq!(sentinel["by_recipe"]["wasm_throughput_kernel"], 1, "{sentinel}");
    }
}
