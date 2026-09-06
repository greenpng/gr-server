//! Multi-source mint gate (product contract 2026-08).
//!
//! **Never mint commercial device_id from a field that has only one data source.**
//! A material enters the mint body only when:
//! 1. ≥2 independent sources carry a value and **agree**, then pick the
//!    **harder-to-forge** source (see `source_trust`); **or**
//! 2. Sources **conflict** on soft/network keys → pick harder-to-forge source
//!    for mint **and** demote OS/BR/RPA; silicon hard conflict → **exclude** mint; **or**
//! 3. A documented **dual-channel** pair exists (webgl+audio, residual+webrtc, …).
//!
//! Single-source fields remain available for OS/BR/RPA **confidence** and demotion.
//!
//! Source trust ladder (not main-default): gateway/CF > worker nest > iframe nest
//! > main silicon > main soft. Details: `source_trust` module.

use crate::source_trust::{
    build_source_auth_ladder_view, resolve_field_source, source_trust_for, KeyClass, key_class,
};
use serde_json::{json, Map, Value};

/// Semantic keys that may feed commercial mint / residual-led body.
pub const MINT_SEMANTIC_KEYS: &[&str] = &[
    "residual_std",
    "residual_mean",
    "residual_ok",
    "hw_curve_webgl",
    "hw_curve_audio",
    "hw_curve_cpu",
    "webrtc_host_ip_hash",
    "webrtc_host_ip_hash_v2",
    "os_instance_hash",
    "hardware_concurrency",
    "device_memory",
    "form_class",
    "platform",
    "os_family",
    "architecture",
    "timezone",
    "user_agent",
    "gateway_user_agent",
    "media_input_count",
    "media_output_count",
    "media_video_count",
    "media_device_count",
    "display_count",
    "screen_width",
    "screen_height",
    "webgl_unmasked_renderer",
    "unit_surface_id",
    "gl_max_texture_size",
    "webgl_max_texture",
    "ja4",
    "protocol_engine",
    "http_header_order_hash",
];

/// Explicitly never commercial mint (even if present on fields).
/// Fixed-name claim fields are already excluded by device_segments body policy;
/// this list is for **diagnostic / timing-ladder / governed** materials that
/// must not enter multi-source mint resolution at all (iss/46 H2–H4, iss/50).
pub const NEVER_COMMERCIAL_MINT_KEYS: &[&str] = &[
    "cpu_cache_ladder",
    "cpu_cache_knee_bytes",
    "h2_priority_fingerprint",
    "h3_pseudo_order",
    "h3_settings_fp",
    "governed_webgl_renderer",
    "webgl_renderer_governed",
    "server_client_ip",
];

/// Independent dual channels: either key present with the other → dual-source ok.
/// Pairs are unordered for matching.
const DUAL_CHANNELS: &[(&str, &str)] = &[
    // Silicon dual anchors (industry: multi-signal canvas/webgl/audio)
    ("hw_curve_webgl", "hw_curve_audio"),
    ("residual_std", "hw_curve_webgl"),
    ("residual_mean", "hw_curve_webgl"),
    ("residual_std", "hw_curve_audio"),
    // Host separator dual (WebRTC + OS instance when both real)
    ("webrtc_host_ip_hash", "os_instance_hash"),
    ("webrtc_host_ip_hash_v2", "os_instance_hash"),
    ("webrtc_host_ip_hash_v2", "webrtc_host_ip_hash"),
    // Env dual
    ("hardware_concurrency", "device_memory"),
    ("media_device_count", "display_count"),
    ("media_input_count", "media_output_count"),
    ("screen_width", "hardware_concurrency"),
    ("form_class", "platform"),
];

fn is_browser_src(src: &str) -> bool {
    let b = src.split(':').next().unwrap_or(src);
    matches!(
        b,
        "main"
            | "iframe"
            | "worker"
            | "sandbox"
            | "sandbox_iframe"
            | "shared_worker"
            | "service_worker"
    )
}

fn is_edge_src(src: &str) -> bool {
    let b = src.split(':').next().unwrap_or(src);
    matches!(b, "gateway" | "cloudflare" | "cf")
}

fn nonempty(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        Value::Bool(_) | Value::Number(_) => true,
    }
}

