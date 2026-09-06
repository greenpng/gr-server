//! Bot scoring — default = balanced (v9-aligned).

use regex::Regex;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct BotScore {
    pub score: i32,
    pub verdict: String,
    pub flags: Vec<String>,
    pub family: String,
    pub algo: String,
    pub details: Value,
    /// Named crawler when UA clearly declares one (Googlebot, Bingbot, …).
    pub robot_name: Option<String>,
}

impl BotScore {
    pub fn to_value(&self) -> Value {
        json!({
            "score": self.score,
            "verdict": self.verdict,
            "flags": self.flags,
            "family": self.family,
            "algo": self.algo,
            "details": self.details,
            "robot_name": self.robot_name,
        })
    }
}

/// Best-effort named robot from UA (product + dashboard).
pub fn robot_name_from_ua(ua: &str) -> Option<String> {
    let ual = ua.to_ascii_lowercase();
    if ual.is_empty() {
        return None;
    }
    const PAIRS: &[(&str, &str)] = &[
        ("googlebot", "Googlebot"),
        ("bingbot", "Bingbot"),
        ("bingpreview", "BingPreview"),
        ("baiduspider", "Baiduspider"),
        ("yandexbot", "YandexBot"),
        ("yandex", "Yandex"),
        ("duckduckbot", "DuckDuckBot"),
        ("applebot", "Applebot"),
        ("facebookexternalhit", "FacebookBot"),
        ("semrushbot", "SemrushBot"),
        ("ahrefsbot", "AhrefsBot"),
        ("gptbot", "GPTBot"),
        ("claudebot", "ClaudeBot"),
        ("bytespider", "Bytespider"),
        ("petalsearch", "PetalBot"),
        ("sogou", "Sogou"),
        ("slurp", "YahooSlurp"),
    ];
    for (n, name) in PAIRS {
        if ual.contains(n) {
            return Some((*name).to_string());
        }
    }
    if ua_declares_bot(ua) {
        return Some("crawler".into());
    }
    None
}

const STRONG_FLAGS: &[&str] = &[
    "webdriver_true",
    "webdriver_descriptor_tampered",
    "playwright_marker",
    "selenium_or_cdc",
    "phantom_marker",
    "headless_ua",
    "crawler_bot_ua",
];

fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0) != 0.0,
        Some(Value::String(s)) => {
            let t = s.trim().to_ascii_lowercase();
            matches!(t.as_str(), "1" | "true" | "yes" | "on" | "present" | "detected")
        }
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

fn field_or_auto(fields: &Map<String, Value>, auto: &Map<String, Value>, key: &str) -> bool {
    if fields.contains_key(key) {
        truthy(fields.get(key))
    } else {
        truthy(auto.get(key))
    }
}

struct Signals {
    ua: String,
    webdriver: bool,
    playwright: bool,
    selenium: bool,
    cdc: bool,
    phantom: bool,
    headless_ua: bool,
    bot_ua: bool,
    env_headless: bool,
    outer_zero: bool,
    swiftshader: bool,
    chrome_runtime_missing: bool,
    claims_chrome: bool,
    no_webgl: bool,
    hardware_concurrency: Option<i64>,
    screen_width: Option<i64>,
    languages: Option<Vec<String>>,
    timezone: Option<String>,
    renderer: String,
    descriptor_tampered: bool,
}

fn truthy_nested(fields: &Map<String, Value>, parent: &str, key: &str) -> bool {
    fields
        .get(parent)
        .and_then(|v| v.get(key))
        .map(|v| truthy(Some(v)))
        .unwrap_or(false)
}

/// B26 `webdriver_descriptor` (CreepJS / FPScanner).
/// FE prefers `Navigator.prototype` descriptor first, so automation hooks often
/// land as `{own:false, has_getter:true, getter_native:false}` — that must count.
/// Clean engines: own=false, native getter, not writable → not tampered.
fn webdriver_descriptor_tampered(fields: &Map<String, Value>) -> bool {
    let d = fields.get("webdriver_descriptor").or_else(|| {
        fields
            .get("automation_globals_v2")
            .and_then(|v| v.get("descriptor"))
    });
    let Some(obj) = d.and_then(|v| v.as_object()) else {
        return false;
    };
    if obj.get("error").and_then(|v| v.as_bool()).unwrap_or(false) {
        return false;
    }
    if obj.get("present").and_then(|v| v.as_bool()) == Some(false) {
        return false;
    }
    let own = obj.get("own").and_then(|v| v.as_bool()).unwrap_or(false);
    let writable = obj.get("writable").and_then(|v| v.as_bool()).unwrap_or(false);
    let has_getter = obj.get("has_getter").and_then(|v| v.as_bool()).unwrap_or(false);
    let getter_native = obj
        .get("getter_native")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    own || writable || (has_getter && !getter_native)
}

