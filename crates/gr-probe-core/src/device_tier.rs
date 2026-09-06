//! Exclusive device_id: family **dh > dv > dg** × algo-group prefix (exactly one).
//!
//! # SSOT — identity single entry
//!
//! This module is the **only** place that assigns commercial `device_id` strings for
//! product projection (`evaluate_session` → `select_device_tier`). Mint is **per-vtid**
//! materials only (no knowledge of other vtids). SoftEdge never promotes.
//!
//! Public id format (v5.8.105+): `{algo_group}_{body}` where algo_group ∈
//! `dh-1|dh-2|dh-3|dv-1|dv-2|dg-1`. Machine field `device_tier` remains family
//! `dh|dv|dg`. Curve-count maps groups: dh 1/2/≥3 curves → dh-1/2/3; dv 1/≥2 → dv-1/2; dg → dg-1.
//!
//! Segments: **hardware** (curves/residual silicon) vs **software** (OS/network stable
//! soft ids — not raw UA). UA / client IP / JA4 are **auxiliary conf only** — never sole
//! merge keys and never hard-reject of otherwise-valid silicon materials.
//!
//! Confidence from field weight + completeness (not marketing constants).

use crate::stack_auth::stack_auth_from_fields;
use crate::trust::{commercial_projection, cores_class};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

/// Product algo tag for tiered commercial device ids.
pub const DEVICE_TIER_ALGO: &str = "device_tier_v3";

/// Six exclusive public algo-group prefixes (family still in `device_tier`).
pub const DEVICE_ALGO_GROUPS: &[&str] = &["dh-1", "dh-2", "dh-3", "dv-1", "dv-2", "dg-1"];

/// Family of a commercial id.
///
/// - multi-segment `dv0-|dv4-|dv5-|dv6-` → `"multi"`
/// - legacy exclusive `dh_*` / `dv_*` / `dg_*` (and `dh-1_` …) → `"dh"`/`"dv"`/`"dg"`
pub fn commercial_family(id: &str) -> Option<&'static str> {
    if crate::device_segments::is_multi_segment_id(id) {
        Some("multi")
    } else if is_dh_id(id) {
        Some("dh")
    } else if is_dv_id(id) {
        Some("dv")
    } else if is_dg_id(id) {
        Some("dg")
    } else {
        None
    }
}

pub fn is_dh_id(id: &str) -> bool {
    // Multi-segment is not dh family.
    if crate::device_segments::is_multi_segment_id(id) {
        return false;
    }
    id.starts_with("dh-") || id.starts_with("dh_")
}

/// True for multi-segment `dv0-|dv4-|dv5-|dv6-` **and** legacy exclusive `dv_*` / `dv-1_*`.
pub fn is_dv_id(id: &str) -> bool {
    crate::device_segments::is_multi_segment_id(id)
        || id.starts_with("dv-")
        || id.starts_with("dv_")
}

pub fn is_dg_id(id: &str) -> bool {
    if crate::device_segments::is_multi_segment_id(id) {
        return false;
    }
    id.starts_with("dg-") || id.starts_with("dg_")
}

pub fn is_commercial_device_id(id: &str) -> bool {
    commercial_family(id).is_some()
}

/// Strip public prefix → body.
/// Handles multi-segment `dv0-…`, legacy algo groups `dh-1_…`, and `dh_`/`dv_`/`dg_`.
pub fn device_id_body(id: &str) -> &str {
    for p in ["dv0-", "dv4-", "dv5-", "dv6-"] {
        if let Some(b) = id.strip_prefix(p) {
            return b;
        }
    }
    for g in DEVICE_ALGO_GROUPS {
        let p = format!("{g}_");
        if let Some(b) = id.strip_prefix(&p) {
            return b;
        }
    }
    id.strip_prefix("dh_")
        .or_else(|| id.strip_prefix("dv_"))
        .or_else(|| id.strip_prefix("dg_"))
        .unwrap_or(id)
}

/// Parse algo / precision group from id.
pub fn parse_algo_group(id: &str) -> Option<&'static str> {
    for p in crate::device_segments::SEGMENT_PREFIXES {
        if id.starts_with(&format!("{p}-")) || id == *p {
            return Some(*p);
        }
    }
    for g in DEVICE_ALGO_GROUPS {
        if id.starts_with(&format!("{g}_")) || id == *g {
            return Some(*g);
        }
    }
    if id.starts_with("dh-") || id.starts_with("dh_") {
        return Some("dh-1");
    }
    if id.starts_with("dv-") || id.starts_with("dv_") {
        return Some("dv-1");
    }
    if id.starts_with("dg-") || id.starts_with("dg_") {
        return Some("dg-1");
    }
    None
}

pub fn format_algo_device_id(algo_group: &str, body: &str) -> String {
    let body = body.trim_start_matches(|c| c == '_' || c == '-');
    format!("{algo_group}_{body}")
}

/// Count independent hardware curve **families** present at mint-quality thresholds.
/// Families: audio (≥8), webgl (≥4), cpu/timing/canvas (≥8). Residual is conf/assist only
/// (does **not** invent a third family when dual curves already present).
pub fn count_hw_curve_families(fields: &Value, proj: &Value) -> usize {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let mut n = 0usize;
    let audio = curve_len(&fo, "hw_curve_audio") >= 8
        || proj
            .pointer("/materials/hw_audio_stable")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty());
    let webgl = curve_len(&fo, "hw_curve_webgl") >= 4
        || proj
            .pointer("/materials/hw_webgl_stable")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty());
    let cpu = curve_len(&fo, "hw_curve_cpu") >= 8
        || f_has(&fo, "cpu_timing_curve")
        || curve_len(&fo, "hw_curve_canvas") >= 8;
    if audio {
        n += 1;
    }
    if webgl {
        n += 1;
    }
    if cpu {
        n += 1;
    }
    n
}

