//! Engine-aware client surface + multi-precision residual analysis + probe coverage gaps.
//!
//! See `docs/ARCH_ENGINE_AWARE_PROBE_ANALYSIS_V1.md`.
//!
//! - Commercial mint floor remains 0.001 (`webgl_comm_v4`); finer ladders only affect
//!   confidence / diagnostics / bucket priority.
//! - Never force-merge distinct measurement channels into one commercial `dh_`.

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

fn str_of(m: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(s) = m.get(*k).and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            return Some(s.to_string());
        }
    }
    None
}

fn f64_of(m: &Map<String, Value>, key: &str) -> Option<f64> {
    m.get(key).and_then(|v| {
        v.as_f64()
            .or_else(|| v.as_i64().map(|i| i as f64))
            .or_else(|| v.as_u64().map(|u| u as f64))
    })
}

fn nested_str(m: &Map<String, Value>, parent: &str, key: &str) -> Option<String> {
    m.get(parent)
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Parse a simple browser name/version from UA (best-effort; not a brand gate).
pub fn parse_browser_from_ua(ua: &str) -> (Option<String>, Option<String>) {
    let u = ua;
    let lower = ua.to_ascii_lowercase();
    // Order matters: Epiphany/GNOME Web often contain AppleWebKit + Safari tokens.
    if lower.contains("epiphany") || lower.contains("gnome web") {
        let ver = capture_ver(u, &["Epiphany/", "Version/"]);
        return (Some("Epiphany".into()), ver);
    }
    if lower.contains("firefox/") && !lower.contains("seamonkey") {
        return (Some("Firefox".into()), capture_ver(u, &["Firefox/"]));
    }
    if lower.contains("edg/") {
        return (Some("Edge".into()), capture_ver(u, &["Edg/", "Edge/"]));
    }
    if lower.contains("opr/") || lower.contains("opera") {
        return (Some("Opera".into()), capture_ver(u, &["OPR/", "Opera/"]));
    }
    if lower.contains("chrome/") && !lower.contains("edg/") {
        return (Some("Chrome".into()), capture_ver(u, &["Chrome/", "CriOS/"]));
    }
    if lower.contains("safari/") && lower.contains("version/") && !lower.contains("chrome") {
        return (Some("Safari".into()), capture_ver(u, &["Version/"]));
    }
    if lower.contains("applewebkit") {
        return (Some("WebKit".into()), capture_ver(u, &["Version/", "AppleWebKit/"]));
    }
    (None, None)
}

fn capture_ver(ua: &str, markers: &[&str]) -> Option<String> {
    for m in markers {
        if let Some(i) = ua.find(m) {
            let rest = &ua[i + m.len()..];
            let ver: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '_' || *c == '-')
                .collect();
            if !ver.is_empty() {
                return Some(ver);
            }
        }
    }
    None
}

/// Best-effort OS name/version from platform / UA-CH / UA.
pub fn parse_os_from_fields(fo: &Map<String, Value>) -> (Option<String>, Option<String>, Option<String>) {
    let family = str_of(fo, &["os_family", "ua_ch_platform", "ua_platform"]);
    let platform = str_of(fo, &["platform", "navigator_platform"]);
    let ua = str_of(fo, &["user_agent", "ua"]).unwrap_or_default();
    let lower = ua.to_ascii_lowercase();

    let mut name: Option<String> = None;
    let mut version: Option<String> = None;

    if let Some(v) = str_of(fo, &["ua_ch_platform_version", "platform_version", "os_version"]) {
        version = Some(v);
    }
    if let Some(p) = str_of(fo, &["ua_ch_platform", "ua_platform"]) {
        name = Some(p);
    }

    if name.is_none() {
        if lower.contains("android") {
            name = Some("Android".into());
            version = version.or_else(|| capture_ver(&ua, &["Android "]));
        } else if lower.contains("iphone") || lower.contains("ipad") || lower.contains("cpu os") {
            name = Some("iOS".into());
            version = version.or_else(|| {
                ua.replace('_', ".")
                    .find("OS ")
                    .map(|i| {
                        let rest = &ua.replace('_', ".")[i + 3..];
                        rest.chars()
                            .take_while(|c| c.is_ascii_digit() || *c == '.')
                            .collect::<String>()
                    })
                    .filter(|s| !s.is_empty())
            });
        } else if lower.contains("mac os x") || lower.contains("macintosh") {
            name = Some("macOS".into());
            version = version.or_else(|| capture_ver(&ua, &["Mac OS X ", "Intel Mac OS X "]));
        } else if lower.contains("windows") {
            name = Some("Windows".into());
            version = version.or_else(|| {
                if lower.contains("windows nt 10") {
                    Some("10/11".into())
                } else if lower.contains("windows nt 6.3") {
                    Some("8.1".into())
                } else if lower.contains("windows nt 6.1") {
                    Some("7".into())
                } else {
                    capture_ver(&ua, &["Windows NT "])
                }
            });
        } else if lower.contains("ubuntu") {
            name = Some("Ubuntu".into());
        } else if lower.contains("linux") || platform.as_deref().is_some_and(|p| p.to_ascii_lowercase().contains("linux")) {
            name = Some("Linux".into());
        }
    }

    let family = family.or_else(|| {
        name.as_ref().map(|n| {
            let l = n.to_ascii_lowercase();
            if l.contains("win") {
                "windows".into()
            } else if l.contains("mac") || l.contains("ios") {
                if l.contains("ios") {
                    "ios".into()
                } else {
                    "macos".into()
                }
            } else if l.contains("android") {
                "android".into()
            } else {
                "linux".into()
            }
        })
    });

    (name, version, family)
}

pub fn classify_gl_stack(fo: &Map<String, Value>) -> String {
    let vendor = str_of(
        fo,
        &[
            "webgl_unmasked_vendor",
            "unmasked_vendor",
            "webgl_vendor",
        ],
    )
    .unwrap_or_default()
    .to_ascii_lowercase();
    let renderer = str_of(
        fo,
        &[
            "webgl_unmasked_renderer",
            "unmasked_renderer",
            "webgl_renderer",
        ],
    )
    .unwrap_or_default()
    .to_ascii_lowercase();
    let engine = str_of(fo, &["residual_probe_engine", "engine_family", "engine_obs", "engine_claim"])
        .unwrap_or_default()
        .to_ascii_lowercase();

    if vendor.contains("google") || renderer.contains("angle") {
        return "angle".into();
    }
    if vendor.contains("apple") || engine == "webkit" {
        if renderer.is_empty() {
            return "webkit_unmask_blocked".into();
        }
        return "webkit_or_apple".into();
    }
    if renderer.contains("llvmpipe") || renderer.contains("swiftshader") || renderer.contains("softpipe") {
        return "software_gl".into();
    }
    if vendor.contains("nvidia") || vendor.contains("amd") || vendor.contains("intel") {
        return "native_desktop_gl".into();
    }
    "unknown".into()
}