/// Generic bot-declaring words, word-boundary anchored (iss/opus5 P0-3).
/// The previous unanchored `bot|crawler|spider|...` matched substrings inside
/// legitimate device-brand UAs ("CUBOT", "Abbott", …), false-flagging real
/// users as crawlers.
fn re_bot_generic() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(?:^|[^a-z])(?:bot|crawler|spider|slurp)(?:[^a-z]|$)").unwrap()
    })
}

/// Unambiguous automation/crawler signatures — safe as plain substrings.
fn re_bot_known() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)bytespider|gptbot|claudebot|googlebot|bingbot|baiduspider|yandexbot|duckduckbot|applebot|semrushbot|ahrefsbot|petalsearch|curl/|wget|python-requests|go-http-client")
            .unwrap()
    })
}

/// Known legitimate UA tokens containing bot-like substrings (device brands,
/// proper nouns). Only consulted for the generic-word path; known-crawler
/// signatures above always count.
fn re_bot_whitelist() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)CUBOT|Abbott|Botswana|Robotis|iRobot|Botvac").unwrap())
}

/// UA declares itself a bot/crawler (anchored + whitelist aware).
fn ua_declares_bot(ua: &str) -> bool {
    if re_bot_known().is_match(ua) {
        return true;
    }
    if re_bot_whitelist().is_match(ua) {
        return false;
    }
    re_bot_generic().is_match(ua)
}

fn re_headless() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)HeadlessChrome|headless").unwrap())
}

fn re_swift() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)SwiftShader|llvmpipe|softpipe|VirtualBox|Microsoft Basic Render").unwrap()
    })
}

fn re_chrome() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)Chrome/|Chromium").unwrap())
}

fn extract_signals(fields: &Value) -> Signals {
    let f = fields.as_object().cloned().unwrap_or_default();
    let auto = f
        .get("automation")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let ua = f
        .get("user_agent")
        .or_else(|| f.get("ua"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let renderer = f
        .get("webgl_unmasked_renderer")
        .or_else(|| f.get("renderer"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let langs: Option<Vec<String>> = match f.get("languages") {
        Some(Value::Array(a)) => Some(
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect(),
        ),
        Some(Value::String(s)) => Some(s.split(',').map(|x| x.trim().to_string()).collect()),
        _ => None,
    };
    let hc = f
        .get("hardware_concurrency")
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)));
    let sw = f
        .get("screen_width")
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)));
    let claims_chrome = re_chrome().is_match(&ua);
    let no_webgl = truthy(f.get("no_webgl")) || (renderer.is_empty() && claims_chrome && re_chrome().is_match(&ua));
    let chrome_runtime_missing = auto.get("chrome_runtime") == Some(&Value::Bool(false))
        || truthy(f.get("chrome_runtime_missing"));
    let mode = f
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    Signals {
        ua: ua.clone(),
        webdriver: field_or_auto(&f, &auto, "webdriver")
            || truthy(f.get("agent_has_webdriver"))
            || truthy_nested(&f, "automation_globals_v2", "webdriver"),
        playwright: field_or_auto(&f, &auto, "playwright")
            || truthy(f.get("agent_has_playwright"))
            || truthy_nested(&f, "automation_globals_v2", "playwright"),
        selenium: field_or_auto(&f, &auto, "selenium")
            || truthy(f.get("agent_has_selenium"))
            || truthy_nested(&f, "automation_globals_v2", "selenium"),
        cdc: field_or_auto(&f, &auto, "cdc")
            || truthy(f.get("agent_has_cdc"))
            || truthy_nested(&f, "automation_globals_v2", "cdc"),
        phantom: field_or_auto(&f, &auto, "phantom")
            || truthy(f.get("agent_has_phantom"))
            || truthy_nested(&f, "automation_globals_v2", "phantom"),
        headless_ua: re_headless().is_match(&ua),
        bot_ua: ua_declares_bot(&ua),
        env_headless: mode == "headless" || truthy(f.get("headless")),
        outer_zero: truthy(f.get("outer_zero")) || truthy(auto.get("outer_zero")),
        swiftshader: re_swift().is_match(&renderer),
        chrome_runtime_missing,
        claims_chrome,
        no_webgl,
        hardware_concurrency: hc,
        screen_width: sw,
        languages: langs,
        timezone: f.get("timezone").and_then(|v| v.as_str()).map(|s| s.to_string()),
        renderer,
        descriptor_tampered: webdriver_descriptor_tampered(&f),
    }
}

