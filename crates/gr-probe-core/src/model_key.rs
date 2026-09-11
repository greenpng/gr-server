//! Hardware model key extraction (iss/63 K, iss/67 A2).
//!
//! **K** = normalized vendor:model_slug (+ backend) for Atlas cell placement.
//! Never placed into commercial `device_id` body digests.
//!
//! iss/69 U9:
//! - `GR_MODEL_KEY_ATLAS=0` disables attaching model_key extras (rollback switch;
//!   cohort key then falls back to coarse `gl_stack_class`).
//! - `GR_MODEL_KEY_ALIAS_FILE` points to a hot-loadable JSON alias table
//!   (`{"vendor_aliases": {...}, "model_family_aliases": {...}}`). Loaded lazily
//!   once; call `reload_model_key_aliases()` to re-read after config updates.

use serde_json::{json, Map, Value};
use std::fs;
use std::sync::{Arc, Mutex};

/// Algo tag for ops / extras.
pub const MODEL_KEY_ALGO: &str = "hw_model_key_v1";

// ─── Alias table (iss/69 U9) ───────────────────────────────────────────────

#[derive(Clone, Default)]
struct AliasTable {
    /// lowercase needle → canonical vendor
    vendor_aliases: Vec<(String, String)>,
    /// lowercase needle → family slug replacement (applied on model string)
    model_family_aliases: Vec<(String, String)>,
}

static ALIAS: Mutex<Option<Arc<AliasTable>>> = Mutex::new(None);

fn parse_alias_table(js: &str) -> Result<AliasTable, String> {
    let v: Value = serde_json::from_str(js).map_err(|e| format!("alias json: {e}"))?;
    let mut t = AliasTable::default();
    if let Some(o) = v.get("vendor_aliases").and_then(|x| x.as_object()) {
        for (k, val) in o {
            if let Some(c) = val.as_str() {
                if !c.is_empty() {
                    t.vendor_aliases
                        .push((k.to_lowercase(), c.to_string()));
                }
            }
        }
    }
    if let Some(o) = v.get("model_family_aliases").and_then(|x| x.as_object()) {
        for (k, val) in o {
            if let Some(c) = val.as_str() {
                if !c.is_empty() {
                    t.model_family_aliases
                        .push((k.to_lowercase(), c.to_string()));
                }
            }
        }
    }
    Ok(t)
}

/// Load alias table from a JSON string (also used by tests; no env needed).
pub fn load_model_key_aliases_from_str(js: &str) -> Result<usize, String> {
    let t = parse_alias_table(js)?;
    let n = t.vendor_aliases.len() + t.model_family_aliases.len();
    let mut g = ALIAS.lock().unwrap_or_else(|e| e.into_inner());
    *g = Some(Arc::new(t));
    Ok(n)
}

/// Re-read alias table from `GR_MODEL_KEY_ALIAS_FILE` (hot reload for ops/config).
/// Returns number of rules loaded; 0 when file missing/empty (clears overrides).
pub fn reload_model_key_aliases() -> usize {
    let path = gr_abi::env::get("MODEL_KEY_ALIAS_FILE").unwrap_or_default();
    if path.trim().is_empty() {
        if let Ok(mut g) = ALIAS.lock() {
            *g = None;
        }
        return 0;
    }
    match fs::read_to_string(&path) {
        Ok(txt) => match load_model_key_aliases_from_str(&txt) {
            Ok(n) => n,
            Err(e) => {
                // keep prior table on bad file — never regress parsing to empty
                eprintln!("model_key alias reload failed ({e}); keeping prior table");
                ALIAS
                    .lock()
                    .map(|g| {
                        g.as_ref()
                            .map(|t| t.vendor_aliases.len() + t.model_family_aliases.len())
                            .unwrap_or(0)
                    })
                    .unwrap_or(0)
            }
        },
        Err(_) => {
            if let Ok(mut g) = ALIAS.lock() {
                *g = None;
            }
            0
        }
    }
}