/// Verified CF projection only (no client-claimed cloudflare alone).
fn project_cf(fo: &Map<String, Value>, evidence: Option<&Value>) -> Value {
    let mut out = Map::new();
    let cf = fo
        .get("cf_fields")
        .and_then(|v| v.as_object())
        .or_else(|| {
            evidence
                .and_then(|e| e.get("cf_fields"))
                .and_then(|v| v.as_object())
        })
        .or_else(|| {
            evidence
                .and_then(|e| e.pointer("/gateway_fields/cf_fields"))
                .and_then(|v| v.as_object())
        });
    let keys = [
        "cf_ray",
        "ray",
        "country",
        "colo",
        "bot_score",
        "bot_management_score",
        "ja3_hash",
        "ja4",
        "http_protocol",
        "tls_version",
        "asn",
        "as_organization",
        "city",
        "continent",
        "latitude",
        "longitude",
        "postal_code",
        "region",
        "timezone",
    ];
    if let Some(c) = cf {
        for k in keys {
            if let Some(v) = c.get(k) {
                if !v.is_null() && v.as_str() != Some("") {
                    out.insert(k.to_string(), v.clone());
                }
            }
        }
    }
    // Flat aliases sometimes present on fields
    for k in ["cf_ray", "cf_country", "cf_bot_score", "cf_colo"] {
        if !out.contains_key(k.trim_start_matches("cf_")) {
            if let Some(v) = fo.get(k) {
                if !v.is_null() {
                    let nk = k.trim_start_matches("cf_");
                    out.entry(nk.to_string()).or_insert(v.clone());
                }
            }
        }
    }
    if out.is_empty() {
        json!(null)
    } else {
        Value::Object(out)
    }
}

