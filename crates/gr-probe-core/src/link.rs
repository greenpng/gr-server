//! Device-link decisions — default = sparse_safe (v9-aligned).

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct LinkResult {
    pub decision: String,
    pub confidence: f64,
    pub hard_matches: i32,
    pub matched: Vec<String>,
    pub mismatched: Vec<String>,
    pub missing: Vec<String>,
    pub vetoes: Vec<String>,
    pub algo: String,
    pub details: Value,
}

impl LinkResult {
    pub fn to_value(&self) -> Value {
        json!({
            "decision": self.decision,
            "confidence": self.confidence,
            "hard_matches": self.hard_matches,
            "matched": self.matched,
            "mismatched": self.mismatched,
            "missing": self.missing,
            "vetoes": self.vetoes,
            "algo": self.algo,
            "details": self.details,
        })
    }
}

const HARD_FIELDS: &[&str] = &[
    "os_family",
    "hardware_concurrency",
    "screen_width",
    "screen_height",
    "timezone",
    "webgl_unmasked_renderer",
    "form_class",
];

const WEIGHTS: &[(&str, f64)] = &[
    ("os_family", 12.0),
    ("hardware_concurrency", 14.0),
    ("screen_width", 10.0),
    ("screen_height", 10.0),
    ("timezone", 9.0),
    ("webgl_unmasked_renderer", 18.0),
    ("form_class", 5.0),
    ("device_memory", 10.0),
    ("platform", 8.0),
];

fn norm(v: Option<&Value>) -> Option<Value> {
    match v {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(Value::String(t.to_string()))
            }
        }
        Some(other) => Some(other.clone()),
    }
}

fn values_eq(a: &Value, b: &Value) -> bool {
    // numeric equality across int/float
    if let (Some(na), Some(nb)) = (a.as_f64(), b.as_f64()) {
        return (na - nb).abs() < f64::EPSILON;
    }
    a == b
}

/// API / backend stopwords stripped from WebGL renderer tokens.
/// Never drop GPU-model vocabulary (radeon/geforce/mali/…); only wrappers.
fn webgl_stopword(tok: &str) -> bool {
    if matches!(
        tok,
        "angle"
            | "google"
            | "inc"
            | "corporation"
            | "direct3d"
            | "direct3d11"
            | "direct3d12"
            | "opengl"
            | "opengles"
            | "vulkan"
            | "d3d11"
            | "d3d12"
            | "vs"
            | "ps"
            | "mesa"
            | "compat"
            | "core"
            | "es"
            | "glsl"
            | "renderer"
            | "unmasked"
            | "device" // "SwiftShader Device" → keep swiftshader/subzero
    ) {
        return true;
    }
    // Dotted API versions only (1.3.0) — keep pure model numbers 580/630/1660 (len≥3, no '.')
    if tok.contains('.') && tok.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return true;
    }
    // Short pure digits (1, 2) are noise; model numbers are len>=3 and kept by alnum_tokens filter
    if tok.len() <= 2 && tok.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    // PCI / adapter ids: 0x67df, 0x000067df — browser-specific noise
    if tok.starts_with("0x") && tok.len() > 2 && tok[2..].chars().all(|c| c.is_ascii_hexdigit()) {
        return true;
    }
    false
}

fn extract_paren_groups(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'(' {
            let mut depth = 0i32;
            let start = i + 1;
            let mut j = i;
            while j < bytes.len() {
                if bytes[j] == b'(' {
                    depth += 1;
                } else if bytes[j] == b')' {
                    depth -= 1;
                    if depth == 0 {
                        if start < j {
                            out.push(s[start..j].to_string());
                        }
                        break;
                    }
                }
                j += 1;
            }
            i = j.saturating_add(1);
        } else {
            i += 1;
        }
    }
    out
}

fn alnum_tokens(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| t.len() >= 3)
        .map(|t| t.to_ascii_lowercase())
        .filter(|t| !webgl_stopword(t))
        .collect()
}