fn alias_table() -> Option<Arc<AliasTable>> {
    {
        let g = ALIAS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = g.as_ref() {
            return Some(t.clone());
        }
    }
    let path = gr_abi::env::get("MODEL_KEY_ALIAS_FILE").unwrap_or_default();
    if path.trim().is_empty() {
        return None;
    }
    reload_model_key_aliases();
    ALIAS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Apply alias table overrides on (vendor, model). Vendor matched on full lowered
/// renderer string; model family alias replaces the first occurrence in lowered model.
fn apply_aliases(vendor: &str, model: &str, lower: &str, t: &AliasTable) -> (String, String) {
    let mut v = vendor.to_string();
    for (needle, canonical) in &t.vendor_aliases {
        if lower.contains(needle) {
            v = canonical.clone();
            break;
        }
    }
    // Apply family aliases on a lowercase working copy so case-insensitive needles
    // always match (ASCII model tokens); rebuild string from lowercase after replace.
    let mut m = model.to_lowercase();
    for (needle, repl) in &t.model_family_aliases {
        if let Some(i) = m.find(needle) {
            m.replace_range(i..i + needle.len(), repl);
            break;
        }
    }
    (v, m)
}

/// iss/69 U9: attach rollback switch. `0|false|off` → model_key extras are NOT
/// attached (mint body unchanged; cohort falls back to coarse stack class).
pub fn model_key_attach_enabled() -> bool {
    match gr_abi::env::get("MODEL_KEY_ATLAS") {
        Some(v) => {
            let l = v.to_ascii_lowercase();
            l != "0" && l != "false" && l != "off"
        }
        None => true,
    }
}

/// Debug helper: current alias rule counts (for ops/config_snapshot).
pub fn model_key_alias_stats() -> Value {
    let t = alias_table();
    json!({
        "algo": "model_key_alias_stats_v1",
        "vendor_rules": t.as_ref().map(|t| t.vendor_aliases.len()).unwrap_or(0),
        "model_family_rules": t.as_ref().map(|t| t.model_family_aliases.len()).unwrap_or(0),
        "attach_enabled": model_key_attach_enabled(),
        "alias_file": gr_abi::env::get("MODEL_KEY_ALIAS_FILE").unwrap_or_default(),
    })
}

/// True when renderer string is a stack/mask label, not a GPU model (iss/63 K hygiene).
/// Production bug class: gl_governor / privacy used to feed "WebKit WebGL" as UNMASKED.
pub fn is_generic_renderer_label(raw: &str) -> bool {
    let l = raw.trim().to_lowercase();
    if l.is_empty() {
        return true;
    }
    matches!(
        l.as_str(),
        "webkit webgl"
            | "webkit"
            | "mozilla"
            | "firefox"
            | "chrome webgl"
            | "chrome"
            | "safari"
            | "edge"
            | "opera"
            | "brave"
            | "chromium"
            | "internet explorer"
            | "microsoft edge"
    ) || l == "apple gpu" // too coarse alone; needs further disambiguation
        || (l.starts_with("webkit") && l.contains("webgl") && !l.contains("angle") && !l.contains("metal"))
        || l == "graphics" || l == "gpu"
}

/// Parse ANGLE / raw WebGL unmasked renderer into vendor + model_slug + backend.
pub fn parse_unmasked_renderer(raw: &str) -> Value {
    let s = raw.trim();
    if s.is_empty() {
        return json!({
            "ok": false,
            "error": "empty_renderer",
            "algo": MODEL_KEY_ALGO,
        });
    }
    let lower = s.to_lowercase();
    // Soft GL / emulators
    if lower.contains("swiftshader")
        || lower.contains("llvmpipe")
        || lower.contains("softpipe")
        || lower.contains("microsoft basic render")
    {
        return json!({
            "ok": true,
            "algo": MODEL_KEY_ALGO,
            "hw_model_key": "soft:software_gl",
            "vendor": "soft",
            "model_slug": "software_gl",
            "gl_backend": backend_of(&lower),
            "key_source": "renderer_soft",
            "key_confidence": 0.95,
            "raw_sanitized": sanitize_preview(s),
        });
    }

    // Generic stack labels are NOT model keys (iss/63 + 178 prod: WebKit WebGL flood).
    if is_generic_renderer_label(s) {
        return json!({
            "ok": false,
            "error": "generic_renderer_label",
            "algo": MODEL_KEY_ALGO,
            "raw_sanitized": sanitize_preview(s),
            "key_source": "rejected_generic",
            "key_confidence": 0.0,
        });
    }

    // Privacy-masked / farbling renderer strings (Firefox rFP, Brave farble):
    // "NVIDIA GeForce GTX 980, or similar" is NOT a real named SKU — refuse named K
    // so model_key_from_fields falls through to class: caps (cross-browser stable).
    // Normalize whitespace / unicode spaces so matching is robust.
    let lower_compact: String = lower
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if lower_compact.contains("or similar")
        || lower_compact.contains("or_similar")
        || lower_compact.contains(", similar")
        || lower_compact.contains("generic renderer")
        || lower_compact.contains("redacted")
        || lower_compact.contains("similar device")
    {
        return json!({
            "ok": false,
            "error": "privacy_masked_renderer",
            "algo": MODEL_KEY_ALGO,
            "raw_sanitized": sanitize_preview(s),
            "key_source": "rejected_privacy_mask",
            "key_confidence": 0.0,
            "note": "engine privacy scrub — use class caps K",
        });
    }

    let mut vendor: String = vendor_token(&lower).to_string();
    let mut model = s.to_string();
    let mut backend = backend_of(&lower);

    // ANGLE (Vendor, Device, Backend) — also tolerate missing trailing ')'
    let angle_inner = s
        .strip_prefix("ANGLE (")
        .map(|x| x.strip_suffix(')').unwrap_or(x));
    if let Some(inner) = angle_inner {
        let parts: Vec<&str> = inner.split(',').map(|p| p.trim()).collect();
        if !parts.is_empty() {
            vendor = vendor_token(parts[0]).to_string();
        }
        if parts.len() >= 2 {
            model = parts[1].to_string();
            // Often "NVIDIA GeForce RTX 3060 Direct3D11 ..."
            model = strip_trailing_api(&model);
        }
        if parts.len() >= 3 {
            backend = backend_of(&parts[2].to_lowercase());
        }
        // Google ANGLE + OpenGL ES adapter strings
        if vendor == "google" && parts.len() >= 2 {
            let mlow = model.to_lowercase();
            if mlow.contains("nvidia") || mlow.contains("geforce") {
                vendor = "nvidia".into();
            } else if mlow.contains("amd") || mlow.contains("radeon") {
                vendor = "amd".into();
            } else if mlow.contains("intel") {
                vendor = "intel".into();
            } else if mlow.contains("adreno") {
                vendor = "qualcomm".into();
            } else if mlow.contains("mali") {
                vendor = "arm".into();
            } else if mlow.contains("apple") {
                vendor = "apple".into();
            }
        }
    } else {
        model = strip_trailing_api(s);
    }

    // iss/69 U9: hot alias table overrides (vendor + model family) before slug
    if let Some(t) = alias_table() {
        let (v2, m2) = apply_aliases(&vendor, &model, &lower, &t);
        vendor = v2;
        model = m2;
    }

    let slug = model_slug(&model, &vendor);
    // Reject empty/unk slugs that just echo generic tokens
    if slug == "unk" || slug == "webkit_webgl" || slug == "webgl" || slug == "gpu" {
        return json!({
            "ok": false,
            "error": "unresolved_model_slug",
            "algo": MODEL_KEY_ALGO,
            "vendor": vendor,
            "model_slug": slug,
            "gl_backend": backend,
            "raw_sanitized": sanitize_preview(s),
            "key_source": "rejected_unresolved",
            "key_confidence": 0.0,
        });
    }
    // Second-line defense: privacy mask often survives as slug `*_or_similar`
    if slug.contains("or_similar") || slug.ends_with("_similar") {
        return json!({
            "ok": false,
            "error": "privacy_masked_renderer",
            "algo": MODEL_KEY_ALGO,
            "vendor": vendor,
            "model_slug": slug,
            "gl_backend": backend,
            "raw_sanitized": sanitize_preview(s),
            "key_source": "rejected_privacy_mask",
            "key_confidence": 0.0,
            "note": "slug contains or_similar — use class caps K",
        });
    }

    let key = format!("{vendor}:{slug}");
    let conf = if vendor != "unk" && slug != "unk" && !slug.contains("webkit") {
        0.85
    } else if vendor != "unk" {
        0.55
    } else {
        0.35
    };

    json!({
        "ok": true,
        "algo": MODEL_KEY_ALGO,
        "hw_model_key": key,
        "vendor": vendor,
        "model_slug": slug,
        "gl_backend": backend,
        "key_source": "webgl_unmasked_renderer",
        "key_confidence": conf,
        "raw_sanitized": sanitize_preview(s),
    })
}

fn backend_of(lower: &str) -> &'static str {
    if lower.contains("d3d11") || lower.contains("direct3d11") {
        "d3d11"
    } else if lower.contains("d3d12") || lower.contains("direct3d12") {
        "d3d12"
    } else if lower.contains("metal") {
        "metal"
    } else if lower.contains("vulkan") {
        "vulkan"
    } else if lower.contains("opengl") || lower.contains("mesa") {
        "gl"
    } else if lower.contains("swiftshader") {
        "swiftshader"
    } else {
        "unk"
    }
}