/// Map completeness family + curve-family count → exclusive algo group.
pub fn algo_group_for_tier(tier: &str, curve_families: usize) -> &'static str {
    match tier {
        "dh" => match curve_families {
            0 | 1 => "dh-1",
            2 => "dh-2",
            _ => "dh-3",
        },
        "dv" => {
            if curve_families >= 2 {
                "dv-2"
            } else {
                "dv-1"
            }
        }
        _ => "dg-1",
    }
}


fn f_has(fo: &Map<String, Value>, key: &str) -> bool {
    match fo.get(key) {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(_) => true,
    }
}

fn curve_len(fo: &Map<String, Value>, key: &str) -> usize {
    fo.get(key)
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .or_else(|| {
            fo.get("hw_noise_curves")
                .and_then(|c| c.get(key.strip_prefix("hw_curve_").unwrap_or(key)))
                .and_then(|v| v.as_array())
                .map(|a| a.len())
        })
        .unwrap_or(0)
}

fn hash_parts(namespace: &str, parts: &[(&str, &str)]) -> String {
    let mut h = Sha256::new();
    h.update(namespace.as_bytes());
    for (k, v) in parts {
        h.update(b"|");
        h.update(k.as_bytes());
        h.update(b"=");
        h.update(v.as_bytes());
    }
    format!("{:x}", h.finalize())
}

/// Dimension scores for tier selection (exposed in tier payload for Ops).
#[derive(Debug, Clone)]
pub struct DimSufficiency {
    pub hw_ok: bool,
    pub soft_ok: bool,
    pub gw_ok: bool,
    pub hw_notes: Vec<String>,
    pub soft_notes: Vec<String>,
    pub gw_notes: Vec<String>,
}

fn dim_hw_ok(fields: &Value, proj: &Value) -> (bool, Vec<String>) {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let stack = stack_auth_from_fields(fields);
    let mut notes = Vec::new();
    let audio = curve_len(&fo, "hw_curve_audio");
    let webgl = curve_len(&fo, "hw_curve_webgl");
    // v5.8.105: single independent HW curve family can satisfy hw dim for **dh-1**.
    // Algo group (not hard reject) encodes 1 / 2 / ≥3 families as dh-1 / dh-2 / dh-3.
    let both = audio >= 8 && webgl >= 4;
    let single_audio = audio >= 8 && webgl < 4;
    let single_webgl = webgl >= 4 && audio < 8;
    let digest = proj
        .get("digest_path")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let residual_present = f_has(&fo, "residual_mean")
        || fo
            .get("residual_std")
            .and_then(|v| v.as_f64())
            .is_some_and(|s| s > 0.0)
        || f_has(&fo, "residual_available");
    // Commercial digests present (post-projection) also count as HW families when curves
    // were already reduced to stable digests without raw arrays on thin evidence paths.
    let dig_audio = proj
        .pointer("/materials/hw_audio_stable")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());
    let dig_webgl = proj
        .pointer("/materials/hw_webgl_stable")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());
    let dual_from_digest = dig_audio && dig_webgl;
    let single_from_digest = (dig_audio || dig_webgl) && !dual_from_digest;
    let dual_hw = both || dual_from_digest;
    let single_hw = (single_audio || single_webgl || single_from_digest) && !dual_hw;
    // real_path: silicon curve material present (1+ family) — not residual-only soft.
    let real_path = digest == "real_curves_v1"
        || digest.starts_with("real_curves")
        || proj
            .get("has_both_curves")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        || dual_hw
        || single_hw
        || (residual_present && (dual_hw || single_hw));
    if stack.soft_stack {
        notes.push("soft_stack".into());
    }
    if stack.gpu_label_untrusted {
        notes.push("gpu_label_untrusted".into());
    }
    if matches!(
        stack.renderer_class.as_str(),
        "swiftshader" | "llvmpipe" | "softpipe" | "soft_other" | "virt_gpu" | "vmware" | "virgl"
    ) {
        notes.push(format!("renderer_class={}", stack.renderer_class));
    }
    if digest.starts_with("soft_") || digest == "soft_aware_v4" || digest == "soft_aware_v4_unit_v1" {
        notes.push("soft_digest_path".into());
    }
    // Clear hardware dimension: real silicon path + curve anchors, not soft-GL.
    // gpu_label_untrusted only means the *string label* is untrusted (stripped from
    // digest) — curves still count as hw when residual is not soft-like.
    let soft_render = stack.soft_stack
        || matches!(
            stack.renderer_class.as_str(),
            "swiftshader" | "llvmpipe" | "softpipe" | "soft_other" | "virt_gpu" | "vmware" | "virgl"
        )
        || digest.starts_with("soft_");
    let clear_silicon = !soft_render;
    // hw_ok: ≥1 mint-quality curve family (single → dh-1; dual/triple via algo group).
    let ok = clear_silicon && (dual_hw || single_hw) && real_path;
    if stack.gpu_label_untrusted {
        notes.push("gpu_label_untrusted".into());
        if ok {
            notes.push("gpu_label_untrusted_curves_still_hw".into());
        }
    }
    if dual_hw {
        notes.push("dual_hw_audio_webgl".into());
    } else if single_hw {
        notes.push("single_hw_curve_family_dh1_ok".into());
    } else {
        notes.push("missing_hw_curve_anchor".into());
    }
    if soft_render {
        notes.push("soft_or_virt_render".into());
    }
    if ok {
        notes.push("hw_sufficient".into());
    }
    (ok, notes)
}