/// Normalize WebGL renderer for compare: peel wrappers, drop API/backend tokens,
/// **keep** GPU model tokens that appear after Vulkan/Direct3D markers
/// (e.g. ANGLE (Google, Vulkan 1.3.0 (AMD Radeon RX 580)) → amd radeon …).
pub fn normalize_webgl_renderer(s: &str) -> String {
    let raw = s.trim();
    if raw.is_empty() {
        return String::new();
    }
    // Candidates: full string + every parenthetical group (deepest models last).
    let mut candidates = vec![raw.to_string()];
    candidates.extend(extract_paren_groups(raw));

    // Score candidates by residual GPU tokens (prefer rich model strings).
    let mut best: Vec<String> = Vec::new();
    let mut best_score = -1i32;
    for c in &candidates {
        let toks = alnum_tokens(c);
        // GPU signal: non-empty residual after stopword strip
        let score = toks.len() as i32
            + if toks.iter().any(|t| {
                t.contains("radeon")
                    || t.contains("geforce")
                    || t.contains("nvidia")
                    || t.contains("amd")
                    || t.contains("intel")
                    || t.contains("mali")
                    || t.contains("adreno")
                    || t.contains("apple")
                    || t.contains("swiftshader")
                    || t.contains("llvmpipe")
                    || t.contains("quadro")
                    || t.contains("iris")
                    || t.contains("uhd")
                    || t.contains("rtx")
                    || t.contains("gtx")
            }) {
                10
            } else {
                0
            };
        if score > best_score {
            best_score = score;
            best = toks;
        }
    }
    // Fallback: whole-string tokens if nothing scored
    if best.is_empty() {
        best = alnum_tokens(raw);
    }
    best.join(" ")
}

/// Soft-stable GPU key for **both** soft-match and commercial device_id hashing.
/// Sorted unique tokens (incl. pure model digits len≥3, e.g. 580/630/1660).
pub fn canonical_webgl_renderer(s: &str) -> String {
    let n = normalize_webgl_renderer(s);
    if n.is_empty() {
        return String::new();
    }
    let mut toks: Vec<String> = n.split_whitespace().map(|t| t.to_string()).collect();
    toks.sort();
    toks.dedup();
    toks.join(" ")
}

/// Single pure GPU key — only equality counts as same GPU (no Jaccard partial match).
pub fn gpu_key(renderer: &str) -> String {
    canonical_webgl_renderer(renderer)
}

/// Same GPU iff non-empty gpu_keys are equal. Empty never matches.
fn webgl_soft_match(a: &str, b: &str) -> bool {
    let ka = gpu_key(a);
    let kb = gpu_key(b);
    !ka.is_empty() && ka == kb
}

/// Offline FE core (when no server edge IP): os + cores + timezone.
/// WebKit may report different concurrency than Chromium on the same host — prefer
/// server_client_ip path below whenever gateway observed the visitor.
#[allow(dead_code)] // kept: algorithm reference / .so-module variant (iss/audit WARN-01: silence, do not remove)
const MACHINE_ID_CORE_KEYS_FE: &[&str] = &["os_family", "hardware_concurrency", "timezone"];

/// Edge-joined core (GA4/AdSense-style): same egress IP + OS + timezone.
/// Stable across Chromium/Firefox/WebKit even when GPU path or cores report differs.
#[allow(dead_code)] // kept: algorithm reference / .so-module variant (iss/audit WARN-01: silence, do not remove)
const MACHINE_ID_CORE_KEYS_EDGE: &[&str] = &["os_family", "timezone", "server_client_ip"];

/// Soft machine materials — collected for LINK confidence / diagnostics when present on both sides.
/// Never change commercial device_id digest (avoids Chrome-only fields splitting multi-browser ids).
#[allow(dead_code)] // kept: algorithm reference / .so-module variant (iss/audit WARN-01: silence, do not remove)
const MACHINE_ID_SOFT_KEYS: &[&str] = &[
    "device_memory",
    "audio_sample_rate",
    "color_depth",
    "max_touch_points",
    "server_client_ip",
    "server_asn",
    "screen_avail_width",
    "screen_avail_height",
    "architecture",
    "storage_quota_class",
    "speech_voices_count",
    "media_input_count",
    "media_output_count",
    "webrtc_host_ip_hash",
];

