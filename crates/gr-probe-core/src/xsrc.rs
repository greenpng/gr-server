//! Cross-source truth + real_band evaluation (R-TRU / Phase C defaults).

use crate::bot::{has_strong_bot, score_bot, BotScore};
use crate::contracts::{batch_ids_from_evidence, load_all_specs, present_packages, ContractError};
use regex::Regex;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::sync::OnceLock;

pub const XSRC_CONSISTENT: &str = "consistent";
pub const XSRC_CONFLICT: &str = "conflict";
pub const XSRC_MISSING_SERVER: &str = "missing_server";
pub const XSRC_MISSING_BROWSER: &str = "missing_browser";
pub const XSRC_PARTIAL: &str = "partial";

#[derive(Debug, Clone)]
pub struct TruthResult {
    pub xsrc_status: String,
    pub real_band: String,
    pub credibility: f64,
    pub fe_only: bool,
    pub has_server_side: bool,
    pub has_main_core: bool,
    pub packages: Value,
    pub reasons: Vec<String>,
    pub details: Value,
}

impl TruthResult {
    pub fn to_value(&self) -> Value {
        json!({
            "xsrc_status": self.xsrc_status,
            "real_band": self.real_band,
            "credibility": self.credibility,
            "fe_only": self.fe_only,
            "has_server_side": self.has_server_side,
            "has_main_core": self.has_main_core,
            "packages": self.packages,
            "reasons": self.reasons,
            "details": self.details,
        })
    }
}

fn sources_from_evidence(evidence: &Value) -> HashSet<String> {
    let mut src = HashSet::new();
    if let Some(arr) = evidence.get("sources").and_then(|v| v.as_array()) {
        for s in arr {
            if let Some(st) = s.as_str() {
                src.insert(st.split(':').next().unwrap_or(st).to_string());
            }
        }
    }
    if let Some(batches) = evidence.get("batches").and_then(|v| v.as_array()) {
        for b in batches {
            if let Some(st) = b.get("source").and_then(|v| v.as_str()) {
                src.insert(st.split(':').next().unwrap_or(st).to_string());
            }
        }
    }
    if evidence
        .get("has_gateway")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || evidence
            .get("b8_gateway")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    {
        src.insert("gateway".into());
    }
    // Do not promote client-only has_cloudflare/cf_edge flags into sources here.
    // Cloudflare is added only when batches/source list include it; has_server still
    // requires verified cf_fields (see has_trusted_server_side).
    let has_main_batch = evidence
        .get("batches")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter().any(|b| {
                b.get("source")
                    .and_then(|v| v.as_str())
                    .unwrap_or("main")
                    .starts_with("main")
            })
        })
        .unwrap_or(false);
    let sources_list_has_main = evidence
        .get("sources")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().any(|s| s.as_str() == Some("main")))
        .unwrap_or(false);
    if evidence.get("has_main").and_then(|v| v.as_bool()).unwrap_or(false)
        || sources_list_has_main
        || has_main_batch
    {
        src.insert("main".into());
    }
    if src.is_empty()
        && evidence
            .get("fields")
            .and_then(|f| f.as_object())
            .is_some_and(|o| !o.is_empty())
    {
        src.insert("main".into());
    }
    src
}

/// Cloudflare is server-side only when edge observation payload is present.
/// Client-asserted `source=cloudflare` or `has_cloudflare`/`cf_edge` flags alone do not count.
fn verified_cloudflare_fields(cf: &Map<String, Value>) -> bool {
    if cf.is_empty() {
        return false;
    }
    // Reject pure stubs / empty placeholders
    const EDGE_KEYS: &[&str] = &[
        "bot_score",
        "country",
        "cf_ray",
        "colo",
        "asn",
        "http_protocol",
        "client_tcp_rtt",
        "tls_version",
    ];
    EDGE_KEYS.iter().any(|k| {
        cf.get(*k).is_some_and(|v| match v {
            Value::Null => false,
            Value::String(s) => !s.is_empty(),
            Value::Bool(false) => false,
            _ => true,
        })
    })
}

/// Trusted server-side presence for band gates (gateway obs or verified CF edge).
fn has_trusted_server_side(
    sources: &HashSet<String>,
    gateway: &Map<String, Value>,
    cf: &Map<String, Value>,
    evidence: &Value,
) -> bool {
    let has_gateway_flag = evidence
        .get("has_gateway")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || evidence
            .get("b8_gateway")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let gateway_batch = evidence
        .get("batches")
        .and_then(|b| b.as_array())
        .map(|arr| {
            arr.iter().any(|b| {
                let src = b
                    .get("source")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .split(':')
                    .next()
                    .unwrap_or("");
                src == "gateway"
            })
        })
        .unwrap_or(false);

    let gateway_ok = !gateway.is_empty() || has_gateway_flag || gateway_batch;
    let cf_ok = verified_cloudflare_fields(cf);

    // Client listing "cloudflare" in sources without cf_fields does NOT count.
    // Client listing "gateway" with gateway batch/fields/flags does count (B8 path).
    let _claimed_cf = sources.contains("cloudflare");
    let _claimed_gw = sources.contains("gateway");

    gateway_ok || cf_ok
}