fn vendor_token(s: &str) -> &'static str {
    let l = s.to_lowercase();
    if l.contains("nvidia") || l.contains("geforce") || l.contains("quadro") {
        "nvidia"
    } else if l.contains("amd") || l.contains("radeon") || l.contains("ati ") {
        "amd"
    } else if l.contains("intel") {
        "intel"
    } else if l.contains("apple") {
        "apple"
    } else if l.contains("qualcomm") || l.contains("adreno") {
        "qualcomm"
    } else if l.contains("arm") || l.contains("mali") {
        "arm"
    } else if l.contains("imagination") || l.contains("powervr") {
        "imgtec"
    } else if l.contains("google") {
        "google"
    } else {
        "unk"
    }
}

fn strip_trailing_api(s: &str) -> String {
    let mut out = s.to_string();
    // Case-insensitive cut markers (ANGLE / Mesa / Apple strings vary).
    let lower = out.to_lowercase();
    for cut in [
        " direct3d11",
        " direct3d12",
        " opengl",
        " (opengl",
        " vs_",
        " ps_",
        "/pcie",
        " sse2",
        " open gl",
    ] {
        if let Some(i) = lower.find(cut) {
            out.truncate(i);
            break;
        }
    }
    out.trim().trim_end_matches('(').trim().to_string()
}

fn model_slug(model: &str, vendor: &str) -> String {
    let mut t = model.to_lowercase();
    // Normalize Adreno "(TM)" form but keep family token for key readability.
    if t.contains("adreno") {
        t = t.replace("(tm)", "").replace("  ", " ");
        // "adreno 650" / "adreno (tm) 650" → keep adreno_NNN
        if let Some(rest) = t.strip_prefix("qualcomm ") {
            t = rest.to_string();
        }
        // collapse later; do not strip adreno prefix
    } else {
        for prefix in [
            "nvidia ",
            "amd ",
            "ati ",
            "intel(r) ",
            "intel ",
            "apple ",
            "qualcomm ",
            "mali-",
            "mali ",
            "geforce ",
            "radeon ",
        ] {
            if let Some(rest) = t.strip_prefix(prefix) {
                t = rest.to_string();
            }
        }
    }
    // collapse non-alnum to underscore
    let mut slug = String::new();
    let mut prev_us = false;
    for ch in t.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            prev_us = false;
        } else if !prev_us {
            slug.push('_');
            prev_us = true;
        }
    }
    // Drop PCI / ACPI device-id tokens (`0x00007d51`) — they fragment same-SKU K on 178.
    let slug = slug
        .split('_')
        .filter(|tok| {
            let t = *tok;
            !(t.starts_with("0x")
                && t.len() >= 3
                && t.bytes().skip(2).all(|b| b.is_ascii_hexdigit()))
        })
        .collect::<Vec<_>>()
        .join("_");
    let slug = slug.trim_matches('_').to_string();
    if slug.is_empty() {
        if vendor == "unk" {
            "unk".into()
        } else {
            format!("{vendor}_gpu")
        }
    } else {
        slug
    }
}

fn sanitize_preview(s: &str) -> String {
    s.chars().take(96).collect()
}

fn first_str(m: &Map<String, Value>, keys: &[&str]) -> String {
    for k in keys {
        if let Some(s) = m.get(*k).and_then(|v| v.as_str()) {
            let t = s.trim();
            if !t.is_empty() && t != "\"\"" && t != "null" {
                return t.to_string();
            }
        }
    }
    String::new()
}

/// Extract i64 from bare number or dual-channel / nested probe objects.
fn i64_from_value(v: &Value) -> Option<i64> {
    if let Some(i) = v.as_i64() {
        return Some(i);
    }
    if let Some(f) = v.as_f64() {
        if f.is_finite() {
            return Some(f as i64);
        }
    }
    if let Some(s) = v.as_str() {
        if let Ok(i) = s.trim().parse::<i64>() {
            return Some(i);
        }
        if let Ok(f) = s.trim().parse::<f64>() {
            if f.is_finite() {
                return Some(f as i64);
            }
        }
    }
    if let Some(o) = v.as_object() {
        for k in [
            "value",
            "resolved",
            "chosen",
            "chosen_value",
            "n",
            "i64",
            "max_texture_size",
            "gl_max_texture_size",
        ] {
            if let Some(inner) = o.get(k) {
                if let Some(i) = i64_from_value(inner) {
                    return Some(i);
                }
            }
        }
    }
    None
}

fn first_i64(m: &Map<String, Value>, keys: &[&str]) -> Option<i64> {
    for k in keys {
        if let Some(v) = m.get(*k) {
            if let Some(i) = i64_from_value(v) {
                return Some(i);
            }
        }
    }
    None
}

