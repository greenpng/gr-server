//! M2 gateway cross-layer coherence (iss/74 §6-M2).
//!
//! All server-side, no FE field increments:
//! - sec-ch three-way version inter-check (UA / ua_ch_ua_full_version /
//!   ua_brands majors)
//! - platform pair check (ua_ch_platform vs ua_platform vs platform class)
//! - TLS / H2 / QUIC / TCP cross-layer engine-family vote
//! - UA engine claim vs gateway JA4-derived engine
//! - `gateway_coherence_score` + verdict for the product network surface.

use crate::protocol_edge::{brand_to_engine_family, classify_protocol_engine};
use serde_json::{json, Value};

fn s<'a>(fo: &'a serde_json::Map<String, Value>, k: &str) -> &'a str {
    fo.get(k).and_then(|v| v.as_str()).unwrap_or("")
}

fn major_version(versionish: &str) -> Option<&str> {
    // "Chrome/120.0.0.0" or "120.0.0.0" or "120" → "120"
    let v = versionish.split('/').last()?;
    let m = v.split('.').next()?;
    if m.is_empty() || !m.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(m)
}

/// Browser major version from a UA string: prefer chrome/firefox/edg/crios
/// brand tokens; only fall back to Safari/ when no other brand matched
/// (WebKit version ≠ browser version in Chromium UAs).
fn ua_browser_major(ua: &str) -> Option<String> {
    let u = ua.to_ascii_lowercase();
    for marker in ["crios/", "edg/", "chrome/", "firefox/", "fxios/", "safari/"] {
        let skip_webkit_fallback = marker == "safari/"
            && (u.contains("chrome/") || u.contains("firefox/") || u.contains("edg/") || u.contains("crios/"));
        if skip_webkit_fallback {
            continue;
        }
        if let Some(pos) = u.find(marker) {
            let rest = &u[pos + marker.len()..];
            let m = rest.split(|c: char| !c.is_ascii_digit()).next().unwrap_or("");
            if !m.is_empty() {
                return Some(m.to_string());
            }
        }
    }
    None
}

/// sec-ch version coherence (M2: sec-ch 三路版本互证, extended multi-source).
/// Sources: UA major, FE ua_ch_ua_full_version major, FE ua_brands major,
/// gateway-seen gw_ua_ch_ua_full_version major, gateway-seen gw_ua_brands major.
/// FE-vs-gateway drift of BOTH observed hint channels is a hard conflict even
/// when UA agrees with FE (spoofed client hints or inconsistent browser).
fn major_from_brands(bs: &str) -> Option<String> {
    bs.split(',')
        .map(|p| p.trim())
        .filter(|p| {
            let b = p.split(':').next().unwrap_or("").to_ascii_lowercase();
            b.contains("chrome") || b.contains("chromium") || b.contains("safari") || b.contains("firefox")
        })
        .filter_map(|p| p.split(':').nth(1).and_then(major_version))
        .map(str::to_string)
        .next()
}

pub fn sec_ch_version_triple(fields: &serde_json::Map<String, Value>) -> Value {
    let ua_major = ua_browser_major(s(fields, "user_agent"));
    let ch_major = if s(fields, "ua_ch_ua_full_version").is_empty() {
        None
    } else {
        major_version(s(fields, "ua_ch_ua_full_version")).map(str::to_string)
    };
    let brands_major = fields
        .get("ua_brands")
        .and_then(|v| v.as_str())
        .and_then(major_from_brands);
    let gw_ch_major = if s(fields, "gw_ua_ch_ua_full_version").is_empty() {
        None
    } else {
        major_version(s(fields, "gw_ua_ch_ua_full_version")).map(str::to_string)
    };
    let gw_brands_major = fields
        .get("gw_ua_brands")
        .and_then(|v| v.as_str())
        .and_then(major_from_brands);
    // FE-claimed vs gateway-seen hint drift → hard conflict (cross-source).
    let fe_gw_drift = (ch_major.is_some() && gw_ch_major.is_some() && ch_major != gw_ch_major)
        || (brands_major.is_some() && gw_brands_major.is_some() && brands_major != gw_brands_major);
    if fe_gw_drift {
        return json!({
            "algo": "m2_sec_ch_triple_v1",
            "verdict": "conflict",
            "note": "fe_gw_hint_drift",
            "sources_present": 0,
            "agree_pairs": 0,
            "ua_major": ua_major,
            "ch_full_major": ch_major,
            "brands_major": brands_major,
            "gw_ch_full_major": gw_ch_major,
            "gw_brands_major": gw_brands_major,
        });
    }
    let mut present = 0usize;
    let mut agree = 0usize;
    let mut ref_major: Option<String> = None;
    for m in [&ua_major, &ch_major, &brands_major, &gw_ch_major, &gw_brands_major]
        .into_iter()
        .flatten()
    {
        present += 1;
        match &ref_major {
            None => ref_major = Some(m.clone()),
            Some(r) if r == m => agree += 1,
            Some(_) => {}
        }
    }
    let verdict = if present == 0 {
        "missing"
    } else if present == 1 || agree == present - 1 {
        "agree"
    } else if agree >= 1 {
        "partial"
    } else {
        "conflict"
    };
    json!({
        "algo": "m2_sec_ch_triple_v1",
        "verdict": verdict,
        "sources_present": present,
        "agree_pairs": agree,
        "ua_major": ua_major,
        "ch_full_major": ch_major,
        "brands_major": brands_major,
        "gw_ch_full_major": gw_ch_major,
        "gw_brands_major": gw_brands_major,
    })
}

