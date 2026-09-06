//! Collision governance (iss/58): hot-bucket mint gate · effective_bits ·
//! Fellegi–Sunter-style identity resolver · confidence surface.
//!
//! # Product decisions (locked 2026-08-07, no external wait)
//! 1. **Default posture = merge_averse (prefer split / anti-fraud)**  
//!    Wrong-merge cost ≫ wrong-split for device commercial id.
//! 2. **Ephemeral / provisional SDK semantics**  
//!    - `mint_posture`: `stable` | `deepen_required` | `ephemeral_cohort` | `empty_anchor`  
//!    - When hot: do **not** publish stable commercial id as final; either withhold
//!      (`device_id=null` + deepen) or publish **session-scoped** `dve-…` ephemeral
//!      with `ephemeral=true`, `ephemeral_ttl_ms=900_000` (15 min), and
//!      `commercial_identity_final=false`.  
//!    - SDK must treat ephemeral as **class bucket assist**, not account-grade bind.
//! 3. **Confidence on product/result surface** as calibrated estimate  
//!    `identity.confidence = 1 − FPP̂` — **not** a contractual UV SLA; ops/SDK readable.
//!
//! Algo: `identity_governance_v1`

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub const IDENTITY_GOVERNANCE_ALGO: &str = "identity_governance_v1";
pub const EPHEMERAL_TTL_MS: u64 = 900_000;
/// Hot if ≥ this many distinct session digests share the same body key in-window.
pub const HOT_BUCKET_T1: usize = 4;
pub const HOT_BUCKET_T2: usize = 12;
pub const HOT_WINDOW_MS: u64 = 3_600_000;

/// Tenant / global mint posture (product default: anti-fraud).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergePosture {
    /// Prefer split when uncertain (default).
    MergeAverse,
    /// Prefer conservative merge (marketing attribution).
    SplitAverse,
}

impl MergePosture {
    pub fn from_env_or_default() -> Self {
        match gr_abi::env::get("MERGE_POSTURE")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "split_averse" | "merge_friendly" | "marketing" => MergePosture::SplitAverse,
            _ => MergePosture::MergeAverse,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            MergePosture::MergeAverse => "merge_averse",
            MergePosture::SplitAverse => "split_averse",
        }
    }

    /// τ_merge (higher = harder to merge).
    /// Uses offline-adopted FS calibration when present (iss/60 L3).
    pub fn tau_merge(self) -> f64 {
        if let Some((m, _)) = crate::conf_cal::adopted_fs_taus() {
            return m;
        }
        match self {
            MergePosture::MergeAverse => 4.5,
            MergePosture::SplitAverse => 2.5,
        }
    }

    /// τ_split (below → definitely different machine).
    pub fn tau_split(self) -> f64 {
        if let Some((_, s)) = crate::conf_cal::adopted_fs_taus() {
            return s;
        }
        match self {
            MergePosture::MergeAverse => 0.5,
            MergePosture::SplitAverse => -0.5,
        }
    }
}

// ─── B4 hot bucket registry (process-local + file share multi-worker) ───

struct HotBucketState {
    /// body_key → list of (session_hint_hash, first_ms)
    hits: HashMap<String, Vec<(u64, u64)>>,
    /// last shared flush ms
    last_flush_ms: u64,
}

impl HotBucketState {
    fn new() -> Self {
        Self {
            hits: HashMap::new(),
            last_flush_ms: 0,
        }
    }
}

static HOT_BUCKETS: Mutex<Option<HotBucketState>> = Mutex::new(None);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn fnv1a64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Stable key for commercial **class-body** heat (slots 0–7 only).
/// Host seps (oi/rtc slots 8–9) are intentionally excluded so many machines
/// sharing a class floor collide into one heat bucket; distinct machines are
/// counted via `machine_heat_unit` (iss/59).
pub fn body_heat_key(device_id: &str, segments: Option<&Value>) -> String {
    let raw = if let Some(segs) = segments {
        segs.get("dv0")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| device_id.to_string())
    } else {
        device_id.to_string()
    };
    if raw.is_empty() {
        return String::new();
    }
    let body = raw
        .strip_prefix("dv0-")
        .or_else(|| raw.strip_prefix("dve-"))
        .unwrap_or(raw.as_str());
    let parts: Vec<&str> = body.split('-').collect();
    if parts.len() >= 8 {
        // Class floor only
        return format!("body8:{}", parts[..8].join("-"));
    }
    if let Some(rest) = raw.strip_prefix("dv0-") {
        return format!("dv0:{rest}");
    }
    if let Some(rest) = raw.strip_prefix("dve-") {
        return format!("body:{rest}");
    }
    format!("id:{raw}")
}

fn heat_level_of(n: usize) -> u8 {
    if n >= HOT_BUCKET_T2 {
        2
    } else if n >= HOT_BUCKET_T1 {
        1
    } else {
        0
    }
}

/// Pull peer worker hits from shared file into process state (union merge).
fn merge_hot_from_shared(st: &mut HotBucketState) {
    use crate::shared_governance::{as_object_mut, with_shared_json};
    if crate::shared_governance::shared_governance_dir().is_none() {
        return;
    }
    let t = now_ms();
    let pulled = with_shared_json("hot_buckets", json!({"hits":{}}), |v| {
        let hits = v
            .get("hits")
            .and_then(|h| h.as_object())
            .cloned()
            .unwrap_or_default();
        // Also write back our local into the shared view (union)
        let o = as_object_mut(v);
        let sh = o
            .entry("hits")
            .or_insert(json!({}))
            .as_object_mut()
            .unwrap();
        for (bk, local) in &st.hits {
            let arr = sh.entry(bk.clone()).or_insert(json!([])).as_array_mut().unwrap();
            let mut have: HashMap<u64, u64> = HashMap::new();
            for item in arr.iter() {
                if let Some(a) = item.as_array() {
                    if a.len() >= 2 {
                        let h = a[0].as_u64().unwrap_or(0);
                        let ts = a[1].as_u64().unwrap_or(0);
                        have.insert(h, ts);
                    }
                }
            }
            for (h, ts) in local {
                let e = have.entry(*h).or_insert(*ts);
                if *ts < *e {
                    *e = *ts;
                }
            }
            // rebuild arr
            arr.clear();
            for (h, ts) in &have {
                if t.saturating_sub(*ts) <= HOT_WINDOW_MS {
                    arr.push(json!([h, ts]));
                }
            }
            // cap
            if arr.len() > 256 {
                arr.truncate(256);
            }
        }
        hits
    });
    let Some(hits) = pulled else {
        return;
    };
    for (bk, arr_v) in hits {
        let entry = st.hits.entry(bk).or_default();
        if let Some(arr) = arr_v.as_array() {
            for item in arr {
                if let Some(a) = item.as_array() {
                    if a.len() >= 2 {
                        let h = a[0].as_u64().unwrap_or(0);
                        let ts = a[1].as_u64().unwrap_or(0);
                        if t.saturating_sub(ts) > HOT_WINDOW_MS {
                            continue;
                        }
                        if !entry.iter().any(|(eh, _)| *eh == h) {
                            entry.push((h, ts));
                        }
                    }
                }
            }
        }
        entry.retain(|(_, ts)| t.saturating_sub(*ts) <= HOT_WINDOW_MS);
        if entry.len() > 256 {
            entry.drain(0..entry.len() - 256);
        }
    }
}

fn flush_hot_to_shared(st: &mut HotBucketState) {
    use crate::shared_governance::{as_object_mut, with_shared_json};
    if crate::shared_governance::shared_governance_dir().is_none() {
        return;
    }
    let t = now_ms();
    // Debounce: flush at most every 200ms unless forced by empty last_flush
    if st.last_flush_ms > 0 && t.saturating_sub(st.last_flush_ms) < 200 {
        return;
    }
    let snapshot: HashMap<String, Vec<(u64, u64)>> = st.hits.clone();
    let _ = with_shared_json("hot_buckets", json!({"hits":{}}), |v| {
        let o = as_object_mut(v);
        o.insert("algo".into(), json!("hot_buckets_shared_v1"));
        o.insert("window_ms".into(), json!(HOT_WINDOW_MS));
        let sh = o
            .entry("hits")
            .or_insert(json!({}))
            .as_object_mut()
            .unwrap();
        for (bk, local) in &snapshot {
            let arr = sh.entry(bk.clone()).or_insert(json!([])).as_array_mut().unwrap();
            let mut have: HashMap<u64, u64> = HashMap::new();
            for item in arr.iter() {
                if let Some(a) = item.as_array() {
                    if a.len() >= 2 {
                        have.insert(a[0].as_u64().unwrap_or(0), a[1].as_u64().unwrap_or(0));
                    }
                }
            }
            for (h, ts) in local {
                let e = have.entry(*h).or_insert(*ts);
                if *ts < *e {
                    *e = *ts;
                }
            }
            arr.clear();
            for (h, ts) in have {
                if t.saturating_sub(ts) <= HOT_WINDOW_MS {
                    arr.push(json!([h, ts]));
                }
            }
            if arr.len() > 256 {
                arr.truncate(256);
            }
        }
    });
    st.last_flush_ms = t;
}

/// Which evidence class produced a heat unit (iss/opus5 P0-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeatUnitKind {
    /// Host separators from fields (os_instance / webrtc host hash).
    MachineFields,
    /// Commercial id slots 8/9 (oi/rtc digests).
    MachineSlot,
    /// Storage-continuity visitor terminal (same terminal = same machine).
    VisitorTerminal,
    /// Session-only — weak; must not accumulate bucket heat.
    SessionWeak,
}