fn approx_eq(a: &Value, b: &Value) -> bool {
    if a == b {
        return true;
    }
    // Numeric soft compare (float noise)
    if let (Some(x), Some(y)) = (a.as_f64(), b.as_f64()) {
        if !x.is_finite() || !y.is_finite() {
            return false;
        }
        let scale = x.abs().max(y.abs()).max(1e-9);
        return (x - y).abs() / scale < 1e-4 || (x - y).abs() < 1e-6;
    }
    // Array curves: compare stable digest of first N samples (length + coarse mean)
    if let (Some(aa), Some(bb)) = (a.as_array(), b.as_array()) {
        if aa.is_empty() || bb.is_empty() {
            return false;
        }
        // Length class
        let la = aa.len();
        let lb = bb.len();
        if (la as i32 - lb as i32).abs() > (la.max(lb) as i32 / 8).max(2) {
            return false;
        }
        let mean = |arr: &[Value]| {
            let mut s = 0.0;
            let mut n = 0.0;
            for v in arr.iter().take(32) {
                if let Some(x) = v.as_f64() {
                    s += x;
                    n += 1.0;
                }
            }
            if n > 0.0 {
                s / n
            } else {
                0.0
            }
        };
        let ma = mean(aa);
        let mb = mean(bb);
        return (ma - mb).abs() < 0.05 * ma.abs().max(mb.abs()).max(0.01)
            || (ma - mb).abs() < 1e-3;
    }
    // String soft: case-insensitive trim
    if let (Some(sa), Some(sb)) = (a.as_str(), b.as_str()) {
        return sa.eq_ignore_ascii_case(sb);
    }
    false
}

fn conflict_keys(source_conflicts: &[String]) -> std::collections::HashSet<String> {
    source_conflicts
        .iter()
        .filter_map(|c| c.strip_prefix("source_conflict:").map(|s| s.to_string()))
        .collect()
}

/// Collect (source, value) for a key across browser **and** edge sources.
fn collect_source_values(
    key: &str,
    fields_by_source: Option<&Map<String, Value>>,
    merged: &Map<String, Value>,
    evidence: Option<&Value>,
) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    if let Some(fbs) = fields_by_source {
        for (src, bucket) in fbs {
            if !(is_browser_src(src) || is_edge_src(src)) {
                continue;
            }
            if let Some(v) = bucket.get(key) {
                if nonempty(v) {
                    out.push((src.clone(), v.clone()));
                }
            }
        }
    }
    // Edge slices may live outside fields_by_source
    if let Some(ev) = evidence {
        // CF only when real edge present (lab has none)
        let cf_ok = ev
            .pointer("/cf_fields")
            .and_then(|v| v.as_object())
            .is_some_and(|o| {
                o.get("cf_connecting_ip")
                    .or_else(|| o.get("cf-connecting-ip"))
                    .or_else(|| o.get("cf_ray"))
                    .is_some_and(nonempty)
            })
            || ev
                .get("fields")
                .and_then(|f| f.get("cf_edge_present"))
                .and_then(|v| v.as_bool())
                == Some(true);

        for (label, path) in [
            ("gateway", "/gateway_fields"),
            ("cloudflare", "/cf_fields"),
        ] {
            if label == "cloudflare" && !cf_ok {
                continue;
            }
            if let Some(obj) = ev.pointer(path).and_then(|v| v.as_object()) {
                if let Some(v) = obj.get(key) {
                    if nonempty(v) && !out.iter().any(|(s, _)| s == label) {
                        out.push((label.into(), v.clone()));
                    }
                }
                // gateway_user_agent supplies user_agent conflicts
                if key == "user_agent" {
                    if let Some(v) = obj
                        .get("gateway_user_agent")
                        .or_else(|| obj.get("edge_user_agent"))
                    {
                        if nonempty(v) && !out.iter().any(|(s, _)| s == label) {
                            out.push((label.into(), v.clone()));
                        }
                    }
                }
                // CF-IP aliases only when CF present
                if key == "server_client_ip" && label == "cloudflare" {
                    for ak in ["cf_connecting_ip", "cf-connecting-ip", "client_ip"] {
                        if let Some(v) = obj.get(ak) {
                            if nonempty(v) && !out.iter().any(|(s, _)| s == label) {
                                out.push((label.into(), v.clone()));
                            }
                        }
                    }
                }
            }
        }
        // Also merge fields from B8 on evidence.fields when tagged gateway
        if let Some(obj) = ev.get("fields").and_then(|v| v.as_object()) {
            if key == "user_agent" || key == "gateway_user_agent" {
                if let Some(v) = obj.get("gateway_user_agent") {
                    if nonempty(v) && !out.iter().any(|(s, _)| s == "gateway") {
                        out.push(("gateway".into(), v.clone()));
                    }
                }
            }
        }
    }
    // Fallback: merged fields alone cannot prove multi-source; still record as main.
    if out.is_empty() {
        if let Some(v) = merged.get(key) {
            if nonempty(v) {
                out.push(("merged".into(), v.clone()));
            }
        }
    }
    out
}

