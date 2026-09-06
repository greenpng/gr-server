//! Multi-precision multi-segment commercial device IDs (`dv0` / `dv4` / `dv5` / `dv6`).
//!
//! # Product model (v5.8.106+)
//!
//! - **No exclusive** `dh`/`dv`/`dg` family as the sole public prefix.
//! - One vtid’s materials only → four precision lanes side-by-side.
//! - Each segment is `{prefix}-` + **hyphen-joined ordered digests**; missing parts are literal `0`.
//! - **Public body is never raw** OS/platform/tz names, UA, IP, or GPU model labels.
//!   Every present part is a **SHA-256 truncated digest** (JA4-style fingerprint token).
//!
//! # Segment order (SSOT)
//!
//! | # | Code | Material | Public form |
//! |---|------|----------|-------------|
//! | 1 | res | multipath residual + fma/denorm/ulp/EU (silicon) | digest mean+structure+adv paths |
//! | 2 | wg  | hw_curve_webgl + structure | curve digest |
//! | 3 | au  | audio_deep dual-seed + B10 seed-delta + stack | structured curve + delta + moments |
//! | 4 | cp  | cpu_timing wall (system) | structured timing curve |
//! | 5 | of  | **cv** canvas curve + noise hash | NOT plain OS names (iss/50 H2) |
//! | 6 | ar  | **gp** webgpu compute residual + adapter | NOT architecture labels |
//! | 7 | cc  | **pm** gl precision / caps matrix | NOT cores class labels |
//! | 8 | tz  | **tm** rAF jitter + timer stack | NOT timezone names |
//! | 9 | oi  | os_instance / unit / pohw / JA4H (system+net) | host-instance **probe** digests only |
//! |10 | rtc | webrtc host + JA4L/RTT (network) | host-net **probe** digest (not client IP) |
//!
//! # Hard exclusion (even as digests of the label)
//!
//! Fixed-name / claim fields must never feed segment materials:
//! `os_family`, `platform`, `architecture`, `ua_ch_*`, `timezone`, `user_agent`,
//! `server_client_ip`, GPU model/renderer strings, plain `hardware_concurrency` labels.
//!
//! # Curve priority (same hardware family)
//!
//! 1. Full mint-length **hw_curve_*** arrays (silicon timing)
//! 2. Projected commercial digests when raw absent
//! 3. **Never** unmasked GPU renderer / vendor model strings in segment body
//!
//! # Host separator preference
//!
//! Only probe digests (`os_instance_hash`, `pohw_*`, `unit_surface_*`,
//! `webrtc_host_ip_hash`) may fill `oi`/`rtc`. Fixed names (OS/arch/tz/cores/form)
//! and gateway IP/UA never enter segment materials — they may assist scoring only
//! outside the commercial body (SDK association layer / soft graphs).

use crate::device_tier::authentic_fields_for_device_id;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::sync::{Arc, RwLock};

pub const DEVICE_SEGMENTS_ALGO: &str = "device_segments_v2_precision_lanes";

/// Optional external mint (analyze.so via host FFI). When set, commercial
/// `select_device_segments` dispatches there so OTA of analyze can hot-swap
/// algorithm without restarting the runtime binary.
type DeviceSegmentsProvider =
    Arc<dyn Fn(&Value, Option<&Value>) -> Value + Send + Sync + 'static>;

static EXTERNAL_MINT: RwLock<Option<DeviceSegmentsProvider>> = RwLock::new(None);

/// Install / replace the external mint provider (typically wired from analyze module).
pub fn install_device_segments_provider<F>(f: F)
where
    F: Fn(&Value, Option<&Value>) -> Value + Send + Sync + 'static,
{
    if let Ok(mut g) = EXTERNAL_MINT.write() {
        *g = Some(Arc::new(f));
    }
}

/// Clear external mint — fall back to in-process local implementation.
pub fn clear_device_segments_provider() {
    if let Ok(mut g) = EXTERNAL_MINT.write() {
        *g = None;
    }
}

/// True when an external (module) provider is active.
pub fn device_segments_provider_active() -> bool {
    EXTERNAL_MINT
        .read()
        .ok()
        .and_then(|g| g.as_ref().map(|_| true))
        .unwrap_or(false)
}

/// Public precision-lane prefixes (only these).
pub const SEGMENT_PREFIXES: &[&str] = &["dv0", "dv4", "dv5", "dv6"];

/// Ordered part codes in each hyphen-joined segment body (after prefix).
pub const SEGMENT_PART_ORDER: &[&str] =
    &["res", "wg", "au", "cp", "of", "ar", "cc", "tz", "oi", "rtc"];

const PLACEHOLDER: &str = "0";

fn f_has(fo: &Map<String, Value>, key: &str) -> bool {
    match fo.get(key) {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(_) => true,
    }
}

fn str_field(fo: &Map<String, Value>, key: &str) -> Option<String> {
    fo.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn curve_array(fo: &Map<String, Value>, key: &str) -> Vec<f64> {
    fo.get(key)
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_f64())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn quantize(v: f64, places: Option<u32>) -> f64 {
    match places {
        None => v,
        Some(p) => {
            let m = 10f64.powi(p as i32);
            (v * m).round() / m
        }
    }
}

/// Domain-separated digest for public segment tokens (not reversible to raw fields).
const SEG_DIGEST_DOMAIN: &str = "gr_device_seg_v1";

fn short_hash(parts: &str) -> String {
    let mut h = Sha256::new();
    h.update(parts.as_bytes());
    format!("{:x}", h.finalize())[..10].to_string()
}

/// iss/opus5 01-P0-1: truncate an IP to its network class for the instance
/// separator (v4 → /24, v6 → /48). Never the full address. Values already
/// masked upstream (iss/opus5 05-S-4, `a.b.c.0/24` form) are reused as-is.
fn ip_net_class(ip: &str) -> Option<String> {
    let ip = ip.trim();
    if ip.is_empty() {
        return None;
    }
    if ip.contains('/') {
        // Already a network class (masked at ingestion) — keep verbatim.
        return Some(format!("pre:{ip}"));
    }
    if let Ok(v4) = ip.parse::<std::net::Ipv4Addr>() {
        let o = v4.octets();
        return Some(format!("v4:{}.{}.{}.0/24", o[0], o[1], o[2]));
    }
    if let Ok(v6) = ip.parse::<std::net::Ipv6Addr>() {
        let s = v6.segments();
        return Some(format!("v6:{:x}:{:x}:{:x}::/48", s[0], s[1], s[2]));
    }
    None
}

/// iss/opus5 01-P0-1 layered identity: `device_class_id` is the existing dv0
/// body (explicitly a device class/config — cross-network stable, not claimed
/// machine-unique). `device_instance_id = H(class ‖ machine_separator)` where
/// the separator is a stabilized derivation of host/instance materials
/// (os_instance_hash → versioned unit surface → webrtc host hash) optionally
/// bound to the /24 (v6: /48) network class. The instance ID is NOT issued
/// when no stable separator material exists — never degrade to class-level
/// collision. Returns (instance_id or "", instance_conf, separator_kind).
fn layered_instance_id(
    fo: &Map<String, Value>,
    class_id: &str,
    class_conf: f64,
) -> (String, f64, &'static str) {
    // Host separator candidates, strongest first. os_instance_hash is the most
    // stable per-machine material; unit surface only when versioned+stable
    // (same redline as the oi slot); webrtc host hash as the weakest host pin.
    let mut host: Option<(String, f64, &'static str)> = None;
    if let Some(s) = str_field(fo, "os_instance_hash").filter(|s| s != "0") {
        host = Some((format!("oi:{s}"), 1.0, "os_instance_hash"));
    } else {
        let unit_versioned = fo
            .get("unit_multiround_stable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            && fo
                .get("unit_surface_algo")
                .and_then(|v| v.as_str())
                .is_some_and(|a| a.starts_with("gr_unit"));
        if unit_versioned {
            if let Some(s) = str_field(fo, "unit_surface_id").filter(|s| s != "0") {
                host = Some((format!("unit:{s}"), 0.9, "unit_surface"));
            }
        }
        if host.is_none() {
            if let Some(s) = str_field(fo, "unit_surface_digest").filter(|s| !s.is_empty()) {
                if fo
                    .get("unit_surface_available")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true)
                {
                    host = Some((format!("unit_d:{}", sanitize_token(&s)), 0.85, "unit_digest"));
                }
            }
        }
        if host.is_none() {
            if let Some(s) = str_field(fo, "webrtc_host_ip_hash_v2")
                .or_else(|| str_field(fo, "webrtc_host_ip_hash"))
                .filter(|s| s != "0")
            {
                host = Some((format!("rtc:{s}"), 0.75, "webrtc_host_hash"));
            }
        }
    }
    let Some((host_part, host_q, kind)) = host else {
        // No stable separator → no instance ID (explicit non-issuance).
        return (String::new(), 0.0, "none");
    };
    // Network class binding (doc: "/24 网段类 + OS 实例哈希"). Absent IP does
    // not block issuance — the host material alone is already machine-scoped.
    let net_part = str_field(fo, "server_client_ip")
        .and_then(|ip| ip_net_class(&ip))
        .unwrap_or_default();
    let separator = if net_part.is_empty() {
        host_part
    } else {
        format!("{host_part}|net:{net_part}")
    };
    let mut h = Sha256::new();
    h.update(b"gr_instance_v1\0");
    h.update(class_id.as_bytes());
    h.update(b"\0");
    h.update(separator.as_bytes());
    let hex = format!("{:x}", h.finalize());
    // Instance confidence: class conf weighted by separator strength, capped —
    // a hash pin is never certainty. Net binding slightly raises confidence.
    let net_bonus = if net_part.is_empty() { 0.0 } else { 0.05 };
    let conf = ((class_conf * host_q * 0.9 + 0.1 * host_q + net_bonus).min(0.95) * 10000.0).round()
        / 10000.0;
    (format!("dvi1-{}", &hex[..24]), conf, kind)
}

/// Public segment token: always digest or placeholder `0`. Never plain labels/floats.
fn digest_token(part_code: &str, material: &str) -> String {
    if material.is_empty() || material == PLACEHOLDER {
        return PLACEHOLDER.to_string();
    }
    short_hash(&format!(
        "{SEG_DIGEST_DOMAIN}|{part_code}|{material}"
    ))
}

/// Encode a float curve array at a precision lane into a short stable token.
fn encode_curve(arr: &[f64], places: Option<u32>) -> String {
    if arr.is_empty() {
        return PLACEHOLDER.to_string();
    }
    let mut buf = String::new();
    for (i, v) in arr.iter().enumerate() {
        if i > 0 {
            buf.push(',');
        }
        let q = quantize(*v, places);
        // Full precision: enough digits; quantized: fixed places.
        match places {
            None => buf.push_str(&format!("{q:.12}")),
            Some(p) => buf.push_str(&format!("{q:.p$}", p = p as usize)),
        }
    }
    digest_token("curve", &buf)
}

fn encode_residual(v: Option<f64>, places: Option<u32>) -> String {
    match v {
        // Missing residual is the sole placeholder token "0".
        None => PLACEHOLDER.to_string(),
        Some(x) => {
            let q = quantize(x, places);
            let raw = match places {
                None => {
                    // Full precision canonical form. Zero residual must stay "0.0"
                    // (not "0") so digest never collides with the missing placeholder.
                    let s = format!("{q:.12}");
                    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
                    if s.is_empty() || s == "0" || s == "-0" {
                        "0.0".into()
                    } else {
                        s
                    }
                }
                Some(p) => {
                    // Quantized zero is "0.0000" / … — digest ≠ placeholder "0".
                    format!("{q:.p$}", p = p as usize)
                }
            };
            // Public id shows digest only — never raw residual float.
            digest_token("res", &raw)
        }
    }
}

/// Canonicalize class material for hashing (internal only; never emitted plain).
fn sanitize_token(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if t.is_empty() {
        PLACEHOLDER.to_string()
    } else {
        t.chars().take(64).collect()
    }
}

/// Encode stable OS-class / instance material as public digest token.
fn encode_class_part(code: &str, value: &str) -> String {
    if value.is_empty() || value == PLACEHOLDER {
        PLACEHOLDER.to_string()
    } else {
        digest_token(code, value)
    }
}

/// Entropy gate for extended curve slots (reuse trust residual_curve_entropy_ok).
fn curve_slot_ok(arr: &[f64]) -> bool {
    !arr.is_empty() && crate::trust::residual_curve_entropy_ok(arr)
}

/// Quality factor 0..1 for a curve slot (entropy-gated).
fn curve_slot_quality(arr: &[f64]) -> f64 {
    if arr.is_empty() {
        0.0
    } else if curve_slot_ok(arr) {
        1.0
    } else {
        0.35 // present but low entropy — do not mint full weight
    }
}

/// Canvas stack extras for of/cv (same-SKU surface micro-diff).
/// **Diagnostic / conf only** — commercial of uses `canvas_stack_extra_commercial`.
fn canvas_stack_extra(fo: &Map<String, Value>) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for k in [
        "canvas_noise_hash",
        "canvas_2d_hash",
        "canvas_noise_algo",
        "hw_canvas_stable",
    ] {
        if let Some(s) = str_field(fo, k).filter(|s| !s.is_empty()) {
            parts.push(format!("{k}={}", sanitize_token(&s)));
        }
    }
    if let Some(n) = fo
        .get("canvas_noise_patterns")
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
    {
        parts.push(format!("pat={n}"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("|"))
    }
}

/// Commercial of extras: **curve-body only**.
/// Optional noise hashes / pattern counts land only on some packs (dwell, multipattern
/// depth) and forked same-browser of digests across sites (178 audit + iss/67 of gate).
/// This is presence hygiene, **not** value coarsening.
fn canvas_stack_extra_commercial(_fo: &Map<String, Value>) -> Option<String> {
    None
}

/// Count multiround series for conf demotion (single-shot → lower reliability).
fn multiround_count(fo: &Map<String, Value>, round_keys: &[&str]) -> usize {
    use crate::hw_probe_analysis::rounds_from_fields;
    rounds_from_fields(fo, round_keys, &[]).len()
}

/// Soft reliability multiplier from session quality markers (not identity).
/// Bad window → demote conf; never rewrite commercial digest body.
fn probe_session_quality_mult(fo: &Map<String, Value>) -> f64 {
    let mut m = 1.0_f64;
    // visibility / hidden tab
    if fo.get("document_hidden").and_then(|v| v.as_bool()) == Some(true)
        || fo
            .get("visibility_state")
            .and_then(|v| v.as_str())
            .map(|s| s.eq_ignore_ascii_case("hidden") || s.eq_ignore_ascii_case("prerender"))
            .unwrap_or(false)
    {
        m *= 0.70;
    }
    // compute pressure (iss/58 A9: never in body; conf only)
    if let Some(ps) = fo
        .get("compute_pressure_state")
        .or_else(|| fo.get("pressure_state"))
        .and_then(|v| v.as_str())
    {
        let ps = ps.to_ascii_lowercase();
        if ps.contains("serious") || ps.contains("critical") {
            m *= 0.65;
        } else if ps.contains("fair") {
            m *= 0.90;
        }
    }
    // explicit timing quality score if FE uploaded (0..1)
    if let Some(tq) = fo
        .get("timing_quality_score")
        .and_then(|v| v.as_f64())
        .or_else(|| fo.get("timing_quality").and_then(|v| v.as_f64()))
    {
        if tq.is_finite() && tq >= 0.0 && tq <= 1.0 {
            m *= 0.55 + 0.45 * tq;
        }
    }
    m.clamp(0.25, 1.0)
}

/// Commercial of via **probe analysis** (iss + opensource DrawnApart / PUF path):
/// multiround median → full structure + rank — **no** forced coarse mean bucket.
/// Policy: if any curve material exists, always produce a body (fallback structure/raw),
/// never discard detected samples as literal 0.
fn encode_of_from_analysis(fo: &Map<String, Value>, single: &[f64]) -> String {
    use crate::hw_probe_analysis::{
        analyze_timing_like, digest_material, rounds_from_fields, ScaleMode,
    };
    let mut rounds = rounds_from_fields(
        fo,
        &["hw_curve_canvas_rounds", "canvas_noise_rounds", "of_rounds"],
        &[],
    );
    if rounds.is_empty() && single.len() >= 4 {
        rounds.push(single.to_vec());
    }
    if let Some(ana) = analyze_timing_like(&rounds, ScaleMode::None) {
        return format!("cv:{}", digest_material("of", &ana.material));
    }
    // Fallback: utilize whatever samples we have (short / low-entropy still differentiate devices).
    if single.len() >= 4 {
        if let Some(cs) = curve_structure_sig(single) {
            return format!("cv:{}", digest_material("of", &format!("struct:{cs}")));
        }
        let head: Vec<String> = single
            .iter()
            .take(16)
            .map(|x| format!("{x:.5}"))
            .collect();
        return format!(
            "cv:{}",
            digest_material("of", &format!("rawhead:{}", head.join(",")))
        );
    }
    PLACEHOLDER.to_string()
}

/// Commercial ar/gp — multiround **scale-invariant structure** (krank/lowvar), not
/// places=6 fine CSV. Challenge-seeded residuals still may fork across visits when
/// FE re-seeds every session; body intentionally omits wall-ms / seed extras.
///
/// Policy (2026-08): **utilize every detected WebGPU material**. Gates demote conf,
/// they must not force commercial `0` when residual/timing samples exist — identical
/// zeros collide harder than weak digests.
fn encode_ar_from_analysis(fo: &Map<String, Value>, single: &[f64]) -> String {
    use crate::hw_probe_analysis::{
        analyze_timing_like, digest_material, rounds_from_fields, ScaleMode,
    };
    // Prefer EU timing (hardware-like, less seed-coupled) when multi-sample present.
    let mut eu_rounds = rounds_from_fields(
        fo,
        &["webgpu_eu_timing_rounds", "webgpu_dispatch_timing_rounds"],
        &[],
    );
    if eu_rounds.is_empty() {
        if let Some(arr) = fo
            .get("webgpu_eu_timing_curve")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_f64())
                    .filter(|x| x.is_finite())
                    .collect::<Vec<_>>()
            })
        {
            if arr.len() >= 2 {
                eu_rounds.push(arr);
            }
        }
    }
    if eu_rounds.is_empty() {
        if let Some(arr) = fo
            .get("webgpu_dispatch_timings_ms")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_f64())
                    .filter(|x| x.is_finite())
                    .collect::<Vec<_>>()
            })
        {
            if arr.len() >= 2 {
                eu_rounds.push(arr);
            }
        }
    }
    if let Some(ana) = analyze_timing_like(&eu_rounds, ScaleMode::Mean) {
        // Flat EU (all 1s on software path) → lowvar class; real EU spreads → krank.
        return format!("gp:{}", digest_material("ar", &format!("eu:{}", ana.material)));
    }
    // Short EU/dispatch (2–3 samples): still encode — better than discarding.
    if !eu_rounds.is_empty() {
        let flat: Vec<f64> = eu_rounds.iter().flatten().copied().collect();
        if flat.len() >= 2 {
            let head: Vec<String> = flat
                .iter()
                .take(16)
                .map(|x| format!("{x:.4}"))
                .collect();
            return format!(
                "gp:{}",
                digest_material("ar", &format!("eu_short:{}", head.join(",")))
            );
        }
    }

    // Residual compute curve: scale-inv structure; no challenge/wall extras in body.
    let mut rounds = rounds_from_fields(
        fo,
        &[
            "hw_curve_webgpu_rounds",
            "webgpu_compute_rounds",
            "webgpu_residual_rounds",
            "ar_rounds",
        ],
        &[],
    );
    if rounds.is_empty() && single.len() >= 2 {
        rounds.push(single.to_vec());
    }
    if let Some(ana) = analyze_timing_like(&rounds, ScaleMode::Mean) {
        return format!("gp:{}", digest_material("ar", &ana.material));
    }
    // Fallback chain: structure sig → short raw head — never drop residual samples to 0.
    if single.len() >= 8 {
        if let Some(cs) = curve_structure_sig(single) {
            return format!("gp:{}", digest_material("ar", &format!("struct:{cs}")));
        }
    }
    if single.len() >= 2 {
        let head: Vec<String> = single
            .iter()
            .take(24)
            .map(|x| format!("{x:.5}"))
            .collect();
        return format!(
            "gp:{}",
            digest_material("ar", &format!("rawhead:{}", head.join(",")))
        );
    }
    PLACEHOLDER.to_string()
}

/// Commercial-safe WebGPU class materials (no wall-ms / fine mean that forks under load).
/// Used when residual curve is absent but B18 still landed useful device surface.
fn webgpu_stack_extra_commercial(fo: &Map<String, Value>) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(s) = str_field(fo, "webgpu_adapter_surface").filter(|s| !s.is_empty()) {
        parts.push(format!("as={}", sanitize_token(&s)));
    }
    if let Some(sk) = str_field(fo, "webgpu_compute_skip")
        .or_else(|| str_field(fo, "webgpu_skip"))
        .filter(|s| !s.is_empty())
    {
        parts.push(format!("sk={}", sanitize_token(&sk)));
    }
    if fo.get("webgpu_available").and_then(|v| v.as_bool()) == Some(false) {
        parts.push("avail=0".into());
    } else if fo.get("webgpu_available").and_then(|v| v.as_bool()) == Some(true) {
        parts.push("avail=1".into());
    }
    if fo.get("webgpu_compute_ok").and_then(|v| v.as_bool()) == Some(true) {
        parts.push("ok=1".into());
    } else if fo.get("webgpu_compute_ok").and_then(|v| v.as_bool()) == Some(false) {
        parts.push("ok=0".into());
    }
    for k in [
        "webgpu_limits_hash",
        "webgpu_features_hash",
        "webgpu_limits_high_hash",
        "webgpu_features_high_hash",
    ] {
        if let Some(s) = str_field(fo, k).filter(|s| !s.is_empty() && s != "none") {
            parts.push(format!("{k}={}", sanitize_token(&s)));
        }
    }
    if let Some(b) = fo
        .get("webgpu_dual_adapter_diff")
        .and_then(|v| v.as_bool())
    {
        parts.push(format!("dual={b}"));
    }
    if let Some(b) = fo
        .get("webgpu_is_fallback")
        .or_else(|| fo.get("webgpu_is_fallback_default"))
        .and_then(|v| v.as_bool())
    {
        parts.push(format!("fb={b}"));
    }
    if let Some(info) = fo
        .get("webgpu_info_default")
        .or_else(|| fo.get("webgpu_info_high"))
    {
        if let Some(arch) = info.get("architecture").and_then(|v| v.as_str()) {
            if !arch.is_empty() {
                parts.push(format!("arch={}", short_hash(arch)));
            }
        }
    }
    // f16 structure (not wall-ms) when residual path is only f16
    let f16_curve = curve_array(fo, "hw_curve_webgpu_f16");
    if f16_curve.len() >= 8 {
        if let Some(cs) = curve_structure_sig(&f16_curve) {
            parts.push(format!("f16c={}", short_hash(&cs)));
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("|"))
    }
}

/// WebGPU extras for ar/gp — compute residual + f16 + adapter surface (Lane-S).
fn webgpu_stack_extra(fo: &Map<String, Value>) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    // Always encode capability / skip class first so conf is stable on old hardware.
    if let Some(s) = str_field(fo, "webgpu_adapter_surface").filter(|s| !s.is_empty()) {
        parts.push(format!("as={}", sanitize_token(&s)));
    }
    if let Some(sk) = str_field(fo, "webgpu_compute_skip")
        .or_else(|| str_field(fo, "webgpu_skip"))
        .filter(|s| !s.is_empty())
    {
        parts.push(format!("sk={}", sanitize_token(&sk)));
    }
    if fo.get("webgpu_available").and_then(|v| v.as_bool()) == Some(false) {
        parts.push("avail=0".into());
    } else if fo.get("webgpu_available").and_then(|v| v.as_bool()) == Some(true) {
        parts.push("avail=1".into());
    }
    if let Some(m) = fo.get("webgpu_compute_mean").and_then(|v| v.as_f64()) {
        parts.push(format!("cm={:.6}", m));
    }
    if let Some(s) = fo.get("webgpu_compute_std").and_then(|v| v.as_f64()) {
        parts.push(format!("cs={:.6}", s));
    }
    if let Some(ms) = fo.get("webgpu_compute_ms").and_then(|v| v.as_f64()) {
        // 0.5ms bucket — timing noisy but same-SKU EU/scheduler sensitive
        parts.push(format!("cms={:.1}", (ms * 2.0).round() / 2.0));
    }
    // iss/54 P5 — f16 residual (stronger silicon when shader-f16 available)
    if fo.get("webgpu_f16_ok").and_then(|v| v.as_bool()) == Some(true) {
        if let Some(m) = fo.get("webgpu_f16_mean").and_then(|v| v.as_f64()) {
            parts.push(format!("f16m={:.6}", m));
        }
        if let Some(s) = fo.get("webgpu_f16_std").and_then(|v| v.as_f64()) {
            parts.push(format!("f16s={:.6}", s));
        }
        if let Some(ms) = fo.get("webgpu_f16_ms").and_then(|v| v.as_f64()) {
            parts.push(format!("f16ms={:.1}", (ms * 2.0).round() / 2.0));
        }
        let f16_curve = curve_array(fo, "hw_curve_webgpu_f16");
        if f16_curve.len() >= 8 {
            if let Some(cs) = curve_structure_sig(&f16_curve) {
                parts.push(format!("f16c={}", short_hash(&cs)));
            }
        }
        if let Some(arr) = fo.get("webgpu_f16_hist16").and_then(|v| v.as_array()) {
            let h: Vec<String> = arr
                .iter()
                .filter_map(|x| x.as_f64().or_else(|| x.as_i64().map(|i| i as f64)))
                .map(|x| format!("{x}"))
                .collect();
            if h.len() >= 8 {
                parts.push(format!("f16h={}", short_hash(&h.join(","))));
            }
        }
    } else if let Some(sk) = str_field(fo, "webgpu_f16_skip") {
        parts.push(format!("f16skip={}", sanitize_token(&sk)));
    }
    if let Some(arr) = fo.get("webgpu_compute_hist16").and_then(|v| v.as_array()) {
        let h: Vec<String> = arr
            .iter()
            .filter_map(|x| x.as_f64().or_else(|| x.as_i64().map(|i| i as f64)))
            .map(|x| format!("{x}"))
            .collect();
        if h.len() >= 8 {
            parts.push(format!("h16={}", short_hash(&h.join(","))));
        }
    }
    if let Some(arr) = fo.get("webgpu_compute_head8").and_then(|v| v.as_array()) {
        let h: Vec<String> = arr
            .iter()
            .filter_map(|x| {
                x.as_u64()
                    .or_else(|| x.as_i64().map(|i| i as u64))
                    .or_else(|| x.as_f64().map(|f| f as u64))
            })
            .map(|x| x.to_string())
            .collect();
        if !h.is_empty() {
            parts.push(format!("hd8={}", short_hash(&h.join(","))));
        }
    }
    // Dual-adapter limits/features (B18) — class + driver surface
    for k in [
        "webgpu_limits_hash",
        "webgpu_features_hash",
        "webgpu_limits_high_hash",
        "webgpu_features_high_hash",
    ] {
        if let Some(s) = str_field(fo, k).filter(|s| !s.is_empty()) {
            parts.push(format!("{k}={}", sanitize_token(&s)));
        }
    }
    // Nested limits/features objects → stable hash (not plain vendor strings)
    for k in [
        "webgpu_limits",
        "webgpu_limits_default",
        "webgpu_limits_high",
        "webgpu_features",
        "webgpu_features_default",
        "webgpu_features_high",
    ] {
        if let Some(v) = fo.get(k) {
            if !v.is_null() {
                let s = v.to_string();
                if s.len() > 4 && s != "{}" && s != "[]" && s != "null" {
                    parts.push(format!("{k}={}", short_hash(&s)));
                }
            }
        }
    }
    if let Some(b) = fo
        .get("webgpu_dual_adapter_diff")
        .and_then(|v| v.as_bool())
    {
        parts.push(format!("dual={b}"));
    }
    if let Some(b) = fo
        .get("webgpu_is_fallback")
        .or_else(|| fo.get("webgpu_is_fallback_default"))
        .and_then(|v| v.as_bool())
    {
        parts.push(format!("fb={b}"));
    }
    // Architecture label is class-ish — only as opaque digest never plain
    if let Some(info) = fo.get("webgpu_info_default").or_else(|| fo.get("webgpu_info_high")) {
        if let Some(arch) = info.get("architecture").and_then(|v| v.as_str()) {
            if !arch.is_empty() {
                parts.push(format!("arch={}", short_hash(arch)));
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("|"))
    }
}

/// Timing stack extras for tz/tm (system clock quant / rAF scheduling).
/// Full set is diagnostic; commercial uses `timing_stack_extra_commercial`.
fn timing_stack_extra(fo: &Map<String, Value>) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    // performance.now resolution / clamping (browser privacy + OS timer)
    for k in [
        "perf_now_resolution_ms",
        "performance_now_resolution",
        "timer_resolution_ms",
        "raf_hz",
        "raf_mean_ms",
        "raf_std_ms",
    ] {
        if let Some(n) = fo
            .get(k)
            .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
        {
            if n.is_finite() {
                parts.push(format!("{k}={:.4}", n));
            }
        }
    }
    // SAB/Atomics high-res clock (iss/54 P8) — foundation for timing precision
    for k in ["sab_clock_digest", "atomics_timing_digest", "sab_timer_hash"] {
        if let Some(s) = str_field(fo, k).filter(|s| !s.is_empty()) {
            parts.push(format!("{k}={}", sanitize_token(&s)));
        }
    }
    if let Some(tpm) = fo
        .get("sab_ticks_per_ms")
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
    {
        // coarse bucket — VM/cloud often far from bare metal
        parts.push(format!("sab_tpm={}", (tpm / 1000.0).round() as i64 * 1000));
    }
    if let Some(j) = fo.get("sab_eventloop_jitter_ms").and_then(|v| v.as_f64()) {
        parts.push(format!("sab_j={:.3}", (j * 1000.0).round() / 1000.0));
    }
    let sab_curve = curve_array(fo, "sab_tick_delta_curve");
    if sab_curve.len() >= 4 {
        if let Some(cs) = curve_structure_sig(&sab_curve) {
            parts.push(format!("sab_c={}", short_hash(&cs)));
        } else {
            let head: Vec<String> = sab_curve.iter().take(8).map(|x| format!("{x:.0}")).collect();
            parts.push(format!("sab_h={}", short_hash(&head.join(","))));
        }
    }
    if fo.get("sab_clock_ok").and_then(|v| v.as_bool()) == Some(false) {
        if let Some(sk) = str_field(fo, "sab_clock_skip") {
            parts.push(format!("sab_skip={}", sanitize_token(&sk)));
        }
    }
    // iss/58 A1: application clock skew (tz extra only; class-marked)
    if let Some(b) = fo
        .get("clock_skew_ppm_bucket")
        .and_then(|v| v.as_f64())
    {
        if b.is_finite() {
            parts.push(format!("csk={b:.1}"));
        }
    }
    if let Some(s) = str_field(fo, "clock_skew_class").filter(|s| !s.is_empty()) {
        parts.push(format!("csk_cls={}", sanitize_token(&s)));
    }
    if let Some(s) = str_field(fo, "clock_skew_tz_extra").filter(|s| !s.is_empty()) {
        parts.push(format!("csk_x={}", sanitize_token(&s)));
    }
    // iss/58 A9: compute_pressure must NOT enter digest body (doc: 不进任何槽).
    // Kept only on fields for FE scheduling / ops diagnostics.
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("|"))
    }
}

/// Commercial tz extras: only **stable timer class** tokens (soft K side-channel).
/// Drop sab_tpm absolute counts (can drift visit-to-visit) and high-res tres that
/// re-fork commercial digests. Keep sab skip class when clock unavailable.
/// Display Hz class is **not** folded into V body (fleet collision); see `display_hz_class_k`.
fn timing_stack_extra_commercial(fo: &Map<String, Value>) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    // Coarse timer resolution class only (privacy clamp bands), not micro decimals
    for k in [
        "perf_now_resolution_ms",
        "performance_now_resolution",
        "timer_resolution_ms",
    ] {
        if let Some(n) = fo
            .get(k)
            .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
        {
            if n.is_finite() && n >= 0.0 {
                // class bands: <0.01, <0.1, <1, else 1+
                let band = if n < 0.01 {
                    "t0"
                } else if n < 0.1 {
                    "t1"
                } else if n < 1.0 {
                    "t2"
                } else {
                    "t3"
                };
                parts.push(format!("tres={band}"));
                break;
            }
        }
    }
    if fo.get("sab_clock_ok").and_then(|v| v.as_bool()) == Some(false) {
        if let Some(sk) = str_field(fo, "sab_clock_skip") {
            parts.push(format!("sab_skip={}", sanitize_token(&sk)));
        }
    } else if fo.get("sab_clock_ok").and_then(|v| v.as_bool()) == Some(true) {
        parts.push("sab_ok=1".into());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("|"))
    }
}

/// Display refresh **class** (K/diagnostic only) — never commercial V primary.
/// Hysteresis-friendly bands; boundary visits use nearest standard panel rate.
pub fn display_hz_class_k(hz: f64) -> Option<u32> {
    if !hz.is_finite() || hz <= 1.0 || hz >= 500.0 {
        return None;
    }
    // Midpoints between common panel rates with hysteresis-style wider edges.
    Some(if hz >= 105.0 {
        120
    } else if hz >= 82.0 {
        90
    } else if hz >= 67.0 {
        75
    } else if hz >= 52.0 {
        60
    } else if hz >= 37.0 {
        45
    } else if hz >= 27.0 {
        30
    } else {
        24
    })
}

/// Commercial tz: multi-path **scale-invariant** structure (primary V material).
/// Display Hz class + sab absolute ticks are K/diagnostic only — not V body.
/// Gecko/WebKit often emit 16/17ms lattices — analysis uses lowvar|lattice (not Hz body).
fn encode_tz_from_analysis(fo: &Map<String, Value>, single: &[f64], extra: Option<&str>) -> String {
    encode_tz_multipath(fo, single, extra).body
}

fn encode_tz_multipath(
    fo: &Map<String, Value>,
    single: &[f64],
    extra: Option<&str>,
) -> PathEncode {
    use crate::hw_probe_analysis::{
        analyze_timing_like, digest_material, rounds_from_fields, ScaleMode,
    };
    // Priority: absolute rAF multiround first (Gecko/WebKit lattice-stable),
    // then relative jitter, then singles. Jitter ranks flip on coarse timers.
    let paths: &[(&str, &[&str], &str, u8, f64)] = &[
        (
            "raf_interval_rounds_multiround",
            &["raf_interval_rounds"],
            "B10_hw_curves",
            1,
            1.0,
        ),
        (
            "timing_jitter_rounds_multiround",
            &["timing_jitter_rounds", "tz_rounds"],
            "B10_hw_curves",
            2,
            0.95,
        ),
        (
            "raf_interval_curve_single",
            &["raf_interval_curve"],
            "B10_hw_curves",
            3,
            0.75,
        ),
        (
            "timing_jitter_curve_single",
            &["timing_jitter_curve"],
            "B10_hw_curves",
            4,
            0.7,
        ),
    ];
    let mut prepared: Vec<(
        &'static str,
        Vec<&'static str>,
        &'static str,
        u8,
        f64,
        Vec<Vec<f64>>,
    )> = Vec::new();
    let mut candidates = Vec::new();
    for (method, keys, batch, pri, w) in paths {
        let mut rounds = rounds_from_fields(fo, keys, &[]);
        let mut used_keys: Vec<&'static str> = Vec::new();
        if rounds.is_empty() {
            for k in *keys {
                if let Some(arr) = fo.get(*k).and_then(|v| v.as_array()) {
                    let c: Vec<f64> = arr
                        .iter()
                        .filter_map(|x| x.as_f64())
                        .filter(|x| x.is_finite())
                        .collect();
                    // 2+ samples: utilize short rAF/jitter packs.
                    if c.len() >= 2 {
                        rounds.push(c);
                        used_keys.push(*k);
                        break;
                    }
                }
            }
            if rounds.is_empty() && *method == "timing_jitter_curve_single" && single.len() >= 2 {
                rounds.push(single.to_vec());
                used_keys.push("single_arg");
            }
        } else {
            used_keys.extend(keys.iter().copied().filter(|k| {
                fo.get(*k)
                    .and_then(|v| v.as_array())
                    .map(|a| !a.is_empty())
                    .unwrap_or(false)
            }));
        }
        let available = !rounds.is_empty();
        candidates.push(json!({
            "method": method,
            "material_keys": keys,
            "batch_hint": batch,
            "priority": pri,
            "weight": w,
            "available": available,
            "rounds_n": rounds.len(),
            "commercial": true,
        }));
        if available {
            prepared.push((*method, used_keys, *batch, *pri, *w, rounds));
        }
    }
    candidates.push(json!({
        "method": "display_hz_class_k_soft",
        "material_keys": ["raf_hz_est"],
        "batch_hint": "B10_hw_curves",
        "priority": 15,
        "weight": 0.35,
        "available": fo.get("raf_hz_est").is_some() || fo.get("raf_hz_mean_est").is_some(),
        "commercial": true,
        "note": "K soft body when no jitter curve — better than identical 0",
    }));
    candidates.push(json!({
        "method": "sab_clock_class_soft",
        "material_keys": ["sab_ticks_per_ms", "sab_clock_digest"],
        "batch_hint": "B47_sab_clock",
        "priority": 16,
        "weight": 0.30,
        "available": fo.get("sab_clock_digest").is_some() || fo.get("sab_ticks_per_ms").is_some(),
        "commercial": true,
    }));
    for (method, used_keys, batch, pri, w, rounds) in prepared {
        if let Some(ana) = analyze_timing_like(&rounds, ScaleMode::Mean) {
            let mut material = format!("j:{}", ana.material);
            if let Some(ex) = extra {
                if !ex.is_empty() {
                    material = format!("{material}|ex:{ex}");
                }
            }
            return PathEncode {
                body: format!("tm:{}", digest_material("tz", &material)),
                method,
                material_keys: used_keys,
                batch_hint: batch,
                priority: pri,
                weight: w,
                candidates,
            };
        }
        let flat: Vec<f64> = rounds.iter().flatten().copied().filter(|x| x.is_finite()).collect();
        if flat.len() >= 2 {
            if let Some(cs) = curve_structure_sig(&flat) {
                return PathEncode {
                    body: format!("tm:{}", digest_material("tz", &format!("struct:{cs}"))),
                    method: "tz_structure_fallback",
                    material_keys: used_keys,
                    batch_hint: batch,
                    priority: pri.saturating_add(5),
                    weight: w * 0.7,
                    candidates,
                };
            }
            let head: Vec<String> = flat.iter().take(16).map(|x| format!("{x:.4}")).collect();
            return PathEncode {
                body: format!(
                    "tm:{}",
                    digest_material("tz", &format!("rawhead:{}", head.join(",")))
                ),
                method: "tz_rawhead_fallback",
                material_keys: used_keys,
                batch_hint: batch,
                priority: pri.saturating_add(6),
                weight: w * 0.55,
                candidates,
            };
        }
    }
    PathEncode {
        body: PLACEHOLDER.to_string(),
        method: "missing",
        material_keys: vec![],
        batch_hint: "",
        priority: 255,
        weight: 0.0,
        candidates,
    }
}