/// User-visible environment context (OS / browser / IP / CF / GL). Not a digest key.
pub fn project_client_env_context(fields: &Value, evidence: Option<&Value>) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let ua = str_of(&fo, &["user_agent", "ua"]).unwrap_or_default();
    let (br_name_ua, br_ver_ua) = parse_browser_from_ua(&ua);
    let (os_name, os_version, os_family) = parse_os_from_fields(&fo);

    let browser_name = str_of(&fo, &["browser_name", "br_name"])
        .or(br_name_ua)
        .or_else(|| nested_str(&fo, "ua_ch_brands", "brand"));
    let browser_version = str_of(&fo, &["browser_version", "br_version", "ua_full_version"])
        .or(br_ver_ua)
        .or_else(|| str_of(&fo, &["ua_ch_full_version"]));

    let engine_family = str_of(
        &fo,
        &[
            "residual_probe_engine",
            "engine_family",
            "engine_obs",
            "engine_claim",
            "protocol_engine",
        ],
    );
    let engine_version = str_of(&fo, &["engine_version", "webkit_version"])
        .or_else(|| capture_ver(&ua, &["AppleWebKit/", "Gecko/", "Chrome/"]));

    let client_ip = str_of(&fo, &["server_client_ip", "client_ip"]).or_else(|| {
        evidence
            .and_then(|e| e.get("server_client_ip"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                evidence
                    .and_then(|e| e.pointer("/gateway_fields/server_client_ip"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
    });
    let client_ip_source = str_of(&fo, &["server_client_ip_source", "client_ip_source"]).or_else(|| {
        evidence
            .and_then(|e| e.get("server_client_ip_source"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    });

    let gl_vendor = str_of(&fo, &["webgl_unmasked_vendor", "webgl_vendor"]);
    let gl_renderer = str_of(&fo, &["webgl_unmasked_renderer", "webgl_renderer"]);
    let unmask_blocked = gl_vendor
        .as_ref()
        .map(|v| v.to_ascii_lowercase().contains("apple"))
        .unwrap_or(false)
        && gl_renderer.as_ref().map(|s| s.is_empty()).unwrap_or(true);

    let gl_stack = classify_gl_stack(&fo);

    json!({
        "algo": "client_env_context_v1",
        "os_name": os_name,
        "os_version": os_version,
        "os_family": os_family,
        "platform": str_of(&fo, &["platform", "navigator_platform"]),
        "browser_name": browser_name,
        "browser_version": browser_version,
        "engine_family": engine_family,
        "engine_version": engine_version,
        "engine_claim": str_of(&fo, &["engine_claim"]),
        "engine_obs": str_of(&fo, &["engine_obs"]),
        "user_agent": if ua.is_empty() { Value::Null } else { json!(ua) },
        "client_ip": client_ip,
        "client_ip_source": client_ip_source,
        "server_asn": str_of(&fo, &["server_asn", "asn"]),
        "server_country": str_of(&fo, &["server_country", "country"]),
        "cf": project_cf(&fo, evidence),
        "gl": {
            "stack_class": gl_stack,
            "unmasked_vendor": gl_vendor,
            "unmasked_renderer": gl_renderer,
            "context_type": str_of(&fo, &["webgl_context_type"]),
            "extensions_count": fo.get("webgl_extensions_count").cloned().unwrap_or(Value::Null),
            "unmask_blocked": unmask_blocked,
        },
        "timezone": str_of(&fo, &["timezone"]),
        "form_class": str_of(&fo, &["form_class"]),
        "note": "Identity context for operators/users — not commercial digest material",
    })
}

/// Pointwise multi-precision agreement between two residual curves.
pub fn residual_multi_precision_match(a: &[f64], b: &[f64]) -> Value {
    let n = a.len().min(b.len());
    if n == 0 {
        return json!({
            "algo": "residual_multi_precision_v1",
            "n": 0,
            "ok": false,
        });
    }
    let mut agree_001 = 0usize;
    let mut agree_0001 = 0usize;
    let mut agree_00001 = 0usize;
    let mut maxabs = 0.0_f64;
    let mut sumabs = 0.0_f64;
    let mut sumsq = 0.0_f64;
    for i in 0..n {
        let d = (a[i] - b[i]).abs();
        maxabs = maxabs.max(d);
        sumabs += d;
        sumsq += d * d;
        if d <= 0.001 + 1e-15 {
            agree_001 += 1;
        }
        if d <= 0.0001 + 1e-15 {
            agree_0001 += 1;
        }
        if d <= 0.00001 + 1e-15 {
            agree_00001 += 1;
        }
    }
    let nf = n as f64;
    let ratio_001 = agree_001 as f64 / nf;
    let ratio_0001 = agree_0001 as f64 / nf;
    let ratio_00001 = agree_00001 as f64 / nf;
    let rms = (sumsq / nf).sqrt();

    // Bucket priority (analysis only — does not rewrite commercial digest).
    let bucket_priority = if ratio_00001 >= 0.8 {
        "L2_near_bit_stable"
    } else if ratio_0001 >= 0.8 {
        "L1_high_precision_majority"
    } else if ratio_001 >= 0.75 {
        "L0_commercial_floor_majority"
    } else if ratio_001 >= 0.5 {
        "partial_measurement_overlap"
    } else {
        "measurement_channel_split"
    };

    // Conf boost suggestion in [0, 0.15]
    let conf_boost = (ratio_0001 * 0.08 + ratio_00001 * 0.05 + (ratio_001 - 0.5).max(0.0) * 0.04)
        .clamp(0.0, 0.15);

    json!({
        "algo": "residual_multi_precision_v1",
        "n": n,
        "ok": true,
        "point_agree_0p001": (ratio_001 * 1000.0).round() / 1000.0,
        "point_agree_0p0001": (ratio_0001 * 1000.0).round() / 1000.0,
        "point_agree_0p00001": (ratio_00001 * 1000.0).round() / 1000.0,
        "agree_n_0p001": agree_001,
        "agree_n_0p0001": agree_0001,
        "agree_n_0p00001": agree_00001,
        "maxabs": (maxabs * 1e6).round() / 1e6,
        "meanabs": (sumabs / nf * 1e6).round() / 1e6,
        "rms": (rms * 1e6).round() / 1e6,
        "bucket_priority": bucket_priority,
        "conf_boost_hint": (conf_boost * 1000.0).round() / 1000.0,
        "commercial_floor_quanta": 0.001,
        "note": "Finer than 0.001 raises confidence only; commercial mint stays webgl_comm_v4 @ 0.001",
    })
}

fn path_curve(entry: &Value) -> Vec<f64> {
    entry
        .get("curve")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_f64().or_else(|| x.as_i64().map(|i| i as f64)))
                .collect()
        })
        .unwrap_or_default()
}

/// Multi-path precision report vs chosen curve (and pairwise summary).
pub fn residual_paths_precision_report(fo: &Map<String, Value>) -> Value {
    let paths = match fo.get("residual_paths").and_then(|v| v.as_array()) {
        Some(a) if !a.is_empty() => a,
        _ => {
            return json!({
                "algo": "residual_paths_precision_v1",
                "n_paths": 0,
                "ok": false,
            });
        }
    };
    let chosen_id = fo
        .get("residual_select")
        .and_then(|v| v.get("chosen_path_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let chosen = paths
        .iter()
        .find(|p| p.get("path_id").and_then(|v| v.as_str()) == Some(chosen_id))
        .or_else(|| paths.iter().find(|p| p.get("ok").and_then(|v| v.as_bool()) == Some(true)))
        .cloned();
    let Some(chosen) = chosen else {
        return json!({
            "algo": "residual_paths_precision_v1",
            "n_paths": paths.len(),
            "ok": false,
        });
    };
    let c0 = path_curve(&chosen);
    let mean0 = chosen
        .get("mean")
        .and_then(|v| v.as_f64())
        .unwrap_or_else(|| {
            if c0.is_empty() {
                0.0
            } else {
                c0.iter().sum::<f64>() / c0.len() as f64
            }
        });
    let std0 = chosen.get("std").and_then(|v| v.as_f64()).unwrap_or(0.0);
    // abs-mag std @0.001 (commercial component)
    let mut mags: Vec<f64> = c0.iter().map(|v| v.abs()).collect();
    mags.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let mag_mean = if mags.is_empty() {
        0.0
    } else {
        mags.iter().sum::<f64>() / mags.len() as f64
    };
    let mag_std = if mags.is_empty() {
        0.0
    } else {
        let v = mags.iter().map(|x| (x - mag_mean) * (x - mag_mean)).sum::<f64>() / mags.len() as f64;
        v.sqrt()
    };

    let mut pairs = Vec::new();
    for p in paths {
        let pid = p.get("path_id").and_then(|v| v.as_str()).unwrap_or("");
        if pid == chosen.get("path_id").and_then(|v| v.as_str()).unwrap_or("") {
            continue;
        }
        if p.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            continue;
        }
        let c2 = path_curve(p);
        if c2.len() < 4 {
            continue;
        }
        let mut m = residual_multi_precision_match(&c0, &c2);
        if let Some(obj) = m.as_object_mut() {
            obj.insert("path_id".into(), json!(pid));
            obj.insert(
                "mean_delta".into(),
                json!({
                    "abs": ((mean0 - p.get("mean").and_then(|v| v.as_f64()).unwrap_or(0.0)).abs() * 1e9).round() / 1e9
                }),
            );
        }
        pairs.push(m);
    }

    json!({
        "algo": "residual_paths_precision_v1",
        "ok": true,
        "n_paths": paths.len(),
        "chosen_path_id": chosen.get("path_id").cloned().unwrap_or(Value::Null),
        "chosen_mean": mean0,
        "chosen_std": std0,
        "commercial_components": {
            "mu_0p001": format!("{:.3}", mag_mean),
            "sd_0p001": format!("{:.3}", mag_std),
            "floor": 0.001,
        },
        "pair_precision": pairs,
        "note": "pair_precision is analysis/conf; mint uses webgl_comm_v4 on chosen curve only",
    })
}

/// Residual deepen pack candidates when measurement contract is incomplete.
///
/// Product principle (v5.8.84):
/// - Do **not** skip B packs based on claimed OS/BR/engine — schedule full B10x set.
/// - **Order** by priority: silicon first, then engine/stack-preferred, then rest.
/// - Execution pressure is handled by FE single-GL + HW serial + per-tick cap,
///   not by dropping packs from the schedule.
pub fn specialized_probe_components_for(engine_family: &str, gl_stack: &str) -> Vec<&'static str> {
    let eng = engine_family.to_ascii_lowercase();
    let gl = gl_stack.to_ascii_lowercase();
    let mut out: Vec<&'static str> = Vec::new();
    let mut push = |id: &'static str| {
        if !out.iter().any(|x| *x == id) {
            out.push(id);
        }
    };

    // Silicon multipath floor — all engines.
    push("B10x_silicon_ulp");
    push("B10x_silicon_rint");
    push("B10x_silicon_noderiv");
    // iss/54 P1–P4 advanced silicon (fma/denorm/tex/interp) — after core trio
    push("B10x_silicon_deep");

    // Engine/stack-preferred next (priority only — not exclusive).
    if eng == "webkit" || gl.contains("webkit") {
        push("B10x_webkit_gl_noise");
        push("B10x_webkit_wave2_warm4");
    }
    if eng == "blink" || gl.contains("angle") || gl.contains("google") {
        push("B10x_angle_crosscheck");
    }
    if eng == "gecko" {
        push("B10x_angle_crosscheck");
        push("B10x_legacy_webgl1");
    }
    if gl.contains("software") || gl.contains("swiftshader") || gl.contains("llvmpipe") {
        push("B10x_softgl_hedge");
    }
    if eng == "unknown" || eng.is_empty() {
        push("B10x_unknown_kernel");
    }

    // Full residual set — every engine eventually runs all (device-id data completeness).
    push("B10x_angle_crosscheck");
    push("B10x_legacy_webgl1");
    push("B10x_unknown_kernel");
    push("B10x_softgl_hedge");
    push("B10x_webkit_gl_noise");
    push("B10x_webkit_wave2_warm4");

    out
}

/// Count how many B10x residual-deepen packs already landed this session.
pub fn count_b10x_ticks(present_packs: &[String]) -> usize {
    present_packs
        .iter()
        .filter(|p| p.starts_with("B10x_"))
        .count()
}

/// Max residual deepen packs after B10 this session.
/// Open schedule: allow full B10x catalog to land for cross-path comparison
/// (not gated by measurement_complete). Band no longer shrinks this.
pub fn max_residual_deepen_ticks(policy_band: Option<&str>) -> usize {
    let _ = policy_band;
    // Full B10x catalog room including silicon_ulp/rint/noderiv research packs.
    9
}

/// **This-session** residual measurement completeness (NOT cross-browser same-machine).
///
/// Brain uses this to decide whether B10x is worth scheduling. Never encodes a target
/// device_id or peer mean cluster. See `brain-residual-measurement-schedule.md`.
pub fn residual_measurement_status(
    fields: &Value,
    present_packs: &[String],
    policy_band: Option<&str>,
) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let mut filled: Vec<&str> = Vec::new();
    let mut missing: Vec<&str> = Vec::new();

    // Primary B10 pack only — do not treat B10x or residual_paths-alone as B10 present.
    let b10_present = present_packs
        .iter()
        .any(|p| *p == "B10_hw_curves" || *p == "mid.curves");
    if b10_present {
        filled.push("b10_present");
    } else {
        missing.push("b10_present");
    }

    let paths = fo
        .get("residual_paths")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut ok_paths = 0usize;
    let mut mean_buckets: std::collections::HashMap<i64, i32> = std::collections::HashMap::new();
    for p in &paths {
        let ok = p.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        let curve_n = p
            .get("curve")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let entropy = p
            .get("entropy_ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(curve_n >= 8);
        if !ok || curve_n < 8 || !entropy {
            continue;
        }
        ok_paths += 1;
        let mean = p
            .get("mean")
            .and_then(|v| v.as_f64())
            .or_else(|| {
                p.get("curve").and_then(|v| v.as_array()).map(|a| {
                    let xs: Vec<f64> = a
                        .iter()
                        .filter_map(|x| x.as_f64().or_else(|| x.as_i64().map(|i| i as f64)))
                        .collect();
                    if xs.is_empty() {
                        0.0
                    } else {
                        xs.iter().sum::<f64>() / xs.len() as f64
                    }
                })
            })
            .unwrap_or(0.0);
        // Only commercial-ish residual band
        if !(0.15..=0.4).contains(&mean) {
            continue;
        }
        let b = (mean * 1000.0).round() as i64;
        *mean_buckets.entry(b).or_default() += 1;
    }

    let paths_ge_3 = ok_paths >= 3;
    if paths_ge_3 {
        filled.push("paths_ge_3");
    } else if b10_present {
        missing.push("paths_ge_3");
    }

    let (cluster_votes, cluster_bucket) = mean_buckets
        .iter()
        .max_by_key(|(_, n)| *n)
        .map(|(b, n)| (*n, *b))
        .unwrap_or((0, 0));
    let need_votes = ((ok_paths as f64) * 0.5).ceil() as i32;
    let need_votes = need_votes.max(2);
    let cluster_stable = ok_paths >= 2 && cluster_votes >= need_votes;
    if cluster_stable {
        filled.push("cluster_stable");
    } else if ok_paths >= 2 {
        missing.push("cluster_stable");
    }

    let entropy_ok = fo
        .get("webgl_residual_entropy_ok")
        .and_then(|v| v.as_bool())
        .or_else(|| fo.get("residual_ok").and_then(|v| v.as_bool()))
        .unwrap_or(ok_paths >= 1);
    if entropy_ok && b10_present {
        filled.push("entropy_ok");
    } else if b10_present {
        missing.push("entropy_ok");
    }

    let has_audio = fo.get("hw_curve_audio").is_some()
        || fo
            .get("hw_noise_curves")
            .and_then(|v| v.get("audio"))
            .is_some()
        || fo.get("hw_audio_stable").is_some()
        || fo.get("audio_noise_energy").is_some();
    let has_webgl = fo.get("hw_curve_webgl").is_some()
        || fo
            .get("hw_noise_curves")
            .and_then(|v| v.get("webgl"))
            .is_some()
        || fo.get("hw_webgl_stable").is_some()
        || ok_paths >= 1;
    let dual_hw = has_audio && has_webgl;
    if dual_hw {
        filled.push("dual_hw");
    } else {
        missing.push("dual_hw");
    }

    let soft_stack = fo
        .get("residual_soft_like")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || fo
            .get("stack_class")
            .and_then(|v| v.as_str())
            .is_some_and(|s| s.contains("soft"));

    let eng = str_of(
        &fo,
        &[
            "residual_probe_engine",
            "engine_family",
            "engine_obs",
            "engine_claim",
        ],
    )
    .unwrap_or_else(|| "unknown".into())
    .to_ascii_lowercase();
    let gl_stack = classify_gl_stack(&fo);

    // Completeness score 0..1 (equal weights on contract bits)
    let contract = [
        ("b10_present", b10_present),
        ("paths_ge_3", paths_ge_3),
        ("cluster_stable", cluster_stable),
        ("entropy_ok", entropy_ok && b10_present),
        ("dual_hw", dual_hw),
    ];
    let score = contract.iter().filter(|(_, ok)| *ok).count() as f64 / contract.len() as f64;
    let measurement_complete = b10_present && paths_ge_3 && cluster_stable && entropy_ok && dual_hw;

    let b10x_ticks = count_b10x_ticks(present_packs);
    let max_ticks = max_residual_deepen_ticks(policy_band);

    // Domain deepen decision (taxonomy: channel vs anomaly vs incomplete).
    // See probe_domain + v5-docs/architecture/probe-domain-deepen.md
    let deepen_dec = crate::probe_domain::residual_deepen_decision(
        fields,
        measurement_complete,
        b10_present,
        b10x_ticks,
        max_ticks,
        soft_stack,
    );
    let worth_deepen = deepen_dec
        .get("worth_deepen")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let deepen_reason = deepen_dec.get("deepen_reason").cloned().unwrap_or(Value::Null);
    let stack_profile = deepen_dec
        .get("stack_profile")
        .cloned()
        .unwrap_or(Value::Null);
    let channel_unconventional = stack_profile
        .get("channel_unconventional")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // Back-compat alias used by older notes / gap code
    let unconventional = channel_unconventional
        || eng == "webkit"
        || eng == "unknown"
        || gl_stack.contains("webkit");

    // Need B10 first
    let need_b10 = !b10_present;

    // Match progress for ops/brain notes: filled vs contract (not cross-browser hit rate).
    let match_progress = json!({
        "matched_n": filled.len(),
        "contract_n": contract.len(),
        "score": (score * 1000.0).round() / 1000.0,
        "ok_paths": ok_paths,
        "cluster_votes": cluster_votes,
        "cluster_need_votes": need_votes,
        "gap_to_stable": if cluster_stable { 0 } else { (need_votes - cluster_votes).max(0) },
        "filled": filled,
        "missing": missing,
    });

    json!({
        "algo": "residual_measurement_status_v5",
        "domain": "device_residual",
        "policy": "always_open_b10x_after_b10",
        "force_merge": false,
        "never_target_peer_dh": true,
        "schedule_gate_forgeable_os_br": false,
        "schedule_gate_measurement_complete": false,
        "measurement_complete": measurement_complete,
        "score": (score * 1000.0).round() / 1000.0,
        "match_progress": match_progress,
        "filled": filled,
        "missing": missing,
        "ok_paths": ok_paths,
        "paths_total": paths.len(),
        "cluster_votes": cluster_votes,
        "cluster_need_votes": need_votes,
        "cluster_mean_bucket_0p001": if cluster_votes > 0 {
            json!(format!("{:.3}", cluster_bucket as f64 / 1000.0))
        } else {
            Value::Null
        },
        "dual_hw": dual_hw,
        "entropy_ok": entropy_ok,
        "soft_stack": soft_stack,
        "engine_family": eng,
        "gl_stack_class": gl_stack,
        "channel_unconventional": channel_unconventional,
        "unconventional_kernel": unconventional,
        "stack_profile": stack_profile,
        "b10x_ticks": b10x_ticks,
        "max_deepen_ticks": max_ticks,
        "worth_deepen": worth_deepen,
        "deepen_reason": deepen_reason,
        "need_b10": need_b10,
        "deepen_components": specialized_probe_components_for(&eng, &gl_stack),
        "note": "Always-open residual: B10 then all B10x deepen packs (no complete/OS gate); multipath data for true-curve cross-browser research",
    })
}

/// Detect missing specialized probe components + residual measurement issues.
pub fn detect_probe_coverage_gap(fields: &Value, present_packs: &[String]) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let env = project_client_env_context(fields, None);
    let engine = env
        .get("engine_family")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let gl_stack = env
        .pointer("/gl/stack_class")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let needed = specialized_probe_components_for(engine, gl_stack);
    let present_set: std::collections::HashSet<&str> =
        present_packs.iter().map(|s| s.as_str()).collect();
    // Also treat residual_paths presence as base B10 coverage
    let has_b10 = present_set.contains("B10_hw_curves")
        || fo.get("residual_paths").and_then(|v| v.as_array()).is_some_and(|a| !a.is_empty())
        || fo.get("hw_curve_webgl").is_some();

    let mut missing: Vec<&str> = needed
        .iter()
        .copied()
        .filter(|p| !present_set.contains(*p))
        .collect();

    let mut issues: Vec<String> = Vec::new();
    if env
        .pointer("/gl/unmask_blocked")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        issues.push("webgl_unmask_blocked".into());
    }
    if !has_b10 {
        issues.push("b10_residual_missing".into());
        if !missing.contains(&"B10_hw_curves") {
            missing.push("B10_hw_curves");
        }
    }

    // Path coverage thin for webkit
    let n_paths = fo
        .get("residual_paths")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    if engine == "webkit" && n_paths > 0 && n_paths < 4 {
        issues.push("webkit_path_plan_thin".into());
    }

    // commercial sd fork risk when mean present
    if let Some(std) = f64_of(&fo, "residual_std") {
        // abs-mag sd not always present; residual_std is curve std
        let sd_q = format!("{:.3}", {
            // approximate from residual_std if mag-std not present
            std
        });
        let _ = sd_q;
    }
    // If residual_select has pair mean_delta near 0.001, mark drift
    if let Some(pairs) = fo
        .get("residual_select")
        .and_then(|v| v.get("pair_agreements"))
        .and_then(|v| v.as_array())
    {
        for p in pairs {
            if let Some(d) = p.get("mean_delta").and_then(|v| v.as_f64()) {
                if d >= 0.0005 && d <= 0.002 {
                    issues.push("residual_mean_near_0p001_fork".into());
                    break;
                }
            }
        }
    }

    let unconventional = engine == "webkit"
        || engine == "unknown"
        || gl_stack.contains("webkit")
        || gl_stack.contains("software")
        || env
            .get("engine_claim")
            .and_then(|v| v.as_str())
            .zip(env.get("engine_obs").and_then(|v| v.as_str()))
            .is_some_and(|(a, b)| a != b);

    let present_gap = unconventional && (!missing.is_empty() || !issues.is_empty());

    json!({
        "algo": "probe_coverage_gap_v1",
        "present": present_gap,
        "code": if present_gap {
            if !missing.is_empty() { "engine_specialized_probe_missing" } else { "engine_measurement_issues" }
        } else {
            "ok"
        },
        "unconventional_kernel": unconventional,
        "engine_family": engine,
        "gl_stack_class": gl_stack,
        "os_name": env.get("os_name").cloned().unwrap_or(Value::Null),
        "os_version": env.get("os_version").cloned().unwrap_or(Value::Null),
        "os_family": env.get("os_family").cloned().unwrap_or(Value::Null),
        "browser_name": env.get("browser_name").cloned().unwrap_or(Value::Null),
        "browser_version": env.get("browser_version").cloned().unwrap_or(Value::Null),
        "missing_components": missing,
        "observed_issues": issues,
        "residual_paths_n": n_paths,
        "has_b10_materials": has_b10,
        "note": "Report for probe catalog iteration — does not rewrite commercial digest",
    })
}