fn band(
    score: i32,
    flags: Vec<String>,
    fam: HashMap<String, i32>,
    algo: &str,
    robot_name: Option<String>,
) -> BotScore {
    let score = score.clamp(0, 100);
    let max_fam = || {
        fam.iter()
            .max_by_key(|(_, v)| *v)
            .map(|(k, _)| k.clone())
            .unwrap_or_else(|| "mixed".into())
    };
    // iss/opus5 03-P2-6: verdict thresholds externalized (signed spec).
    let w = crate::bot_weights::active();
    let (cf, tb, ts, tw) = w.thresholds();
    let (verdict, family) = if flags.iter().any(|x| x == "crawler_bot_ua") && score >= cf {
        ("crawler".into(), "crawler".into())
    } else if score >= tb {
        ("bot".into(), max_fam())
    } else if score >= ts {
        ("suspect".into(), max_fam())
    } else if score >= tw {
        ("watch".into(), max_fam())
    } else {
        ("human".into(), "human".into())
    };
    let has_strong = flags.iter().any(|f| STRONG_FLAGS.contains(&f.as_str()));
    let mut details = json!({ "has_strong": has_strong });
    if let Some(d) = details.as_object_mut() {
        if let Some(meta) = crate::bot_weights::meta_value().as_object() {
            for (k, v) in meta {
                d.insert(k.clone(), v.clone());
            }
        }
    }
    BotScore {
        score,
        verdict,
        flags,
        family,
        algo: algo.into(),
        details,
        robot_name,
    }
}

fn site_locale(fields: &Value) -> String {
    fields
        .get("site_locale")
        .and_then(|v| v.as_str())
        .unwrap_or("zh")
        .to_string()
}

/// Shared accumulation for one weights table: returns (score, flags, family).
/// iss/opus5 03-P2-6: every rule weight comes from the (signed) spec table; the
/// same routine runs under the builtin table for the shadow record.
fn baseline_accumulate(s: &Signals, fields: &Value, w: &crate::bot_weights::BotWeights) -> (i32, Vec<String>, HashMap<String, i32>) {
    let mut score = 0i32;
    let mut flags = Vec::new();
    let mut fam: HashMap<String, i32> = HashMap::new();
    let mut add = |pts: i32, flag: &str, family: Option<&str>| {
        score += pts;
        flags.push(flag.to_string());
        if let Some(f) = family {
            *fam.entry(f.to_string()).or_default() += pts;
        }
    };
    if s.webdriver {
        add(w.flag_pts("baseline", "webdriver_true", false), "webdriver_true", Some("headless"));
    }
    if s.descriptor_tampered {
        add(w.flag_pts("baseline", "webdriver_descriptor_tampered", false), "webdriver_descriptor_tampered", Some("headless"));
    }
    if s.playwright {
        add(w.flag_pts("baseline", "playwright_marker", false), "playwright_marker", Some("playwright"));
    }
    if s.selenium || s.cdc {
        add(w.flag_pts("baseline", "selenium_or_cdc", false), "selenium_or_cdc", Some("selenium"));
    }
    if s.phantom {
        add(w.flag_pts("baseline", "phantom_marker", false), "phantom_marker", Some("headless"));
    }
    if s.headless_ua {
        add(w.flag_pts("baseline", "headless_ua", false), "headless_ua", Some("headless"));
    }
    if s.env_headless {
        add(w.flag_pts("baseline", "session_mode_headless", false), "session_mode_headless", Some("headless"));
    }
    if s.bot_ua {
        add(w.flag_pts("baseline", "crawler_bot_ua", false), "crawler_bot_ua", Some("crawler"));
    }
    if s.outer_zero {
        add(w.flag_pts("baseline", "outer_window_zero", false), "outer_window_zero", Some("headless"));
    }
    if s.swiftshader {
        add(w.flag_pts("baseline", "gpu_swiftshader_or_software", false), "gpu_swiftshader_or_software", Some("headless"));
    }
    // Demo→v5: residual vs high-end GPU label spoof (fp-browser) — authenticity, not crawler.
    {
        use crate::stack_auth::stack_auth_from_fields;
        let auth = stack_auth_from_fields(fields);
        if auth.gpu_label_untrusted || auth.spoof_score >= 0.4 {
            add(w.flag_pts("baseline", "gpu_label_spoof_or_soft_residual", false), "gpu_label_spoof_or_soft_residual", Some("spoof"));
        } else if auth.soft_stack && !s.swiftshader {
            // residual soft without soft label string (camoufox path after residual only)
            add(w.flag_pts("baseline", "soft_stack_residual", false), "soft_stack_residual", Some("spoof"));
        }
    }
    let sl = site_locale(fields);
    if sl.starts_with("zh") {
        if let Some(ref langs) = s.languages {
            if !langs.is_empty()
                && langs
                    .iter()
                    .all(|x| x.to_ascii_lowercase().starts_with("en"))
            {
                add(w.flag_pts("baseline", "en_only_on_zh_site", false), "en_only_on_zh_site", Some("watch"));
            }
        }
        if matches!(s.timezone.as_deref(), Some("UTC") | Some("Etc/UTC")) {
            add(w.flag_pts("baseline", "utc_timezone_on_zh_site", false), "utc_timezone_on_zh_site", Some("crawler"));
        }
    }
    (score, flags, fam)
}