fn has_surface_pkg(fields: &Map<String, Value>) -> bool {
    let has_identity = fields
        .get("os_family")
        .or_else(|| fields.get("surface"))
        .or_else(|| fields.get("form_class"))
        .is_some_and(|v| !v.is_null() && v.as_str() != Some(""));
    let keys = [
        "webgl_unmasked_renderer",
        "screen_width",
        "screen_height",
        "timezone",
        "hardware_concurrency",
        "automation",
    ];
    let mut has_obs = keys.iter().any(|k| {
        fields.get(*k).is_some_and(|v| !v.is_null() && v.as_str() != Some(""))
    });
    if let Some(Value::Object(auto)) = fields.get("automation") {
        if !auto.is_empty() {
            has_obs = true;
        }
    }
    has_identity && has_obs
}

fn main_core_present(batch_ids: &HashSet<String>, fields: &Map<String, Value>) -> Result<bool, ContractError> {
    let specs = load_all_specs()?;
    let core = specs.main_core_batches();
    let core_hit: HashSet<&str> = batch_ids
        .iter()
        .filter(|b| core.contains(*b))
        .map(|s| s.as_str())
        .collect();
    // Short-visit honesty: a single B0 must not count as full main_core for confirmed_real.
    // v9/v11.3: usable main needs lite identity surface + automation/hardware anchors.
    if core_hit.len() >= 3 {
        return Ok(true);
    }
    if core_hit.contains("B0_bootstrap")
        && core_hit.contains("B1_conflict")
        && (core_hit.contains("B2_hardware") || core_hit.contains("B3_system"))
    {
        return Ok(true);
    }
    // Field-rich fallback when batches sparse but packages present (replayed corpus rows)
    let pkgs = present_packages(&Value::Object(fields.clone()), &HashSet::new())?;
    let auto = pkgs.get("PKG_AUTO").and_then(|v| v.as_bool()).unwrap_or(false);
    let link = pkgs.get("PKG_LINK").and_then(|v| v.as_bool()).unwrap_or(false);
    if has_surface_pkg(fields) && auto && link {
        return Ok(true);
    }
    Ok(false)
}

fn re_bot_ua() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)bot|crawler|spider").unwrap())
}