fn os_class(p: &str) -> &'static str {
    let p = p.to_ascii_lowercase();
    if p.contains("win") {
        "win"
    } else if p.contains("mac") || p.contains("iphone") || p.contains("ipad") {
        "mac"
    } else if p.contains("android") {
        "android"
    } else if p.contains("linux") || p.contains("x11") || p.contains("chromeos") {
        "linux"
    } else if p.is_empty() {
        ""
    } else {
        "other"
    }
}

/// Platform pair check: ua_ch_platform vs ua_platform vs platform, plus the
/// gateway-seen gw_ua_ch_platform (class-level majority vote).
/// Majority rule: all same → agree; a strict majority (>N/2) → partial;
/// a 2:2 split on four sources is a tie → conflict (no majority).
pub fn platform_pair_check(fields: &serde_json::Map<String, Value>) -> Value {
    let ch = os_class(s(fields, "ua_ch_platform"));
    let uap = os_class(s(fields, "ua_platform"));
    let plat = os_class(s(fields, "platform"));
    let gw = os_class(s(fields, "gw_ua_ch_platform"));
    let mut classes: Vec<&str> = Vec::new();
    if !ch.is_empty() {
        classes.push(ch);
    }
    if !uap.is_empty() {
        classes.push(uap);
    }
    if !plat.is_empty() {
        classes.push(plat);
    }
    if !gw.is_empty() {
        classes.push(gw);
    }
    let verdict = if classes.len() < 2 {
        "missing"
    } else if classes.iter().all(|c| *c == classes[0]) {
        "agree"
    } else {
        let agree_n = classes.iter().filter(|c| **c == classes[0]).count();
        if agree_n * 2 > classes.len() {
            "partial"
        } else {
            "conflict"
        }
    };
    let mut js_classes: Vec<&str> = Vec::new();
    if !ch.is_empty() {
        js_classes.push(ch);
    }
    if !uap.is_empty() {
        js_classes.push(uap);
    }
    if !plat.is_empty() {
        js_classes.push(plat);
    }
    if !gw.is_empty() {
        js_classes.push(gw);
    }
    json!({
        "algo": "m2_platform_pair_v1",
        "verdict": verdict,
        "classes": js_classes,
        "ua_ch_platform": s(fields, "ua_ch_platform"),
        "ua_platform": s(fields, "ua_platform"),
        "platform": s(fields, "platform"),
        "gw_ua_ch_platform": s(fields, "gw_ua_ch_platform"),
    })
}