/// Class-level fallback K when unmasked renderer is generic (iss/63 / 178).
///
/// Commercial class K uses only **GL texture-size buckets** (cross-browser stable).
/// Engine-sensitive / farblable signals stay diagnostic only:
/// - `webgl_extensions_hash` → `class_ext_token` (iss/73 / iss/75)
/// - `hardware_concurrency` → `class_cores_token` (Brave farble forks c4 vs c12p)
///
/// Without a numeric texture size, commercial K is **incomplete** (no `class:c*` /
/// `class:ext*` slug). CIF must wait for caps — 178 showed same silicon forking
/// `t16k` vs `c12p` when texture was missing on some sites.
fn class_key_from_caps(fields: &Map<String, Value>) -> Option<Value> {
    let ext = first_str(
        fields,
        &[
            "webgl_extensions_hash",
            "gl_ext_full_hash",
            "webgl_ext_hash",
            "webgl_params_digest",
        ],
    );
    let tex = first_i64(
        fields,
        &[
            "gl_max_texture_size",
            "webgl_max_texture",
            "max_texture_size",
            "claimed_max_tex",
            "caps_claimed_max_tex",
            "actual_max_tex",
        ],
    );
    let cores = first_i64(
        fields,
        &[
            "hardware_concurrency",
            "hw_concurrency",
            "navigator_hardware_concurrency",
        ],
    );
    if ext.is_empty() && tex.is_none() && cores.is_none() {
        return None;
    }
    let class_cores_token = cores.map(|c| {
        match c {
            x if x >= 12 => "c12p",
            x if x >= 8 => "c8",
            x if x >= 4 => "c4",
            x if x >= 2 => "c2",
            _ => "c1",
        }
        .to_string()
    });
    let class_ext_token = if !ext.is_empty() {
        let tok = if ext.len() > 12 { &ext[..12] } else { &ext };
        Some(format!("ext{tok}"))
    } else {
        None
    };

    let Some(t) = tex else {
        let mut out = json!({
            "ok": false,
            "error": "incomplete_class_caps_no_texture",
            "algo": MODEL_KEY_ALGO,
            "vendor": "class",
            "gl_backend": "unk",
            "key_source": "gl_caps_class",
            "key_confidence": 0.0,
            "note": "incomplete_k_await_texture",
            "commercial_k_ready": false,
        });
        if let Some(o) = out.as_object_mut() {
            if let Some(ext_tok) = class_ext_token {
                o.insert("class_ext_token".into(), json!(ext_tok));
            }
            if let Some(cb) = class_cores_token {
                o.insert("class_cores_token".into(), json!(cb));
            }
        }
        return Some(out);
    };

    let bucket = match t {
        x if x >= 16384 => "t16k",
        x if x >= 8192 => "t8k",
        x if x >= 4096 => "t4k",
        x if x >= 2048 => "t2k",
        _ => "t1k",
    };
    let mut out = json!({
        "ok": true,
        "algo": MODEL_KEY_ALGO,
        "hw_model_key": format!("class:{bucket}"),
        "vendor": "class",
        "model_slug": bucket,
        "gl_backend": "unk",
        "key_source": "gl_caps_class",
        "key_confidence": 0.4,
        "note": "fallback_when_unmasked_generic_or_missing",
        "commercial_k_ready": true,
        "texture_bucket": bucket,
        "gl_max_texture_size": t,
    });
    if let Some(o) = out.as_object_mut() {
        if let Some(ext_tok) = class_ext_token {
            o.insert("class_ext_token".into(), json!(ext_tok));
        }
        if let Some(cb) = class_cores_token {
            o.insert("class_cores_token".into(), json!(cb));
        }
    }
    Some(out)
}

/// Whether Model-ID is ready for commercial CIF (texture class / named / soft / mobile).
/// Rejects cores-only / ext-only / incomplete class fallbacks.
pub fn commercial_model_key_ready(mk: &Value) -> bool {
    if mk.get("commercial_k_ready").and_then(|v| v.as_bool()) == Some(false) {
        return false;
    }
    if mk.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return false;
    }
    let key = mk
        .get("hw_model_key")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if key.is_empty() || key == "unk" || key == "unk:unk" {
        return false;
    }
    if key.starts_with("class:t") {
        return true;
    }
    if key.starts_with("class:c") || key.starts_with("class:ext") {
        return false;
    }
    if key.starts_with("soft:") || key.starts_with("mobile:") {
        return true;
    }
    if key.contains(':') && !key.starts_with("class:") {
        return true;
    }
    false
}

/// iss/73 P1-4 / iss/75: class caps K is the cross-browser default; named is ops enhance.
/// Opt out with `GR_MODEL_KEY_DEFAULT_CLASS=0|false|off`.
pub fn model_key_default_class() -> bool {
    match gr_abi::env::get("MODEL_KEY_DEFAULT_CLASS") {
        Some(v) => !(v == "0" || v == "false" || v == "off"),
        None => true,
    }
}