fn dim_soft_ok(fields: &Value, proj: &Value) -> (bool, Vec<String>) {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let mut notes = Vec::new();
    let form = f_has(&fo, "form_class")
        || (f_has(&fo, "screen_width") && f_has(&fo, "platform"));
    let env = f_has(&fo, "platform")
        || f_has(&fo, "os_family")
        || f_has(&fo, "timezone")
        || f_has(&fo, "hardware_concurrency")
        || f_has(&fo, "architecture")
        || proj
            .get("materials")
            .and_then(|m| m.get("cores_class"))
            .is_some()
        || proj
            .get("materials")
            .and_then(|m| m.get("architecture"))
            .is_some();
    // Soft-path related: residual / webrtc / soft separator / device_model are optional boosters
    let soft_related = f_has(&fo, "residual_mean")
        || f_has(&fo, "webrtc_host_ip_hash")
        || proj
            .get("soft_has_host_separator")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        || proj
            .get("device_model_id")
            .and_then(|v| v.as_str())
            .is_some_and(|s| s.starts_with("dm_"))
        || curve_len(&fo, "hw_curve_audio") > 0
        || curve_len(&fo, "hw_curve_webgl") > 0;
    if form {
        notes.push("form_ok".into());
    } else {
        notes.push("missing_form".into());
    }
    if env {
        notes.push("env_ok".into());
    } else {
        notes.push("missing_env".into());
    }
    if soft_related {
        notes.push("soft_related_ok".into());
    }
    // Soft dimension sufficient: form + env (+ any FE material surface)
    let ok = form && env && soft_related;
    if ok {
        notes.push("soft_sufficient".into());
    }
    (ok, notes)
}

fn dim_gw_ok(fields: &Value, evidence: Option<&Value>) -> (bool, Vec<String>) {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let mut notes = Vec::new();
    let gw = evidence
        .and_then(|e| e.get("gateway_fields"))
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let has_ip = f_has(&fo, "server_client_ip")
        || gw.get("server_client_ip").and_then(|v| v.as_str()).is_some_and(|s| !s.is_empty());
    let has_asn = f_has(&fo, "server_asn")
        || gw.get("server_asn").and_then(|v| v.as_str()).is_some_and(|s| !s.is_empty());
    let has_cc = f_has(&fo, "server_country")
        || gw.get("server_country").and_then(|v| v.as_str()).is_some_and(|s| !s.is_empty());
    let has_gw_flag = evidence
        .and_then(|e| e.get("has_gateway"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let has_b8 = evidence
        .and_then(|e| e.get("sources"))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter().any(|s| {
                let t = s.as_str().unwrap_or("");
                t.contains("gateway") || t.contains("cloudflare") || t.contains("B8")
            })
        })
        .unwrap_or(false)
        || evidence
            .and_then(|e| e.get("batches"))
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter().any(|b| {
                    let id = b.get("batch_id").and_then(|x| x.as_str()).unwrap_or("");
                    let src = b.get("source").and_then(|x| x.as_str()).unwrap_or("");
                    id.contains("B8") || src == "gateway" || src == "cloudflare"
                })
            })
            .unwrap_or(false);
    // Sufficient gateway: IP (or B8/has_gateway) — ASN/country boost conf but not required alone
    let ok = has_ip || has_gw_flag || has_b8;
    if has_ip {
        notes.push("server_client_ip".into());
    }
    if has_asn {
        notes.push("server_asn".into());
    }
    if has_cc {
        notes.push("server_country".into());
    }
    if has_b8 || has_gw_flag {
        notes.push("gateway_batch".into());
    }
    if ok {
        notes.push("gw_sufficient".into());
    } else {
        notes.push("missing_gateway".into());
    }
    (ok, notes)
}

pub fn dimension_sufficiency(fields: &Value, proj: &Value, evidence: Option<&Value>) -> DimSufficiency {
    let (hw_ok, hw_notes) = dim_hw_ok(fields, proj);
    let (soft_ok, soft_notes) = dim_soft_ok(fields, proj);
    let (gw_ok, gw_notes) = dim_gw_ok(fields, evidence);
    DimSufficiency {
        hw_ok,
        soft_ok,
        gw_ok,
        hw_notes,
        soft_notes,
        gw_notes,
    }
}

/// dh: multi-dimension sufficient (hw + soft/FE + gateway) — not hardware-only.
pub fn qualifies_dh(fields: &Value, proj: &Value) -> bool {
    // Backward-compatible entry: without evidence, gateway dim cannot pass → not dh.
    qualifies_dh_with_evidence(fields, proj, None)
}

pub fn qualifies_dh_with_evidence(fields: &Value, proj: &Value, evidence: Option<&Value>) -> bool {
    let d = dimension_sufficiency(fields, proj, evidence);
    let eligible = proj
        .get("eligible")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // Commercial machine digest should be eligible when dims are full.
    d.hw_ok && d.soft_ok && d.gw_ok && (eligible || d.hw_ok)
}

/// dv: FE/JS ran with partial materials (not full triple, not gateway-only).
pub fn qualifies_dv(fields: &Value, proj: &Value, client_js_ran: bool) -> bool {
    qualifies_dv_with_evidence(fields, proj, client_js_ran, None)
}