/// Commercial webgl_comm_v4 components from a curve (analysis mirror).
pub fn webgl_comm_components(curve: &[f64]) -> Value {
    if curve.len() < 4 {
        return json!({"ok": false});
    }
    let mut mags: Vec<f64> = curve.iter().map(|v| v.abs()).collect();
    mags.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let top8: Vec<i64> = mags.iter().take(8).map(|m| (*m / 0.02).round() as i64).collect();
    let n = mags.len() as f64;
    let mean = mags.iter().sum::<f64>() / n;
    let var = mags.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n;
    let std = var.sqrt();
    let mid: Vec<i64> = mags
        .iter()
        .skip(4)
        .take(8)
        .map(|m| (*m / 0.03).round() as i64)
        .collect();
    json!({
        "ok": true,
        "mu_0p001": format!("{:.3}", mean),
        "sd_0p001": format!("{:.3}", std),
        "top8_q02": top8,
        "mid_q03": mid,
        "mean_raw": mean,
        "std_raw": std,
    })
}

/// Near-host cluster analysis (opt-in soft signal — **never force-merges** commercial dh).
///
/// When magrank/mu match and only sd@0.001 differs by one quantum with high point agree
/// among multipath, mark `near_host_cluster` for conf boost / ops review.
pub fn near_host_cluster_analysis(fo: &Map<String, Value>) -> Value {
    let paths = fo
        .get("residual_paths")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let chosen_id = fo
        .get("residual_select")
        .and_then(|v| v.get("chosen_path_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let chosen_curve: Vec<f64> = paths
        .iter()
        .find(|p| p.get("path_id").and_then(|v| v.as_str()) == Some(chosen_id))
        .or_else(|| paths.iter().find(|p| p.get("ok").and_then(|v| v.as_bool()) == Some(true)))
        .map(|p| {
            p.get("curve")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_f64().or_else(|| x.as_i64().map(|i| i as f64)))
                        .collect()
                })
                .unwrap_or_default()
        })
        .unwrap_or_else(|| {
            fo.get("hw_curve_webgl")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_f64().or_else(|| x.as_i64().map(|i| i as f64)))
                        .collect()
                })
                .unwrap_or_default()
        });

    let comm = webgl_comm_components(&chosen_curve);
    let mu = comm.get("mu_0p001").and_then(|v| v.as_str()).unwrap_or("");
    let sd = comm.get("sd_0p001").and_then(|v| v.as_str()).unwrap_or("");
    let top8 = comm.get("top8_q02").cloned().unwrap_or(json!([]));

    // Within-session multipath: max point_agree_0p001 vs chosen
    let mut best_agree = 0.0_f64;
    let mut best_path = Value::Null;
    let mut pair_n = 0usize;
    for p in &paths {
        let pid = p.get("path_id").and_then(|v| v.as_str()).unwrap_or("");
        if pid == chosen_id {
            continue;
        }
        if p.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            continue;
        }
        let c2: Vec<f64> = p
            .get("curve")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_f64().or_else(|| x.as_i64().map(|i| i as f64)))
                    .collect()
            })
            .unwrap_or_default();
        if c2.len() < 8 {
            continue;
        }
        pair_n += 1;
        let m = residual_multi_precision_match(&chosen_curve, &c2);
        let a001 = m.get("point_agree_0p001").and_then(|v| v.as_f64()).unwrap_or(0.0);
        if a001 > best_agree {
            best_agree = a001;
            best_path = json!({
                "path_id": pid,
                "point_agree_0p001": a001,
                "point_agree_0p0001": m.get("point_agree_0p0001"),
                "bucket_priority": m.get("bucket_priority"),
            });
        }
    }

    // Heuristic: commercial near-fork risk when sd quantum is on boundary
    let std_raw = comm.get("std_raw").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let near_sd_boundary = (std_raw - 0.0905).abs() < 0.0015;

    let eng = str_of(
        fo,
        &[
            "residual_probe_engine",
            "engine_family",
            "engine_obs",
            "engine_claim",
        ],
    )
    .unwrap_or_else(|| "unknown".into());

    let is_near = (eng == "webkit" || eng == "unknown")
        && !mu.is_empty()
        && near_sd_boundary
        && (best_agree >= 0.5 || pair_n == 0);

    let conf_boost = if is_near {
        (0.04 + best_agree * 0.06).clamp(0.0, 0.12)
    } else if best_agree >= 0.8 {
        0.03
    } else {
        0.0
    };

    json!({
        "algo": "near_host_cluster_v1",
        "near_host_cluster": is_near,
        "force_merge": false,
        "policy": "soft_signal_only_never_rewrite_dh",
        "engine_family": eng,
        "commercial": comm,
        "mu_0p001": mu,
        "sd_0p001": sd,
        "top8_q02": top8,
        "near_sd_boundary": near_sd_boundary,
        "within_session_best_pair": best_path,
        "within_session_pair_n": pair_n,
        "conf_boost_hint": (conf_boost * 1000.0).round() / 1000.0,
        "note": "If magrank+mu agree and only sd@0.001 forks vs host ANGLE cluster, raise conf and schedule B10x — do not silent-merge device_id",
    })
}