fn claim_obs_conflict(fields: &Map<String, Value>, gateway: &Map<String, Value>) -> Vec<String> {
    let mut conflicts = Vec::new();
    let claim_mobile = fields
        .get("form_class")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .eq_ignore_ascii_case("mobile");
    if claim_mobile {
        if let Some(sw) = fields
            .get("screen_width")
            .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
        {
            if sw >= 1600 {
                conflicts.push("form_claim_vs_screen".into());
            }
        }
    }
    let ua = fields
        .get("user_agent")
        .or_else(|| fields.get("ua"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let osf = fields
        .get("os_family")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ua.contains("Android") && matches!(osf.as_str(), "windows" | "mac" | "linux") {
        conflicts.push("ua_android_vs_desktop_os".into());
    }
    if ua.contains("Windows") && osf == "android" {
        conflicts.push("ua_windows_vs_android_os".into());
    }
    let gua = gateway
        .get("user_agent")
        .or_else(|| gateway.get("ua"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !gua.is_empty() && !ua.is_empty() {
        let g_bot = re_bot_ua().is_match(gua);
        let f_bot = re_bot_ua().is_match(ua);
        if g_bot != f_bot {
            conflicts.push("gateway_ua_bot_vs_fe".into());
        }
    }
    // H13-class: protocol fingerprint (JA4/H2) vs UA engine claim-obs.
    // Neutral when protocol fields absent — do not invent agreement or conflict.
    let ja4 = gateway
        .get("ja4")
        .or_else(|| gateway.get("tls_ja4"))
        .or_else(|| fields.get("ja4"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let h2_fp = gateway
        .get("h2_fingerprint")
        .or_else(|| gateway.get("http2_fingerprint"))
        .or_else(|| fields.get("h2_fingerprint"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let proto_engine = gateway
        .get("protocol_engine")
        .or_else(|| fields.get("protocol_engine"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !ja4.is_empty() || !h2_fp.is_empty() || !proto_engine.is_empty() {
        // iss/21 T-ENG-1: compare engine **families** (blink/gecko/webkit), not brand names.
        use crate::protocol_edge::{brand_to_engine_family, derive_engine_claim_obs};
        let (fe_family, _) = derive_engine_claim_obs(fields);
        let proto_family = brand_to_engine_family(&proto_engine);
        // Explicit protocol_engine from edge vs UA claim — family level only
        if !proto_engine.is_empty()
            && fe_family != "unknown"
            && proto_family != "unknown"
            && proto_family != fe_family
        {
            conflicts.push("ja4_vs_ua_engine_mismatch".into());
        }
        // Heuristic: JA4 prefix tags used by some lab injectors — family compare
        if !ja4.is_empty() && fe_family != "unknown" {
            let j = ja4.to_ascii_lowercase();
            let j_family = brand_to_engine_family(&j);
            if j_family != "unknown" && j_family != fe_family {
                conflicts.push("ja4_vs_ua_engine_mismatch".into());
            }
        }
        if !h2_fp.is_empty() && fe_family != "unknown" {
            let h = h2_fp.to_ascii_lowercase();
            let h_family = brand_to_engine_family(&h);
            if h_family != "unknown" && h_family != fe_family {
                conflicts.push("h2_vs_ua_engine_mismatch".into());
            }
        }
        // QUIC vs TCP JA4 (Pingora dual-path)
        let quic_ja4 = gateway
            .get("quic_tls_ja4")
            .or_else(|| fields.get("quic_tls_ja4"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !ja4.is_empty() && !quic_ja4.is_empty() && ja4 != quic_ja4 {
            conflicts.push("tcp_ja4_vs_quic_ja4_mismatch".into());
        }
    }
    // Side-channel present → record as observations (not always conflict)
    if fields
        .get("tcp_syn_present")
        .or_else(|| gateway.get("tcp_syn_present"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        // no conflict code — brain uses presence as material
    }
    // H3 app layer present → soft observation (strengthens multi-path protocol)
    let h3_app = gateway
        .get("h3_app_present")
        .or_else(|| fields.get("h3_app_present"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if h3_app {
        // no conflict by itself; used by product/stack hits
        let _ = gateway
            .get("h3_settings_fp")
            .or_else(|| fields.get("h3_settings_fp"));
    }

    // PROXY protocol real-client vs FE-claimed server_client_ip (soft consistency)
    let proxy_on = gateway
        .get("proxy_protocol_present")
        .or_else(|| fields.get("proxy_protocol_present"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if proxy_on {
        let proxy_ip = gateway
            .get("server_client_ip")
            .or_else(|| fields.get("server_client_ip"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let fe_ip = fields
            .get("client_ip_claim")
            .or_else(|| fields.get("webrtc_host_ip"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        // Only conflict when FE explicitly claims a public IP that disagrees with PROXY src
        if !proxy_ip.is_empty()
            && !fe_ip.is_empty()
            && proxy_ip != fe_ip
            && !fe_ip.starts_with("127.")
            && !fe_ip.starts_with("10.")
            && !fe_ip.starts_with("192.168.")
        {
            conflicts.push("proxy_ip_vs_fe_ip_claim".into());
        }
    }
    conflicts
}

pub fn evaluate_xsrc(
    evidence: &Value,
    bot: Option<&BotScore>,
) -> Result<TruthResult, ContractError> {
    let specs = load_all_specs()?;
    let fields = evidence
        .get("fields")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let gateway = evidence
        .get("gateway_fields")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let cf = evidence
        .get("cf_fields")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let sources = sources_from_evidence(evidence);
    let batch_ids: HashSet<String> = batch_ids_from_evidence(evidence).into_iter().collect();
    let browser_side = specs.browser_side();
    // Never trust client-asserted cloudflare source alone; require gateway obs or verified CF fields.
    let has_server = has_trusted_server_side(&sources, &gateway, &cf, evidence);
    let has_browser = sources.intersection(&browser_side).next().is_some() || !fields.is_empty();
    let has_main_core = main_core_present(&batch_ids, &fields)?;
    let pkgs = present_packages(&Value::Object(fields.clone()), &sources)?;
    let fe_only = has_browser && !has_server;

    let mut conflicts = claim_obs_conflict(&fields, &gateway);
    if let Some(bs) = cf.get("bot_score").and_then(|v| v.as_f64()) {
        if bs >= 80.0 {
            let wd = fields
                .get("webdriver")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                || fields
                    .get("automation")
                    .and_then(|a| a.get("webdriver"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
            if !wd {
                conflicts.push("cf_high_bot_vs_fe_clean".into());
            }
        }
    }

    let xsrc = if !has_server {
        XSRC_MISSING_SERVER
    } else if !has_browser {
        XSRC_MISSING_BROWSER
    } else if !conflicts.is_empty() {
        XSRC_CONFLICT
    } else if has_server && has_main_core {
        XSRC_CONSISTENT
    } else {
        XSRC_PARTIAL
    };

    let owned_bot;
    let bot_ref = if let Some(b) = bot {
        b
    } else {
        owned_bot = score_bot(&Value::Object(fields.clone()), "balanced")
            .map_err(ContractError::new)?;
        &owned_bot
    };
    let strong = has_strong_bot(bot_ref);

    let mut reasons = Vec::new();
    let mut cred: f64 = 0.15;
    if has_server {
        cred += 0.25;
    }
    if has_main_core {
        cred += 0.20;
    }
    if xsrc == XSRC_CONSISTENT {
        cred += 0.25;
    }
    let pkg_any = pkgs
        .get("PKG_AUTO")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || pkgs.get("PKG_GPU").and_then(|v| v.as_bool()).unwrap_or(false)
        || pkgs
            .get("PKG_GEOCPU")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    if pkg_any {
        cred += 0.10;
    }
    if strong {
        cred -= 0.35;
    }
    if xsrc == XSRC_CONFLICT {
        cred -= 0.20;
    }
    if fe_only {
        cred -= 0.15;
    }
    cred = (cred.max(0.0).min(1.0) * 1000.0).round() / 1000.0;

    let mut band: String;
    if strong && bot_ref.verdict == "crawler" {
        band = "crawler".into();
        reasons.push("strong_crawler".into());
    } else if strong && matches!(bot_ref.verdict.as_str(), "bot" | "suspect") {
        band = "likely_bot".into();
        reasons.push("strong_bot".into());
    } else if fe_only || xsrc == XSRC_MISSING_SERVER {
        if bot_ref.verdict == "watch" {
            band = "watch".into();
        } else if bot_ref.score < 25 && has_main_core {
            reasons.push("fe_only_cap".into());
            band = if !has_main_core {
                "insufficient".into()
            } else {
                "watch".into()
            };
        } else {
            band = if !has_main_core {
                "insufficient".into()
            } else {
                "watch".into()
            };
            reasons.push("fe_only_or_missing_server".into());
        }
    } else if xsrc == XSRC_CONFLICT {
        band = "watch".into();
        reasons.push("xsrc_conflict".into());
    } else if has_server
        && has_main_core
        && xsrc != XSRC_CONFLICT
        && !strong
        && !fe_only
        && cred >= 0.55
        && matches!(bot_ref.verdict.as_str(), "human" | "watch")
        && bot_ref.score < 45
    {
        band = "confirmed_real".into();
        reasons.push("confirmed_minimum_met".into());
        if bot_ref.verdict == "watch" && bot_ref.score >= 25 {
            band = "likely_real".into();
            reasons.push("soft_watch_demote_from_confirmed".into());
        }
    } else if has_server && has_main_core && !strong {
        band = "likely_real".into();
        reasons.push("server_and_main_no_strong_bot".into());
    } else if !has_main_core {
        band = "insufficient".into();
        reasons.push("thin_main".into());
    } else {
        band = "watch".into();
        reasons.push("default_watch".into());
    }

    if band == "confirmed_real"
        && (fe_only || !has_server || !has_main_core || xsrc == XSRC_CONFLICT || strong)
    {
        band = if has_main_core {
            "watch".into()
        } else {
            "insufficient".into()
        };
        reasons.push("confirmed_gate_blocked".into());
        cred = cred.min(0.49);
    }

    // iss/43 R5 · phase D: gateway-only / no FE curves must never be confirmed_real.
    // Association ladder gateway ∧ authenticity=confirmed_real was a product dashboard lie.
    let has_fe_curves = fields
        .get("hw_curve_webgl")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty())
        || fields
            .get("hw_curve_audio")
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty());
    let gateway_only_sources = !sources.is_empty()
        && sources.iter().all(|s| {
            let b = s.split(':').next().unwrap_or(s.as_str());
            matches!(b, "gateway" | "cloudflare")
        });
    if matches!(band.as_str(), "confirmed_real" | "likely_real")
        && (gateway_only_sources || (!has_fe_curves && !has_main_core))
    {
        band = if has_main_core {
            "watch".into()
        } else {
            "insufficient".into()
        };
        reasons.push("gateway_or_thin_never_confirmed_real".into());
        cred = cred.min(0.45);
    }

    let mut sources_sorted: Vec<_> = sources.into_iter().collect();
    sources_sorted.sort();
    let mut batch_sorted: Vec<_> = batch_ids.into_iter().collect();
    batch_sorted.sort();

    Ok(TruthResult {
        xsrc_status: xsrc.into(),
        real_band: band,
        credibility: cred,
        fe_only,
        has_server_side: has_server,
        has_main_core,
        packages: pkgs,
        reasons,
        details: json!({
            "sources": sources_sorted,
            "batch_ids": batch_sorted,
            "conflicts": conflicts,
            "bot_verdict": bot_ref.verdict,
            "bot_score": bot_ref.score,
            "bot_algo": bot_ref.algo,
        }),
    })
}
