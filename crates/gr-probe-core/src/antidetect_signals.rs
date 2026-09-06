//! Antidetect / fingerprint-browser signals (iss/60 E2–E9, partial E6).
//! Pure server-side consumption of existing probe fields — no hard ban.
//!
//! E2 noise_structure_v1 · E3 version_capability_mismatch · E4 kernel_patched
//! E5 profile_farm_report · E6 ja4_ua_mismatch · E7 realm_physical_cross
//! E9 probe_blocking_pattern_v1

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;

pub const NOISE_STRUCTURE_ALGO: &str = "noise_structure_v1";
pub const VERSION_CAP_ALGO: &str = "version_capability_v1";
pub const KERNEL_CENSUS_ALGO: &str = "kernel_object_census_v1";
pub const PROFILE_FARM_ALGO: &str = "profile_farm_v1";
pub const JA4_UA_ALGO: &str = "ja4_ua_consistency_v1";
pub const REALM_CROSS_ALGO: &str = "realm_physical_cross_v1";
pub const PROBE_BLOCKING_ALGO: &str = "probe_blocking_pattern_v1";
pub const BEHAVIOR_SESSION_ALGO: &str = "behavior_session_v1";
pub const JA4_FULL_ALGO: &str = "ja4_full_surface_v1";
pub const ANTIDETECT_BUNDLE_ALGO: &str = "antidetect_signals_v1";

// ─── Atlas templates ───────────────────────────────────────────────────────

struct ModeCount {
    n: u64,
    by_hash: HashMap<String, u64>,
}

impl ModeCount {
    fn new() -> Self {
        Self {
            n: 0,
            by_hash: HashMap::new(),
        }
    }
    fn observe(&mut self, h: &str) {
        if h.is_empty() {
            return;
        }
        self.n += 1;
        *self.by_hash.entry(h.to_string()).or_insert(0) += 1;
        if self.by_hash.len() > 512 {
            if let Some(k) = self.by_hash.iter().min_by_key(|(_, c)| *c).map(|(k, _)| k.clone()) {
                self.by_hash.remove(&k);
            }
        }
    }
    fn top_rate(&self) -> f64 {
        if self.n == 0 {
            return 0.0;
        }
        let top = self.by_hash.values().copied().max().unwrap_or(0) as f64;
        top / self.n as f64
    }
}

struct FarmState {
    /// machine_unit → set of profile fingerprints (count)
    by_machine: HashMap<u64, HashMap<String, u64>>,
}

struct Atlas {
    api_by_ver: HashMap<String, ModeCount>,
    css_by_ver: HashMap<String, ModeCount>,
    object_by_ver: HashMap<String, ModeCount>,
    noise_modes: ModeCount,
    skip_patterns: ModeCount,
    farm: FarmState,
}

static ATLAS: Mutex<Option<Atlas>> = Mutex::new(None);

fn with_atlas<R>(f: impl FnOnce(&mut Atlas) -> R) -> R {
    let mut g = ATLAS.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(Atlas {
            api_by_ver: HashMap::new(),
            css_by_ver: HashMap::new(),
            object_by_ver: HashMap::new(),
            noise_modes: ModeCount::new(),
            skip_patterns: ModeCount::new(),
            farm: FarmState {
                by_machine: HashMap::new(),
            },
        });
    }
    f(g.as_mut().unwrap())
}

pub fn reset_antidetect_for_tests() {
    with_atlas(|a| {
        a.api_by_ver.clear();
        a.css_by_ver.clear();
        a.object_by_ver.clear();
        a.noise_modes = ModeCount::new();
        a.skip_patterns = ModeCount::new();
        a.farm.by_machine.clear();
    });
}

fn s(fields: &Value, keys: &[&str]) -> String {
    for k in keys {
        if let Some(v) = fields.get(*k).and_then(|x| x.as_str()) {
            if !s_empty(v) {
                return v.to_string();
            }
        }
    }
    String::new()
}

fn s_empty(v: &str) -> bool {
    v.is_empty() || v == "0" || v == "null"
}