/// Shared accumulation for one weights table (balanced rule set).
fn balanced_accumulate(s: &Signals, fields: &Value, w: &crate::bot_weights::BotWeights) -> (i32, Vec<String>, HashMap<String, i32>) {
    let mut score = 0i32;
    let mut flags = Vec::new();
    let mut fam: HashMap<String, i32> = HashMap::new();
    let mut add = |pts: i32, flag: &str, family: Option<&str>| {
        score += pts;
        flags.push(flag.to_string());
        if let Some(f) = family {
            *fam.entry(f.to_string()).or_default() += pts;
        }
    };
    if s.webdriver {
        add(w.flag_pts("balanced", "webdriver_true", false), "webdriver_true", Some("headless"));
    }
    if s.descriptor_tampered {
        add(w.flag_pts("balanced", "webdriver_descriptor_tampered", false), "webdriver_descriptor_tampered", Some("headless"));
    }
    if s.playwright {
        add(w.flag_pts("balanced", "playwright_marker", false), "playwright_marker", Some("playwright"));
    }
    if s.selenium || s.cdc {
        add(w.flag_pts("balanced", "selenium_or_cdc", false), "selenium_or_cdc", Some("selenium"));
    }
    if s.phantom {
        add(w.flag_pts("balanced", "phantom_marker", false), "phantom_marker", Some("headless"));
    }
    if s.headless_ua {
        add(w.flag_pts("balanced", "headless_ua", false), "headless_ua", Some("headless"));
    }
    if s.env_headless {
        add(w.flag_pts("balanced", "session_mode_headless", false), "session_mode_headless", Some("headless"));
    }
    if s.bot_ua {
        add(w.flag_pts("balanced", "crawler_bot_ua", false), "crawler_bot_ua", Some("crawler"));
    }
    if s.outer_zero {
        add(w.flag_pts("balanced", "outer_window_zero", false), "outer_window_zero", Some("headless"));
    }
    if s.swiftshader {
        let pts = w.flag_pts("balanced", "gpu_swiftshader_or_software", s.env_headless || s.headless_ua);
        add(pts, "gpu_swiftshader_or_software", Some("headless"));
    }
    // Demo→v5 residual vs GPU label spoof (balanced path used in evaluate)
    {
        use crate::stack_auth::stack_auth_from_fields;
        let auth = stack_auth_from_fields(fields);
        if auth.gpu_label_untrusted || auth.spoof_score >= 0.4 {
            add(w.flag_pts("balanced", "gpu_label_spoof_or_soft_residual", false), "gpu_label_spoof_or_soft_residual", Some("spoof"));
        } else if auth.soft_stack && !s.swiftshader {
            add(w.flag_pts("balanced", "soft_stack_residual", false), "soft_stack_residual", Some("spoof"));
        }
    }
    if s.chrome_runtime_missing && s.claims_chrome && !s.bot_ua {
        add(w.flag_pts("balanced", "chrome_runtime_missing", false), "chrome_runtime_missing", Some("spoofed"));
    }
    if s.no_webgl && s.claims_chrome {
        add(w.flag_pts("balanced", "webgl_missing_while_chrome", false), "webgl_missing_while_chrome", Some("spoofed"));
    }
    if let (Some(hc), Some(sw)) = (s.hardware_concurrency, s.screen_width) {
        if hc >= 32 && sw <= 1024 {
            add(w.flag_pts("balanced", "high_cores_small_screen", false), "high_cores_small_screen", Some("headless"));
        }
    }
    if s.hardware_concurrency.is_some_and(|hc| hc >= 64) {
        add(w.flag_pts("balanced", "extreme_hardware_concurrency", false), "extreme_hardware_concurrency", Some("headless"));
    }
    let sl = site_locale(fields);
    if sl.starts_with("zh") {
        if let Some(ref langs) = s.languages {
            if !langs.is_empty()
                && langs
                    .iter()
                    .all(|x| x.to_ascii_lowercase().starts_with("en"))
            {
                add(w.flag_pts("balanced", "en_only_on_zh_site", false), "en_only_on_zh_site", Some("watch"));
            }
        }
        if matches!(s.timezone.as_deref(), Some("UTC") | Some("Etc/UTC")) {
            add(w.flag_pts("balanced", "utc_timezone_on_zh_site", false), "utc_timezone_on_zh_site", Some("crawler"));
        }
    }
    (score, flags, fam)
}

