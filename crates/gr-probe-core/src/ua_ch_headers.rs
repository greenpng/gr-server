//! Gateway-side Client Hints (sec-ch-*) ingestion → ua_ch_* canonical keys (M2).
//!
//! The gateway receives the browser's own Client Hints on the request line
//! (sec-ch-ua*, sent automatically by the browser for same-origin requests).
//! We normalize them under the same canonical `ua_ch_*` grammar the FE uses,
//! but scoped as `gw_ua_ch_*` so M2 coherence compares FE-observed hints
//! against gateway-seen hints (a genuine cross-source material).
//!
//! Trust: sec-ch headers are **client-declared** (spoofable by a hostile
//! client). They feed coherence / multi-source checks only — never mint,
//! never hard browser uniqueness.

use serde_json::{json, Map, Value};

/// Request headers we accept, mapped to their canonical ua_ch_* base name.
pub const TRUSTED_SEC_CH_HEADERS: &[(&str, &str)] = &[
    ("sec-ch-ua-platform", "ua_ch_platform"),
    ("sec-ch-ua-platform-version", "ua_ch_platform_version"),
    ("sec-ch-ua-mobile", "ua_ch_mobile"),
    ("sec-ch-ua-arch", "ua_ch_architecture"),
    ("sec-ch-ua-model", "ua_ch_model"),
    ("sec-ch-ua-bitness", "ua_ch_bitness"),
    ("sec-ch-ua-wow64", "ua_ch_wow64"),
    ("sec-ch-ua-full-version-list", "ua_ch_ua_full_version_list"),
];

/// Normalize a `Sec-CH-UA` brand list into the FE `ua_brands` grammar
/// (`Brand:version,Brand2:version`), e.g.
/// `"Chromium";v="120.0.6099.130", "Google Chrome";v="120.0.6099.130"`
/// → `Chromium:120.0.6099.130,Google Chrome:120.0.6099.130`.
/// Returns None when nothing parseable.
pub fn normalize_sec_ch_ua_brands(v: &str) -> Option<String> {
    let mut out: Vec<String> = Vec::new();
    for part in v.split(',') {
        let p = part.trim();
        let rest;
        let brand;
        // optional leading quote
        if let Some(stripped) = p.strip_prefix('"') {
            let Some((head, tail)) = stripped.split_once('"') else {
                continue;
            };
            brand = head.trim().to_string();
            rest = tail;
        } else {
            // unquoted brand before ";"
            let Some((head, tail)) = p.split_once(';') else {
                continue;
            };
            brand = head.trim().to_string();
            rest = tail;
        }
        // version: `;v="120.0.0.0"` or `;v=120`
        let vi = rest.find("v=");
        let Some(vi) = vi else { continue };
        let mut vpart = rest[vi + 2..].trim().to_string();
        if let Some(qv) = vpart.strip_prefix('"') {
            vpart = qv.to_string();
        }
        if let Some((head, _)) = vpart.split_once('"') {
            vpart = head.to_string();
        }
        let vpart = vpart.trim();
        if brand.is_empty() || vpart.is_empty() {
            continue;
        }
        out.push(format!("{brand}:{vpart}"));
    }
    if out.is_empty() {
        None
    } else {
        Some(out.join(","))
    }
}

/// First chrome-family full version from a `Sec-CH-UA-Full-Version-List`
/// (e.g. `"Chromium";v="120.0.6099.130", "Not?A_Brand";v="24"` → the Chromium one).
pub fn sec_ch_full_version_list_v(fvl: &str) -> Option<String> {
    let mut fallback: Option<String> = None;
    for part in fvl.split(',') {
        let p = part.trim();
        let Some((head, tail)) = p.split_once(';') else {
            continue;
        };
        let brand = head.trim().trim_matches('"').to_ascii_lowercase();
        if fallback.is_none() {
            let vi = tail.find("v=");
            if let Some(vi) = vi {
                let vpart = tail[vi + 2..].trim().trim_matches('"');
                if !vpart.is_empty() {
                    fallback = Some(vpart.to_string());
                }
            }
        }
        if brand.contains("chrome") || brand.contains("chromium") || brand.contains("edge") {
            let vi = tail.find("v=");
            if let Some(vi) = vi {
                let vpart = tail[vi + 2..].trim().trim_matches('"');
                if !vpart.is_empty() {
                    return Some(vpart.to_string());
                }
            }
        }
    }
    fallback
}