/// Full material key set for machine_materials() projection (core + soft).
const MACHINE_ID_ALL_KEYS: &[&str] = &[
    "os_family",
    "hardware_concurrency",
    "timezone",
    "device_memory",
    "audio_sample_rate",
    "color_depth",
    "max_touch_points",
    "server_client_ip",
    "server_asn",
    "screen_avail_width",
    "screen_avail_height",
    "architecture",
    "storage_quota_class",
    "speech_voices_count",
    "media_input_count",
    "media_output_count",
    "webrtc_host_ip_hash",
];

/// Browser-surface keys (GPU path) — diagnostics / same-browser stability only.
const SURFACE_ID_HASH_KEYS: &[&str] = &[
    "webgl_unmasked_renderer",
    "webgl_max_texture",
    "webgl_ext_hash",
    "canvas_2d_hash",
];

fn norm_for_device_id(key: &str, v: &Value) -> Option<String> {
    match key {
        "webgl_unmasked_renderer" => v
            .as_str()
            .map(gpu_key)
            .filter(|s| !s.is_empty()),
        "os_family" | "timezone" | "server_client_ip" | "server_asn" | "webgl_ext_hash"
        | "canvas_2d_hash" | "architecture" | "storage_quota_class" | "webrtc_host_ip_hash" => v
            .as_str()
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty()),
        "hardware_concurrency" | "device_memory" | "audio_sample_rate" | "color_depth"
        | "max_touch_points" | "webgl_max_texture" | "screen_width" | "screen_height"
        | "screen_avail_width" | "screen_avail_height" | "speech_voices_count"
        | "media_input_count" | "media_output_count" => v
            .as_i64()
            .or_else(|| v.as_f64().map(|f| f as i64))
            .map(|n| n.to_string()),
        _ => None,
    }
}

fn backfill_os_family(fo: &Map<String, Value>) -> Option<String> {
    if let Some(s) = fo
        .get("os_family")
        .and_then(|v| v.as_str())
        .map(|s| s.to_ascii_lowercase())
        .filter(|s| !s.is_empty())
    {
        return Some(s);
    }
    // Prefer platform (honest on Playwright WebKit) over UA (often spoofed Macintosh).
    let platform = fo
        .get("platform")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let ua = fo
        .get("user_agent")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if platform.contains("android") {
        return Some("android".into());
    }
    if platform.contains("iphone") || platform.contains("ipad") {
        return Some("ios".into());
    }
    if platform.contains("win") {
        return Some("windows".into());
    }
    if platform.contains("linux") || platform.contains("x11") {
        return Some("linux".into());
    }
    if platform.contains("mac") {
        return Some("macos".into());
    }
    if ua.contains("android") {
        Some("android".into())
    } else if ua.contains("iphone") || ua.contains("ipad") {
        Some("ios".into())
    } else if ua.contains("windows") || ua.contains("win32") {
        Some("windows".into())
    } else if ua.contains("linux") || ua.contains("x11") {
        Some("linux".into())
    } else if ua.contains("mac") {
        Some("macos".into())
    } else {
        None
    }
}

fn hash_keys(materials: &Map<String, Value>, keys: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for k in keys {
        if let Some(v) = materials.get(*k) {
            hasher.update(k.as_bytes());
            hasher.update(b"=");
            hasher.update(v.to_string().as_bytes());
            hasher.update(b"|");
        }
    }
    format!("{:x}", hasher.finalize())
}

/// Collect machine materials (cross-browser same host): core + soft probes.
pub fn machine_materials(fields: &Value) -> Map<String, Value> {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let mut materials = Map::new();
    if let Some(o) = backfill_os_family(&fo) {
        materials.insert("os_family".into(), json!(o));
    }
    // server_* may live at evidence root or fields
    for k in MACHINE_ID_ALL_KEYS {
        if *k == "os_family" {
            continue;
        }
        if let Some(v) = fo.get(*k).or_else(|| fields.get(*k)) {
            if let Some(n) = norm_for_device_id(k, v) {
                materials.insert((*k).into(), json!(n));
            }
        }
    }
    materials
}