fn chrome_major(fields: &Value) -> String {
    let ua = s(fields, &["user_agent", "ua", "sec_ch_ua"]);
    // Chrome/140.0.0.0 or "Chromium";v="140"
    for part in ua.split([' ', ',', ';', '"']) {
        if let Some(rest) = part.strip_prefix("Chrome/") {
            let maj = rest.split('.').next().unwrap_or("");
            if !maj.is_empty() {
                return maj.to_string();
            }
        }
        if part.chars().all(|c| c.is_ascii_digit()) && part.len() >= 2 && part.len() <= 3 {
            // weak
        }
    }
    if let Some(v) = fields
        .get("ua_ch_full_version_list")
        .and_then(|x| x.as_str())
        .or_else(|| fields.get("ua_ch_full_version").and_then(|x| x.as_str()))
    {
        for tok in v.split(['.', ',', ' ', '"']) {
            if tok.len() >= 2 && tok.len() <= 3 && tok.chars().all(|c| c.is_ascii_digit()) {
                let n: u32 = tok.parse().unwrap_or(0);
                if (80..=200).contains(&n) {
                    return tok.to_string();
                }
            }
        }
    }
    "unk".into()
}

// ─── E2 noise structure ────────────────────────────────────────────────────

/// Analyze repeated sample means / challenge repeats for noise class.
pub fn noise_structure(fields: &Value) -> Value {
    let mut samples: Vec<f64> = Vec::new();
    for k in [
        "challenge_repeat_means",
        "canvas_repeat_means",
        "noise_sample_means",
        "canvas_mean_series",
    ] {
        if let Some(a) = fields.get(k).and_then(|v| v.as_array()) {
            samples = a.iter().filter_map(|x| x.as_f64()).collect();
            if samples.len() >= 3 {
                break;
            }
        }
    }
    // Also use multipath residual path means if present
    if samples.len() < 3 {
        if let Some(a) = fields.get("residual_path_means").and_then(|v| v.as_array()) {
            samples = a.iter().filter_map(|x| x.as_f64()).collect();
        }
    }

    let noise_suspect = fields
        .get("noise_suspect")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let (class, entropy, corr) = if samples.len() >= 3 {
        let mean = samples.iter().sum::<f64>() / samples.len() as f64;
        let var = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / samples.len() as f64;
        let std = var.sqrt();
        // pairwise diffs entropy proxy
        let mut diffs: Vec<f64> = Vec::new();
        for i in 1..samples.len() {
            diffs.push((samples[i] - samples[i - 1]).abs());
        }
        let dmean = diffs.iter().sum::<f64>() / diffs.len().max(1) as f64;
        // spatial corr proxy: consecutive same sign
        let mut same_sign = 0usize;
        for i in 2..samples.len() {
            let a = samples[i] - samples[i - 1];
            let b = samples[i - 1] - samples[i - 2];
            if a * b > 0.0 {
                same_sign += 1;
            }
        }
        let corr = if samples.len() > 2 {
            same_sign as f64 / (samples.len() - 2) as f64
        } else {
            0.0
        };
        let class = if std < 1e-12 {
            "none"
        } else if dmean > std * 0.5 && corr < 0.35 {
            "random_per_call"
        } else if std > 1e-9 && corr > 0.7 {
            "fixed_offset"
        } else if noise_suspect {
            "per_session"
        } else {
            "structured_or_none"
        };
        // entropy-ish: normalized unique bins
        let mut bins: Vec<i64> = samples.iter().map(|x| (x * 1e6).round() as i64).collect();
        bins.sort_unstable();
        bins.dedup();
        let entropy = (bins.len() as f64).ln().max(0.0);
        (class, entropy, corr)
    } else if noise_suspect {
        ("suspect_flag_only", 0.0, 0.0)
    } else {
        ("insufficient_samples", 0.0, 0.0)
    };

    with_atlas(|a| {
        a.noise_modes.observe(class);
    });

    let vendor_like = with_atlas(|a| {
        // High concentration of same noise class across "devices" → vendor signature
        a.noise_modes.top_rate() > 0.4 && a.noise_modes.n >= 20 && class != "none"
    });

    json!({
        "algo": NOISE_STRUCTURE_ALGO,
        "noise_class": class,
        "entropy": (entropy * 1000.0).round() / 1000.0,
        "spatial_corr": (corr * 1000.0).round() / 1000.0,
        "sample_n": samples.len(),
        "noise_suspect_flag": noise_suspect,
        "vendor_noise_signature_hint": vendor_like,
        "action": if class == "random_per_call" || class == "fixed_offset" {
            "downweight_conf"
        } else {
            "none"
        },
        "hard_ban": false,
    })
}

// ─── E3 version-capability ─────────────────────────────────────────────────