/// Ingest trusted sec-ch request headers into gateway fields as `gw_ua_ch_*`
/// + `gw_ua_brands` / `gw_ua_ch_ua_full_version` + honesty markers.
/// `headers` keys must be lowercased (callers lowercase before invoke).
pub fn inject_gateway_ua_ch(fields: &mut Map<String, Value>, headers: &Map<String, Value>) {
    let mut present_n = 0usize;
    for (hk, fk) in TRUSTED_SEC_CH_HEADERS {
        if let Some(v) = headers.get(*hk).and_then(|x| x.as_str()) {
            let t = v.trim();
            if !t.is_empty() {
                fields.insert(format!("gw_{fk}"), json!(t));
                present_n += 1;
            }
        }
    }
    if let Some(v) = headers.get("sec-ch-ua").and_then(|x| x.as_str()) {
        if let Some(b) = normalize_sec_ch_ua_brands(v) {
            fields.insert("gw_ua_brands".into(), json!(b));
        }
    }
    if let Some(v) = headers.get("sec-ch-ua-full-version-list").and_then(|x| x.as_str()) {
        if let Some(fv) = sec_ch_full_version_list_v(v) {
            fields.insert("gw_ua_ch_ua_full_version".into(), json!(fv));
        }
    }
    if let Some(v) = headers.get("sec-ch-ua-mobile").and_then(|x| x.as_str()) {
        let m = v.trim().trim_start_matches('?');
        if m == "1" || m == "0" {
            fields.insert("gw_ua_ch_mobile".into(), json!(m == "1"));
        }
    }
    if present_n > 0 || fields.contains_key("gw_ua_brands") {
        fields.insert("gw_ua_ch_present".into(), json!(true));
        fields.insert("gw_ua_ch_n".into(), json!(present_n));
        fields.insert("gw_ua_ch_origin".into(), json!("gateway_headers"));
        // Honesty: client-declared hints — coherence only, never mint/hard uniqueness.
        fields.insert("gw_ua_ch__client_declared".into(), json!(true));
        fields.insert("gw_ua_ch__hard_browser_uniqueness".into(), json!(false));
        fields.insert("gw_ua_ch__commercial_mint".into(), json!(false));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_sec_ch_ua_brands() {
        let v = r#""Not_A Brand";v="99", "Chromium";v="120.0.6099.130", "Google Chrome";v="120.0.6099.130""#;
        let out = normalize_sec_ch_ua_brands(v).unwrap();
        assert!(out.contains("Chromium:120.0.6099.130"), "{out}");
        assert!(!out.is_empty());
        assert!(out.split(',').count() == 3, "{out}");
    }

    #[test]
    fn normalizes_unquoted_version() {
        let out = normalize_sec_ch_ua_brands(r#""Chromium";v=120"#).unwrap();
        assert_eq!(out, "Chromium:120");
    }

    #[test]
    fn rejects_garbage_brand_list() {
        assert!(normalize_sec_ch_ua_brands("not a header").is_none());
        assert!(normalize_sec_ch_ua_brands("").is_none());
    }

    #[test]
    fn full_version_list_prefers_chrome_family() {
        let fvl = r#""Not?A_Brand";v="24", "Chromium";v="120.0.6099.131", "Google Chrome";v="120.0.6099.131""#;
        assert_eq!(sec_ch_full_version_list_v(fvl).as_deref(), Some("120.0.6099.131"));
        // fallback to first when no chrome-family brand
        let fvl2 = r#""Firefox";v="115.0.1""#;
        assert_eq!(sec_ch_full_version_list_v(fvl2).as_deref(), Some("115.0.1"));
        assert!(sec_ch_full_version_list_v("").is_none());
    }

    #[test]
    fn inject_gateway_ua_ch_maps_fields() {
        let mut f = Map::new();
        let mut h = Map::new();
        h.insert("sec-ch-ua-platform".into(), json!("Linux"));
        h.insert("sec-ch-ua-mobile".into(), json!("?0"));
        h.insert(
            "sec-ch-ua-full-version-list".into(),
            json!(r#""Chromium";v="120.0.6099.130""#),
        );
        h.insert("sec-ch-ua".into(), json!(r#""Chromium";v="120""#));
        inject_gateway_ua_ch(&mut f, &h);
        assert_eq!(f.get("gw_ua_ch_platform").and_then(|v| v.as_str()), Some("Linux"));
        assert_eq!(f.get("gw_ua_ch_mobile").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(
            f.get("gw_ua_ch_ua_full_version").and_then(|v| v.as_str()),
            Some("120.0.6099.130")
        );
        assert_eq!(f.get("gw_ua_brands").and_then(|v| v.as_str()), Some("Chromium:120"));
        assert_eq!(f.get("gw_ua_ch_present").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(
            f.get("gw_ua_ch__hard_browser_uniqueness").and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn inject_no_headers_no_markers() {
        let mut f = Map::new();
        let h = Map::new();
        inject_gateway_ua_ch(&mut f, &h);
        assert!(f.is_empty());
    }
}