fn dual_channel_partner_present(key: &str, merged: &Map<String, Value>) -> Option<&'static str> {
    for (a, b) in DUAL_CHANNELS {
        if *a == key {
            if merged.get(*b).is_some_and(nonempty) {
                return Some(*b);
            }
        } else if *b == key {
            if merged.get(*a).is_some_and(nonempty) {
                return Some(*a);
            }
        }
    }
    None
}

/// Decide whether `key` may enter commercial mint body.
///
/// Conflict resolution uses **source_trust ladder** (harder-to-forge wins), not main-default.
pub fn field_mint_decision(
    key: &str,
    fields: &Value,
    fields_by_source: Option<&Map<String, Value>>,
    source_conflicts: &[String],
) -> Value {
    field_mint_decision_ex(key, fields, fields_by_source, source_conflicts, None)
}

pub fn field_mint_decision_ex(
    key: &str,
    fields: &Value,
    fields_by_source: Option<&Map<String, Value>>,
    source_conflicts: &[String],
    evidence: Option<&Value>,
) -> Value {
    if NEVER_COMMERCIAL_MINT_KEYS.contains(&key) {
        return json!({
            "key": key,
            "mint_ok": false,
            "mint_value": null,
            "reason": "never_commercial_mint_key",
            "source_count": 0,
            "sources": [],
            "commercial_mint_allowed": false,
        });
    }
    let fo = fields.as_object().cloned().unwrap_or_default();
    let entries = collect_source_values(key, fields_by_source, &fo, evidence);
    let listed_conflict = conflict_keys(source_conflicts).contains(key);
    let source_count = entries.len();
    let sources: Vec<String> = entries.iter().map(|(s, _)| s.clone()).collect();

    let dual = dual_channel_partner_present(key, &fo);
    let dual_ok = dual.is_some();

    let res = resolve_field_source(key, &entries, approx_eq);
    let conflicted = listed_conflict || res.conflicted;

    // Value selection: trust-ladder chosen source, else dual-channel merged, else single.
    let resolved = if let Some(ref src) = res.chosen_source {
        entries
            .iter()
            .find(|(s, _)| s == src)
            .map(|(_, v)| v.clone())
            .or_else(|| fo.get(key).cloned())
    } else if dual_ok {
        fo.get(key).cloned()
    } else if source_count == 1 {
        entries.first().map(|(_, v)| v.clone())
    } else {
        None
    };

    // Mint ok:
    // - multi agree + ladder pick
    // - soft/network conflict resolved to harder source (mint_value_ok)
    // - dual channel without silicon hard-conflict
    // - single-source **non-silicon** inventory/caps (GL texture, cores, viewport):
    //   OSS (FPJS/Thumbmark) collect these sync in one shot; v5 must not drop them
    //   as conf_only-only when only `main` has reported (178 missing class:t* K).
    let kclass = key_class(key);
    let non_silicon_single_ok = resolved.is_some()
        && source_count == 1
        && !conflicted
        && !matches!(kclass, KeyClass::Silicon);
    let mint_ok = resolved.is_some()
        && (res.mint_value_ok
            || (dual_ok && kclass != KeyClass::Silicon)
            || (dual_ok && !conflicted)
            || non_silicon_single_ok);

    // Silicon dual-channel still requires no hard multi-source silicon conflict
    let mint_ok = if kclass == KeyClass::Silicon && res.conflicted {
        false
    } else {
        mint_ok
    };

    let conf_only = resolved.is_some() && !mint_ok;
    let chosen_trust = res
        .chosen_source
        .as_ref()
        .map(|s| source_trust_for(s, key).as_str())
        .unwrap_or("none");

    json!({
        "key": key,
        "source_count": source_count,
        "sources": sources,
        "agreed": res.agreed,
        "dual_channel": dual,
        "mint_ok": mint_ok,
        "conf_only": conf_only,
        "conflicted": conflicted,
        "chosen_source": res.chosen_source,
        "chosen_trust": chosen_trust,
        "key_class": format!("{:?}", key_class(key)),
        "reason": res.reason,
        "resolved_present": resolved.is_some(),
        "policy": "prefer_harder_to_forge_on_conflict",
    })
}