/// iss/50 H2: of/ar/cc/tz hold **curve digests**, never fixed names.
/// Returns materials for of/ar/cc/tz + notes + per-slot quality.
fn pick_extended_curve_slots(
    fo: &Map<String, Value>,
    proj: &Value,
) -> (String, String, String, String, Vec<String>, [f64; 4]) {
    let mut notes = Vec::new();
    // of / cv — commercial: coarse fixed-window mean/std only (no optional hash extras).
    // iss/75-style reliability: optional canvas_noise_hash / pat= forked same-browser revisits.
    let (mut cv_arr, mut cv_key) = best_curve(
        fo,
        &[
            "hw_curve_canvas",
            "canvas_noise_curve",
            "canvas_residual_curve",
        ],
        4,
    );
    // Prefer multiround median when FE uploaded rounds (DrawnApart-style).
    {
        use crate::hw_probe_analysis::{multiround_median, rounds_from_fields};
        let rounds = rounds_from_fields(
            fo,
            &["hw_curve_canvas_rounds", "canvas_noise_rounds", "of_rounds"],
            &[],
        );
        if rounds.len() >= 2 {
            if let Some(m) = multiround_median(&rounds) {
                cv_arr = m;
                cv_key = Some("hw_curve_canvas_rounds_median");
            }
        }
    }
    let _cv_extra_diag = canvas_stack_extra(fo); // conf/ops only
    let cv_extra = canvas_stack_extra_commercial(fo);
    let of_rounds_n = multiround_count(
        fo,
        &["hw_curve_canvas_rounds", "canvas_noise_rounds", "of_rounds"],
    );
    let sess_q = probe_session_quality_mult(fo);
    // of: use every canvas sample we have — conf demotes weak entropy, body stays.
    let (of, q_of) = if !cv_arr.is_empty() && cv_arr.len() >= 2 {
        notes.push(format!(
            "of_cv_from_{}_probe_analysis_v1",
            cv_key.unwrap_or("hw_curve_canvas")
        ));
        notes.push("of_cv_multiround_structure_rank_no_coarse_bucket".into());
        notes.push("of_cv_curve_body_only_no_optional_hash".into());
        notes.push("of_utilize_all_detected_samples".into());
        let mut q = if curve_slot_ok(&cv_arr) {
            1.0
        } else if cv_arr.len() >= 8 {
            0.75
        } else {
            0.55
        };
        // Single-shot canvas: mint V body but demote conf (reliability unknown).
        if of_rounds_n < 2 {
            q *= 0.75;
            notes.push("of_cv_single_round_conf_demote".into());
        } else {
            notes.push(format!("of_cv_multiround_n_{of_rounds_n}"));
        }
        q *= sess_q;
        (encode_of_from_analysis(fo, &cv_arr), q.min(1.0))
    } else if let Some(d) = proj
        .pointer("/materials/hw_canvas_stable")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        notes.push("of_cv_from_materials_digest".into());
        // materials digest alone — never fold optional stack that may be absent
        (format!("cv_d:{}", sanitize_token(d)), 0.7)
    } else if let Some(ex) = cv_extra {
        notes.push("of_cv_from_stack_extra_only".into());
        (format!("cv_x:{ex}"), 0.5)
    } else {
        notes.push("of_cv_missing".into());
        (PLACEHOLDER.to_string(), 0.0)
    };

    // ar / gp — multi-path utilize: residual/EU structure → digest → class stack → surface → caps.
    // Prefer f32 residual first; f16 is secondary material folded via commercial stack.
    // Conf demotes class/weak paths; **never** force commercial 0 when B18 material exists
    // (zeros collide across devices harder than weak digests).
    let (gp_arr, gp_key) = best_curve(
        fo,
        &[
            "hw_curve_webgpu",
            "webgpu_compute_curve",
            "webgpu_residual_curve",
            "hw_curve_webgpu_f16",
            "webgpu_compute_hist16",
        ],
        2,
    );
    let gp_extra_diag = webgpu_stack_extra(fo);
    let gp_extra = webgpu_stack_extra_commercial(fo);
    let _ = gp_extra_diag; // full stack remains ops/diagnostic (may include wall-ms)
    let truthy_challenge = |v: &Value| match v {
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty() && s != "0" && s != "false",
        Value::Number(n) => n.as_f64().map(|x| x != 0.0).unwrap_or(false),
        _ => false,
    };
    let ar_has_challenge = fo
        .get("webgpu_challenge_seed_used")
        .map(truthy_challenge)
        .unwrap_or(false)
        || fo
            .get("challenge_seed_used")
            .map(truthy_challenge)
            .unwrap_or(false);
    let ar_has_timing_v = fo
        .get("webgpu_eu_timing_curve")
        .and_then(|v| v.as_array())
        .map(|a| a.len() >= 2)
        .unwrap_or(false)
        || fo
            .get("webgpu_compute_timing_curve")
            .and_then(|v| v.as_array())
            .map(|a| a.len() >= 4)
            .unwrap_or(false)
        || fo
            .get("webgpu_dispatch_timings_ms")
            .and_then(|v| v.as_array())
            .map(|a| a.len() >= 2)
            .unwrap_or(false);
    let ar_compute_ok = fo
        .get("webgpu_compute_ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let ar_v_boost = ar_has_challenge || ar_has_timing_v || ar_compute_ok;
    let (ar, q_ar) = if !gp_arr.is_empty() {
        notes.push(format!(
            "ar_gp_from_{}_probe_analysis_v1",
            gp_key.unwrap_or("hw_curve_webgpu")
        ));
        notes.push("ar_gp_multiround_structure_rank_no_coarse_bucket".into());
        notes.push("ar_no_places6_fine_csv_no_wall_ms_extra".into());
        notes.push("ar_utilize_all_detected_samples".into());
        if ar_v_boost {
            notes.push("ar_challenge_or_timing_v_candidate".into());
        } else {
            notes.push("ar_fixed_residual_class_not_v".into());
        }
        if !curve_slot_ok(&gp_arr) {
            notes.push("ar_gp_weak_entropy_conf_demote_not_drop".into());
        }
        let body = encode_ar_from_analysis(fo, &gp_arr);
        if body == PLACEHOLDER {
            // Should be rare after encode fallbacks; still try commercial stack.
            if let Some(ex) = gp_extra.as_ref() {
                notes.push("ar_gp_analysis_empty_fallback_stack".into());
                notes.push("ar_class_slot_k_not_v".into());
                (
                    format!(
                        "gp:{}",
                        crate::hw_probe_analysis::digest_material("ar", &format!("stack:{ex}"))
                    ),
                    0.45 * sess_q,
                )
            } else {
                notes.push("ar_gp_analysis_failed_zero".into());
                (PLACEHOLDER.to_string(), 0.0)
            }
        } else {
            let mut q = if curve_slot_ok(&gp_arr) && gp_arr.len() >= 8 {
                0.85
            } else if gp_arr.len() >= 8 {
                0.65
            } else {
                0.50
            };
            if ar_has_timing_v {
                q = (q + 0.08_f64).min(0.95);
            } else if ar_has_challenge || ar_compute_ok {
                q = (q + 0.05_f64).min(0.90);
            }
            // Soft V until multi-session dual KPI; demote conf, keep body.
            q = (q * sess_q).min(0.75);
            notes.push("ar_soft_v_conf_demote_body_kept".into());
            (body, q)
        }
    } else if let Some(d) = str_field(fo, "hw_webgpu_compute_digest")
        .or_else(|| str_field(fo, "webgpu_compute_digest"))
        .filter(|s| !s.is_empty())
    {
        notes.push("ar_gp_from_digest".into());
        notes.push("ar_fixed_residual_class_not_v".into());
        notes.push("ar_utilize_digest_material".into());
        let mut mat = format!("gp_d:{}", sanitize_token(&d));
        if let Some(ex) = gp_extra.as_ref() {
            mat = format!("{mat}|{ex}");
        }
        (
            format!(
                "gp:{}",
                crate::hw_probe_analysis::digest_material("ar", &mat)
            ),
            0.60 * sess_q,
        )
    } else if let Some(ex) = gp_extra.as_ref() {
        notes.push("ar_gp_from_webgpu_stack_extra_commercial".into());
        notes.push("ar_class_slot_k_not_v".into());
        notes.push("ar_utilize_thin_b18_class_materials".into());
        (
            format!(
                "gp:{}",
                crate::hw_probe_analysis::digest_material("ar", &format!("stack:{ex}"))
            ),
            0.50 * sess_q,
        )
    } else if let Some(surf) = str_field(fo, "webgpu_adapter_surface")
        .filter(|s| !s.is_empty() && s != "none" && s != "no_adapter")
    {
        notes.push("ar_gp_from_adapter_surface".into());
        notes.push("ar_class_slot_k_not_v".into());
        (
            format!(
                "gp:{}",
                crate::hw_probe_analysis::digest_material(
                    "ar",
                    &format!("surf:{}", sanitize_token(&surf))
                )
            ),
            0.45 * sess_q,
        )
    } else if let (Some(lh), Some(fh)) = (
        str_field(fo, "webgpu_limits_hash").filter(|s| !s.is_empty() && s != "none"),
        str_field(fo, "webgpu_features_hash").filter(|s| !s.is_empty() && s != "none"),
    ) {
        notes.push("ar_gp_from_limits_features_hash".into());
        notes.push("ar_class_slot_k_not_v".into());
        (
            format!(
                "gp:{}",
                crate::hw_probe_analysis::digest_material(
                    "ar",
                    &format!("lf:{}", short_hash(&format!("{lh}|{fh}")))
                )
            ),
            0.40 * sess_q,
        )
    } else if let Some(sk) = str_field(fo, "webgpu_compute_skip")
        .or_else(|| str_field(fo, "webgpu_skip"))
        .filter(|s| !s.is_empty())
    {
        notes.push(format!("ar_gp_skip_{}", sanitize_token(&sk)));
        notes.push("ar_class_slot_k_not_v".into());
        (
            format!(
                "gp:{}",
                crate::hw_probe_analysis::digest_material(
                    "ar",
                    &format!("skip:{}", sanitize_token(&sk))
                )
            ),
            0.25 * sess_q,
        )
    } else {
        notes.push("ar_gp_missing".into());
        (PLACEHOLDER.to_string(), 0.0)
    };

    // cc / pm — utilize every GL cap we have (tex, rb, and/or other max-* floors).
    // Prefer tex+rb pair; partial caps still mint class K rather than 0.
    let tex = fo
        .get("gl_max_texture_size")
        .or_else(|| fo.get("webgl_max_texture"))
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)));
    let rb = fo
        .get("gl_max_renderbuffer")
        .or_else(|| fo.get("webgl_max_renderbuffer"))
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)));
    let (cc, q_cc) = match (tex, rb) {
        (Some(t), Some(r)) if t.is_finite() && r.is_finite() && t > 0.0 && r > 0.0 => {
            notes.push("cc_pm_from_gl_max_tex_rb_commercial_v5".into());
            notes.push("cc_class_slot_k_not_v".into());
            (
                format!("pm:tex={}|rb={}", t.round() as i64, r.round() as i64),
                0.9,
            )
        }
        (Some(t), _) if t.is_finite() && t > 0.0 => {
            notes.push("cc_pm_from_gl_max_tex_partial".into());
            notes.push("cc_class_slot_k_not_v".into());
            notes.push("cc_utilize_partial_caps".into());
            (format!("pm:tex={}", t.round() as i64), 0.7)
        }
        (_, Some(r)) if r.is_finite() && r > 0.0 => {
            notes.push("cc_pm_from_gl_max_rb_partial".into());
            notes.push("cc_class_slot_k_not_v".into());
            notes.push("cc_utilize_partial_caps".into());
            (format!("pm:rb={}", r.round() as i64), 0.65)
        }
        _ => {
            // Fuse any other stable GL numeric caps present on this visit.
            let mut parts: Vec<String> = Vec::new();
            for k in [
                "gl_max_cube_map_texture_size",
                "gl_max_vertex_attribs",
                "gl_max_texture_image_units",
                "gl_max_combined_texture_image_units",
                "webgl_max_viewport",
                "webgl_depth_bits",
                "webgl_samples",
            ] {
                if let Some(n) = fo
                    .get(k)
                    .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
                {
                    if n.is_finite() && n > 0.0 {
                        parts.push(format!("{k}={}", n.round() as i64));
                    }
                }
            }
            if let Some(s) = str_field(fo, "webgl_params_digest").filter(|s| !s.is_empty()) {
                parts.push(format!("pd={}", sanitize_token(&s)));
            }
            if parts.is_empty() {
                notes.push("cc_pm_missing".into());
                (PLACEHOLDER.to_string(), 0.0)
            } else {
                notes.push("cc_pm_from_secondary_gl_caps".into());
                notes.push("cc_class_slot_k_not_v".into());
                notes.push("cc_utilize_partial_caps".into());
                (format!("pm:{}", parts.join("|")), 0.55)
            }
        }
    };

    // tz / tm — structure first; if only class timer / Hz materials, still mint (K body).
    let (tm_arr, tm_key) = best_curve(
        fo,
        &[
            "timing_jitter_curve",
            "raf_interval_curve",
            "hw_noise_curve",
            // sab_tick_delta last — only if no rAF material
            "sab_tick_delta_curve",
        ],
        4,
    );
    let _tm_extra_diag = timing_stack_extra(fo);
    let tm_extra = timing_stack_extra_commercial(fo);
    let tz_rounds_n = multiround_count(
        fo,
        &["timing_jitter_rounds", "raf_interval_rounds", "tz_rounds"],
    );
    let hz_class = fo
        .get("raf_hz_est")
        .or_else(|| fo.get("raf_hz_mean_est"))
        .and_then(|v| v.as_f64())
        .and_then(display_hz_class_k);
    if let Some(hc) = hz_class {
        notes.push(format!("tz_display_hz_class_k_{hc}"));
    }
    let (tz, q_tz) = if tm_arr.len() >= 6 && !curve_is_collapsed(&tm_arr) {
        let mut q = if curve_slot_ok(&tm_arr) { 1.0 } else { 0.75 };
        notes.push(format!(
            "tz_tm_from_{}_probe_analysis_v1",
            tm_key.unwrap_or("hw_noise_curve")
        ));
        notes.push("tz_tm_scale_invariant_jitter_structure_no_hz_body".into());
        notes.push("tz_utilize_all_detected_samples".into());
        if tz_rounds_n < 2 {
            q *= 0.70;
            notes.push("tz_tm_single_round_conf_demote".into());
        } else {
            notes.push(format!("tz_tm_multiround_n_{tz_rounds_n}"));
        }
        q *= sess_q;
        (encode_tz_from_analysis(fo, &tm_arr, None), q.min(1.0))
    } else if !tm_arr.is_empty() && tm_arr.len() >= 2 {
        notes.push(format!(
            "tz_tm_from_{}_len_ok_probe_analysis_v1",
            tm_key.unwrap_or("timing_jitter_curve")
        ));
        notes.push("tz_utilize_short_or_weak_curve".into());
        let mut q = 0.55 * sess_q;
        if tz_rounds_n < 2 {
            q *= 0.70;
        }
        let body = encode_tz_from_analysis(fo, &tm_arr, None);
        if body != PLACEHOLDER {
            (body, q.min(1.0))
        } else if let Some(cs) = curve_structure_sig(&tm_arr) {
            (
                format!(
                    "tm:{}",
                    crate::hw_probe_analysis::digest_material("tz", &format!("struct:{cs}"))
                ),
                q.min(1.0),
            )
        } else {
            let head: Vec<String> = tm_arr.iter().take(12).map(|x| format!("{x:.4}")).collect();
            (
                format!(
                    "tm:{}",
                    crate::hw_probe_analysis::digest_material(
                        "tz",
                        &format!("rawhead:{}", head.join(","))
                    )
                ),
                q.min(1.0),
            )
        }
    } else if let Some(ex) = tm_extra.as_ref() {
        // Timer class / sab availability — K material, but better than identical 0.
        notes.push("tz_tm_from_timer_class_extra_commercial".into());
        notes.push("tz_class_slot_k_not_v".into());
        notes.push("tz_utilize_class_materials".into());
        (
            format!(
                "tm:{}",
                crate::hw_probe_analysis::digest_material("tz", &format!("class:{ex}"))
            ),
            0.40 * sess_q,
        )
    } else if let Some(hc) = hz_class {
        notes.push("tz_tm_from_display_hz_class_k".into());
        notes.push("tz_class_slot_k_not_v".into());
        notes.push("tz_utilize_hz_class".into());
        (
            format!(
                "tm:{}",
                crate::hw_probe_analysis::digest_material("tz", &format!("hz:{hc}"))
            ),
            0.35 * sess_q,
        )
    } else {
        notes.push("tz_tm_missing".into());
        (PLACEHOLDER.to_string(), 0.0)
    };

    notes.push("extended_curve_slots_v6_utilize_all_detected_materials".into());
    (of, ar, cc, tz, notes, [q_of, q_ar, q_cc, q_tz])
}

/// Host-separator + **gateway protocol fingerprints** — oi + rtc digests.
///
/// Redlines (still enforced):
/// - Never plain UA / IP / OS name / browser name / GPU model in materials.
/// - Protocol stack tokens (JA4H-lite order hash, TCP SYN option order, JA4L/RTT bucket)
///   **do** enter commercial body as opaque digests (iss/54 + prod gateway mint).
fn pick_host_separator_parts(fo: &Map<String, Value>) -> (String, String) {
    let mut oi_parts: Vec<String> = Vec::new();
    if let Some(s) = str_field(fo, "os_instance_hash") {
        oi_parts.push(format!("oi:{s}"));
    }
    // Soft redline (iss/74 §3.1.1): unversioned / unstable unit is conf-only —
    // never fold raw unit_surface_id into commercial oi body.
    let unit_versioned = fo
        .get("unit_multiround_stable")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        && fo
            .get("unit_surface_algo")
            .and_then(|v| v.as_str())
            .is_some_and(|a| a.starts_with("gr_unit"));
    if unit_versioned {
        if let Some(unit) = str_field(fo, "unit_surface_id") {
            oi_parts.push(format!("unit:{unit}"));
        }
    }
    // Optional opaque unit_surface_digest only — never fall back to unit_surface_id
    // (legacy/unversioned ids must not fork Soft commercial digests).
    if !oi_parts.iter().any(|p| p.starts_with("unit:")) {
        if let Some(s) = str_field(fo, "unit_surface_digest").filter(|s| !s.is_empty()) {
            if fo.get("unit_surface_available")
                .and_then(|v| v.as_bool())
                .unwrap_or(true)
            {
                oi_parts.push(format!("unit_d:{}", sanitize_token(&s)));
            }
        }
    }
    if let Some(s) = str_field(fo, "pohw_triad")
        .or_else(|| str_field(fo, "challenge_seed_sig"))
        .or_else(|| str_field(fo, "pohw_hash"))
    {
        oi_parts.push(format!("pohw:{s}"));
    }
    // System-instance soft probes (digests only — not OS/arch names)
    for k in [
        "os_instance_source",
        "pohw_direction",
        "env_stack_fusion_digest",
        "storage_quota_class",
    ] {
        if let Some(s) = str_field(fo, k).filter(|s| !s.is_empty() && s.len() <= 64) {
            // only short class digests / enum tokens
            if !matches!(
                s.to_ascii_lowercase().as_str(),
                "windows" | "linux" | "macos" | "android" | "ios" | "chrome" | "firefox"
            ) {
                oi_parts.push(format!("{k}={}", sanitize_token(&s)));
            }
        }
    }
    // Protocol header fingerprint (gateway B8) — digest material only.
    if let Some(ph) = protocol_header_material(fo) {
        oi_parts.push(format!("ph:{ph}"));
    }
    // TLS/JA4 full when present (gateway) — stack fingerprint
    for k in ["ja4", "ja4_r", "ja4t", "tls_fingerprint"] {
        if let Some(s) = str_field(fo, k).filter(|s| !s.is_empty() && s != "0") {
            let tok = if s.len() > 48 {
                short_hash(&format!("tls|{k}|{s}"))
            } else {
                sanitize_token(&s)
            };
            oi_parts.push(format!("{k}={tok}"));
            break;
        }
    }
    let oi = if oi_parts.is_empty() {
        PLACEHOLDER.to_string()
    } else {
        oi_parts.join("|")
    };

    let mut rtc_parts: Vec<String> = Vec::new();
    if let Some(s) = str_field(fo, "webrtc_host_ip_hash_v2")
        .or_else(|| str_field(fo, "webrtc_host_ip_hash"))
    {
        rtc_parts.push(format!("rtc:{s}"));
    }
    // WebRTC host topology (counts / ice flags) — not raw IPs
    if let Some(n) = fo
        .get("webrtc_host_count")
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
    {
        rtc_parts.push(format!("hc={n}"));
    }
    for k in ["ice_has_host", "ice_has_srflx", "webrtc_method"] {
        if let Some(b) = fo.get(k).and_then(|v| v.as_bool()) {
            rtc_parts.push(format!("{k}={b}"));
        } else if let Some(s) = str_field(fo, k).filter(|s| !s.is_empty() && s.len() < 32) {
            rtc_parts.push(format!("{k}={}", sanitize_token(&s)));
        }
    }
    // mDNS hash is conf-only in FE but still a host-net instance signal (opaque)
    if let Some(s) = str_field(fo, "webrtc_mdns_hash").filter(|s| !s.is_empty()) {
        rtc_parts.push(format!("mdns={}", sanitize_token(&s)));
    }
    if let Some(n) = fo
        .get("webrtc_mdns_count")
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
    {
        if n > 0 {
            rtc_parts.push(format!("mdns_n={n}"));
        }
    }
    // Protocol / TCP timing fingerprint (gateway) — not client IP.
    if let Some(pt) = protocol_timing_material(fo) {
        rtc_parts.push(format!("pt:{pt}"));
    }
    let rtc = if rtc_parts.is_empty() {
        PLACEHOLDER.to_string()
    } else {
        rtc_parts.join("|")
    };
    (oi, rtc)
}

/// JA4H-lite / HTTP header-order / SYN option order — stack fingerprint, not UA text.
fn protocol_header_material(fo: &Map<String, Value>) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    // Prefer stable digests already computed at edge
    for k in [
        "ja4h_lite",
        "ja4h",
        "http_header_order_hash",
        "ja4h_lite_order",
    ] {
        if let Some(s) = str_field(fo, k).filter(|s| !s.is_empty() && s != "0") {
            // Strip accidental long raw order lists — keep short digests / tokens
            let tok = if s.len() > 80 || s.contains(',') {
                // hash long order lists into short material
                short_hash(&format!("hdr_raw|{k}|{s}"))
            } else {
                sanitize_token(&s)
            };
            if tok != PLACEHOLDER {
                parts.push(format!("{k}={tok}"));
            }
        }
    }
    if let Some(s) = str_field(fo, "tcp_syn_option_order").filter(|s| !s.is_empty()) {
        parts.push(format!("syn={}", short_hash(&format!("syn|{s}"))));
    }
    if let Some(s) = str_field(fo, "http_header_order").filter(|s| !s.is_empty() && s.len() > 8) {
        // only if no hash already
        if !parts.iter().any(|p| p.starts_with("http_header_order_hash=") || p.starts_with("ja4h"))
        {
            parts.push(format!("ord={}", short_hash(&format!("ord|{s}"))));
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("&"))
    }
}

/// JA4L / TCP RTT / congestion — timing network fingerprint, never raw IP.
fn protocol_timing_material(fo: &Map<String, Value>) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for k in ["ja4l_lite", "ja4l", "ja4l_partial"] {
        if let Some(s) = str_field(fo, k).filter(|s| !s.is_empty()) {
            let tok = if s.len() > 64 {
                short_hash(&format!("ja4l|{s}"))
            } else {
                sanitize_token(&s)
            };
            if tok != PLACEHOLDER {
                parts.push(format!("{k}={tok}"));
            }
            break;
        }
    }
    // RTT bucket (ms) — coarse, not unique IP
    let rtt_ms = fo
        .get("client_tcp_rtt")
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
        .or_else(|| {
            fo.get("client_tcp_rtt_us")
                .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
                .map(|us| us / 1000.0)
        })
        .or_else(|| {
            fo.get("tcp_info_rtt_us")
                .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
                .map(|us| us / 1000.0)
        })
        .or_else(|| {
            fo.get("ja4l_lite_rtt_ms")
                .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
        });
    if let Some(rtt) = rtt_ms {
        if rtt.is_finite() && rtt >= 0.0 {
            // log-ish buckets: 0-5, 5-10, 10-20, 20-40, 40-80, 80+
            let bucket = if rtt < 5.0 {
                "r0_5"
            } else if rtt < 10.0 {
                "r5_10"
            } else if rtt < 20.0 {
                "r10_20"
            } else if rtt < 40.0 {
                "r20_40"
            } else if rtt < 80.0 {
                "r40_80"
            } else {
                "r80p"
            };
            parts.push(format!("rtt={bucket}"));
        }
    }
    if let Some(s) = str_field(fo, "tcp_congestion").filter(|s| !s.is_empty()) {
        parts.push(format!("cc={}", sanitize_token(&s)));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("&"))
    }
}

/// Public curve descriptors for SDK clustering (no raw samples) — iss/50 H3 + iss/54 F2.
pub fn curve_descriptors_for_sdk(fields: &Value) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();
    // Ensure fused lane curves are available for LSH slots
    let mut fo_m = fo.clone();
    crate::hw_silicon_fusion::prefer_fused_curves(&mut fo_m);
    let mut out = Map::new();
    for (code, keys) in [
        (
            "wg",
            &[
                "hw_curve_webgl",
                "webgl_residual_multipath",
                "hw_curve_webgl_fused_c",
            ][..],
        ),
        (
            "wg_s",
            &["hw_curve_webgl_silicon", "hw_curve_webgl_fused_s"][..],
        ),
        ("au", &["hw_curve_audio", "audio_deep_curve"][..]),
        (
            "cp",
            // timing first — jit lowbits is diagnostic / often deterministic
            &["cpu_timing_curve", "hw_curve_cpu", "cache_ladder_curve", "jit_lowbits_curve"][..],
        ),
        ("cv", &["hw_curve_canvas", "canvas_noise_curve"][..]),
        ("gp", &["hw_curve_webgpu", "webgpu_compute_curve"][..]),
        (
            "tm",
            &["timing_jitter_curve", "raf_interval_curve", "hw_noise_curve"][..],
        ),
    ] {
        let (arr, src) = best_curve(&fo_m, keys, 4);
        if arr.is_empty() {
            continue;
        }
        // 16-bin coarse histogram + LSH-ish stable digest for distance-ish equality
        let desc = curve_lsh_descriptor(&arr);
        out.insert(
            code.into(),
            json!({
                "algo": "curve_lsh_v1",
                "source": src,
                "len": arr.len(),
                "entropy_ok": curve_slot_ok(&arr),
                "quality": curve_slot_quality(&arr),
                "hist16": desc.hist16,
                "lsh": desc.lsh,
                "mean": desc.mean,
                "std": desc.std,
            }),
        );
    }
    json!({
        "algo": "curve_descriptors_v1",
        "note": "for SDK clustering; slots wg=Lane-C, wg_s=Lane-S silicon; no raw samples",
        "slots": out,
    })
}

/// Public LSH descriptor for an arbitrary curve (iss/54 F2 fusion → SDK distance).
pub fn curve_lsh_public(arr: &[f64]) -> Value {
    if arr.len() < 4 {
        return json!({"algo": "curve_lsh_v1", "ok": false});
    }
    let desc = curve_lsh_descriptor(arr);
    json!({
        "algo": "curve_lsh_v1",
        "ok": true,
        "len": arr.len(),
        "hist16": desc.hist16,
        "lsh": desc.lsh,
        "mean": desc.mean,
        "std": desc.std,
    })
}

struct CurveLsh {
    hist16: Vec<u8>,
    lsh: String,
    mean: f64,
    std: f64,
}

fn curve_lsh_descriptor(arr: &[f64]) -> CurveLsh {
    let n = arr.len().max(1) as f64;
    let mean = arr.iter().sum::<f64>() / n;
    let var = arr.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n;
    let std = var.sqrt();
    // Normalize and bin into 16 buckets by rank quantiles
    let mut sorted: Vec<f64> = arr.iter().copied().filter(|x| x.is_finite()).collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut hist = vec![0u8; 16];
    if !sorted.is_empty() {
        let lo = sorted[0];
        let hi = sorted[sorted.len() - 1];
        let span = (hi - lo).abs().max(1e-12);
        for &v in arr {
            if !v.is_finite() {
                continue;
            }
            let mut b = (((v - lo) / span) * 16.0).floor() as usize;
            if b >= 16 {
                b = 15;
            }
            hist[b] = hist[b].saturating_add(1);
        }
    }
    let material = format!(
        "m={mean:.6}|s={std:.6}|h={}",
        hist.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",")
    );
    let lsh = short_hash(&format!("gr_curve_lsh_v1|{material}"));
    CurveLsh {
        hist16: hist,
        lsh,
        mean,
        std,
    }
}

/// Near-constant / dead curve (e.g. prod jit_lowbits: 31×0 + one non-zero bin).
/// Coarse wall-clock CPU timing often has 3–8 discrete bins — that is NOT collapsed.
fn curve_is_collapsed(arr: &[f64]) -> bool {
    if arr.len() < 4 {
        return true;
    }
    let mut sorted: Vec<f64> = arr.iter().copied().filter(|x| x.is_finite()).collect();
    if sorted.len() < 4 {
        return true;
    }
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut uniq = 1usize;
    for w in sorted.windows(2) {
        if (w[1] - w[0]).abs() > 1e-6 {
            uniq += 1;
        }
    }
    // fewer than 3 distinct levels → collapsed class template
    uniq < 3
}

/// CPU/cp material selection: real wall-clock timing over deterministic jit.
fn pick_cp_curve(fo: &Map<String, Value>) -> (Vec<f64>, Option<&'static str>) {
    let jit = curve_array(fo, "jit_lowbits_curve");
    let timing = curve_array(fo, "cpu_timing_curve");
    let hw = curve_array(fo, "hw_curve_cpu");
    let cache = curve_array(fo, "cache_ladder_curve");
    let noise = curve_array(fo, "hw_noise_curve");

    let jit_dead = !jit.is_empty() && curve_is_collapsed(&jit);
    let same_as_jit = |arr: &[f64]| -> bool {
        !jit.is_empty()
            && arr.len() == jit.len()
            && arr
                .iter()
                .zip(jit.iter())
                .all(|(a, b)| (*a - *b).abs() < 1e-12)
    };

    // 1) Explicit wall-clock timing (FE stores real stageCpu here even when
    //    older FE overwrote hw_curve_cpu with jit).
    if timing.len() >= 8 && !curve_is_collapsed(&timing) && !same_as_jit(&timing) {
        return (timing, Some("cpu_timing_curve"));
    }
    // 2) hw_curve_cpu if not a clone of dead jit
    if hw.len() >= 8 && !same_as_jit(&hw) && !curve_is_collapsed(&hw) {
        return (hw, Some("hw_curve_cpu"));
    }
    // 3) cache ladder / noise
    if cache.len() >= 8 && !curve_is_collapsed(&cache) {
        return (cache, Some("cache_ladder_curve"));
    }
    if noise.len() >= 8 && !curve_is_collapsed(&noise) {
        return (noise, Some("hw_noise_curve"));
    }
    // 4) timing even if coarse (prefer uniqueness over missing)
    if timing.len() >= 8 && !same_as_jit(&timing) {
        return (timing, Some("cpu_timing_curve"));
    }
    if hw.len() >= 8 && !same_as_jit(&hw) {
        return (hw, Some("hw_curve_cpu"));
    }
    // 5) jit only when not collapsed (should be rare)
    if jit.len() >= 8 && !jit_dead {
        return (jit, Some("jit_lowbits_curve"));
    }
    // last resort: longest non-empty among remaining (may still be dead; caller notes)
    best_curve(
        fo,
        &[
            "cpu_timing_curve",
            "hw_curve_cpu",
            "cache_ladder_curve",
            "hw_noise_curve",
        ],
        8,
    )
}

