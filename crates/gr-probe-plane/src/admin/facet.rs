//! Visitor class for business dashboard: **browser | robots** (+ robot_name).
//!
//! v5.8.7+: binary taxonomy only for **new** writes. Historical js/nojs rows are left
//! untouched (no migration). New dashboard counts only browser/robots.
//! Does **not** enter commercial digest.

use serde_json::{json, Map, Value};

/// Named robot tokens (order: more specific first).
const NAMED_ROBOTS: &[(&str, &str)] = &[
    ("googlebot", "Googlebot"),
    ("bingbot", "Bingbot"),
    ("bingpreview", "BingPreview"),
    ("baiduspider", "Baiduspider"),
    ("yandexbot", "YandexBot"),
    ("yandex", "Yandex"),
    ("duckduckbot", "DuckDuckBot"),
    ("applebot", "Applebot"),
    ("facebookexternalhit", "FacebookBot"),
    ("meta-externalagent", "MetaBot"),
    ("semrushbot", "SemrushBot"),
    ("ahrefsbot", "AhrefsBot"),
    ("gptbot", "GPTBot"),
    ("claudebot", "ClaudeBot"),
    ("anthropic-ai", "Anthropic"),
    ("bytespider", "Bytespider"),
    ("petalsearch", "PetalBot"),
    ("sogou", "Sogou"),
    ("slurp", "YahooSlurp"),
    ("dotbot", "DotBot"),
    ("mj12bot", "MJ12bot"),
    ("seznambot", "SeznamBot"),
];

const GENERIC_ROBOT_UA: &[&str] = &[
    "bot",
    "spider",
    "crawl",
    "slurp",
    "facebookexternalhit",
    "bingpreview",
    "yandex",
    "baiduspider",
    "duckduckbot",
    "applebot",
    "semrush",
    "ahrefs",
    "gptbot",
    "claudebot",
    "bytespider",
    "petalsearch",
    "sogou",
    "curl/",
    "wget/",
    "python-requests",
    "go-http-client",
    "java/",
    "libwww",
    "httpclient",
    "scrapy",
    "headlesschrome",
];

#[derive(Debug, Clone)]
pub struct VisitClass {
    pub facet: &'static str,
    pub robot_name: Option<String>,
    pub reasons: Vec<String>,
}

impl VisitClass {
    pub fn to_json(&self) -> Value {
        json!({
            "visitor_facet": self.facet,
            "robot_name": self.robot_name,
            "class_reasons": self.reasons,
        })
    }
}

/// Extract a display robot name from UA when clearly a crawler/bot.
pub fn extract_robot_name(ua: &str) -> Option<String> {
    let ual = ua.to_ascii_lowercase();
    if ual.is_empty() {
        return None;
    }
    for (needle, name) in NAMED_ROBOTS {
        if ual.contains(needle) {
            return Some((*name).to_string());
        }
    }
    if ual.contains("curl/") {
        return Some("curl".into());
    }
    if ual.contains("wget/") {
        return Some("wget".into());
    }
    if ual.contains("python-requests") {
        return Some("python-requests".into());
    }
    if ual.contains("go-http-client") {
        return Some("Go-http-client".into());
    }
    if GENERIC_ROBOT_UA.iter().any(|p| ual.contains(p)) {
        return Some("crawler".into());
    }
    None
}

/// Inputs kept for call-site compat; only UA drives binary class now.
#[derive(Debug, Clone, Default)]
pub struct ClassInputs {
    pub ua: String,
    /// Pixel / noscript path (still **browser** unless UA is robot).
    pub is_pixel: bool,
    /// FE main/worker/iframe ingest present.
    pub has_fe_main: bool,
    /// Session open initiated by FE boot.
    pub fe_open_hint: bool,
    /// visitor_terminal_id present.
    pub has_vtid: bool,
    /// Backend SDK claimed client_class=js.
    pub backend_claims_js: bool,
}

/// Binary classify for biz dashboard.
/// - **robots**: UA has a clear crawler/tool name
/// - **browser**: everything else (JS FE, gateway-only short visit, noscript pixel, unknown UA)
pub fn classify_visit_class(inp: &ClassInputs) -> VisitClass {
    let mut reasons = Vec::new();
    let robot_name = extract_robot_name(&inp.ua);
    if robot_name.is_some() {
        reasons.push("ua_robot".into());
        return VisitClass {
            facet: "robots",
            robot_name,
            reasons,
        };
    }
    if inp.has_fe_main {
        reasons.push("fe_main_source".into());
    } else if inp.fe_open_hint {
        reasons.push("fe_open_hint".into());
    } else if inp.backend_claims_js && inp.has_vtid {
        reasons.push("backend_sdk_js_vtid".into());
    } else if inp.is_pixel {
        reasons.push("pixel_or_s0".into());
    } else if inp.has_vtid {
        reasons.push("has_vtid".into());
    } else {
        reasons.push("default_browser".into());
    }
    VisitClass {
        facet: "browser",
        robot_name: None,
        reasons,
    }
}