pub fn score_bot_baseline(fields: &Value) -> BotScore {
    let s = extract_signals(fields);
    let w = crate::bot_weights::active();
    let (score, flags, fam) = baseline_accumulate(&s, fields, w);
    let rname = robot_name_from_ua(&s.ua);
    band(score, flags, fam, "baseline", rname)
}

pub fn score_bot_balanced(fields: &Value) -> BotScore {
    let s = extract_signals(fields);
    let w = crate::bot_weights::active();
    let (score, flags, fam) = balanced_accumulate(&s, fields, w);
    let rname = robot_name_from_ua(&s.ua);
    let mut out = band(score, flags, fam, "balanced", rname);
    // Shadow A/B (iss/opus5 03-P2-6): when enabled, the candidate spec table is
    // computed record-only — the returned verdict still belongs to the old weights.
    if crate::bot_weights::shadow_mode() && crate::bot_weights::candidate().source != w.source {
        let sw = crate::bot_weights::candidate();
        let (sscore, _, _) = balanced_accumulate(&s, fields, sw);
        let (sverdict, _) = crate::bot_weights::verdict_with(sw, sscore, &[]);
        if let Some(obj) = out.details.as_object_mut() {
            obj.insert(
                "shadow".into(),
                json!({
                    "candidate_source": sw.source,
                    "candidate_version": sw.version,
                    "shadow_score": sscore,
                    "shadow_verdict": sverdict,
                    "score_delta": sscore - score,
                    "mode": "record_only",
                }),
            );
        }
    }
    let signals = json!({
        "ua": s.ua,
        "webdriver": s.webdriver,
        "playwright": s.playwright,
        "selenium": s.selenium,
        "cdc": s.cdc,
        "phantom": s.phantom,
        "headless_ua": s.headless_ua,
        "bot_ua": s.bot_ua,
        "env_headless": s.env_headless,
        "outer_zero": s.outer_zero,
        "swiftshader": s.swiftshader,
        "chrome_runtime_missing": s.chrome_runtime_missing,
        "claims_chrome": s.claims_chrome,
        "no_webgl": s.no_webgl,
        "hardware_concurrency": s.hardware_concurrency,
        "screen_width": s.screen_width,
        "languages": s.languages,
        "timezone": s.timezone,
        "renderer": s.renderer,
        "descriptor_tampered": s.descriptor_tampered,
    });
    if let Some(obj) = out.details.as_object_mut() {
        obj.insert("signals".into(), signals);
    }
    out
}

pub fn score_bot(fields: &Value, algo: &str) -> Result<BotScore, String> {
    match algo {
        "baseline" => Ok(score_bot_baseline(fields)),
        "balanced" => Ok(score_bot_balanced(fields)),
        other => Err(format!("unsupported bot algo: {other}")),
    }
}

pub fn has_strong_bot(score: &BotScore) -> bool {
    score
        .details
        .get("has_strong")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || matches!(score.verdict.as_str(), "bot" | "crawler")
}

#[cfg(test)]
mod uploaded_field_consume_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn score_bot_consumes_webdriver_descriptor_and_agent_has() {
        let clean = json!({
            "user_agent": "Mozilla/5.0 Chrome/120.0.0.0",
            "webdriver": false,
        });
        let clean_s = score_bot_balanced(&clean);
        assert!(
            !clean_s.flags.iter().any(|f| f == "webdriver_descriptor_tampered"),
            "{:?}",
            clean_s.flags
        );

