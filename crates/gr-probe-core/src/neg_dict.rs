//! H15 negative / spoof dictionary — known abnormal surface codes.
//!
//! FE may upload `neg_dict_hits`; server also re-derives hits from core fields
//! so dictionary is not FE-only trust.

use serde_json::{Map, Value};

/// Codes that indicate automation / soft / spoof surfaces.
pub const NEG_DICT_CODES: &[&str] = &[
    "webdriver",
    "outer_zero",
    "plugins_empty",
    "languages_empty",
    "chrome_missing",
    "callPhantom",
    "selenium",
    "puppeteer",
    "cdc_prop",
    "headless_ua",
    "webgl_soft_label",
    "agent_automation_globals",
    "caps_claim_vs_actual",
    "native_integrity_low",
    "ja4_vs_ua_engine_mismatch",
];

fn boolish(fo: &Map<String, Value>, key: &str) -> bool {
    match fo.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0) != 0.0,
        Some(Value::String(s)) => !s.is_empty() && s != "0" && s != "false",
        _ => false,
    }
}

fn f64_field(fo: &Map<String, Value>, key: &str) -> Option<f64> {
    fo.get(key).and_then(|v| {
        v.as_f64()
            .or_else(|| v.as_i64().map(|i| i as f64))
            .or_else(|| v.as_u64().map(|u| u as f64))
    })
}

/// Derive dictionary hits from field map (server authority).
pub fn derive_neg_dict_hits(fo: &Map<String, Value>) -> Vec<String> {
    let mut hits = Vec::new();
    if boolish(fo, "webdriver") {
        hits.push("webdriver".into());
    }
    if boolish(fo, "outer_zero") {
        hits.push("outer_zero".into());
    }
    if f64_field(fo, "plugins_length").unwrap_or(1.0) == 0.0 {
        hits.push("plugins_empty".into());
    }
    if fo.get("languages")
        .and_then(|v| v.as_array())
        .map(|a| a.is_empty())
        .unwrap_or(false)
    {
        hits.push("languages_empty".into());
    }
    let ua = fo
        .get("user_agent")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if ua.contains("Chrome/") && fo.get("chrome_runtime").and_then(|v| v.as_bool()) == Some(false) {
        // only if explicitly false
        hits.push("chrome_missing".into());
    }
    if ua.to_ascii_lowercase().contains("headless")
        || ua.contains("PhantomJS")
        || ua.contains("Electron")
    {
        hits.push("headless_ua".into());
    }
    let renderer = fo
        .get("webgl_unmasked_renderer")
        .or_else(|| fo.get("renderer"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if renderer.contains("swiftshader")
        || renderer.contains("llvmpipe")
        || renderer.contains("softpipe")
        || renderer.contains("basic render")
    {
        hits.push("webgl_soft_label".into());
    }
    if f64_field(fo, "agent_automation_globals_n").unwrap_or(0.0) >= 1.0 {
        hits.push("agent_automation_globals".into());
    }
    if boolish(fo, "caps_claim_vs_actual") {
        hits.push("caps_claim_vs_actual".into());
    }
    if f64_field(fo, "native_integrity_ratio").unwrap_or(1.0) < 0.7 {
        hits.push("native_integrity_low".into());
    }
    // Merge FE-reported hits
    if let Some(arr) = fo.get("neg_dict_hits").and_then(|v| v.as_array()) {
        for v in arr {
            if let Some(s) = v.as_str() {
                if NEG_DICT_CODES.contains(&s) && !hits.iter().any(|h| h == s) {
                    hits.push(s.to_string());
                }
            }
        }
    }
    hits.sort();
    hits.dedup();
    hits
}

/// Score: more hits → higher spoof contribution (0..1).
pub fn neg_dict_spoof_boost(hits: &[String]) -> f64 {
    if hits.is_empty() {
        return 0.0;
    }
    // High-weight codes
    let mut w: f64 = 0.0;
    for h in hits {
        w += match h.as_str() {
            "webdriver" | "puppeteer" | "selenium" | "callPhantom" | "cdc_prop" => 0.2,
            "headless_ua" | "agent_automation_globals" => 0.15,
            "webgl_soft_label" | "caps_claim_vs_actual" | "native_integrity_low" => 0.12,
            "outer_zero" | "plugins_empty" | "chrome_missing" => 0.08,
            _ => 0.05,
        };
    }
    w.min(0.85_f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn derives_webdriver_and_soft_label() {
        let fo = json!({
            "webdriver": true,
            "webgl_unmasked_renderer": "Google SwiftShader",
            "neg_dict_hits": ["outer_zero"],
        })
        .as_object()
        .cloned()
        .unwrap();
        let hits = derive_neg_dict_hits(&fo);
        assert!(hits.iter().any(|h| h == "webdriver"));
        assert!(hits.iter().any(|h| h == "webgl_soft_label"));
        assert!(hits.iter().any(|h| h == "outer_zero"));
        assert!(neg_dict_spoof_boost(&hits) >= 0.3);
    }
}