/// Classify for business dashboard display (compat wrapper).
pub fn classify_visitor_facet(ua: &str, is_pixel: bool, has_fe_main: bool) -> &'static str {
    classify_visit_class(&ClassInputs {
        ua: ua.to_string(),
        is_pixel,
        has_fe_main,
        ..Default::default()
    })
    .facet
}

/// Map any stored/legacy facet to binary display bucket (read-side only).
pub fn to_binary_facet(f: &str) -> &'static str {
    match f {
        "robots" => "robots",
        "browser" | "js" | "nojs" => "browser",
        _ => "browser",
    }
}

/// Display-only visitor facet helpers.
#[allow(dead_code)]
pub fn facet_from_meta(meta: &Value) -> Option<&'static str> {
    meta.get("visitor_facet")
        .and_then(|v| v.as_str())
        .map(to_binary_facet)
}

/// Merge visitor_facet (+ robot_name) into meta. Sticky: robots > browser.
pub fn merge_facet_meta(meta: Option<Value>, ua: &str, is_pixel: bool, has_fe_main: bool) -> Value {
    merge_visit_class(
        meta,
        &ClassInputs {
            ua: ua.to_string(),
            is_pixel,
            has_fe_main,
            ..Default::default()
        },
    )
}

/// Full multi-source merge. Existing **robots** never demotes; legacy js/nojs → browser.
pub fn merge_visit_class(meta: Option<Value>, inp: &ClassInputs) -> Value {
    let mut m: Map<String, Value> = meta
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    let existing_raw = m
        .get("visitor_facet")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let existing = to_binary_facet(existing_raw);
    let next = classify_visit_class(inp);
    let final_f = match (existing, next.facet) {
        ("robots", _) => "robots",
        (_, "robots") => "robots",
        _ => "browser",
    };
    m.insert("visitor_facet".into(), json!(final_f));
    if final_f == "robots" {
        let name = next
            .robot_name
            .clone()
            .or_else(|| {
                m.get("robot_name")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
            })
            .or_else(|| extract_robot_name(&inp.ua));
        if let Some(n) = name {
            m.insert("robot_name".into(), json!(n));
        }
    }
    m.insert("class_reasons".into(), json!(next.reasons));
    m.insert(
        "visitor_facet_note".into(),
        json!("binary_browser_or_robots_v2"),
    );
    Value::Object(m)
}

/// Sticky facet when upserting into biz_visits (new write schema).
pub fn sticky_facet(existing: Option<&str>, incoming: &str) -> String {
    let e = existing.map(to_binary_facet).unwrap_or("browser");
    let i = to_binary_facet(incoming);
    if e == "robots" || i == "robots" {
        "robots".into()
    } else {
        "browser".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn robots_ua_named() {
        let c = classify_visit_class(&ClassInputs {
            ua: "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)".into(),
            ..Default::default()
        });
        assert_eq!(c.facet, "robots");
        assert_eq!(c.robot_name.as_deref(), Some("Googlebot"));
    }

    #[test]
    fn robots_ua() {
        assert_eq!(
            classify_visitor_facet("Mozilla/5.0 (compatible; Googlebot/2.1)", false, true),
            "robots"
        );
    }

    #[test]
    fn pixel_is_browser() {
        assert_eq!(
            classify_visitor_facet("Mozilla/5.0 Chrome/120", true, false),
            "browser"
        );
    }

    #[test]
    fn fe_is_browser() {
        assert_eq!(
            classify_visitor_facet("Mozilla/5.0 Chrome/120", false, true),
            "browser"
        );
    }

    #[test]
    fn gateway_only_is_browser() {
        let c = classify_visit_class(&ClassInputs {
            ua: "Mozilla/5.0 Chrome/120".into(),
            has_vtid: true,
            ..Default::default()
        });
        assert_eq!(c.facet, "browser");
    }

    #[test]
    fn sticky_robots() {
        let m = merge_visit_class(
            Some(json!({"visitor_facet": "robots", "robot_name": "Googlebot"})),
            &ClassInputs {
                ua: "Mozilla/5.0 Chrome/120".into(),
                has_fe_main: true,
                ..Default::default()
            },
        );
        assert_eq!(
            m.get("visitor_facet").and_then(|v| v.as_str()),
            Some("robots")
        );
    }

    #[test]
    fn legacy_js_maps_to_browser_bucket() {
        assert_eq!(to_binary_facet("js"), "browser");
        assert_eq!(to_binary_facet("nojs"), "browser");
        assert_eq!(sticky_facet(Some("nojs"), "browser"), "browser");
        assert_eq!(sticky_facet(Some("robots"), "browser"), "robots");
    }
}