/// Commercial device_id = **trust-gated machine-stable** projection (`machine_trust_v2`).
/// High-trust anchors: live hardware-noise curve digests + WebRTC host hash.
/// Does **not** hard-split on `server_client_ip` / UA / WebGL string alone.
pub fn commercial_device_id_from_fields(fields: &Value) -> Option<String> {
    // Prefer already-minted id on the vector (evaluate injects device_id for link).
    if let Some(id) = fields
        .get("device_id")
        .or_else(|| fields.get("commercial_device_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| crate::device_tier::is_commercial_device_id(s))
    {
        return Some(id);
    }
    crate::trust::commercial_device_id_trusted(fields)
}

/// Cross-engine machine link id (not commercial device_id) — stable hash of machine core.
pub fn machine_link_id_from_pair(a: &Value, b: &Value) -> Option<String> {
    let ma = machine_materials(a);
    let mb = machine_materials(b);
    if ma.is_empty() || mb.is_empty() {
        return None;
    }
    let mut keys: Vec<String> = ma.keys().chain(mb.keys()).cloned().collect();
    keys.sort();
    keys.dedup();
    let mut hasher = Sha256::new();
    for k in &keys {
        let va = ma.get(k).map(|v| v.to_string()).unwrap_or_default();
        let vb = mb.get(k).map(|v| v.to_string()).unwrap_or_default();
        // only include keys present and equal on both sides
        if !va.is_empty() && va == vb {
            hasher.update(k.as_bytes());
            hasher.update(b"=");
            hasher.update(va.as_bytes());
            hasher.update(b"|");
        }
    }
    let dig = format!("{:x}", hasher.finalize());
    if dig.chars().all(|c| c == '0') {
        return None;
    }
    Some(format!("ml_{}", &dig[..16]))
}

/// Browser surface id (GPU/canvas path) for diagnostics — not commercial identity.
pub fn browser_surface_id_from_fields(fields: &Value) -> Option<String> {
    let fo = fields.as_object()?;
    let mut materials = Map::new();
    for k in SURFACE_ID_HASH_KEYS {
        if let Some(v) = fo.get(*k) {
            if let Some(n) = norm_for_device_id(k, v) {
                materials.insert((*k).into(), json!(n));
            }
        }
    }
    if materials.is_empty() {
        return None;
    }
    let digest = hash_keys(&materials, SURFACE_ID_HASH_KEYS);
    Some(format!("bs_{}", &digest[..16]))
}

/// Structural invariant: strong LINK (same commercial id) vs MACHINE_BOUND (cross-engine same host).
/// - LINK: commercial device_ids present and equal
/// - MACHINE_BOUND: machine-core hard match without same dh_ (cross-browser association)
/// - POSSIBLE: weaker evidence
fn enforce_link_same_device_id(decision: String, a: &Value, b: &Value) -> String {
    if decision != "LINK" && decision != "MACHINE_BOUND" {
        return decision;
    }
    match (
        commercial_device_id_from_fields(a),
        commercial_device_id_from_fields(b),
    ) {
        (Some(ida), Some(idb)) if ida == idb => "LINK".into(),
        // Cross-engine: different commercial digests, keep MACHINE_BOUND if caller set it.
        (Some(_), Some(_)) if decision == "MACHINE_BOUND" => "MACHINE_BOUND".into(),
        (Some(_), Some(_)) => "MACHINE_BOUND".into(),
        // One side minted — association pending peer mint, not UNRELATED.
        (Some(_), None) | (None, Some(_)) => {
            if decision == "LINK" {
                "MACHINE_BOUND".into()
            } else {
                decision
            }
        }
        _ => {
            if decision == "LINK" {
                "POSSIBLE".into()
            } else {
                decision
            }
        }
    }
}

fn platform_family(p: &str) -> &'static str {
    let t = p.to_ascii_lowercase();
    if t.contains("win") {
        "windows"
    } else if t.contains("mac") || t.contains("iphone") || t.contains("ipad") {
        "apple"
    } else if t.contains("linux") || t.contains("android") || t.contains("x11") {
        "linuxoid"
    } else {
        "other"
    }
}

struct CompareOut {
    matched: Vec<String>,
    mismatched: Vec<String>,
    missing: Vec<String>,
    vetoes: Vec<String>,
    conf: f64,
    hard: i32,
}

fn compare_pair(a: &Map<String, Value>, b: &Map<String, Value>) -> CompareOut {
    // Fields should already be prepare_link_fields-stripped by associate_* entry points.
    let a = a;
    let b = b;
    let mut matched = Vec::new();
    let mut mismatched = Vec::new();
    let mut missing = Vec::new();
    let mut vetoes = Vec::new();
    let mut score = 0.0;
    let mut max_score = 0.0;
    let mut hard = 0i32;

    // When both sides expose stack_class, require agreement for GPU soft-match credit.
    let stack_a = a
        .get("stack_class")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let stack_b = b
        .get("stack_class")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let stack_ok = stack_a.is_empty()
        || stack_b.is_empty()
        || stack_a == "unknown"
        || stack_b == "unknown"
        || stack_a == stack_b;
    if !stack_ok && !stack_a.is_empty() && !stack_b.is_empty() {
        vetoes.push("stack_class_mismatch".into());
    }
    // Soft stacks: require renderer_class agreement (SwiftShader must not LINK llvmpipe).
    let rc_a = a
        .get("renderer_class")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let rc_b = b
        .get("renderer_class")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let both_soft = (stack_a == "soft_render" || crate::stack_auth::is_soft_renderer_class(rc_a))
        && (stack_b == "soft_render" || crate::stack_auth::is_soft_renderer_class(rc_b));
    if both_soft
        && !rc_a.is_empty()
        && !rc_b.is_empty()
        && rc_a != "unknown"
        && rc_b != "unknown"
        && rc_a != rc_b
    {
        vetoes.push("renderer_class_mismatch_soft".into());
    }

    for (name, w) in WEIGHTS {
        max_score += w;
        let va = norm(a.get(*name));
        let vb = norm(b.get(*name));
        match (va, vb) {
            (None, _) | (_, None) => missing.push((*name).to_string()),
            (Some(va), Some(vb)) if values_eq(&va, &vb) => {
                // GPU exact match only when stack classes compatible
                if *name == "webgl_unmasked_renderer" && !stack_ok {
                    mismatched.push((*name).to_string());
                    continue;
                }
                matched.push((*name).to_string());
                score += w;
                if HARD_FIELDS.contains(name) {
                    hard += 1;
                }
            }
            (Some(va), Some(vb))
                if *name == "hardware_concurrency"
                    && va.as_i64().or_else(|| va.as_f64().map(|f| f as i64)).zip(
                        vb.as_i64().or_else(|| vb.as_f64().map(|f| f as i64)),
                    ).is_some_and(|(ia, ib)| {
                        // WebKit often under-reports vs Chromium/Firefox on same host (8 vs 12).
                        // Soft match when close — not a fixed sample table.
                        let d = (ia - ib).abs();
                        d > 0 && d <= 4
                    }) =>
            {
                matched.push((*name).to_string());
                score += w * 0.9;
                hard += 1;
            }
            (Some(va), Some(vb))
                if *name == "webgl_unmasked_renderer"
                    && stack_ok
                    && va.as_str().zip(vb.as_str()).is_some_and(|(sa, sb)| webgl_soft_match(sa, sb)) =>
            {
                // Cross-browser GPU string variance: soft match counts as hard match.
                matched.push((*name).to_string());
                score += w;
                hard += 1;
            }
            (Some(va), Some(vb))
                if *name == "platform"
                    && va.as_str().zip(vb.as_str()).is_some_and(|(sa, sb)| {
                        platform_family(sa) == platform_family(sb) && platform_family(sa) != "other"
                    }) =>
            {
                matched.push((*name).to_string());
                score += w * 0.85;
            }
            (Some(va), Some(vb)) => {
                mismatched.push((*name).to_string());
                if *name == "os_family" {
                    vetoes.push("os_family_mismatch".into());
                }
                if *name == "form_class" {
                    let set: HashSet<String> = [va.clone(), vb.clone()]
                        .into_iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect();
                    if set.contains("desktop") && set.contains("mobile") {
                        vetoes.push("form_class_desktop_vs_mobile".into());
                    }
                }
                if *name == "webgl_unmasked_renderer" {
                    // Hard-veto only when both sides report *identifiable real* GPUs that disagree.
                    // Software/virtual/generic browser masks differ by kernel path on the same host
                    // (SwiftShader, Apple GPU string on Linux WebKit, llvmpipe, …) — never hard-block
                    // multi-browser machine association. Physical multi-GPU clash still vetoes.
                    let sa = va.as_str().unwrap_or("");
                    let sb = vb.as_str().unwrap_or("");
                    let ka = gpu_key(sa);
                    let kb = gpu_key(sb);
                    let soft_or_generic = |k: &str| {
                        let t = k.to_ascii_lowercase();
                        t.is_empty()
                            || t.contains("swiftshader")
                            || t.contains("llvmpipe")
                            || t.contains("softpipe")
                            || t.contains("subzero")
                            || t.contains("microsoft basic")
                            || t.contains("virtualbox")
                            || t.contains("virgl")
                            || t.contains("vmware")
                            || t.contains("parallels")
                            || t.contains("qemu")
                            || t.contains("gallium")
                            || t.contains("gdi generic")
                            || t == "apple gpu"
                            || t == "apple"
                            || t == "webkit"
                            || t == "mozilla"
                            || t == "google"
                    };
                    if !soft_or_generic(&ka) && !soft_or_generic(&kb) {
                        vetoes.push("gpu_model_mismatch".into());
                    }
                }
                if *name == "hardware_concurrency" {
                    if let (Some(ia), Some(ib)) = (
                        va.as_i64().or_else(|| va.as_f64().map(|f| f as i64)),
                        vb.as_i64().or_else(|| vb.as_f64().map(|f| f as i64)),
                    ) {
                        if (ia - ib).abs() >= 8 {
                            vetoes.push("cpu_cores_far_apart".into());
                        }
                    }
                }
            }
        }
    }
    let conf = if max_score > 0.0 {
        score / max_score
    } else {
        0.0
    };
    CompareOut {
        matched,
        mismatched,
        missing,
        vetoes,
        conf,
        hard,
    }
}

fn as_map(obj: &Value) -> Map<String, Value> {
    obj.as_object().cloned().unwrap_or_default()
}

/// Ensure machine materials usable for compare (os_family from platform when omitted).
/// Also strips untrusted GPU labels via stack_auth (demo→v5) so spoof cannot strengthen LINK.
fn prepare_link_fields(fields: &Value) -> Value {
    let mut m = as_map(fields);
    if !m
        .get("os_family")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty())
    {
        if let Some(os) = backfill_os_family(&m) {
            m.insert("os_family".into(), json!(os));
        }
    }
    let auth = crate::stack_auth::stack_auth_from_fields(&Value::Object(m.clone()));
    if auth.gpu_label_untrusted {
        m.remove("webgl_unmasked_renderer");
        m.remove("webgl_unmasked_vendor");
        m.insert("gpu_label_untrusted".into(), json!(true));
        m.insert("stack_class".into(), json!(auth.stack_class));
        m.insert("spoof_score".into(), json!(auth.spoof_score));
        m.insert("renderer_class".into(), json!(auth.renderer_class));
    } else if !auth.stack_class.is_empty() && auth.stack_class != "unknown" {
        m.insert("stack_class".into(), json!(auth.stack_class));
        m.insert("renderer_class".into(), json!(auth.renderer_class));
    }
    Value::Object(m)
}