/// Full mint gate over semantic keys + overall residual/host eligibility.
pub fn assess_mint_gate(fields: &Value, evidence: Option<&Value>) -> Value {
    let fbs = evidence
        .and_then(|e| e.get("fields_by_source"))
        .and_then(|v| v.as_object());
    let conflicts: Vec<String> = evidence
        .and_then(|e| e.get("source_conflicts"))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let mut decisions = Map::new();
    let mut mint_ok_keys = Vec::new();
    let mut conf_only_keys = Vec::new();
    let mut conflict_keys_v = Vec::new();
    // Any silicon key with multi-source hard conflict poisons the whole silicon mint body.
    // Sibling dual channels (e.g. residual_std+audio while residual_mean+webgl conflict)
    // must NOT resurrect residual_mint_ok / silicon_mint_ok.
    let mut silicon_hard_conflict = false;
    let mut silicon_conflict_keys: Vec<String> = Vec::new();

    for k in MINT_SEMANTIC_KEYS {
        let d = field_mint_decision_ex(k, fields, fbs, &conflicts, evidence);
        let conflicted = d.get("conflicted").and_then(|v| v.as_bool()).unwrap_or(false);
        let mint_ok = d.get("mint_ok").and_then(|v| v.as_bool()).unwrap_or(false);
        let conf_only = d.get("conf_only").and_then(|v| v.as_bool()).unwrap_or(false);
        let reason = d.get("reason").and_then(|v| v.as_str()).unwrap_or("");
        if key_class(k) == KeyClass::Silicon
            && (conflicted || reason == "silicon_conflict_exclude_mint")
        {
            silicon_hard_conflict = true;
            silicon_conflict_keys.push((*k).to_string());
        }
        if mint_ok {
            mint_ok_keys.push((*k).to_string());
        }
        if conf_only {
            conf_only_keys.push((*k).to_string());
        }
        if conflicted {
            conflict_keys_v.push((*k).to_string());
        }
        decisions.insert((*k).into(), d);
    }

    // Poison: strip all residual/curve keys from mint_ok when any silicon hard conflict.
    if silicon_hard_conflict {
        mint_ok_keys.retain(|k| {
            !matches!(
                k.as_str(),
                "residual_std"
                    | "residual_mean"
                    | "residual_ok"
                    | "residual_algo"
                    | "hw_curve_webgl"
                    | "hw_curve_audio"
                    | "hw_curve_cpu"
                    | "unit_surface_id"
                    | "webgl_unmasked_renderer"
                    | "webgl_unmasked_vendor"
            ) && !k.starts_with("hw_curve_")
        });
        for sk in &silicon_conflict_keys {
            if !conf_only_keys.iter().any(|c| c == sk) {
                conf_only_keys.push(sk.clone());
            }
        }
    }

    // Ops: full source-trust ladder view for dashboard
    let auth_ladder = fbs
        .map(|m| build_source_auth_ladder_view(m, MINT_SEMANTIC_KEYS, approx_eq))
        .unwrap_or_else(|| json!({"algo": "gr_source_trust_ladder_v1", "keys": {}}));

    // Silicon mint = Lane-C materials (residual / webgl). Audio alone is V_aux
    // (iss/24 / iss/75) and must not set silicon_mint_ok / mint_silicon_ok on 178.
    let silicon_mint = !silicon_hard_conflict
        && mint_ok_keys.iter().any(|k| {
            matches!(
                k.as_str(),
                "residual_std" | "residual_mean" | "hw_curve_webgl"
            ) || k.starts_with("hw_curve_webgl")
                || k.starts_with("webgl_residual")
        });
    // Prefer residual when residual scalars are mint_ok, **or** webgl curve is
    // mint_ok (Lane-C). Audio is no longer a dual-channel substitute for residual.
    let residual_mint_ok = !silicon_hard_conflict
        && (mint_ok_keys.iter().any(|k| {
            matches!(
                k.as_str(),
                "residual_std" | "residual_mean" | "residual_ok" | "hw_curve_webgl"
            )
        }));
    // Host separator: dual/multi-source host keys, **or** additive host field when
    // residual/silicon already dual-ok (host forks mint without being sole identity).
    // Prevents: (a) host-only mint from single spoofed webrtc; (b) blocking remint when
    // silicon dual-ok and os_instance/webrtc later arrives from one surface.
    // Host must not ride on poisoned silicon residual either.
    let host_key_present = {
        let fo = fields.as_object().cloned().unwrap_or_default();
        ["webrtc_host_ip_hash", "webrtc_host_ip_hash_v2", "os_instance_hash"]
            .iter()
            .any(|k| fo.get(*k).is_some_and(nonempty))
    };
    let host_mint_ok = mint_ok_keys.iter().any(|k| {
        matches!(
            k.as_str(),
            "webrtc_host_ip_hash" | "webrtc_host_ip_hash_v2" | "os_instance_hash"
        )
    }) || (residual_mint_ok && host_key_present)
        || (silicon_mint && host_key_present);

    // Conflict pressure for OS/BR/RPA demotion (0..1)
    let conflict_pressure = (conflict_keys_v.len() as f64 / 6.0).clamp(0.0, 1.0);
    let single_source_pressure = (conf_only_keys.len() as f64 / 10.0).clamp(0.0, 0.6);

    json!({
        "algo": "multi_source_mint_gate_v2_trust_ladder",
        "policy": "prefer_harder_to_forge_on_conflict; silicon_hard_conflict_poisons_all_silicon",
        "decisions": decisions,
        "source_auth_ladder": auth_ladder,
        "mint_ok_keys": mint_ok_keys,
        "conf_only_keys": conf_only_keys,
        "conflict_keys": conflict_keys_v,
        "silicon_hard_conflict": silicon_hard_conflict,
        "silicon_conflict_keys": silicon_conflict_keys,
        "silicon_mint_ok": silicon_mint,
        "residual_mint_ok": residual_mint_ok,
        "host_mint_ok": host_mint_ok,
        "conflict_pressure": conflict_pressure,
        "single_source_pressure": single_source_pressure,
        "source_conflicts_n": conflicts.len(),
    })
}