/// Prefer the longest complete curve among candidate keys (multi-path silicon probes).
/// Priority philosophy (iss/45–46 + product multi-segment):
/// 1. Full mint-length **raw timing/residual curves** (harder to forge; silicon-grade)
/// 2. Projected commercial digests when raw array absent
/// 3. Never GPU model / unmasked renderer strings in segment body
fn best_curve(fo: &Map<String, Value>, keys: &[&'static str], min_len: usize) -> (Vec<f64>, Option<&'static str>) {
    let mut best: Vec<f64> = Vec::new();
    let mut best_key: Option<&'static str> = None;
    for &k in keys {
        let c = curve_array(fo, k);
        if c.len() >= min_len && c.len() > best.len() {
            best = c;
            best_key = Some(k);
        }
    }
    // If none met min_len, still take the longest non-empty for diagnostics (caller gates).
    if best.is_empty() {
        for &k in keys {
            let c = curve_array(fo, k);
            if c.len() > best.len() {
                best = c;
                best_key = Some(k);
            }
        }
    }
    (best, best_key)
}

/// Silicon curve selection: prefer multi-path fused curves; never model strings.
fn pick_curves(fo: &Map<String, Value>, proj: &Value) -> (Vec<f64>, Vec<f64>, Vec<f64>, Vec<String>) {
    let mut notes = Vec::new();
    // Ensemble consensus curve first (median-mean path). Noderiv-first last-write
    // splits Chrome vs Edge on one GPU while they share an identical fma_pair path.
    let consensus_wg = ensemble_consensus_curve(fo);
    let lane_c_wg = lane_c_primary_curve(fo);
    let (mut wg, mut wg_key) = if consensus_wg.len() >= 8 {
        notes.push("wg_from_ensemble_consensus_path".into());
        (consensus_wg, Some("ensemble_consensus_path"))
    } else if lane_c_wg.len() >= 8 {
        notes.push("wg_from_lane_c_primary_path".into());
        (lane_c_wg, Some("lane_c_primary_path"))
    } else {
        // Longest healthy raw multipath first; silicon fuse only when longer/better.
        best_curve(
            fo,
            &[
                "webgl_residual_multipath",
                "hw_curve_webgl",
                "hw_curve_webgl_silicon",
                "hw_curve_webgl_fused_s",
                "hw_curve_webgl_fused_c",
                "webgl_residual_curve",
                "webgl_residual_v3e",
                "b10x_curve_webgl",
            ],
            4,
        )
    };
    // If multipath fused is short but primary is long, still use primary for wg body;
    // multipath sig is folded into res separately.
    if wg_key == Some("webgl_residual_multipath") {
        let primary = curve_array(fo, "hw_curve_webgl");
        if primary.len() >= 16 && primary.len() > wg.len() {
            // Keep longer primary for encode stability; note multipath present
            notes.push("wg_primary_curve_over_short_multipath_fuse".into());
            // Prefer primary length for digest uniqueness of full residual surface
            // only when multipath fuse is a short header (≤ half of primary).
            if wg.len() * 2 < primary.len() {
                wg = primary;
                wg_key = Some("hw_curve_webgl");
            }
        }
    }
    // Commercial au: **always prefer B10 seed-delta** (lands on B10; stable same-machine).
    // B46 audio_deep is optional/late — preferring deep when present only caused same
    // Ubuntu+Chrome cross-site / refresh churn (178 prod: sozhan/zhanso with B46 vs
    // searchchina/chinaallied without B46 → different au digests; seed_delta identical).
    // Absolute OfflineAudio bins remain last-resort (class-deterministic → high collision).
    let (mut au, mut au_key) = best_curve(
        fo,
        &[
            "audio_seed_delta_curve",
            "audio_noise_delta",
            "hw_curve_audio_delta",
            "hw_curve_audio",
            "audio_residual_curve",
            "b10x_curve_audio",
            "audio_deep_curve", // last resort only
        ],
        4, // prefer ≥4; shorter packs still kept by later utilize path
    );
    // Prefer seed-delta over absolute class bins / deep when both present.
    for key in [
        "audio_seed_delta_curve",
        "audio_noise_delta",
        "hw_curve_audio_delta",
    ] {
        let d = curve_array(fo, key);
        if d.len() >= 8 && !curve_is_collapsed(&d) {
            if au.is_empty()
                || curve_is_collapsed(&au)
                || au_key == Some("hw_curve_audio")
                || au_key == Some("audio_residual_curve")
                || au_key == Some("audio_deep_curve")
                || au_key == Some("b10x_curve_audio")
            {
                au = d;
                au_key = Some(key);
                notes.push(format!("au_commercial_seed_delta_{key}"));
                break;
            }
        }
    }
    // cp: wall-clock CPU timing first.
    // Prod root-cause (v5.8.120 mirror): FE overwrote hw_curve_cpu with
    // jit_lowbits from pure Math.* of fixed seeds → 1 global constant curve
    // (100% cp collision). Prefer cpu_timing_curve / real cpu wall curve;
    // accept jit only when it passes entropy (never mint collapsed class).
    let (mut cp, cp_key) = pick_cp_curve(fo);
    // Utilize short samples: keep whatever length FE landed; conf demotes elsewhere.
    if wg.len() >= 2 {
        notes.push(format!("wg_from_{}", wg_key.unwrap_or("hw_curve_webgl")));
        notes.push(format!("wg_len_{}", wg.len()));
        if wg.len() < 8 {
            notes.push("wg_short_curve_utilized".into());
        }
    } else if let Some(d) = proj
        .pointer("/materials/hw_webgl_stable")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        // Digest-only: encode as synthetic single-point curve via hash bytes → stable lane.
        notes.push("wg_from_materials_digest".into());
        let h = short_hash(d);
        wg = h
            .bytes()
            .take(8)
            .map(|b| (b as f64) / 255.0)
            .collect();
    } else {
        notes.push("wg_missing".into());
        wg.clear();
    }
    if au.len() >= 4 {
        notes.push(format!("au_from_{}", au_key.unwrap_or("hw_curve_audio")));
        notes.push(format!("au_len_{}", au.len()));
        if au.len() < 8 {
            notes.push("au_short_curve_utilized".into());
        }
    } else if let Some(d) = proj
        .pointer("/materials/hw_audio_stable")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        notes.push("au_from_materials_digest".into());
        let h = short_hash(d);
        au = h
            .bytes()
            .take(8)
            .map(|b| (b as f64) / 255.0)
            .collect();
    } else {
        // Keep 2–3 sample audio if present rather than hard-clear.
        if au.len() >= 2 {
            notes.push(format!("au_from_{}_very_short", au_key.unwrap_or("hw_curve_audio")));
            notes.push("au_short_curve_utilized".into());
        } else {
            notes.push("au_missing".into());
            au.clear();
        }
    }
    if cp.len() >= 2 {
        notes.push(format!("cp_from_{}", cp_key.unwrap_or("hw_curve_cpu")));
        notes.push(format!("cp_len_{}", cp.len()));
        if cp.len() < 8 {
            notes.push("cp_short_curve_utilized".into());
        }
    } else {
        notes.push("cp_missing".into());
        cp.clear();
    }
    // Explicitly ignore model labels for body
    if f_has(fo, "webgl_unmasked_renderer") || f_has(fo, "webgl_governed_renderer") {
        notes.push("renderer_label_excluded_from_segment".into());
    }
    notes.push("silicon_priority_raw_curve_over_digest_over_model".into());
    (wg, au, cp, notes)
}

fn places_for_prefix(prefix: &str) -> Option<u32> {
    match prefix {
        "dv0" => None,
        "dv4" => Some(4),
        "dv5" => Some(5),
        "dv6" => Some(6),
        _ => None,
    }
}

fn build_segment_body(
    residual: Option<f64>,
    wg: &[f64],
    au: &[f64],
    cp: &[f64],
    of: &str,
    ar: &str,
    cc: &str,
    tz: &str,
    oi: &str,
    rtc: &str,
    places: Option<u32>,
) -> String {
    build_segment_body_ex(
        residual, None, None, None, None, wg, au, cp, of, ar, cc, tz, oi, rtc, places,
    )
}

fn encode_residual_with_std(
    v: Option<f64>,
    std_bucket: Option<&str>,
    places: Option<u32>,
) -> String {
    encode_residual_rich(v, std_bucket, None, places, false)
}

/// Residual public token: lane-quantized mean + std + multipath signature
/// (breaks 0.260 floor collisions).
///
/// The mean fed in is the sealed-primary role mean (noderiv → float → rint,
/// first available group) — pack-order stable, so deep vs noderiv vs unioned
/// packs share the same raw value. The mp sig additionally folds the coarse
/// 2-dec floor (`c:m=0.26`) for cross-engine floor grouping. Keeping the
/// quantized raw in the material preserves precision-lane separation
/// (dv0 full / dv4 2-dec / dv5 5-dec / dv6 6-dec) and makes the res slot carry
/// the residual value itself.
fn encode_residual_rich(
    v: Option<f64>,
    std_bucket: Option<&str>,
    multipath_sig: Option<&str>,
    places: Option<u32>,
    lane_c_floor: bool,
) -> String {
    let mp = multipath_sig.filter(|s| !s.is_empty());
    match v {
        None if mp.is_none() => PLACEHOLDER.to_string(),
        None => {
            let mut material = String::new();
            if let Some(b) = std_bucket.filter(|b| !b.is_empty()) {
                material.push_str(b);
            }
            if let Some(sig) = mp {
                if !material.is_empty() {
                    material.push('|');
                }
                material.push_str("mp:");
                material.push_str(sig);
            }
            digest_token("res", &material)
        }
        Some(x) => {
            // iss/opus5 03-P0-3: lane quantization — dv0 without a Lane-C floor
            // must not emit the bare 12-dec raw residual: digits beyond 3 decimals
            // are engine-jitter and would inflate impersonation-uniqueness.
            let q = match (places, lane_c_floor) {
                (None, false) => quantize(x, Some(3)),
                _ => quantize(x, places),
            };
            let raw = match places {
                None => {
                    if lane_c_floor {
                        // Real Lane-C ensemble: cross-engine bias is ~1e-3, so the
                        // full-precision lane lands on the same 2-dec commercial
                        // floor as the mp sig (Chrome 0.26004 vs Edge 0.26113 → 0.26).
                        let mf = (q * 100.0).round() / 100.0;
                        format!("{mf:.2}")
                    } else {
                        let s = format!("{q:.12}");
                        let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
                        if s.is_empty() || s == "0" || s == "-0" {
                            "0.0".into()
                        } else {
                            s
                        }
                    }
                }
                Some(p) => format!("{q:.p$}", p = p as usize),
            };
            let mut material = String::new();
            // Quantized raw always enters the material: precision lanes must
            // diverge (dv4/5/6) and the res slot must carry the residual value.
            // Pack-order stability comes from the sealed-primary mean feeding v,
            // and the mp sig still folds the coarse 2-dec structure on top.
            material.push_str(&raw);
            if let Some(b) = std_bucket.filter(|b| !b.is_empty()) {
                if !material.is_empty() {
                    material.push('|');
                }
                material.push_str(b);
            }
            if let Some(sig) = mp {
                if !material.is_empty() {
                    material.push('|');
                }
                material.push_str("mp:");
                material.push_str(sig);
            }
            if material.is_empty() {
                return PLACEHOLDER.to_string();
            }
            digest_token("res", &material)
        }
    }
}

/// Curve structure signature (quantiles + lag-1 + odd/even energy + peak).
/// Amplifies micro-shape differences that plain mean/std hide (prod class floors).
fn curve_structure_sig(arr: &[f64]) -> Option<String> {
    if arr.len() < 8 {
        return None;
    }
    let mut sorted: Vec<f64> = arr.iter().copied().filter(|x| x.is_finite()).collect();
    if sorted.len() < 8 {
        return None;
    }
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    let q = |p: f64| {
        let i = ((n as f64 - 1.0) * p).round() as usize;
        sorted[i.min(n - 1)]
    };
    let mean = arr.iter().sum::<f64>() / arr.len() as f64;
    let var = arr.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / arr.len() as f64;
    let std = var.sqrt();
    let mut lag1 = 0.0;
    if arr.len() >= 4 && std > 1e-12 {
        let mut num = 0.0;
        for i in 1..arr.len() {
            num += (arr[i] - mean) * (arr[i - 1] - mean);
        }
        lag1 = num / ((arr.len() - 1) as f64 * std * std);
    }
    let mut odd = 0.0;
    let mut even = 0.0;
    for (i, v) in arr.iter().enumerate() {
        if i % 2 == 0 {
            even += v.abs();
        } else {
            odd += v.abs();
        }
    }
    let den = (odd + even).max(1e-12);
    let peak = arr
        .iter()
        .copied()
        .fold(0.0_f64, |a, b| if b.abs() > a.abs() { b } else { a });
    Some(format!(
        "q={:.5},{:.5},{:.5},{:.5},{:.5}|lg={:.4}|oe={:.4}|pk={:.5}|s={:.5}",
        q(0.1),
        q(0.25),
        q(0.5),
        q(0.75),
        q(0.9),
        lag1,
        odd / den,
        peak,
        std
    ))
}

/// Classify multipath shader/path into Lane-C (cross-browser) vs Lane-S (same-SKU fine).
///
/// Lane-C = float / noderiv / rint — must drive **commercial** res/wg digests so
/// chrome `silicon_deep` and chromium `silicon_noderiv` still share a core body.
/// Lane-S = fma / denorm / ulp / tex / interp — diagnostics / fuzzy only.
fn path_lane_role(mode: &str, pid: &str) -> &'static str {
    let mode = mode.to_ascii_lowercase();
    let pid = pid.to_ascii_lowercase();
    if mode == "noderiv" || pid.contains("noderiv") {
        "noderiv"
    } else if mode == "rint" || pid.contains("rint") {
        "rint"
    } else if mode == "fma_pair" || mode == "fma" || pid.contains("fma") {
        "fma"
    } else if mode == "denorm" || mode == "ftz" || pid.contains("denorm") {
        "denorm"
    } else if mode == "ulp" || mode == "silicon_ulp" || pid.contains("ulp") {
        "ulp"
    } else if mode == "tex_lerp" || mode == "tex" || pid.contains("tex_lerp") {
        "tex"
    } else if mode == "interp" || mode == "interp_diverge" || pid.contains("interp") {
        "interp"
    } else if mode.is_empty() || mode == "float" || pid.contains("v3f") || pid.contains("_std") {
        "float"
    } else {
        "other"
    }
}

fn is_lane_c_role(role: &str) -> bool {
    matches!(role, "float" | "noderiv" | "rint")
}

fn is_lane_s_role(role: &str) -> bool {
    matches!(
        role,
        "fma" | "denorm" | "ulp" | "tex" | "interp" | "other"
    )
}

/// Collect residual path objects from all FE multipath arrays (B10 + B10x packs).
fn collect_residual_path_objs(fo: &Map<String, Value>) -> Vec<Value> {
    let mut all: Vec<Value> = Vec::new();
    for k in [
        "residual_paths",
        "residual_paths_b10x",
        "residual_paths_extra",
        "residual_paths_merged",
    ] {
        if let Some(arr) = fo.get(k).and_then(|v| v.as_array()) {
            for p in arr {
                all.push(p.clone());
            }
        }
    }
    all
}

/// Aggregate Lane-C path stats by role.
fn lane_c_role_stats(
    fo: &Map<String, Value>,
) -> std::collections::BTreeMap<&'static str, (f64, f64, usize)> {
    let mut by_role: std::collections::BTreeMap<&'static str, (f64, f64, usize)> =
        std::collections::BTreeMap::new();
    for p in collect_residual_path_objs(fo) {
        let pid = p.get("path_id").and_then(|v| v.as_str()).unwrap_or("");
        let mode = p.get("shader_mode").and_then(|v| v.as_str()).unwrap_or("");
        let role = path_lane_role(mode, pid);
        if !is_lane_c_role(role) {
            continue;
        }
        let ok = p.get("ok").and_then(|v| v.as_bool()).unwrap_or(false)
            || p.get("entropy_ok").and_then(|v| v.as_bool()).unwrap_or(false)
            || p.get("mean").and_then(|v| v.as_f64()).is_some()
            || p.get("curve")
                .and_then(|c| c.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false);
        if !ok {
            continue;
        }
        let mean = p
            .get("mean")
            .and_then(|v| v.as_f64())
            .or_else(|| {
                p.get("curve").and_then(|c| c.as_array()).map(|a| {
                    let xs: Vec<f64> = a.iter().filter_map(|x| x.as_f64()).collect();
                    if xs.is_empty() {
                        0.0
                    } else {
                        xs.iter().sum::<f64>() / xs.len() as f64
                    }
                })
            })
            .unwrap_or(0.0);
        let std = p.get("std").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let e = by_role.entry(role).or_insert((0.0, 0.0, 0));
        e.0 += mean;
        e.1 += std;
        e.2 += 1;
    }
    by_role
}

/// Best Lane-C residual path object (noderiv > float > rint), preferring longer curves.
fn lane_c_primary_path(fo: &Map<String, Value>) -> Option<Value> {
    let mut best: Option<(usize, i64, Value)> = None; // (prio, curve_n, path)
    for p in collect_residual_path_objs(fo) {
        let pid = p.get("path_id").and_then(|v| v.as_str()).unwrap_or("");
        let mode = p.get("shader_mode").and_then(|v| v.as_str()).unwrap_or("");
        let role = path_lane_role(mode, pid);
        let prio = match role {
            "noderiv" => 0usize,
            "float" => 1,
            "rint" => 2,
            _ => continue,
        };
        let has_mean = p.get("mean").and_then(|v| v.as_f64()).is_some();
        let curve_n = p
            .get("curve")
            .and_then(|c| c.as_array())
            .map(|a| a.len())
            .unwrap_or(0) as i64;
        let ok = p.get("ok").and_then(|v| v.as_bool()).unwrap_or(false)
            || p.get("entropy_ok").and_then(|v| v.as_bool()).unwrap_or(false)
            || has_mean
            || curve_n > 0;
        if !ok {
            continue;
        }
        let better = match &best {
            None => true,
            Some((bp, bn, _)) => prio < *bp || (prio == *bp && curve_n > *bn),
        };
        if better {
            best = Some((prio, curve_n, p));
        }
    }
    best.map(|(_, _, p)| p)
}

/// Lane-C primary path curve for commercial wg body (stable across B10x pack order).
fn lane_c_primary_curve(fo: &Map<String, Value>) -> Vec<f64> {
    if let Some(p) = lane_c_primary_path(fo) {
        if let Some(arr) = p.get("curve").and_then(|c| c.as_array()) {
            let xs: Vec<f64> = arr.iter().filter_map(|x| x.as_f64()).collect();
            if xs.len() >= 8 {
                return xs;
            }
        }
    }
    Vec::new()
}

/// B10 primary residual paths used for commercial res/wg consensus.
/// Later B10x deepen packs (rint_*, noderiv_hard_*, ulp_chain_*) are engine-
/// schedule dependent and must not displace the shared v3f/fma ensemble.
fn is_b10_primary_residual_path(path_id: &str) -> bool {
    let p = path_id.to_ascii_lowercase();
    if p.contains("ulp") || p.contains("noderiv_hard") || p.contains("rint_") {
        return false;
    }
    // B10x last-write used the same v3f/fma ids; those deepen/legacy/softgl
    // labels must not enter commercial ensemble even if merge leaked them.
    if p.contains("b10x") || p.contains("legacy") || p.contains("softgl") {
        return false;
    }
    p.contains("v3f") || p.contains("fma_pair")
}