pub fn version_capability_mismatch(fields: &Value) -> Value {
    let ver = chrome_major(fields);
    let api = s(fields, &["api_flags_hash", "api_surface_hash"]);
    let css = s(fields, &["css_supports_hash", "css_support_hash"]);
    let mq = s(fields, &["media_query_hash", "mq_hash"]);

    with_atlas(|a| {
        if !api.is_empty() {
            a.api_by_ver.entry(ver.clone()).or_insert_with(ModeCount::new).observe(&api);
        }
        if !css.is_empty() {
            a.css_by_ver.entry(ver.clone()).or_insert_with(ModeCount::new).observe(&css);
        }
    });

    let mut mismatch = false;
    let mut reasons = Vec::new();
    with_atlas(|a| {
        if let Some(mc) = a.api_by_ver.get(&ver) {
            if mc.n >= 10 && !api.is_empty() {
                let cnt = mc.by_hash.get(&api).copied().unwrap_or(0);
                let rate = cnt as f64 / mc.n as f64;
                if rate < 0.05 && mc.top_rate() > 0.3 {
                    mismatch = true;
                    reasons.push(json!({
                        "id": "api_flags_rare_for_version",
                        "version": ver,
                        "rate": rate,
                    }));
                }
            }
        }
        if let Some(mc) = a.css_by_ver.get(&ver) {
            if mc.n >= 10 && !css.is_empty() {
                let cnt = mc.by_hash.get(&css).copied().unwrap_or(0);
                let rate = cnt as f64 / mc.n as f64;
                if rate < 0.05 && mc.top_rate() > 0.3 {
                    mismatch = true;
                    reasons.push(json!({
                        "id": "css_supports_rare_for_version",
                        "version": ver,
                        "rate": rate,
                    }));
                }
            }
        }
    });

    // Heuristic without atlas: UA claims very new chrome but api_flags empty
    if ver != "unk" {
        if let Ok(v) = ver.parse::<u32>() {
            if v >= 130 && api.is_empty() && mq.is_empty() && css.is_empty() {
                // missing capability surface while claiming modern chrome
                reasons.push(json!({
                    "id": "modern_ua_empty_capability_surface",
                    "version": ver,
                }));
                mismatch = true;
            }
        }
    }

    json!({
        "algo": VERSION_CAP_ALGO,
        "chrome_major": ver,
        "mismatch": mismatch,
        "reasons": reasons,
        "action": if mismatch { "downweight_conf" } else { "none" },
        "hard_ban": false,
    })
}

// ─── E4 object census ──────────────────────────────────────────────────────

pub fn kernel_object_census(fields: &Value) -> Value {
    let ver = chrome_major(fields);
    let oh = s(
        fields,
        &[
            "object_census_struct_hash",
            "nav_struct_hash",
            "window_keys_hash",
            "object_census_hash",
        ],
    );
    with_atlas(|a| {
        if !oh.is_empty() {
            a.object_by_ver
                .entry(ver.clone())
                .or_insert_with(ModeCount::new)
                .observe(&oh);
        }
    });
    let mut suspect = false;
    let mut detail = Value::Null;
    with_atlas(|a| {
        if let Some(mc) = a.object_by_ver.get(&ver) {
            if mc.n >= 15 && !oh.is_empty() {
                let cnt = mc.by_hash.get(&oh).copied().unwrap_or(0);
                let rate = cnt as f64 / mc.n as f64;
                if rate < 0.03 && mc.top_rate() > 0.25 {
                    suspect = true;
                    detail = json!({
                        "struct_hash": oh,
                        "version": ver,
                        "rate": rate,
                        "population_n": mc.n,
                    });
                }
            }
        }
    });
    json!({
        "algo": KERNEL_CENSUS_ALGO,
        "kernel_patched_suspect": suspect,
        "detail": detail,
        "action": if suspect { "downweight_conf_force_deepen" } else { "none" },
        "hard_ban": false,
    })
}

// ─── E5 profile farm ───────────────────────────────────────────────────────