/// Build model key object from probe fields map / JSON object.
pub fn model_key_from_fields(fields: &Value) -> Value {
    let fo = fields.as_object();
    let Some(m) = fo else {
        return json!({"ok": false, "error": "no_fields", "algo": MODEL_KEY_ALGO});
    };

    // Prefer true unmasked; never treat generic stack labels as GPU model.
    // Also accept governed_* only if they look like real GPU strings (not WebKit WebGL).
    let candidates = [
        "webgl_unmasked_renderer",
        "unmasked_renderer",
        "webgl_debug_renderer",
        "governed_webgl_renderer",
        "webgl_renderer_governed",
        "webgl_renderer",
    ];
    let mut mk = json!({
        "ok": false,
        "error": "no_renderer",
        "algo": MODEL_KEY_ALGO,
    });
    let mut named_mk: Option<Value> = None;
    for k in candidates {
        let r = first_str(m, &[k]);
        if r.is_empty() {
            continue;
        }
        let parsed = parse_unmasked_renderer(&r);
        if parsed.get("ok").and_then(|v| v.as_bool()) == Some(true) {
            let mut p = parsed;
            if let Some(o) = p.as_object_mut() {
                o.insert("renderer_field".into(), json!(k));
            }
            named_mk = Some(p.clone());
            mk = p;
            break;
        }
        // keep last rejection for diagnostics
        mk = parsed;
        if let Some(o) = mk.as_object_mut() {
            o.insert("renderer_field".into(), json!(k));
        }
    }

    // iss/73 P1-4: class-default K (named kept as ops enhance field)
    if model_key_default_class() {
        if let Some(class_k) = class_key_from_caps(m) {
            // When named is soft stack, still prefer soft over class
            if let Some(named) = named_mk.as_ref() {
                let nk = named
                    .get("hw_model_key")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if nk.starts_with("soft:") {
                    return named.clone();
                }
            }
            let mut class_out = class_k;
            let class_ready = commercial_model_key_ready(&class_out);
            if let Some(named) = named_mk.as_ref() {
                if let Some(o) = class_out.as_object_mut() {
                    o.insert(
                        "hw_model_key_named".into(),
                        named
                            .get("hw_model_key")
                            .cloned()
                            .unwrap_or(Value::Null),
                    );
                    o.insert(
                        "named_key_confidence".into(),
                        named
                            .get("key_confidence")
                            .cloned()
                            .unwrap_or(json!(0.0)),
                    );
                    o.insert("default_mode".into(), json!("class_primary_named_ops"));
                    // Bump conf only when texture class K is commercial-ready.
                    // Never inflate incomplete / cores-await paths to 0.55 (178 hygiene).
                    if class_ready && named.get("ok").and_then(|v| v.as_bool()) == Some(true) {
                        o.insert("key_confidence".into(), json!(0.55));
                    }
                }
            } else if let Some(o) = class_out.as_object_mut() {
                o.insert("default_mode".into(), json!("class_only"));
            }
            return class_out;
        }
    }

    // WebGPU adapter device can upgrade K when renderer failed or conf low
    if let Some(dev) = {
        let d = first_str(
            m,
            &[
                "webgpu_adapter_device",
                "webgpu_device",
                "webgpu_adapter_name",
                "gpu_adapter_device",
            ],
        );
        if d.is_empty() { None } else { Some(d) }
    } {
        let conf = mk
            .get("key_confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let ok = mk.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        if (!ok || conf < 0.55) && !is_generic_renderer_label(&dev) {
            let parsed = parse_unmasked_renderer(&dev);
            if parsed.get("ok").and_then(|v| v.as_bool()) == Some(true) {
                mk = parsed;
                if let Some(o) = mk.as_object_mut() {
                    o.insert("key_source".into(), json!("webgpu_adapter_device"));
                    o.insert(
                        "key_confidence".into(),
                        json!(o
                            .get("key_confidence")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.7)
                            .min(0.8)),
                    );
                }
            }
        }
        if let Some(o) = mk.as_object_mut() {
            o.insert(
                "webgpu_adapter_device_preview".into(),
                json!(sanitize_preview(&dev)),
            );
        }
    }

    // Android / UA-CH model as mobile K when GPU string weak
    if let Some(model) = {
        let d = first_str(m, &["ua_ch_model", "sec_ch_ua_model", "ua_model"]);
        if d.is_empty() { None } else { Some(d) }
    } {
        let conf = mk
            .get("key_confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let ok = mk.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        if !ok || conf < 0.5 {
            let slug = model_slug(&model, "mobile");
            if slug != "unk" && slug != "webgl" {
                mk = json!({
                    "ok": true,
                    "algo": MODEL_KEY_ALGO,
                    "hw_model_key": format!("mobile:{slug}"),
                    "vendor": "mobile",
                    "model_slug": slug,
                    "gl_backend": mk.get("gl_backend").cloned().unwrap_or(json!("unk")),
                    "key_source": "ua_ch_model",
                    "key_confidence": 0.55,
                    "ua_ch_model_preview": sanitize_preview(&model),
                });
            }
        } else if let Some(o) = mk.as_object_mut() {
            o.insert(
                "ua_ch_model_preview".into(),
                json!(sanitize_preview(&model)),
            );
        }
    }

    // Final fallback: class K from caps (high-repeat, honest low conf)
    let ok = mk.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    if !ok {
        if let Some(class_k) = class_key_from_caps(m) {
            return class_k;
        }
    }
    mk
}

/// Multi-precision Model-ID lanes (`dk0` full … `dk4/5/6` progressive coarsen).
/// Probe materials stay full; these are **analysis outputs** for match / recall.
pub fn model_key_segments_from_info(mk: &Value, _fields: &Value) -> Value {
    let primary = mk
        .get("hw_model_key")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let named = mk
        .get("hw_model_key_named")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let dk0 = if !named.is_empty() {
        named.clone()
    } else if !primary.is_empty() {
        primary.clone()
    } else {
        String::new()
    };

    let (dk4, dk5, dk6) = if dk0.is_empty() {
        (String::new(), String::new(), String::new())
    } else if dk0.starts_with("class:") {
        let bucket = dk0.trim_start_matches("class:");
        let dk4 = format!("class:{bucket}");
        let dk5 = match bucket {
            "t16k" | "t8k" => "class:thigh".to_string(),
            "t4k" | "t2k" | "t1k" => "class:tmid".to_string(),
            _ => format!("class:{bucket}"),
        };
        let dk6 = "class:gpu".to_string();
        (dk4, dk5, dk6)
    } else if dk0.starts_with("soft:") {
        (
            "soft:software_gl".to_string(),
            "soft:software_gl".to_string(),
            "soft:any".to_string(),
        )
    } else if dk0.starts_with("mobile:") {
        let slug = dk0.trim_start_matches("mobile:");
        let family = mobile_family_slug(slug);
        (
            format!("mobile:{family}"),
            format!("mobile:{}", mobile_vendor_slug(slug)),
            "mobile:any".to_string(),
        )
    } else if let Some((vendor, model)) = dk0.split_once(':') {
        let family = model_family_slug(model);
        let series = model_series_slug(model);
        (
            format!("{vendor}:{family}"),
            format!("{vendor}:{series}"),
            vendor.to_string(),
        )
    } else {
        (dk0.clone(), dk0.clone(), dk0.clone())
    };

    // Compat commercial key: prefer class default when configured, else dk0.
    let commercial = if !primary.is_empty() {
        primary
    } else {
        dk0.clone()
    };

    json!({
        "ok": !dk0.is_empty(),
        "algo": "hw_model_key_segments_v1",
        "dk0": if dk0.is_empty() { Value::Null } else { json!(dk0) },
        "dk4": if dk4.is_empty() { Value::Null } else { json!(dk4) },
        "dk5": if dk5.is_empty() { Value::Null } else { json!(dk5) },
        "dk6": if dk6.is_empty() { Value::Null } else { json!(dk6) },
        "hw_model_key": if commercial.is_empty() { Value::Null } else { json!(commercial) },
        "precision_lanes": {
            "dk0": "full_named_or_finest_class",
            "dk4": "family_or_texture_bucket",
            "dk5": "series_or_texture_tier",
            "dk6": "vendor_or_class_floor",
        },
        "policy": "probe_full_raw; analysis_emits_dk0_4_5_6; no_force_merge_via_global_coarsen",
    })
}