        // Instance override (own:true).
        let own_inst = json!({
            "user_agent": "Mozilla/5.0 Chrome/120.0.0.0",
            "webdriver": false,
            "webdriver_descriptor": {
                "present": true,
                "own": true,
                "writable": true,
                "has_getter": true,
                "getter_native": false
            }
        });
        let t_own = score_bot_balanced(&own_inst);
        assert!(
            t_own.flags.iter().any(|f| f == "webdriver_descriptor_tampered"),
            "own-instance descriptor must set flag flags={:?}",
            t_own.flags
        );
        assert!(t_own.score > clean_s.score);

        // Real FE prototype-first hook (registry.mid.core.js): own=false.
        let proto_hook = json!({
            "user_agent": "Mozilla/5.0 Chrome/120.0.0.0",
            "webdriver": false,
            "webdriver_descriptor": {
                "present": true,
                "own": false,
                "writable": false,
                "has_getter": true,
                "getter_native": false
            }
        });
        let t_proto = score_bot_balanced(&proto_hook);
        assert!(
            t_proto.flags.iter().any(|f| f == "webdriver_descriptor_tampered"),
            "FE prototype-hook shape (own:false, non-native getter) must set flag flags={:?}",
            t_proto.flags
        );
        assert!(t_proto.score > clean_s.score);

        // Clean prototype native getter must not fire.
        let native_proto = json!({
            "user_agent": "Mozilla/5.0 Chrome/120.0.0.0",
            "webdriver": false,
            "webdriver_descriptor": {
                "present": true,
                "own": false,
                "writable": false,
                "has_getter": true,
                "getter_native": true
            }
        });
        let t_nat = score_bot_balanced(&native_proto);
        assert!(
            !t_nat.flags.iter().any(|f| f == "webdriver_descriptor_tampered"),
            "native prototype getter must not set flag flags={:?}",
            t_nat.flags
        );

        let pw = json!({
            "user_agent": "Mozilla/5.0 Chrome/120.0.0.0",
            "agent_has_playwright": true,
            "automation_globals_v2": {"playwright": true, "hit_n": 1}
        });
        let p = score_bot_balanced(&pw);
        assert!(
            p.flags.iter().any(|f| f == "playwright_marker"),
            "agent_has_playwright must reach score_bot_balanced flags={:?}",
            p.flags
        );
    }

    #[test]
    fn bot_ua_regex_anchored_no_brand_false_positive() {
        // iss/opus5 P0-3: real device-brand UAs containing "bot" substrings
        // must NOT be flagged as crawlers.
        for ua in [
            "Mozilla/5.0 (Linux; Android 13; CUBOT KINGKONG 9) AppleWebKit/537.36 Chrome/120.0 Mobile Safari/537.36",
            "Mozilla/5.0 (X11; Linux x86_64) Abbott Diagnostics Viewer/2.1",
            "Mozilla/5.0 (Windows NT 10.0) BotswanaPost Tracker/1.0",
            "Mozilla/5.0 (Linux; Android 12) iRobot Home/5.2",
        ] {
            assert!(!ua_declares_bot(ua), "false positive: {ua}");
            assert_eq!(robot_name_from_ua(ua), None, "false positive: {ua}");
        }
    }

    #[test]
    fn bot_ua_regex_still_catches_real_crawlers() {
        for (ua, named) in [
            ("Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)", true),
            ("Mozilla/5.0 (compatible; bingbot/2.0; +http://www.bing.com/bingbot.htm)", true),
            ("Bytespider; spider@bytedance.com", true),
            ("Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; GPTBot/1.2; +https://openai.com/gptbot)", true),
            ("curl/8.4.0", false),
            ("python-requests/2.31.0", false),
            ("Go-http-client/1.1", false),
            ("Wget/1.21.3", false),
            ("Some random spider bot", false),
            ("AhrefsBot/7.0", true),
        ] {
            assert!(ua_declares_bot(ua), "missed crawler: {ua}");
            if named {
                assert!(robot_name_from_ua(ua).is_some(), "missed named robot: {ua}");
            }
        }
        // Clean browser UAs stay clean.
        for ua in [
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15",
        ] {
            assert!(!ua_declares_bot(ua), "false positive: {ua}");
        }
    }
}