/// Machine-level heat unit (iss/59 P1-1): prefer host seps over session_id.
/// Distinct physical-machine evidence = oi|rtc|os_instance|webrtc host hashes.
/// Session_id is **only** a fallback when no host seps exist (weak).
pub fn machine_heat_unit(fields: Option<&Value>, device_id: &str, session_id: &str) -> u64 {
    machine_heat_unit_ex(fields, device_id, session_id).0
}

/// Extended heat unit with evidence kind (iss/opus5 P0-2).
///
/// Priority: fields host seps → commercial id slots 8/9 → visitor_terminal_id
/// (storage continuity: a returning terminal must count as ONE unit, not N) →
/// session_id (weak last resort).
pub fn machine_heat_unit_ex(
    fields: Option<&Value>,
    device_id: &str,
    session_id: &str,
) -> (u64, HeatUnitKind) {
    if let Some(f) = fields {
        let oi = f
            .get("os_instance_hash")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let rtc = f
            .get("webrtc_host_ip_hash")
            .or_else(|| f.get("webrtc_host_ip_hash_v2"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !oi.is_empty() || !rtc.is_empty() {
            return (fnv1a64(&format!("m|{oi}|{rtc}")), HeatUnitKind::MachineFields);
        }
    }
    // From commercial id slots 8/9 when present
    if let Some(oi) = slot_digest_from_device_id(device_id, 8) {
        let rtc = slot_digest_from_device_id(device_id, 9).unwrap_or_default();
        if oi != "0" && !oi.is_empty() {
            return (fnv1a64(&format!("m|{oi}|{rtc}")), HeatUnitKind::MachineSlot);
        }
    }
    // iss/opus5 P0-2 (a): storage-continuity VT before session fallback.
    if let Some(f) = fields {
        let vt = f
            .get("visitor_terminal_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !vt.is_empty() {
            return (fnv1a64(&format!("v|{vt}")), HeatUnitKind::VisitorTerminal);
        }
    }
    // Weak fallback: session (discouraged — single machine multi-session must not heat)
    (
        fnv1a64(&format!("s|{session_id}")),
        HeatUnitKind::SessionWeak,
    )
}

/// Record an observation into the hot-bucket registry; return (distinct_count, heat_level).
/// heat_level: 0 cool · 1 warm (≥T1) · 2 hot (≥T2)
///
/// **Heat unit = distinct machine evidence** (oi|rtc), not session_id (iss/59).
/// Multi-worker: merges peer hits from shared governance file.
pub fn observe_bucket_heat(body_key: &str, session_id: &str) -> (usize, u8) {
    observe_bucket_heat_ex(body_key, session_id, "", None)
}

/// Extended heat observe with device_id + fields for machine-level dedup.
pub fn observe_bucket_heat_ex(
    body_key: &str,
    session_id: &str,
    device_id: &str,
    fields: Option<&Value>,
) -> (usize, u8) {
    if body_key.is_empty() {
        return (0, 0);
    }
    let t = now_ms();
    let (unit_h, unit_kind) = machine_heat_unit_ex(fields, device_id, session_id);
    // iss/opus5 P0-2 (b): session-only (weak) units must not accumulate heat —
    // collapse them to a single per-bucket "unknown" unit so N sessions from
    // one unidentified machine count once instead of N times. Real device
    // farms still carry distinguishing evidence (oi/rtc/VT) and remain visible.
    let unit_h = if unit_kind == HeatUnitKind::SessionWeak {
        fnv1a64(&format!("u|{body_key}"))
    } else {
        unit_h
    };
    let mut guard = HOT_BUCKETS.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = Some(HotBucketState::new());
    }
    let st = guard.as_mut().unwrap();
    merge_hot_from_shared(st);
    let entry = st.hits.entry(body_key.to_string()).or_default();
    entry.retain(|(_, ts)| t.saturating_sub(*ts) <= HOT_WINDOW_MS);
    if !entry.iter().any(|(h, _)| *h == unit_h) {
        entry.push((unit_h, t));
    }
    if entry.len() > 256 {
        entry.drain(0..entry.len() - 256);
    }
    let n = entry.len();
    flush_hot_to_shared(st);
    (n, heat_level_of(n))
}

pub fn reset_hot_buckets_for_tests() {
    let mut guard = HOT_BUCKETS.lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some(HotBucketState::new());
    // Clear shared file hits when test dir is set
    use crate::shared_governance::{as_object_mut, with_shared_json};
    if crate::shared_governance::shared_governance_dir().is_some() {
        let _ = with_shared_json("hot_buckets", json!({"hits":{}}), |v| {
            let o = as_object_mut(v);
            o.insert("hits".into(), json!({}));
        });
    }
}

// ─── B7 effective bits ─────────────────────────────────────────────────────

fn curve_from(fo: &Map<String, Value>, keys: &[&str]) -> Vec<f64> {
    for k in keys {
        if let Some(arr) = fo.get(*k).and_then(|v| v.as_array()) {
            let xs: Vec<f64> = arr.iter().filter_map(|x| x.as_f64()).filter(|x| x.is_finite()).collect();
            if xs.len() >= 4 {
                return xs;
            }
        }
    }
    Vec::new()
}

fn uniq_approx(xs: &[f64], quanta: f64) -> usize {
    if xs.is_empty() {
        return 0;
    }
    let mut bins: Vec<i64> = xs
        .iter()
        .map(|x| (x / quanta.max(1e-12)).round() as i64)
        .collect();
    bins.sort_unstable();
    bins.dedup();
    bins.len()
}

fn stability_bits_from_std(std: f64, n: usize) -> f64 {
    // Higher std + length → more structure; crude proxy until multi-tick labels.
    if n < 4 {
        return 0.0;
    }
    let s = std.max(0.0);
    let bits = (s * 80.0).ln_1p() * 2.5 + (n as f64 / 16.0).min(4.0);
    bits.clamp(0.0, 24.0)
}

fn uniqueness_bits_from_uniq(uniq: usize, n: usize) -> f64 {
    if n < 4 || uniq <= 1 {
        return 0.5;
    }
    let ratio = uniq as f64 / n as f64;
    (ratio * 16.0 + (uniq as f64).log2().max(0.0)).clamp(0.0, 24.0)
}

/// Per-slot effective_bits = min(stability, uniqueness) proxies from single observation.
pub fn slot_effective_bits(fields: &Value) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let mut rows = Map::new();
    let slots: &[(&str, &[&str], f64)] = &[
        ("res", &["webgl_residual_multipath", "hw_curve_webgl"], 0.002),
        ("wg", &["hw_curve_webgl"], 0.002),
        (
            "au",
            &[
                "audio_deep_curve",
                "audio_seed_delta_curve",
                "audio_noise_delta",
                "hw_curve_audio",
            ],
            0.001,
        ),
        ("cp", &["cpu_timing_curve", "hw_curve_cpu"], 0.01),
        ("of", &["hw_curve_canvas"], 0.005),
        ("ar", &["hw_curve_webgpu", "hw_curve_webgpu_f16"], 0.002),
        ("tz", &["timing_jitter_curve", "raf_interval_curve"], 0.02),
    ];
    let mut total_eff = 0.0;
    let mut collapsed = Vec::new();
    for (slot, keys, q) in slots {
        let c = curve_from(&fo, keys);
        let n = c.len();
        let mean = if n > 0 {
            c.iter().sum::<f64>() / n as f64
        } else {
            0.0
        };
        let std = if n > 1 {
            (c.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n as f64).sqrt()
        } else {
            0.0
        };
        let uniq = uniq_approx(&c, *q);
        let stab = stability_bits_from_std(std, n);
        let uniq_b = uniqueness_bits_from_uniq(uniq, n);
        let eff = stab.min(uniq_b);
        total_eff += eff;
        if n >= 8 && eff < 2.0 {
            collapsed.push(slot.to_string());
        }
        rows.insert(
            (*slot).into(),
            json!({
                "n": n,
                "std": (std * 1e6).round() / 1e6,
                "uniq_bins": uniq,
                "stability_bits": (stab * 100.0).round() / 100.0,
                "uniqueness_bits": (uniq_b * 100.0).round() / 100.0,
                "effective_bits": (eff * 100.0).round() / 100.0,
                "collapsed": n >= 8 && eff < 2.0,
            }),
        );
    }
    // Class-level slots (cc/oi/rtc) — digest presence only
    for (slot, present) in [
        (
            "cc",
            fo.get("webgl_extensions_hash").is_some()
                || fo.get("gl_high_float").is_some()
                || fo.get("webgl_max_viewport").is_some(),
        ),
        (
            "oi",
            fo.get("os_instance_hash")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty()),
        ),
        (
            "rtc",
            fo.get("webrtc_host_ip_hash")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty()),
        ),
    ] {
        let eff = if present { 3.0 } else { 0.0 };
        total_eff += eff;
        rows.insert(
            slot.into(),
            json!({
                "n": if present { 1 } else { 0 },
                "effective_bits": eff,
                "stability_bits": eff,
                "uniqueness_bits": if present { 2.0 } else { 0.0 },
                "collapsed": false,
                "class_slot": true,
            }),
        );
    }
    json!({
        "algo": "effective_bits_v1",
        "slots": rows,
        "total_effective_bits": (total_eff * 100.0).round() / 100.0,
        "collapsed_slots": collapsed,
        "alert_collapse": !collapsed.is_empty(),
    })
}