pub fn associate_baseline(a: &Value, b: &Value) -> LinkResult {
    let a = prepare_link_fields(a);
    let b = prepare_link_fields(b);
    let c = compare_pair(&as_map(&a), &as_map(&b));
    let hard_vetoes = [
        "os_family_mismatch",
        "form_class_desktop_vs_mobile",
        "gpu_model_mismatch",
        "cpu_cores_far_apart",
    ];
    let mut decision = if c.vetoes.iter().any(|v| hard_vetoes.contains(&v.as_str())) {
        "UNRELATED".into()
    } else if c.conf >= 0.62 && c.hard >= 3 && c.vetoes.is_empty() {
        "LINK".into()
    } else if c.conf >= 0.40 && c.hard >= 2 {
        "POSSIBLE".into()
    } else {
        "UNRELATED".into()
    };
    // Structural: LINK ⇒ same commercial device_id (closes residual diff_id+LINK).
    decision = enforce_link_same_device_id(decision, &a, &b);
    LinkResult {
        decision,
        confidence: (c.conf * 10000.0).round() / 10000.0,
        hard_matches: c.hard,
        matched: c.matched,
        mismatched: c.mismatched,
        missing: c.missing,
        vetoes: c.vetoes,
        algo: "baseline".into(),
        details: json!({
            "hard_matches": c.hard,
            "device_id_a": commercial_device_id_from_fields(&a),
            "device_id_b": commercial_device_id_from_fields(&b),
        }),
    }
}