fn model_family_slug(model: &str) -> String {
    let m = model.to_ascii_lowercase();
    // gtx_1050_ti → gtx_1050; rtx_3060_laptop → rtx_3060
    let mut parts: Vec<&str> = m.split('_').collect();
    if parts.len() >= 3 {
        let last = parts[parts.len() - 1];
        if matches!(last, "ti" | "super" | "laptop" | "mobile" | "maxq" | "refresh") {
            parts.pop();
            return parts.join("_");
        }
    }
    m
}

fn model_series_slug(model: &str) -> String {
    let m = model.to_ascii_lowercase();
    // gtx_1050 → gtx_10xx; rtx_4090 → rtx_40xx; rx_7900 → rx_7xxx
    let parts: Vec<&str> = m.split('_').collect();
    if parts.len() >= 2 {
        let family = parts[0];
        let num = parts[1].chars().take_while(|c| c.is_ascii_digit()).collect::<String>();
        if num.len() >= 2 {
            // 1050 → 10xx; 4090 → 40xx; 7900 → 79xx
            let stem = &num[..num.len() - 2];
            if !stem.is_empty() {
                return format!("{family}_{stem}xx");
            }
            let h = &num[..1];
            return format!("{family}_{h}xx");
        }
    }
    model_family_slug(model)
}

fn mobile_family_slug(slug: &str) -> String {
    let s = slug.to_ascii_lowercase();
    if let Some(i) = s.find(|c: char| c.is_ascii_digit()) {
        let (head, rest) = s.split_at(i);
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.len() >= 2 {
            return format!("{head}{}", &digits[..digits.len() - 1]);
        }
        return format!("{head}{digits}");
    }
    s
}

fn mobile_vendor_slug(slug: &str) -> String {
    let s = slug.to_ascii_lowercase();
    if s.starts_with("adreno") {
        "adreno".into()
    } else if s.starts_with("mali") {
        "mali".into()
    } else if s.starts_with("apple") {
        "apple".into()
    } else if s.contains("xclipse") {
        "xclipse".into()
    } else {
        s.split(|c: char| !c.is_ascii_alphabetic())
            .next()
            .unwrap_or("mobile")
            .to_string()
    }
}

/// Attach model_key extras onto a mutable fields object (diagnostic / Atlas only).
/// `GR_MODEL_KEY_ATLAS=0` disables attachment entirely (iss/69 U9 rollback).
pub fn attach_model_key_extras(fields: &mut Map<String, Value>) {
    if !model_key_attach_enabled() {
        fields.insert("hw_model_key_attach".into(), json!("disabled_v1"));
        return;
    }
    let mk = model_key_from_fields(&Value::Object(fields.clone()));
    let segs = model_key_segments_from_info(&mk, &Value::Object(fields.clone()));
    fields.insert("hw_model_key_info".into(), mk.clone());
    fields.insert("hw_model_key_segments".into(), segs);
    if let Some(key) = mk.get("hw_model_key").and_then(|v| v.as_str()) {
        fields.insert("hw_model_key".into(), json!(key));
    }
    if let Some(named) = mk.get("hw_model_key_named").and_then(|v| v.as_str()) {
        fields.insert("hw_model_key_named".into(), json!(named));
    }
    if let Some(be) = mk.get("gl_backend").and_then(|v| v.as_str()) {
        fields.insert("gl_backend_class".into(), json!(be));
    }
}