// ─── B1 Fellegi–Sunter style score (single observation readiness + pair) ───

fn slot_digest_from_device_id(device_id: &str, idx: usize) -> Option<String> {
    let body = device_id
        .strip_prefix("dv0-")
        .or_else(|| device_id.strip_prefix("dve-"))
        .unwrap_or(device_id);
    let parts: Vec<&str> = body.split('-').collect();
    parts.get(idx).map(|s| s.to_string())
}

/// Per-slot Fellegi–Sunter m/u priors (population). Online census updates when available.
/// m = P(agree | same machine), u = P(agree | different machines).
#[derive(Clone, Copy)]
struct SlotMu {
    m: f64,
    u: f64,
}

fn default_slot_mu(name: &str) -> SlotMu {
    // Class-heavy slots: high u (many machines share) → low weight when agree
    match name {
        "cc" => SlotMu { m: 0.92, u: 0.75 },
        "of" => SlotMu { m: 0.88, u: 0.35 },
        "ar" => SlotMu { m: 0.85, u: 0.25 },
        "res" | "wg" => SlotMu { m: 0.90, u: 0.40 }, // dead residual class → high u in practice
        "au" => SlotMu { m: 0.88, u: 0.30 },
        "cp" => SlotMu { m: 0.80, u: 0.20 },
        "tz" => SlotMu { m: 0.82, u: 0.22 },
        "oi" | "rtc" => SlotMu { m: 0.95, u: 0.05 }, // host seps: low u
        _ => SlotMu { m: 0.85, u: 0.30 },
    }
}

// ─── Online m/u census (iss/58 residual #2) ─────────────────────────────────

struct MuSlotCensus {
    /// total observations of this slot
    n: u64,
    /// digest → count (capped keys)
    freq: HashMap<String, u64>,
    /// same-device reobservation: agree / total
    same_agree: u64,
    same_total: u64,
    /// last digest per device (for m learning)
    last_by_device: HashMap<String, String>,
}

impl MuSlotCensus {
    fn new() -> Self {
        Self {
            n: 0,
            freq: HashMap::new(),
            same_agree: 0,
            same_total: 0,
            last_by_device: HashMap::new(),
        }
    }
}

struct MuCensusState {
    slots: HashMap<String, MuSlotCensus>,
    last_flush_ms: u64,
}

impl MuCensusState {
    fn new() -> Self {
        Self {
            slots: HashMap::new(),
            last_flush_ms: 0,
        }
    }
}

static MU_CENSUS: Mutex<Option<MuCensusState>> = Mutex::new(None);

fn with_mu<R>(f: impl FnOnce(&mut MuCensusState) -> R) -> R {
    let mut g = MU_CENSUS.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(MuCensusState::new());
        if let Some(ref mut st) = *g {
            load_mu_from_shared(st);
        }
    }
    f(g.as_mut().unwrap())
}

pub fn reset_mu_census_for_tests() {
    let mut g = MU_CENSUS.lock().unwrap_or_else(|e| e.into_inner());
    *g = Some(MuCensusState::new());
    use crate::shared_governance::{as_object_mut, with_shared_json};
    if crate::shared_governance::shared_governance_dir().is_some() {
        let _ = with_shared_json("mu_census", json!({"slots":{}}), |v| {
            as_object_mut(v).insert("slots".into(), json!({}));
        });
    }
}

/// Options for census observe (iss/59 P1-2 learning guardrails).
#[derive(Clone, Copy, Debug)]
pub struct MuObserveOpts {
    /// Only learn m (same-device reobs) when true.
    pub allow_m_learn: bool,
    /// Always learn population u frequency.
    pub allow_u_learn: bool,
}

impl Default for MuObserveOpts {
    fn default() -> Self {
        Self {
            allow_m_learn: true,
            allow_u_learn: true,
        }
    }
}

/// Build opts from governance posture (stable + conf only for m).
pub fn mu_opts_from_governance(fields: &Value, mint_posture: &str, conf: f64, heat_level: u8) -> MuObserveOpts {
    let stable = mint_posture == "stable";
    let conf_ok = conf >= 0.55;
    // Hot buckets: pause m learning (error-merge risk); still learn u
    let allow_m = stable && conf_ok && heat_level < 2;
    // Allow field override for explicit tests
    let force = fields
        .get("mu_allow_m_learn")
        .and_then(|v| v.as_bool());
    MuObserveOpts {
        allow_m_learn: force.unwrap_or(allow_m),
        allow_u_learn: true,
    }
}

/// Observe slot digests for online m/u learning (call on catalog_register / mint).
///
/// **Guardrails (iss/59)**: m learning only when `opts.allow_m_learn` (stable + conf,
/// not hot). u (population freq) always updates when `allow_u_learn`. Never use
/// ephemeral / commercial-blocked ids for m (caller must set opts).
pub fn observe_mu_census(device_id: &str, fields: &Value) {
    observe_mu_census_ex(device_id, fields, MuObserveOpts::default());
}