pub fn associate_sparse_safe(a: &Value, b: &Value) -> LinkResult {
    let a = prepare_link_fields(a);
    let b = prepare_link_fields(b);
    let base = associate_baseline(&a, &b);
    let hard = base.hard_matches;
    let mut vetoes = base.vetoes.clone();
    let has_gpu = base.matched.iter().any(|m| m == "webgl_unmasked_renderer");
    let has_geo = base.matched.iter().any(|m| m == "screen_width")
        && base.matched.iter().any(|m| m == "screen_height")
        && base.matched.iter().any(|m| m == "timezone");
    let has_cpu = base.matched.iter().any(|m| m == "hardware_concurrency");
    let has_machine_core = base.matched.iter().any(|m| m == "os_family")
        && has_cpu
        && base.matched.iter().any(|m| m == "timezone");
    let same_device = match (
        commercial_device_id_from_fields(&a),
        commercial_device_id_from_fields(&b),
    ) {
        (Some(ida), Some(idb)) if ida == idb => true,
        _ => false,
    };
    let mut decision = base.decision.clone();
    // Multi-browser same host: software GPU path often differs (SwiftShader vs real).
    // When commercial device_ids already equal and machine core matched, do not require GPU.
    // baseline already ran enforce_link_same_device_id; re-apply sparse gates then re-enforce.
    if decision == "LINK" && !has_gpu && !(has_geo && has_cpu) && !(same_device && has_machine_core) {
        decision = if hard >= 2 {
            "POSSIBLE".into()
        } else {
            "UNRELATED".into()
        };
        if !has_gpu && !has_geo && !vetoes.iter().any(|v| v == "sparse_pair") {
            vetoes.push("sparse_pair".into());
        }
    } else if decision == "LINK" && (base.confidence < 0.65 || hard < 3) {
        // Soften demotion when same machine-stable commercial id is already proven.
        if !(same_device && has_machine_core && hard >= 2) {
            decision = if hard >= 2 {
                "POSSIBLE".into()
            } else {
                "UNRELATED".into()
            };
        }
    }
    // Hard vetoes still block (real GPU clash, os mismatch, desktop vs mobile).
    let hard_block = vetoes.iter().any(|v| {
        matches!(
            v.as_str(),
            "os_family_mismatch"
                | "form_class_desktop_vs_mobile"
                | "gpu_model_mismatch"
                | "cpu_cores_far_apart"
        )
    });
    // Same commercial device_id is the multi-browser join key (edge IP / machine core).
    // Screen/GPU surface may diverge (Playwright viewport, SwiftShader vs real) — still LINK.
    if !hard_block && same_device && has_machine_core {
        decision = "LINK".into();
    } else if !hard_block && same_device && hard >= 2 {
        decision = "LINK".into();
    } else if !hard_block && decision == "POSSIBLE" && same_device && hard >= 2 {
        decision = "LINK".into();
    } else if !hard_block
        && !same_device
        && has_machine_core
        && hard >= 5
        && (has_gpu || has_geo)
    {
        // Soft-GL (SwiftShader/llvmpipe) vs real discrete GPU: machine-core may match
        // (os/cores/tz) but commercial floors differ — cap at POSSIBLE, never MACHINE_BOUND.
        let soft_gl = |f: &Value| -> bool {
            let r = f
                .get("webgl_unmasked_renderer")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            r.contains("swiftshader")
                || r.contains("llvmpipe")
                || r.contains("softpipe")
                || r.contains("microsoft basic render")
                || r.contains("gdi generic")
        };
        if soft_gl(&a) != soft_gl(&b) && !has_gpu {
            decision = "POSSIBLE".into();
        } else {
            // Cross-engine same host: commercial digests differ by design; bind machine graph.
            decision = "MACHINE_BOUND".into();
        }
    } else if !hard_block && !same_device && has_machine_core && hard >= 7 {
        decision = "MACHINE_BOUND".into();
    }
    decision = enforce_link_same_device_id(decision, &a, &b);
    let id_a = commercial_device_id_from_fields(&a);
    let id_b = commercial_device_id_from_fields(&b);
    let machine_link = if decision == "MACHINE_BOUND" || decision == "LINK" {
        machine_link_id_from_pair(&a, &b)
    } else {
        None
    };
    let device_link_bound = decision == "LINK" || decision == "MACHINE_BOUND";
    LinkResult {
        decision,
        confidence: base.confidence,
        hard_matches: hard,
        matched: base.matched,
        mismatched: base.mismatched,
        missing: base.missing,
        vetoes,
        algo: "sparse_safe".into(),
        details: json!({
            "hard_matches": hard,
            "has_gpu_match": has_gpu,
            "has_geo_match": has_geo,
            "has_cpu_match": has_cpu,
            "has_machine_core": has_machine_core,
            "same_device_id": same_device,
            "gpu_key_a": a.get("webgl_unmasked_renderer").and_then(|v| v.as_str()).map(gpu_key),
            "gpu_key_b": b.get("webgl_unmasked_renderer").and_then(|v| v.as_str()).map(gpu_key),
            "device_id_a": id_a,
            "device_id_b": id_b,
            "machine_link_id": machine_link,
            "device_link_bound": device_link_bound,
            "link_requires_same_device_id": false,
            "link_same_device_id_for_LINK": true,
        }),
    }
}

