//! Stack co-occurrence sample library (population prior).
//!
//! File: `spec/stack_cooccurrence_prior_v1.json`
//!
//! Brain uses this to judge whether a (form, os, engine, browser_shell) stack is:
//! - **common** / **uncommon_native** / **rare_native** → often residual channel prior only
//! - **migration_suspect** / **impossible** / **conflict** → authenticity deepen
//!
//! **Not** a brand allowlist hard gate. **Not** peer same-machine. **Not** digest material.
//!
//! Critical distinctions encoded in the sample library:
//! - Epiphany/WebKitGTK on Linux = **rare_native** (legitimate), B10x channel prior
//! - Safari brand on Linux = **migration_suspect**
//! - Chrome on iOS = WebKit **common** (Apple policy), not engine migration

use serde_json::{json, Map, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::contracts::{find_spec_dir, ContractError};

static LIB: OnceLock<Result<Value, String>> = OnceLock::new();

pub fn load_stack_cooccurrence_prior() -> Result<&'static Value, ContractError> {
    let r = LIB.get_or_init(|| {
        let path = PathBuf::from(find_spec_dir()).join("stack_cooccurrence_prior_v1.json");
        let raw = fs::read_to_string(&path).map_err(|e| format!("read {path:?}: {e}"))?;
        serde_json::from_str(&raw).map_err(|e| format!("parse stack cooccur: {e}"))
    });
    r.as_ref().map_err(|e| ContractError::Msg(e.clone()))
}

fn alias_map<'a>(lib: &'a Value, section: &str) -> Option<&'a Map<String, Value>> {
    lib.pointer(&format!("/normalize/{section}"))
        .and_then(|v| v.as_object())
}

fn norm_token(lib: &Value, section: &str, raw: &str) -> String {
    let s = raw.trim().to_ascii_lowercase();
    if s.is_empty() {
        return "unknown".into();
    }
    if let Some(m) = alias_map(lib, section) {
        // exact
        if let Some(v) = m.get(&s).and_then(|x| x.as_str()) {
            return v.to_string();
        }
        // substring contains for browser names
        for (k, v) in m {
            if s.contains(k) {
                if let Some(out) = v.as_str() {
                    return out.to_string();
                }
            }
        }
    }
    // light heuristics when alias miss
    if section == "browser_shell_aliases" {
        if s.contains("epiphany") || s.contains("gnome") {
            return "epiphany".into();
        }
        if s.contains("samsung") {
            return "samsung".into();
        }
        if s.contains("firefox") || s.contains("fxios") {
            return "firefox".into();
        }
        if s.contains("edg") {
            return "edge".into();
        }
        if s.contains("chrome") || s.contains("chromium") || s.contains("crios") {
            return "chrome".into();
        }
        if s.contains("safari") {
            return "safari".into();
        }
        if s.contains("opera") || s.contains("opr/") {
            return "opera".into();
        }
        if s.contains("brave") {
            return "brave".into();
        }
    }
    if section == "os_family_aliases" {
        if s.contains("android") {
            return "android".into();
        }
        if s.contains("iphone") || s.contains("ipad") || s.contains("ios") {
            return "ios".into();
        }
        if s.contains("win") {
            return "windows".into();
        }
        if s.contains("mac") || s.contains("darwin") {
            return "macos".into();
        }
        if s.contains("cros") || s.contains("chrome os") {
            return "chromeos".into();
        }
        if s.contains("linux") || s.contains("x11") || s.contains("ubuntu") {
            return "linux".into();
        }
    }
    s
}