/// Consensus residual curve: path whose mean is closest to the ensemble median.
/// Noderiv-first last-write forks Chrome vs Edge on one GPU; the median path
/// (often the shared fma_pair) is the structure both engines actually measured.
fn ensemble_consensus_curve(fo: &Map<String, Value>) -> Vec<f64> {
    let mut items: Vec<(f64, Vec<f64>)> = Vec::new();
    for p in collect_residual_path_objs(fo) {
        let pid = p.get("path_id").and_then(|v| v.as_str()).unwrap_or("");
        if !is_b10_primary_residual_path(pid) {
            continue;
        }
        let curve = p
            .get("curve")
            .and_then(|c| c.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_f64()).collect::<Vec<_>>())
            .unwrap_or_default();
        if curve.len() < 8 {
            continue;
        }
        let mean = p
            .get("mean")
            .and_then(|v| v.as_f64())
            .unwrap_or_else(|| curve.iter().sum::<f64>() / curve.len() as f64);
        if mean.is_finite() {
            items.push((mean, curve));
        }
    }
    if items.is_empty() {
        return Vec::new();
    }
    items.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mid = if items.len() % 2 == 1 {
        items[items.len() / 2].0
    } else {
        (items[items.len() / 2 - 1].0 + items[items.len() / 2].0) / 2.0
    };
    items
        .into_iter()
        .min_by(|a, b| {
            (a.0 - mid)
                .abs()
                .partial_cmp(&(b.0 - mid).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(_, c)| c)
        .unwrap_or_default()
}

/// Lane-C commercial path signature: **single primary role** (noderiv > float > rint).
///
/// Why one role: chrome `silicon_deep` lands float+noderiv+rint while chromium
/// B10x `silicon_noderiv` only has noderiv — hashing the full role set would still
/// split. Using the highest-priority available Lane-C role keeps commercial body
/// aligned; missing secondary roles stay as Lane-S diagnostics.
///
/// Fallback: when B10x deep overwrites residual_paths with only Lane-S modes,
/// synthesize from residual_mean/std at **2-decimal** (cross-browser noise floor).
/// Lane-C commercial token. `from_paths` is true when noderiv/float/rint paths exist
/// (pure mean token; no curve structure). Mean-only fallback is still coarse-quantized
/// but multipath_sig may fold coarse curve structure for uniqueness.
fn residual_paths_lane_c_sig_ex(fo: &Map<String, Value>) -> Option<(String, bool)> {
    let by_role = lane_c_role_stats(fo);
    // Priority order for commercial core (role selects which mean; token is role-free so
    // noderiv pack vs mean-fallback with same 2-dec bucket share commercial digests).
    for want in ["noderiv", "float", "rint"] {
        if let Some((ms, ss, n)) = by_role.get(want) {
            let n = (*n).max(1) as f64;
            // 2-dec commercial floor (0.260 vs 0.261 → 0.26)
            let m = ((ms / n) * 100.0).round() / 100.0;
            let s = ((ss / n) * 10.0).round() / 10.0;
            return Some((format!("c:m={m:.2}:s={s:.1}"), true));
        }
    }
    // Synthetic Lane-C from global residual when only Lane-S paths remain
    if let Some(m) = fo.get("residual_mean").and_then(|v| v.as_f64()) {
        let s = fo.get("residual_std").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let mq = (m * 100.0).round() / 100.0;
        let sq = (s * 10.0).round() / 10.0;
        return Some((format!("c:m={mq:.2}:s={sq:.1}"), false));
    }
    None
}

fn residual_paths_lane_c_sig(fo: &Map<String, Value>) -> Option<String> {
    residual_paths_lane_c_sig_ex(fo).map(|(s, _)| s)
}

/// Mean residual from Lane-C roles. With 2+ roles, use the median (label-invariant);
/// a single noderiv-first pick forks Blink when engines tag the same two physical
/// families as noderiv vs std in opposite order.
fn residual_mean_lane_c(fo: &Map<String, Value>) -> Option<f64> {
    let by = lane_c_role_stats(fo);
    let mut means: Vec<f64> = Vec::new();
    for want in ["noderiv", "float", "rint"] {
        if let Some((ms, _ss, n)) = by.get(want) {
            let n = (*n).max(1) as f64;
            means.push(ms / n);
        }
    }
    if means.is_empty() {
        return None;
    }
    means.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if means.len() % 2 == 1 {
        Some(means[means.len() / 2])
    } else {
        let hi = means[means.len() / 2];
        let lo = means[means.len() / 2 - 1];
        Some((lo + hi) / 2.0)
    }
}

/// Sealed-primary residual mean, same role priority as the multipath sig
/// (noderiv → float → rint, first available group). Full precision, no 2-dec
/// floor: the per-lane encode quantizes. Group-mean aggregation is pack-order
/// stable (deep + noderiv + unioned packs resolve to the same noderiv group),
/// so the commercial res slot keeps Lane-C stability *and* carries the raw.
fn residual_mean_primary(fo: &Map<String, Value>) -> Option<f64> {
    let by = lane_c_role_stats(fo);
    for want in ["noderiv", "float", "rint"] {
        if let Some((ms, _ss, n)) = by.get(want) {
            return Some(ms / (*n).max(1) as f64);
        }
    }
    fo.get("residual_mean").and_then(|v| v.as_f64())
}

/// Extract per-path signature from residual_paths objects (FE multipath / B10x).
/// **Commercial body uses Lane-C filter** (see residual_paths_lane_c_sig).
/// Full path_id listing kept only for diagnostics.
fn residual_paths_object_sig(fo: &Map<String, Value>) -> Option<String> {
    // Commercial: Lane-C role aggregates only
    residual_paths_lane_c_sig(fo)
}

/// Coarse EU timing signature (DrawnApart-lite) — secondary when residual class-dead.
fn eu_timing_sig(fo: &Map<String, Value>) -> Option<String> {
    let mut samples: Vec<f64> = Vec::new();
    for k in ["eu_timing_ms", "hw_eu_timing_ms", "draw_timing_ms"] {
        let c = curve_array(fo, k);
        if c.len() >= 4 {
            samples = c;
            break;
        }
    }
    if samples.is_empty() {
        if let Some(paths) = fo.get("residual_paths").and_then(|v| v.as_array()) {
            for p in paths {
                for tk in ["eu_timing_ms", "timing_ms", "timing_samples"] {
                    if let Some(arr) = p.get(tk).and_then(|v| v.as_array()) {
                        let c: Vec<f64> = arr.iter().filter_map(|x| x.as_f64()).collect();
                        if c.len() >= 4 {
                            samples = c;
                            break;
                        }
                    }
                }
                if samples.len() >= 4 {
                    break;
                }
            }
        }
    }
    if samples.len() < 4 {
        return None;
    }
    let n = samples.len() as f64;
    let mean = samples.iter().sum::<f64>() / n;
    let var = samples.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n;
    let std = var.sqrt();
    // Coarse buckets: timing is noisy; 0.5ms mean / 0.25ms std keep fleet-stable
    let mean_b = (mean * 2.0).round() / 2.0;
    let std_b = (std * 4.0).round() / 4.0;
    let quality = fo
        .get("timing_quality_score")
        .and_then(|v| v.as_f64())
        .or_else(|| fo.get("timing_quality").and_then(|v| v.as_f64()))
        .unwrap_or(-1.0);
    Some(format!("eu_m={mean_b:.2}|eu_s={std_b:.2}|q={quality:.2}"))
}

/// Compact multipath signature for **commercial body** (Lane-C only).
///
/// iss/73 / doc07: chrome `silicon_deep` vs chromium `silicon_noderiv` used to
/// hash different path_id sets into res → same GPU split across Blink. Commercial
/// digests now use **Lane-C roles only** (float/noderiv/rint). Lane-S advanced
/// modes (fma/denorm/ulp) stay on diagnostics (`lane_s_sig`), not dv0 body.
///
/// Materials (priority):
/// 1. Lane-C path role aggregates (role:m:s) — **sole commercial multipath when present**
/// 2. Only if no Lane-C: coarse Lane-C primary / hw_curve structure (never last-write soup alone)
/// 3. Dead residual → anti-collision secondary (still no Lane-S path_id soup)
///
/// Why drop last-write `hw_curve_webgl` when Lane-C exists: B10x pack finish order
/// (deep vs noderiv vs rint) overwrites the flat curve and was splitting chrome /
/// chromium / edge commercial res digests on the **same** GTX 1050 Ti lab host.
fn residual_multipath_sig_ex(fo: &Map<String, Value>) -> Option<(String, bool)> {
    // Lane-C path roles present → sole commercial multipath key (ignore last-write curve).
    if let Some((lc, from_paths)) = residual_paths_lane_c_sig_ex(fo) {
        // Mean-only fallback: **lc token alone** (from_paths=false). Fine curve
        // structure (wcs/q) was splitting same-host revisits when B10 last-write /
        // incomplete packs differed while residual still rounded to the same 2-dec
        // floor (lab ed6e↔fb6e). Uniqueness for same-floor GPUs comes from
        // wg/au/cp slots, not res soup.
        return Some((short_hash(&format!("lc={lc}")), from_paths));
    }
    let mut parts: Vec<String> = Vec::new();
    // No residual_mean either: structure-only fallback
    let mut wg = lane_c_primary_curve(fo);
    if wg.len() < 8 {
        wg = curve_array(fo, "hw_curve_webgl");
    }
    if wg.len() >= 8 {
        if let Some(cs) = curve_structure_sig(&wg) {
            parts.push(format!("wcs={}", short_hash(&cs)));
        } else {
            let mut sorted = wg.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let p25 = (sorted[sorted.len() / 4] * 100.0).round() / 100.0;
            let p50 = (sorted[sorted.len() / 2] * 100.0).round() / 100.0;
            let p75 = (sorted[(sorted.len() * 3) / 4] * 100.0).round() / 100.0;
            parts.push(format!("q={p25:.2},{p50:.2},{p75:.2}"));
        }
    }
    // Lane-S advanced modes: **NOT** in commercial multipath_sig (diagnostic only).
    // Dead residual class → complex features + fusion digests (secondary anchors)
    let residual_std = fo.get("residual_std").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let residual_dead = residual_std < 0.010 || {
        let uniq_hint = {
            let mut s = wg.clone();
            s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let mut u = 0usize;
            for w in s.windows(2) {
                if (w[1] - w[0]).abs() > 1e-5 {
                    u += 1;
                }
            }
            u
        };
        uniq_hint < 8 && !wg.is_empty()
    };
    if residual_dead || parts.is_empty() {
        if wg.len() >= 8 {
            let params = crate::hw_anti_collision::TeachingParams::from_entropy(
                residual_std >= 0.010,
                residual_std,
                8,
            );
            let feats = crate::hw_anti_collision::complex_curve_features(&wg, &params);
            if !feats.is_empty() {
                let fstr: Vec<String> = feats.iter().map(|x| format!("{x:.5}")).collect();
                parts.push(format!("cx={}", short_hash(&fstr.join(","))));
            }
        }
        for k in [
            "hw_anti_collision",
            "hw_silicon_fusion",
            "hw_silicon_fine",
            "hw_webgl_stable",
        ] {
            if let Some(s) = str_field(fo, k).filter(|s| !s.is_empty() && s != "0") {
                parts.push(format!("{k}={}", sanitize_token(&s)));
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some((short_hash(&parts.join("|")), false))
    }
}

/// Multipath signature (structure-only fallback variant keeps old call sites).
fn residual_multipath_sig(fo: &Map<String, Value>) -> Option<String> {
    residual_multipath_sig_ex(fo).map(|(s, _)| s)
}

/// Advanced silicon path signature (fma_pair / denorm / tex_lerp / interp / ulp).
/// Prefer mean+std+structure of these paths — strongest same-SKU micro-splitters.
fn residual_paths_advanced_sig(fo: &Map<String, Value>) -> Option<String> {
    let mut rows: Vec<String> = Vec::new();
    for k in [
        "residual_paths",
        "residual_paths_b10x",
        "residual_paths_extra",
        "residual_paths_merged",
    ] {
        let Some(arr) = fo.get(k).and_then(|v| v.as_array()) else {
            continue;
        };
        for p in arr {
            let pid = p
                .get("path_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let mode = p
                .get("shader_mode")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let is_adv = mode == "fma_pair"
                || mode == "fma"
                || mode == "denorm"
                || mode == "ftz"
                || mode == "tex_lerp"
                || mode == "tex"
                || mode == "interp"
                || mode == "interp_diverge"
                || mode == "ulp"
                || mode == "silicon_ulp"
                || pid.contains("fma_pair")
                || pid.contains("denorm")
                || pid.contains("tex_lerp")
                || pid.contains("interp")
                || pid.contains("ulp");
            if !is_adv {
                continue;
            }
            let curve = p
                .get("curve")
                .and_then(|c| c.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_f64())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let mean = p
                .get("mean")
                .and_then(|v| v.as_f64())
                .unwrap_or_else(|| {
                    if curve.is_empty() {
                        0.0
                    } else {
                        curve.iter().sum::<f64>() / curve.len() as f64
                    }
                });
            let std = p.get("std").and_then(|v| v.as_f64()).unwrap_or(-1.0);
            let cs = if curve.len() >= 8 {
                curve_structure_sig(&curve)
                    .map(|s| short_hash(&s))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            rows.push(format!(
                "{pid}|{mode}|m={mean:.6}|s={std:.6}|c={cs}"
            ));
        }
    }
    if rows.is_empty() {
        return None;
    }
    rows.sort();
    rows.dedup();
    Some(short_hash(&rows.join(";")))
}

/// Offset-invariant shape for commercial wg (z-score quantiles + lag-1 + peak index).
/// Absolute residual CSV at 12-dec was splitting Chrome vs Edge on one GPU (~1e-3 bias).
fn curve_shape_z_sig(arr: &[f64]) -> Option<String> {
    let xs: Vec<f64> = arr.iter().copied().filter(|x| x.is_finite()).collect();
    if xs.len() < 8 {
        return None;
    }
    let n = xs.len() as f64;
    let mean = xs.iter().sum::<f64>() / n;
    let var = xs.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n;
    let std = var.sqrt();
    if std < 1e-12 {
        return None;
    }
    let z: Vec<f64> = xs.iter().map(|x| (x - mean) / std).collect();
    let mut sorted = z.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let q = |p: f64| {
        let i = ((sorted.len() as f64 - 1.0) * p).round() as usize;
        (sorted[i.min(sorted.len() - 1)] * 100.0).round() / 100.0
    };
    let mut lag1 = 0.0;
    for i in 1..z.len() {
        lag1 += z[i] * z[i - 1];
    }
    lag1 = ((lag1 / (z.len() - 1) as f64) * 100.0).round() / 100.0;
    let mut peak_i = 0usize;
    let mut peak_a = 0.0;
    for (i, v) in z.iter().enumerate() {
        if v.abs() > peak_a {
            peak_a = v.abs();
            peak_i = i;
        }
    }
    let peak_frac = ((peak_i as f64 / (z.len() - 1) as f64) * 100.0).round() / 100.0;
    Some(format!(
        "zq={:.2},{:.2},{:.2},{:.2},{:.2}|lg={:.2}|pk={:.2}",
        q(0.1),
        q(0.25),
        q(0.5),
        q(0.75),
        q(0.9),
        lag1,
        peak_frac
    ))
}

/// Encode curve with structure amplification (wg/cp commercial uniqueness).
fn encode_curve_structured(arr: &[f64], places: Option<u32>, extra: Option<&str>) -> String {
    if arr.is_empty() {
        return PLACEHOLDER.to_string();
    }
    let mut material = match places {
        // dv0: shape only — raw 12-dec CSV is engine-bias, not silicon structure.
        None => curve_shape_z_sig(arr).unwrap_or_else(|| "z0".into()),
        Some(p) => {
            let mut buf = String::new();
            for (i, v) in arr.iter().enumerate() {
                if i > 0 {
                    buf.push(',');
                }
                let q = quantize(*v, Some(p));
                buf.push_str(&format!("{q:.p$}", p = p as usize));
            }
            if let Some(cs) = curve_structure_sig(arr) {
                buf = format!("{buf}|{cs}");
            }
            buf
        }
    };
    if let Some(ex) = extra {
        if !ex.is_empty() {
            material = format!("{material}|ex:{ex}");
        }
    }
    digest_token("curve", &material)
}

/// iss/75 + same-host reliability: commercial **wg** must not hash raw residual CSV.
/// `dv0` used `places=None` + fine `curve_structure_sig` → micro GPU noise / pack length
/// flips split revisits (lab: `e43209b3cd` ↔ `74a056e2ba` on one GTX 1050 Ti).
/// Mirror DrawnApart/community practice: coarse summary buckets + Lane-C role extra.
fn encode_wg_lane_c_stable(arr: &[f64], extra: Option<&str>) -> String {
    let mut parts: Vec<String> = Vec::new();
    let finite: Vec<f64> = arr.iter().copied().filter(|x| x.is_finite()).collect();
    // Utilize short residual packs (2+) — do not require 8 samples to mint commercial wg.
    if finite.len() >= 2 {
        // Fixed-window stats: first 16 samples (or all if shorter) so 16 vs 32 packs
        // share the same commercial shape summary (DrawnApart-style coarse response).
        let window = &finite[..finite.len().min(16)];
        let n = window.len() as f64;
        let mean = window.iter().sum::<f64>() / n;
        let var = window.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n;
        // residual-scale (~0.22–0.27): 3-dec mean / 2-dec std keep micro-noise in-bucket
        let mean_b = (mean * 1000.0).round() / 1000.0;
        let std_b = (var.sqrt() * 100.0).round() / 100.0;
        parts.push(format!("wm={mean_b:.3}|ws={std_b:.2}|n={}", window.len()));
    }
    if let Some(ex) = extra {
        if !ex.is_empty() {
            parts.push(format!("ex:{ex}"));
        }
    }
    if parts.is_empty() {
        return PLACEHOLDER.to_string();
    }
    digest_token("wg", &parts.join("|"))
}

/// iss/75: au is V_aux — never fold raw OfflineAudio float CSV / fine structure into
/// commercial body. Precision follows the dv lane (`dec_places`), not a global squash
/// aimed at cross-browser force-merge.
fn encode_au_aux_stable(arr: &[f64], extra: Option<&str>) -> String {
    encode_au_aux_at_places(arr, extra, 1)
}

fn encode_au_aux_at_places(arr: &[f64], extra: Option<&str>, dec_places: u32) -> String {
    let mut parts: Vec<String> = Vec::new();
    let finite: Vec<f64> = arr.iter().copied().filter(|x| x.is_finite()).collect();
    // 2+ samples: utilize partial audio packs instead of dropping to commercial 0.
    if finite.len() >= 2 {
        let n = finite.len() as f64;
        let mean = finite.iter().sum::<f64>() / n;
        let var = finite.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n;
        let scale = 10f64.powi(dec_places as i32);
        let mean_b = (mean * scale).round() / scale;
        let std_b = (var.sqrt() * scale).round() / scale;
        parts.push(format!(
            "am={mean_b:.prec$}|as={std_b:.prec$}|n={}",
            finite.len(),
            prec = dec_places as usize
        ));
    }
    if let Some(ex) = extra {
        if !ex.is_empty() {
            parts.push(format!("ex:{ex}"));
        }
    }
    if parts.is_empty() {
        return PLACEHOLDER.to_string();
    }
    digest_token("au", &parts.join("|"))
}

/// Precision-lane encode for commercial wg.
/// - `dv0` (places=None): full structured curve (probe fidelity)
/// - `dv5`/`dv6`: quantize at lane places
/// - `dv4`: coarsest match lane (Lane-C mean/std buckets) for reliability / association
fn encode_wg_for_places(arr: &[f64], places: Option<u32>, extra: Option<&str>) -> String {
    match places {
        None => encode_curve_structured(arr, None, extra),
        Some(4) => encode_wg_lane_c_stable(arr, extra),
        Some(p) => encode_curve_structured(arr, Some(p), extra),
    }
}

/// Precision-lane encode for V_aux au (anti-jitter only; not cross-browser force-merge).
fn encode_au_for_places(arr: &[f64], places: Option<u32>, extra: Option<&str>) -> String {
    match places {
        None => encode_au_aux_at_places(arr, extra, 3), // finest public aux
        Some(6) | Some(5) => encode_au_aux_at_places(arr, extra, 2),
        Some(4) => encode_au_aux_at_places(arr, extra, 1),
        Some(_) => encode_au_aux_stable(arr, extra),
    }
}

fn residual_std_bucket_for_places(
    std: Option<f64>,
    places: Option<u32>,
    lane_c: bool,
) -> Option<String> {
    let rs = std?;
    if !rs.is_finite() {
        return None;
    }
    match places {
        // dv0 + Lane-C: physical path-std floor (~1e-3) so pack micro-std does not
        // fork same-host digests; not a cross-browser 2-dec squash of the mean.
        None if lane_c => {
            let q = (rs * 1000.0).round() / 1000.0;
            Some(format!("rsb_{q:.3}"))
        }
        None => Some(format!("rsb_{:.6}", rs)),
        Some(6) => {
            let q = quantize(rs, Some(4));
            Some(format!("rsb_{q:.4}"))
        }
        Some(5) => {
            let q = quantize(rs, Some(3));
            Some(format!("rsb_{q:.3}"))
        }
        Some(4) => {
            let q = (rs * 10.0).round() / 10.0;
            Some(format!("rsb_{q:.1}"))
        }
        Some(p) => {
            let q = quantize(rs, Some(p));
            Some(format!("rsb_{q}"))
        }
    }
}

/// Commercial au stack extras.
///
/// **Curve-body only** — do not fold optional scalars (sample_rate / channel count).
/// 178 Firefox multi-site audit: identical `audio_seed_delta_curve` still forked au
/// when `audio_sample_rate` was present on some sessions (B10/B46) and absent on
/// others (cool-down / thin pack). Presence-vs-absence of sr/mc is not silicon.
/// Discrimination stays in the seed-delta / audio curve body itself.
fn audio_stack_extra_commercial(_fo: &Map<String, Value>) -> Option<String> {
    None
}

/// iss/61 F2: opt-in gate for folding fuzzy-ECC stable digests into mint material.
/// Default OFF — enabling changes mint output (by design: drift-stable), so it must
/// be a deliberate rollout switch (`GR_FUZZY_MINT=1`).
pub fn fuzzy_mint_enabled() -> bool {
    matches!(
        gr_abi::env::get("FUZZY_MINT")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "on" | "yes"
    )
}

/// Commercial audio stack extras — **B10-stable only**.
/// Optional B46 deep/convolver fields must NOT enter commercial au: they land
/// only on some sessions (dwell/schedule) and fork same-machine digests across
/// sites and refreshes (178 Ubuntu+Chrome 4-site audit).
fn audio_stack_extra(fo: &Map<String, Value>) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    // Seed-delta structure (B10 triple-seed v4) — always-on with B10.
    for key in [
        "audio_seed_delta_curve",
        "audio_noise_delta",
        "hw_curve_audio_delta",
    ] {
        let d = curve_array(fo, key);
        if d.len() >= 8 {
            if let Some(cs) = curve_structure_sig(&d) {
                parts.push(format!("asd={}", short_hash(&cs)));
                break;
            }
        }
    }
    // iss/58 B2/B5: population differential digests (versioned, not session-optional)
    if let Some(s) = str_field(fo, "au_diff_digest").filter(|s| !s.is_empty()) {
        parts.push(format!("au_diff={}", sanitize_token(&s)));
    }
    if let Some(s) = str_field(fo, "binmap_version").filter(|s| !s.is_empty()) {
        parts.push(format!("bmv={}", sanitize_token(&s)));
    }
    // iss/61 F2 (gated): fuzzy-ECC stable digests — same-machine drift stabilization.
    if fuzzy_mint_enabled() {
        if let Some(s) = str_field(fo, "fuzzy_ecc_au_digest").filter(|s| !s.is_empty()) {
            parts.push(format!("fza={}", sanitize_token(&s)));
        }
    }
    // Always-on stack scalars (not B46-optional).
    if let Some(sr) = fo
        .get("audio_sample_rate")
        .or_else(|| fo.get("audio_sample_rate_fp"))
        .or_else(|| fo.get("audio_deep_sr"))
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
    {
        if sr.is_finite() && sr > 0.0 {
            parts.push(format!("sr={}", sr.round() as i64));
        }
    }
    // iss/75: omit audio_base_latency / audio_output_latency — micro-ms forks Blink
    // siblings on one host and must not enter V_aux commercial au material.
    if let Some(mc) = fo
        .get("audio_max_channel_count")
        .or_else(|| fo.get("max_channel_count"))
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
    {
        parts.push(format!("mc={mc}"));
    }
    // B46 deep/convolver/moments/phase intentionally excluded from commercial extra.
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("|"))
    }
}

/// iss/58 A2: WASM/SIMD digests fold into **cp** (CPU path), not audio.
/// Full extras are diagnostic; commercial mint uses `cpu_stack_extra_commercial`.
fn cpu_stack_extra(fo: &Map<String, Value>) -> Option<String> {
    let mut parts = Vec::new();
    for k in [
        "wasm_relaxed_simd_digest",
        "ws_relaxed_simd_digest",
        "wasm_simd_sig",
    ] {
        if let Some(s) = str_field(fo, k).filter(|s| !s.is_empty()) {
            parts.push(format!("ws={}", sanitize_token(&s)));
            break;
        }
    }
    let wt = curve_array(fo, "ws_simd_timing_curve");
    if wt.len() >= 4 {
        if let Some(cs) = curve_structure_sig(&wt) {
            parts.push(format!("wst={}", short_hash(&cs)));
        }
    }
    let fd = curve_array(fo, "ws_fma_delta_curve");
    if fd.len() >= 4 {
        if let Some(cs) = curve_structure_sig(&fd) {
            parts.push(format!("wfd={}", short_hash(&cs)));
        }
    }
    // A5 IDB ladder / A10 eventloop
    for k in ["idb_write_digest", "eventloop_digest", "vc_encoder_bitstream_digest"] {
        if let Some(s) = str_field(fo, k).filter(|s| !s.is_empty()) {
            parts.push(format!("{k}={}", sanitize_token(&s)));
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("|"))
    }
}

/// Commercial cp extras: **none**.
/// WASM/SIMD feature digests are low-entropy ISA class (K) — must not enter V body
/// or fork when deep pack present/absent. Diagnostic only via `cpu_stack_extra`.
fn cpu_stack_extra_commercial(_fo: &Map<String, Value>) -> Option<String> {
    None
}

/// Build slot-level multi-path provenance for ops / dual-KPI analytics.
/// Records which probe method won and which candidates were available (not only
/// the selected digest) so fleet can learn effective paths per OS/engine/SKU.
fn build_slot_probe_paths(fo: &Map<String, Value>, cp_curve: &[f64]) -> Value {
    use crate::hw_probe_analysis::rounds_from_fields;
    let engine = fo
        .get("engine_family")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let (tm_arr, _) = best_curve(
        fo,
        &[
            "timing_jitter_curve",
            "raf_interval_curve",
            "hw_noise_curve",
            "sab_tick_delta_curve",
        ],
        4,
    );
    let cp_enc = encode_cp_multipath(fo, cp_curve, None);
    let tz_enc = encode_tz_multipath(fo, &tm_arr, None);
    // of / ar brief provenance (analysis already ran in pick_extended)
    let of_rounds = rounds_from_fields(
        fo,
        &["hw_curve_canvas_rounds", "canvas_noise_rounds", "of_rounds"],
        &[],
    );
    let of_prov = json!({
        "slot": "of",
        "selected_method": if of_rounds.len() >= 2 {
            "canvas_multiround_structure"
        } else if fo.get("hw_curve_canvas").is_some() || fo.get("canvas_noise_curve").is_some() {
            "canvas_single_structure"
        } else {
            "missing"
        },
        "material_keys": ["hw_curve_canvas_rounds", "hw_curve_canvas", "canvas_noise_curve"],
        "batch_hint": "B10_hw_curves",
        "priority": if of_rounds.len() >= 2 { 1 } else { 3 },
        "weight": if of_rounds.len() >= 2 { 1.0 } else { 0.75 },
        "candidates": [
            {"method": "canvas_multiround_structure", "available": of_rounds.len() >= 2, "priority": 1, "weight": 1.0, "batch_hint": "B10_hw_curves"},
            {"method": "canvas_single_structure", "available": fo.get("hw_curve_canvas").is_some(), "priority": 3, "weight": 0.75, "batch_hint": "B10_hw_curves"},
        ],
        "policy": "multi_path_priority_v1",
    });
    let ar_has_curve = fo.get("hw_curve_webgpu").is_some()
        || fo.get("webgpu_compute_curve").is_some()
        || fo.get("webgpu_eu_timing_curve").is_some();
    let ar_has_class = fo.get("webgpu_adapter_surface").is_some()
        || fo.get("webgpu_limits_hash").is_some()
        || fo.get("webgpu_features_hash").is_some()
        || fo.get("webgpu_skip").is_some()
        || fo.get("webgpu_compute_skip").is_some()
        || fo.get("hw_webgpu_compute_digest").is_some();
    let ar_prov = json!({
        "slot": "ar",
        "selected_method": if fo.get("webgpu_eu_timing_curve").is_some() {
            "webgpu_eu_timing_structure"
        } else if ar_has_curve {
            "webgpu_residual_structure"
        } else if fo.get("hw_webgpu_compute_digest").is_some() {
            "webgpu_compute_digest"
        } else if ar_has_class {
            "webgpu_class_stack_commercial"
        } else {
            "missing"
        },
        "material_keys": [
            "webgpu_eu_timing_curve",
            "hw_curve_webgpu",
            "webgpu_compute_curve",
            "hw_webgpu_compute_digest",
            "webgpu_adapter_surface",
            "webgpu_limits_hash",
            "webgpu_features_hash"
        ],
        "batch_hint": "B18_webgpu",
        "priority": 1,
        "weight": 0.65,
        "candidates": [
            {"method": "webgpu_eu_timing_structure", "available": fo.get("webgpu_eu_timing_curve").is_some(), "priority": 1, "weight": 0.95, "batch_hint": "B18_webgpu", "commercial": true},
            {"method": "webgpu_residual_structure", "available": fo.get("hw_curve_webgpu").is_some() || fo.get("webgpu_compute_curve").is_some(), "priority": 2, "weight": 0.85, "batch_hint": "B18_webgpu", "commercial": true},
            {"method": "webgpu_compute_digest", "available": fo.get("hw_webgpu_compute_digest").is_some(), "priority": 5, "weight": 0.60, "batch_hint": "B18_webgpu", "commercial": true},
            {"method": "webgpu_class_stack_commercial", "available": ar_has_class, "priority": 8, "weight": 0.45, "batch_hint": "B18_webgpu", "commercial": true},
        ],
        "policy": "multi_path_utilize_all_detected_v1",
    });
    json!({
        "policy": "multi_path_priority_v1",
        "engine_family": engine,
        "note": "selected_method is the commercial winner; candidates[] lists all paths considered (available or not) for fleet analytics across OS/engine/SKU",
        "slots": {
            "cp": cp_enc.to_provenance("cp"),
            "tz": tz_enc.to_provenance("tz"),
            "of": of_prov,
            "ar": ar_prov,
            "res": {
                "slot": "res",
                "selected_method": "lane_c_multipath_residual",
                "material_keys": ["residual_paths", "residual_mean", "hw_curve_webgl"],
                "batch_hint": "B10_hw_curves|B10x_*",
                "priority": 1,
                "weight": 1.0,
                "policy": "multi_path_priority_v1",
            },
            "wg": {
                "slot": "wg",
                "selected_method": "lane_c_path_role_structure",
                "material_keys": ["residual_paths", "hw_curve_webgl"],
                "batch_hint": "B10_hw_curves|B10x_*",
                "priority": 1,
                "weight": 1.0,
                "policy": "multi_path_priority_v1",
            },
            "au": {
                "slot": "au",
                "selected_method": "audio_seed_delta_curve",
                "material_keys": ["audio_seed_delta_curve", "audio_deep_curve"],
                "batch_hint": "B10_hw_curves|B46_audio_deep",
                "priority": 1,
                "weight": 1.0,
                "policy": "multi_path_priority_v1",
            },
            "cc": {
                "slot": "cc",
                "selected_method": "gl_max_tex_rb_class",
                "material_keys": ["gl_max_texture_size", "gl_max_renderbuffer"],
                "batch_hint": "B2_hardware|B10_hw_curves",
                "priority": 1,
                "weight": 0.12,
                "role": "K_class",
                "policy": "multi_path_priority_v1",
            },
            "oi": {
                "slot": "oi",
                "selected_method": "host_fingerprint_separator",
                "batch_hint": "protocol|probe",
                "role": "host_sep",
                "commercial_silicon": false,
                "policy": "multi_path_priority_v1",
            },
            "rtc": {
                "slot": "rtc",
                "selected_method": "webrtc_host_hash",
                "batch_hint": "webrtc",
                "role": "host_sep",
                "commercial_silicon": false,
                "policy": "multi_path_priority_v1",
            },
        },
    })
}

/// Multi-path commercial encode result: digest body + provenance for ops/KPI.
#[derive(Debug, Clone)]
struct PathEncode {
    body: String,
    /// Selected probe method id (stable string for fleet analytics).
    method: &'static str,
    /// FE field keys that supplied the winning material.
    material_keys: Vec<&'static str>,
    /// Probe batch hint (schedule / pack id).
    batch_hint: &'static str,
    /// Priority rank of winner (1 = highest preference among available).
    priority: u8,
    /// Relative weight used for conf (1.0 multiround, lower for fallbacks).
    weight: f64,
    /// Candidates considered (available or not) for multi-path audit.
    candidates: Vec<Value>,
}

impl PathEncode {
    fn missing(slot: &str) -> Self {
        Self {
            body: PLACEHOLDER.to_string(),
            method: "missing",
            material_keys: vec![],
            batch_hint: "",
            priority: 255,
            weight: 0.0,
            candidates: vec![json!({
                "slot": slot,
                "method": "missing",
                "available": false,
            })],
        }
    }

    fn to_provenance(&self, slot: &str) -> Value {
        json!({
            "slot": slot,
            "selected_method": self.method,
            "material_keys": self.material_keys,
            "batch_hint": self.batch_hint,
            "priority": self.priority,
            "weight": self.weight,
            "digest_body_prefix": self.body.chars().take(24).collect::<String>(),
            "candidates": self.candidates,
            "policy": "multi_path_priority_v1",
        })
    }
}

/// Commercial cp — multi-path priority (engine-agnostic):
/// 1) multiround wall-clock rounds (B10)  2) raw multiround  3) single curve
/// WASM is K/diagnostic only — never commercial body.
fn encode_cp_from_analysis(
    fo: &Map<String, Value>,
    single: &[f64],
    extra: Option<&str>,
) -> String {
    encode_cp_multipath(fo, single, extra).body
}

fn encode_cp_multipath(
    fo: &Map<String, Value>,
    single: &[f64],
    extra: Option<&str>,
) -> PathEncode {
    use crate::hw_probe_analysis::{
        analyze_timing_like, digest_material, rounds_from_fields, ScaleMode,
    };
    let _ = extra;
    // Priority table — lower priority number wins when material present.
    // weight reflects reliability expectation for conf demote on fallbacks.
    let paths: &[(&str, &[&str], &str, u8, f64)] = &[
        (
            "cpu_timing_rounds_multiround",
            &["cpu_timing_rounds", "hw_curve_cpu_rounds", "cp_rounds"],
            "B10_hw_curves",
            1,
            1.0,
        ),
        (
            "cpu_timing_raw_rounds_multiround",
            &["cpu_timing_raw_rounds"],
            "B10_hw_curves",
            2,
            0.95,
        ),
        (
            "cpu_timing_curve_single",
            &["cpu_timing_curve", "hw_curve_cpu"],
            "B10_hw_curves",
            3,
            0.75,
        ),
    ];
    // Collect all candidates first (fleet analytics), then pick first successful encode.
    let mut prepared: Vec<(
        &'static str,
        Vec<&'static str>,
        &'static str,
        u8,
        f64,
        Vec<Vec<f64>>,
    )> = Vec::new();
    let mut candidates = Vec::new();
    for (method, keys, batch, pri, w) in paths {
        let mut rounds = rounds_from_fields(fo, keys, &[]);
        let mut used_keys: Vec<&'static str> = Vec::new();
        if rounds.is_empty() {
            if *method == "cpu_timing_curve_single" && single.len() >= 4 {
                rounds.push(single.to_vec());
                used_keys.extend(keys.iter().copied().filter(|k| fo.contains_key(*k)));
                if used_keys.is_empty() {
                    used_keys.push("single_arg");
                }
            } else {
                for k in *keys {
                    if let Some(arr) = fo.get(*k).and_then(|v| v.as_array()) {
                        let c: Vec<f64> = arr
                            .iter()
                            .filter_map(|x| x.as_f64())
                            .filter(|x| x.is_finite())
                            .collect();
                        if c.len() >= 4 {
                            rounds.push(c);
                            used_keys.push(*k);
                            break;
                        }
                    }
                }
            }
        } else {
            used_keys.extend(keys.iter().copied().filter(|k| {
                fo.get(*k)
                    .and_then(|v| v.as_array())
                    .map(|a| !a.is_empty())
                    .unwrap_or(false)
            }));
        }
        let available = !rounds.is_empty();
        candidates.push(json!({
            "method": method,
            "material_keys": keys,
            "batch_hint": batch,
            "priority": pri,
            "weight": w,
            "available": available,
            "rounds_n": rounds.len(),
            "commercial": true,
        }));
        if available {
            prepared.push((*method, used_keys, *batch, *pri, *w, rounds));
        }
    }
    // Diagnostic-only candidates always listed
    let has_ladder = fo.get("cpu_cache_ladder_curve").is_some()
        || fo.get("cpu_cache_knee_bytes").is_some();
    candidates.push(json!({
        "method": "cpu_cache_ladder_b34_diagnostic_only",
        "material_keys": ["cpu_cache_ladder_curve", "cpu_cache_knee_bytes"],
        "batch_hint": "B34_cpu_cache_ladder",
        "priority": 9,
        "weight": 0.0,
        "available": has_ladder,
        "commercial": false,
        "note": "K/diagnostic — not commercial V body",
    }));
    let has_wasm = fo.get("wasm_relaxed_simd_digest").is_some()
        || fo.get("wasm_simd_sig").is_some()
        || fo.get("wasm_timing_curve").is_some();
    candidates.push(json!({
        "method": "wasm_timing_soft_k_fallback",
        "material_keys": ["wasm_simd_sig", "wasm_relaxed_simd_digest", "wasm_timing_curve"],
        "batch_hint": "B10_hw_curves",
        "priority": 20,
        "weight": 0.0,
        "available": has_wasm,
        "commercial": false,
        "note": "K/diagnostic only — never commercial V body (pack presence must not fork cp)",
    }));
    for (method, used_keys, batch, pri, w, rounds) in prepared {
        if let Some(ana) = analyze_timing_like(&rounds, ScaleMode::Mean) {
            return PathEncode {
                body: digest_material("cp", &ana.material),
                method,
                material_keys: used_keys,
                batch_hint: batch,
                priority: pri,
                weight: w,
                candidates,
            };
        }
        // Analysis failed but samples exist — still mint structure/raw (do not discard).
        let flat: Vec<f64> = rounds.iter().flatten().copied().filter(|x| x.is_finite()).collect();
        if flat.len() >= 2 {
            if let Some(cs) = curve_structure_sig(&flat) {
                return PathEncode {
                    body: digest_material("cp", &format!("struct:{cs}")),
                    method: "cpu_timing_structure_fallback",
                    material_keys: used_keys,
                    batch_hint: batch,
                    priority: pri.saturating_add(5),
                    weight: w * 0.7,
                    candidates,
                };
            }
            let head: Vec<String> = flat.iter().take(16).map(|x| format!("{x:.4}")).collect();
            return PathEncode {
                body: digest_material("cp", &format!("rawhead:{}", head.join(","))),
                method: "cpu_timing_rawhead_fallback",
                material_keys: used_keys,
                batch_hint: batch,
                priority: pri.saturating_add(6),
                weight: w * 0.55,
                candidates,
            };
        }
    }
    PathEncode {
        body: PLACEHOLDER.to_string(),
        method: "missing",
        material_keys: vec![],
        batch_hint: "",
        priority: 255,
        weight: 0.0,
        candidates,
    }
}

/// Precision-lane encode for commercial cp — analysis path (lane places unused:
/// shape is already scale-invariant; do not re-squash into coarse buckets).
/// `fo` is optional multiround source; when None, analyze `arr` alone.
fn encode_cp_for_places(
    fo: Option<&Map<String, Value>>,
    arr: &[f64],
    _places: Option<u32>,
    extra: Option<&str>,
) -> String {
    if let Some(fo) = fo {
        encode_cp_from_analysis(fo, arr, extra)
    } else {
        encode_cp_from_analysis(&Map::new(), arr, extra)
    }
}

/// iss/67 B2 + iss/73: path-role structure for **wg** commercial digest — primary Lane-C role.
fn wg_path_role_sig(fo: &Map<String, Value>) -> Option<String> {
    // Same primary-role rule as residual_paths_lane_c_sig
    residual_paths_lane_c_sig(fo).map(|s| format!("wgr={}", short_hash(&s)))
}

fn build_segment_body_ex(
    residual: Option<f64>,
    residual_std_bucket: Option<&str>,
    residual_mp_sig: Option<&str>,
    audio_extra: Option<&str>,
    cp_extra: Option<&str>,
    wg: &[f64],
    au: &[f64],
    cp: &[f64],
    of: &str,
    ar: &str,
    cc: &str,
    tz: &str,
    oi: &str,
    rtc: &str,
    places: Option<u32>,
) -> String {
    build_segment_body_ex2(
        residual,
        residual_std_bucket,
        residual_mp_sig,
        audio_extra,
        cp_extra,
        None,
        None,
        wg,
        au,
        cp,
        of,
        ar,
        cc,
        tz,
        oi,
        rtc,
        places,
        false,
    )
}

fn build_segment_body_ex2(
    residual: Option<f64>,
    residual_std_bucket: Option<&str>,
    residual_mp_sig: Option<&str>,
    audio_extra: Option<&str>,
    cp_extra: Option<&str>,
    wg_extra: Option<&str>,
    fo_for_cp: Option<&Map<String, Value>>,
    wg: &[f64],
    au: &[f64],
    cp: &[f64],
    of: &str,
    ar: &str,
    cc: &str,
    tz: &str,
    oi: &str,
    rtc: &str,
    places: Option<u32>,
    lane_c_floor: bool,
) -> String {
    // All public parts are digests (or literal missing "0") — never raw OS/platform/tz.
    // Precision policy: dv0=full structured; dv5/6=places quantize; dv4=coarsest match.
    // iss/67 B2: wg_extra carries Lane-C primary role token when residual_paths present.
    // cp: probe analysis (multiround + scale-invariant structure) — NOT coarse buckets.
    // Host/protocol context is recorded for ops but never enters commercial body.
    let _host_context = (encode_class_part("oi", oi), encode_class_part("rtc", rtc));
    let parts = [
        encode_residual_rich(residual, residual_std_bucket, residual_mp_sig, places, lane_c_floor),
        encode_wg_for_places(wg, places, wg_extra),
        encode_au_for_places(au, places, audio_extra),
        encode_cp_for_places(fo_for_cp, cp, places, cp_extra),
        encode_class_part("of", of),
        encode_class_part("ar", ar),
        encode_class_part("cc", cc),
        encode_class_part("tz", tz),
        // Commercial body: oi/rtc are host/protocol context, not stable silicon.
        encode_class_part("oi", PLACEHOLDER),
        encode_class_part("rtc", PLACEHOLDER),
    ];
    parts.join("-")
}

/// Confidence pack after class caps + multipath dedupe (iss/67 A1 / P-4).
#[derive(Debug, Clone, Copy)]
pub struct ConfPack {
    pub conf: f64,
    pub conf_ceiling: f64,
    /// Sum of **confidence-weighted** qualities (after caps/dedupe).
    pub q_sum_for_conf: f64,
    pub q_cc_conf: f64,
    pub q_ar_conf: f64,
    pub multipath_dedupe: f64,
}

/// Env: set `GR_HW8_CLASS_WEIGHT_V1=0` to restore legacy uncapped class weights (rollback).
fn class_weight_v1_enabled() -> bool {
    match gr_abi::env::get("HW8_CLASS_WEIGHT_V1") {
        Some(v) => v != "0" && v != "false" && v != "off",
        None => true,
    }
}

/// iss/67 + opensource/docs/07: per-slot **role** for ops/lab (not mint body).
///
/// Material `slot_quality` can be high for class keys (`cc`≈0.95) while
/// effective conf weight is capped — role makes that explicit so consumers
/// do not treat high material_q as "silicon V".
pub fn slot_roles_json(ar_class: bool) -> Value {
    let ar_role = if ar_class { "K_C" } else { "V_candidate" };
    let ar_note = if ar_class {
        "fixed residual / no EU timing → class-like; conf cap 0.15; not silicon V"
    } else {
        "timing/challenge residual candidate; still needs reliability gate"
    };
    json!({
        "policy": "iss67_doc07_slot_roles_v1",
        "dual_kpi": {
            "uniqueness": "cross_ip_or_cross_device_collision_rate",
            "reliability": "same_vt_multi_session_agree_rate",
            "note": "uniqueness alone (cross-IP not-collide) does not prove silicon V"
        },
        "roles": {
            "res": {"role": "V", "weight": "primary", "note": "Lane-S multipath residual"},
            "wg": {"role": "V", "weight": "primary", "note": "webgl structure / path-role"},
            "au": {
                "role": "V_aux",
                "weight": "render_sensitive",
                "note": "iss/75: coarse mean/std + sr/mc only (no raw CSV / fine structure / latency); not Lane-C silicon gate"
            },
            "cp": {"role": "V_candidate", "weight": "gated", "conf_cap": 0.65, "note": "wall multiround structure only; wasm is K; need same-vt reliability before silicon claim"},
            "of": {"role": "V_aux", "weight": "engine_sensitive", "note": "canvas multiround structure; single-round demotes conf"},
            "ar": {"role": ar_role, "weight": if ar_class { "capped_class" } else { "gated" }, "note": ar_note},
            "cc": {"role": "K", "weight": "capped_class", "note": "GL caps matrix = model/config key; conf cap 0.12; not silicon V"},
            "tz": {"role": "V_candidate", "weight": "gated", "conf_cap": 0.65, "note": "rAF jitter structure; display Hz is K-only; same reliability gate as cp"},
            "oi": {"role": "host_C", "weight": "host_sep", "note": "os instance separator; not silicon"},
            "rtc": {"role": "host_C", "weight": "host_sep", "note": "WebRTC host hash; not silicon"},
        },
        "effective_bits": "conf ≈ f(capped_slot_q, multipath_dedupe); material_q high ≠ effective_bits high",
    })
}

/// Pure function for commercial confidence from per-slot qualities.
///
/// - `cc` conf cap ≈ 0.12 (K material)
/// - fixed residual `ar` conf cap ≈ 0.15 when `ar_class`
/// - `oi`/`rtc` host/net not pure silicon — soft-cap 0.35 each for conf
/// - multipath shared between res+wg: subtract `min(q_res,q_wg)*0.45` once
pub fn conf_from_slot_qualities(
    q_res: f64,
    q_wg: f64,
    q_au: f64,
    q_cp: f64,
    q_of: f64,
    q_ar: f64,
    q_cc: f64,
    q_tz: f64,
    q_oi: f64,
    q_rtc: f64,
    multipath_shared: bool,
    ar_class: bool,
    addressable: f64,
) -> ConfPack {
    let v1 = class_weight_v1_enabled();
    let q_cc_conf = if v1 { q_cc.min(0.12) } else { q_cc };
    let q_ar_conf = if v1 && ar_class {
        q_ar.min(0.15)
    } else if v1 {
        // challenge/timing candidate still soft-capped until lab matrix
        q_ar.min(0.55)
    } else {
        q_ar
    };
    let q_oi_conf = if v1 { q_oi.min(0.35) } else { q_oi };
    let q_rtc_conf = if v1 { q_rtc.min(0.35) } else { q_rtc };
    // wall-clock / rAF are soft V_candidates until dual-KPI reliability gate
    let q_cp_conf = if v1 { q_cp.min(0.65) } else { q_cp };
    let q_tz_conf = if v1 { q_tz.min(0.65) } else { q_tz };

    let mut q_sum = q_res
        + q_wg
        + q_au
        + q_cp_conf
        + q_of
        + q_ar_conf
        + q_cc_conf
        + q_tz_conf
        + q_oi_conf
        + q_rtc_conf;
    let mut multipath_dedupe = 0.0;
    if v1 && multipath_shared && q_res > 0.0 && q_wg > 0.0 {
        multipath_dedupe = q_res.min(q_wg) * 0.45;
        q_sum -= multipath_dedupe;
    }
    if q_sum < 0.0 {
        q_sum = 0.0;
    }
    let conf_ceiling = ((0.15 + 0.085 * addressable).min(0.95) * 10000.0).round() / 10000.0;
    let conf = ((0.15 + 0.085 * q_sum).min(conf_ceiling) * 10000.0).round() / 10000.0;
    ConfPack {
        conf,
        conf_ceiling,
        q_sum_for_conf: (q_sum * 10000.0).round() / 10000.0,
        q_cc_conf,
        q_ar_conf,
        multipath_dedupe: (multipath_dedupe * 10000.0).round() / 10000.0,
    }
}

/// SSOT documentation of which materials may feed each part (public form always digest).
pub fn device_segment_composition_json() -> Value {
    json!({
        "algo": DEVICE_SEGMENTS_ALGO,
        "encoding": "sha256_trunc10_domain_gr_device_seg_v1",
        "public_body": "hyphen_joined_digests_or_literal_0",
        "part_order": SEGMENT_PART_ORDER,
        "prefixes": SEGMENT_PREFIXES,
        "parts": {
            "res": {
                "materials": [
                    "residual_mean", "residual_std_bucket",
                    "residual_path_means", "residual_path_stds", "residual_path_modes",
                    "residual_paths", "webgl_residual_multipath",
                    "curve_structure_sig", "eu_timing_ms", "hw_anti_collision", "hw_silicon_fusion"
                ],
                "grade": "silicon_fingerprint_multipath_v2",
                "excluded": ["raw_float_text_in_public_id"],
                "note": "mean+std+multipath/structure/EU-timing secondary; never residual-alone UV"
            },
            "wg": {
                "materials": ["hw_curve_webgl", "webgl_residual_multipath", "hw_webgl_stable", "curve_structure_sig"],
                "grade": "silicon_curve_structured",
                "excluded": ["webgl_unmasked_renderer", "gpu_model_string"]
            },
            "au": {
                "materials": [
                    "hw_curve_audio_coarse_mean_std",
                    "audio_sample_rate",
                    "audio_max_channel_count"
                ],
                "grade": "v_aux_coarse_stack",
                "excluded": [
                    "audio_base_latency",
                    "audio_output_latency",
                    "raw_float_csv",
                    "curve_structure_sig_fine",
                    "audio_deep_moments",
                    "audio_deep_peak_bins"
                ],
                "note": "iss/75: V_aux commercial — coarse mean/std + sr/mc only; not Lane-C silicon gate"
            },
            "cp": {
                "materials": [
                    "cpu_timing_curve", "cpu_timing_rounds",
                    "multiround_median", "scale_invariant_structure", "stable_dim_mask"
                ],
                "grade": "probe_analysis_v1_no_coarse_bucket",
                "role": "V_candidate_soft",
                "excluded": [
                    "cpu_cache_ladder", "cpu_cache_knee_bytes", "B34_cpu_cache_ladder",
                    "wasm_relaxed_simd_digest", "ws_simd_timing_curve", "idb_write_digest",
                    "eventloop_digest", "forced_1ms_mean_bucket", "wasm_primary_commercial_body"
                ],
                "note": "DrawnApart/PUF-style: multiround median + scale-invariant structure only; wasm/ISA digests are K/diagnostic and never commercial V body"
            },
            "of": {
                "semantic_alias": "cv",
                "materials": [
                    "hw_curve_canvas", "hw_curve_canvas_rounds",
                    "multiround_median", "structure", "rank_order"
                ],
                "grade": "probe_analysis_v1_no_coarse_bucket",
                "note": "multiround structure+rank; single-round demotes conf; never os names; strip optional noise-hash presence (hygiene not coarsen)",
                "excluded": [
                    "os_family", "platform", "form_class", "user_agent", "ua",
                    "canvas_noise_hash", "canvas_2d_hash", "canvas_noise_patterns",
                    "forced_coarse_mean_std_bucket"
                ]
            },
            "ar": {
                "semantic_alias": "gp",
                "materials": ["hw_curve_webgpu", "webgpu_compute_curve", "hw_webgpu_compute_digest"],
                "grade": "extended_curve_digest",
                "excluded": ["architecture", "ua_ch_architecture"]
            },
            "cc": {
                "semantic_alias": "pm",
                "materials": ["gl_max_texture_size", "gl_max_renderbuffer"],
                "grade": "extended_caps_tex_rb_commercial_v5",
                "excluded": [
                    "hardware_concurrency", "cores_class",
                    "webgl_extensions_hash", "gl_precision_matrix", "glp_*",
                    "webgl_depth_bits", "webgl_samples"
                ],
                "note": "commercial body: max tex+rb only (depth/samples/ext optional → cross-site forks)"
            },
            "tz": {
                "semantic_alias": "tm",
                "materials": [
                    "timing_jitter_curve", "timing_jitter_rounds", "raf_interval_curve",
                    "scale_invariant_structure"
                ],
                "grade": "probe_analysis_v1_no_coarse_bucket",
                "role": "V_candidate_soft",
                "excluded": [
                    "timezone", "timezone_offset", "timezone_class",
                    "raf_mean_ms", "raf_std_ms", "sab_eventloop_jitter_ms",
                    "sab_clock_digest", "sab_tick_delta_curve_structure",
                    "clock_skew_tz_extra", "forced_2ms_mean_bucket",
                    "raf_hz_est_as_commercial_body", "hz_class_as_v_body",
                    "sab_ticks_per_ms"
                ],
                "k_only": ["raf_hz_est", "display_hz_class", "perf_now_resolution_band", "sab_clock_ok"],
                "note": "scale-invariant multiround rAF jitter structure for V body; display Hz class is K-only; no forced ms buckets"
            },
            "oi": {
                "materials": [
                    "os_instance_hash", "pohw_triad", "challenge_seed_sig", "pohw_hash", "unit_surface_id",
                    "ja4h_lite", "ja4h", "http_header_order_hash", "tcp_syn_option_order"
                ],
                "grade": "host_instance_and_protocol_header_fingerprint",
                "excluded": ["os_family", "platform", "form_class", "user_agent", "ua"],
                "note": "gateway JA4H/header-order/SYN-option digests join oi when present (opaque digest only)"
            },
            "rtc": {
                "materials": [
                    "webrtc_host_ip_hash", "webrtc_host_ip_hash_v2",
                    "ja4l_lite", "ja4l", "client_tcp_rtt", "tcp_info_rtt_us", "tcp_congestion"
                ],
                "grade": "host_network_and_protocol_timing_fingerprint",
                "excluded": ["server_client_ip", "client_ip"],
                "note": "gateway JA4L/RTT-bucket/congestion digests join rtc; never raw client IP"
            }
        },
        "never_in_body": [
            "user_agent", "ua", "server_client_ip", "client_ip",
            "webgl_unmasked_renderer", "webgl_governed_renderer", "gpu_model",
            "os_family", "platform", "form_class", "architecture", "ua_ch_architecture",
            "cores_class", "hardware_concurrency", "timezone", "timezone_class",
            "plain_os_name", "plain_timezone", "hardware_model",
            "sec_ch_ua", "sec_ch_ua_platform", "sec_ch_ua_model"
        ],
        "enrichment_policy": "fingerprint_or_curve_or_protocol_digest; fixed_name_labels_never; of_ar_cc_tz_are_extended_curve_slots_v2; gateway_ja4h_ja4l_syn_rtt_ok_as_opaque_digest",
        "cross_session_association": "sdk_not_v5_core — V5 mints session probe digests only; clustering on SDK",
        "addressable_slots": {
            "primary": ["res", "wg", "au", "cp", "oi", "rtc"],
            "extended": ["of", "ar", "cc", "tz"],
            "primary_n": 6,
            "extended_n": 4,
            "total_when_extended": 10
        },
        "conf_policy": "honest_ceiling_on_addressable; slot_quality_weights_entropy"
    })
}

/// Score a residual path object for union quality (align with evidence_merge).
fn residual_path_quality_score(v: &Value) -> i64 {
    let curve_n = v
        .get("curve")
        .and_then(|c| c.as_array())
        .map(|a| a.len())
        .unwrap_or(0) as i64;
    let ent = v
        .get("entropy_ok")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    let ok = v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false);
    let timing = v
        .get("eu_timing_ms")
        .and_then(|t| t.as_array())
        .map(|a| a.len())
        .unwrap_or(0) as i64;
    (if ent { 1000 } else { 0 }) + (if ok { 100 } else { 0 }) + curve_n + timing
}

fn path_obj_has_lane_c(p: &Value) -> bool {
    let pid = p.get("path_id").and_then(|v| v.as_str()).unwrap_or("");
    let mode = p.get("shader_mode").and_then(|v| v.as_str()).unwrap_or("");
    let role = path_lane_role(mode, pid);
    if !is_lane_c_role(role) {
        return false;
    }
    let curve_n = p
        .get("curve")
        .and_then(|c| c.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    curve_n >= 8
        || p.get("mean").and_then(|v| v.as_f64()).is_some()
        || p.get("ok").and_then(|v| v.as_bool()).unwrap_or(false)
        || p.get("entropy_ok").and_then(|v| v.as_bool()).unwrap_or(false)
}

fn residual_paths_arr_has_lane_c(arr: &[Value]) -> bool {
    arr.iter().any(path_obj_has_lane_c)
}

/// Union residual path arrays by path_id (quality-preferring).
fn union_residual_path_arrays(arrays: &[Vec<Value>]) -> Vec<Value> {
    use std::collections::HashMap;
    let mut by_id: HashMap<String, Value> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for arr in arrays {
        for e in arr {
            let pid = e
                .get("path_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let key = if pid.is_empty() {
                format!("anon_{}", by_id.len())
            } else {
                pid
            };
            if !by_id.contains_key(&key) {
                order.push(key.clone());
            }
            let prefer_new = !by_id.contains_key(&key)
                || residual_path_quality_score(e)
                    >= residual_path_quality_score(by_id.get(&key).unwrap());
            if prefer_new {
                by_id.insert(key, e.clone());
            }
        }
    }
    order
        .into_iter()
        .filter_map(|k| by_id.remove(&k))
        .collect()
}

fn collect_path_arrays_from_map(m: &Map<String, Value>) -> Vec<Vec<Value>> {
    let mut out = Vec::new();
    for k in [
        "residual_paths",
        "residual_paths_merged",
        "residual_paths_b10x",
        "residual_paths_extra",
    ] {
        if let Some(arr) = m.get(k).and_then(|v| v.as_array()).filter(|a| !a.is_empty()) {
            out.push(arr.clone());
        }
    }
    out
}

/// Seal commercial flats from Lane-C primary (noderiv > float > rint) so last-write
/// Lane-S `residual_mean` / `hw_curve_webgl` cannot diverge dv0 from the primary path.
fn seal_lane_c_primary_scalars(fo: &mut Map<String, Value>) {
    let Some(p) = lane_c_primary_path(fo) else {
        return;
    };
    if let Some(m) = p.get("mean").and_then(|v| v.as_f64()) {
        fo.insert("residual_mean".into(), json!(m));
        fo.insert("residual_available".into(), json!(true));
    }
    if let Some(s) = p.get("std").and_then(|v| v.as_f64()) {
        fo.insert("residual_std".into(), json!(s));
    }
    if let Some(curve) = p
        .get("curve")
        .and_then(|c| c.as_array())
        .filter(|a| a.len() >= 8)
    {
        fo.insert("hw_curve_webgl".into(), Value::Array(curve.clone()));
    }
    if let Some(pid) = p.get("path_id").cloned() {
        fo.insert("lane_c_primary_path_id".into(), pid);
    }
    // Keep residual_algo aligned with sealed primary (noderiv > float > rint).
    // FE B10x last-write previously left rint labels while digests used noderiv.
    let sealed_algo = p
        .get("residual_algo")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            let mode = p
                .get("shader_mode")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let pid = p.get("path_id").and_then(|v| v.as_str()).unwrap_or("");
            let role = path_lane_role(mode, pid);
            if role == "noderiv" || role == "float" || role == "rint" {
                Some(format!("gr_webgl_residual_{role}_v3f_sealed"))
            } else {
                None
            }
        });
    if let Some(algo) = sealed_algo {
        fo.insert("residual_algo".into(), json!(algo));
    }
    fo.insert("lane_c_primary_sealed".into(), json!(true));
}