/// Same-host precision matrix row (for KPI harness / ops).
pub fn precision_matrix_row(
    label: &str,
    engine: &str,
    device_id: &str,
    curve: &[f64],
    peer_curve: Option<&[f64]>,
) -> Value {
    let comm = webgl_comm_components(curve);
    let vs_peer = peer_curve
        .map(|p| residual_multi_precision_match(curve, p))
        .unwrap_or(json!(null));
    json!({
        "label": label,
        "engine_family": engine,
        "device_id": device_id,
        "commercial": comm,
        "vs_peer": vs_peer,
        "curve_n": curve.len(),
        "mean": if curve.is_empty() { Value::Null } else {
            json!(curve.iter().sum::<f64>() / curve.len() as f64)
        },
    })
}

/// Soft host cluster (ops / continuity only — **never rewrites commercial dh_**).
///
/// Key materials (all optional but residual required):
/// - residual_mean quantized @ 0.005 (xbr noise absorb; commercial floor remains 0.001 on wg_*)
/// - webrtc_host_ip_hash when present (strong host sep)
/// - form_class + cores bucket (SW conf, not mint body)
///
/// Empty residual / gateway-only → `eligible=false` (no cluster claim).
pub fn soft_host_cluster_from_fields(fields: &Value) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let residual_mean = fo
        .get("residual_mean")
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)));
    let residual_std = fo
        .get("residual_std")
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)));
    let webrtc = fo
        .get("webrtc_host_ip_hash")
        .or_else(|| fo.get("webrtc_host_ip_hash_v2"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let os_i = fo
        .get("os_instance_hash")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let form = fo
        .get("form_class")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let cores = fo
        .get("hardware_concurrency")
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
        .map(|c| {
            if c <= 2.0 {
                "c2"
            } else if c <= 4.0 {
                "c4"
            } else if c <= 8.0 {
                "c8"
            } else if c <= 16.0 {
                "c16"
            } else {
                "c32p"
            }
        })
        .unwrap_or("c_unk");

    let Some(rm) = residual_mean else {
        return json!({
            "algo": "soft_host_cluster_v1",
            "eligible": false,
            "force_merge": false,
            "policy": "soft_signal_only_never_rewrite_dh",
            "reason": "no_residual_mean",
        });
    };
    // XBR residual quanta 0.005 — absorb WebKit/ANGLE micro-drift for soft cluster only.
    let rm_q = (rm / 0.005).round() * 0.005;
    let rs_q = residual_std
        .map(|s| (s / 0.005).round() * 0.005)
        .unwrap_or(-1.0);
    let host = webrtc
        .clone()
        .or_else(|| os_i.clone())
        .unwrap_or_else(|| "host_unknown".into());
    let host_kind = if webrtc.is_some() {
        "webrtc"
    } else if os_i.is_some() {
        "os_instance"
    } else {
        "none"
    };
    let body = format!(
        "soft_host_v1|{rm_q:.5}|{rs_q:.5}|{host}|{form}|{cores}|{host_kind}"
    );
    let mut h = Sha256::new();
    h.update(body.as_bytes());
    let dig = format!("{:x}", h.finalize());
    let cluster_id = format!("shc_{}", &dig[..16]);
    // Confidence: residual alone is weak under CGNAT; host sep raises soft confidence.
    let conf = match host_kind {
        "webrtc" => 0.82,
        "os_instance" => 0.68,
        _ => 0.48,
    };
    json!({
        "algo": "soft_host_cluster_v1",
        "eligible": true,
        "force_merge": false,
        "policy": "soft_signal_only_never_rewrite_dh",
        "soft_host_cluster_id": cluster_id,
        "residual_mean_q005": rm_q,
        "residual_std_q005": if rs_q < 0.0 { Value::Null } else { json!(rs_q) },
        "host_kind": host_kind,
        "host_token_present": host_kind != "none",
        "form_class": if form.is_empty() { Value::Null } else { json!(form) },
        "cores_bucket": cores,
        "confidence": conf,
        "note": "Same shc_* across browsers = soft same-host continuity; public dh_ may still fork on wg_ floors",
    })
}

/// Attach env + residual_precision + probe_coverage + near_host onto a product object.
pub fn attach_engine_surface_to_product(
    product: &mut Value,
    fields: &Value,
    evidence: Option<&Value>,
    present_packs: &[String],
) {
    let env = project_client_env_context(fields, evidence);
    let fo = fields.as_object().cloned().unwrap_or_default();
    let precision = residual_paths_precision_report(&fo);
    let gap = detect_probe_coverage_gap(fields, present_packs);
    let near = near_host_cluster_analysis(&fo);
    let soft_host = soft_host_cluster_from_fields(fields);
    let measurement = residual_measurement_status(fields, present_packs, None);

    // Apply conf boost hint onto device_confidence if present (soft only).
    if let Some(boost) = near.get("conf_boost_hint").and_then(|v| v.as_f64()) {
        if boost > 0.0 {
            if let Some(obj) = product.as_object_mut() {
                if let Some(c) = obj.get("device_confidence").and_then(|v| v.as_f64()) {
                    obj.insert(
                        "device_confidence".into(),
                        json!((c + boost).clamp(0.0, 1.0)),
                    );
                    obj.insert("device_confidence_near_host_boost".into(), json!(boost));
                }
            }
        }
    }

    if let Some(obj) = product.as_object_mut() {
        let mut env_obj = env.as_object().cloned().unwrap_or_default();
        env_obj.insert("probe_coverage_gap".into(), gap.clone());
        env_obj.insert("near_host_cluster".into(), near.clone());
        env_obj.insert("soft_host_cluster".into(), soft_host.clone());
        obj.insert("env".into(), Value::Object(env_obj));
        obj.insert("residual_precision".into(), precision);
        obj.insert("probe_coverage_gap".into(), gap.clone());
        obj.insert("near_host_cluster".into(), near);
        obj.insert("soft_host_cluster".into(), soft_host.clone());
        if let Some(shc) = soft_host
            .get("soft_host_cluster_id")
            .and_then(|v| v.as_str())
        {
            obj.insert("soft_host_cluster_id".into(), json!(shc));
        }
        obj.insert("residual_measurement".into(), measurement.clone());
        obj.insert(
            "complete_probe".into(),
            json!({
                "policy": "always_open_b10x_after_b10",
                "schedule_gate_forgeable_os_br": false,
                "schedule_gate_measurement_complete": false,
                "static_anchors": [
                    "B11_interaction",
                    "B0_bootstrap",
                    "B12_anti_camouflage",
                    "B1_conflict",
                    "B3_system",
                    "B2_hardware",
                    "B10_hw_curves",
                    "B7_sandbox",
                    "B8_gateway_early"
                ],
                "residual_deepen_gate": "always_open_after_b10",
                "b10x_max_ticks": 6,
                "doc": "v5-docs/architecture/b-batch-complete-probe-schedule.md",
                "cross_engine_curve_research": "v5-docs/architecture/cross-engine-true-residual-research.md",
            }),
        );
        // Surface into unknown_bucket so existing ops hub can aggregate without new clients.
        if gap.get("present").and_then(|v| v.as_bool()).unwrap_or(false) {
            let mut codes: Vec<Value> = Vec::new();
            if let Some(c) = gap.get("code").and_then(|v| v.as_str()) {
                codes.push(json!(c));
            }
            if let Some(iss) = gap.get("observed_issues").and_then(|v| v.as_array()) {
                for i in iss {
                    if let Some(s) = i.as_str() {
                        codes.push(json!(format!("issue:{s}")));
                    }
                }
            }
            let mut tags = vec![json!("engine_probe_gap")];
            if let Some(eng) = gap.get("engine_family") {
                tags.push(eng.clone());
            }
            obj.insert(
                "unknown_bucket".into(),
                json!({
                    "present": true,
                    "code": gap.get("code").cloned().unwrap_or(json!("engine_specialized_probe_missing")),
                    "codes": codes,
                    "tags": tags,
                    "probe_coverage_gap": gap,
                    "reason": "unconventional_kernel_or_missing_b10x",
                    "out_of_envelope": true,
                }),
            );
        }
    }
}

/// Aggregate probe_coverage_gap across session products for ops hub.
pub fn aggregate_probe_coverage_gaps(sessions: &[Value]) -> Value {
    let mut by_code: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    let mut by_engine: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    let mut by_missing: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    let mut by_browser: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    let mut samples: Vec<Value> = Vec::new();
    let mut with_gap = 0u64;

    for s in sessions {
        let gap = s
            .get("probe_coverage_gap")
            .or_else(|| s.pointer("/product/probe_coverage_gap"))
            .or_else(|| s.pointer("/result/product/probe_coverage_gap"))
            .or_else(|| s.pointer("/env/probe_coverage_gap"))
            .cloned()
            .unwrap_or(Value::Null);
        if gap.is_null() {
            continue;
        }
        let present = gap.get("present").and_then(|v| v.as_bool()).unwrap_or(false);
        if !present {
            continue;
        }
        with_gap += 1;
        if let Some(code) = gap.get("code").and_then(|v| v.as_str()) {
            *by_code.entry(code.to_string()).or_default() += 1;
        }
        if let Some(eng) = gap.get("engine_family").and_then(|v| v.as_str()) {
            *by_engine.entry(eng.to_string()).or_default() += 1;
        }
        if let Some(bn) = gap.get("browser_name").and_then(|v| v.as_str()) {
            *by_browser.entry(bn.to_string()).or_default() += 1;
        }
        if let Some(arr) = gap.get("missing_components").and_then(|v| v.as_array()) {
            for m in arr {
                if let Some(ms) = m.as_str() {
                    *by_missing.entry(ms.to_string()).or_default() += 1;
                }
            }
        }
        if samples.len() < 32 {
            samples.push(json!({
                "session_id": s.get("session_id").cloned().unwrap_or(Value::Null),
                "device_id": s.get("device_id")
                    .or_else(|| s.pointer("/product/device_id"))
                    .cloned()
                    .unwrap_or(Value::Null),
                "gap": gap,
            }));
        }
    }

    fn top_map(m: std::collections::HashMap<String, u64>, key: &str) -> Vec<Value> {
        let mut v: Vec<(String, u64)> = m.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        v.into_iter()
            .take(40)
            .map(|(k, n)| json!({ key: k, "count": n }))
            .collect()
    }

    json!({
        "algo": "probe_coverage_gap_hub_v1",
        "sessions_scanned": sessions.len(),
        "sessions_with_gap": with_gap,
        "codes": top_map(by_code, "code"),
        "engines": top_map(by_engine, "engine"),
        "browsers": top_map(by_browser, "browser"),
        "missing_components": top_map(by_missing, "component"),
        "samples": samples,
        "note": "Hub for specialized probe catalog iteration — not auto commercial policy",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn residual_deepen_candidates_never_empty_for_any_engine() {
        for eng in ["blink", "gecko", "webkit", "unknown", ""] {
            let c = specialized_probe_components_for(eng, "angle");
            assert!(!c.is_empty(), "engine={eng}");
            assert!(c.iter().any(|p| p.starts_with("B10x_")));
        }
        let g = specialized_probe_components_for("gecko", "native");
        assert!(g.contains(&"B10x_angle_crosscheck") || g.contains(&"B10x_legacy_webgl1"));
    }

    #[test]
    fn measurement_status_incomplete_without_paths() {
        let fields = json!({
            "engine_family": "webkit",
            "webgl_unmasked_vendor": "Apple Inc.",
        });
        let m = residual_measurement_status(&fields, &[], None);
        assert_eq!(m["need_b10"], json!(true));
        assert_eq!(m["worth_deepen"], json!(false)); // no B10 yet
        assert_eq!(m["force_merge"], json!(false));
        assert_eq!(m["never_target_peer_dh"], json!(true));
    }

    #[test]
    fn measurement_status_worth_deepen_webkit_thin_paths() {
        let curve: Vec<Value> = (0..32)
            .map(|i| json!(0.22 + (i as f64) * 0.001))
            .collect();
        let fields = json!({
            "form_class": "desktop",
            "os_family": "linux",
            "engine_family": "webkit",
            "residual_probe_engine": "webkit",
            "browser_name": "Epiphany",
            "platform": "Linux x86_64",
            "user_agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 Epiphany/46.5",
            "webgl_unmasked_vendor": "Apple Inc.",
            "hw_curve_audio": curve,
            "hw_curve_webgl": curve,
            "residual_ok": true,
            "webgl_residual_entropy_ok": true,
            "residual_paths": [
                {"path_id":"a","ok":true,"entropy_ok":true,"mean":0.260,"curve": curve},
                {"path_id":"b","ok":true,"entropy_ok":true,"mean":0.261,"curve": curve},
            ]
        });
        let m = residual_measurement_status(
            &fields,
            &["B10_hw_curves".into()],
            Some("mid"),
        );
        // only 2 ok paths → not complete → worth deepen once
        assert_eq!(m["measurement_complete"], json!(false));
        assert_eq!(m["worth_deepen"], json!(true));
        // Thin paths: always-open B10x (may also note contract incomplete in reason).
        assert!(matches!(
            m["deepen_reason"].as_str(),
            Some("always_open_b10x_after_b10")
                | Some("always_open_b10x_contract_also_incomplete")
        ));
        assert!(m["missing"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x.as_str() == Some("paths_ge_3")));
    }

    #[test]
    fn residual_complete_still_always_open_b10x() {
        // Always-open policy: even when residual contract is full, B10x still schedules
        // so multipath deepen data is collected for cross-browser comparison.
        let curve: Vec<Value> = (0..32)
            .map(|i| json!(0.261 + (i as f64) * 0.0001))
            .collect();
        let path = |id: &str| {
            json!({"path_id": id, "ok": true, "entropy_ok": true, "mean": 0.261, "curve": curve})
        };
        let fields = json!({
            "form_class": "desktop",
            "os_family": "linux",
            "engine_family": "webkit",
            "residual_probe_engine": "webkit",
            "browser_name": "Epiphany",
            "platform": "Linux x86_64",
            "user_agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 Epiphany/46.5",
            "hw_curve_audio": curve,
            "hw_curve_webgl": curve,
            "webgl_residual_entropy_ok": true,
            "residual_paths": [path("a"), path("b"), path("c"), path("d")],
        });
        let m = residual_measurement_status(
            &fields,
            &["B10_hw_curves".into()],
            Some("mid"),
        );
        assert_eq!(m["measurement_complete"], json!(true));
        assert_eq!(m["worth_deepen"], json!(true));
        assert_eq!(m["deepen_reason"], json!("always_open_b10x_after_b10"));
        assert_eq!(m["schedule_gate_forgeable_os_br"], json!(false));
        assert_eq!(m["schedule_gate_measurement_complete"], json!(false));
        assert_eq!(m["policy"], json!("always_open_b10x_after_b10"));
        // After all B10x ticks landed → stop
        // Exhaust max_deepen_ticks (9) B10x packs
        let mut present: Vec<String> = vec!["B10_hw_curves".into()];
        for i in 0..9 {
            present.push(format!("B10x_tick_{i}"));
        }
        // count_b10x_ticks only counts B10x_ prefix — use real pack names
        present = vec![
            "B10_hw_curves".into(),
            "B10x_silicon_ulp".into(),
            "B10x_silicon_rint".into(),
            "B10x_silicon_noderiv".into(),
            "B10x_webkit_gl_noise".into(),
            "B10x_webkit_wave2_warm4".into(),
            "B10x_angle_crosscheck".into(),
            "B10x_legacy_webgl1".into(),
            "B10x_unknown_kernel".into(),
            "B10x_softgl_hedge".into(),
        ];
        let m2 = residual_measurement_status(&fields, &present, Some("mid"));
        assert_eq!(m2["worth_deepen"], json!(false));
    }

    #[test]
    fn blink_complete_still_opens_b10x() {
        let curve: Vec<Value> = (0..32).map(|_| json!(0.260)).collect();
        let path = |id: &str| {
            json!({"path_id": id, "ok": true, "entropy_ok": true, "mean": 0.260, "curve": curve})
        };
        let fields = json!({
            "form_class": "desktop",
            "os_family": "linux",
            "engine_family": "blink",
            "residual_probe_engine": "blink",
            "browser_name": "Chrome",
            "platform": "Linux x86_64",
            "user_agent": "Mozilla/5.0 (X11; Linux x86_64) Chrome/120.0.0.0",
            "hw_curve_audio": curve,
            "hw_curve_webgl": curve,
            "webgl_residual_entropy_ok": true,
            "residual_paths": [path("a"), path("b"), path("c")],
        });
        let m = residual_measurement_status(
            &fields,
            &["B10_hw_curves".into()],
            Some("mid"),
        );
        assert_eq!(m["measurement_complete"], json!(true));
        assert_eq!(m["worth_deepen"], json!(true));
        assert_eq!(m["deepen_reason"], json!("always_open_b10x_after_b10"));
    }

    #[test]
    fn multi_precision_identical_curves() {
        let a = vec![0.26; 32];
        let b = vec![0.26; 32];
        let m = residual_multi_precision_match(&a, &b);
        assert_eq!(m["point_agree_0p00001"], json!(1.0));
        assert_eq!(m["bucket_priority"], json!("L2_near_bit_stable"));
    }

    #[test]
    fn multi_precision_sd_fork_scale() {
        // blink vs webkit-like: most points differ by ~0.003
        let a: Vec<f64> = (0..32).map(|i| 0.22 + (i as f64) * 0.001).collect();
        let b: Vec<f64> = a.iter().map(|x| x + 0.003).collect();
        let m = residual_multi_precision_match(&a, &b);
        assert!(m["point_agree_0p001"].as_f64().unwrap() < 0.5);
        assert_eq!(m["bucket_priority"], json!("measurement_channel_split"));
    }

    #[test]
    fn parse_epiphany_ua() {
        let ua = "Mozilla/5.0 (X11; Ubuntu; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15 Epiphany/46.5";
        let (n, v) = parse_browser_from_ua(ua);
        assert_eq!(n.as_deref(), Some("Epiphany"));
        assert_eq!(v.as_deref(), Some("46.5"));
    }

    #[test]
    fn env_and_gap_webkit() {
        let fields = json!({
            "user_agent": "Mozilla/5.0 (X11; Ubuntu; Linux x86_64) AppleWebKit/605.1.15 Version/17.0 Safari/605.1.15 Epiphany/46.5",
            "platform": "Linux x86_64",
            "engine_family": "webkit",
            "residual_probe_engine": "webkit",
            "webgl_unmasked_vendor": "Apple Inc.",
            "webgl_unmasked_renderer": "",
            "server_client_ip": "203.0.113.9",
            "residual_paths": [{
                "path_id":"warm2_v3f_128_noderiv",
                "ok":true,
                "mean":0.26,
                "std":0.09,
                "curve": [0.26,0.26,0.26,0.26,0.26,0.26,0.26,0.26,0.26,0.26,0.26,0.26,0.26,0.26,0.26,0.26,0.26,0.26,0.26,0.26,0.26,0.26,0.26,0.26,0.26,0.26,0.26,0.26,0.26,0.26,0.26,0.26]
            }],
            "residual_select": {"chosen_path_id":"warm2_v3f_128_noderiv"}
        });
        let env = project_client_env_context(&fields, None);
        assert_eq!(env["browser_name"], json!("Epiphany"));
        assert_eq!(env["client_ip"], json!("203.0.113.9"));
        assert_eq!(env["gl"]["unmask_blocked"], json!(true));
        let gap = detect_probe_coverage_gap(&fields, &[]);
        assert_eq!(gap["present"], json!(true));
        assert!(gap["missing_components"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x.as_str() == Some("B10x_webkit_gl_noise")));
    }
}