/// Extract normalized stack dimensions from fields.
pub fn extract_stack_dims(fields: &Value, lib: &Value) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();

    let form_raw = fo
        .get("form_class")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| {
            let ua = fo
                .get("user_agent")
                .or_else(|| fo.get("ua"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ua.contains("mobile") || ua.contains("android") || ua.contains("iphone") {
                "mobile"
            } else if ua.contains("ipad") || ua.contains("tablet") {
                "tablet"
            } else {
                "desktop"
            }
        });
    let form = norm_token(lib, "form_aliases", form_raw);

    let os_raw = fo
        .get("os_family")
        .and_then(|v| v.as_str())
        .or_else(|| fo.get("platform").and_then(|v| v.as_str()))
        .or_else(|| fo.get("navigator_platform").and_then(|v| v.as_str()))
        .or_else(|| {
            let ua = fo
                .get("user_agent")
                .or_else(|| fo.get("ua"))
                .and_then(|v| v.as_str())?;
            Some(ua)
        })
        .unwrap_or("unknown");
    let os_family = norm_token(lib, "os_family_aliases", os_raw);

    let eng_raw = fo
        .get("residual_probe_engine")
        .or_else(|| fo.get("engine_family"))
        .or_else(|| fo.get("engine_obs"))
        .or_else(|| fo.get("engine_claim"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let engine = norm_token(lib, "engine_aliases", eng_raw);

    let br_raw = fo
        .get("browser_name")
        .or_else(|| fo.get("br_name"))
        .and_then(|v| v.as_str())
        .or_else(|| {
            fo.get("user_agent")
                .or_else(|| fo.get("ua"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or("unknown");
    let browser_shell = norm_token(lib, "browser_shell_aliases", br_raw);

    let arch = fo
        .get("ua_ch_architecture")
        .or_else(|| fo.get("cpu_arch"))
        .and_then(|v| v.as_str())
        .map(|a| {
            let a = a.to_ascii_lowercase();
            if a.contains("arm") {
                "arm"
            } else if a.contains("x86") || a.contains("amd64") || a.contains("x64") {
                "x86"
            } else {
                "*"
            }
        })
        .unwrap_or_else(|| {
            let p = fo
                .get("platform")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if p.contains("aarch64") || p.contains("arm") {
                "arm"
            } else if p.contains("x86_64") || p.contains("win64") || p.contains("linux x86") {
                "x86"
            } else {
                "*"
            }
        });

    json!({
        "form": form,
        "os_family": os_family,
        "engine": engine,
        "browser_shell": browser_shell,
        "arch": arch,
    })
}

fn tuple_key(form: &str, os: &str, eng: &str, br: &str, arch: &str) -> String {
    format!("{form}|{os}|{eng}|{br}|{arch}")
}

/// Wildcard match: pattern may use `*` in any slot; more specific wins later via search order.
fn pattern_matches(pattern: &str, key: &str) -> bool {
    let pp: Vec<&str> = pattern.split('|').collect();
    let kk: Vec<&str> = key.split('|').collect();
    if pp.len() != 5 || kk.len() != 5 {
        return false;
    }
    for i in 0..5 {
        if pp[i] != "*" && pp[i] != kk[i] {
            return false;
        }
    }
    true
}

fn pattern_specificity(pattern: &str) -> i32 {
    pattern
        .split('|')
        .map(|p| if p == "*" { 0 } else { 1 })
        .sum()
}

/// Lookup co-occurrence entry for this session's stack.
pub fn lookup_stack_cooccurrence(fields: &Value) -> Value {
    let Ok(lib) = load_stack_cooccurrence_prior() else {
        return json!({
            "algo": "stack_cooccurrence_prior_v1",
            "loaded": false,
            "status": "uncommon_native",
            "b10x_prior": true,
            "note": "prior file missing — safe rare fallback",
        });
    };

    let dims = extract_stack_dims(fields, lib);
    let form = dims.get("form").and_then(|v| v.as_str()).unwrap_or("unknown");
    let os = dims
        .get("os_family")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let eng = dims.get("engine").and_then(|v| v.as_str()).unwrap_or("unknown");
    let br = dims
        .get("browser_shell")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let arch = dims.get("arch").and_then(|v| v.as_str()).unwrap_or("*");
    let key = tuple_key(form, os, eng, br, arch);

    let tuples = lib
        .get("tuples")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut best: Option<(i32, Value)> = None;
    for t in &tuples {
        let pat = t.get("key").and_then(|v| v.as_str()).unwrap_or("");
        if !pattern_matches(pat, &key) {
            continue;
        }
        let spec = pattern_specificity(pat);
        if best.as_ref().map(|(s, _)| spec > *s).unwrap_or(true) {
            best = Some((spec, t.clone()));
        }
    }

    let (status, rate, residual_channel, b10x_prior, domains, tuple_note, matched_key) =
        if let Some((_, t)) = best {
            (
                t.get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("uncommon_native")
                    .to_string(),
                t.get("rate").and_then(|v| v.as_f64()).unwrap_or(0.0001),
                t.get("residual_channel")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unconventional")
                    .to_string(),
                t.get("b10x_prior")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                t.get("domains").cloned().unwrap_or(json!([])),
                t.get("note")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                t.get("key")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            )
        } else {
            (
                "uncommon_native".into(),
                0.0001,
                "unconventional".into(),
                true,
                json!(["device_residual", "browser_kernel"]),
                "no tuple match".into(),
                String::new(),
            )
        };

    // Conflict overlays from field-level flags (caller may also pass flags).
    let fo = fields.as_object().cloned().unwrap_or_default();
    let mut conflict_hits: Vec<Value> = Vec::new();
    let mut anomaly_score = 0.0_f64;
    let mut residual_skip_silicon = false;
    let mut domains_acc: Vec<String> = domains
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let mut b10x = b10x_prior;
    let mut final_status = status.clone();

    // Detect runtime conflict signals for conflict_rules
    let claim = fo
        .get("engine_claim")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let obs = fo
        .get("engine_obs")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut active_rules: Vec<&str> = Vec::new();
    if !claim.is_empty()
        && !obs.is_empty()
        && claim != obs
        && claim != "unknown"
        && obs != "unknown"
    {
        // Exception: iOS brand chrome/firefox/edge with webkit obs is expected — not conflict
        if !(os == "ios" && eng == "webkit") {
            active_rules.push("engine_claim_ne_obs");
        }
    }
    let ua = fo
        .get("user_agent")
        .or_else(|| fo.get("ua"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let platform = fo
        .get("platform")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let ua_win = ua.contains("windows");
    let ua_mac = ua.contains("mac os") || ua.contains("macintosh");
    let ua_linux = ua.contains("linux") || ua.contains("x11");
    let plat_win = platform.contains("win");
    let plat_mac = platform.contains("mac");
    let plat_linux = platform.contains("linux") || platform.contains("x11");
    if (ua_win && (plat_mac || plat_linux))
        || (ua_mac && (plat_win || plat_linux))
        || (ua_linux && (plat_win || plat_mac))
    {
        active_rules.push("ua_platform_os_mismatch");
    }
    let mobile_ua = ua.contains("mobile") || ua.contains("android") || ua.contains("iphone");
    if form == "desktop" && mobile_ua {
        active_rules.push("form_desktop_mobile_ua");
    }
    if form == "mobile" && !mobile_ua && ua.contains("windows nt") {
        active_rules.push("form_mobile_desktop_ua");
    }
    if os == "ios" && eng == "blink" {
        active_rules.push("ios_true_blink_claim");
    }
    let soft = fo
        .get("residual_soft_like")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || fo
            .get("webgl_unmasked_renderer")
            .and_then(|v| v.as_str())
            .map(|r| {
                let r = r.to_ascii_lowercase();
                r.contains("swiftshader") || r.contains("llvmpipe") || r.contains("softpipe")
            })
            .unwrap_or(false);
    if soft && form == "desktop" {
        active_rules.push("soft_gl_desktop");
    }

    if let Some(rules) = lib.get("conflict_rules").and_then(|v| v.as_array()) {
        for r in rules {
            let id = r.get("id").and_then(|v| v.as_str()).unwrap_or("");
            if !active_rules.iter().any(|a| *a == id) {
                continue;
            }
            conflict_hits.push(r.clone());
            if let Some(w) = r.get("anomaly_weight").and_then(|v| v.as_f64()) {
                anomaly_score = (anomaly_score + w).min(0.85);
            }
            if let Some(st) = r.get("status").and_then(|v| v.as_str()) {
                // escalate status severity
                if st == "impossible"
                    || st == "conflict"
                    || (st == "migration_suspect" && final_status != "impossible")
                {
                    final_status = st.to_string();
                }
            }
            if r.get("b10x_prior").and_then(|v| v.as_bool()) == Some(true) {
                b10x = true;
            }
            if r.get("residual_skip_silicon")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                residual_skip_silicon = true;
            }
            if let Some(ds) = r.get("domains").and_then(|v| v.as_array()) {
                for d in ds {
                    if let Some(s) = d.as_str() {
                        if !domains_acc.iter().any(|x| x == s) {
                            domains_acc.push(s.to_string());
                        }
                    }
                }
            }
        }
    }

    // Status → anomaly from tuple alone
    let thr = lib.get("thresholds").cloned().unwrap_or(json!({}));
    let anomaly_statuses = thr
        .get("anomaly_if_status")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let channel_statuses = thr
        .get("channel_prior_if_status")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let status_is_anomaly = anomaly_statuses
        .iter()
        .any(|s| s.as_str() == Some(final_status.as_str()));
    if status_is_anomaly && anomaly_score < 0.25 {
        anomaly_score = 0.35;
    }

    // rare_native / channel → B10x prior (unless skip silicon)
    let status_channel_prior = channel_statuses
        .iter()
        .any(|s| s.as_str() == Some(final_status.as_str()))
        || residual_channel == "unconventional"
        || b10x_prior;
    if status_channel_prior && !residual_skip_silicon {
        b10x = true;
    }
    // migration_suspect still often wants residual deepen (measurement) + authenticity
    if final_status == "migration_suspect" || final_status == "impossible" {
        b10x = b10x || residual_channel == "unconventional";
        if !domains_acc.iter().any(|d| d == "browser_kernel") {
            domains_acc.insert(0, "browser_kernel".into());
        }
    }

    let stack_anomaly = status_is_anomaly || anomaly_score >= 0.25;
    let channel_unconventional =
        residual_channel == "unconventional" || b10x || eng == "webkit" || eng == "unknown";

    // Rarity score 0..1 (lower rate → higher rarity)
    let rarity = (1.0 - (rate * 20.0).min(1.0)).clamp(0.0, 1.0);

    json!({
        "algo": "stack_cooccurrence_prior_v1",
        "loaded": true,
        "dims": dims,
        "lookup_key": key,
        "matched_tuple": matched_key,
        "status": final_status,
        "rate": rate,
        "rarity": (rarity * 1000.0).round() / 1000.0,
        "residual_channel": residual_channel,
        "b10x_prior": b10x && !residual_skip_silicon,
        "residual_skip_silicon": residual_skip_silicon,
        "stack_anomaly": stack_anomaly,
        "channel_unconventional": channel_unconventional,
        "anomaly_score": (anomaly_score * 1000.0).round() / 1000.0,
        "preferred_domains": domains_acc,
        "conflict_hits": conflict_hits.iter().map(|c| c.get("id").cloned().unwrap_or(Value::Null)).collect::<Vec<_>>(),
        "tuple_note": tuple_note,
        "policy": "sample_library_judgment",
        "never_target_peer_dh": true,
        "note": "Epiphany/WebKitGTK on Linux = rare_native channel (B10x), not Safari migration; Safari brand on Linux = migration_suspect",
    })
}

pub fn stack_cooccurrence_json() -> Value {
    load_stack_cooccurrence_prior()
        .cloned()
        .unwrap_or_else(|_| json!({"loaded": false}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lab_epiphany_linux_rare_native_b10x() {
        let fields = json!({
            "form_class": "desktop",
            "os_family": "linux",
            "engine_family": "webkit",
            "residual_probe_engine": "webkit",
            "browser_name": "Epiphany",
            "platform": "Linux x86_64",
            "user_agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 Epiphany/46.5",
        });
        let r = lookup_stack_cooccurrence(&fields);
        assert_eq!(r["status"], json!("rare_native"));
        assert_eq!(r["b10x_prior"], json!(true));
        assert_eq!(r["stack_anomaly"], json!(false));
        assert_eq!(r["channel_unconventional"], json!(true));
        assert_eq!(r["dims"]["browser_shell"], json!("epiphany"));
    }

    #[test]
    fn safari_brand_linux_migration_suspect() {
        let fields = json!({
            "form_class": "desktop",
            "os_family": "linux",
            "engine_family": "webkit",
            "browser_name": "Safari",
            "platform": "Linux x86_64",
            "user_agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 Version/17.0 Safari/605.1.15",
        });
        let r = lookup_stack_cooccurrence(&fields);
        assert_eq!(r["status"], json!("migration_suspect"));
        assert_eq!(r["b10x_prior"], json!(true));
        assert_eq!(r["stack_anomaly"], json!(true));
    }

    #[test]
    fn linux_chrome_common_no_b10x_prior() {
        let fields = json!({
            "form_class": "desktop",
            "os_family": "linux",
            "engine_family": "blink",
            "browser_name": "Chrome",
            "platform": "Linux x86_64",
            "user_agent": "Mozilla/5.0 (X11; Linux x86_64) Chrome/120.0.0.0",
        });
        let r = lookup_stack_cooccurrence(&fields);
        assert_eq!(r["status"], json!("common"));
        assert_eq!(r["b10x_prior"], json!(false));
    }

    #[test]
    fn ios_chrome_webkit_common_not_migration() {
        let fields = json!({
            "form_class": "mobile",
            "os_family": "ios",
            "engine_family": "webkit",
            "engine_claim": "blink",
            "engine_obs": "webkit",
            "browser_name": "Chrome",
            "user_agent": "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) CriOS/120.0.0.0 Mobile/15E148",
        });
        let r = lookup_stack_cooccurrence(&fields);
        assert_eq!(r["status"], json!("common"));
        assert_eq!(r["b10x_prior"], json!(true));
        // claim blink vs obs webkit on iOS is expected — not forced conflict
        assert_eq!(r["stack_anomaly"], json!(false));
    }

    #[test]
    fn android_chrome_common() {
        let fields = json!({
            "form_class": "mobile",
            "os_family": "android",
            "engine_family": "blink",
            "browser_name": "Chrome",
            "user_agent": "Mozilla/5.0 (Linux; Android 14) Chrome/120.0.0.0 Mobile",
        });
        let r = lookup_stack_cooccurrence(&fields);
        assert_eq!(r["status"], json!("common"));
        assert_eq!(r["b10x_prior"], json!(false));
    }
}