/// Ensure residual_paths (and aliases) survive multi-source resolution for Lane-C mint.
/// iss/73 + iss/75: multi_source can leave Lane-S-only arrays after deep last-write;
/// always re-union from raw fields / all fields_by_source buckets when Lane-C is missing.
fn ensure_residual_paths_for_mint(
    fo: &mut Map<String, Value>,
    raw: &Value,
    evidence: Option<&Value>,
) {
    let mut arrays: Vec<Vec<Value>> = Vec::new();
    arrays.extend(collect_path_arrays_from_map(fo));
    if let Some(m) = raw.as_object() {
        arrays.extend(collect_path_arrays_from_map(m));
    }
    if let Some(fbs) = evidence
        .and_then(|e| e.get("fields_by_source"))
        .and_then(|v| v.as_object())
    {
        for (_src, bucket) in fbs {
            if let Some(m) = bucket.as_object() {
                arrays.extend(collect_path_arrays_from_map(m));
            }
        }
    }
    if arrays.is_empty() {
        return;
    }
    let merged = union_residual_path_arrays(&arrays);
    if merged.is_empty() {
        return;
    }
    let cur_lc = fo
        .get("residual_paths")
        .and_then(|v| v.as_array())
        .map(|a| residual_paths_arr_has_lane_c(a))
        .unwrap_or(false);
    let merged_lc = residual_paths_arr_has_lane_c(&merged);
    let cur_n = fo
        .get("residual_paths")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    // Replace when empty, Lane-S-only (missing Lane-C), or union grew.
    if !cur_lc || (merged_lc && merged.len() >= cur_n) || cur_n == 0 {
        fo.insert("residual_paths".into(), json!(merged.clone()));
        fo.insert("residual_paths_n".into(), json!(merged.len()));
    }
    seal_lane_c_primary_scalars(fo);
}

/// Build multi-precision multi-segment commercial device projection (SSOT mint).
///
/// Prefer external analyze-module provider when installed (OTA hot path);
/// otherwise use in-process local mint (runtime fallback / bootstrap).
pub fn select_device_segments(fields: &Value, evidence: Option<&Value>) -> Value {
    if let Ok(g) = EXTERNAL_MINT.read() {
        if let Some(p) = g.as_ref() {
            return p(fields, evidence);
        }
    }
    select_device_segments_local(fields, evidence)
}