pub fn observe_mu_census_ex(device_id: &str, fields: &Value, opts: MuObserveOpts) {
    if device_id.is_empty() {
        return;
    }
    // Never train m on ephemeral prefixes
    let ephemeral_id = device_id.starts_with("dve-");
    let allow_m = opts.allow_m_learn && !ephemeral_id;
    let slot_names = ["res", "wg", "au", "cp", "of", "ar", "cc", "tz", "oi", "rtc"];
    let digests: Vec<(String, String)> = slot_names
        .iter()
        .enumerate()
        .filter_map(|(idx, name)| {
            let d = slot_digest_from_device_id(device_id, idx)
                .filter(|s| !s.is_empty() && s != "0")
                .or_else(|| {
                    let key = match *name {
                        "wg" => fields
                            .get("wg_whiten_lsh")
                            .or_else(|| fields.get("curve_lsh"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.chars().take(10).collect::<String>()),
                        "au" => fields
                            .get("au_whiten_lsh")
                            .and_then(|v| v.as_str())
                            .map(|s| s.chars().take(10).collect::<String>()),
                        "oi" => fields
                            .get("os_instance_hash")
                            .and_then(|v| v.as_str())
                            .map(|s| s.chars().take(10).collect::<String>()),
                        "rtc" => fields
                            .get("webrtc_host_ip_hash")
                            .and_then(|v| v.as_str())
                            .map(|s| s.chars().take(10).collect::<String>()),
                        _ => None,
                    };
                    key
                })?;
            Some(((*name).to_string(), d))
        })
        .collect();
    with_mu(|st| {
        for (name, dig) in digests {
            let slot = st.slots.entry(name).or_insert_with(MuSlotCensus::new);
            if opts.allow_u_learn {
                slot.n = slot.n.saturating_add(1);
                *slot.freq.entry(dig.clone()).or_insert(0) += 1;
                if slot.freq.len() > 512 {
                    if let Some(k) = slot
                        .freq
                        .iter()
                        .min_by_key(|(_, c)| *c)
                        .map(|(k, _)| k.clone())
                    {
                        slot.freq.remove(&k);
                    }
                }
            }
            // m learning only under guardrails
            if allow_m {
                let dev_key = device_id.to_string();
                if let Some(prev) = slot.last_by_device.get(&dev_key) {
                    slot.same_total = slot.same_total.saturating_add(1);
                    if prev == &dig {
                        slot.same_agree = slot.same_agree.saturating_add(1);
                    }
                }
                slot.last_by_device.insert(dev_key, dig);
                if slot.last_by_device.len() > 2048 {
                    let keys: Vec<String> = slot.last_by_device.keys().take(1024).cloned().collect();
                    for k in keys {
                        slot.last_by_device.remove(&k);
                    }
                }
            }
        }
        flush_mu_to_shared(st);
    });
}

/// Record that two different device_ids agreed on a slot (u evidence from FS pair).
pub fn observe_mu_pair_agree(slot: &str, agree: bool) {
    // u evidence: when different machines compared, agree rate → u
    // We only get this from pair_match_score path when ids differ.
    with_mu(|st| {
        let slot_c = st.slots.entry(slot.to_string()).or_insert_with(MuSlotCensus::new);
        // Encode in freq special keys would pollute — use same_total for m only;
        // store cross-device as negative space via freq mass: already have population freq.
        // Extra: track cross_agree via a reserved pseudo device.
        let key = "__cross__".to_string();
        if agree {
            *slot_c.freq.entry(format!("__agree_cross__")).or_insert(0) += 1;
        }
        *slot_c.freq.entry(key).or_insert(0) += 1;
    });
}

fn load_mu_from_shared(st: &mut MuCensusState) {
    use crate::shared_governance::with_shared_json;
    if crate::shared_governance::shared_governance_dir().is_none() {
        return;
    }
    let pulled = with_shared_json("mu_census", json!({"slots":{}}), |v| {
        v.get("slots")
            .and_then(|s| s.as_object())
            .cloned()
            .unwrap_or_default()
    });
    let Some(slots) = pulled else {
        return;
    };
    for (name, sv) in slots {
        let slot = st.slots.entry(name).or_insert_with(MuSlotCensus::new);
        slot.n = slot.n.max(sv.get("n").and_then(|x| x.as_u64()).unwrap_or(0));
        slot.same_agree = slot
            .same_agree
            .max(sv.get("same_agree").and_then(|x| x.as_u64()).unwrap_or(0));
        slot.same_total = slot
            .same_total
            .max(sv.get("same_total").and_then(|x| x.as_u64()).unwrap_or(0));
        if let Some(freq) = sv.get("freq").and_then(|f| f.as_object()) {
            for (d, c) in freq {
                let cv = c.as_u64().unwrap_or(0);
                let e = slot.freq.entry(d.clone()).or_insert(0);
                *e = (*e).max(cv);
            }
        }
    }
}

fn flush_mu_to_shared(st: &mut MuCensusState) {
    use crate::shared_governance::{as_object_mut, with_shared_json};
    if crate::shared_governance::shared_governance_dir().is_none() {
        return;
    }
    let t = now_ms();
    if st.last_flush_ms > 0 && t.saturating_sub(st.last_flush_ms) < 500 {
        return;
    }
    let snap: Vec<(String, u64, u64, u64, HashMap<String, u64>)> = st
        .slots
        .iter()
        .map(|(n, s)| {
            (n.clone(), s.n, s.same_agree, s.same_total, s.freq.clone())
        })
        .collect();
    let _ = with_shared_json("mu_census", json!({"slots":{}}), |v| {
        let o = as_object_mut(v);
        o.insert("algo".into(), json!("mu_census_online_v1"));
        let slots = o
            .entry("slots")
            .or_insert(json!({}))
            .as_object_mut()
            .unwrap();
        for (name, n, sa, stot, freq) in snap {
            let mut freq_j = Map::new();
            for (d, c) in freq {
                // skip internal counters in export size if huge — keep top 128
                freq_j.insert(d, json!(c));
            }
            // trim
            if freq_j.len() > 128 {
                let mut pairs: Vec<_> = freq_j.iter().map(|(k, v)| (k.clone(), v.as_u64().unwrap_or(0))).collect();
                pairs.sort_by(|a, b| b.1.cmp(&a.1));
                freq_j = pairs.into_iter().take(128).map(|(k, c)| (k, json!(c))).collect();
            }
            slots.insert(
                name,
                json!({
                    "n": n,
                    "same_agree": sa,
                    "same_total": stot,
                    "freq": freq_j,
                }),
            );
        }
    });
    st.last_flush_ms = t;
}

/// Learned m/u for a slot given optional digest (for u from frequency).
fn census_slot_mu_from(st: &MuCensusState, name: &str, digest: Option<&str>) -> Option<SlotMu> {
    let slot = st.slots.get(name)?;
    let prior = default_slot_mu(name);
    // m from same-device reobs (Bayesian shrink to prior)
    let m = if slot.same_total >= 5 {
        let raw = slot.same_agree as f64 / slot.same_total as f64;
        let w = (slot.same_total as f64 / (slot.same_total as f64 + 20.0)).min(0.85);
        prior.m * (1.0 - w) + raw * w
    } else {
        prior.m
    };
    // u from population frequency of this digest, or average collision rate
    let u = if let Some(d) = digest {
        if slot.n >= 10 {
            let c = slot.freq.get(d).copied().unwrap_or(0) as f64;
            // P(random other obs has same digest) ≈ (c-1)/(n-1) when c>=1
            let raw = if slot.n > 1 && c >= 1.0 {
                ((c - 1.0) / (slot.n as f64 - 1.0)).clamp(0.01, 0.95)
            } else {
                prior.u
            };
            let w = (slot.n as f64 / (slot.n as f64 + 50.0)).min(0.8);
            prior.u * (1.0 - w) + raw * w
        } else {
            prior.u
        }
    } else if slot.n >= 20 {
        // Average top-digest mass as class collision proxy
        let top = slot.freq.values().copied().max().unwrap_or(0) as f64;
        let raw = (top / slot.n as f64).clamp(0.01, 0.95);
        let w = 0.5;
        prior.u * (1.0 - w) + raw * w
    } else {
        prior.u
    };
    // Invariant: m > u so agreement → +weight, mismatch → −weight
    let m = m.clamp(0.55, 0.99);
    let u = u.clamp(0.01, 0.95).min(m - 0.05).max(0.01);
    Some(SlotMu { m, u })
}

fn census_slot_mu(name: &str, digest: Option<&str>) -> Option<SlotMu> {
    with_mu(|st| census_slot_mu_from(st, name, digest))
}

/// Snapshot for ops / product surface.
pub fn mu_census_snapshot() -> Value {
    with_mu(|st| {
        let mut slots = Map::new();
        for (name, s) in &st.slots {
            let top: Vec<Value> = {
                let mut pairs: Vec<_> = s.freq.iter().filter(|(k, _)| !k.starts_with("__")).collect();
                pairs.sort_by(|a, b| b.1.cmp(a.1));
                pairs
                    .into_iter()
                    .take(5)
                    .map(|(d, c)| json!({"digest": d, "n": c}))
                    .collect()
            };
            let learned = census_slot_mu_from(st, name, None);
            slots.insert(
                name.clone(),
                json!({
                    "n": s.n,
                    "same_agree": s.same_agree,
                    "same_total": s.same_total,
                    "unique_digests": s.freq.len(),
                    "top": top,
                    "m_hat": learned.map(|m| (m.m * 1000.0).round() / 1000.0),
                    "u_hat": learned.map(|m| (m.u * 1000.0).round() / 1000.0),
                }),
            );
        }
        json!({
            "algo": "mu_census_online_v1",
            "slots": slots,
            "shared": crate::shared_governance::shared_state_paths(),
        })
    })
}

/// Adjust u upward when effective_bits collapse (class-dead channel).
/// Blends **online census** m/u with priors when census has signal.
fn slot_mu_adjusted(name: &str, eff: Option<&Value>) -> SlotMu {
    let mut mu = census_slot_mu(name, None).unwrap_or_else(|| default_slot_mu(name));
    if let Some(e) = eff {
        if let Some(row) = e.pointer(&format!("/slots/{name}")) {
            let eb = row.get("effective_bits").and_then(|v| v.as_f64()).unwrap_or(8.0);
            let collapsed = row.get("collapsed").and_then(|v| v.as_bool()).unwrap_or(false);
            if collapsed || eb < 2.0 {
                // Dead channel: agreement almost uninformative
                mu.u = (mu.u + 0.45).min(0.95);
                mu.m = (mu.m - 0.05).max(0.55);
            } else if eb > 8.0 {
                mu.u = (mu.u * 0.7).max(0.02);
            }
        }
    }
    mu
}

/// Like slot_mu_adjusted but uses pair digests for frequency-based u.
///
/// **Direction asymmetry (iss/59 P0-2)**: census may only make agreement *less*
/// merge-promoting than the prior (raise u / lower m). It must never alone
/// increase m/u ratio above the prior (which would push wrong-merge).
fn slot_mu_for_pair(name: &str, dig_a: &str, dig_b: &str, eff: Option<&Value>) -> SlotMu {
    let prior = default_slot_mu(name);
    let dig = if dig_a == dig_b { Some(dig_a) } else { None };
    let mut mu = census_slot_mu(name, dig).unwrap_or(prior);
    if let Some(e) = eff {
        if let Some(row) = e.pointer(&format!("/slots/{name}")) {
            let eb = row.get("effective_bits").and_then(|v| v.as_f64()).unwrap_or(8.0);
            let collapsed = row.get("collapsed").and_then(|v| v.as_bool()).unwrap_or(false);
            if collapsed || eb < 2.0 {
                mu.u = (mu.u + 0.45).min(0.95);
                mu.m = (mu.m - 0.05).max(0.55);
            } else if eb > 8.0 {
                mu.u = (mu.u * 0.7).max(0.02);
            }
        }
    }
    // Cap merge-promoting strength at prior: m ≤ prior.m, u ≥ prior.u
    // (census can only push toward split/abstain on agreement)
    if mu.m > prior.m {
        mu.m = prior.m;
    }
    if mu.u < prior.u {
        mu.u = prior.u;
    }
    // Maintain m > u
    if mu.u >= mu.m {
        mu.u = (mu.m - 0.05).max(0.01);
    }
    mu
}

/// Prior-only m/u (no census) for asymmetry check.
fn slot_mu_prior_only(name: &str, eff: Option<&Value>) -> SlotMu {
    let mut mu = default_slot_mu(name);
    if let Some(e) = eff {
        if let Some(row) = e.pointer(&format!("/slots/{name}")) {
            let eb = row.get("effective_bits").and_then(|v| v.as_f64()).unwrap_or(8.0);
            let collapsed = row.get("collapsed").and_then(|v| v.as_bool()).unwrap_or(false);
            if collapsed || eb < 2.0 {
                mu.u = (mu.u + 0.45).min(0.95);
                mu.m = (mu.m - 0.05).max(0.55);
            } else if eb > 8.0 {
                mu.u = (mu.u * 0.7).max(0.02);
            }
        }
    }
    if mu.u >= mu.m {
        mu.u = (mu.m - 0.05).max(0.01);
    }
    mu
}

fn log2_weight(m: f64, u: f64, agree: bool) -> f64 {
    let m = m.clamp(0.01, 0.99);
    let u = u.clamp(0.01, 0.99);
    if agree {
        (m / u).log2()
    } else {
        // mismatch weight
        ((1.0 - m) / (1.0 - u)).log2()
    }
}

/// Pairwise Fellegi–Sunter score (log2 likelihood ratio sum).
pub fn pair_match_score(fields_a: &Value, id_a: &str, fields_b: &Value, id_b: &str) -> Value {
    let posture = MergePosture::from_env_or_default();
    let eff_a = slot_effective_bits(fields_a);
    let eff_b = slot_effective_bits(fields_b);
    let mut score = 0.0;
    let mut score_prior = 0.0;
    let mut parts = Vec::new();
    let slot_names: &[(usize, &str)] = &[
        (0, "res"),
        (1, "wg"),
        (2, "au"),
        (3, "cp"),
        (4, "of"),
        (5, "ar"),
        (6, "cc"),
        (7, "tz"),
        (8, "oi"),
        (9, "rtc"),
    ];
    let mut host_sep_mismatch = false;
    for (idx, name) in slot_names {
        let da = slot_digest_from_device_id(id_a, *idx).unwrap_or_default();
        let db = slot_digest_from_device_id(id_b, *idx).unwrap_or_default();
        if da.is_empty() || db.is_empty() || da == "0" || db == "0" {
            parts.push(json!({"slot": name, "level": "missing", "w": 0.0}));
            continue;
        }
        if (*name == "oi" || *name == "rtc") && da != db {
            host_sep_mismatch = true;
        }
        let mu_a = slot_mu_for_pair(name, &da, &db, Some(&eff_a));
        let mu_b = slot_mu_for_pair(name, &da, &db, Some(&eff_b));
        let m = (mu_a.m + mu_b.m) / 2.0;
        let u = (mu_a.u + mu_b.u) / 2.0;
        let prior_a = slot_mu_prior_only(name, Some(&eff_a));
        let prior_b = slot_mu_prior_only(name, Some(&eff_b));
        let pm = (prior_a.m + prior_b.m) / 2.0;
        let pu = (prior_a.u + prior_b.u) / 2.0;
        if da == db {
            let w = log2_weight(m, u, true);
            let wp = log2_weight(pm, pu, true);
            score += w;
            score_prior += wp;
            parts.push(json!({
                "slot": name, "level": "exact", "w": (w * 1000.0).round() / 1000.0,
                "m": m, "u": u, "mu_source": "census_blend_asym",
            }));
        } else {
            let partial = da.len() >= 4 && db.len() >= 4 && &da[..4] == &db[..4];
            if partial {
                let w = log2_weight(m, u, true) * 0.35;
                let wp = log2_weight(pm, pu, true) * 0.35;
                score += w;
                score_prior += wp;
                parts.push(json!({
                    "slot": name, "level": "structure_match", "w": (w * 1000.0).round() / 1000.0,
                    "m": m, "u": u,
                }));
            } else {
                let w = log2_weight(m, u, false);
                let wp = log2_weight(pm, pu, false);
                score += w;
                score_prior += wp;
                parts.push(json!({
                    "slot": name, "level": "mismatch", "w": (w * 1000.0).round() / 1000.0,
                    "m": m, "u": u,
                }));
            }
        }
    }
    // Curve LSH soft agreement (does not use census)
    let lsh_a = fields_a
        .get("curve_lsh")
        .or_else(|| fields_a.get("wg_whiten_lsh"))
        .or_else(|| fields_a.pointer("/curve_descriptors/webgl/lsh"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let lsh_b = fields_b
        .get("curve_lsh")
        .or_else(|| fields_b.get("wg_whiten_lsh"))
        .or_else(|| fields_b.pointer("/curve_descriptors/webgl/lsh"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !lsh_a.is_empty() && !lsh_b.is_empty() {
        let d = crate::cluster_lsh::hex_digest_distance(lsh_a, lsh_b);
        if d <= 4 {
            let w = log2_weight(0.9, 0.15, true);
            score += w;
            score_prior += w;
            parts.push(json!({"slot": "curve_lsh", "level": "lsh_close", "dist": d, "w": w}));
        } else if d <= 12 {
            let w = log2_weight(0.9, 0.15, true) * 0.4;
            score += w;
            score_prior += w;
            parts.push(json!({"slot": "curve_lsh", "level": "lsh_mid", "dist": d, "w": w}));
        } else if d >= 20 {
            let w = log2_weight(0.9, 0.15, false);
            score += w;
            score_prior += w;
            parts.push(json!({"slot": "curve_lsh", "level": "lsh_far", "dist": d, "w": w}));
        }
    }
    if !id_a.is_empty() && id_a == id_b {
        score += 20.0;
        score_prior += 20.0;
        parts.push(json!({"slot": "full_id", "level": "exact", "w": 20.0}));
    }

    // Hard veto: host seps disagree → never merge (physical machine evidence)
    let mut decision = if score >= posture.tau_merge() {
        "merge"
    } else if score <= posture.tau_split() {
        "split"
    } else {
        "abstain"
    };
    let mut asymmetry_forced = false;
    if host_sep_mismatch && decision == "merge" && id_a != id_b {
        decision = "split";
        asymmetry_forced = true;
        parts.push(json!({
            "slot": "host_sep_veto",
            "level": "hard_split",
            "w": 0.0,
            "note": "oi/rtc mismatch forbids merge",
        }));
    }
    // Census cannot alone promote prior-abstain/split into merge
    if decision == "merge"
        && score_prior < posture.tau_merge()
        && id_a != id_b
        && !host_sep_mismatch
    {
        decision = if score_prior <= posture.tau_split() {
            "split"
        } else {
            "abstain"
        };
        asymmetry_forced = true;
        parts.push(json!({
            "slot": "census_asymmetry",
            "level": "block_merge_promotion",
            "score_prior": (score_prior * 1000.0).round() / 1000.0,
            "score_census": (score * 1000.0).round() / 1000.0,
            "note": "census may not alone elevate to merge",
        }));
    }

    let margin = score - posture.tau_merge();
    let fpp = 1.0 / (1.0 + margin.exp());
    let conf = (1.0 - fpp).clamp(0.05, 0.99);
    json!({
        "algo": "identity_resolver_fs_v2",
        "score": (score * 1000.0).round() / 1000.0,
        "score_prior": (score_prior * 1000.0).round() / 1000.0,
        "decision": decision,
        "tau_merge": posture.tau_merge(),
        "tau_split": posture.tau_split(),
        "merge_posture": posture.as_str(),
        "parts": parts,
        "host_sep_mismatch": host_sep_mismatch,
        "asymmetry_forced": asymmetry_forced,
        "fpp_est": (fpp * 1000.0).round() / 1000.0,
        "confidence": (conf * 1000.0).round() / 1000.0,
    })
}

// ─── B1 candidate LSH index + B6 consensus ─────────────────────────────────

struct DeviceCatalog {
    /// body_key → device_ids that minted that body
    by_body: HashMap<String, Vec<String>>,
    /// device_id → (fields, id, last_ms, agree_n, ephemeral)
    records: HashMap<String, DeviceRecord>,
    /// lsh prefix (8 hex) → device_ids
    by_lsh: HashMap<String, Vec<String>>,
}

#[derive(Clone)]
struct DeviceRecord {
    fields: Value,
    device_id: String,
    last_ms: u64,
    agree_n: u32,
    ephemeral: bool,
    body_key: String,
}

impl DeviceCatalog {
    fn new() -> Self {
        Self {
            by_body: HashMap::new(),
            records: HashMap::new(),
            by_lsh: HashMap::new(),
        }
    }
}

static DEVICE_CATALOG: Mutex<Option<DeviceCatalog>> = Mutex::new(None);

fn with_catalog<R>(f: impl FnOnce(&mut DeviceCatalog) -> R) -> R {
    let mut g = DEVICE_CATALOG.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(DeviceCatalog::new());
    }
    f(g.as_mut().unwrap())
}

pub fn reset_device_catalog_for_tests() {
    with_catalog(|c| *c = DeviceCatalog::new());
}

fn lsh_prefix(fields: &Value) -> String {
    fields
        .get("wg_whiten_lsh")
        .or_else(|| fields.get("curve_lsh"))
        .or_else(|| fields.pointer("/curve_descriptors/webgl/lsh"))
        .and_then(|v| v.as_str())
        .map(|s| s.chars().take(8).collect())
        .unwrap_or_default()
}

/// Register a device observation into process catalog (for B1 recall + B6).
pub fn catalog_register(device_id: &str, fields: &Value, body_key: &str, ephemeral: bool) {
    if device_id.is_empty() {
        return;
    }
    let t = now_ms();
    let lsh = lsh_prefix(fields);
    with_catalog(|c| {
        let rec = c.records.entry(device_id.to_string()).or_insert(DeviceRecord {
            fields: fields.clone(),
            device_id: device_id.to_string(),
            last_ms: t,
            agree_n: 0,
            ephemeral,
            body_key: body_key.to_string(),
        });
        rec.fields = fields.clone();
        rec.last_ms = t;
        rec.agree_n = rec.agree_n.saturating_add(1);
        rec.ephemeral = ephemeral;
        rec.body_key = body_key.to_string();
        if !body_key.is_empty() {
            let v = c.by_body.entry(body_key.to_string()).or_default();
            if !v.iter().any(|x| x == device_id) {
                v.push(device_id.to_string());
            }
        }
        if !lsh.is_empty() {
            let v = c.by_lsh.entry(lsh).or_default();
            if !v.iter().any(|x| x == device_id) {
                v.push(device_id.to_string());
            }
        }
    });
    // Online m/u census with guardrails + weak-sup contrastive + multi-layer HNSW
    let conf = fields
        .get("identity_confidence")
        .or_else(|| fields.pointer("/identity/confidence"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.5);
    let posture = fields
        .get("mint_posture")
        .and_then(|v| v.as_str())
        .unwrap_or(if ephemeral { "ephemeral_cohort" } else { "stable" });
    let heat_lv = fields
        .pointer("/bucket_heat/heat_level")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u8;
    let opts = mu_opts_from_governance(fields, posture, conf, heat_lv);
    observe_mu_census_ex(device_id, fields, opts);
    crate::contrastive_sup::observe_contrastive_online(device_id, fields);
    crate::hnsw_lite::hnsw_insert(device_id, fields);
}

/// B1: recall candidates by body heat + LSH prefix, score with FS, return best decision.
pub fn resolve_identity_candidates(
    fields: &Value,
    minted_id: &str,
    body_key: &str,
    binder_candidates: &[String],
    candidate_fields: &HashMap<String, Value>,
) -> Value {
    let mut cand_ids: Vec<String> = binder_candidates.to_vec();
    // Expand from catalog LSH + body
    with_catalog(|c| {
        if !body_key.is_empty() {
            if let Some(ids) = c.by_body.get(body_key) {
                for id in ids {
                    if !cand_ids.contains(id) {
                        cand_ids.push(id.clone());
                    }
                }
            }
        }
        let lsh = lsh_prefix(fields);
        if !lsh.is_empty() {
            if let Some(ids) = c.by_lsh.get(&lsh) {
                for id in ids {
                    if !cand_ids.contains(id) {
                        cand_ids.push(id.clone());
                    }
                }
            }
        }
        // Also pull fields from catalog for candidates missing in map
        let _ = c;
    });
    // B3 HNSW-lite ANN recall (contrastive embedding)
    let ann_hits = crate::hnsw_lite::hnsw_search(fields, 16);
    for (id, _dist) in &ann_hits {
        if !cand_ids.contains(id) {
            cand_ids.push(id.clone());
        }
    }

    let mut best_id: Option<String> = None;
    let mut best_score = f64::NEG_INFINITY;
    let mut best_decision = "split".to_string();
    let mut best_meta = Value::Null;
    let mut scored = Vec::new();

    for did in &cand_ids {
        let ex_fields = candidate_fields.get(did).cloned().or_else(|| {
            with_catalog(|c| c.records.get(did).map(|r| r.fields.clone()))
        });
        let Some(ex_f) = ex_fields else { continue };
        let meta = pair_match_score(&ex_f, did, fields, minted_id);
        let sc = meta.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let dec = meta
            .get("decision")
            .and_then(|v| v.as_str())
            .unwrap_or("split")
            .to_string();
        scored.push(json!({"device_id": did, "score": sc, "decision": dec}));
        if sc > best_score {
            best_score = sc;
            best_id = Some(did.clone());
            best_decision = dec;
            best_meta = meta;
        }
    }

    // B6: if best is merge to ephemeral that has enough agrees → promote note
    let mut promote = false;
    if best_decision == "merge" {
        if let Some(ref id) = best_id {
            with_catalog(|c| {
                if let Some(r) = c.records.get_mut(id) {
                    r.agree_n = r.agree_n.saturating_add(1);
                    // N consistent observations → promote ephemeral
                    if r.ephemeral && r.agree_n >= 3 {
                        r.ephemeral = false;
                        promote = true;
                    }
                    // Time decay: if last_ms too old, decay agree
                    let age = now_ms().saturating_sub(r.last_ms);
                    if age > 86_400_000 {
                        r.agree_n = r.agree_n.saturating_sub(1);
                    }
                    r.last_ms = now_ms();
                }
            });
        }
    }

    let posture = MergePosture::from_env_or_default();
    let final_decision = if best_id.is_none() {
        "mint".to_string()
    } else {
        best_decision.clone()
    };

    json!({
        "algo": "identity_resolver_hotpath_v1",
        "decision": final_decision,
        "best_device_id": best_id,
        "best_score": if best_score.is_finite() { json!((best_score * 1000.0).round() / 1000.0) } else { Value::Null },
        "candidates_n": cand_ids.len(),
        "scored": scored,
        "fs": best_meta,
        "b6_promote_ephemeral": promote,
        "merge_posture": posture.as_str(),
        "tau_merge": posture.tau_merge(),
        "tau_split": posture.tau_split(),
        "ann_recall_n": ann_hits.len(),
        "ann_algo": crate::hnsw_lite::HNSW_LITE_ALGO,
        "mu_census": "online_v1",
    })
}

/// B6: half-life decay weight for observations (hours).
pub fn consensus_decay_weight(age_ms: u64) -> f64 {
    let half_life_ms = 12.0 * 3600.0 * 1000.0; // 12h
    let age = age_ms as f64;
    0.5_f64.powf(age / half_life_ms)
}

fn short_hash(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    format!("{:x}", h.finalize())[..12].to_string()
}

/// Build ephemeral cohort id (not stable commercial).
pub fn ephemeral_cohort_id(body_key: &str, session_id: &str) -> String {
    let dig = short_hash(&format!("eph|{body_key}|{session_id}"));
    format!("dve-{dig}")
}

/// Apply B4/B7/B1 governance to device + confidence after segments mint.
///
/// Returns patch object merged into device / product by evaluate.
pub fn apply_identity_governance(
    fields: &Value,
    device_id: &str,
    segments: &Value,
    session_id: &str,
    base_confidence: f64,
    collision_risk_in: bool,
) -> Value {
    let posture = MergePosture::from_env_or_default();
    let eff = slot_effective_bits(fields);
    let body_key = body_heat_key(device_id, Some(segments));
    let (heat_n, heat_level) =
        observe_bucket_heat_ex(&body_key, session_id, device_id, Some(fields));

    let total_eff = eff
        .get("total_effective_bits")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let collapsed = eff
        .get("collapsed_slots")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    // Self-resolver readiness (no candidate): use entropy collapse + heat as abstain.
    let mut mint_posture = "stable";
    let mut ephemeral = false;
    let mut publish_id = device_id.to_string();
    let mut collision_risk = collision_risk_in;
    let mut force_deepen = false;
    let mut deepen_packs: Vec<&str> = Vec::new();
    let mut commercial_blocked = false;

    if device_id.is_empty() {
        mint_posture = "empty_anchor";
        commercial_blocked = true;
    } else if heat_level >= 2 || (heat_level >= 1 && total_eff < 12.0) {
        // Hot bucket: merge_averse → deepen or ephemeral
        collision_risk = true;
        force_deepen = true;
        deepen_packs.extend_from_slice(&[
            "B10x_silicon_deep",
            "B10x_silicon_ulp",
            "B46_audio_deep",
            "B47_sab_clock",
            "B18_webgpu",
            "B81_webcodecs_bitstream",
            "B82_idb_write_ladder",
            "B83_eventloop_signature",
            // iss/74: cross-check surfaces — os/engine/wasm/audio/emoji/storage
            "B86_os_gecko_surface",
            "B87_engine_behavior_diff",
            "B88_wasm_instruction_throughput",
            "B89_audio_known_lock",
            "B90_os_emoji_raster",
            "B91_storage_disk_quota",
            // iss/74 Phase 2: research packs (GPU contention / SAB dual clock /
            // EU timing) + static blink fork matrix join deepen under heat
            "B92_webgpu_atomic_contention",
            "B93_blink_fork_matrix",
            "B94_sab_dual_clock_differential",
            "B95_gpu_eu_timing",
        ]);
        if heat_level >= 2 || collapsed >= 3 {
            mint_posture = "ephemeral_cohort";
            ephemeral = true;
            publish_id = ephemeral_cohort_id(&body_key, session_id);
            commercial_blocked = true; // not stable commercial
        } else {
            mint_posture = "deepen_required";
            // merge_averse: withhold stable id until deepen lands
            if posture == MergePosture::MergeAverse {
                publish_id.clear();
                commercial_blocked = true;
            }
        }
    } else if total_eff < 8.0 || collapsed >= 4 {
        mint_posture = "deepen_required";
        force_deepen = true;
        collision_risk = true;
        deepen_packs.extend_from_slice(&["B10x_silicon_deep", "B46_audio_deep"]);
        if posture == MergePosture::MergeAverse && total_eff < 5.0 {
            publish_id.clear();
            commercial_blocked = true;
        }
    }

    // Confidence: blend base with heat/entropy FPP proxy
    let heat_penalty = match heat_level {
        2 => 0.35,
        1 => 0.18,
        _ => 0.0,
    };
    let entropy_boost = (total_eff / 40.0).clamp(0.0, 0.25);
    let mut conf = (base_confidence * (1.0 - heat_penalty) + entropy_boost).clamp(0.05, 0.99);
    if ephemeral || commercial_blocked {
        conf = conf.min(0.55);
    }
    if mint_posture == "stable" && !collision_risk {
        conf = conf.max(0.55);
    }
    let fpp_est = (1.0 - conf).clamp(0.01, 0.95);

    json!({
        "algo": IDENTITY_GOVERNANCE_ALGO,
        "merge_posture": posture.as_str(),
        "mint_posture": mint_posture,
        "ephemeral": ephemeral,
        "ephemeral_ttl_ms": if ephemeral { EPHEMERAL_TTL_MS } else { 0 },
        "device_id_governed": if publish_id.is_empty() { Value::Null } else { json!(publish_id) },
        "stable_device_id_candidate": if device_id.is_empty() { Value::Null } else { json!(device_id) },
        "commercial_blocked": commercial_blocked,
        "collision_risk": collision_risk,
        "force_deepen": force_deepen,
        "deepen_packs": deepen_packs,
        "bucket_heat": {
            "body_key_hash": short_hash(&body_key),
            "distinct_machines": heat_n,
            "distinct_sessions": heat_n, // legacy alias; unit is machine evidence
            "heat_unit": "machine_oi_rtc",
            "heat_level": heat_level,
            "t1": HOT_BUCKET_T1,
            "t2": HOT_BUCKET_T2,
            "window_ms": HOT_WINDOW_MS,
        },
        "effective_bits": eff,
        "identity": {
            "confidence": (conf * 1000.0).round() / 1000.0,
            "fpp_est": (fpp_est * 1000.0).round() / 1000.0,
            "confidence_kind": "calibrated_estimate_not_uv_sla",
            "sdk_note": "confidence is 1-FPP estimate for ops/SDK; not a uniqueness guarantee",
        },
        "sdk": {
            "ephemeral_means": "class_bucket_assist_not_account_bind",
            "stable_means": "commercial_device_id_when_mint_posture_stable",
            "default_posture": "merge_averse_prefer_split",
        },
    })
}

/// Inject deepen packs into a route_plan-like array.
///
/// iss/65: do **not** stamp `force_recollect` on every inject when pack already listed —
/// multi-tick + hot-bucket was re-uploading B10x_deep 20–30× per session.
/// First inject may force once; subsequent ticks only schedule if missing from plan.
pub fn inject_deepen_packs(route_packs: &mut Vec<Value>, deepen: &[String]) {
    for id in deepen {
        let already = route_packs.iter().any(|p| {
            p.get("pack_id")
                .or_else(|| p.get("id"))
                .and_then(|v| v.as_str())
                == Some(id.as_str())
        });
        if already {
            // Keep existing entry; do not escalate force_recollect each analyze.
            continue;
        }
        route_packs.push(json!({
            "pack_id": id,
            "id": id,
            // First schedule only — FE short-circuits after sealed_ok.
            "force_recollect": false,
            "priority": 1500,
            "reason": "identity_governance_hot_or_low_entropy",
            "schedule": "dynamic",
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard};

    /// Per-test isolation: exclusive lock + thread-local shared dir + full reset.
    fn isolate(label: &str) -> (MutexGuard<'static, ()>, PathBuf) {
        let guard = crate::shared_governance::ISS58_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::set_var("GR_CONTRASTIVE_WARM_FLEET", "0");
        let dir = crate::shared_governance::test_isolation_dir(label);
        reset_hot_buckets_for_tests();
        reset_mu_census_for_tests();
        reset_device_catalog_for_tests();
        crate::hnsw_lite::reset_hnsw_for_tests();
        crate::contrastive_sup::reset_contrastive_for_tests();
        crate::population_atlas::reset_atlas_for_tests();
        (guard, dir)
    }

    fn cleanup(_guard: MutexGuard<'static, ()>, dir: PathBuf) {
        crate::shared_governance::clear_shared_governance_files();
        crate::shared_governance::set_shared_governance_dir_for_tests(None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn merge_posture_default_is_averse() {
        let p = MergePosture::MergeAverse;
        assert!(p.tau_merge() > p.tau_split());
    }

    #[test]
    fn hot_bucket_escalates_and_ephemeral() {
        let (g0, dir) = isolate("hot_esc");
        // Class body shared, **distinct machines** (oi/rtc) — heat unit is machine
        let class = "aaaaaaaaaa";
        for i in 0..HOT_BUCKET_T2 + 1 {
            let oi = format!("oi{:08x}", 0x1000 + i as u32);
            let rtc = format!("rt{:08x}", 0x2000 + i as u32);
            let did = format!(
                "dv0-{c}-{c}-{c}-{c}-{c}-{c}-{c}-{c}-{oi}-{rtc}",
                c = class,
                oi = oi,
                rtc = rtc
            );
            let fields = json!({
                "hw_curve_webgl": vec![0.26_f64; 32],
                "hw_curve_audio": vec![0.1_f64; 32],
                "os_instance_hash": oi,
                "webrtc_host_ip_hash": rtc,
            });
            let segs = json!({"dv0": did});
            let g = apply_identity_governance(
                &fields,
                &did,
                &segs,
                &format!("session_{i}"),
                0.8,
                false,
            );
            if i + 1 >= HOT_BUCKET_T2 {
                assert_eq!(g["mint_posture"], "ephemeral_cohort", "i={i} g={g}");
                assert_eq!(g["ephemeral"], true);
                assert!(g["device_id_governed"]
                    .as_str()
                    .unwrap_or("")
                    .starts_with("dve-"));
            }
        }
        cleanup(g0, dir);
    }

    /// iss/opus5 P0-2: same visitor_terminal_id across many sessions counts once.
    #[test]
    fn same_visitor_terminal_session_storm_does_not_heat() {
        let (g0, dir) = isolate("vt_storm");
        let body = "dv0:vtstorm";
        // Slots 8/9 = "0" (real PLACEHOLDER) → slot path skipped, VT path used.
        let did = "dv0-aaaaaaaaaa-bbbbbbbbbb-cccccccccc-dddddddddd-eeeeeeeeee-ffffffffff-gggggggggg-hhhhhhhhhh-0-0";
        for i in 0..40 {
            let f = json!({ "visitor_terminal_id": "vt_stable_terminal_1" });
            let (n, level) = observe_bucket_heat_ex(
                body,
                &format!("session_vtstorm_{i}"),
                did,
                Some(&f),
            );
            assert_eq!(n, 1, "same VT must count once, n={n} i={i}");
            assert_eq!(level, 0, "must stay cool");
        }
        cleanup(g0, dir);
    }

    /// iss/opus5 P0-2 (b): sessions with NO identity evidence collapse to one
    /// unknown unit per bucket — they must never accumulate heat.
    #[test]
    fn no_identity_sessions_collapse_to_single_unknown_unit() {
        let (g0, dir) = isolate("unknown_collapse");
        let body = "dv0:unknowncollapse";
        // Slots 8/9 = "0" (real PLACEHOLDER) → slot path skipped, session-weak
        // collapse is what keeps the bucket cool.
        let did = "dv0-aaaaaaaaaa-bbbbbbbbbb-cccccccccc-dddddddddd-eeeeeeeeee-ffffffffff-gggggggggg-hhhhhhhhhh-0-0";
        for i in 0..40 {
            let (n, level) = observe_bucket_heat_ex(
                body,
                &format!("session_noid_{i}"),
                did,
                None,
            );
            assert_eq!(n, 1, "no-evidence sessions must collapse, n={n} i={i}");
            assert_eq!(level, 0, "weak-only heat must stay cool");
        }
        cleanup(g0, dir);
    }

    /// Distinct visitor terminals (real farm) still accumulate heat.
    #[test]
    fn distinct_visitor_terminals_still_heat() {
        let (g0, dir) = isolate("vt_distinct");
        let body = "dv0:vtdistinct";
        // Slots 8/9 = "0" (the real PLACEHOLDER) so the slot path is skipped
        // and the VT path is what accumulates.
        let did = "dv0-aaaaaaaaaa-bbbbbbbbbb-cccccccccc-dddddddddd-eeeeeeeeee-ffffffffff-gggggggggg-hhhhhhhhhh-0-0";
        for i in 0..HOT_BUCKET_T2 {
            let f = json!({ "visitor_terminal_id": format!("vt_farm_{i}") });
            let (n, level) = observe_bucket_heat_ex(
                body,
                &format!("session_farm_{i}"),
                did,
                Some(&f),
            );
            if i + 1 >= HOT_BUCKET_T2 {
                assert!(n >= HOT_BUCKET_T2, "distinct VTs must accumulate, n={n}");
                assert!(level >= 2, "level={level}");
            }
        }
        cleanup(g0, dir);
    }

    /// Single machine many sessions must NOT heat the bucket (iss/59 P1-1).
    #[test]
    fn single_machine_session_storm_does_not_heat() {
        let (g0, dir) = isolate("storm");
        let body = "dv0:sharedbody";
        let fields = json!({
            "os_instance_hash": "oi_fixed_host",
            "webrtc_host_ip_hash": "rtc_fixed_host",
        });
        for i in 0..40 {
            let (n, level) = observe_bucket_heat_ex(
                body,
                &format!("session_storm_{i}"),
                "dv0-aaaaaaaaaa-bbbbbbbbbb-cccccccccc-dddddddddd-eeeeeeeeee-ffffffffff-gggggggggg-hhhhhhhhhh-oifixed001-rtcfixed01",
                Some(&fields),
            );
            assert_eq!(n, 1, "same machine must count once, n={n} i={i}");
            assert_eq!(level, 0, "must stay cool");
        }
        cleanup(g0, dir);
    }

    #[test]
    fn effective_bits_marks_collapsed_flat_curve() {
        let fields = json!({
            "hw_curve_webgl": vec![0.26_f64; 32],
            "hw_curve_audio": vec![0.1_f64; 32],
        });
        let e = slot_effective_bits(&fields);
        assert!(e["alert_collapse"].as_bool().unwrap_or(false));
        assert!(e["total_effective_bits"].as_f64().unwrap_or(99.0) < 20.0);
    }

    #[test]
    fn pair_score_merge_on_identical_ids() {
        let (g0, dir) = isolate("pair_merge");
        let id = "dv0-aaaaaaaaaa-bbbbbbbbbb-cccccccccc-dddddddddd-eeeeeeeeee-ffffffffff-gggggggggg-hhhhhhhhhh-iiiiiiiiii-jjjjjjjjjj";
        let f = json!({});
        let r = pair_match_score(&f, id, &f, id);
        assert_eq!(r["decision"], "merge");
        assert!(r["confidence"].as_f64().unwrap() > 0.5);
        cleanup(g0, dir);
    }

    #[test]
    fn pair_score_split_on_full_mismatch() {
        let (g0, dir) = isolate("pair_split");
        let a = "dv0-aaaaaaaaaa-bbbbbbbbbb-cccccccccc-dddddddddd-eeeeeeeeee-ffffffffff-gggggggggg-hhhhhhhhhh-iiiiiiiiii-jjjjjjjjjj";
        let b = "dv0-zzzzzzzzzz-yyyyyyyyyy-xxxxxxxxxx-wwwwwwwwww-vvvvvvvvvv-uuuuuuuuuu-tttttttttt-ssssssssss-rrrrrrrrrr-qqqqqqqqqq";
        let r = pair_match_score(&json!({}), a, &json!({}), b);
        assert_eq!(r["decision"], "split");
        assert_eq!(r["host_sep_mismatch"], true);
        cleanup(g0, dir);
    }

    /// Polluted census must not flip different machines to merge (iss/59 P0-2).
    #[test]
    fn polluted_census_cannot_force_merge() {
        let (g0, dir) = isolate("pollute");
        // Flood census with class-floor digests
        let class = "deadclass01";
        for i in 0..40 {
            let oi = format!("oi{:08x}", 0x9000 + i);
            let rtc = format!("rt{:08x}", 0xa000 + i);
            let id = format!(
                "dv0-{c}-{c}-{c}-{c}-{c}-{c}-{c}-{c}-{oi}-{rtc}",
                c = class,
                oi = oi,
                rtc = rtc
            );
            observe_mu_census_ex(
                &id,
                &json!({"hw_curve_webgl": vec![0.26_f64; 32], "mu_allow_m_learn": true}),
                MuObserveOpts {
                    allow_m_learn: true,
                    allow_u_learn: true,
                },
            );
        }
        let a = format!(
            "dv0-{c}-{c}-{c}-{c}-{c}-{c}-{c}-{c}-oi11111111-rt11111111",
            c = class
        );
        let b = format!(
            "dv0-{c}-{c}-{c}-{c}-{c}-{c}-{c}-{c}-oi22222222-rt22222222",
            c = class
        );
        let r = pair_match_score(&json!({}), &a, &json!({}), &b);
        assert_ne!(
            r["decision"], "merge",
            "polluted census must not merge different hosts: {r}"
        );
        assert!(
            r["host_sep_mismatch"].as_bool().unwrap_or(false)
                || r["decision"] == "split"
                || r["decision"] == "abstain",
            "{r}"
        );
        cleanup(g0, dir);
    }

    /// Synthetic same-SKU fleet: exact-hash wrong-merge rate vs FS resolver.
    #[test]
    fn fleet_fs_reduces_wrong_merge_vs_exact_hash() {
        let (g0, dir) = isolate("fleet");
        // Same res/wg/au/cc class floors (collision class), different oi/rtc host seps
        let class = "deadclass01"; // 10 hex
        let mut exact_collide = 0usize;
        let mut fs_merge_wrong = 0usize;
        let mut pairs = 0usize;
        let n = 12usize;
        let mut ids = Vec::new();
        let mut fields_list = Vec::new();
        for i in 0..n {
            let oi = format!("oi{:08x}", 0x1000u32 + i as u32);
            let rtc = format!("rt{:08x}", 0x2000u32 + i as u32 * 3);
            // identical silicon class slots 0-7, unique host seps 8-9
            let id = format!(
                "dv0-{c}-{c}-{c}-{c}-{c}-{c}-{c}-{c}-{oi}-{rtc}",
                c = class,
                oi = oi,
                rtc = rtc
            );
            let f = json!({
                "hw_curve_webgl": vec![0.26_f64; 32],
                "hw_curve_audio": vec![0.1_f64; 32],
                "os_instance_hash": oi,
                "webrtc_host_ip_hash": rtc,
                "wg_whiten_lsh": format!("{:016x}", (i as u64).wrapping_mul(0x9e3779b97f4a7c15)),
            });
            ids.push(id);
            fields_list.push(f);
        }
        // Pairwise: exact hash of first 8 slots would collide for all pairs
        for i in 0..n {
            for j in (i + 1)..n {
                pairs += 1;
                // exact class body (slots 0-7)
                let bi: String = ids[i].split('-').skip(1).take(8).collect::<Vec<_>>().join("-");
                let bj: String = ids[j].split('-').skip(1).take(8).collect::<Vec<_>>().join("-");
                if bi == bj {
                    exact_collide += 1;
                }
                let r = pair_match_score(&fields_list[i], &ids[i], &fields_list[j], &ids[j]);
                if r["decision"] == "merge" {
                    fs_merge_wrong += 1;
                }
            }
        }
        assert!(pairs > 0);
        assert_eq!(exact_collide, pairs, "fixture must be full class-floor collide");
        // FS must wrong-merge far less thanks to oi/rtc mismatch weights
        let exact_rate = exact_collide as f64 / pairs as f64;
        let fs_rate = fs_merge_wrong as f64 / pairs as f64;
        assert!(
            fs_rate <= exact_rate * 0.5,
            "FS wrong-merge {fs_rate} must be ≤50% of exact {exact_rate} (pairs={pairs} fs_merge={fs_merge_wrong})"
        );
        cleanup(g0, dir);
    }

    #[test]
    fn resolve_hotpath_links_identical_and_splits_different() {
        let (g0, dir) = isolate("resolve");
        let id1 = "dv0-aaaaaaaaaa-bbbbbbbbbb-cccccccccc-dddddddddd-eeeeeeeeee-ffffffffff-gggggggggg-hhhhhhhhhh-iiiiiiiiii-jjjjjjjjjj";
        let f1 = json!({"hw_curve_webgl": vec![0.2_f64; 16], "wg_whiten_lsh": "aabbccdd11223344"});
        catalog_register(id1, &f1, "dv0:aaaaaaaaaa-bbbbbbbbbb", false);
        let mut map = HashMap::new();
        map.insert(id1.to_string(), f1.clone());
        let r = resolve_identity_candidates(&f1, id1, "dv0:aaaaaaaaaa-bbbbbbbbbb", &[id1.to_string()], &map);
        assert_eq!(r["decision"], "merge");
        let id2 = "dv0-zzzzzzzzzz-yyyyyyyyyy-xxxxxxxxxx-wwwwwwwwww-vvvvvvvvvv-uuuuuuuuuu-tttttttttt-ssssssssss-rrrrrrrrrr-qqqqqqqqqq";
        let f2 = json!({"hw_curve_webgl": vec![0.9_f64; 16], "wg_whiten_lsh": "ffffffffffff0000"});
        let r2 = resolve_identity_candidates(&f2, id2, "dv0:zzzzzzzzzz", &[id1.to_string()], &map);
        assert!(
            r2["decision"] == "split" || r2["decision"] == "mint" || r2["decision"] == "abstain",
            "expected non-merge for different machine, got {}",
            r2["decision"]
        );
        cleanup(g0, dir);
    }

    #[test]
    fn mu_census_learns_high_u_for_class_digest() {
        let (g0, dir) = isolate("mu_u");
        let class = "deadclass01";
        for i in 0..30 {
            let oi = format!("oi{:08x}", 0x1000u32 + i as u32);
            let rtc = format!("rt{:08x}", 0x2000u32 + i as u32);
            let id = format!(
                "dv0-{c}-{c}-{c}-{c}-{c}-{c}-{c}-{c}-{oi}-{rtc}",
                c = class,
                oi = oi,
                rtc = rtc
            );
            let f = json!({"hw_curve_webgl": vec![0.26_f64; 32]});
            // u learning only (no m from mint — guardrail path)
            observe_mu_census_ex(
                &id,
                &f,
                MuObserveOpts {
                    allow_m_learn: false,
                    allow_u_learn: true,
                },
            );
        }
        let snap = mu_census_snapshot();
        let u = snap["slots"]["res"]["u_hat"].as_f64().unwrap_or(0.0);
        assert!(u > 0.45, "expected elevated u for class digest, got {u} snap={snap}");
        cleanup(g0, dir);
    }

    #[test]
    fn shared_hot_bucket_cross_process_file() {
        let (g0, dir) = isolate("shared_hot");
        let body = "dv0:sharedbodytest";
        // Distinct machines via oi/rtc in fields
        for i in 0..HOT_BUCKET_T2 {
            let f = json!({
                "os_instance_hash": format!("oi{i}"),
                "webrtc_host_ip_hash": format!("rtc{i}"),
            });
            let (n, level) = observe_bucket_heat_ex(
                body,
                &format!("s{i}"),
                &format!("dv0-aaaaaaaaaa-bbbbbbbbbb-cccccccccc-dddddddddd-eeeeeeeeee-ffffffffff-gggggggggg-hhhhhhhhhh-oi{i:08}-rt{i:08}"),
                Some(&f),
            );
            if i + 1 >= HOT_BUCKET_T2 {
                assert!(level >= 2, "n={n} level={level}");
            }
        }
        // Simulate other process: clear memory, reload from file
        {
            let mut guard = HOT_BUCKETS.lock().unwrap();
            *guard = Some(HotBucketState::new());
        }
        let f = json!({
            "os_instance_hash": "oi_new",
            "webrtc_host_ip_hash": "rtc_new",
        });
        let (n, level) = observe_bucket_heat_ex(body, "s_new_peer", "dv0-new", Some(&f));
        assert!(n >= HOT_BUCKET_T2, "shared file must restore heat n={n}");
        assert!(level >= 2, "level={level}");
        cleanup(g0, dir);
    }
}