pub fn profile_farm_observe(fields: &Value, machine_unit: u64, session_id: &str) {
    if machine_unit == 0 {
        return;
    }
    // Profile fingerprint: storage + UA slice (not full content)
    let mut h = Sha256::new();
    h.update(s(fields, &["user_agent", "ua"]).as_bytes());
    h.update(b"|");
    h.update(s(fields, &["local_storage_seed", "storage_bind", "vtid"]).as_bytes());
    h.update(b"|");
    h.update(session_id.as_bytes());
    let pf = format!("{:x}", h.finalize())[..12].to_string();
    with_atlas(|a| {
        let m = a.farm.by_machine.entry(machine_unit).or_default();
        *m.entry(pf).or_insert(0) += 1;
        if m.len() > 256 {
            // drop rarest
            if let Some(k) = m.iter().min_by_key(|(_, c)| *c).map(|(k, _)| k.clone()) {
                m.remove(&k);
            }
        }
    });
}

pub fn profile_farm_report(machine_unit: u64) -> Value {
    with_atlas(|a| {
        let profiles = a
            .farm
            .by_machine
            .get(&machine_unit)
            .map(|m| m.len())
            .unwrap_or(0);
        let heat = if profiles >= 20 {
            2
        } else if profiles >= 8 {
            1
        } else {
            0
        };
        json!({
            "algo": PROFILE_FARM_ALGO,
            "machine_unit_hash": format!("{:016x}", machine_unit),
            "distinct_profile_n": profiles,
            "heat_level": heat,
            "alert": profiles >= 8,
            "privacy": "counts_only_no_profile_content",
            "action": if profiles >= 8 { "ephemeral_or_deepen" } else { "none" },
            "hard_ban": false,
        })
    })
}

// ─── E6 JA4 vs UA ──────────────────────────────────────────────────────────

pub fn ja4_ua_consistency(fields: &Value) -> Value {
    let ja4 = s(fields, &["ja4", "tls_ja4"]);
    let eng = s(fields, &["protocol_engine", "gateway_protocol_engine"]);
    let ua = s(fields, &["user_agent", "ua"]).to_ascii_lowercase();
    let mut mismatch = false;
    let mut reasons = Vec::new();
    if !ja4.is_empty() {
        let j = ja4.to_ascii_lowercase();
        if (ua.contains("firefox") || ua.contains("gecko"))
            && j.starts_with("t13")
            && eng.contains("chrome")
        {
            mismatch = true;
            reasons.push("ua_firefox_ja4_chrome_class");
        }
        if ua.contains("chrome") && !ua.contains("edg") && eng.contains("firefox") {
            mismatch = true;
            reasons.push("ua_chrome_protocol_firefox");
        }
        if ua.contains("chrome/") {
            // crude: very old ja4 class with new chrome
            let ver = chrome_major(fields);
            if let Ok(v) = ver.parse::<u32>() {
                if v >= 120 && j.contains("t12d") {
                    mismatch = true;
                    reasons.push("modern_chrome_tls12_class");
                }
            }
        }
    }
    json!({
        "algo": JA4_UA_ALGO,
        "mismatch": mismatch,
        "ja4_present": !ja4.is_empty(),
        "reasons": reasons,
        "action": if mismatch { "downweight_conf" } else { "none" },
        "hard_ban": false,
        "note": "full JA4T/L/H still depends on gateway export (iss/45); uses available ja4/engine fields",
    })
}

// ─── E7 realm physical cross ───────────────────────────────────────────────

pub fn realm_physical_cross(fields: &Value) -> Value {
    let main_res = fields
        .get("residual_mean")
        .or_else(|| fields.get("webgl_residual_mean"))
        .and_then(|v| v.as_f64());
    let sand_res = fields
        .get("sandbox_residual_mean")
        .or_else(|| fields.get("iframe_residual_mean"))
        .and_then(|v| v.as_f64());
    let layer_div = fields
        .get("layer_divergence")
        .or_else(|| fields.get("cross_realm_divergence"))
        .cloned()
        .unwrap_or(Value::Null);

    let mut hook_suspect = false;
    let mut details = Vec::new();
    if let (Some(m), Some(s)) = (main_res, sand_res) {
        let d = (m - s).abs();
        // Same hardware → residual means should be close
        if d > 0.05 {
            hook_suspect = true;
            details.push(json!({
                "id": "main_sandbox_residual_divergence",
                "main": m,
                "sandbox": s,
                "abs_delta": d,
            }));
        }
    }
    if let Some(obj) = layer_div.as_object() {
        let flags = obj
            .get("divergent_keys")
            .or_else(|| obj.get("keys"))
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        if flags >= 3 {
            hook_suspect = true;
            details.push(json!({
                "id": "layer_divergence_many_keys",
                "n": flags,
            }));
        }
    }

    json!({
        "algo": REALM_CROSS_ALGO,
        "main_realm_hook_suspect": hook_suspect,
        "details": details,
        "action": if hook_suspect { "downweight_conf_force_deepen" } else { "none" },
        "hard_ban": false,
    })
}