/// Whether residual class may enter ServerMint body.
pub fn residual_allowed_for_mint(gate: &Value) -> bool {
    gate.get("residual_mint_ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Whether host separator may enter ServerMint body.
pub fn host_sep_allowed_for_mint(gate: &Value) -> bool {
    gate.get("host_mint_ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Conflict demotion delta for product scores (shared OS/BR/RPA).
pub fn conflict_score_demotion(gate: &Value, source_conflicts: &[String]) -> (f64, Vec<String>) {
    let mut risk = 0.0;
    let mut reasons = Vec::new();
    let cp = gate
        .get("conflict_pressure")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let sp = gate
        .get("single_source_pressure")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    if cp > 0.0 {
        risk += 0.08 + 0.22 * cp;
        reasons.push(format!("multi_source_conflict_pressure={cp:.2}"));
    }
    if !source_conflicts.is_empty() {
        risk += 0.06 + 0.04 * (source_conflicts.len().min(5) as f64);
        reasons.push(format!(
            "source_conflicts_n={}",
            source_conflicts.len()
        ));
    }
    // Single-source heavy surface: mild demotion (not as harsh as conflict)
    if sp > 0.25 {
        risk += 0.04 * sp;
        reasons.push(format!("single_source_pressure={sp:.2}"));
    }
    (risk.min(0.45), reasons)
}

/// Keys that must keep the evidence_merge **union** view (never fbs last-write).
fn is_residual_paths_union_key(key: &str) -> bool {
    matches!(
        key,
        "residual_paths"
            | "residual_paths_b10x"
            | "residual_paths_extra"
            | "residual_paths_merged"
            | "residual_paths_n"
    )
}

/// Resolve multi-source field map using trust ladder (not main-default).
///
/// Returns:
/// - `resolved_fields`: merged fields with per-key winner values applied when mint_ok or soft resolve
/// - `resolutions`: per-key decision (chosen_source, conflicted, mint_ok, …)
/// - `summary`: conflict/agree counts for evaluate surface
pub fn resolve_fields_multi_source(fields: &Value, evidence: Option<&Value>) -> Value {
    let fbs = evidence
        .and_then(|e| e.get("fields_by_source"))
        .and_then(|v| v.as_object());
    let conflicts: Vec<String> = evidence
        .and_then(|e| e.get("source_conflicts"))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let mut resolved = fields.as_object().cloned().unwrap_or_default();
    let mut resolutions = Map::new();
    let mut conflict_n = 0usize;
    let mut agree_n = 0usize;
    let mut mint_ok_n = 0usize;
    let mut conf_only_n = 0usize;
    let mut silicon_excluded_n = 0usize;
    let mut silicon_hard_conflict = false;

    // Collect all keys present across sources + merged
    let mut keys: std::collections::BTreeSet<String> = MINT_SEMANTIC_KEYS
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    if let Some(fo) = fields.as_object() {
        for k in fo.keys() {
            keys.insert(k.clone());
        }
    }
    if let Some(fbs) = fbs {
        for (_src, bucket) in fbs {
            if let Some(obj) = bucket.as_object() {
                for k in obj.keys() {
                    keys.insert(k.clone());
                }
            }
        }
    }

    // Pass 1: decisions + detect silicon hard conflict
    let mut decision_list: Vec<(String, Value)> = Vec::new();
    for key in &keys {
        let d = field_mint_decision_ex(key, fields, fbs, &conflicts, evidence);
        let conflicted = d.get("conflicted").and_then(|v| v.as_bool()).unwrap_or(false);
        let reason = d.get("reason").and_then(|v| v.as_str()).unwrap_or("");
        if key_class(key) == KeyClass::Silicon
            && (conflicted || reason == "silicon_conflict_exclude_mint")
        {
            silicon_hard_conflict = true;
        }
        decision_list.push((key.clone(), d));
    }

    // Pass 2: apply resolutions with full silicon_hard_conflict knowledge
    for (key, d) in decision_list {
        let conflicted = d.get("conflicted").and_then(|v| v.as_bool()).unwrap_or(false);
        let agreed = d.get("agreed").and_then(|v| v.as_bool()).unwrap_or(false);
        let mint_ok = d.get("mint_ok").and_then(|v| v.as_bool()).unwrap_or(false);
        let conf_only = d.get("conf_only").and_then(|v| v.as_bool()).unwrap_or(false);
        let chosen = d
            .get("chosen_source")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let reason = d.get("reason").and_then(|v| v.as_str()).unwrap_or("");

        if conflicted {
            conflict_n += 1;
        }
        if agreed {
            agree_n += 1;
        }
        if mint_ok {
            mint_ok_n += 1;
        }
        if conf_only {
            conf_only_n += 1;
        }
        if key_class(&key) == KeyClass::Silicon
            && (conflicted || reason == "silicon_conflict_exclude_mint" || silicon_hard_conflict)
        {
            silicon_excluded_n += 1;
            resolved.remove(&key);
        }

        // Apply harder-to-forge / class-prefer value when we have a chosen source.
        // Include conf_only so single-source caps (texture/cores) still land in fields
        // for Model-ID — previously skipped when mint_ok=false && !agreed (178 K gap).
        if !chosen.is_empty() && (mint_ok || conflicted || agreed || conf_only) {
            if let Some(fbs) = fbs {
                if let Some(v) = fbs
                    .get(chosen)
                    .and_then(|b| b.get(&key))
                    .filter(|v| !v.is_null())
                {
                    // iss/75 B10x land: residual_paths are unioned at evidence_merge.
                    // Never re-apply fields_by_source last-write (Lane-S deep pack would
                    // wipe noderiv/float/rint and mint the incomplete 0900/e432 cluster).
                    let class = key_class(&key);
                    let apply = match class {
                        KeyClass::Silicon => mint_ok && !silicon_hard_conflict,
                        KeyClass::SoftIdentity
                        | KeyClass::Protocol
                        | KeyClass::Network
                        | KeyClass::Viewport
                        | KeyClass::Inventory
                        | KeyClass::Other => !is_residual_paths_union_key(&key),
                    };
                    if apply {
                        resolved.insert(key.clone(), v.clone());
                    }
                }
            }
        }

        let src_n = d.get("source_count").and_then(|v| v.as_u64()).unwrap_or(0);
        if src_n >= 1 || conflicted || mint_ok {
            resolutions.insert(key, d);
        }
    }

    // Poison all residual/curve siblings when any silicon hard conflict.
    if silicon_hard_conflict {
        let poison_keys: Vec<String> = resolved
            .keys()
            .filter(|k| {
                matches!(
                    k.as_str(),
                    "residual_mean"
                        | "residual_std"
                        | "residual_ok"
                        | "residual_algo"
                        | "hw_curve_webgl"
                        | "hw_curve_audio"
                        | "hw_curve_cpu"
                        | "hw_webgl_stable"
                        | "hw_audio_stable"
                        | "unit_surface_id"
                ) || k.starts_with("hw_curve_")
            })
            .cloned()
            .collect();
        for k in poison_keys {
            resolved.remove(&k);
            silicon_excluded_n += 1;
        }
    }

    json!({
        "algo": "resolve_fields_multi_source_v2",
        "policy": "prefer_harder_to_forge_on_conflict; silicon_hard_conflict_poisons_all_silicon; residual_paths_keep_union",
        "resolved_fields": Value::Object(resolved),
        "resolutions": resolutions,
        "summary": {
            "conflict_keys_n": conflict_n,
            "agree_keys_n": agree_n,
            "mint_ok_keys_n": mint_ok_n,
            "conf_only_keys_n": conf_only_n,
            "silicon_excluded_n": silicon_excluded_n,
            "silicon_hard_conflict": silicon_hard_conflict,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn single_source_residual_not_mint_without_dual() {
        let fields = json!({
            "residual_std": 0.09,
            // no webgl curve, no second source
        });
        let g = assess_mint_gate(&fields, None);
        assert_eq!(g["residual_mint_ok"], false);
        assert!(g["conf_only_keys"]
            .as_array()
            .unwrap()
            .iter()
            .any(|k| k.as_str() == Some("residual_std")));
    }

    #[test]
    fn dual_channel_webgl_audio_makes_silicon_mint_ok() {
        let fields = json!({
            "hw_curve_webgl": [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8],
            "hw_curve_audio": [0.01, 0.02, 0.03, 0.04, 0.05, 0.06, 0.07, 0.08],
            "residual_std": 0.09075,
            "residual_mean": 0.26,
        });
        let g = assess_mint_gate(&fields, None);
        assert_eq!(g["silicon_mint_ok"], true);
        assert_eq!(g["residual_mint_ok"], true);
    }

    #[test]
    fn audio_only_does_not_make_silicon_mint_ok() {
        let fields = json!({
            "hw_curve_audio": [0.01, 0.02, 0.03, 0.04, 0.05, 0.06, 0.07, 0.08],
        });
        let g = assess_mint_gate(&fields, None);
        assert_eq!(g["silicon_mint_ok"], false, "{g}");
        assert_eq!(g["residual_mint_ok"], false, "{g}");
    }

    #[test]
    fn multi_source_agree_cores_mint_ok() {
        let fields = json!({"hardware_concurrency": 12});
        let evidence = json!({
            "fields_by_source": {
                "main": {"hardware_concurrency": 12},
                "iframe": {"hardware_concurrency": 12},
                "worker": {"hardware_concurrency": 12}
            },
            "source_conflicts": []
        });
        let d = field_mint_decision(
            "hardware_concurrency",
            &fields,
            evidence
                .get("fields_by_source")
                .and_then(|v| v.as_object()),
            &[],
        );
        assert_eq!(d["mint_ok"], true);
        assert_eq!(d["source_count"], 3);
    }

    #[test]
    fn single_source_texture_caps_mint_ok_and_resolve() {
        // OSS-style: one main WebGL getParameter — must be usable for class:t* K.
        let fields = json!({});
        let evidence = json!({
            "fields_by_source": {
                "main": {"gl_max_texture_size": 16384, "hardware_concurrency": 12}
            },
            "source_conflicts": []
        });
        let fbs = evidence
            .get("fields_by_source")
            .and_then(|v| v.as_object());
        let d = field_mint_decision("gl_max_texture_size", &fields, fbs, &[]);
        assert_eq!(d["mint_ok"], true, "{d}");
        assert_eq!(d["conf_only"], false, "{d}");
        let out = resolve_fields_multi_source(&fields, Some(&evidence));
        let rf = out
            .get("resolved_fields")
            .cloned()
            .unwrap_or(out.clone());
        assert_eq!(rf.get("gl_max_texture_size"), Some(&json!(16384)), "{out}");
    }

    #[test]
    fn soft_conflict_prefers_worker_not_main() {
        let fields = json!({"platform": "Win32"});
        let evidence = json!({
            "fields_by_source": {
                "main": {"platform": "Win32"},
                "worker:d1": {"platform": "Linux x86_64"},
                "iframe:d1": {"platform": "Linux x86_64"}
            },
            "source_conflicts": ["source_conflict:platform"]
        });
        let d = field_mint_decision_ex(
            "platform",
            &fields,
            evidence
                .get("fields_by_source")
                .and_then(|v| v.as_object()),
            &["source_conflict:platform".into()],
            Some(&evidence),
        );
        assert_eq!(d["conflicted"], true);
        // Harder-to-forge worker wins over spoofed main Win32
        assert_eq!(d["chosen_source"], "worker:d1");
        assert_eq!(d["mint_ok"], true);
        assert_eq!(d["policy"], "prefer_harder_to_forge_on_conflict");
    }

    #[test]
    fn silicon_conflict_still_excludes_mint() {
        let fields = json!({
            "hw_curve_webgl": [0.1, 0.2],
        });
        let evidence = json!({
            "fields_by_source": {
                "main": {"hw_curve_webgl": [0.1, 0.2, 0.3]},
                "iframe:d1": {"hw_curve_webgl": [0.9, 0.8, 0.7]}
            },
            "source_conflicts": []
        });
        let d = field_mint_decision_ex(
            "hw_curve_webgl",
            &fields,
            evidence
                .get("fields_by_source")
                .and_then(|v| v.as_object()),
            &[],
            Some(&evidence),
        );
        assert_eq!(d["conflicted"], true);
        assert_eq!(d["mint_ok"], false);
        assert!(d["reason"]
            .as_str()
            .unwrap_or("")
            .contains("silicon_conflict"));
    }

    #[test]
    fn b34_and_h2_priority_never_commercial_mint() {
        for key in [
            "cpu_cache_ladder",
            "cpu_cache_knee_bytes",
            "h2_priority_fingerprint",
            "h3_pseudo_order",
            "governed_webgl_renderer",
        ] {
            let d = field_mint_decision(key, &json!({key: "x"}), None, &[]);
            assert_eq!(d["mint_ok"], false, "{key}");
            assert_eq!(d["reason"], "never_commercial_mint_key", "{key}");
        }
    }

    #[test]
    fn residual_paths_union_survives_deep_last_write() {
        let noderiv = json!({
            "path_id": "v3f_128_noderiv",
            "shader_mode": "noderiv",
            "mean": 0.2611,
            "std": 0.09,
            "ok": true,
            "entropy_ok": true,
            "curve": (0..32).map(|i| 0.229 + (i as f64) * 0.001).collect::<Vec<f64>>()
        });
        let fma = json!({
            "path_id": "fma_pair_webgl1",
            "shader_mode": "fma_pair",
            "mean": 0.255,
            "ok": true,
            "curve": (0..16).map(|i| 0.2 + (i as f64) * 0.001).collect::<Vec<f64>>()
        });
        // Top-level = evidence_merge union (Lane-C + Lane-S)
        let fields = json!({
            "residual_mean": 0.2603659375,
            "residual_paths": [noderiv.clone(), fma.clone()],
            "residual_paths_n": 2,
        });
        // fields_by_source.main = last B10x_silicon_deep pack only
        let evidence = json!({
            "fields_by_source": {
                "main": {
                    "residual_mean": 0.2603659375,
                    "residual_paths": [fma],
                    "residual_paths_n": 1,
                    "b10x_pack": "B10x_silicon_deep",
                }
            },
            "source_conflicts": []
        });
        let out = resolve_fields_multi_source(&fields, Some(&evidence));
        let rp = out["resolved_fields"]["residual_paths"]
            .as_array()
            .expect("residual_paths present");
        assert!(
            rp.iter().any(|p| p.get("shader_mode").and_then(|v| v.as_str()) == Some("noderiv")),
            "unioned noderiv must survive deep last-write: {out}"
        );
        assert_eq!(out["algo"], "resolve_fields_multi_source_v2");
    }
}