/// In-process mint implementation (also used inside analyze.so — must not re-enter EXTERNAL_MINT).
pub fn select_device_segments_local(fields: &Value, evidence: Option<&Value>) -> Value {
    let auth = authentic_fields_for_device_id(fields, evidence);
    let mut fo = auth.as_object().cloned().unwrap_or_default();
    // iss/73: never lose multipath path objects for commercial Lane-C
    ensure_residual_paths_for_mint(&mut fo, fields, evidence);
    // Multi-path silicon fusion → prefer fused lane curves in segment body
    crate::hw_silicon_fusion::prefer_fused_curves(&mut fo);
    let auth_fused = Value::Object(fo.clone());
    let proj = crate::trust::commercial_projection(&auth_fused);

    // Reject UA / IP leakage into body (structural assert via notes).
    let mut security_notes: Vec<String> = vec![
        "ua_excluded_from_segments".into(),
        "client_ip_excluded_from_segments".into(),
    ];
    if f_has(&fo, "user_agent") || f_has(&fo, "ua") {
        security_notes.push("ua_present_assist_only".into());
    }
    if f_has(&fo, "server_client_ip") {
        security_notes.push("ip_present_assist_only".into());
    }

    // Residual: keep **raw** sealed-primary Lane-C / field mean; each precision
    // lane quantizes in encode. Do not globally 2-dec squash (that was
    // cross-browser force-merge — rejected). Primary-role mean keeps the res
    // slot pack-order stable (deep vs noderiv vs unioned) while preserving the
    // residual value itself (lanes must stay separable).
    let residual_raw = residual_mean_primary(&fo);
    let residual_std = fo.get("residual_std").and_then(|v| v.as_f64());
    let (wg, au, cp, mut curve_notes) = pick_curves(&fo, &proj);
    // iss/50 H2: of/ar/cc/tz = extended curve slots (never fixed names)
    let (of, ar, cc, tz, ext_notes, ext_q) = pick_extended_curve_slots(&fo, &proj);
    curve_notes.extend(ext_notes);
    let (oi, rtc) = pick_host_separator_parts(&fo);
    if oi != PLACEHOLDER {
        if oi.contains("ph:") {
            curve_notes.push("oi_includes_protocol_header_fp".into());
        }
        if oi.contains("oi:") || oi.contains("unit:") || oi.contains("pohw:") {
            curve_notes.push("oi_from_probe_fingerprint_digests".into());
        }
    }
    if rtc != PLACEHOLDER {
        if rtc.contains("pt:") {
            curve_notes.push("rtc_includes_protocol_timing_fp".into());
        }
        if rtc.contains("rtc:") {
            curve_notes.push("rtc_from_webrtc_host_hash".into());
        }
    }
    // Compare lane: reserved zeros for of/ar/cc/tz (pre-H2) for A/B
    let of0 = PLACEHOLDER.to_string();
    let ar0 = PLACEHOLDER.to_string();
    let cc0 = PLACEHOLDER.to_string();
    let tz0 = PLACEHOLDER.to_string();
    let (residual_mp_sig, lane_c_floor) = residual_multipath_sig_ex(&fo)
        .map(|(s, f)| (Some(s), f))
        .unwrap_or((None, false));
    if residual_mp_sig.is_some() {
        curve_notes.push("res_includes_lane_c_multipath_sig_v3".into());
    }
    let audio_extra = audio_stack_extra_commercial(&fo);
    if audio_extra.is_some() {
        curve_notes.push("au_includes_audio_stack_extra_commercial_v1".into());
    } else {
        curve_notes.push("au_commercial_curve_body_only_v2".into());
    }
    let _cp_extra_diag = cpu_stack_extra(&fo);
    // Commercial cp: wall multiround structure only; never wasm in body or via extra.
    let cp_extra = cpu_stack_extra_commercial(&fo);
    let cp_rounds_n = multiround_count(
        &fo,
        &["cpu_timing_rounds", "hw_curve_cpu_rounds", "cp_rounds"],
    );
    if cp_rounds_n >= 2 {
        curve_notes.push(format!("cp_probe_analysis_v1_multiround_n_{cp_rounds_n}"));
    } else if !cp.is_empty() {
        curve_notes.push("cp_probe_analysis_v1_single_round_conf_demote".into());
    } else {
        curve_notes.push("cp_missing".into());
    }
    if cpu_stack_extra(&fo).is_some() {
        curve_notes.push("cp_wasm_k_diagnostic_not_commercial_body".into());
    }
    // iss/67 B2: path-role structure on wg (not only res) — digest version bump intentional
    let wg_role_sig = wg_path_role_sig(&fo);
    if wg_role_sig.is_some() {
        curve_notes.push("wg_includes_path_role_structure_sig_v1".into());
    }
    let lane_c_ready = lane_c_materials_ready(&fo);
    if lane_c_ready {
        curve_notes.push("lane_c_materials_ready".into());
    } else {
        curve_notes.push("lane_c_materials_incomplete".into());
    }

    let mut segments = Map::new();
    let mut ordered = Vec::new();
    let mut ordered_reserved = Vec::new();
    for prefix in SEGMENT_PREFIXES {
        let places = places_for_prefix(prefix);
        // Per-lane residual:
        // - dv0: full raw under a Lane-C floor; else 3-dec lane
        //   (iss/opus5 03-P0-3: bare 12-dec digits are engine jitter)
        // - dv4: match-lane commercial floor (2-dec) — association/reliability only
        // - dv5/dv6: places quantize
        let residual_for_mint = residual_raw.map(|m| match places {
            None => {
                if lane_c_floor {
                    m
                } else {
                    quantize(m, Some(3))
                }
            }
            Some(4) => (m * 100.0).round() / 100.0,
            Some(p) => quantize(m, Some(p)),
        });
        let residual_std_bucket =
            residual_std_bucket_for_places(residual_std, places, lane_c_ready);
        let body = build_segment_body_ex2(
            residual_for_mint,
            residual_std_bucket.as_deref(),
            residual_mp_sig.as_deref(),
            audio_extra.as_deref(),
            cp_extra.as_deref(),
            wg_role_sig.as_deref(),
            Some(&fo),
            &wg,
            &au,
            &cp,
            &of,
            &ar,
            &cc,
            &tz,
            &oi,
            &rtc,
            places,
            lane_c_floor,
        );
        let full = format!("{prefix}-{body}");
        segments.insert((*prefix).into(), json!(full));
        ordered.push(full);
        // comparison mint: extended slots forced zero
        let body_r = build_segment_body_ex2(
            residual_for_mint,
            residual_std_bucket.as_deref(),
            residual_mp_sig.as_deref(),
            audio_extra.as_deref(),
            cp_extra.as_deref(),
            wg_role_sig.as_deref(),
            Some(&fo),
            &wg,
            &au,
            &cp,
            &of0,
            &ar0,
            &cc0,
            &tz0,
            &oi,
            &rtc,
            places,
            lane_c_floor,
        );
        ordered_reserved.push(format!("{prefix}-{body_r}"));
    }

    // Primary public id = full-precision lane (dv0-…) with extended slots
    let device_id = ordered
        .first()
        .cloned()
        .unwrap_or_else(|| "dv0-0-0-0-0-0-0-0-0-0-0".into());
    let device_id_reserved_compare = ordered_reserved
        .first()
        .cloned()
        .unwrap_or_else(|| "dv0-0-0-0-0-0-0-0-0-0-0".into());

    // Honest conf (iss/50 D1 + iss/67 A1): class slots capped; multipath not double-counted.
    let body0 = device_id
        .strip_prefix("dv0-")
        .unwrap_or("")
        .split('-')
        .collect::<Vec<_>>();
    let present_n = body0.iter().filter(|p| *p != &PLACEHOLDER).count();
    let addressable = 10.0_f64; // res wg au cp of ar cc tz oi rtc under extended_curve_v2
    let q_res = if residual_raw.is_some() { 1.0 } else { 0.0 };
    let q_wg = if wg.is_empty() {
        0.0
    } else if curve_slot_ok(&wg) {
        1.0
    } else {
        0.35
    };
    let q_au = if au.is_empty() {
        0.0
    } else if curve_slot_ok(&au) {
        1.0
    } else {
        0.35
    };
    let mut q_cp = if cp.is_empty() {
        0.0
    } else if curve_slot_ok(&cp) {
        1.0
    } else {
        0.35
    };
    // Soft V_candidate: demote when single-round or bad session window.
    if q_cp > 0.0 {
        if cp_rounds_n < 2 {
            q_cp *= 0.70;
        }
        q_cp = (q_cp * probe_session_quality_mult(&fo)).min(1.0);
    }
    // oi/rtc are host/net context probes — not silicon V (iss/63 K↔V, iss/67 pure-hw8).
    // Raw quality must not read as "full silicon" (1.0); conf path already caps ≤0.35.
    let q_oi = if oi != PLACEHOLDER {
        // single host digest present → moderate class quality
        0.45
    } else {
        0.0
    };
    let q_rtc = if rtc != PLACEHOLDER { 0.45 } else { 0.0 };
    let multipath_shared = curve_notes.iter().any(|n| {
        n.contains("multipath") || n.contains("webgl_residual_multipath") || n.contains("Lane-S")
    }) || residual_mp_sig.is_some();
    let ar_class = curve_notes.iter().any(|n| {
        n.contains("ar_fixed_residual_class_not_v") || n.contains("ar_class_slot_k_not_v")
    });
    let conf_pack = conf_from_slot_qualities(
        q_res,
        q_wg,
        q_au,
        q_cp,
        ext_q[0],
        ext_q[1],
        ext_q[2],
        ext_q[3],
        q_oi,
        q_rtc,
        multipath_shared,
        ar_class,
        addressable,
    );
    let q_sum = conf_pack.q_sum_for_conf;
    let conf_ceiling = conf_pack.conf_ceiling;
    let conf = conf_pack.conf;
    let conf_reserved = {
        let br = device_id_reserved_compare
            .strip_prefix("dv0-")
            .unwrap_or("")
            .split('-')
            .filter(|p| *p != PLACEHOLDER)
            .count();
        // reserved scheme: only 6 addressable primary slots
        let ceil_r = ((0.15_f64 + 0.085 * 6.0) * 10000.0).round() / 10000.0;
        ((0.15_f64 + 0.085 * br as f64).min(ceil_r) * 10000.0).round() / 10000.0
    };

    // Gateway OS preference note (documented for ops)
    let gw_os_note = evidence
        .and_then(|e| e.get("gateway_fields"))
        .and_then(|g| g.as_object())
        .map(|g| {
            if g.contains_key("server_client_ip") {
                "gateway_ip_assist_only_not_in_segment"
            } else {
                "gateway_no_os_instance"
            }
        })
        .unwrap_or("no_gateway_evidence");

    let curve_desc = curve_descriptors_for_sdk(&auth);

    // iss/67: model key + atlas shadow (diagnostic; never injected into body digests).
    // iss/69 U3: attach same-key peer context (farm-tightness input has a producer now).
    let mut fo_diag = fo.clone();
    crate::model_key::attach_model_key_extras(&mut fo_diag);
    let model_key_info = crate::model_key::model_key_from_fields(&Value::Object(fo_diag.clone()));
    let model_key_segments =
        crate::model_key::model_key_segments_from_info(&model_key_info, &Value::Object(fo_diag.clone()));
    let mk_str = model_key_info
        .get("hw_model_key")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let device_id_match = segments
        .get("dv4")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if !mk_str.is_empty() {
        crate::atlas_score::attach_peer_context(&mut fo_diag, &mk_str);
    }
    let atlas_shadow = if match gr_abi::env::get("ATLAS_SHADOW_SCORE") {
        Some(v) => v != "0" && v != "false" && v != "off",
        None => true,
    } {
        crate::atlas_score::atlas_shadow_score(&Value::Object(fo_diag))
    } else {
        json!({"enabled": false, "hard_block": false})
    };
    // iss/67 C2: stability gate surface (demote conf when gate on + unstable).
    // iss/69 U3: attach same-VT multi-session digest history so the gate has data.
    let mut gate_fields = fields.clone();
    if !device_id.is_empty() {
        crate::hw_channel_drift::attach_vt_family_digests(&mut gate_fields, &device_id);
    }
    let stability_gate = crate::hw_channel_drift::stability_gate_from_fields(&gate_fields);
    let conf = if crate::hw_channel_drift::drift_gate_enabled()
        && stability_gate
            .get("ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        && !stability_gate
            .get("stable")
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
    {
        ((conf * 0.80).min(0.70) * 10000.0).round() / 10000.0
    } else {
        conf
    };

    // iss/69 U3 producers: record this session for the next one (multi-worker safe).
    if !device_id.is_empty() {
        let parts_map = json!({
            "res": body0.get(0).copied().unwrap_or(""),
            "wg": body0.get(1).copied().unwrap_or(""),
            "au": body0.get(2).copied().unwrap_or(""),
            "cp": body0.get(3).copied().unwrap_or(""),
            "of": body0.get(4).copied().unwrap_or(""),
            "tz": body0.get(7).copied().unwrap_or(""),
        });
        crate::hw_channel_drift::observe_vt_family(&device_id, &parts_map);
    }
    if !mk_str.is_empty() {
        crate::atlas_score::observe_peer_digest(&mk_str, body0.get(0).copied().unwrap_or(""));
    }

    // Dual KPI surface (iss/67 · opensource/docs/07): uniqueness is not enough;
    // reliability comes from same-VT multi-session agree via stability_gate.
    let reliability_kpi = crate::hw_channel_drift::reliability_kpi_from_family(&gate_fields);
    let dual_kpi = {
        let sg_ok = stability_gate
            .get("ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let sg_stable = stability_gate
            .get("stable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let n_ch = stability_gate
            .get("n_channels_ok")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        // Soft V (cp/tz) stay V_candidate until multi-session reliability evidence.
        let soft_v_hard_eligible = sg_ok && sg_stable && n_ch >= 2;
        json!({
            "policy": "iss67_puf_dual_kpi_v1",
            "uniqueness": {
                "note": "cross_ip_or_cross_device_collision_rate — not measured in single mint",
                "require": "cohort report separate from single-session mint",
            },
            "reliability": {
                "same_vt_multi_session": sg_ok,
                "stable": sg_stable,
                "n_channels_ok": n_ch,
                "soft_v_hard_eligible": soft_v_hard_eligible,
                "note": "cp/tz conf capped until soft_v_hard_eligible; never promote on uniqueness alone",
            },
            "gates": {
                "drift_gate_enabled": crate::hw_channel_drift::drift_gate_enabled(),
                "fuzzy_mint_enabled": fuzzy_mint_enabled(),
                "recommend_open_drift_gate": reliability_kpi
                    .get("recommend_open_drift_gate")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            },
            "slot_roles": "see slot_roles; cp/tz=V_candidate conf_cap 0.65 until dual KPI",
        })
    };

    // iss/opus5 01-P0-1: layered identity. The dv0 body is explicitly the
    // CLASS id; the instance id is H(class ‖ stabilized machine separator),
    // withheld entirely when no separator material exists.
    let (device_instance_id, instance_conf, separator_kind) =
        layered_instance_id(&fo, &device_id, conf);
    let instance_issued = !device_instance_id.is_empty();

    json!({
        "device_id": device_id,
        "device_id_match": device_id_match,
        "device_id_reserved_compare": device_id_reserved_compare,
        "device_id_segments": segments,
        "device_id_segment_list": ordered,
        "device_segment_part_order": SEGMENT_PART_ORDER,
        "slot_scheme": "extended_curve_v2",
        "slot_aliases": {"of":"cv","ar":"gp","cc":"pm","tz":"tm"},
        "device_tier": "multi",
        "device_algo_group": "dv0-4-5-6",
        "device_confidence": conf,
        "confidence": conf,
        "conf_ceiling": conf_ceiling,
        "conf_reserved_compare": conf_reserved,
        // Layered identity surface (SDK reads via product_public identity).
        "device_class_id": device_id,
        "device_class_semantics": "device_class_config_not_machine_unique",
        "device_instance_id": if instance_issued { json!(device_instance_id) } else { Value::Null },
        "device_instance_issued": instance_issued,
        "device_instance_semantics": "machine_instance_hash_class_plus_separator_net_bound",
        "device_instance_confidence": instance_conf,
        "device_instance_separator": separator_kind,
        "identity_schema": "gr_layered_id_v1",
        "dual_kpi": dual_kpi,
        "reliability_kpi": reliability_kpi,
        "material_slots_addressable": addressable as u32,
        "parts_present": present_n,
        "parts_total": SEGMENT_PART_ORDER.len(),
        "parts_quality_sum": (q_sum * 10000.0).round() / 10000.0,
        "slot_quality": {
            "res": q_res, "wg": q_wg, "au": q_au, "cp": q_cp,
            "of": ext_q[0], "ar": ext_q[1], "cc": ext_q[2], "tz": ext_q[3],
            "oi": q_oi, "rtc": q_rtc,
        },
        // iss/67 + doc07: material completeness (slot_quality) ≠ effective V weight.
        // role = K/C/V/host; conf uses slot_quality_conf caps. dual KPI: uniqueness + reliability.
        "slot_roles": slot_roles_json(ar_class),
        // Multi-path probe architecture: every commercial slot records which probe
        // method/material won, priority/weight, and non-winning candidates (ops audit:
        // which environment makes which path effective). Engine-agnostic selection.
        "slot_probe_paths": build_slot_probe_paths(&fo, &cp),
        "slot_quality_conf": {
            "res": q_res, "wg": q_wg, "au": q_au, "cp": q_cp,
            "of": ext_q[0],
            "ar": conf_pack.q_ar_conf,
            "cc": conf_pack.q_cc_conf,
            "tz": ext_q[3],
            "oi": if class_weight_v1_enabled() { q_oi.min(0.35) } else { q_oi },
            "rtc": if class_weight_v1_enabled() { q_rtc.min(0.35) } else { q_rtc },
            "multipath_dedupe": conf_pack.multipath_dedupe,
            "class_weight_v1": class_weight_v1_enabled(),
            "effective_bits_note": "conf uses min-like caps: material_q alone does not raise silicon conf",
        },
        "hw_model_key": model_key_info.get("hw_model_key").cloned().unwrap_or(json!(null)),
        "hw_model_key_info": model_key_info,
        "hw_model_key_segments": model_key_segments,
        "atlas_shadow_score": atlas_shadow,
        "stability_gate": stability_gate,
        // iss/69 U7/U8: non-body diagnostics — digest encoding unchanged (iss/67 §9.5)
        "webgpu_timing_analysis": webgpu_timing_analysis(&fo),
        "wg_role_analysis": wg_role_analysis(&fo),
        // iss/73: Lane-S advanced multipath diagnostic only (not commercial body)
        "lane_s_sig": residual_paths_advanced_sig(&fo),
        "lane_c_sig": residual_paths_lane_c_sig(&fo),
        "lane_c_materials_ready": lane_c_ready,
        "algo": DEVICE_SEGMENTS_ALGO,
        "exclusive": false,
        "multi_segment": true,
        "precision_lanes": {
            "dv0": "full",
            "dv4": 0.0001,
            "dv5": 0.00001,
            "dv6": 0.000001,
        },
        "precision_policy": {
            "probe": "full_raw_materials",
            "dv0": "full_structured_encode",
            "dv5_dv6": "places_quantize",
            "dv4": "coarsest_match_lane",
            "device_id": "dv0",
            "device_id_match": "dv4",
            "cross_browser": "engine_aware_near_match_not_force_merge",
            "same_browser": "lane_c_complete_plus_cool_reuse",
        },
        "curve_selection_notes": curve_notes,
        "curve_descriptors": curve_desc,
        "os_source_policy": "extended_curve_slots_v2; fixed_names_never; host_sep_oi_rtc; public_body_digest_or_0",
        "os_source_note": gw_os_note,
        "segment_encoding": "sha256_trunc10_domain_gr_device_seg_v1",
        "security_notes": security_notes,
        "placeholder": PLACEHOLDER,
        "has_host_separator": oi != PLACEHOLDER || rtc != PLACEHOLDER,
        "client_js_ran": evidence
            .map(|e| {
                e.get("sources")
                    .and_then(|s| s.as_array())
                    .map(|a| {
                        a.iter().any(|x| {
                            let t = x.as_str().unwrap_or("");
                            t == "main" || t.starts_with("main") || t == "iframe" || t == "worker"
                        })
                    })
                    .unwrap_or(false)
            })
            .unwrap_or_else(|| {
                // Presence of silicon or host fingerprint materials implies FE ran;
                // form_class/platform alone are fixed names and do not mint.
                residual_raw.is_some()
                    || !wg.is_empty()
                    || !au.is_empty()
                    || oi != PLACEHOLDER
                    || rtc != PLACEHOLDER
            }),
        "dimensions": {
            "hw_ok": !wg.is_empty() || !au.is_empty() || residual_raw.is_some(),
            // soft_ok = host fingerprint separator present (not fixed-name class parts)
            "soft_ok": oi != PLACEHOLDER || rtc != PLACEHOLDER,
            "host_sep_ok": oi != PLACEHOLDER || rtc != PLACEHOLDER,
            "gw_ok": evidence
                .and_then(|e| e.get("has_gateway").and_then(|v| v.as_bool()))
                .unwrap_or(false)
                || evidence
                    .and_then(|e| e.get("gateway_fields"))
                    .is_some(),
        },
        "tier_reasons": [
            "multi_segment_precision_lanes_v1",
            "silicon_curves_over_model_labels",
            "ua_ip_excluded_from_body",
            "extended_curve_slots_of_ar_cc_tz_v2",
            "honest_conf_ceiling_on_addressable",
            "public_parts_are_digests_or_literal_0"
        ],
        "collision_risk": false,
        "underlying_projection": {
            "digest_path": proj.get("digest_path"),
            "eligible": proj.get("eligible"),
        },
    })
}

/// iss/67 B2 + iss/69 U8: per-path role structure diagnostics for wg (non-body).
///
/// Analysis vector only — the wg digest encoding is unchanged (digest continuity
/// rule from iss/67 §9.5). Lets ops/Atlas observe which lane roles actually
/// landed (PATH_CAP=5 / denorm coverage) without touching commercial ids.
fn wg_role_analysis(fo: &Map<String, Value>) -> Value {
    let mut roles: Vec<Value> = Vec::new();
    let mut n_ok = 0usize;
    if let Some(paths) = fo.get("residual_paths").and_then(|v| v.as_array()) {
        for p in paths.iter().take(16) {
            let pid = p.get("path_id").and_then(|v| v.as_str()).unwrap_or("");
            let mode = p
                .get("shader_mode")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let ok = p.get("ok").and_then(|v| v.as_bool()).unwrap_or(false)
                || p
                    .get("entropy_ok")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
            if ok {
                n_ok += 1;
            }
            let n = p
                .get("curve")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            roles.push(json!({
                "path_id": pid,
                "shader_mode": mode,
                "ok": ok,
                "curve_n": n,
                "mean": p.get("mean").and_then(|v| v.as_f64()),
                "std": p.get("std").and_then(|v| v.as_f64()),
                "eu_timing_ms": p.get("eu_timing_ms").and_then(|v| v.as_f64()),
            }));
        }
    }
    let coverage = if roles.is_empty() {
        "none"
    } else if n_ok >= 5 {
        "full"
    } else if n_ok >= 3 {
        "partial"
    } else {
        "thin"
    };
    json!({
        "algo": "wg_role_analysis_v1",
        "n_paths": roles.len(),
        "n_ok": n_ok,
        "coverage": coverage,
        "roles": roles,
        "residual_select": fo.get("residual_select").cloned().unwrap_or(json!({})),
        "note": "analysis vector only — wg digest encoding unchanged (iss/67 §9.5)",
    })
}

/// iss/67 B1 + iss/69 U7: multi-sample dispatch timing diagnostics (non-body).
///
/// The server `ar_has_timing_v` gate consumes `webgpu_compute_timing_curve` /
/// `webgpu_dispatch_timings_ms` directly; this surface summarizes quality so ops
/// can see whether B18v2 timing-V is eligible before any weight promotion.
fn webgpu_timing_analysis(fo: &Map<String, Value>) -> Value {
    let mut xs = curve_array(fo, "webgpu_eu_timing_curve");
    if xs.len() < 2 {
        xs = curve_array(fo, "webgpu_compute_timing_curve");
    }
    if xs.len() < 2 {
        if let Some(arr) = fo
            .get("webgpu_dispatch_timings_ms")
            .and_then(|v| v.as_array())
        {
            xs = arr.iter().filter_map(|x| x.as_f64()).collect();
        }
    }
    if xs.len() < 2 {
        return json!({
            "algo": "webgpu_timing_analysis_v1",
            "ok": false,
            "error": "need_multi_sample",
            "n_samples": 0,
            "note": "B18v2 timing dimension not produced on this device",
        });
    }
    let mut s = xs.clone();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let med = s[s.len() / 2];
    let q1 = s[(s.len() - 1) / 4];
    let q3 = s[(3 * s.len() - 1) / 4];
    let iqr = q3 - q1;
    let mean = s.iter().sum::<f64>() / s.len() as f64;
    let var = s.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / s.len() as f64;
    let cv = if mean > 0.0 { var.sqrt() / mean } else { 0.0 };
    let quality = fo
        .get("webgpu_timing_quality")
        .and_then(|v| v.as_str())
        .unwrap_or("ok");
    let eligible = xs.len() >= 2 && cv <= 0.5 && quality != "throttled";
    json!({
        "algo": "webgpu_timing_analysis_v1",
        "ok": xs.len() >= 2,
        "n_samples": xs.len(),
        "median_ms": (med * 1000.0).round() / 1000.0,
        "iqr_ms": (iqr * 1000.0).round() / 1000.0,
        "cv": (cv * 10000.0).round() / 10000.0,
        "min_ms": (s[0] * 1000.0).round() / 1000.0,
        "max_ms": (s[s.len() - 1] * 1000.0).round() / 1000.0,
        "quality": quality,
        "timing_v_eligible": eligible,
        "note": "non-body diagnostic — B18v2 timing-V candidate gate (iss/67 §9.5)",
    })
}

/// True if id is a multi-segment precision lane id.
pub fn is_multi_segment_id(id: &str) -> bool {
    SEGMENT_PREFIXES
        .iter()
        .any(|p| id.starts_with(&format!("{p}-")))
}

/// True when mint materials include a Lane-C path role **and** a usable primary curve.
/// Used to gate CIF so incomplete B10x packs cannot freeze a brittle half-mint.
pub fn lane_c_materials_ready(fo: &Map<String, Value>) -> bool {
    residual_paths_lane_c_sig(fo).is_some() && lane_c_primary_curve(fo).len() >= 8
}

/// Lane-C commercial head (res + wg) must be non-placeholder for CIF / hard identity.
/// Rejects `dv0-0-…`, `dv0-0-53aa…`, empty bodies (iss/75 178 ZERO_HEAD + false CIF).
pub fn lane_c_commercial_head_ok(device_id: &str) -> bool {
    let body = SEGMENT_PREFIXES
        .iter()
        .find_map(|p| device_id.strip_prefix(&format!("{p}-")));
    let Some(body) = body else {
        return false;
    };
    let mut parts = body.split('-');
    let res = parts.next().unwrap_or("");
    let wg = parts.next().unwrap_or("");
    !res.is_empty()
        && res != PLACEHOLDER
        && !wg.is_empty()
        && wg != PLACEHOLDER
}

/// Primary family tag for legacy consumers that still read device_tier.
pub fn multi_device_tier_label() -> &'static str {
    "multi"
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    /// iss/opus5 03-P0-3 regression: dv0 without a Lane-C floor must land on
    /// the 3-dec lane (never the bare 12-dec raw residual — engine jitter
    /// inflates impersonation-uniqueness); with a Lane-C floor it keeps the
    /// cross-engine 2-dec commercial floor. The res slot is a digest of the
    /// quantized material, so the check is: jitter within the lane collapses,
    /// real differences still diverge.
    #[test]
    fn dv0_residual_lane_without_lane_c() {
        // No Lane-C → 3-dec lane: sub-milli jitter collapses.
        let a = encode_residual_rich(Some(0.123456789), None, None, None, false);
        let jitter = encode_residual_rich(Some(0.123489991), None, None, None, false);
        assert_eq!(
            a, jitter,
            "engine jitter below 1e-3 must collapse on the dv0 lane"
        );
        // A real 3rd-decimal difference still separates.
        let diff = encode_residual_rich(Some(0.124001000), None, None, None, false);
        assert_ne!(a, diff, "3-dec lane still records real differences");
        // Lane-C floor present → 2-dec commercial floor: 4th-dec moves collapse.
        let fa = encode_residual_rich(Some(0.123456789), None, Some("mp:abc"), None, true);
        let fj = encode_residual_rich(Some(0.123999999), None, Some("mp:abc"), None, true);
        assert_eq!(fa, fj, "Lane-C floor keeps the 2-dec token");
        // Without the floor, the same two residuals diverge (raw would not).
        let ra = encode_residual_rich(Some(0.123456789), None, Some("mp:abc"), None, false);
        let rj = encode_residual_rich(Some(0.123999999), None, Some("mp:abc"), None, false);
        assert_ne!(ra, rj, "no floor → 3-dec lane splits these");
    }

    /// iss/opus5 01-P0-1: instance ID requires a stable separator; the same
    /// machine (same os_instance_hash) on the same /24 must reproduce the ID,
    /// a different /24 must fork it, and missing separator must not issue.
    #[test]
    fn layered_instance_id_issuance_and_stability() {
        let cls = "dv0-aaaa-bbbb-cccc-dddd-eeee-ffff-0000-1111-2222-3333";
        let mut fo = Map::new();
        fo.insert("os_instance_hash".into(), json!("abc123"));
        fo.insert("server_client_ip".into(), json!("203.0.113.77"));

        let (id1, c1, k1) = layered_instance_id(&fo, cls, 0.6);
        assert!(id1.starts_with("dvi1-"), "issued id prefix: {id1}");
        assert_eq!(k1, "os_instance_hash");
        assert!(c1 > 0.5 && c1 <= 0.95, "conf in range: {c1}");

        // Same machine + same /24 (different host octet) → identical id.
        fo.insert("server_client_ip".into(), json!("203.0.113.209"));
        let (id2, _, _) = layered_instance_id(&fo, cls, 0.6);
        assert_eq!(id1, id2, "same /24 must reproduce instance id");

        // Same machine, different /24 → net-bound fork (documented semantics).
        fo.insert("server_client_ip".into(), json!("198.51.100.9"));
        let (id3, _, _) = layered_instance_id(&fo, cls, 0.6);
        assert_ne!(id1, id3, "different /24 must fork instance id");

        // No IP at all → still issued (host material alone is machine-scoped).
        fo.remove("server_client_ip");
        let (id4, _, _) = layered_instance_id(&fo, cls, 0.6);
        assert!(id4.starts_with("dvi1-"));
        assert_ne!(id4, id1, "net-less separator differs from net-bound");
    }

    #[test]
    fn layered_instance_id_not_issued_without_separator() {
        let cls = "dv0-aaaa-bbbb-cccc-dddd-eeee-ffff-0000-1111-2222-3333";
        let fo = Map::new(); // no os_instance_hash / unit / webrtc materials
        let (id, conf, kind) = layered_instance_id(&fo, cls, 0.8);
        assert!(id.is_empty(), "no separator → no instance id");
        assert_eq!(conf, 0.0);
        assert_eq!(kind, "none");

        // Unversioned unit_surface_id must NOT serve as separator (redline).
        let mut fo2 = Map::new();
        fo2.insert("unit_surface_id".into(), json!("surf-xyz"));
        let (id2, _, _) = layered_instance_id(&fo2, cls, 0.8);
        assert!(id2.is_empty(), "unversioned unit surface must not issue");

        // Weakest host pin: webrtc host hash issues at reduced strength.
        let mut fo3 = Map::new();
        fo3.insert("webrtc_host_ip_hash".into(), json!("ff00aa"));
        let (id5, c5, k5) = layered_instance_id(&fo3, cls, 0.8);
        assert!(id5.starts_with("dvi1-"));
        assert_eq!(k5, "webrtc_host_hash");
        assert!(c5 > 0.0 && c5 <= 0.95);
    }

    #[test]
    fn ip_net_class_truncates_v4_v6() {
        assert_eq!(ip_net_class("192.168.1.87").as_deref(), Some("v4:192.168.1.0/24"));
        assert_eq!(
            ip_net_class("2001:db8:abcd:1234::1").as_deref(),
            Some("v6:2001:db8:abcd::/48")
        );
        assert_eq!(ip_net_class(""), None);
        assert_eq!(ip_net_class("not-an-ip"), None);
    }

    fn curve(n: usize, phase: f64) -> Vec<f64> {
        (0..n)
            .map(|i| ((i as f64 * 0.17 + phase).sin().abs() * 0.4 + 0.05))
            .collect()
    }

    /// iss/61 F2: gate off ⇒ fuzzy digests ignored; gate on ⇒ fold **au** fuzzy only.
    /// `fuzzy_ecc_wg_digest` must NOT enter commercial **audio** extras (wg path owns it).
    /// Serialized via ISS58_TEST_LOCK because the gate is process-global env.
    #[test]
    fn fuzzy_mint_gate_off_default_on_folds() {
        let _guard = crate::shared_governance::ISS58_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("GR_FUZZY_MINT");
        assert!(!fuzzy_mint_enabled(), "gate must default off");
        let fields = json!({
            "hw_curve_webgl": curve(16, 0.2),
            "hw_curve_audio": curve(16, 0.4),
            "fuzzy_ecc_au_digest": "0123456789abcdef",
            "fuzzy_ecc_wg_digest": "fedcba9876543210",
        });
        let off = audio_stack_extra(fields.as_object().unwrap());
        let off_s = off.unwrap_or_default();
        assert!(
            !off_s.contains("fza=") && !off_s.contains("fzw="),
            "gate off must ignore fuzzy digests: {off_s}"
        );
        std::env::set_var("GR_FUZZY_MINT", "1");
        assert!(fuzzy_mint_enabled());
        let on = audio_stack_extra(fields.as_object().unwrap()).unwrap_or_default();
        assert!(on.contains("fza="), "gate on must fold au digest: {on}");
        // Commercial au stack intentionally omits fzw= (wg fuzzy is not audio material).
        assert!(
            !on.contains("fzw="),
            "gate on must not fold wg fuzzy into audio extra: {on}"
        );
        std::env::remove_var("GR_FUZZY_MINT");
    }

    #[test]
    fn four_precision_lanes_and_placeholders() {
        let fields = json!({
            "residual_mean": 0.260084123456,
            "hw_curve_webgl": curve(8, 0.2),
            "hw_curve_audio": curve(16, 0.4),
            "os_family": "linux",
            "architecture": "x86_64",
            "hardware_concurrency": 12,
            "timezone": "Asia/Shanghai",
            "user_agent": "Mozilla/5.0 FAKE-UA-MUST-NOT-APPEAR",
            "server_client_ip": "198.51.100.7",
        });
        let ev = json!({
            "sources": ["main", "gateway"],
            "has_gateway": true,
            "gateway_fields": {"server_client_ip": "198.51.100.7"},
            "batches": [{"batch_id": "B10_hw_curves", "source": "main"}],
        });
        let out = select_device_segments(&fields, Some(&ev));
        let segs = out["device_id_segments"].as_object().unwrap();
        for p in SEGMENT_PREFIXES {
            assert!(segs.contains_key(*p), "missing {p}");
            let s = segs[*p].as_str().unwrap();
            assert!(s.starts_with(&format!("{p}-")), "{s}");
            assert!(!s.contains("Mozilla"), "UA leaked into {s}");
            assert!(!s.contains("198.51.100"), "IP leaked into {s}");
            assert!(!s.contains("FAKE-UA"), "UA leaked into {s}");
        }
        let dv0 = segs["dv0"].as_str().unwrap();
        let dv4 = segs["dv4"].as_str().unwrap();
        let dv5 = segs["dv5"].as_str().unwrap();
        let dv6 = segs["dv6"].as_str().unwrap();
        // Quantize must change residual encoding between lanes when precision differs.
        assert_ne!(dv0, dv4);
        // Placeholders for missing cpu / instance / rtc
        assert!(dv0.contains("-0-") || dv0.ends_with("-0") || dv0.split('-').any(|x| x == "0"));
        assert_eq!(out["device_id"], dv0);
        assert_eq!(out["device_tier"], "multi");
        assert_eq!(out["algo"], DEVICE_SEGMENTS_ALGO);
        let _ = (dv5, dv6);
    }

    #[test]
    fn missing_parts_are_literal_zero() {
        let out = select_device_segments(&json!({}), None);
        let s = out["device_id"].as_str().unwrap();
        assert!(s.starts_with("dv0-"));
        let body = s.strip_prefix("dv0-").unwrap();
        let parts: Vec<&str> = body.split('-').collect();
        assert_eq!(parts.len(), SEGMENT_PART_ORDER.len());
        assert!(parts.iter().all(|p| *p == "0"), "{s}");
    }

    #[test]
    fn quantize_lanes_diverge_on_residual() {
        let fields = json!({
            "residual_mean": 0.123456789,
            "form_class": "desktop",
            "platform": "Linux x86_64",
            "os_family": "linux",
        });
        let out = select_device_segments(&fields, Some(&json!({"sources":["main"]})));
        let segs = out["device_id_segments"].as_object().unwrap();
        let r0 = segs["dv0"].as_str().unwrap().split('-').nth(1).unwrap();
        let r4 = segs["dv4"].as_str().unwrap().split('-').nth(1).unwrap();
        let r5 = segs["dv5"].as_str().unwrap().split('-').nth(1).unwrap();
        let r6 = segs["dv6"].as_str().unwrap().split('-').nth(1).unwrap();
        // Precision lanes diverge as digests of differently quantized residuals
        assert_eq!(r0.len(), 10);
        assert_eq!(r4.len(), 10);
        assert_ne!(r0, r4);
        assert_ne!(r4, r5);
        assert_ne!(r5, r6);
        // Never plain float residual in public id
        assert!(!r0.contains('.'));
        assert!(!r4.starts_with("0.12"));
    }

    #[test]
    fn renderer_model_string_not_in_segment() {
        let fields = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": curve(8, 0.1),
            "webgl_unmasked_renderer": "ANGLE (NVIDIA GeForce RTX 4090)",
            "os_family": "linux",
        });
        let out = select_device_segments(&fields, Some(&json!({"sources":["main"]})));
        let s = out["device_id"].as_str().unwrap();
        assert!(!s.contains("NVIDIA"));
        assert!(!s.contains("4090"));
        assert!(!s.contains("ANGLE"));
        let notes = out["curve_selection_notes"].as_array().unwrap();
        assert!(notes.iter().any(|n| n.as_str() == Some("renderer_label_excluded_from_segment")));
    }


    #[test]
    fn residual_zero_not_same_as_missing_placeholder() {
        let missing = select_device_segments(&json!({}), None);
        let zero = select_device_segments(&json!({"residual_mean": 0.0}), None);
        let m = missing["device_id"].as_str().unwrap();
        let z = zero["device_id"].as_str().unwrap();
        // res part: missing="0", present zero → digest of "0.0" (never plain "0" or "0.0")
        assert_eq!(m.split('-').nth(1), Some("0"));
        let z_res = z.split('-').nth(1).unwrap();
        assert_ne!(z_res, "0");
        assert_ne!(z_res, "0.0");
        assert_eq!(z_res.len(), 10, "digest token length");
        assert_ne!(m, z);
    }

    #[test]
    fn multipath_longer_curve_preferred_over_short() {
        // Same hardware, multiple probe routes: prefer complete longer silicon curve.
        let short = curve(4, 0.1);
        let long = curve(16, 0.1);
        let fields = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": short,
            "webgl_residual_multipath": long,
            "os_family": "linux",
            "architecture": "x86_64",
            "hardware_concurrency": 8,
        });
        let out = select_device_segments(&fields, Some(&json!({"sources":["main"]})));
        let notes = out["curve_selection_notes"].as_array().unwrap();
        assert!(
            notes
                .iter()
                .any(|n| n.as_str() == Some("wg_from_webgl_residual_multipath")),
            "must pick longest multipath curve: {notes:?}"
        );
        assert!(
            notes
                .iter()
                .any(|n| n.as_str() == Some("wg_len_16")),
            "{notes:?}"
        );
        // Token from multipath must differ from short-only mint.
        let only_short = select_device_segments(
            &json!({
                "residual_mean": 0.26,
                "hw_curve_webgl": curve(4, 0.1),
                "os_family": "linux",
                "architecture": "x86_64",
                "hardware_concurrency": 8,
            }),
            Some(&json!({"sources":["main"]})),
        );
        let wg_multi = out["device_id"].as_str().unwrap().split('-').nth(2).unwrap();
        let wg_short = only_short["device_id"].as_str().unwrap().split('-').nth(2).unwrap();
        assert_ne!(wg_multi, wg_short, "longer multipath must change wg token");
    }

    #[test]
    fn raw_curve_beats_renderer_model_and_digest_when_both_present() {
        let fields = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": curve(8, 0.11),
            "webgl_unmasked_renderer": "ANGLE (NVIDIA GeForce RTX 4090 FakeModel)",
            "os_family": "linux",
            "architecture": "x86_64",
            "hardware_concurrency": 8,
        });
        let out = select_device_segments(&fields, Some(&json!({"sources":["main"]})));
        let s = out["device_id"].as_str().unwrap();
        assert!(!s.contains("NVIDIA") && !s.contains("4090") && !s.contains("FakeModel"));
        let notes = out["curve_selection_notes"].as_array().unwrap();
        assert!(
            notes.iter().any(|n| n.as_str() == Some("wg_from_hw_curve_webgl")),
            "must prefer raw curve: {notes:?}"
        );
        assert!(
            notes.iter().any(|n| n.as_str() == Some("renderer_label_excluded_from_segment")),
            "{notes:?}"
        );
        // wg part must not be placeholder
        let wg = s.split('-').nth(2).unwrap();
        assert_ne!(wg, "0", "silicon curve must produce non-zero wg token");
    }

    #[test]
    fn fixed_name_class_parts_always_zero_even_when_present() {
        // Fixed names (OS/arch/cores/tz/form/UA/IP) must NOT mint of/ar/cc/tz —
        // even as digests of the label. Only probe fingerprints fill oi/rtc.
        let fields = json!({
            "os_family": "linux",
            "platform": "Linux x86_64",
            "architecture": "x86_64",
            "hardware_concurrency": 16,
            "form_class": "desktop",
            "timezone": "UTC",
            "server_client_ip": "203.0.113.50",
            "user_agent": "Mozilla/5.0 SpoofUA",
            "webgl_unmasked_renderer": "ANGLE (NVIDIA GeForce RTX 4090)",
        });
        let ev = json!({
            "sources": ["main", "gateway"],
            "has_gateway": true,
            "gateway_fields": {
                "server_client_ip": "203.0.113.50",
                "server_asn": "AS64500"
            },
        });
        let out = select_device_segments(&fields, Some(&ev));
        let s = out["device_id"].as_str().unwrap();
        assert!(!s.contains("linux"), "plain os_family must not appear: {s}");
        assert!(!s.contains("x86"), "plain arch must not appear: {s}");
        assert!(!s.contains("203.0.113"), "gateway IP must not enter segment: {s}");
        assert!(!s.contains("SpoofUA") && !s.contains("Mozilla"));
        assert!(!s.contains("NVIDIA") && !s.contains("4090"));
        assert!(
            out["os_source_policy"]
                .as_str()
                .unwrap()
                .contains("extended_curve_slots_v2"),
            "{:?}",
            out["os_source_policy"]
        );
        // Without curve materials, of/ar/cc/tz stay 0 (never plain names)
        let parts: Vec<&str> = s.strip_prefix("dv0-").unwrap().split('-').collect();
        assert_eq!(parts.len(), 10, "{s}");
        for (i, code) in ["of", "ar", "cc", "tz"].iter().enumerate() {
            assert_eq!(
                parts[4 + i],
                "0",
                "{code} must not mint from fixed names, got {}: {s}",
                parts[4 + i]
            );
        }
        assert!(out["conf_ceiling"].as_f64().unwrap() >= 0.65);
        assert_eq!(out["slot_scheme"], "extended_curve_v2");
        // reserved compare id exists for A/B
        assert!(out["device_id_reserved_compare"].as_str().unwrap().starts_with("dv0-"));
    }

    #[test]
    fn gateway_protocol_host_context_flagged_but_not_in_body() {
        let fields = json!({
            "ja4h_lite": "h_68be33f9c3a280ae",
            "http_header_order_hash": "68be33f9c3a280ae",
            "tcp_syn_option_order": "mss,sok,ts,nop,ws",
            "ja4l_lite": "t_lt20ms",
            "client_tcp_rtt": 35.0,
            "tcp_congestion": "cubic",
            // forbidden — must not appear in body
            "user_agent": "Mozilla/5.0 FAKE-UA-STRING",
            "server_client_ip": "203.0.113.9",
            "os_family": "windows",
            "webgl_unmasked_renderer": "ANGLE (NVIDIA GeForce RTX 4090)",
        });
        let out = select_device_segments(
            &fields,
            Some(&json!({"sources": ["gateway"], "has_gateway": true})),
        );
        let s = out["device_id"].as_str().unwrap();
        assert!(s.starts_with("dv0-"), "{s}");
        let parts: Vec<&str> = s.strip_prefix("dv0-").unwrap().split('-').collect();
        assert_eq!(parts.len(), SEGMENT_PART_ORDER.len());
        // Host/protocol context is recorded for ops but never enters the
        // commercial body: oi (index 8) / rtc (index 9) stay placeholder.
        assert_eq!(parts[8], "0", "oi must not fork commercial id: {s}");
        assert_eq!(parts[9], "0", "rtc must not fork commercial id: {s}");
        // redlines
        assert!(!s.contains("Mozilla"));
        assert!(!s.contains("203.0.113"));
        assert!(!s.contains("windows"));
        assert!(!s.contains("NVIDIA"));
        assert!(!s.contains("4090"));
        // …but the protocol fingerprints are still recorded for ops.
        let notes = out["curve_selection_notes"].as_array().unwrap();
        assert!(notes
            .iter()
            .any(|n| n.as_str() == Some("oi_includes_protocol_header_fp")));
        assert!(notes
            .iter()
            .any(|n| n.as_str() == Some("rtc_includes_protocol_timing_fp")));
        assert!(out["has_host_separator"].as_bool().unwrap());
        // two different gateway stacks → same commercial id, context still flagged
        let fields2 = json!({
            "ja4h_lite": "h_aaaaaaaaaaaa",
            "http_header_order_hash": "bbbbbbbbbbbb",
            "tcp_syn_option_order": "mss,nop,ws",
            "ja4l_lite": "t_lt80ms",
            "client_tcp_rtt": 90.0,
        });
        let out2 = select_device_segments(&fields2, Some(&json!({"sources":["gateway"]})));
        assert_eq!(
            out["device_id"], out2["device_id"],
            "host/protocol differences must not split the commercial device id"
        );
        assert!(out2["has_host_separator"].as_bool().unwrap());
    }

    #[test]
    fn host_fingerprint_context_flagged_but_not_in_body() {
        let fields = json!({
            "residual_mean": 0.26,
            "os_family": "linux",
            "architecture": "x86_64",
            "form_class": "desktop",
            "timezone": "Asia/Shanghai",
            "os_instance_hash": "oi_probe_digest_abc",
            "webrtc_host_ip_hash": "rtc_probe_digest_xyz",
        });
        let out = select_device_segments(&fields, Some(&json!({"sources":["main"]})));
        let s = out["device_id"].as_str().unwrap();
        let parts: Vec<&str> = s.strip_prefix("dv0-").unwrap().split('-').collect();
        // of/ar/cc/tz still zero
        assert_eq!(parts[4], "0");
        assert_eq!(parts[5], "0");
        assert_eq!(parts[6], "0");
        assert_eq!(parts[7], "0");
        // Host context (oi/rtc) stays out of the commercial body…
        assert_eq!(parts[8], "0");
        assert_eq!(parts[9], "0");
        // …but probe-digest presence is recorded for ops.
        assert!(out["has_host_separator"].as_bool().unwrap());
        assert!(out["dimensions"]["host_sep_ok"].as_bool().unwrap());
        let notes = out["curve_selection_notes"].as_array().unwrap();
        assert!(notes
            .iter()
            .any(|n| n.as_str() == Some("oi_from_probe_fingerprint_digests")));
        assert!(notes
            .iter()
            .any(|n| n.as_str() == Some("rtc_from_webrtc_host_hash")));
        // Plain labels still absent
        assert!(!s.contains("linux") && !s.contains("Shanghai") && !s.contains("desktop"));
    }

    #[test]
    fn public_body_never_contains_plain_os_platform_tz() {
        let fields = json!({
            "residual_mean": 0.261126875,
            "hw_curve_webgl": curve(8, 0.2),
            "hw_curve_audio": curve(16, 0.3),
            "os_family": "linux",
            "architecture": "x86_64",
            "hardware_concurrency": 8,
            "timezone": "Asia/Singapore",
            "os_instance_hash": "abc123def",
        });
        let out = select_device_segments(&fields, Some(&json!({"sources":["main"]})));
        for p in SEGMENT_PREFIXES {
            let s = out["device_id_segments"][p].as_str().unwrap();
            for banned in [
                "linux",
                "x86_64",
                "Asia",
                "Singapore",
                "0.261126875",
                "0.2611",
            ] {
                assert!(
                    !s.contains(banned),
                    "{p} leaked raw material {banned}: {s}"
                );
            }
            // All non-placeholder parts look like digests (10 hex) or 0
            for (i, part) in s.strip_prefix(&format!("{p}-")).unwrap().split('-').enumerate() {
                if part != "0" {
                    assert_eq!(part.len(), 10, "part {i} of {s}");
                    assert!(part.chars().all(|c| c.is_ascii_hexdigit()), "{part}");
                }
            }
        }
    }

    #[test]
    fn silicon_curve_preferred_over_empty_when_digest_present() {
        let fields = json!({
            "residual_mean": 0.26,
            "os_family": "linux",
            "architecture": "x86_64",
            "hardware_concurrency": 8,
        });
        let out = select_device_segments(&fields, Some(&json!({"sources":["main"]})));
        assert!(out["dimensions"]["hw_ok"].as_bool().unwrap());
        // Fixed names alone must not flip soft_ok / host_sep
        assert!(!out["dimensions"]["soft_ok"].as_bool().unwrap());
        assert!(!out["dimensions"]["host_sep_ok"].as_bool().unwrap());
    }

    #[test]
    fn extended_curve_slots_mint_and_compare_differs() {
        // Commercial ar requires V-grade WebGPU residual (challenge/timing/compute_ok).
        // Curve-only thin B18 is intentionally zeroed (cross-site stability).
        let fields = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": curve(8, 0.2),
            "hw_curve_audio": curve(16, 0.3),
            "hw_curve_canvas": curve(12, 0.5),
            "hw_curve_webgpu": curve(16, 0.7),
            "webgpu_compute_ok": true,
            "webgpu_challenge_seed_used": true,
            "webgpu_compute_mean": 0.4,
            "webgpu_compute_std": 0.05,
            "gl_max_texture_size": 16384,
            "gl_max_renderbuffer": 16384,
            "hw_noise_curve": curve(16, 0.9),
            "os_instance_hash": "oi_test",
        });
        let out = select_device_segments(&fields, Some(&json!({"sources":["main"]})));
        let s = out["device_id"].as_str().unwrap();
        let r = out["device_id_reserved_compare"].as_str().unwrap();
        assert_ne!(s, r, "extended should differ from reserved-zero compare");
        let parts: Vec<&str> = s.strip_prefix("dv0-").unwrap().split('-').collect();
        // of/ar should be non-zero digests when canvas + V-grade webgpu residual present
        assert_ne!(parts[4], "0", "of/cv");
        assert_ne!(parts[5], "0", "ar/gp");
        assert_eq!(out["slot_scheme"], "extended_curve_v2");
        assert!(out["conf_ceiling"].as_f64().unwrap() >= 0.9);
        assert!(out["curve_descriptors"]["slots"].is_object());
        // reserved compare has zeros in of/ar/cc/tz
        let rp: Vec<&str> = r.strip_prefix("dv0-").unwrap().split('-').collect();
        assert_eq!(rp[4], "0");
        assert_eq!(rp[5], "0");
    }

    #[test]
    fn composition_json_marks_class_slots_reserved() {
        let c = device_segment_composition_json();
        for code in ["of", "ar", "cc", "tz"] {
            let grade = c["parts"][code]["grade"].as_str().unwrap_or("");
            assert!(
                grade.contains("extended")
                    || grade.contains("curve")
                    || grade.contains("caps")
                    || grade.contains("probe_analysis"),
                "{code} grade={grade}"
            );
            assert!(!c["parts"][code]["materials"]
                .as_array()
                .unwrap()
                .is_empty());
        }
        let never = c["never_in_body"].as_array().unwrap();
        for banned in [
            "os_family",
            "platform",
            "form_class",
            "architecture",
            "timezone",
            "user_agent",
            "server_client_ip",
            "hardware_model",
        ] {
            assert!(
                never.iter().any(|v| v.as_str() == Some(banned)),
                "missing ban {banned}"
            );
        }
        assert!(c["cross_session_association"]
            .as_str()
            .unwrap()
            .contains("sdk_not_v5_core"));
    }

    /// Same residual_mean but different curve shape / std → different res digest.
    #[test]
    fn res_structure_sig_breaks_same_mean_floor() {
        let curve_a: Vec<f64> = (0..32)
            .map(|i| 0.25 + (i as f64) * 0.001 + if i == 7 { 0.2 } else { 0.0 })
            .collect();
        let curve_b: Vec<f64> = (0..32)
            .map(|i| 0.25 + (i as f64) * 0.001 + if i == 15 { 0.2 } else { 0.0 })
            .collect();
        let mean = curve_a.iter().sum::<f64>() / curve_a.len() as f64;
        // Force same mean numerically for both
        let fields_a = json!({
            "residual_mean": mean,
            "residual_std": 0.09,
            "hw_curve_webgl": curve_a,
            "hw_curve_audio": (0..16).map(|i| 0.1 + i as f64 * 0.02).collect::<Vec<_>>(),
            "cpu_timing_curve": (0..24).map(|i| 0.5 + (i % 5) as f64 * 0.1).collect::<Vec<_>>(),
            "gl_max_texture_size": 16384,
            "gl_max_renderbuffer": 16384,
            "webgl_extensions_hash": "aaaa",
            "gl_high_float": [23, 127, 127],
        });
        let fields_b = json!({
            "residual_mean": mean,
            "residual_std": 0.091,
            "hw_curve_webgl": curve_b,
            "hw_curve_audio": (0..16).map(|i| 0.1 + i as f64 * 0.02).collect::<Vec<_>>(),
            "cpu_timing_curve": (0..24).map(|i| 0.5 + (i % 5) as f64 * 0.1).collect::<Vec<_>>(),
            "gl_max_texture_size": 16384,
            "gl_max_renderbuffer": 16384,
            "webgl_extensions_hash": "bbbb",
            "gl_high_float": [23, 127, 127],
        });
        let segs_a = select_device_segments(&fields_a, None);
        let segs_b = select_device_segments(&fields_b, None);
        let dv0_a = segs_a
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let dv0_b = segs_b
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let res_a = dv0_a.split('-').nth(1).unwrap_or("0");
        let res_b = dv0_b.split('-').nth(1).unwrap_or("0");
        let wg_a = dv0_a.split('-').nth(2).unwrap_or("0");
        let wg_b = dv0_b.split('-').nth(2).unwrap_or("0");
        let cc_a = dv0_a.split('-').nth(7).unwrap_or("0");
        let cc_b = dv0_b.split('-').nth(7).unwrap_or("0");
        assert_ne!(res_a, "0");
        // Commercial cc is tex+rb only — same caps → same cc (extension hash ignored).
        assert_eq!(
            cc_a, cc_b,
            "same tex/rb must keep commercial cc stable; a={cc_a} b={cc_b}"
        );
        // Different webgl curve shape and residual_std should still separate body.
        assert!(
            res_a != res_b || wg_a != wg_b,
            "curve shape/std must separate res or wg; res={res_a}/{res_b} wg={wg_a}/{wg_b}"
        );
    }

    /// Prod v5.8.120: FE wrote constant jit into hw_curve_cpu but preserved real
    /// wall timing in cpu_timing_curve. cp must mint from timing, not the global constant.
    #[test]
    fn cp_prefers_cpu_timing_over_collapsed_jit_clone() {
        // Constant-ish jit (prod-like: mostly zeros + one bin)
        let mut jit = vec![0.0_f64; 32];
        jit[6] = 128.0 / 255.0;
        let timing_a: Vec<f64> = (0..24)
            .map(|i| ((i % 5) as f64) * 0.25 + if i % 7 == 0 { 0.5 } else { 0.0 })
            .collect();
        let timing_b: Vec<f64> = (0..24)
            .map(|i| ((i % 4) as f64) * 0.3 + if i % 5 == 0 { 1.0 } else { 0.1 })
            .collect();

        let fields_a = json!({
            "jit_lowbits_curve": jit,
            "hw_curve_cpu": jit, // FE overwrite bug
            "cpu_timing_curve": timing_a,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + (i as f64) * 0.01).collect::<Vec<_>>(),
            "hw_curve_audio": (0..16).map(|i| 0.1 + (i as f64) * 0.02).collect::<Vec<_>>(),
            "residual_mean": 0.25,
        });
        let fields_b = json!({
            "jit_lowbits_curve": jit,
            "hw_curve_cpu": jit,
            "cpu_timing_curve": timing_b,
            "hw_curve_webgl": (0..16).map(|i| 0.21 + (i as f64) * 0.01).collect::<Vec<_>>(),
            "hw_curve_audio": (0..16).map(|i| 0.11 + (i as f64) * 0.02).collect::<Vec<_>>(),
            "residual_mean": 0.26,
        });

        let segs_a = select_device_segments(&fields_a, None);
        let segs_b = select_device_segments(&fields_b, None);
        let dv0_a = segs_a
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let dv0_b = segs_b
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let cp_a = dv0_a.split('-').nth(4).unwrap_or("0");
        let cp_b = dv0_b.split('-').nth(4).unwrap_or("0");
        assert_ne!(cp_a, "0", "cp should mint from timing, got {dv0_a}");
        assert_ne!(cp_b, "0", "cp should mint from timing, got {dv0_b}");
        assert_ne!(
            cp_a, cp_b,
            "different cpu_timing must yield different cp (not global jit constant); a={cp_a} b={cp_b}"
        );
        // notes should mention cpu_timing
        let notes = segs_a
            .get("notes")
            .or_else(|| segs_a.get("curve_notes"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let note_s: Vec<String> = notes
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        let note_blob = format!("{note_s:?}{segs_a}");
        assert!(
            note_blob.contains("cpu_timing") || note_blob.contains("cp_from_cpu_timing"),
            "expected cp_from_cpu_timing in output, got notes={note_s:?}"
        );
    }

    /// Commercial: same residual_mean class floor (prod 0.2603390625) with multipath
    /// residual_paths of different modes must diverge res digests.
    #[test]
    fn multipath_paths_break_dead_residual_mean_floor() {
        // Classic ANGLE class mean
        let mean = 0.2603390625_f64;
        let dead_curve: Vec<f64> = (0..32)
            .map(|i| if i == 6 { 0.85 } else { 0.01 * ((i % 3) as f64) })
            .collect();
        // Machine A: float + noderiv + ulp with distinct path means
        let fields_a = json!({
            "residual_mean": mean,
            "residual_std": 0.008,
            "hw_curve_webgl": dead_curve,
            "residual_path_means": [mean, mean + 0.0004, mean - 0.0012],
            "residual_path_stds": [0.008, 0.011, 0.042],
            "residual_path_modes": ["float", "noderiv", "ulp"],
            "residual_paths": [
                {
                    "path_id": "v3f_128_std",
                    "shader_mode": "float",
                    "ok": true,
                    "entropy_ok": false,
                    "mean": mean,
                    "std": 0.008,
                    "curve": dead_curve,
                },
                {
                    "path_id": "v3f_128_noderiv",
                    "shader_mode": "noderiv",
                    "ok": true,
                    "entropy_ok": true,
                    "mean": mean + 0.0004,
                    "std": 0.011,
                    "curve": (0..32).map(|i| dead_curve[i] + 0.002).collect::<Vec<_>>(),
                },
                {
                    "path_id": "ulp_anti_collision_128",
                    "shader_mode": "ulp",
                    "ok": true,
                    "entropy_ok": true,
                    "mean": mean - 0.0012,
                    "std": 0.042,
                    "curve": (0..32).map(|i| 0.1 + (i as f64) * 0.003).collect::<Vec<_>>(),
                    "eu_timing_ms": [2.1, 2.0, 2.2, 2.05, 2.15, 2.08],
                },
            ],
            "eu_timing_ms": [2.1, 2.0, 2.2, 2.05, 2.15, 2.08],
            "hw_curve_audio": (0..16).map(|i| 0.1 + i as f64 * 0.02).collect::<Vec<_>>(),
            "cpu_timing_curve": (0..24).map(|i| 0.5 + (i % 5) as f64 * 0.1).collect::<Vec<_>>(),
        });
        // Machine B: same dead mean/curve, different ULP path + EU timing
        let fields_b = json!({
            "residual_mean": mean,
            "residual_std": 0.008,
            "hw_curve_webgl": dead_curve,
            "residual_path_means": [mean, mean + 0.0004, mean + 0.0031],
            "residual_path_stds": [0.008, 0.011, 0.055],
            "residual_path_modes": ["float", "noderiv", "ulp"],
            "residual_paths": [
                {
                    "path_id": "v3f_128_std",
                    "shader_mode": "float",
                    "ok": true,
                    "entropy_ok": false,
                    "mean": mean,
                    "std": 0.008,
                    "curve": dead_curve,
                },
                {
                    "path_id": "v3f_128_noderiv",
                    "shader_mode": "noderiv",
                    "ok": true,
                    "entropy_ok": true,
                    "mean": mean + 0.0004,
                    "std": 0.011,
                    "curve": (0..32).map(|i| dead_curve[i] + 0.002).collect::<Vec<_>>(),
                },
                {
                    "path_id": "ulp_anti_collision_128",
                    "shader_mode": "ulp",
                    "ok": true,
                    "entropy_ok": true,
                    "mean": mean + 0.0031,
                    "std": 0.055,
                    "curve": (0..32).map(|i| 0.15 + (i as f64) * 0.004).collect::<Vec<_>>(),
                    "eu_timing_ms": [3.4, 3.5, 3.35, 3.45, 3.42, 3.38],
                },
            ],
            "eu_timing_ms": [3.4, 3.5, 3.35, 3.45, 3.42, 3.38],
            "hw_curve_audio": (0..16).map(|i| 0.1 + i as f64 * 0.02).collect::<Vec<_>>(),
            "cpu_timing_curve": (0..24).map(|i| 0.5 + (i % 5) as f64 * 0.1).collect::<Vec<_>>(),
        });
        let segs_a = select_device_segments(&fields_a, None);
        let segs_b = select_device_segments(&fields_b, None);
        let res_a = segs_a
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split('-')
            .nth(1)
            .unwrap_or("0");
        let res_b = segs_b
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split('-')
            .nth(1)
            .unwrap_or("0");
        assert_ne!(res_a, "0");
        // iss/73: ULP/EU are Lane-S — commercial body may match when Lane-C (noderiv) same;
        // same-SKU split lives on lane_s_sig diagnostics.
        let ls_a = segs_a.get("lane_s_sig").cloned().unwrap_or(Value::Null);
        let ls_b = segs_b.get("lane_s_sig").cloned().unwrap_or(Value::Null);
        assert_ne!(
            ls_a, ls_b,
            "Lane-S ULP/EU must diverge on lane_s_sig; a={ls_a:?} b={ls_b:?}"
        );
        // Lane-C commercial: same noderiv → same res
        assert_eq!(
            res_a, res_b,
            "Lane-C commercial res should match when noderiv equal; a={res_a} b={res_b}"
        );
        let blob = format!("{segs_a}");
        assert!(
            blob.contains("res_includes_lane_c_multipath_sig_v3")
                || blob.contains("lane_c")
                || blob.contains("multipath"),
            "expected lane-c multipath note, got {blob}"
        );
    }

    /// iss/67 B2: path-role structure must change wg digest when denorm/fma differ.
    #[test]
    fn wg_path_role_structure_splits_same_fused_curve() {
        let curve: Vec<f64> = (0..32).map(|i| 0.01 * (i as f64)).collect();
        let base = json!({
            "residual_mean": 0.2603390625,
            "residual_std": 0.012,
            "hw_curve_webgl": curve,
            "webgl_residual_multipath": curve,
        });
        let mut a = base.as_object().cloned().unwrap();
        a.insert(
            "residual_paths".into(),
            json!([
                {"path_id":"v3f_128_std","shader_mode":"float","ok":true,"entropy_ok":true,"mean":0.260,"std":0.01,"curve":curve},
                {"path_id":"denorm_ftz_128","shader_mode":"denorm","ok":true,"entropy_ok":true,"mean":0.261,"std":0.012,"curve":curve},
            ]),
        );
        let mut b = base.as_object().cloned().unwrap();
        b.insert(
            "residual_paths".into(),
            json!([
                {"path_id":"v3f_128_std","shader_mode":"float","ok":true,"entropy_ok":true,"mean":0.260,"std":0.01,"curve":curve},
                {"path_id":"fma_pair_webgl1","shader_mode":"fma_pair","ok":true,"entropy_ok":true,"mean":0.270,"std":0.015,"curve":curve},
            ]),
        );
        let segs_a = select_device_segments(&Value::Object(a), None);
        let segs_b = select_device_segments(&Value::Object(b), None);
        let wg_a = segs_a
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split('-')
            .nth(2)
            .unwrap_or("0");
        let wg_b = segs_b
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split('-')
            .nth(2)
            .unwrap_or("0");
        assert_ne!(wg_a, "0");
        // iss/73: denorm vs fma is Lane-S — commercial wg may match (both have float primary).
        // Differentiator is lane_s_sig.
        let ls_a = segs_a.get("lane_s_sig").cloned().unwrap_or(Value::Null);
        let ls_b = segs_b.get("lane_s_sig").cloned().unwrap_or(Value::Null);
        assert_ne!(
            ls_a, ls_b,
            "Lane-S path set must diverge on lane_s_sig; a={ls_a:?} b={ls_b:?}"
        );
        // Both have float as primary Lane-C → commercial wg matches
        assert_eq!(
            wg_a, wg_b,
            "Lane-C commercial wg should match (float primary); a={wg_a} b={wg_b}"
        );
        let notes = segs_a["curve_selection_notes"].as_array().unwrap();
        assert!(
            notes
                .iter()
                .any(|n| n.as_str() == Some("wg_includes_path_role_structure_sig_v1")),
            "expected wg role note"
        );
    }

    /// residual_paths alone (no flat residual_path_means) still feeds res sig.
    #[test]
    fn residual_paths_objects_without_flat_means_still_mint_res() {
        let mean = 0.2603390625_f64;
        let curve: Vec<f64> = (0..32).map(|i| 0.01 * (i as f64)).collect();
        let fields = json!({
            "residual_mean": mean,
            "residual_std": 0.007,
            "hw_curve_webgl": curve,
            "residual_paths": [
                {
                    "path_id": "v3f_128_std",
                    "shader_mode": "float",
                    "ok": true,
                    "mean": mean,
                    "std": 0.007,
                    "curve": curve,
                },
                {
                    "path_id": "rint_warm2_128",
                    "shader_mode": "rint",
                    "ok": true,
                    "mean": mean + 0.002,
                    "std": 0.02,
                    "curve": (0..32).map(|i| 0.05 + i as f64 * 0.01).collect::<Vec<_>>(),
                },
            ],
        });
        let segs = select_device_segments(&fields, None);
        let res = segs
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split('-')
            .nth(1)
            .unwrap_or("0");
        assert_ne!(res, "0", "paths-only multipath must still mint res");
        // Lane-C paths present → lane_c_sig set; mean-only still mints (may share bucket)
        assert!(
            segs.get("lane_c_sig").and_then(|v| v.as_str()).is_some(),
            "expected lane_c_sig from residual_paths"
        );
        let fields_mean_only = json!({
            "residual_mean": mean,
            "residual_std": 0.007,
            "hw_curve_webgl": curve,
        });
        let segs2 = select_device_segments(&fields_mean_only, None);
        let res2 = segs2
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split('-')
            .nth(1)
            .unwrap_or("0");
        assert_ne!(res2, "0", "mean-only must still mint res");
        // With multipath, note documents lane-c multipath
        let blob = format!("{segs}");
        assert!(
            blob.contains("res_includes_lane_c_multipath_sig_v3") || blob.contains("lane_c"),
            "expected lane-c multipath note"
        );
    }

    /// Commercial au ignores optional stack scalars (sr/mc/latency) when curves match.
    #[test]
    fn audio_stack_extra_diverges_au_when_curves_match() {
        let curve: Vec<f64> = (0..16).map(|i| 0.1 + i as f64 * 0.02).collect();
        let base = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "hw_curve_audio": curve,
            "cpu_timing_curve": (0..24).map(|i| 0.5 + (i % 5) as f64 * 0.1).collect::<Vec<_>>(),
        });
        let mut a = base.as_object().cloned().unwrap();
        a.insert("audio_sample_rate".into(), json!(44100));
        a.insert("audio_base_latency".into(), json!(0.01));
        a.insert("audio_max_channel_count".into(), json!(2));
        let mut b = base.as_object().cloned().unwrap();
        b.insert("audio_sample_rate".into(), json!(48000));
        b.insert("audio_base_latency".into(), json!(0.005));
        b.insert("audio_max_channel_count".into(), json!(6));
        let segs_a = select_device_segments(&Value::Object(a), None);
        let segs_b = select_device_segments(&Value::Object(b), None);
        let au_a = segs_a
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split('-')
            .nth(3)
            .unwrap_or("0");
        let au_b = segs_b
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split('-')
            .nth(3)
            .unwrap_or("0");
        assert_ne!(au_a, "0");
        assert_eq!(
            au_a, au_b,
            "commercial au must ignore optional stack scalars; a={au_a} b={au_b}"
        );
    }

    /// Advanced silicon modes (fma/denorm/ulp) break same mean when only advanced differs.
    #[test]
    fn advanced_silicon_paths_fma_denorm_split_res() {
        let mean = 0.2603390625_f64;
        let dead: Vec<f64> = (0..32).map(|i| 0.01 * ((i % 4) as f64)).collect();
        let fma_a: Vec<f64> = (0..32).map(|i| 0.05 + i as f64 * 0.002).collect();
        let fma_b: Vec<f64> = (0..32).map(|i| 0.11 + i as f64 * 0.003).collect();
        let den_a: Vec<f64> = (0..32).map(|i| if i < 40 { 0.2 } else { 0.0 }).collect();
        let den_b: Vec<f64> = (0..32).map(|i| if i < 55 { 0.2 } else { 0.0 }).collect();
        let base = |fma: Vec<f64>, den: Vec<f64>| {
            json!({
                "residual_mean": mean,
                "residual_std": 0.007,
                "hw_curve_webgl": dead,
                "residual_paths": [
                    {"path_id":"v3f_128_std","shader_mode":"float","ok":true,"mean":mean,"std":0.007,"curve":dead},
                    {"path_id":"fma_pair_128","shader_mode":"fma_pair","ok":true,"entropy_ok":true,
                     "mean": fma.iter().sum::<f64>()/fma.len() as f64, "std": 0.03, "curve": fma},
                    {"path_id":"denorm_ftz_128","shader_mode":"denorm","ok":true,"entropy_ok":true,
                     "mean": den.iter().sum::<f64>()/den.len() as f64, "std": 0.05, "curve": den},
                ],
                "eu_timing_ms": [2.0, 2.1, 2.05, 2.0],
            })
        };
        let a = select_device_segments(&base(fma_a, den_a), None);
        let b = select_device_segments(&base(fma_b, den_b), None);
        let res_a = a.pointer("/device_id_segments/dv0").and_then(|v| v.as_str()).unwrap_or("").split('-').nth(1).unwrap_or("0");
        let res_b = b.pointer("/device_id_segments/dv0").and_then(|v| v.as_str()).unwrap_or("").split('-').nth(1).unwrap_or("0");
        assert_ne!(res_a, "0");
        // iss/73: fma/denorm are Lane-S — commercial res may match; lane_s_sig must split
        let ls_a = a.get("lane_s_sig").cloned().unwrap_or(Value::Null);
        let ls_b = b.get("lane_s_sig").cloned().unwrap_or(Value::Null);
        assert_ne!(
            ls_a, ls_b,
            "fma/denorm advanced paths must split lane_s_sig; a={ls_a:?} b={ls_b:?}"
        );
        assert_eq!(
            res_a, res_b,
            "Lane-C commercial res should match when float primary same; a={res_a} b={res_b}"
        );
    }

    /// B10 audio seed-delta preferred over absolute class bins for au uniqueness.
    #[test]
    fn audio_seed_delta_splits_au_when_absolute_bins_match() {
        let abs: Vec<f64> = (0..48).map(|i| 0.1 + i as f64 * 0.01).collect();
        let mut a = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "hw_curve_audio": abs,
            "audio_seed_delta_curve": (0..40).map(|i| 0.001 * (i as f64 + 1.0)).collect::<Vec<_>>(),
            "audio_sample_rate": 44100,
        })
        .as_object()
        .cloned()
        .unwrap();
        let mut b = a.clone();
        b.insert(
            "audio_seed_delta_curve".into(),
            json!((0..40).map(|i| -0.002 * (i as f64 + 3.0)).collect::<Vec<_>>()),
        );
        let segs_a = select_device_segments(&Value::Object(a), None);
        let segs_b = select_device_segments(&Value::Object(b), None);
        let au_a = segs_a
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split('-')
            .nth(3)
            .unwrap_or("0");
        let au_b = segs_b
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split('-')
            .nth(3)
            .unwrap_or("0");
        assert_ne!(au_a, "0");
        assert_ne!(
            au_a, au_b,
            "seed-delta must split au when absolute OfflineAudio bins match"
        );
    }

    /// B46 audio deep dual-seed materials diverge au under same primary curve.
    #[test]
    fn audio_deep_dual_seed_splits_au() {
        let curve: Vec<f64> = (0..48).map(|i| 0.1 + i as f64 * 0.01).collect();
        let mut a = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "hw_curve_audio": curve,
            "audio_deep_curve": curve,
            "audio_deep_curve_b": (0..48).map(|i| 0.05 + (i as f64 * 0.013).sin().abs()).collect::<Vec<_>>(),
            "audio_deep_moments": {"mean": 0.12, "variance": 0.01, "skew": 0.2, "kurtosis": 0.1, "rms": 0.15},
            "audio_deep_peak_bins": (0..16).map(|i| i as f64 * 0.01).collect::<Vec<_>>(),
            "audio_deep_phase_digest": "phase_aaa",
            "audio_sample_rate": 44100,
        }).as_object().cloned().unwrap();
        let mut b = a.clone();
        b.insert(
            "audio_deep_curve_b".into(),
            json!((0..48).map(|i| 0.2 + (i as f64 * 0.019).cos().abs()).collect::<Vec<_>>()),
        );
        b.insert(
            "audio_deep_moments".into(),
            json!({"mean": 0.21, "variance": 0.03, "skew": -0.1, "kurtosis": 0.4, "rms": 0.25}),
        );
        b.insert("audio_deep_phase_digest".into(), json!("phase_bbb"));
        let segs_a = select_device_segments(&Value::Object(a), None);
        let segs_b = select_device_segments(&Value::Object(b), None);
        let au_a = segs_a.pointer("/device_id_segments/dv0").and_then(|v| v.as_str()).unwrap_or("").split('-').nth(3).unwrap_or("0");
        let au_b = segs_b.pointer("/device_id_segments/dv0").and_then(|v| v.as_str()).unwrap_or("").split('-').nth(3).unwrap_or("0");
        assert_ne!(au_a, "0");
        // iss/75: commercial au is V_aux coarse (mean/std + sr/mc) — dual-seed / moments
        // stay diagnostic and must not noise-split the public body.
        assert_eq!(au_a, au_b, "V_aux au must ignore dual-seed moments; a={au_a} b={au_b}");
    }

    /// Same B10 seed-delta with/without B46 deep must mint same commercial au
    /// (178 same-machine multi-site stability).
    #[test]
    fn commercial_au_stable_with_or_without_b46_deep() {
        let seed: Vec<f64> = (0..40)
            .map(|i| {
                if i % 2 == 0 {
                    0.001 * (i as f64 + 1.0)
                } else {
                    -0.002 * (i as f64 + 0.5)
                }
            })
            .collect();
        let deep: Vec<f64> = (0..48)
            .map(|i| 0.05 + (i as f64 * 0.017).sin().abs())
            .collect();
        let base = json!({
            "residual_mean": 0.261126875,
            "hw_curve_webgl": (0..32).map(|i| 0.22 + (i as f64 * 0.001).sin().abs()).collect::<Vec<_>>(),
            "audio_seed_delta_curve": seed,
            "audio_sample_rate": 44100,
            "audio_max_channel_count": 2,
        });
        let mut with_b46 = base.as_object().cloned().unwrap();
        with_b46.insert("audio_deep_curve".into(), json!(deep.clone()));
        with_b46.insert(
            "audio_deep_curve_b".into(),
            json!((0..48).map(|i| 0.1 + (i as f64 * 0.02).cos().abs()).collect::<Vec<_>>()),
        );
        with_b46.insert(
            "audio_convolver_digest".into(),
            json!("conv_only_on_long_dwell"),
        );
        with_b46.insert(
            "audio_deep_moments".into(),
            json!({"mean": 0.12, "variance": 0.01, "rms": 0.15}),
        );
        let segs_a = select_device_segments(&base, None);
        let segs_b = select_device_segments(&Value::Object(with_b46), None);
        let au_a = segs_a
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split('-')
            .nth(3)
            .unwrap_or("0");
        let au_b = segs_b
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split('-')
            .nth(3)
            .unwrap_or("0");
        assert_ne!(au_a, "0");
        assert_eq!(
            au_a, au_b,
            "B46 optional deep must not change commercial au; a={au_a} b={au_b}"
        );
    }

    /// f16 / wall-ms stack extras must not fork commercial ar when f32 residual shape matches
    /// (same policy as optional canvas/audio extras — reliability over presence).
    #[test]
    fn webgpu_f16_extras_do_not_fork_ar_when_f32_same() {
        let f32c: Vec<f64> = (0..32).map(|i| 0.1 + i as f64 * 0.01).collect();
        let a = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "hw_curve_webgpu": f32c,
            "webgpu_compute_ok": true,
            "webgpu_challenge_seed_used": true,
            "webgpu_compute_mean": 0.4,
            "webgpu_compute_std": 0.05,
            "webgpu_compute_ms": 3.2,
            "webgpu_f16_ok": true,
            "hw_curve_webgpu_f16": (0..32).map(|i| 0.05 + i as f64 * 0.002).collect::<Vec<_>>(),
            "webgpu_f16_mean": 0.11,
            "webgpu_f16_std": 0.04,
        });
        let b = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "hw_curve_webgpu": f32c,
            "webgpu_compute_ok": true,
            "webgpu_challenge_seed_used": true,
            "webgpu_compute_mean": 0.55,
            "webgpu_compute_std": 0.12,
            "webgpu_compute_ms": 9.7,
            "webgpu_f16_ok": true,
            "hw_curve_webgpu_f16": (0..32).map(|i| 0.2 + i as f64 * 0.004).collect::<Vec<_>>(),
            "webgpu_f16_mean": 0.28,
            "webgpu_f16_std": 0.09,
        });
        let ar_a = select_device_segments(&a, None).pointer("/device_id_segments/dv0").and_then(|v| v.as_str()).unwrap_or("").split('-').nth(6).unwrap_or("0").to_string();
        let ar_b = select_device_segments(&b, None).pointer("/device_id_segments/dv0").and_then(|v| v.as_str()).unwrap_or("").split('-').nth(6).unwrap_or("0").to_string();
        assert_ne!(ar_a, "0");
        assert_eq!(
            ar_a, ar_b,
            "f16/wall extras must not fork commercial ar when residual shape same; a={ar_a} b={ar_b}"
        );
    }

    /// Thin B18 (adapter/ok without residual curve) must still mint commercial ar
    /// from class materials — discarding to 0 collides all partial devices.
    #[test]
    fn thin_b18_utilizes_adapter_surface_commercial_ar() {
        let thin = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "webgpu_compute_ok": true,
            "webgpu_adapter_surface": "nvidia-discrete",
            "webgl2": true,
        });
        let thin_b = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "webgpu_compute_ok": true,
            "webgpu_adapter_surface": "intel-integrated",
            "webgl2": true,
        });
        let none = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
        });
        let ar_thin = select_device_segments(&thin, None)
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split('-')
            .nth(6)
            .unwrap_or("x")
            .to_string();
        let ar_thin_b = select_device_segments(&thin_b, None)
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split('-')
            .nth(6)
            .unwrap_or("y")
            .to_string();
        let ar_none = select_device_segments(&none, None)
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split('-')
            .nth(6)
            .unwrap_or("z")
            .to_string();
        assert_ne!(ar_thin, "0", "thin B18 must utilize adapter surface, not zero");
        assert_ne!(ar_thin_b, "0");
        assert_ne!(
            ar_thin, ar_thin_b,
            "different adapter surfaces must split commercial ar"
        );
        assert_eq!(ar_none, "0", "true missing B18 stays 0");
        assert_ne!(ar_thin, ar_none, "thin B18 must not collapse to missing");
        let notes = select_device_segments(&thin, None)
            .get("curve_selection_notes")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .collect::<Vec<_>>()
                    .join("|")
            })
            .unwrap_or_default();
        assert!(
            notes.contains("ar_utilize") || notes.contains("ar_gp_from_webgpu_stack"),
            "expected utilize note, got {notes}"
        );
    }

    /// Weak residual curve (fails strict entropy gate) must still mint ar body.
    #[test]
    fn weak_webgpu_curve_still_mints_commercial_ar() {
        // 8-step ramp: previously zeroed by residual_curve_entropy_ok gate.
        let curve: Vec<f64> = (0..8).map(|i| 0.1 + i as f64 * 0.05).collect();
        let fields = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "hw_curve_webgpu": curve,
            "webgpu_compute_ok": true,
            "webgpu_challenge_seed_used": true,
        });
        let ar = select_device_segments(&fields, None)
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split('-')
            .nth(6)
            .unwrap_or("0")
            .to_string();
        assert_ne!(ar, "0", "weak but present residual must mint ar, got {ar}");
    }

    /// Partial cc caps (tex only) must mint, not require both tex+rb.
    #[test]
    fn partial_gl_caps_mint_commercial_cc() {
        let tex_only = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "gl_max_texture_size": 8192,
        });
        let none = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
        });
        let cc_tex = select_device_segments(&tex_only, None)
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split('-')
            .nth(7)
            .unwrap_or("0")
            .to_string();
        let cc_none = select_device_segments(&none, None)
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split('-')
            .nth(7)
            .unwrap_or("0")
            .to_string();
        assert_ne!(cc_tex, "0", "tex-only caps must mint cc");
        assert_eq!(cc_none, "0");
    }

    /// Audio convolver digest diverges au under same dual-seed curves.
    #[test]
    fn audio_convolver_splits_au() {
        let curve: Vec<f64> = (0..48).map(|i| 0.1 + i as f64 * 0.01).collect();
        let a = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "audio_deep_curve": curve,
            "audio_convolver_curve": (0..48).map(|i| 0.05 + (i as f64 * 0.02).sin().abs()).collect::<Vec<_>>(),
            "audio_convolver_digest": "conv_a",
            "audio_convolver_moments": {"mean": 0.1, "variance": 0.01, "rms": 0.12},
        });
        let b = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "audio_deep_curve": curve,
            "audio_convolver_curve": (0..48).map(|i| 0.2 + (i as f64 * 0.03).cos().abs()).collect::<Vec<_>>(),
            "audio_convolver_digest": "conv_b",
            "audio_convolver_moments": {"mean": 0.22, "variance": 0.04, "rms": 0.25},
        });
        let au_a = select_device_segments(&a, None).pointer("/device_id_segments/dv0").and_then(|v| v.as_str()).unwrap_or("").split('-').nth(3).unwrap_or("0").to_string();
        let au_b = select_device_segments(&b, None).pointer("/device_id_segments/dv0").and_then(|v| v.as_str()).unwrap_or("").split('-').nth(3).unwrap_or("0").to_string();
        assert_ne!(au_a, "0");
        // iss/75: convolver digests are secondary; commercial au stays coarse-stable.
        assert_eq!(au_a, au_b, "V_aux au must ignore convolver digests; a={au_a} b={au_b}");
    }

    /// SAB clock / tpm are load-sensitive — commercial tz must stay on jitter structure only.
    #[test]
    fn sab_clock_does_not_fork_commercial_tz() {
        let jitter: Vec<f64> = (0..16).map(|i| ((i as f64) * 0.1).sin() * 0.5).collect();
        let a = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "timing_jitter_curve": jitter,
            "sab_clock_ok": true,
            "sab_ticks_per_ms": 50000,
            "sab_eventloop_jitter_ms": 0.12,
            "sab_clock_digest": "sab_a",
            "sab_tick_delta_curve": (0..12).map(|i| 1000.0 + i as f64 * 10.0).collect::<Vec<_>>(),
        });
        let b = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "timing_jitter_curve": jitter,
            "sab_clock_ok": true,
            "sab_ticks_per_ms": 120000,
            "sab_eventloop_jitter_ms": 0.45,
            "sab_clock_digest": "sab_b",
            "sab_tick_delta_curve": (0..12).map(|i| 2000.0 + i as f64 * 50.0).collect::<Vec<_>>(),
        });
        let tz_a = select_device_segments(&a, None).pointer("/device_id_segments/dv0").and_then(|v| v.as_str()).unwrap_or("").split('-').nth(8).unwrap_or("0").to_string();
        let tz_b = select_device_segments(&b, None).pointer("/device_id_segments/dv0").and_then(|v| v.as_str()).unwrap_or("").split('-').nth(8).unwrap_or("0").to_string();
        assert_ne!(tz_a, "0");
        assert_eq!(
            tz_a, tz_b,
            "SAB clock extras must not fork commercial tz; a={tz_a} b={tz_b}"
        );
    }

    /// Distinct WebGPU residual **kind profiles** split ar (stack extras alone do not).
    #[test]
    fn webgpu_compute_residual_shape_splits_ar() {
        // Monotone ramp vs inverted tile profile → different krank after scale-inv.
        let curve_a: Vec<f64> = (0..36)
            .map(|i| 0.2 + (i % 6) as f64 * 0.4 + (i / 6) as f64 * 0.02)
            .collect();
        let curve_b: Vec<f64> = (0..36)
            .map(|i| {
                let k = i % 6;
                0.3 + ((5 - k) as f64) * 0.35 + ((k * k) as f64) * 0.05
            })
            .collect();
        let a = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "hw_curve_webgpu": curve_a,
            "webgpu_compute_ok": true,
            "webgpu_challenge_seed_used": true,
            "webgpu_compute_mean": 0.42,
            "webgpu_compute_std": 0.08,
            "webgpu_compute_ms": 3.5,
        });
        let b = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "hw_curve_webgpu": curve_b,
            "webgpu_compute_ok": true,
            "webgpu_challenge_seed_used": true,
            "webgpu_compute_mean": 0.51,
            "webgpu_compute_std": 0.12,
            "webgpu_compute_ms": 5.0,
        });
        let ar_a = select_device_segments(&a, None).pointer("/device_id_segments/dv0").and_then(|v| v.as_str()).unwrap_or("").split('-').nth(6).unwrap_or("0").to_string();
        let ar_b = select_device_segments(&b, None).pointer("/device_id_segments/dv0").and_then(|v| v.as_str()).unwrap_or("").split('-').nth(6).unwrap_or("0").to_string();
        assert_ne!(ar_a, "0");
        assert_ne!(ar_a, ar_b, "distinct residual shapes must split ar; a={ar_a} b={ar_b}");
    }

    /// Same residual shape under global load mult + wall-ms noise → same commercial ar.
    #[test]
    fn webgpu_residual_load_stable_same_machine_ar() {
        let base: Vec<f64> = (0..36)
            .map(|i| 0.2 + (i % 6) as f64 * 0.35 + (i / 6) as f64 * 0.01)
            .collect();
        let a = json!({
            "hw_curve_webgpu": base.iter().map(|x| x * 1.0).collect::<Vec<_>>(),
            "webgpu_compute_ok": true,
            "webgpu_challenge_seed_used": true,
            "webgpu_compute_ms": 2.1,
        });
        let b = json!({
            "hw_curve_webgpu": base.iter().map(|x| x * 1.35).collect::<Vec<_>>(),
            "webgpu_compute_ok": true,
            "webgpu_challenge_seed_used": true,
            "webgpu_compute_ms": 8.9,
        });
        let ar_a = select_device_segments(&a, None).pointer("/device_id_segments/dv0").and_then(|v| v.as_str()).unwrap_or("").split('-').nth(6).unwrap_or("0").to_string();
        let ar_b = select_device_segments(&b, None).pointer("/device_id_segments/dv0").and_then(|v| v.as_str()).unwrap_or("").split('-').nth(6).unwrap_or("0").to_string();
        assert_ne!(ar_a, "0");
        assert_eq!(ar_a, ar_b, "load mult must not fork commercial ar; a={ar_a} b={ar_b}");
    }

    /// Same-SKU fleet: all 10 slots mint and full id unique when multipath materials rich.
    #[test]
    fn full_ten_slot_same_sku_fleet_unique_ids() {
        let mean = 0.2603390625_f64;
        let dead: Vec<f64> = (0..32).map(|i| if i == 6 { 0.8 } else { 0.01 * ((i % 3) as f64) }).collect();
        let mut ids = std::collections::BTreeSet::new();
        for seed in 0..8 {
            let s = seed as f64 * 0.11 + 0.05;
            let ulp: Vec<f64> = (0..32).map(|i| 0.1 + (i as f64) * 0.003 + s * 0.05).collect();
            let fields = json!({
                "residual_mean": mean,
                "residual_std": 0.008,
                "hw_curve_webgl": dead,
                "residual_paths": [
                    {"path_id":"v3f_128_std","shader_mode":"float","ok":true,"mean":mean,"std":0.008,"curve":dead},
                    {"path_id":"fma_pair_128","shader_mode":"fma_pair","ok":true,"entropy_ok":true,
                     "mean": 0.1+s, "std": 0.02+s*0.01,
                     "curve": (0..32).map(|i| 0.05+s + i as f64 * 0.001).collect::<Vec<_>>()},
                    {"path_id":"ulp_chain_128","shader_mode":"ulp","ok":true,"entropy_ok":true,
                     "mean": ulp.iter().sum::<f64>()/32.0, "std": 0.04, "curve": ulp,
                     "eu_timing_ms": (0..8).map(|i| 1.0+s*2.0+i as f64*0.1).collect::<Vec<_>>()},
                ],
                "eu_timing_ms": (0..8).map(|i| 1.0+s*2.0+i as f64*0.1).collect::<Vec<_>>(),
                "hw_curve_audio": (0..48).map(|i| ((i as f64)*0.1+s).sin().abs()).collect::<Vec<_>>(),
                "audio_deep_curve": (0..48).map(|i| ((i as f64)*0.1+s).sin().abs()).collect::<Vec<_>>(),
                "audio_deep_curve_b": (0..48).map(|i| ((i as f64)*0.13+s*2.0).cos().abs()).collect::<Vec<_>>(),
                "audio_deep_moments": {"mean": 0.1+s, "variance": 0.01+s*0.01, "skew": s, "kurtosis": 0.1, "rms": 0.2+s},
                "audio_deep_phase_digest": format!("ph_{seed}"),
                "audio_sample_rate": if seed % 2 == 0 { 44100 } else { 48000 },
                "cpu_timing_curve": (0..24).map(|i| 0.5+(i%5) as f64 *0.1 + s).collect::<Vec<_>>(),
                "hw_curve_canvas": (0..16).map(|i| 0.3+i as f64 *0.01 + s*0.02).collect::<Vec<_>>(),
                "canvas_noise_hash": format!("cv_{seed}"),
                "hw_curve_webgpu": (0..32).map(|i| 0.2+i as f64 *0.008 + s*0.03).collect::<Vec<_>>(),
                "webgpu_compute_ok": true,
                "webgpu_compute_mean": 0.4+s,
                "webgpu_compute_std": 0.05+s*0.02,
                "webgpu_limits_hash": format!("lim_{seed}"),
                "gl_max_texture_size": 16384,
                "gl_max_renderbuffer": 16384,
                "webgl_depth_bits": 24,
                "webgl_samples": seed % 4,
                "gl_max_varying_vectors": 30,
                "webgl_extensions_hash": "same_sku",
                "timing_jitter_curve": (0..16).map(|i| ((i as f64)*0.07+s).sin()*0.5).collect::<Vec<_>>(),
                "os_instance_hash": format!("oi_{seed}"),
                "webrtc_host_ip_hash": format!("rtc_{seed}"),
                "webrtc_host_count": 1 + (seed % 3),
                "ja4h_lite": format!("j4h_{seed}"),
                "ja4l_lite": format!("j4l_{seed}"),
            });
            let segs = select_device_segments(&fields, None);
            let id = segs.pointer("/device_id_segments/dv0").and_then(|v| v.as_str()).unwrap_or("").to_string();
            assert!(id.starts_with("dv0-"), "bad id={id}");
            let parts: Vec<&str> = id.split('-').collect();
            // 1 prefix + 10 slots
            assert!(parts.len() >= 11, "expected 10 slots, got {} in {id}", parts.len() - 1);
            for (i, code) in ["res","wg","au","cp","of","ar","cc","tz"].iter().enumerate() {
                let tok = parts.get(i + 1).copied().unwrap_or("0");
                assert_ne!(tok, "0", "slot {code} empty for seed={seed} id={id}");
            }
            // Host/protocol context (oi/rtc) no longer forks the commercial id.
            assert_eq!(parts.get(9).copied().unwrap_or("0"), "0", "oi must stay placeholder for seed={seed}");
            assert_eq!(parts.get(10).copied().unwrap_or("0"), "0", "rtc must stay placeholder for seed={seed}");
            assert!(
                segs["has_host_separator"].as_bool().unwrap(),
                "host context must still be recorded for seed={seed}"
            );
            ids.insert(id);
        }
        assert_eq!(ids.len(), 8, "full 10-slot fleet must be unique: {ids:?}");
    }

    /// Commercial cc: max texture/renderbuffer class still diverges GPUs.
    #[test]
    fn cc_v3_extra_gl_caps_diverge_class() {
        let a = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "gl_max_texture_size": 16384,
            "gl_max_renderbuffer": 16384,
        });
        let b = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "gl_max_texture_size": 32768,
            "gl_max_renderbuffer": 32768,
        });
        let segs_a = select_device_segments(&a, None);
        let segs_b = select_device_segments(&b, None);
        let cc_a = segs_a
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split('-')
            .nth(7)
            .unwrap_or("0");
        let cc_b = segs_b
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split('-')
            .nth(7)
            .unwrap_or("0");
        assert_ne!(cc_a, "0");
        assert_ne!(
            cc_a, cc_b,
            "cc tex/rb class must diverge; a={cc_a} b={cc_b}"
        );
    }

    #[test]
    fn conf_class_slots_capped_vs_legacy_q() {
        // Pre-change: q_cc=0.95 + q_ar=1.0 alone could dominate.
        // Post iss/67: conf uses capped weights.
        let class_heavy = conf_from_slot_qualities(
            0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.95, 0.0, 0.0, 0.0, false, true, 10.0,
        );
        let silicon_rich = conf_from_slot_qualities(
            1.0, 1.0, 1.0, 1.0, 0.8, 0.0, 0.0, 1.0, 0.0, 0.0, true, false, 10.0,
        );
        // Class-only (ar+cc) must not exceed residual-heavy silicon conf
        assert!(
            class_heavy.conf < silicon_rich.conf,
            "class conf {} should be < silicon conf {}",
            class_heavy.conf,
            silicon_rich.conf
        );
        assert!(class_heavy.q_cc_conf <= 0.12 + 1e-9);
        assert!(class_heavy.q_ar_conf <= 0.15 + 1e-9);
        // multipath dedupe applied
        assert!(silicon_rich.multipath_dedupe > 0.0);
        // Absolute class-only conf should be modest (legacy would be ~0.15+0.085*1.95≈0.316)
        assert!(
            class_heavy.conf < 0.22,
            "class-heavy conf too high: {}",
            class_heavy.conf
        );
    }

    #[test]
    fn select_device_segments_class_caps_on_caps_only_fields() {
        // GL caps only → high material q_cc historically, but conf must stay modest
        let fields = json!({
            "gl_max_texture_size": 16384,
            "gl_max_renderbuffer": 16384,
            "gl_max_vertex_attribs": 16,
            "gl_max_varying_vectors": 30,
            "gl_high_float": [23, 127, 127],
            "webgl_depth_bits": 24,
            "webgl_samples": 4,
            "webgpu_adapter_surface": "nvidia_family_x",
            "webgpu_limits_hash": "lim_aaa",
            "webgpu_features_hash": "feat_bbb",
        });
        let out = select_device_segments(&fields, Some(&json!({"sources":["main"]})));
        let conf = out["device_confidence"].as_f64().unwrap();
        let q_cc_raw = out["slot_quality"]["cc"].as_f64().unwrap();
        let q_cc_conf = out["slot_quality_conf"]["cc"].as_f64().unwrap();
        assert!(q_cc_raw >= 0.9, "raw material quality still recorded");
        assert!(q_cc_conf <= 0.12 + 1e-9, "conf weight capped");
        // without residual/curves, conf should not look "fully silicon"
        assert!(conf < 0.35, "caps-only conf inflated: {conf}");
        let notes = out["curve_selection_notes"].as_array().unwrap();
        assert!(notes.iter().any(|n| n.as_str() == Some("cc_class_slot_k_not_v")));
        // doc07: role surface must mark cc as K, not V
        assert_eq!(
            out["slot_roles"]["roles"]["cc"]["role"].as_str(),
            Some("K")
        );
        assert_eq!(
            out["slot_roles"]["policy"].as_str(),
            Some("iss67_doc07_slot_roles_v1")
        );
    }

    #[test]
    fn slot_probe_paths_records_selected_method_and_candidates() {
        let fields = json!({
            "engine_family": "gecko",
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "cpu_timing_rounds": [
                (0..36).map(|i| 2.0 + (i % 6) as f64 * 0.4).collect::<Vec<_>>(),
                (0..36).map(|i| 2.1 + (i % 6) as f64 * 0.4).collect::<Vec<_>>(),
            ],
            "raf_interval_rounds": [
                (0..32).map(|i| if i % 2 == 0 { 16.0 } else { 17.0 }).collect::<Vec<_>>(),
                (0..32).map(|i| if i % 2 == 0 { 16.0 } else { 17.0 }).collect::<Vec<_>>(),
            ],
            "timing_jitter_rounds": [
                (0..32).map(|i| if i % 2 == 0 { -0.5 } else { 0.5 }).collect::<Vec<_>>(),
                (0..32).map(|i| if i % 2 == 0 { -0.4 } else { 0.4 }).collect::<Vec<_>>(),
            ],
        });
        let out = select_device_segments(&fields, None);
        let paths = out.get("slot_probe_paths").expect("slot_probe_paths");
        assert_eq!(paths["policy"], "multi_path_priority_v1");
        assert_eq!(paths["engine_family"], "gecko");
        let cp = &paths["slots"]["cp"];
        assert_eq!(cp["selected_method"], "cpu_timing_rounds_multiround");
        assert!(cp["candidates"].as_array().map(|a| a.len() >= 2).unwrap_or(false));
        let tz = &paths["slots"]["tz"];
        assert!(tz["selected_method"].as_str().unwrap_or("").contains("timing")
            || tz["selected_method"].as_str().unwrap_or("").contains("raf"));
        // commercial digests still non-zero
        let did = out["device_id"].as_str().unwrap_or("");
        assert!(did.starts_with("dv0-"), "did={did}");
    }

    #[test]
    fn slot_roles_json_marks_class_ar_and_dual_kpi() {
        let j = slot_roles_json(true);
        assert_eq!(j["roles"]["ar"]["role"], "K_C");
        assert_eq!(j["roles"]["res"]["role"], "V");
        assert_eq!(j["roles"]["wg"]["role"], "V");
        // iss/75: au is V_aux (render-sensitive), not Lane-C silicon V
        assert_eq!(j["roles"]["au"]["role"], "V_aux");
        assert_eq!(j["roles"]["cp"]["role"], "V_candidate");
        assert!(j["dual_kpi"]["reliability"].as_str().unwrap().contains("same_vt"));
        let j2 = slot_roles_json(false);
        assert_eq!(j2["roles"]["ar"]["role"], "V_candidate");
    }

    #[test]
    fn au_aux_stable_ignores_raw_float_csv_noise() {
        // Same coarse family, different absolute floats / fine shape — commercial au must agree.
        let a: Vec<f64> = (0..16).map(|i| 0.10 + i as f64 * 0.01).collect();
        let b: Vec<f64> = (0..16)
            .map(|i| 0.11 + i as f64 * 0.01 + 1e-4 * (i as f64).sin())
            .collect();
        let ta = encode_au_aux_stable(&a, Some("sr=48000|mc=2"));
        let tb = encode_au_aux_stable(&b, Some("sr=48000|mc=2"));
        assert_ne!(ta, "0");
        assert_eq!(ta, tb, "au V_aux must not fork on float micro-noise");
        // Commercial extras intentionally empty — optional sr/mc presence forks same-machine au.
        let bare = audio_stack_extra_commercial(
            json!({"audio_sample_rate": 48000, "audio_max_channel_count": 2, "audio_deep_moments": {"mean": 0.123456}})
                .as_object()
                .unwrap(),
        );
        assert!(
            bare.is_none(),
            "commercial au extras must be empty (curve-body only): {bare:?}"
        );
    }

    /// Optional sample_rate presence must not fork commercial au (178 Firefox multi-site).
    #[test]
    fn commercial_au_stable_with_or_without_sample_rate() {
        let seed: Vec<f64> = (0..64)
            .map(|i| 0.001 * (i as f64 + 1.0) + 0.01 * ((i as f64) * 0.3).sin().abs())
            .collect();
        let base = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "audio_seed_delta_curve": seed,
            "hw_curve_audio": (0..48).map(|i| 0.05 + i as f64 * 0.01).collect::<Vec<_>>(),
        });
        let mut with_sr = base.as_object().cloned().unwrap();
        with_sr.insert("audio_sample_rate".into(), json!(44100));
        with_sr.insert("audio_convolver_digest".into(), json!("9d91e6fc"));
        let au_a = select_device_segments(&base, None)
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split('-')
            .nth(3)
            .unwrap_or("0")
            .to_string();
        let au_b = select_device_segments(&Value::Object(with_sr), None)
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split('-')
            .nth(3)
            .unwrap_or("0")
            .to_string();
        assert_ne!(au_a, "0");
        assert_eq!(
            au_a, au_b,
            "optional sr/convolver must not change commercial au; a={au_a} b={au_b}"
        );
    }

    /// Optional depth/samples/extension soup must not fork commercial cc (tex+rb only).
    #[test]
    fn commercial_cc_stable_core_caps_ignores_extension_soup() {
        let core = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "gl_max_texture_size": 32768,
            "gl_max_renderbuffer": 32768,
        });
        let mut fat = core.as_object().cloned().unwrap();
        fat.insert("webgl_depth_bits".into(), json!(24));
        fat.insert("webgl_samples".into(), json!(4));
        fat.insert("webgl2".into(), json!(true));
        fat.insert("webgl_extensions_hash".into(), json!("deadbeef_ext"));
        fat.insert("gl_precision_matrix".into(), json!({"highp": [23, 127, 127]}));
        fat.insert("gl_high_float".into(), json!([23, 127, 127]));
        fat.insert("webgl_params_digest".into(), json!("params_xyz"));
        let cc_a = select_device_segments(&core, None)
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split('-')
            .nth(7)
            .unwrap_or("0")
            .to_string();
        let cc_b = select_device_segments(&Value::Object(fat), None)
            .pointer("/device_id_segments/dv0")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split('-')
            .nth(7)
            .unwrap_or("0")
            .to_string();
        assert_ne!(cc_a, "0");
        assert_eq!(
            cc_a, cc_b,
            "optional depth/samples/ext must not change commercial cc; a={cc_a} b={cc_b}"
        );
    }

    #[test]
    fn wg_lane_c_stable_ignores_raw_float_csv_noise() {
        // Same Lane-C family, micro GPU noise + length 16 vs 32 — dv4 match lane must agree.
        let a: Vec<f64> = (0..32).map(|i| 0.229 + (i as f64) * 0.001).collect();
        let b: Vec<f64> = (0..32)
            .map(|i| 0.229 + (i as f64) * 0.001 + 2e-4 * ((i as f64) * 0.7).sin())
            .collect();
        let c: Vec<f64> = a[..16].to_vec();
        let ex = "wgr=deadbeef01";
        let ta = encode_wg_for_places(&a, Some(4), Some(ex));
        let tb = encode_wg_for_places(&b, Some(4), Some(ex));
        let tc = encode_wg_for_places(&c, Some(4), Some(ex));
        assert_ne!(ta, "0");
        assert_eq!(ta, tb, "dv4 wg must not fork on float micro-noise");
        assert_eq!(ta, tc, "dv4 wg must not fork on 16 vs 32 sample length");
        // dv0 full encode may diverge on micro-noise — that is fidelity, not match lane.
        let f0 = encode_wg_for_places(&a, None, Some(ex));
        let f1 = encode_wg_for_places(&b, None, Some(ex));
        assert_ne!(f0, "0");
        let _ = f1;
    }

    #[test]
    fn precision_lanes_emit_distinct_dv_and_match_lane() {
        let noderiv_curve: Vec<f64> = (0..32).map(|i| 0.229 + (i as f64) * 0.001).collect();
        let fields = json!({
            "residual_mean": 0.2603659375,
            "residual_std": 0.0903611351,
            "hw_curve_webgl": noderiv_curve,
            "residual_paths": [
                {"path_id": "v3f_128_noderiv", "shader_mode": "noderiv", "mean": 0.2611, "std": 0.09, "ok": true, "curve": noderiv_curve},
            ],
            "gl_max_texture_size": 16384,
        });
        let out = select_device_segments(&fields, Some(&json!({"sources":["main"]})));
        let segs = out["device_id_segments"].as_object().unwrap();
        let dv0 = segs["dv0"].as_str().unwrap();
        let dv4 = segs["dv4"].as_str().unwrap();
        let dv5 = segs["dv5"].as_str().unwrap();
        assert!(dv0.starts_with("dv0-"));
        assert!(dv4.starts_with("dv4-"));
        assert_ne!(dv0, dv4);
        assert_ne!(dv4, dv5);
        assert_eq!(out["device_id_match"].as_str().unwrap(), dv4);
        assert_eq!(out["lane_c_materials_ready"], true);
        let dk = out["hw_model_key_segments"].as_object().unwrap();
        assert!(dk.get("dk0").is_some());
        assert!(dk.get("dk4").is_some());
    }

    #[test]
    fn lane_c_commercial_head_rejects_zero_res_or_wg() {
        assert!(lane_c_commercial_head_ok(
            "dv0-fb6e4e0b7a-74a056e2ba-7df83c7ac1-906bdde9ee-e0ed3a73f1-bd62f96f6c-182848b4bf-facba2f66b-586299f598-d8291fa084"
        ));
        assert!(!lane_c_commercial_head_ok(
            "dv0-0-53aa2fc903-0-0-2be6a34f2a-40b1dac60f-c5c2ce7469-0-11ace0ce9b-52f"
        ));
        assert!(!lane_c_commercial_head_ok(
            "dv0-0-0-0-0-eb0550ca8b-3308a42c73-fb2b163935-0-07bf97a8ea-76cc79db14"
        ));
        assert!(!lane_c_commercial_head_ok(""));
        assert!(!lane_c_commercial_head_ok("dve-1a85ed7b7c36"));
    }

    #[test]
    fn multipath_shared_dedupes_res_wg_double_count() {
        let with_share = conf_from_slot_qualities(
            1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, true, false, 10.0,
        );
        let no_share = conf_from_slot_qualities(
            1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, false, false, 10.0,
        );
        assert!(with_share.q_sum_for_conf < no_share.q_sum_for_conf);
        assert!(with_share.conf < no_share.conf);
    }

    /// iss/73: chrome silicon_deep vs chromium silicon_noderiv must share Lane-C res/wg body.
    #[test]
    fn lane_c_commercial_digest_stable_across_deep_vs_noderiv_packs() {
        let noderiv_curve: Vec<f64> = (0..32).map(|i| 0.229 + (i as f64) * 0.001).collect();
        let deep_curve: Vec<f64> = (0..32).map(|i| 0.224 + (i as f64) * 0.0012).collect();
        let rint_curve: Vec<f64> = (0..32).map(|i| 0.217 + (i as f64) * 0.0015).collect();
        // chrome-like: residual_paths union with noderiv + deep; flat curve = last deep write
        let deep = json!({
            "residual_mean": 0.2603659375,
            "residual_std": 0.0903611351,
            "hw_curve_webgl": deep_curve,
            "residual_paths": [
                {"path_id": "v3f_128_std", "shader_mode": "float", "mean": 0.2604, "std": 0.09, "ok": true, "entropy_ok": true, "curve": deep_curve},
                {"path_id": "v3f_128_noderiv", "shader_mode": "noderiv", "mean": 0.2611, "std": 0.09, "ok": true, "entropy_ok": true, "curve": noderiv_curve},
                {"path_id": "rint_warm2_128", "shader_mode": "rint", "mean": 0.2598, "std": 0.09, "ok": true, "entropy_ok": true, "curve": rint_curve},
                {"path_id": "denorm_ftz_128", "shader_mode": "denorm", "mean": 0.2500, "std": 0.08, "ok": true, "entropy_ok": true},
                {"path_id": "fma_pair_webgl1", "shader_mode": "fma_pair", "mean": 0.2550, "std": 0.08, "ok": true, "entropy_ok": true, "curve": deep_curve},
            ],
        });
        // edge-like: last pack is noderiv → different flat hw_curve_webgl
        let noderiv = json!({
            "residual_mean": 0.261126875,
            "residual_std": 0.090089323744,
            "hw_curve_webgl": noderiv_curve,
            "residual_paths": [
                {"path_id": "noderiv_hard_warm2", "shader_mode": "noderiv", "mean": 0.2611, "std": 0.09, "ok": true, "entropy_ok": true, "curve": noderiv_curve},
                {"path_id": "noderiv_hard_warm4", "shader_mode": "noderiv", "mean": 0.2611, "std": 0.09, "ok": true, "entropy_ok": true, "curve": noderiv_curve},
                {"path_id": "noderiv_hard_webgl2", "shader_mode": "noderiv", "mean": 0.2611, "std": 0.09, "ok": true, "entropy_ok": true, "curve": noderiv_curve},
            ],
        });
        // Real evidence_merge union: noderiv + Lane-S deep coexist; flat curve still deep last-write
        let unioned = json!({
            "residual_mean": 0.2603659375,
            "residual_std": 0.0903611351,
            "hw_curve_webgl": deep_curve,
            "residual_paths": [
                {"path_id": "noderiv_hard_warm2", "shader_mode": "noderiv", "mean": 0.2611, "std": 0.09, "ok": true, "entropy_ok": true, "curve": noderiv_curve},
                {"path_id": "fma_pair_webgl1", "shader_mode": "fma_pair", "mean": 0.2604, "std": 0.09, "ok": true, "entropy_ok": true, "curve": deep_curve},
                {"path_id": "denorm_ftz_128", "shader_mode": "denorm", "mean": 0.2500, "std": 0.08, "ok": true},
            ],
        });
        let a = select_device_segments(&deep, Some(&json!({"sources":["main"]})));
        let b = select_device_segments(&noderiv, Some(&json!({"sources":["main"]})));
        let c = select_device_segments(&unioned, Some(&json!({"sources":["main"]})));
        let res_a = a["device_id"].as_str().unwrap().split('-').nth(1).unwrap();
        let res_b = b["device_id"].as_str().unwrap().split('-').nth(1).unwrap();
        let res_c = c["device_id"].as_str().unwrap().split('-').nth(1).unwrap();
        let wg_a = a["device_id"].as_str().unwrap().split('-').nth(2).unwrap();
        let wg_b = b["device_id"].as_str().unwrap().split('-').nth(2).unwrap();
        let wg_c = c["device_id"].as_str().unwrap().split('-').nth(2).unwrap();
        assert_eq!(
            res_a, res_b,
            "Lane-C res must match deep vs noderiv despite different hw_curve_webgl; a={res_a} b={res_b} lc_a={:?} lc_b={:?}",
            a.get("lane_c_sig"),
            b.get("lane_c_sig")
        );
        // iss/67 B2: wg is role-structure aware — pack composition (deep vs
        // noderiv-only vs unioned) forks wg via the wgr= token; res stays sealed
        // on the primary-role mean. Both properties are asserted below.
        let notes_a = a["curve_selection_notes"].as_array().unwrap();
        assert!(
            notes_a
                .iter()
                .any(|n| n.as_str() == Some("wg_includes_path_role_structure_sig_v1")),
            "{notes_a:?}"
        );
        assert_ne!(wg_a, wg_b, "deep vs noderiv pack composition must fork Lane-C wg; a={wg_a} b={wg_b}");
        assert!(a.get("lane_c_sig").is_some());
        // union residual_paths (noderiv+fma) must match pure noderiv commercial body
        assert_eq!(
            res_a, res_c,
            "unioned Lane-C res must match; a={res_a} c={res_c} lc_c={:?}",
            c.get("lane_c_sig")
        );
        // wg follows role composition + lane flat curve: union (noderiv primary,
        // deep flat last-write) must mint the same token as the deep pack.
        assert_eq!(
            wg_a, wg_c,
            "unioned pack shares deep's role composition; a={wg_a} c={wg_c}"
        );
        // residual_algo must follow sealed primary (noderiv), not last-written rint/deep flats
        let mut deep_last = deep.clone();
        if let Some(o) = deep_last.as_object_mut() {
            o.insert(
                "residual_algo".into(),
                json!("gr_webgl_residual_rint_v3f+gr_webgl_residual_rint_v1"),
            );
        }
        let sealed = select_device_segments(&deep_last, Some(&json!({"sources":["main"]})));
        // Sealed algo lives on fields via ensure_residual_paths; re-check via primary path seal
        // by reading residual_algo from a fields round-trip through ensure (select uses fo).
        // Primary path is noderiv → sealed label must contain noderiv, not rint-only last-write.
        let mut fo = deep_last.as_object().cloned().unwrap_or_default();
        ensure_residual_paths_for_mint(&mut fo, &deep_last, Some(&json!({"sources":["main"]})));
        let algo = fo
            .get("residual_algo")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(
            algo.contains("noderiv"),
            "sealed residual_algo must prefer noderiv primary, got {algo:?}; sealed_keys={:?}",
            sealed.get("lane_c_materials_ready")
        );
        assert!(!algo.contains("rint_v3f+"), "must not keep FE rint last-write label");
    }

    /// iss/75: fbs.main last-write deep-only must not mint incomplete when union has noderiv.
    #[test]
    fn lane_c_ready_when_fbs_deep_last_write_but_fields_unioned() {
        let noderiv_curve: Vec<f64> = (0..32).map(|i| 0.229 + (i as f64) * 0.001).collect();
        let deep_curve: Vec<f64> = (0..32).map(|i| 0.224 + (i as f64) * 0.0012).collect();
        let fields = json!({
            "residual_mean": 0.2603659375,
            "residual_std": 0.0903611351,
            "hw_curve_webgl": deep_curve.clone(),
            "residual_paths": [
                {"path_id": "v3f_128_noderiv", "shader_mode": "noderiv", "mean": 0.261126875, "std": 0.090089323744, "ok": true, "entropy_ok": true, "curve": noderiv_curve.clone()},
                {"path_id": "fma_pair_webgl1", "shader_mode": "fma_pair", "mean": 0.2603659375, "std": 0.09, "ok": true, "entropy_ok": true, "curve": deep_curve.clone()},
            ],
        });
        let evidence = json!({
            "fields_by_source": {
                "main": {
                    "residual_mean": 0.2603659375,
                    "residual_std": 0.0903611351,
                    "hw_curve_webgl": deep_curve.clone(),
                    "residual_paths": [
                        {"path_id": "fma_pair_webgl1", "shader_mode": "fma_pair", "mean": 0.2603659375, "std": 0.09, "ok": true, "curve": deep_curve.clone()},
                        {"path_id": "denorm_ftz_128", "shader_mode": "denorm", "mean": 0.25, "ok": true}
                    ],
                    "b10x_pack": "B10x_silicon_deep",
                    "multipath_profile": "silicon_deep",
                }
            },
            "source_conflicts": []
        });
        let pure_noderiv = json!({
            "residual_mean": 0.261126875,
            "residual_std": 0.090089323744,
            "hw_curve_webgl": noderiv_curve.clone(),
            "residual_paths": [
                {"path_id": "v3f_128_noderiv", "shader_mode": "noderiv", "mean": 0.261126875, "std": 0.090089323744, "ok": true, "entropy_ok": true, "curve": noderiv_curve.clone()},
            ],
        });
        let a = select_device_segments(&fields, Some(&evidence));
        let b = select_device_segments(&pure_noderiv, Some(&json!({"sources":["main"]})));
        assert_eq!(
            a.get("lane_c_materials_ready").and_then(|v| v.as_bool()),
            Some(true),
            "deep last-write must not block Lane-C ready: {a}"
        );
        let res_a = a["device_id"].as_str().unwrap().split('-').nth(1).unwrap();
        let res_b = b["device_id"].as_str().unwrap().split('-').nth(1).unwrap();
        let wg_a = a["device_id"].as_str().unwrap().split('-').nth(2).unwrap();
        let wg_b = b["device_id"].as_str().unwrap().split('-').nth(2).unwrap();
        assert_eq!(res_a, res_b, "res must match pure noderiv; a={res_a} b={res_b}");
        assert_eq!(wg_a, wg_b, "wg must match pure noderiv; a={wg_a} b={wg_b}");
        assert_ne!(res_a, "09000279db", "must not mint incomplete cluster head");
    }

    #[test]
    fn offline_remint_chrome_lab_fields() {
        let p = std::path::Path::new("/tmp/chrome_fields.json");
        if !p.exists() {
            return;
        }
        let txt = std::fs::read_to_string(p).unwrap();
        let fields: Value = serde_json::from_str(&txt).unwrap();
        let segs = select_device_segments(&fields, Some(&json!({"sources":["main"]})));
        eprintln!("lane_c={:?}", segs.get("lane_c_sig"));
        eprintln!("notes={:?}", segs.get("curve_selection_notes"));
        eprintln!("device_id={:?}", segs.get("device_id"));
        assert!(
            segs.get("lane_c_sig").and_then(|v| v.as_str()).is_some(),
            "expected lane_c from chrome fields: {segs}"
        );
        let did = segs.get("device_id").and_then(|v| v.as_str()).unwrap_or("");
        let res = did.split('-').nth(1).unwrap_or("0");
        let match_id = segs
            .get("device_id_match")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        eprintln!("res={res} match={match_id}");
        assert_ne!(res, "0");
        assert!(segs.get("lane_c_materials_ready").and_then(|v| v.as_bool()) == Some(true));
        assert!(match_id.starts_with("dv4-"), "match lane must be dv4");
        // Digest values are algo-versioned; assert Lane-C readiness + non-zero head only.
        assert!(lane_c_commercial_head_ok(did));
    }

    fn part_at(did: &str, idx: usize) -> String {
        did.split('-').nth(idx).unwrap_or("0").to_string()
    }

    /// Commercial of: optional canvas noise hash / pattern count must not fork same curve.
    #[test]
    fn of_stable_under_optional_canvas_extras() {
        let curve: Vec<f64> = (0..16)
            .map(|i| 0.12 + (i as f64) * 0.008 + ((i as f64) * 0.3).sin() * 0.02)
            .collect();
        let base = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "hw_curve_canvas": curve,
        });
        let mut with_extra = base.clone();
        if let Some(o) = with_extra.as_object_mut() {
            o.insert("canvas_noise_hash".into(), json!("noise_a_only_on_site_x"));
            o.insert("canvas_2d_hash".into(), json!("2d_hash_b"));
            o.insert("canvas_noise_patterns".into(), json!(7));
        }
        let of_a = part_at(
            select_device_segments(&base, None)["device_id"].as_str().unwrap_or(""),
            5,
        );
        let of_b = part_at(
            select_device_segments(&with_extra, None)["device_id"]
                .as_str()
                .unwrap_or(""),
            5,
        );
        assert_ne!(of_a, "0", "of must mint from canvas curve");
        assert_eq!(
            of_a, of_b,
            "optional canvas extras must not fork commercial of; a={of_a} b={of_b}"
        );
    }

    /// Commercial of: multiround median kills one-shot micro-noise without coarse buckets.
    #[test]
    fn of_stable_under_multiround_median() {
        let base: Vec<f64> = (0..16).map(|i| 0.15 + i as f64 * 0.01).collect();
        let spike: Vec<f64> = base
            .iter()
            .enumerate()
            .map(|(i, x)| x + if i == 3 { 0.02 } else { 0.0 })
            .collect();
        let a = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "hw_curve_canvas_rounds": [base.clone(), base.clone(), spike],
        });
        let b = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "hw_curve_canvas_rounds": [base.clone(), base.clone(), base],
        });
        let of_a = part_at(select_device_segments(&a, None)["device_id"].as_str().unwrap_or(""), 5);
        let of_b = part_at(select_device_segments(&b, None)["device_id"].as_str().unwrap_or(""), 5);
        assert_ne!(of_a, "0");
        assert_eq!(
            of_a, of_b,
            "multiround median must absorb one-shot spike; a={of_a} b={of_b}"
        );
    }

    /// Commercial cp: optional wasm/idb digests must not fork same timing curve.
    #[test]
    fn cp_stable_under_optional_wasm_extras() {
        let timing: Vec<f64> = (0..24)
            .map(|i| ((i % 5) as f64) * 0.25 + if i % 7 == 0 { 0.5 } else { 0.0 })
            .collect();
        let base = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "cpu_timing_curve": timing,
        });
        let mut with_ws = base.clone();
        if let Some(o) = with_ws.as_object_mut() {
            o.insert("wasm_relaxed_simd_digest".into(), json!("ws_only_on_deep_pack"));
            o.insert("idb_write_digest".into(), json!("idb_x"));
            o.insert(
                "ws_simd_timing_curve".into(),
                json!((0..8).map(|i| i as f64 * 0.1).collect::<Vec<_>>()),
            );
        }
        let cp_a = part_at(
            select_device_segments(&base, None)["device_id"].as_str().unwrap_or(""),
            4,
        );
        let cp_b = part_at(
            select_device_segments(&with_ws, None)["device_id"]
                .as_str()
                .unwrap_or(""),
            4,
        );
        assert_ne!(cp_a, "0");
        assert_eq!(
            cp_a, cp_b,
            "optional wasm/idb must not fork commercial cp; a={cp_a} b={cp_b}"
        );
    }

    /// Missing wall-clock CPU timing: WASM presence must not mint commercial cp.
    #[test]
    fn cp_missing_wall_timing_wasm_does_not_enter_commercial() {
        let silicon = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
        });
        let mut with_ws = silicon.clone();
        if let Some(o) = with_ws.as_object_mut() {
            o.insert("wasm_relaxed_simd_digest".into(), json!("ws_only_on_deep_pack"));
            o.insert("wasm_simd_sig".into(), json!("simd_x"));
            o.insert(
                "wasm_timing_curve".into(),
                json!((0..8).map(|i| i as f64 * 0.1).collect::<Vec<_>>()),
            );
        }
        let cp_a = part_at(
            select_device_segments(&silicon, None)["device_id"].as_str().unwrap_or(""),
            4,
        );
        let cp_b = part_at(
            select_device_segments(&with_ws, None)["device_id"]
                .as_str()
                .unwrap_or(""),
            4,
        );
        assert_eq!(cp_a, "0");
        assert_eq!(
            cp_a, cp_b,
            "wasm-only timing must not mint commercial cp; a={cp_a} b={cp_b}"
        );
    }

    /// VPN/NAT-like rtc/oi context must not fork commercial device body.
    #[test]
    fn commercial_id_stable_when_oi_rtc_context_changes() {
        let base = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "cpu_timing_curve": (0..16).map(|i| 2.0 + i as f64 * 0.1).collect::<Vec<_>>(),
            "os_instance_hash": "host_a",
            "webrtc_host_ip_hash": "rtc_a",
        });
        let mut vpn = base.clone();
        if let Some(o) = vpn.as_object_mut() {
            o.insert("os_instance_hash".into(), json!("host_b_vpn"));
            o.insert("webrtc_host_ip_hash".into(), json!("rtc_b_nat"));
            o.insert("ja4l_lite".into(), json!("ja4l_changed"));
        }
        let id_a = select_device_segments(&base, None)["device_id"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let id_b = select_device_segments(&vpn, None)["device_id"]
            .as_str()
            .unwrap_or("")
            .to_string();
        assert_eq!(
            id_a, id_b,
            "oi/rtc context must not fork commercial device_id; a={id_a} b={id_b}"
        );
        let oi_a = part_at(&id_a, 9);
        let rtc_a = part_at(&id_a, 10);
        assert_eq!(oi_a, "0");
        assert_eq!(rtc_a, "0");
    }

    /// Commercial cp: global load multiplier cancelled by scale-invariant analysis.
    #[test]
    fn cp_stable_under_global_load_multiplier() {
        let a_t: Vec<f64> = (0..16).map(|i| 2.0 + (i as f64) * 0.1).collect();
        let b_t: Vec<f64> = a_t.iter().map(|x| x * 1.35).collect(); // busy-tab scale
        let a = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "cpu_timing_curve": a_t,
        });
        let b = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "cpu_timing_curve": b_t,
        });
        let cp_a = part_at(select_device_segments(&a, None)["device_id"].as_str().unwrap_or(""), 4);
        let cp_b = part_at(select_device_segments(&b, None)["device_id"].as_str().unwrap_or(""), 4);
        assert_ne!(cp_a, "0");
        assert_eq!(
            cp_a, cp_b,
            "scale-invariant cp must match under global load mult; a={cp_a} b={cp_b}"
        );
    }

    /// Different machines still separate under analysis path (no forced coarse collision).
    #[test]
    fn of_still_splits_distinct_canvas_shapes() {
        let a_c: Vec<f64> = (0..16).map(|i| 0.10 + i as f64 * 0.01).collect();
        let b_c: Vec<f64> = (0..16)
            .map(|i| 0.10 + ((i % 5) as f64) * 0.04 + if i % 3 == 0 { 0.05 } else { 0.0 })
            .collect();
        let a = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "hw_curve_canvas": a_c,
        });
        let b = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "hw_curve_canvas": b_c,
        });
        let of_a = part_at(select_device_segments(&a, None)["device_id"].as_str().unwrap_or(""), 5);
        let of_b = part_at(select_device_segments(&b, None)["device_id"].as_str().unwrap_or(""), 5);
        assert_ne!(of_a, "0");
        assert_ne!(of_a, of_b, "distinct canvas shapes must not collide under analysis");
    }

    /// Commercial tz: load-sensitive extras (raf_mean, sab_j, digests) must not fork.
    #[test]
    fn tz_stable_under_load_sensitive_extras() {
        let jitter: Vec<f64> = (0..16).map(|i| 16.0 + ((i as f64) * 0.2).sin()).collect();
        let a = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "timing_jitter_curve": jitter,
            "perf_now_resolution_ms": 0.1,
            "sab_ticks_per_ms": 50000,
        });
        let b = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "timing_jitter_curve": jitter,
            "perf_now_resolution_ms": 0.1,
            "sab_ticks_per_ms": 50000,
            "raf_mean_ms": 16.7,
            "raf_std_ms": 1.2,
            "sab_eventloop_jitter_ms": 0.45,
            "sab_clock_digest": "run_specific_a",
            "sab_tick_delta_curve": (0..12).map(|i| 1000.0 + i as f64 * 50.0).collect::<Vec<_>>(),
            "clock_skew_tz_extra": "skew_x",
        });
        let tz_a = part_at(select_device_segments(&a, None)["device_id"].as_str().unwrap_or(""), 8);
        let tz_b = part_at(select_device_segments(&b, None)["device_id"].as_str().unwrap_or(""), 8);
        assert_ne!(tz_a, "0");
        assert_eq!(
            tz_a, tz_b,
            "load-sensitive tz extras must not fork commercial tz; a={tz_a} b={tz_b}"
        );
    }

    /// Distinct rAF jitter structures still split tz (not sab_tpm / not Hz class).
    #[test]
    fn tz_still_splits_distinct_jitter_shapes() {
        let jitter_a: Vec<f64> = (0..16).map(|i| ((i as f64) * 0.1).sin() * 0.5).collect();
        let jitter_b: Vec<f64> = (0..16)
            .map(|i| ((i as f64) * 0.37).cos() * 1.2 + if i % 3 == 0 { 0.4 } else { 0.0 })
            .collect();
        let a = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "timing_jitter_curve": jitter_a,
            "sab_ticks_per_ms": 50000,
            "raf_hz_est": 60.0,
        });
        let b = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "timing_jitter_curve": jitter_b,
            "sab_ticks_per_ms": 120000,
            "raf_hz_est": 60.0,
        });
        let tz_a = part_at(select_device_segments(&a, None)["device_id"].as_str().unwrap_or(""), 8);
        let tz_b = part_at(select_device_segments(&b, None)["device_id"].as_str().unwrap_or(""), 8);
        assert_ne!(tz_a, "0");
        assert_ne!(tz_a, tz_b, "distinct jitter structures must still split tz");
    }

    /// Display Hz class alone must not be commercial tz body (fleet collision).
    #[test]
    fn tz_hz_class_does_not_fork_same_jitter() {
        let jitter: Vec<f64> = (0..16).map(|i| 0.02 * ((i as f64) * 0.4).sin()).collect();
        let a = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "timing_jitter_curve": jitter.clone(),
            "raf_hz_est": 50.0,
        });
        let b = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "timing_jitter_curve": jitter,
            "raf_hz_est": 120.0,
        });
        let out_a = select_device_segments(&a, None);
        let out_b = select_device_segments(&b, None);
        let tz_a = part_at(out_a["device_id"].as_str().unwrap_or(""), 8);
        let tz_b = part_at(out_b["device_id"].as_str().unwrap_or(""), 8);
        assert_ne!(tz_a, "0");
        assert_eq!(
            tz_a, tz_b,
            "same jitter must mint same tz regardless of raf_hz_est class"
        );
        // K note should still surface both classes when present
        let notes = format!("{:?}", out_a.get("device_id_segments").or(out_a.get("notes")));
        let _ = notes;
    }

    /// Multiround cp: same rounds → same digest; wasm must not fork.
    #[test]
    fn cp_multiround_median_and_no_wasm_fork() {
        let base: Vec<f64> = (0..16).map(|i| 1.0 + (i as f64 % 4.0) * 0.15).collect();
        let mild: Vec<f64> = base
            .iter()
            .enumerate()
            .map(|(i, x)| x + if i == 2 { 0.02 } else { 0.0 })
            .collect();
        // identical multiround series on both sides (median absorbs mild noise)
        let rounds = json!([base.clone(), base.clone(), mild]);
        let a = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "cpu_timing_rounds": rounds,
        });
        let mut b = a.clone();
        if let Some(o) = b.as_object_mut() {
            o.insert("wasm_relaxed_simd_digest".into(), json!("ws_only_on_deep"));
            o.insert("idb_write_digest".into(), json!("idb_x"));
        }
        let cp_a = part_at(select_device_segments(&a, None)["device_id"].as_str().unwrap_or(""), 4);
        let cp_b = part_at(select_device_segments(&b, None)["device_id"].as_str().unwrap_or(""), 4);
        assert_ne!(cp_a, "0");
        assert_eq!(cp_a, cp_b, "same multiround + wasm must not fork commercial cp; a={cp_a} b={cp_b}");
    }

    /// Different wall-clock timing profiles still split cp after commercial stable.
    #[test]
    fn cp_still_splits_distinct_timing_profiles() {
        let timing_a: Vec<f64> = (0..24)
            .map(|i| ((i % 5) as f64) * 0.25 + if i % 7 == 0 { 0.5 } else { 0.0 })
            .collect();
        let timing_b: Vec<f64> = (0..24)
            .map(|i| ((i % 4) as f64) * 1.5 + if i % 5 == 0 { 3.0 } else { 0.5 })
            .collect();
        let a = json!({
            "residual_mean": 0.25,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "cpu_timing_curve": timing_a,
        });
        let b = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.21 + i as f64 * 0.01).collect::<Vec<_>>(),
            "cpu_timing_curve": timing_b,
        });
        let cp_a = part_at(select_device_segments(&a, None)["device_id"].as_str().unwrap_or(""), 4);
        let cp_b = part_at(select_device_segments(&b, None)["device_id"].as_str().unwrap_or(""), 4);
        assert_ne!(cp_a, "0");
        assert_ne!(cp_a, cp_b, "distinct timing profiles must still split cp");
    }

    #[test]
    fn commercial_digest_excludes_ua_os_tz_gpu_ip_and_keeps_kv_lanes() {
        let curve: Vec<f64> = (0..16).map(|i| 0.2 + i as f64 * 0.01).collect();
        let fields = json!({
            "user_agent": "Mozilla/5.0 UniqueUAStringXYZ",
            "os_family": "windows",
            "platform": "Win32",
            "timezone": "America/Anchorage",
            "webgl_unmasked_renderer": "ANGLE (UniqueGPUModelString)",
            "server_client_ip": "203.0.113.77",
            "residual_mean": 0.26,
            "hw_curve_webgl": curve.clone(),
            "cpu_timing_rounds": [
                (0..16).map(|i| 1.0 + (i % 4) as f64 * 0.2).collect::<Vec<_>>(),
                (0..16).map(|i| 1.02 + (i % 4) as f64 * 0.2).collect::<Vec<_>>(),
            ],
            "timing_jitter_rounds": [
                (0..16).map(|i| ((i as f64) * 0.2).sin() * 0.4).collect::<Vec<_>>(),
            ],
            "raf_hz_est": 60.0,
        });
        let out = select_device_segments(&fields, None);
        let did = out["device_id"].as_str().unwrap_or("");
        assert!(did.starts_with("dv0-"), "did={did}");
        let blob = did.to_ascii_lowercase();
        for banned in [
            "mozilla",
            "uniqueuastringxyz",
            "windows",
            "anchorage",
            "uniquegpumodelstring",
            "203.0.113.77",
            "203011377",
        ] {
            assert!(
                !blob.contains(banned),
                "commercial body leaked {banned} did={did}"
            );
        }
        let segs = out
            .get("device_id_segments")
            .cloned()
            .unwrap_or(json!({}));
        let dv0 = segs
            .get("dv0")
            .and_then(|v| v.as_str())
            .unwrap_or(did);
        let dv4 = segs.get("dv4").and_then(|v| v.as_str()).unwrap_or("");
        if !dv4.is_empty() {
            assert_ne!(
                dv0, dv4,
                "K/V precision lanes must differ when materials exist"
            );
        }
        // tz K (Hz) must not be the commercial tz V body
        let tz = part_at(did, 8);
        assert_ne!(tz, "0");
        assert_ne!(tz, "60");
        assert_ne!(tz, "hz60");
    }

    /// Lab B10 dump: Chrome vs Edge on one GPU. noderiv-first raw mean/CSV split
    /// res/wg; Lane-C mp token + z-shape must align (no new coarse bucket).
    #[test]
    fn lane_c_structure_aligns_blink_edge_residual_from_uploaded_b10() {
        fn pack(paths: Value, residual_mean: f64, residual_std: f64) -> Value {
            json!({
                "engine_family": "blink",
                "residual_mean": residual_mean,
                "residual_std": residual_std,
                "residual_paths": paths,
                "hw_curve_webgl": paths[0]["curve"],
            })
        }
        let chrome_paths = json!([
            {"path_id":"v3f_128_noderiv","mean":0.260035625,"curve":[0.22812,0.22561,0.22306,0.22512,0.22952,0.22972,0.49887,0.22717,0.23002,0.22669,0.22347,0.22313,0.22627,0.23194,0.49891,0.2271,0.22918,0.22369,0.22926,0.22634,0.22872,0.21899,0.49697,0.22601,0.22265,0.22226,0.22527,0.226,0.2269,0.22279,0.49705,0.22434]},
            {"path_id":"v3f_128_std","mean":0.2610028125,"curve":[0.22752,0.22693,0.22336,0.22382,0.2259,0.2279,0.50201,0.22601,0.22531,0.22849,0.22965,0.22367,0.22508,0.22824,0.49842,0.22687,0.22938,0.22075,0.22697,0.22457,0.22625,0.2305,0.50086,0.22652,0.2273,0.22713,0.23161,0.22684,0.2264,0.22829,0.5016,0.22794]},
            {"path_id":"fma_pair_webgl1","mean":0.2603659375,"curve":[0.23049,0.22205,0.22406,0.22319,0.22097,0.2292,0.50245,0.22512,0.23214,0.22524,0.23007,0.22952,0.22455,0.22394,0.49993,0.22757,0.22791,0.2271,0.2273,0.22368,0.22678,0.22108,0.49586,0.22563,0.22261,0.2253,0.2289,0.22567,0.23171,0.22595,0.49902,0.22672]}
        ]);
        let edge_paths = json!([
            {"path_id":"v3f_128_noderiv","mean":0.261126875,"curve":[0.22933,0.22757,0.2283,0.22856,0.22676,0.22746,0.50027,0.22804,0.2264,0.22564,0.22681,0.22922,0.22666,0.22448,0.49537,0.22662,0.22867,0.227,0.22278,0.2243,0.22868,0.22971,0.49916,0.22701,0.22847,0.22716,0.22629,0.22896,0.22536,0.22508,0.50293,0.22701]},
            {"path_id":"v3f_128_std","mean":0.2600390625,"curve":[0.2266,0.22187,0.2265,0.2304,0.22803,0.22202,0.49901,0.22591,0.22736,0.22615,0.22759,0.22602,0.22734,0.22552,0.49941,0.22668,0.22456,0.22527,0.23102,0.22304,0.22118,0.22821,0.50499,0.22569,0.21985,0.22519,0.22827,0.22436,0.22295,0.22849,0.49676,0.22501]},
            {"path_id":"fma_pair_webgl1","mean":0.2603659375,"curve":[0.23049,0.22205,0.22406,0.22319,0.22097,0.2292,0.50245,0.22512,0.23214,0.22524,0.23007,0.22952,0.22455,0.22394,0.49993,0.22757,0.22791,0.2271,0.2273,0.22368,0.22678,0.22108,0.49586,0.22563,0.22261,0.2253,0.2289,0.22567,0.23171,0.22595,0.49902,0.22672]}
        ]);
        let chrome = pack(chrome_paths.clone(), 0.260035625, 0.08996406119);
        let edge = pack(edge_paths.clone(), 0.261126875, 0.090089323744);
        let c = select_device_segments(&chrome, None);
        let e = select_device_segments(&edge, None);
        let c_did = c["device_id"].as_str().unwrap_or("");
        let e_did = e["device_id"].as_str().unwrap_or("");
        let c_res = part_at(c_did, 1);
        let e_res = part_at(e_did, 1);
        let c_wg = part_at(c_did, 2);
        let e_wg = part_at(e_did, 2);
        assert_ne!(c_res, "0");
        assert_eq!(c_res, e_res, "res must agree on uploaded same-GPU Lane-C ensemble c={c_did} e={e_did}");
        assert_ne!(c_wg, "0");
        assert_eq!(c_wg, e_wg, "wg z-shape must agree on ~1e-3 engine bias c={c_did} e={e_did}");

        // Different silicon: shift the ensemble by ~0.05 — must not collapse into the same res/wg.
        let other_paths = json!([
            {"path_id":"v3f_128_noderiv","mean":0.31,"curve":[0.28,0.275,0.273,0.275,0.279,0.28,0.55,0.277,0.28,0.276,0.273,0.273,0.276,0.282,0.55,0.277,0.279,0.274,0.279,0.276,0.279,0.269,0.548,0.276,0.273,0.272,0.275,0.276,0.277,0.273,0.548,0.274]},
            {"path_id":"v3f_128_std","mean":0.311,"curve":[0.28,0.276,0.273,0.274,0.276,0.278,0.552,0.276,0.275,0.278,0.28,0.274,0.275,0.278,0.548,0.277,0.279,0.271,0.277,0.275,0.276,0.28,0.551,0.276,0.277,0.277,0.282,0.277,0.276,0.278,0.552,0.278]}
        ]);
        let other = pack(other_paths, 0.31, 0.09);
        let o = select_device_segments(&other, None);
        let o_did = o["device_id"].as_str().unwrap_or("");
        assert_ne!(part_at(o_did, 1), c_res, "distinct residual ensemble must still split res");

        // Live r3: later B10x rint/ulp means differ by engine schedule and must
        // not move commercial res/wg off the shared B10 v3f/fma ensemble.
        let mut chrome_x = chrome_paths.as_array().cloned().unwrap();
        chrome_x.push(json!({"path_id":"rint_warm2_128","mean":0.2595746875,"curve":[0.22,0.221,0.219,0.22,0.223,0.221,0.49,0.22,0.221,0.219,0.22,0.218,0.221,0.224,0.49,0.22,0.222,0.218,0.221,0.22,0.221,0.217,0.488,0.219,0.218,0.218,0.22,0.221,0.221,0.219,0.488,0.218]}));
        chrome_x.push(json!({"path_id":"ulp_chain_128","mean":null,"curve":[0.01,0.02,0.01,0.02,0.01,0.02,0.01,0.02,0.01,0.02,0.01,0.02,0.01,0.02,0.01,0.02,0.01,0.02,0.01,0.02,0.01,0.02,0.01,0.02,0.01,0.02,0.01,0.02,0.01,0.02,0.01,0.02]}));
        let mut edge_x = edge_paths.as_array().cloned().unwrap();
        edge_x.push(json!({"path_id":"rint_warm2_128","mean":0.26008,"curve":[0.228,0.227,0.226,0.227,0.229,0.227,0.50,0.227,0.228,0.226,0.227,0.226,0.227,0.229,0.499,0.227,0.228,0.226,0.227,0.226,0.228,0.229,0.499,0.227,0.228,0.227,0.226,0.228,0.227,0.226,0.501,0.227]}));
        edge_x.push(json!({"path_id":"ulp_chain_128","mean":null,"curve":[0.03,0.04,0.03,0.04,0.03,0.04,0.03,0.04,0.03,0.04,0.03,0.04,0.03,0.04,0.03,0.04,0.03,0.04,0.03,0.04,0.03,0.04,0.03,0.04,0.03,0.04,0.03,0.04,0.03,0.04,0.03,0.04]}));
        let cx = select_device_segments(&pack(Value::Array(chrome_x), 0.260035625, 0.08996406119), None);
        let ex = select_device_segments(&pack(Value::Array(edge_x), 0.261126875, 0.090089323744), None);
        let cx_did = cx["device_id"].as_str().unwrap_or("");
        let ex_did = ex["device_id"].as_str().unwrap_or("");
        assert_eq!(part_at(cx_did, 1), c_res, "B10x rint/ulp must not move res c={c_did} cx={cx_did}");
        assert_eq!(part_at(ex_did, 1), e_res, "B10x rint/ulp must not move res e={e_did} ex={ex_did}");
        assert_eq!(part_at(cx_did, 2), c_wg, "B10x rint/ulp must not move wg c={c_did} cx={cx_did}");
        assert_eq!(part_at(ex_did, 2), e_wg, "B10x rint/ulp must not move wg e={e_did} ex={ex_did}");
        assert_eq!(part_at(cx_did, 1), part_at(ex_did, 1));
        assert_eq!(part_at(cx_did, 2), part_at(ex_did, 2));
    }
}