pub fn qualifies_dv_with_evidence(
    fields: &Value,
    proj: &Value,
    client_js_ran: bool,
    evidence: Option<&Value>,
) -> bool {
    if !client_js_ran {
        return false;
    }
    if qualifies_dh_with_evidence(fields, proj, evidence) {
        return false;
    }
    let fo = fields.as_object().cloned().unwrap_or_default();
    let form = f_has(&fo, "form_class")
        || (f_has(&fo, "screen_width") && f_has(&fo, "platform"));
    let soft_sep = proj
        .get("soft_has_host_separator")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let software = f_has(&fo, "platform")
        || f_has(&fo, "timezone")
        || f_has(&fo, "hardware_concurrency")
        || f_has(&fo, "webgl_unmasked_renderer")
        || curve_len(&fo, "hw_curve_audio") > 0
        || curve_len(&fo, "hw_curve_webgl") > 0
        || soft_sep
        || proj
            .get("device_model_id")
            .and_then(|v| v.as_str())
            .is_some_and(|s| s.starts_with("dm_"));
    form && software
}

fn confidence_from_coverage(weight_sum: f64, present: f64, required: f64) -> f64 {
    if required <= 0.0 {
        return 0.0;
    }
    // present/required = completeness; weight_sum = importance mass of materials used
    let cov = (present / required).clamp(0.0, 1.0);
    let w = (weight_sum / required.max(1.0)).clamp(0.0, 1.0);
    // Align with product confidence_algo completeness_x_importance_v1
    ((0.20 + 0.50 * cov + 0.30 * w) * 10000.0).round() / 10000.0
}

/// Weighted hard materials for dh confidence (importance × presence).
/// Required mass is fixed so thinner evidence always lowers conf.
fn dh_material_coverage(fields: &Value, proj: &Value) -> (f64, f64, f64) {
    let fo = fields.as_object().cloned().unwrap_or_default();
    // (weight, present?)
    let mut items: Vec<(f64, bool)> = vec![
        (1.0, f_has(&fo, "form_class")),
        (1.2, curve_len(&fo, "hw_curve_audio") >= 8),
        (1.2, curve_len(&fo, "hw_curve_webgl") >= 4),
        (0.8, curve_len(&fo, "hw_curve_cpu") >= 8 || f_has(&fo, "cpu_timing_curve")),
        (0.7, f_has(&fo, "webgl_unmasked_renderer")),
        (0.5, f_has(&fo, "webgl_extensions_hash") || f_has(&fo, "webgl2_support")),
        (0.5, f_has(&fo, "gl_precision_matrix")),
        (0.4, f_has(&fo, "hardware_concurrency")),
        (0.3, f_has(&fo, "timezone") || f_has(&fo, "os_family")),
        (
            0.4,
            proj.get("has_both_curves")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                || (curve_len(&fo, "hw_curve_audio") >= 8 && curve_len(&fo, "hw_curve_webgl") >= 4),
        ),
    ];
    // Prefer matrix device_id **T0/T1 material** only (F-11 / large matrix).
    // Counting all conf/diag rows dilutes confidence as the matrix grows.
    if let Ok(m) = crate::product_matrix::load_field_product_matrix() {
        for row in &m.fields {
            let Some(role) = row.axes.get("device_id") else {
                continue;
            };
            if role != "material" {
                continue;
            }
            if row.trust_tier != "T0" && row.trust_tier != "T1" {
                continue;
            }
            let w = crate::product_scores::role_importance(role, &row.trust_tier) * 0.35;
            let present = match fo.get(&row.field) {
                None | Some(Value::Null) => false,
                Some(Value::String(s)) => !s.is_empty(),
                Some(Value::Array(a)) => !a.is_empty(),
                Some(Value::Object(o)) => !o.is_empty(),
                Some(_) => true,
            };
            items.push((w, present));
        }
    }
    let required: f64 = items.iter().map(|(w, _)| *w).sum();
    let present_w: f64 = items.iter().filter(|(_, p)| *p).map(|(w, _)| *w).sum();
    let weight_sum = present_w;
    (weight_sum, present_w, required.max(1.0))
}

fn dv_material_coverage(fields: &Value, proj: &Value) -> (f64, f64, f64) {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let soft_sep = proj
        .get("soft_has_host_separator")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let items: Vec<(f64, bool)> = vec![
        (1.0, f_has(&fo, "form_class") || (f_has(&fo, "screen_width") && f_has(&fo, "platform"))),
        (0.9, f_has(&fo, "platform")),
        (0.8, f_has(&fo, "timezone")),
        (0.7, f_has(&fo, "hardware_concurrency")),
        (0.6, f_has(&fo, "os_family")),
        (0.5, f_has(&fo, "webgl_unmasked_renderer")),
        (0.5, curve_len(&fo, "hw_curve_audio") > 0 || curve_len(&fo, "hw_curve_webgl") > 0),
        (0.6, soft_sep || f_has(&fo, "webrtc_host_ip_hash")),
        (
            0.5,
            proj.get("device_model_id")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.starts_with("dm_")),
        ),
        (0.4, f_has(&fo, "device_memory")),
        (0.3, f_has(&fo, "language") || f_has(&fo, "languages")),
    ];
    let required: f64 = items.iter().map(|(w, _)| *w).sum();
    let present_w: f64 = items.iter().filter(|(_, p)| *p).map(|(w, _)| *w).sum();
    (present_w, present_w, required.max(1.0))
}