/// Assert commercial device_id body does not embed raw model strings / keys.
pub fn commercial_body_excludes_plaintext_model(device_id: &str, model_key: &str) -> bool {
    if model_key.is_empty() || model_key == "unk:unk" {
        return true;
    }
    // body segments are sha10 digests / structured tokens — must not contain full key
    !device_id.contains(model_key)
        && !device_id.to_lowercase().contains("geforce")
        && !device_id.to_lowercase().contains("radeon")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_key_segments_named_and_class() {
        let named = json!({
            "ok": true,
            "hw_model_key": "class:t16k",
            "hw_model_key_named": "nvidia:gtx_1050_ti",
            "key_source": "unmasked",
        });
        let segs = model_key_segments_from_info(&named, &json!({}));
        assert_eq!(segs["dk0"], "nvidia:gtx_1050_ti");
        assert_eq!(segs["dk4"], "nvidia:gtx_1050");
        assert_eq!(segs["dk5"], "nvidia:gtx_10xx");
        assert_eq!(segs["dk6"], "nvidia");
        let class_only = json!({
            "ok": true,
            "hw_model_key": "class:t16k",
            "key_source": "gl_caps_class",
        });
        let c = model_key_segments_from_info(&class_only, &json!({}));
        assert_eq!(c["dk0"], "class:t16k");
        assert_eq!(c["dk4"], "class:t16k");
        assert_eq!(c["dk5"], "class:thigh");
        assert_eq!(c["dk6"], "class:gpu");
    }

    #[test]
    fn parse_angle_nvidia() {
        let r = parse_unmasked_renderer(
            "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 Direct3D11 vs_5_0 ps_5_0, D3D11)",
        );
        assert_eq!(r["vendor"], "nvidia");
        assert_eq!(r["hw_model_key"], "nvidia:rtx_3060");
        assert_eq!(r["gl_backend"], "d3d11");
        assert!(r["key_confidence"].as_f64().unwrap() >= 0.8);
    }

    #[test]
    fn parse_adreno() {
        let r = parse_unmasked_renderer("Adreno (TM) 650");
        assert_eq!(r["vendor"], "qualcomm");
        assert!(r["hw_model_key"].as_str().unwrap().contains("adreno"));
    }

    #[test]
    fn parse_swiftshader() {
        let r = parse_unmasked_renderer(
            "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero)), SwiftShader driver)",
        );
        assert_eq!(r["hw_model_key"], "soft:software_gl");
    }

    #[test]
    fn rejects_webkit_webgl_generic_label() {
        assert!(is_generic_renderer_label("WebKit WebGL"));
        let r = parse_unmasked_renderer("WebKit WebGL");
        assert_eq!(r["ok"], false);
        assert_eq!(r["error"], "generic_renderer_label");
    }

    #[test]
    fn from_fields_falls_back_to_class_key_when_webkit_only() {
        let fields = json!({
            "webgl_unmasked_renderer": "WebKit WebGL",
            "webgl_renderer": "WebKit WebGL",
            "webgl_extensions_hash": "fb2a1ce4ddb4f317aabb",
            "gl_max_texture_size": 8192,
            "hardware_concurrency": 8,
        });
        let mk = model_key_from_fields(&fields);
        assert_eq!(mk["ok"], true);
        assert_eq!(mk["key_source"], "gl_caps_class");
        let key = mk["hw_model_key"].as_str().unwrap();
        assert!(key.starts_with("class:"), "got {key}");
        // Commercial class K = texture bucket only; ext/cores stay diagnostic.
        assert_eq!(key, "class:t8k");
        assert_eq!(mk["class_ext_token"], "extfb2a1ce4ddb4");
        assert_eq!(mk["class_cores_token"], "c8");
        assert!(!key.contains("ext"), "ext must not fork commercial K: {key}");
        assert!(mk["key_confidence"].as_f64().unwrap() < 0.5);
    }

    #[test]
    fn class_key_stable_across_divergent_extension_hashes() {
        // iss/73 lab: Edge/Firefox often carry a different webgl_extensions_hash
        // than Chrome while texture buckets match — K must agree.
        // Brave farble also forks hardware_concurrency — cores must not enter K.
        let a = json!({
            "webgl_unmasked_renderer": "WebKit WebGL",
            "webgl_extensions_hash": "aaaaaaaaaaaaaaaaaaaa",
            "gl_max_texture_size": 16384,
            "hardware_concurrency": 12,
        });
        let b = json!({
            "webgl_unmasked_renderer": "WebKit WebGL",
            "webgl_extensions_hash": "bbbbbbbbbbbbbbbbbbbb",
            "gl_max_texture_size": 16384,
            "hardware_concurrency": 4,
        });
        let ka = model_key_from_fields(&a);
        let kb = model_key_from_fields(&b);
        assert_eq!(ka["hw_model_key"], "class:t16k");
        assert_eq!(kb["hw_model_key"], "class:t16k");
        assert!(commercial_model_key_ready(&ka));
        assert!(commercial_model_key_ready(&kb));
        assert_ne!(ka["class_ext_token"], kb["class_ext_token"]);
        assert_ne!(ka["class_cores_token"], kb["class_cores_token"]);
    }

    #[test]
    fn no_texture_is_incomplete_not_cores_commercial_key() {
        let fields = json!({
            "webgl_unmasked_renderer": "WebKit WebGL",
            "webgl_extensions_hash": "aaaaaaaaaaaaaaaaaaaa",
            "hardware_concurrency": 12,
        });
        let mk = model_key_from_fields(&fields);
        assert_eq!(mk["ok"], false);
        assert_eq!(mk["note"], "incomplete_k_await_texture");
        assert_eq!(mk["commercial_k_ready"], false);
        assert!(mk.get("hw_model_key").is_none() || mk["hw_model_key"].is_null());
        assert_eq!(mk["class_cores_token"], "c12p");
        assert!(!commercial_model_key_ready(&mk));
    }

    #[test]
    fn named_ok_does_not_inflate_incomplete_class_confidence() {
        let fields = json!({
            "webgl_unmasked_renderer": "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 Direct3D11 vs_5_0 ps_5_0, D3D11)",
            "hardware_concurrency": 12,
            // no texture → incomplete under class-default (wait for caps)
        });
        let mk = model_key_from_fields(&fields);
        assert_eq!(mk["ok"], false, "{mk}");
        assert_eq!(mk["note"], "incomplete_k_await_texture");
        assert!(mk["key_confidence"].as_f64().unwrap_or(1.0) < 0.1);
        assert!(!commercial_model_key_ready(&mk));
        // Named stays ops-only; must not become commercial class:c*
        let named = mk["hw_model_key_named"].as_str().unwrap_or("");
        assert!(named.contains("rtx_3060") || named.starts_with("nvidia:"), "{mk}");
        assert!(mk.get("hw_model_key").is_none() || mk["hw_model_key"].is_null());
    }

    #[test]
    fn named_key_strips_pci_device_id_token() {
        std::env::set_var("GR_MODEL_KEY_DEFAULT_CLASS", "0");
        let r = parse_unmasked_renderer(
            "ANGLE (Intel, Intel(R) Arc(TM) 140T GPU (16GB) (0x00007D51) Direct3D11 vs_5_0 ps_5_0, D3D11)",
        );
        assert_eq!(r["ok"], true, "{r}");
        let key = r["hw_model_key"].as_str().unwrap_or("");
        assert!(key.starts_with("intel:"), "{r}");
        assert!(
            !key.contains("0x") && !key.contains("07d51"),
            "PCI id must not fragment named K: {key}"
        );
        std::env::remove_var("GR_MODEL_KEY_DEFAULT_CLASS");
    }

    #[test]
    fn from_fields_prefers_real_angle_over_webkit_mask() {
        std::env::set_var("GR_MODEL_KEY_DEFAULT_CLASS", "0");
        let fields = json!({
            "webgl_renderer": "WebKit WebGL",
            "webgl_unmasked_renderer": "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 Direct3D11 vs_5_0 ps_5_0, D3D11)",
        });
        let mk = model_key_from_fields(&fields);
        assert_eq!(mk["ok"], true);
        assert_eq!(mk["hw_model_key"], "nvidia:rtx_3060");
        assert!(mk["key_confidence"].as_f64().unwrap() >= 0.8);
        std::env::remove_var("GR_MODEL_KEY_DEFAULT_CLASS");
    }

    #[test]
    fn parse_apple_m1_and_adreno_angle() {
        let a = parse_unmasked_renderer("Apple M1");
        assert_eq!(a["ok"], true);
        assert!(a["hw_model_key"].as_str().unwrap().contains("m1"));
        let b = parse_unmasked_renderer(
            "ANGLE (Google, Vulkan 1.1.0 (Adreno (TM) 650), OpenGL ES 3.2)",
        );
        assert_eq!(b["ok"], true);
        assert_eq!(b["vendor"], "qualcomm");
        assert!(b["hw_model_key"].as_str().unwrap().contains("adreno"));
    }

    /// Serialize tests that mutate the process-global alias table.
    fn alias_test_guard() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn alias_table_rewrites_vendor_and_model_family() {
        let _g = alias_test_guard();
        let n = load_model_key_aliases_from_str(
            r#"{"vendor_aliases": {"tokentech": "token"},
                "model_family_aliases": {"tt-": "tt"}}"#,
        )
        .unwrap();
        assert_eq!(n, 2);
        let r = parse_unmasked_renderer("TokenTech TT-9000 (OpenGL 4.6)");
        assert_eq!(r["vendor"], "token", "{r}");
        let key = r["hw_model_key"].as_str().unwrap_or("");
        assert!(key.contains("tt9000"), "family alias must glue TT-9000: {r}");
        assert!(key.starts_with("token:"), "{r}");
        // model family alias replaces first occurrence: "tt-9000" → "tt9000"
        assert!(!key.contains("tt_9000"), "alias should remove the dash: {r}");
        // reset table for other tests
        let _ = load_model_key_aliases_from_str("{}");
        let r2 = parse_unmasked_renderer("TokenTech TT-9000 (OpenGL 4.6)");
        assert_eq!(r2["vendor"], "unk", "{r2}");
    }

    #[test]
    fn bad_alias_file_keeps_prior_table() {
        let _g = alias_test_guard();
        let _ = load_model_key_aliases_from_str(r#"{"vendor_aliases": {"tokentech": "token"}}"#)
            .unwrap();
        let r = load_model_key_aliases_from_str("{not json");
        assert!(r.is_err());
        // prior table still active
        let p = parse_unmasked_renderer("TokenTech TT-1");
        assert_eq!(p["vendor"], "token", "{p}");
        let _ = load_model_key_aliases_from_str("{}");
    }

    #[test]
    fn from_fields_attaches_without_body_leak() {
        let mut fo = Map::new();
        fo.insert(
            "webgl_unmasked_renderer".into(),
            json!("ANGLE (NVIDIA Corporation, NVIDIA GeForce GTX 1050 Ti/PCIe/SSE2, OpenGL 4.5.0)"),
        );
        // Real multipath residual so select_device_segments mints a body
        fo.insert(
            "hw_curve_webgl".into(),
            json!([0.11, 0.22, 0.13, 0.24, 0.15, 0.26, 0.17, 0.28, 0.19, 0.21, 0.12, 0.23, 0.14, 0.25, 0.16, 0.27]),
        );
        fo.insert("residual_mean".into(), json!(0.2603390625));
        fo.insert("residual_std".into(), json!(0.012));
        attach_model_key_extras(&mut fo);
        let key = fo["hw_model_key"].as_str().unwrap().to_string();
        assert!(key.starts_with("nvidia:"));
        // Prove real mint body (select_device_segments SSOT) does not embed plaintext model/key
        let segs = crate::device_segments::select_device_segments(
            &Value::Object(fo.clone()),
            None,
        );
        let device_id = segs
            .get("device_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(!device_id.is_empty(), "expected mint body from segments");
        assert!(
            commercial_body_excludes_plaintext_model(device_id, &key),
            "commercial body leaked model key or brand: id={device_id} key={key}"
        );
        assert!(
            !device_id.to_lowercase().contains("1050"),
            "body must not contain raw model number: {device_id}"
        );
        // K↔V diagnostics present on segment pack
        assert!(segs.get("hw_model_key").and_then(|v| v.as_str()).is_some());
        assert!(segs.get("slot_quality_conf").is_some());
    }

    #[test]
    fn privacy_masked_or_similar_refuses_named_key() {
        let r = parse_unmasked_renderer("NVIDIA GeForce GTX 980, or similar");
        assert_eq!(r["ok"], false, "{r}");
        assert_eq!(r["error"], "privacy_masked_renderer");
        assert_eq!(r["key_source"], "rejected_privacy_mask");
        // from_fields must fall through to class when only privacy-masked renderer
        let fields = json!({
            "webgl_unmasked_renderer": "NVIDIA GeForce GTX 980, or similar",
            "gl_max_texture_size": 16384,
            "gl_max_renderbuffer": 16384,
            "gl_max_vertex_attribs": 16,
            "gl_max_varying_vectors": 30,
            "gl_high_float": [23, 127, 127],
            "webgl_depth_bits": 24,
            "webgl_samples": 4,
        });
        let mk = model_key_from_fields(&fields);
        let key = mk["hw_model_key"].as_str().unwrap_or("");
        assert!(
            key.starts_with("class:") || key.starts_with("unk:"),
            "privacy mask should not mint named GTX980; got {key}"
        );
        assert_ne!(key, "nvidia:gtx_980_or_similar");
    }

    #[test]
    fn default_class_mode_puts_named_as_ops_field() {
        std::env::set_var("GR_MODEL_KEY_DEFAULT_CLASS", "1");
        let fields = json!({
            "webgl_unmasked_renderer": "ANGLE (NVIDIA Corporation, NVIDIA GeForce GTX 1050 Ti/PCIe/SSE2, OpenGL 4.5.0)",
            "gl_max_texture_size": 16384,
            "gl_max_renderbuffer": 16384,
            "gl_max_vertex_attribs": 16,
            "gl_max_varying_vectors": 30,
            "gl_high_float": [23, 127, 127],
            "webgl_depth_bits": 24,
            "webgl_samples": 4,
        });
        let mk = model_key_from_fields(&fields);
        let key = mk["hw_model_key"].as_str().unwrap_or("");
        assert!(
            key.starts_with("class:"),
            "default class mode should use class: key; got {key} {mk}"
        );
        let named = mk["hw_model_key_named"].as_str().unwrap_or("");
        assert!(
            named.contains("nvidia:") || named.contains("1050"),
            "named should be preserved for ops: {mk}"
        );
        std::env::remove_var("GR_MODEL_KEY_DEFAULT_CLASS");
    }
}