/// TLS / H2 / QUIC / TCP cross-layer engine-family vote.
/// Known-depth fields: ja4/tls_ja4 (TLS), h2_fingerprint (H2), quic_tp_summary
/// (QUIC), tcp_syn_p0f_sig (TCP). Each layer maps to an engine family where
/// recognizable; unknown layers abstain.
pub fn protocol_cross_layer_coherence(fields: &serde_json::Map<String, Value>) -> Value {
    let ja4 = s(fields, "ja4");
    let ja4 = if ja4.is_empty() { s(fields, "tls_ja4") } else { ja4 };
    let h2 = s(fields, "h2_fingerprint");
    if h2.is_empty() {
        // allow http2_fingerprint alias
        // (inject_protocol_from_headers writes both; keep single read path)
    }
    let h2 = if h2.is_empty() { s(fields, "http2_fingerprint") } else { h2 };
    let quic = s(fields, "quic_tp_summary");
    let tcp = s(fields, "tcp_syn_p0f_sig");

    let mut layers: Vec<(&str, &'static str, String)> = Vec::new(); // (layer, family, raw)
    if !ja4.is_empty() {
        let eng = classify_protocol_engine(ja4, "", "");
        // classify returns chrome/firefox/safari/edge/unknown_tls13/unknown — map to family
        let fam = match eng {
            "chrome" | "edge" => "blink",
            "firefox" => "gecko",
            "safari" => "webkit",
            _ => "unknown",
        };
        layers.push(("tls", fam, ja4.to_string()));
    }
    if !h2.is_empty() {
        let fam = brand_to_engine_family(&h2);
        layers.push(("h2", fam, h2.to_string()));
    }
    if !quic.is_empty() {
        let fam = brand_to_engine_family(quic);
        layers.push(("quic", fam, quic.to_string()));
    }
    if !tcp.is_empty() {
        let fam = brand_to_engine_family(tcp);
        layers.push(("tcp", fam, tcp.to_string()));
    }

    let known: Vec<(String, &'static str)> = layers
        .iter()
        .filter(|(_, fam, _)| *fam != "unknown")
        .map(|(l, fam, _)| (l.to_string(), *fam))
        .collect();
    let verdict = if known.is_empty() {
        "missing"
    } else if known.iter().all(|(_, f)| *f == known[0].1) {
        "agree"
    } else {
        let first = known[0].1;
        let agree_n = known.iter().filter(|(_, f)| *f == first).count();
        // A 2-layer split is a tie → conflict; only ≥3 layers can show a
        // non-trivial majority (partial).
        if agree_n >= 2 && agree_n * 2 > known.len() {
            "partial"
        } else {
            "conflict"
        }
    };
    json!({
        "algo": "m2_protocol_cross_layer_v1",
        "verdict": verdict,
        "layers_known": known.iter().map(|(l, f)| json!({"layer": l, "family": f})).collect::<Vec<_>>(),
        "layers_present": layers.len(),
    })
}

/// UA engine claim vs gateway JA4 engine (M2: JA4H/UA 相干).
pub fn ua_ja4_engine_coherence(fields: &serde_json::Map<String, Value>) -> Value {
    let ua = s(fields, "user_agent").to_ascii_lowercase();
    let claim = if ua.contains("firefox") {
        "gecko"
    } else if ua.contains("safari") && !ua.contains("chrome") && !ua.contains("chromium") {
        "webkit"
    } else if ua.contains("chrome") || ua.contains("chromium") || ua.contains("edg") || ua.contains("crios") {
        "blink"
    } else {
        "unknown"
    };
    let ja4 = s(fields, "ja4");
    let ja4 = if ja4.is_empty() { s(fields, "tls_ja4") } else { ja4 };
    let obs = if ja4.is_empty() {
        "unknown"
    } else {
        match classify_protocol_engine(&ja4, "", "") {
            "chrome" | "edge" => "blink",
            "firefox" => "gecko",
            "safari" => "webkit",
            _ => "unknown",
        }
    };
    let verdict = if claim == "unknown" || obs == "unknown" {
        "missing"
    } else if claim == obs {
        "agree"
    } else {
        "conflict"
    };
    json!({
        "algo": "m2_ua_ja4_engine_v1",
        "verdict": verdict,
        "ua_engine": claim,
        "ja4_engine": obs,
    })
}

/// M2 aggregate: gateway protocol coherence score ∈ [0,1] over the components
/// that had material; verdict agree/partial/conflict/qualified.
pub fn gateway_coherence(fields: &serde_json::Map<String, Value>) -> Value {
    let sec = sec_ch_version_triple(fields);
    let plat = platform_pair_check(fields);
    let proto = protocol_cross_layer_coherence(fields);
    let ua_ja4 = ua_ja4_engine_coherence(fields);

    let mut total = 0.0f64;
    let mut weight = 0.0f64;
    for (v, w) in [
        (sec.get("verdict").and_then(|x| x.as_str()).unwrap_or("missing"), 0.30),
        (plat.get("verdict").and_then(|x| x.as_str()).unwrap_or("missing"), 0.20),
        (proto.get("verdict").and_then(|x| x.as_str()).unwrap_or("missing"), 0.35),
        (ua_ja4.get("verdict").and_then(|x| x.as_str()).unwrap_or("missing"), 0.15),
    ] {
        if v == "missing" || v == "qualified" {
            continue;
        }
        weight += w;
        total += w * match v {
            "agree" => 1.0,
            "partial" => 0.5,
            _ => 0.0,
        };
    }
    let (score, verdict) = if weight == 0.0 {
        (1.0f64, "qualified")
    } else {
        let sc = total / weight;
        let v = if sc >= 0.85 {
            "agree"
        } else if sc >= 0.5 {
            "partial"
        } else {
            "conflict"
        };
        ((sc * 10000.0).round() / 10000.0, v)
    };
    json!({
        "algo": "m2_gateway_coherence_v1",
        "verdict": verdict,
        "score": score,
        "components": {
            "sec_ch_version_triple": sec,
            "platform_pair": plat,
            "protocol_cross_layer": proto,
            "ua_ja4_engine": ua_ja4,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use serde_json::Map;

    fn fo(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn sec_ch_triple_agrees_on_matching_majors() {
        let f = fo(json!({
            "user_agent": "Mozilla/5.0 Chrome/120.0.0.0 Safari/537.36",
            "ua_ch_ua_full_version": "120.0.6099.130",
            "ua_brands": "Not A(Brand):99.0.0.0,Chromium:120.0.0.0,Google Chrome:120.0.0.0",
        }));
        let v = sec_ch_version_triple(&f);
        assert_eq!(v["verdict"], "agree", "{v}");
        assert_eq!(v["ua_major"], "120");
        assert_eq!(v["ch_full_major"], "120");
        assert_eq!(v["brands_major"], "120");
    }

    #[test]
    fn sec_ch_triple_conflict_on_major_drift() {
        let f = fo(json!({
            "user_agent": "Mozilla/5.0 Chrome/120.0.0.0 Safari/537.36",
            "ua_ch_ua_full_version": "131.0.0.0",
            "ua_brands": "Not A(Brand):99.0.0.0,Chromium:131.0.0.0",
        }));
        let v = sec_ch_version_triple(&f);
        assert_eq!(v["verdict"], "conflict", "{v}");
    }

    #[test]
    fn sec_ch_triple_partial_and_missing() {
        let f = fo(json!({
            "user_agent": "Mozilla/5.0 Chrome/120.0.0.0 Safari/537.36",
            "ua_ch_ua_full_version": "120.0.6099.130",
        }));
        let v = sec_ch_version_triple(&f);
        assert_eq!(v["verdict"], "agree", "2 sources agree: {v}");
        let f2 = fo(json!({"platform": "Linux"}));
        let v2 = sec_ch_version_triple(&f2);
        assert_eq!(v2["verdict"], "missing", "{v2}");
    }

    #[test]
    fn platform_pair_conflict_detected() {
        // 2-of-3 classes agree → partial (majority), not a hard conflict.
        let f = fo(json!({
            "ua_ch_platform": "Windows",
            "ua_platform": "Windows",
            "platform": "Linux x86_64",
        }));
        let v = platform_pair_check(&f);
        assert_eq!(v["verdict"], "partial", "{v}");
        // No majority at all → conflict.
        let f3 = fo(json!({
            "ua_ch_platform": "Windows",
            "ua_platform": "MacIntel",
            "platform": "Linux x86_64",
        }));
        assert_eq!(platform_pair_check(&f3)["verdict"], "conflict");
        let f2 = fo(json!({
            "ua_ch_platform": "Linux",
            "ua_platform": "Linux",
            "platform": "Linux x86_64",
        }));
        assert_eq!(platform_pair_check(&f2)["verdict"], "agree");
    }

    #[test]
    fn protocol_cross_layer_vote() {
        // TLS (labeled cr_ → blink) + H2 both blink; QUIC unknown abstains.
        let f = fo(json!({
            "ja4": "cr_120_t13d1516h2_8daaf6152771_b0da82dd1658",
            "h2_fingerprint": "Chrome 120",
            "quic_tp_summary": "x",
        }));
        let v = protocol_cross_layer_coherence(&f);
        assert_eq!(v["verdict"], "agree", "{v}");
        assert_eq!(v["layers_known"].as_array().unwrap().len(), 2, "{v}");
        // TLS-labeled gecko vs blink h2 → conflict family vote
        let f2 = fo(json!({
            "ja4": "firefox_115_nss",
            "h2_fingerprint": "Chrome 120",
        }));
        let v2 = protocol_cross_layer_coherence(&f2);
        assert_eq!(v2["verdict"], "conflict", "{v2}");
        // Untagged t13d JA4 abstains (unknown) — only known layers vote.
        let f3 = fo(json!({
            "ja4": "t13d1516h2_8daaf6152771_b0da82dd1658",
            "h2_fingerprint": "Chrome 120",
        }));
        let v3 = protocol_cross_layer_coherence(&f3);
        assert_eq!(v3["layers_known"].as_array().unwrap().len(), 1, "{v3}");
        assert_eq!(v3["verdict"], "agree", "{v3}");
    }

    #[test]
    fn ua_ja4_engine_conflict() {
        let f = fo(json!({
            "user_agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/120 Safari/537.36",
            "ja4": "firefox_115_nss_3k",
        }));
        let v = ua_ja4_engine_coherence(&f);
        assert_eq!(v["verdict"], "conflict", "{v}");
        assert_eq!(v["ua_engine"], "blink");
        assert_eq!(v["ja4_engine"], "gecko");
    }

    #[test]
    fn sec_ch_triple_gateway_fe_drift_conflicts() {
        // FE claims 120, gateway-seen hints say 131, UA agrees with FE — the
        // FE-vs-gateway drift must surface as a conflict (cross-source).
        let f = fo(json!({
            "user_agent": "Mozilla/5.0 Chrome/120.0.0.0 Safari/537.36",
            "ua_ch_ua_full_version": "120.0.6099.130",
            "ua_brands": "Not A(Brand):99.0.0.0,Chromium:120.0.0.0",
            "gw_ua_ch_ua_full_version": "131.0.6778.86",
            "gw_ua_brands": "Chromium:131.0.6778.86",
        }));
        let v = sec_ch_version_triple(&f);
        assert_eq!(v["verdict"], "conflict", "{v}");
        assert_eq!(v["note"], "fe_gw_hint_drift", "{v}");
        // Gateway-only consistency with UA → agree (no FE drift possible).
        let f2 = fo(json!({
            "user_agent": "Mozilla/5.0 Chrome/131.0.0.0 Safari/537.36",
            "gw_ua_ch_ua_full_version": "131.0.6778.86",
            "gw_ua_brands": "Chromium:131.0.6778.86",
        }));
        let v2 = sec_ch_version_triple(&f2);
        assert_eq!(v2["verdict"], "agree", "{v2}");
        // Matching FE+GW hints → agree with more sources.
        let f3 = fo(json!({
            "user_agent": "Mozilla/5.0 Chrome/120.0.0.0 Safari/537.36",
            "ua_ch_ua_full_version": "120.0.6099.130",
            "gw_ua_ch_ua_full_version": "120.0.6099.130",
        }));
        let v3 = sec_ch_version_triple(&f3);
        assert_eq!(v3["verdict"], "agree", "{v3}");
        assert_eq!(v3["sources_present"], 3, "{v3}");
    }

    #[test]
    fn platform_pair_joins_gateway_hint() {
        // FE + gateway both say Windows, platform says Linux → majority partial.
        let f = fo(json!({
            "ua_ch_platform": "Windows",
            "gw_ua_ch_platform": "Windows",
            "platform": "Linux x86_64",
        }));
        assert_eq!(platform_pair_check(&f)["verdict"], "partial");
        // FE claims Windows but gateway + platform agree Linux → conflict.
        let f2 = fo(json!({
            "ua_ch_platform": "Windows",
            "gw_ua_ch_platform": "Linux",
            "platform": "Linux x86_64",
        }));
        assert_eq!(platform_pair_check(&f2)["verdict"], "conflict");
        // Four-way tie 2:2 → conflict (no strict majority).
        let f3 = fo(json!({
            "ua_ch_platform": "Windows",
            "gw_ua_ch_platform": "Windows",
            "ua_platform": "MacIntel",
            "platform": "Linux x86_64",
        }));
        assert_eq!(platform_pair_check(&f3)["verdict"], "conflict");
    }

    #[test]
    fn gateway_coherence_aggregates_and_qualified() {
        let f = fo(json!({
            "user_agent": "Mozilla/5.0 Chrome/120.0.0.0 Safari/537.36",
            "ua_ch_ua_full_version": "120.0.6099.130",
            "ua_brands": "Chromium:120.0.0.0,Google Chrome:120.0.0.0",
            "ua_ch_platform": "Linux",
            "ua_platform": "Linux",
            "platform": "Linux x86_64",
            "ja4": "t13d1516h2_8daaf6152771_b0da82dd1658",
            "h2_fingerprint": "Chrome 120",
        }));
        let v = gateway_coherence(&f);
        assert_eq!(v["verdict"], "agree", "{v}");
        assert!(v["score"].as_f64().unwrap() >= 0.85, "{v}");
        let f2 = fo(json!({"platform": "Linux"}));
        let v2 = gateway_coherence(&f2);
        assert_eq!(v2["verdict"], "qualified", "{v2}");
        assert_eq!(v2["score"], 1.0);
    }
}