fn dh_id(fields: &Value, proj: &Value) -> (String, f64) {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let (wsum, present, required) = dh_material_coverage(fields, proj);
    let conf = confidence_from_coverage(wsum, present, required);
    // Prefer existing commercial digest body if present (strip any public prefix).
    if let Some(raw) = proj.get("device_id").and_then(|v| v.as_str()) {
        let body = device_id_body(raw);
        if body.len() >= 12 {
            return (format!("dh_{body}"), conf); // temporary; select rewrites with algo group
        }
    }
    let audio = fo
        .get("hw_curve_audio")
        .map(|v| v.to_string())
        .unwrap_or_default();
    let webgl = fo
        .get("hw_curve_webgl")
        .map(|v| v.to_string())
        .unwrap_or_default();
    let form = fo
        .get("form_class")
        .and_then(|v| v.as_str())
        .unwrap_or("desktop");
    let dig = hash_parts(
        "dh_v1|",
        &[
            ("form", form),
            ("a", &audio[..audio.len().min(256)]),
            ("w", &webgl[..webgl.len().min(256)]),
        ],
    );
    (format!("dh_{}", &dig[..16]), conf)
}

fn dv_id(fields: &Value, proj: &Value) -> (String, f64) {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let (wsum, present, required) = dv_material_coverage(fields, proj);
    let conf = confidence_from_coverage(wsum, present, required);
    // Prefer commercial projection body when eligible (soft or real).
    // Multi-segment product ids (dv0-|dv4-|dv5-|dv6-) and legacy exclusive dv_*.
    if let Some(raw) = proj.get("device_id").and_then(|v| v.as_str()) {
        if is_dv_id(raw) {
            return (raw.to_string(), conf);
        }
    }
    if let Some(dm) = proj.get("device_model_id").and_then(|v| v.as_str()) {
        if let Some(body) = dm.strip_prefix("dm_") {
            return (format!("dv_{body}"), conf);
        }
    }
    let form = fo
        .get("form_class")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let plat = fo
        .get("platform")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let cores = fo
        .get("hardware_concurrency")
        .and_then(|v| v.as_i64())
        .map(cores_class)
        .unwrap_or_else(|| "c0".into());
    let tz = fo
        .get("timezone")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    // Soft residual / coarse webgl / webrtc: multi-component separator so host soft and
    // guest soft do not collapse when residual_mean alone matches (SwiftShader farm).
    // Root-cause: software GL residual is class-level, not host-unique — combine anchors.
    let mats = proj.get("materials").and_then(|v| v.as_object());
    let residual_b = mats
        .and_then(|m| m.get("soft_residual_bucket"))
        .and_then(|v| v.as_str())
        .or_else(|| None)
        .unwrap_or("");
    let webgl_b = mats
        .and_then(|m| m.get("hw_webgl_stable"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let rtc_b = fo
        .get("webrtc_host_ip_hash")
        .and_then(|v| v.as_str())
        .or_else(|| {
            mats.and_then(|m| m.get("webrtc_host_ip_hash"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or("");
    let rm_b = fo
        .get("residual_mean")
        .and_then(|v| v.as_f64())
        .map(|m| format!("rm_{:.5}", (m * 100000.0).round() / 100000.0))
        .unwrap_or_default();
    let arch_b = mats
        .and_then(|m| m.get("architecture"))
        .and_then(|v| v.as_str())
        .or_else(|| fo.get("architecture").and_then(|v| v.as_str()))
        .unwrap_or("");
    // dv_v3 multi-sep: residual|webgl|rtc|rm|arch (empty slots allowed; still better than single)
    let soft_sep = format!("{residual_b}|{webgl_b}|{rtc_b}|{rm_b}|{arch_b}");
    let dig = hash_parts(
        "dv_v3|",
        &[
            ("form", form),
            ("plat", plat),
            ("cores", &cores),
            ("tz", tz),
            ("sep", soft_sep.as_str()),
        ],
    );
    (format!("dv_{}", &dig[..16]), conf)
}

fn dg_id(fields: &Value, evidence: Option<&Value>) -> (String, f64) {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let gw = evidence
        .and_then(|e| e.get("gateway_fields"))
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let ip = fo
        .get("server_client_ip")
        .or_else(|| gw.get("server_client_ip"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let asn = fo
        .get("server_asn")
        .or_else(|| gw.get("server_asn"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let cc = fo
        .get("server_country")
        .or_else(|| gw.get("server_country"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let ua = fo
        .get("user_agent")
        .or_else(|| fo.get("ua"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let dig = hash_parts(
        "dg_v1|",
        &[("ip", ip), ("asn", asn), ("cc", cc), ("ua", ua)],
    );
    // Importance-weighted gateway materials (not equal weights)
    let items = [
        (1.0_f64, !ip.is_empty()),
        (0.8, !asn.is_empty()),
        (0.6, !cc.is_empty()),
        (0.5, !ua.is_empty()),
    ];
    let required: f64 = items.iter().map(|(w, _)| *w).sum();
    let present_w: f64 = items.iter().filter(|(_, p)| *p).map(|(w, _)| *w).sum();
    (
        format!("dg_{}", &dig[..16]),
        confidence_from_coverage(present_w, present_w, required).min(0.55),
    )
}

fn js_ran(evidence: Option<&Value>, fields: &Value) -> bool {
    if let Some(ev) = evidence {
        let sources: Vec<String> = ev
            .get("sources")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let has_main = sources.iter().any(|s| {
            let b = s.split(':').next().unwrap_or(s);
            matches!(b, "main" | "iframe" | "worker" | "sandbox")
        });
        let only_gw = !sources.is_empty()
            && sources.iter().all(|s| {
                let b = s.split(':').next().unwrap_or(s);
                matches!(b, "gateway" | "cloudflare")
            });
        if only_gw {
            return false;
        }
        if has_main {
            return true;
        }
    }
    let fo = fields.as_object().cloned().unwrap_or_default();
    f_has(&fo, "form_class")
        || f_has(&fo, "platform")
        || curve_len(&fo, "hw_curve_audio") > 0
        || f_has(&fo, "webgl_unmasked_renderer")
}

/// Host separator for dh gate: WebRTC host hash or OS instance (iss/43 R6 · phase A).
fn fields_have_host_sep(fields: &Value, proj: &Value) -> bool {
    let fo = fields.as_object();
    let webrtc = fo
        .and_then(|m| {
            m.get("webrtc_host_ip_hash_v2")
                .or_else(|| m.get("webrtc_host_ip_hash"))
        })
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());
    let os_i = fo
        .and_then(|m| m.get("os_instance_hash"))
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());
    let proj_soft = proj
        .get("soft_has_host_separator")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    webrtc || os_i || proj_soft
}

/// Identity keys preferred from main for mint authenticity (aligned with store evidence_merge).
#[allow(dead_code)]
fn is_identity_prefer_main_key(key: &str) -> bool {
    matches!(
        key,
        "user_agent"
            | "platform"
            | "os_family"
            | "language"
            | "languages"
            | "webdriver"
            | "hardware_concurrency"
            | "device_memory"
            | "max_touch_points"
            | "vendor"
            | "timezone"
            | "timezone_offset_min"
            | "form_class"
            | "screen_width"
            | "screen_height"
            | "architecture"
            | "hw_curve_audio"
            | "hw_curve_webgl"
            | "hw_curve_cpu"
            | "webgl_unmasked_renderer"
            | "webgl_unmasked_vendor"
            | "webrtc_host_ip_hash"
            | "os_instance_hash"
            | "font_count"
            | "fonts_present"
            | "plugins_len"
            | "mime_types_len"
            | "chrome_runtime"
            | "product_sub"
            | "app_version"
            | "residual_mean"
            | "residual_std"
            | "residual_algo"
            | "residual_available"
            | "residual_ok"
            | "unit_surface_id"
            | "unit_surface_algo"
            | "unit_multiround_stable"
            | "unit_surface_digest"
            | "engine_family"
            | "probe_profile"
            | "media_input_count"
            | "media_output_count"
            | "media_video_count"
            | "media_device_count"
            | "display_count"
            | "screen_color_depth"
            | "net_type"
            | "net_effective_type"
            | "battery_charging"
            | "gamepad_count"
            | "hw_inventory_algo"
            | "probe_paths"
    ) || key.starts_with("hw_curve_")
        || key.starts_with("hw_audio_")
        || key.starts_with("hw_webgl_")
}

/// Freeze mint materials using **source trust ladder** (not blind main-default).
///
/// Policy (same host/network, multi probe paths):
/// - Soft identity: worker > iframe > main (harder-to-forge nest wins on conflict)
/// - Silicon: multi-agree prefer main fidelity; silicon hard conflict → exclude mint key
/// - Protocol/network: gateway/CF authoritative
/// - Viewport: main preferred (nest iframe not authoritative)
///
/// Product: device_id commercial path must not use nest-overwritten lies as if main.
pub fn authentic_fields_for_device_id(fields: &Value, evidence: Option<&Value>) -> Value {
    let Some(ev) = evidence else {
        return fields.clone();
    };
    let has_fbs = ev
        .get("fields_by_source")
        .and_then(|v| v.as_object())
        .is_some_and(|m| !m.is_empty());
    let resolution = if has_fbs {
        Some(crate::multi_source_mint::resolve_fields_multi_source(
            fields, evidence,
        ))
    } else {
        None
    };
    let mut fo = resolution
        .as_ref()
        .and_then(|r| r.get("resolved_fields").cloned())
        .unwrap_or_else(|| fields.clone());
    let Some(obj) = fo.as_object_mut() else {
        return fo;
    };
    // Capability dead + no main curves → strip residual soft claims that could mint false dv
    let cap_dead = obj
        .get("js_ok_sandbox_dead")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || obj
            .get("sandbox_capability_score")
            .and_then(|v| v.as_f64())
            .is_some_and(|c| c <= 0.05);
    if cap_dead {
        let has_main_curve = ev
            .get("fields_by_source")
            .and_then(|v| v.get("main"))
            .and_then(|v| v.as_object())
            .is_some_and(|m| {
                m.get("hw_curve_audio")
                    .and_then(|v| v.as_array())
                    .is_some_and(|a| !a.is_empty())
                    || m.get("hw_curve_webgl")
                        .and_then(|v| v.as_array())
                        .is_some_and(|a| !a.is_empty())
            });
        if !has_main_curve {
            obj.insert("mint_auth_demote".into(), json!(true));
        }
    }
    if let Some(sum) = resolution
        .as_ref()
        .and_then(|r| r.get("summary").cloned())
    {
        obj.insert("multi_source_resolution_summary".into(), sum);
    }
    fo
}

/// Commercial device projection SSOT.
///
/// **v5.8.106+:** delegates to [`crate::device_segments::select_device_segments`] —
/// multi-precision multi-segment `dv0|dv4|dv5|dv6` (no exclusive dh/dv/dg family).
/// Legacy exclusive-tier helpers remain for internal diagnostics only.
pub fn select_device_tier(fields: &Value, evidence: Option<&Value>) -> Value {
    crate::device_segments::select_device_segments(fields, evidence)
}

#[allow(dead_code)]
fn select_device_tier_legacy_exclusive(fields: &Value, evidence: Option<&Value>) -> Value {
    // Always mint from authentic (main-prefer) view when evidence available
    let auth = authentic_fields_for_device_id(fields, evidence);
    let fields = &auth;
    let proj = commercial_projection(fields);
    let client_js = js_ran(evidence, fields);
    let dims = dimension_sufficiency(fields, &proj, evidence);
    // dh claims multi-dim **machine** identity. When commercial projection flags
    // collision_risk (soft no host-sep, or real dead residual without host sep),
    // or host separator is absent, do not promote to dh — emit dv + collision posture.
    let proj_collision = proj
        .get("collision_risk")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let host_sep = fields_have_host_sep(fields, &proj);
    // Empty/thin projection must not emit stable dv from form-only constants when
    // commercial projection is ineligible and materials are empty.
    let proj_eligible = proj
        .get("eligible")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let proj_id_empty = proj
        .get("device_id")
        .and_then(|v| v.as_str())
        .map(|s| s.is_empty())
        .unwrap_or(true);
    // v5.8.105: host_sep / collision_risk / UA·IP·JA4 are **assist/conf**, not hard demote
    // of multi-dim silicon. Soft never promotes; force-merge across disagreeing floors still forbidden.
    let (id, tier, conf, mut reasons) =
        if client_js && qualifies_dh_with_evidence(fields, &proj, evidence) {
            let (id, conf) = dh_id(fields, &proj);
            let mut r = vec!["tier_dh_multi_dim_sufficient".to_string()];
            if host_sep {
                r.push("tier_dh_host_sep_assist".into());
            } else {
                r.push("tier_dh_no_host_sep_assist_only".into());
            }
            if proj_collision {
                r.push("tier_dh_collision_flag_conf_only".into());
            }
            (id, "dh", conf, r)
        } else if qualifies_dv_with_evidence(fields, &proj, client_js, evidence)
            && (proj_eligible || !proj_id_empty || curve_len(
                &fields.as_object().cloned().unwrap_or_default(),
                "hw_curve_audio",
            ) > 0
                || curve_len(
                    &fields.as_object().cloned().unwrap_or_default(),
                    "hw_curve_webgl",
                ) > 0
                || fields
                    .get("residual_mean")
                    .and_then(|v| v.as_f64())
                    .is_some())
        {
            let (id, conf) = dv_id(fields, &proj);
            let mut r = vec!["tier_dv_partial_dims".to_string()];
            if !dims.hw_ok {
                r.push("dim_hw_insufficient".into());
            }
            if !dims.soft_ok {
                r.push("dim_soft_insufficient".into());
            }
            if !dims.gw_ok {
                r.push("dim_gw_insufficient".into());
            }
            if !host_sep {
                r.push("no_host_sep".into());
            }
            (id, "dv", conf, r)
        } else {
            // Gateway-only OR empty-anchor FE thin → independent dg (IP/ASN/UA), never
            // ServerMint empty body (iss/43 R7 · phase D).
            let (id, conf) = dg_id(fields, evidence);
            let mut r = vec!["tier_dg_gateway_only_or_thin".to_string()];
            if client_js {
                r.push("js_ran_but_no_dh_dv_materials".into());
            } else {
                r.push("no_fe_js_surface".into());
            }
            if !proj_eligible && proj_id_empty {
                r.push("empty_anchor_gateway_id".into());
            }
            (id, "dg", conf, r)
        };
    if dims.hw_ok {
        reasons.push("dim_hw_ok".into());
    }
    if dims.soft_ok {
        reasons.push("dim_soft_ok".into());
    }
    if dims.gw_ok {
        reasons.push("dim_gw_ok".into());
    }
    if host_sep {
        reasons.push("host_sep_present".into());
    } else {
        reasons.push("host_sep_absent".into());
    }
    let curve_n = count_hw_curve_families(fields, &proj);
    let algo_group = algo_group_for_tier(tier, curve_n);
    let body = device_id_body(&id);
    let id = format_algo_device_id(algo_group, body);
    // collision_risk remains diagnostic — does not strip commercial silicon id
    let collision_out = proj_collision;
    // hw vs sw segments (software = OS/network class — not raw UA)
    let fo_seg = fields.as_object().cloned().unwrap_or_default();
    let hw_segment = {
        let mut parts = Vec::new();
        if curve_len(&fo_seg, "hw_curve_audio") > 0 {
            parts.push("audio");
        }
        if curve_len(&fo_seg, "hw_curve_webgl") > 0 {
            parts.push("webgl");
        }
        if curve_len(&fo_seg, "hw_curve_cpu") > 0 || f_has(&fo_seg, "cpu_timing_curve") {
            parts.push("cpu");
        }
        if f_has(&fo_seg, "residual_mean") {
            parts.push("residual");
        }
        parts
    };
    let sw_segment = {
        let mut parts = Vec::new();
        if f_has(&fo_seg, "platform") || f_has(&fo_seg, "os_family") {
            parts.push("os");
        }
        if f_has(&fo_seg, "webrtc_host_ip_hash") || f_has(&fo_seg, "webrtc_host_ip_hash_v2") {
            parts.push("webrtc");
        }
        if f_has(&fo_seg, "os_instance_hash") {
            parts.push("os_instance");
        }
        if f_has(&fo_seg, "timezone") {
            parts.push("timezone");
        }
        if f_has(&fo_seg, "net_type") || f_has(&fo_seg, "net_effective_type") {
            parts.push("network");
        }
        parts
    };
    let aux_conf = {
        let mut parts = Vec::new();
        if f_has(&fo_seg, "user_agent") || f_has(&fo_seg, "ua") {
            parts.push("ua_assist");
        }
        if f_has(&fo_seg, "server_client_ip") {
            parts.push("ip_assist");
        }
        if f_has(&fo_seg, "ja4") || f_has(&fo_seg, "quic_tls_ja4") {
            parts.push("ja4_assist");
        }
        parts
    };
    json!({
        "device_id": id,
        "device_tier": tier,
        "device_algo_group": algo_group,
        "hw_curve_families": curve_n,
        "device_segments": {
            "hardware": hw_segment,
            "software": sw_segment,
            "aux_conf": aux_conf,
        },
        "device_confidence": conf,
        "confidence": conf,
        "algo": DEVICE_TIER_ALGO,
        "tier_reasons": reasons,
        "exclusive": true,
        "client_js_ran": client_js,
        "has_host_separator": host_sep,
        "dimensions": {
            "hw_ok": dims.hw_ok,
            "soft_ok": dims.soft_ok,
            "gw_ok": dims.gw_ok,
            "hw_notes": dims.hw_notes,
            "soft_notes": dims.soft_notes,
            "gw_notes": dims.gw_notes,
        },
        "underlying_projection": {
            "digest_path": proj.get("digest_path"),
            "eligible": proj.get("eligible"),
            "device_model_id": proj.get("device_model_id"),
            "soft_has_host_separator": proj.get("soft_has_host_separator"),
            "collision_risk": proj.get("collision_risk"),
            "analysis_posture": proj.get("analysis_posture"),
            "materials_included": proj.get("materials_included"),
        },
        "collision_risk": collision_out,
        "analysis_posture": proj
            .get("analysis_posture")
            .cloned()
            .unwrap_or(json!([])),
    })
}


#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn curve(n: usize, phase: f64) -> Vec<f64> {
        (0..n)
            .map(|i| ((i as f64 * 0.17 + phase).sin().abs() * 0.4 + 0.05))
            .collect()
    }

    #[test]
    fn select_device_tier_is_multi_segment() {
        let thin = select_device_tier(&json!({}), None);
        let id = thin["device_id"].as_str().unwrap();
        assert!(id.starts_with("dv0-"), "{id}");
        assert_eq!(thin["device_tier"], "multi");
        assert_eq!(thin["algo"], crate::device_segments::DEVICE_SEGMENTS_ALGO);
        let segs = thin["device_id_segments"].as_object().unwrap();
        for p in ["dv0", "dv4", "dv5", "dv6"] {
            assert!(segs.contains_key(p));
        }
    }

    #[test]
    fn rich_silicon_emits_all_lanes() {
        let fields = json!({
            "form_class": "desktop",
            "platform": "Linux x86_64",
            "os_family": "linux",
            "architecture": "x86_64",
            "hardware_concurrency": 12,
            "timezone": "UTC",
            "residual_mean": 0.26008,
            "hw_curve_webgl": curve(8, 0.2),
            "hw_curve_audio": curve(16, 0.3),
            "hw_curve_cpu": curve(12, 0.4),
            "webrtc_host_ip_hash": "rtc_host_1",
            "user_agent": "Mozilla/5.0 MUST-NOT-LEAK",
            "server_client_ip": "203.0.113.9",
        });
        let ev = json!({
            "sources": ["main", "gateway"],
            "has_gateway": true,
            "gateway_fields": {"server_client_ip": "203.0.113.9"},
            "batches": [{"batch_id":"B10_hw_curves","source":"main"}],
        });
        let out = select_device_tier(&fields, Some(&ev));
        let id = out["device_id"].as_str().unwrap();
        assert!(id.starts_with("dv0-"), "{id}");
        assert!(!id.contains("Mozilla"));
        assert!(!id.contains("203.0.113"));
        let segs = out["device_id_segments"].as_object().unwrap();
        assert_ne!(segs["dv0"], segs["dv4"]);
        let parts = id.strip_prefix("dv0-").unwrap().split('-').collect::<Vec<_>>();
        // Silicon lanes (res/wg/au/cp): all present.
        assert!(out["parts_present"].as_u64().unwrap() >= 4, "parts_present={} id={id}", out["parts_present"].as_u64().unwrap());
        for (i, code) in ["res", "wg", "au", "cp"].iter().enumerate() {
            assert_ne!(parts[i], "0", "slot {code} must be present: {id}");
        }
        // Host context (rtc here) is recorded for ops but not in the body.
        assert_eq!(parts[8], "0", "oi must stay placeholder: {id}");
        assert_eq!(parts[9], "0", "rtc must stay placeholder: {id}");
        assert!(out["has_host_separator"].as_bool().unwrap());
    }

    #[test]
    fn confidence_rises_with_materials() {
        let thin = select_device_tier(&json!({}), None);
        let rich = select_device_tier(
            &json!({
                "residual_mean": 0.26,
                "hw_curve_webgl": curve(8, 0.1),
                "hw_curve_audio": curve(16, 0.2),
                "os_family": "linux",
                "architecture": "x86_64",
                "hardware_concurrency": 8,
                "timezone": "UTC",
                "os_instance_hash": "oi_abc",
                "webrtc_host_ip_hash": "rtc_abc",
            }),
            Some(&json!({"sources":["main"]})),
        );
        let ct = thin["device_confidence"].as_f64().unwrap();
        let cr = rich["device_confidence"].as_f64().unwrap();
        assert!(cr > ct, "rich {cr} thin {ct}");
    }
}