pub fn associate_loose(a: &Value, b: &Value) -> LinkResult {
    let a = prepare_link_fields(a);
    let b = prepare_link_fields(b);
    let c = compare_pair(&as_map(&a), &as_map(&b));
    let mut decision = if c
        .vetoes
        .iter()
        .any(|v| v == "os_family_mismatch" || v == "form_class_desktop_vs_mobile")
    {
        "UNRELATED".into()
    } else if c.conf >= 0.50 && c.hard >= 2 {
        "LINK".into()
    } else if c.conf >= 0.30 && c.hard >= 1 {
        "POSSIBLE".into()
    } else {
        "UNRELATED".into()
    };
    decision = enforce_link_same_device_id(decision, &a, &b);
    LinkResult {
        decision,
        confidence: (c.conf * 10000.0).round() / 10000.0,
        hard_matches: c.hard,
        matched: c.matched,
        mismatched: c.mismatched,
        missing: c.missing,
        vetoes: c.vetoes,
        algo: "loose".into(),
        details: json!({ "hard_matches": c.hard }),
    }
}

pub fn associate(a: &Value, b: &Value, algo: &str) -> Result<LinkResult, String> {
    match algo {
        "baseline" => Ok(associate_baseline(a, b)),
        "loose" => Ok(associate_loose(a, b)),
        "sparse_safe" => Ok(associate_sparse_safe(a, b)),
        other => Err(format!("unsupported link algo: {other}")),
    }
}

#[cfg(test)]
mod multi_segment_commercial_tests {
    use super::commercial_device_id_from_fields;
    use serde_json::json;

    #[test]
    fn pre_minted_multi_segment_id_is_kept() {
        let multi =
            "dv0-0.26-abc123def0-1111111111-2222222222-linux-x86_64-c3-UTC-oihash000-rtchash00";
        let fields = json!({
            "device_id": multi,
            // decoy materials that would mint a different legacy id if filter dropped multi
            "residual_mean": 0.99,
            "hw_curve_webgl": [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
        });
        let got = commercial_device_id_from_fields(&fields).expect("kept multi");
        assert_eq!(got, multi, "must not re-derive legacy hash when multi present");
    }

    #[test]
    fn commercial_device_id_key_also_accepts_multi() {
        let multi = "dv4-0.2600-wg00000000-au00000000-cp00000000-linux-x86_64-c3-UTC-0-0";
        let fields = json!({ "commercial_device_id": multi });
        let got = commercial_device_id_from_fields(&fields).expect("kept multi via commercial_device_id");
        assert_eq!(got, multi);
    }
}