// ─── E9 probe blocking ─────────────────────────────────────────────────────

pub fn probe_blocking_pattern(fields: &Value) -> Value {
    let mut skips = Vec::new();
    if let Some(obj) = fields.as_object() {
        for (k, v) in obj {
            if k.ends_with("_skip") || k.ends_with("_blocked") || k == "pack_skip_reasons" {
                if let Some(s) = v.as_str() {
                    if !s.is_empty() {
                        skips.push(format!("{k}={s}"));
                    }
                } else if let Some(a) = v.as_array() {
                    for x in a {
                        if let Some(s) = x.as_str() {
                            skips.push(s.to_string());
                        }
                    }
                }
            }
        }
    }
    // Boolean capability collapses
    let mut missing = Vec::new();
    for (k, label) in [
        ("has_webgl", "webgl"),
        ("has_webgl2", "webgl2"),
        ("has_audio_ctx", "audio"),
        ("has_worker", "worker"),
        ("has_shared_array", "sab"),
        ("has_webgpu", "webgpu"),
    ] {
        if fields.get(k).and_then(|v| v.as_bool()) == Some(false) {
            missing.push(label);
        }
    }
    let pattern = {
        let mut p = skips.clone();
        p.extend(missing.iter().map(|m| format!("miss:{m}")));
        p.sort();
        p.join("|")
    };
    with_atlas(|a| {
        if !pattern.is_empty() {
            a.skip_patterns.observe(&pattern);
        }
    });

    // Abnormal: many critical miss together (rare on real Chrome)
    let critical_miss = missing.contains(&"webgl") && missing.contains(&"audio");
    let sab_worker_miss = missing.contains(&"sab") && missing.contains(&"worker");
    let alert = critical_miss || sab_worker_miss || skips.len() >= 6;
    let pattern_digest = {
        let mut h = Sha256::new();
        h.update(pattern.as_bytes());
        format!("{:x}", h.finalize())[..12].to_string()
    };

    json!({
        "algo": PROBE_BLOCKING_ALGO,
        "skip_n": skips.len(),
        "missing_caps": missing,
        "pattern_digest": pattern_digest,
        "blocking_alert": alert,
        "action": if alert { "downweight_conf" } else { "none" },
        "hard_ban": false,
        "note": "absence itself is a signal; no hard ban",
    })
}

// ─── E10 behavior session (privacy-safe, no biometrics) ───────────────────

/// Unattended / bot-like session stats from already-collected timing curves.
/// Does **not** use keystroke/mouse biometrics.
pub fn behavior_session_signals(fields: &Value) -> Value {
    let raf: Vec<f64> = fields
        .get("raf_spacing_curve")
        .or_else(|| fields.get("raf_interval_curve"))
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_f64()).collect())
        .unwrap_or_default();
    let sched: Vec<f64> = fields
        .get("scheduler_timing_curve")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_f64()).collect())
        .unwrap_or_default();
    let has_input = fields
        .get("had_user_input")
        .or_else(|| fields.get("interaction_events_n"))
        .map(|v| {
            v.as_bool().unwrap_or(false)
                || v.as_u64().unwrap_or(0) > 0
                || v.as_f64().unwrap_or(0.0) > 0.0
        })
        .unwrap_or(false);
    let dwell_ms = fields
        .get("session_dwell_ms")
        .or_else(|| fields.get("page_visible_ms"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    let mut perfect_60 = false;
    let mut raf_std = 0.0;
    if raf.len() >= 6 {
        let mean = raf.iter().sum::<f64>() / raf.len() as f64;
        raf_std = (raf.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / raf.len() as f64).sqrt();
        // ~16.67ms ± 0.3 and tiny std → synthetic clock
        let near_16 = raf.iter().filter(|x| (**x - 16.67).abs() < 0.35).count();
        perfect_60 = near_16 as f64 / raf.len() as f64 > 0.9 && raf_std < 0.25;
    }

    let unattended_long = dwell_ms >= 30_000.0 && !has_input;
    let bot_like = (perfect_60 && !has_input) || (unattended_long && perfect_60);
    let score = (if perfect_60 { 0.35 } else { 0.0 })
        + (if unattended_long { 0.25 } else { 0.0 })
        + (if bot_like { 0.2 } else { 0.0 });

    json!({
        "algo": BEHAVIOR_SESSION_ALGO,
        "raf_n": raf.len(),
        "raf_std_ms": (raf_std * 1000.0).round() / 1000.0,
        "perfect_60fps_clock": perfect_60,
        "scheduler_n": sched.len(),
        "had_user_input": has_input,
        "dwell_ms": dwell_ms,
        "unattended_long_session": unattended_long,
        "bot_like_session": bot_like,
        "score": ((score as f64) * 1000.0).round() / 1000.0,
        "action": if score >= 0.35 { "downweight_conf" } else { "none" },
        "hard_ban": false,
        "privacy": "no_keystroke_mouse_biometrics",
    })
}

// ─── E6 JA4 full surface (T/L/H assemble + product) ───────────────────────

/// Assemble JA4 / JA4T / JA4L / JA4H product surface from gateway+FE fields.
pub fn ja4_full_surface(fields: &Value) -> Value {
    let ja4 = s(fields, &["ja4", "tls_ja4"]);
    let ja4_r = s(fields, &["ja4_r", "tls_ja4_r"]);
    let ja4t = s(fields, &["ja4t", "tcp_syn_ja4t", "tcp_syn_option_order"]);
    let ja4l = s(fields, &["ja4l", "ja4l_lite", "ja4l_partial"]);
    let ja4h = s(fields, &["ja4h", "ja4h_lite"]);
    let mut parts_present = Map::new();
    parts_present.insert("ja4".into(), json!(!ja4.is_empty()));
    parts_present.insert("ja4_r".into(), json!(!ja4_r.is_empty()));
    parts_present.insert("ja4t".into(), json!(!ja4t.is_empty()));
    parts_present.insert("ja4l".into(), json!(!ja4l.is_empty()));
    parts_present.insert("ja4h".into(), json!(!ja4h.is_empty()));

    // Composite digest for ops (not commercial mint alone)
    let mut h = Sha256::new();
    h.update(b"ja4_full_v1|");
    for p in [&ja4, &ja4t, &ja4l, &ja4h] {
        h.update(p.as_bytes());
        h.update(b"|");
    }
    let composite = format!("{:x}", h.finalize())[..16].to_string();

    // Depth score
    let depth = [&ja4, &ja4t, &ja4l, &ja4h]
        .iter()
        .filter(|s| !s.is_empty())
        .count();

    // Ensure JA4H product computed if order present
    let ja4h_prod = crate::ja4h_lite::ja4h_lite_product(fields);

    // UA consistency
    let ua_cons = ja4_ua_consistency(fields);

    json!({
        "algo": JA4_FULL_ALGO,
        "ja4": if ja4.is_empty() { Value::Null } else { json!(ja4) },
        "ja4_r": if ja4_r.is_empty() { Value::Null } else { json!(ja4_r) },
        "ja4t": if ja4t.is_empty() { Value::Null } else { json!(ja4t) },
        "ja4l": if ja4l.is_empty() { Value::Null } else { json!(ja4l) },
        "ja4h": if ja4h.is_empty() {
            ja4h_prod.get("ja4h").cloned().unwrap_or(Value::Null)
        } else {
            json!(ja4h)
        },
        "parts_present": parts_present,
        "depth_n": depth,
        "composite_digest": composite,
        "ja4h_product": ja4h_prod,
        "ua_consistency": ua_cons,
        "full_surface": depth >= 3,
        "commercial_mint": false,
        "role": "protocol_observer_conf_assist",
        "action": if ua_cons.get("mismatch").and_then(|v| v.as_bool()).unwrap_or(false) {
            "downweight_conf"
        } else {
            "none"
        },
        "hard_ban": false,
        "note": "FoxIO-complete when gateway injects ja4+ja4t+ja4l+ja4h; partial ok for lab",
    })
}

// ─── E11 Atlas shadow (iss/67 C3, iss/69 U8) ───────────────────────────────

pub const ATLAS_SHADOW_ALGO: &str = "atlas_shadow_signal_v1";

/// E11: K↔V Atlas signals (iss/63 commercial, iss/67 C3).
/// Soft path: can request deepen/downweight when `should_deepen` / demote_weight high.
/// **Never hard-ban.**
pub fn atlas_shadow_signal(fields: &Value) -> Value {
    let score = fields
        .get("atlas_shadow_score")
        .cloned()
        .unwrap_or_else(|| crate::atlas_score::atlas_shadow_score(fields));
    let signals: Vec<String> = score
        .get("signals")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let flags = vec![
        (
            "spoof_shadow",
            score
                .get("spoof_shadow")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        ),
        (
            "farm_tightness",
            score
                .get("farm_tightness")
                .and_then(|v| v.get("tight").and_then(|x| x.as_bool()))
                .unwrap_or(false),
        ),
        (
            "zero_silicon_shadow",
            score
                .get("zero_silicon_shadow")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        ),
        (
            "replay_exact_shadow",
            score
                .get("replay_exact_shadow")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        ),
    ];
    let demote = score
        .get("demote_weight")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let should_deepen = score
        .get("should_deepen")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || demote >= 0.24;
    let should_downweight = score
        .get("should_downweight")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || demote >= 0.18;
    let action = if should_deepen {
        "deepen_and_downweight"
    } else if should_downweight {
        "downweight_conf"
    } else if !signals.is_empty() || flags.iter().any(|(_, v)| *v) {
        "shadow_watch"
    } else {
        "none"
    };
    json!({
        "algo": ATLAS_SHADOW_ALGO,
        "code": "E11",
        "present": action != "none",
        "signals": signals,
        "flags": flags,
        "demote_weight": demote,
        "atlas_fit_score": score.get("atlas_fit_score").cloned().unwrap_or(json!(null)),
        "best_alt_key": score.get("best_alt_key").cloned().unwrap_or(json!(null)),
        "hw_model_key": score.get("hw_model_key").cloned().unwrap_or(json!(null)),
        "action": action,
        "hard_ban": false,
        "note": "E11 K/V atlas — soft demote/deepen only; never hard-ban",
    })
}

// ─── Bundle ────────────────────────────────────────────────────────────────

pub fn antidetect_signals_surface(fields: &Value, machine_unit: u64, session_id: &str) -> Value {
    profile_farm_observe(fields, machine_unit, session_id);
    let noise = noise_structure(fields);
    let ver = version_capability_mismatch(fields);
    let kern = kernel_object_census(fields);
    let farm = profile_farm_report(machine_unit);
    let ja4 = ja4_ua_consistency(fields);
    let ja4_full = ja4_full_surface(fields);
    let realm = realm_physical_cross(fields);
    let block = probe_blocking_pattern(fields);
    let behavior = behavior_session_signals(fields);
    let atlas_shadow = atlas_shadow_signal(fields);

    let mut score = 0.0;
    let mut deepen = false;
    for (v, w) in [
        (&noise, 0.25),
        (&ver, 0.2),
        (&kern, 0.25),
        (&farm, 0.3),
        (&ja4, 0.2),
        (&ja4_full, 0.15),
        (&realm, 0.3),
        (&block, 0.2),
        (&behavior, 0.25),
        (&atlas_shadow, 0.22), // E11 K/V atlas soft weight
    ] {
        let act = v.get("action").and_then(|x| x.as_str()).unwrap_or("none");
        if act != "none" && act != "shadow_watch" {
            score += w;
        } else if act == "shadow_watch" {
            score += w * 0.35; // light contribution
        }
        if act.contains("deepen") || act.contains("ephemeral") {
            deepen = true;
        }
    }
    if farm.get("alert").and_then(|v| v.as_bool()).unwrap_or(false) {
        deepen = true;
    }
    if atlas_shadow
        .get("action")
        .and_then(|v| v.as_str())
        .map(|a| a.contains("deepen"))
        .unwrap_or(false)
    {
        deepen = true;
    }

    json!({
        "algo": ANTIDETECT_BUNDLE_ALGO,
        "noise": noise,
        "version_capability": ver,
        "kernel_census": kern,
        "profile_farm": farm,
        "ja4_ua": ja4,
        "ja4_full": ja4_full,
        "realm_cross": realm,
        "probe_blocking": block,
        "behavior_session": behavior,
        "atlas_shadow": atlas_shadow,
        "bundle_score": ((score as f64) * 1000.0).round() / 1000.0,
        "should_downweight": score >= 0.25,
        "should_deepen": deepen || score >= 0.45,
        "hard_ban": false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// These tests mutate the process-global atlas state (`with_atlas`) without
    /// any synchronization; parallel `cargo test` threads can interleave a
    /// `reset_antidetect_for_tests()` with another test's observations and wipe
    /// them mid-flight (observed as a flaky `profile_farm_counts` failure).
    /// Serialize all atlas-mutating tests on one lock.
    static ATLAS_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn noise_flags_random_series() {
        let _g = ATLAS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_antidetect_for_tests();
        let f = json!({
            "challenge_repeat_means": [0.1, 0.9, 0.2, 0.85, 0.15],
            "noise_suspect": true,
        });
        let r = noise_structure(&f);
        assert!(
            r["noise_class"].as_str().unwrap_or("") == "random_per_call"
                || r["noise_class"].as_str().unwrap_or("").contains("suspect"),
            "{r}"
        );
    }

    #[test]
    fn profile_farm_counts() {
        let _g = ATLAS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_antidetect_for_tests();
        let mu = 0xabcdu64;
        for i in 0..12 {
            let f = json!({"user_agent": format!("UA{i}"), "vtid": format!("vt{i}")});
            profile_farm_observe(&f, mu, &format!("s{i}"));
        }
        let r = profile_farm_report(mu);
        assert!(r["distinct_profile_n"].as_u64().unwrap() >= 8, "{r}");
        assert_eq!(r["alert"], true);
    }

    #[test]
    fn blocking_webgl_audio_missing() {
        let f = json!({"has_webgl": false, "has_audio_ctx": false, "has_worker": true});
        let r = probe_blocking_pattern(&f);
        assert_eq!(r["blocking_alert"], true);
    }

    #[test]
    fn atlas_shadow_signal_reflects_score_without_block() {
        let f = json!({
            "atlas_shadow_score": {
                "signals": ["farm_tightness_shadow", "replay_exact_shadow"],
                "spoof_shadow": false,
                "farm_tightness": {"tight": true},
                "zero_silicon_shadow": false,
                "replay_exact_shadow": true,
            }
        });
        let r = atlas_shadow_signal(&f);
        assert_eq!(r["present"], true);
        assert_eq!(r["action"], "shadow_watch");
        assert_eq!(r["hard_ban"], false);
        assert!(r["signals"].as_array().unwrap().len() >= 2);
        let b = antidetect_signals_surface(&f, 0xdef1, "s1");
        assert!(b.get("atlas_shadow").is_some());
        assert_eq!(b["hard_ban"], false);
    }

    #[test]
    fn e11_deepen_and_downweight_when_demote_high() {
        let f = json!({
            "atlas_shadow_score": {
                "signals": ["model_silicon_mismatch_shadow", "zero_silicon_shadow", "replay_exact_shadow"],
                "spoof_shadow": true,
                "farm_tightness": {"tight": false},
                "zero_silicon_shadow": true,
                "replay_exact_shadow": true,
                "demote_weight": 0.40,
                "should_deepen": true,
                "should_downweight": true,
                "atlas_fit_score": 0.22,
                "best_alt_key": "nvidia:gtx_1050_ti",
                "hw_model_key": "nvidia:rtx_4090",
            }
        });
        let r = atlas_shadow_signal(&f);
        assert_eq!(r["code"], "E11");
        assert_eq!(r["action"], "deepen_and_downweight", "{r}");
        assert_eq!(r["hard_ban"], false);
        assert!(r["demote_weight"].as_f64().unwrap() >= 0.24);
        assert_eq!(r["best_alt_key"], "nvidia:gtx_1050_ti");
        let b = antidetect_signals_surface(&f, 0xabc1, "e11s");
        assert_eq!(b["should_deepen"], true, "{b}");
        assert_eq!(b["hard_ban"], false);
        assert!(
            b["atlas_shadow"]["action"].as_str().unwrap().contains("deepen"),
            "{b}"
        );
    }

    #[test]
    fn e11_downweight_only_mid_demote() {
        let f = json!({
            "atlas_shadow_score": {
                "signals": ["farm_tightness_shadow"],
                "spoof_shadow": false,
                "farm_tightness": {"tight": true},
                "zero_silicon_shadow": false,
                "replay_exact_shadow": false,
                "demote_weight": 0.18,
                "should_deepen": false,
                "should_downweight": true,
            }
        });
        let r = atlas_shadow_signal(&f);
        assert_eq!(r["action"], "downweight_conf", "{r}");
        assert_eq!(r["hard_ban"], false);
    }
}
