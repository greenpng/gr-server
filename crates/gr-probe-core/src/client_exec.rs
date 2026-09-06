//! Client execution quality — detect no-JS / early page-close / thin FE evidence.
//!
//! Product need: when visitors close the window quickly, block JS, or never run
//! the FE collector, analyze must **explicitly** report that client JS did not
//! deliver usable probe data — not only a vague `insufficient` band.
//!
//! Status codes (frozen for product):
//! - `js_ok`           — main_core present, enough FE materials
//! - `js_thin`         — some FE batches/fields but not main_core
//! - `js_early_exit`   — pagehide / very few main batches before core complete
//! - `js_unavailable`  — no meaningful FE execution (UA-only or empty main)
//! - `server_only`     — only gateway/CF (or pixel) observation, no main FE

use crate::xsrc::TruthResult;
use serde_json::{json, Map, Value};
use std::collections::HashSet;

/// FE field keys that prove browser JS collectors actually ran (beyond mirrored UA).
const FE_SIGNAL_KEYS: &[&str] = &[
    "webgl_unmasked_renderer",
    "webgl_unmasked_vendor",
    "screen_width",
    "screen_height",
    "timezone",
    "hardware_concurrency",
    "device_memory",
    "form_class",
    "platform",
    "os_family",
    "automation",
    "webdriver",
    "languages",
    "hw_curve_webgl",
    "hw_curve_audio",
    "behavior_early_bound",
    "behavior_events",
    "behavior_count",
    "pagehide_flush",
    "residual_mean",
    "stack_class",
    "plugins_length",
    "chrome_runtime",
    "canvas_hash",
    "audio_hash",
];

fn sources_from(evidence: &Value) -> HashSet<String> {
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
    src
}

fn main_batch_ids(evidence: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    if let Some(batches) = evidence.get("batches").and_then(|v| v.as_array()) {
        for b in batches {
            let src = b
                .get("source")
                .and_then(|v| v.as_str())
                .unwrap_or("main");
            let root = src.split(':').next().unwrap_or(src);
            if root == "main" || root.starts_with("sandbox") || root.starts_with("iframe") || root.starts_with("worker") {
                if let Some(id) = b.get("batch_id").and_then(|v| v.as_str()) {
                    ids.push(id.to_string());
                }
            }
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

fn fe_signal_count(fields: &Map<String, Value>) -> (usize, Vec<String>) {
    let mut hits = Vec::new();
    for k in FE_SIGNAL_KEYS {
        match fields.get(*k) {
            None | Some(Value::Null) => {}
            Some(Value::String(s)) if s.is_empty() => {}
            Some(Value::Array(a)) if a.is_empty() => {}
            Some(Value::Object(o)) if o.is_empty() => {}
            Some(Value::Bool(false)) if *k == "webdriver" || *k == "behavior_early_bound" => {
                // explicit false still proves JS ran for webdriver/bind probes
                hits.push((*k).to_string());
            }
            Some(_) => hits.push((*k).to_string()),
        }
    }
    (hits.len(), hits)
}

fn only_ua_like(fields: &Map<String, Value>) -> bool {
    let (n, hits) = fe_signal_count(fields);
    if n == 0 {
        // may still have user_agent / ua only
        return true;
    }
    // If only form/platform/os mirrored weakly without screen/gpu/auto — still thin
    let strong = hits.iter().any(|h| {
        matches!(
            h.as_str(),
            "webgl_unmasked_renderer"
                | "screen_width"
                | "hardware_concurrency"
                | "automation"
                | "webdriver"
                | "hw_curve_webgl"
                | "hw_curve_audio"
                | "behavior_early_bound"
                | "behavior_events"
                | "pagehide_flush"
                | "residual_mean"
        )
    });
    !strong
}

/// Assess how much client JS actually contributed to this session's evidence.
pub fn assess_client_execution(evidence: &Value, truth: &TruthResult) -> Value {
    let sources = sources_from(evidence);
    let main_batches = main_batch_ids(evidence);
    let main_batch_count = main_batches.len();
    let fields = evidence
        .get("fields")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let (signal_n, signal_hits) = fe_signal_count(&fields);
    let pagehide = fields
        .get("pagehide_flush")
        .and_then(|v| v.as_bool())
        .or_else(|| fields.get("behavior_pagehide").and_then(|v| v.as_bool()))
        .unwrap_or(false);
    let has_main_source = sources.iter().any(|s| s == "main" || s.starts_with("sandbox"));
    let has_server = truth.has_server_side;
    let ua_only = only_ua_like(&fields);
    let has_ua = fields
        .get("user_agent")
        .or_else(|| fields.get("ua"))
        .map(|v| !v.is_null() && v.as_str().map(|s| !s.is_empty()).unwrap_or(true))
        .unwrap_or(false);

    let mut reasons = Vec::new();
    let status: &str;

    if !has_main_source && main_batch_count == 0 && signal_n == 0 {
        if has_server {
            status = "server_only";
            reasons.push("no_main_source".into());
            reasons.push("no_main_batches".into());
            reasons.push("no_fe_signals".into());
            reasons.push("server_observation_only".into());
        } else {
            status = "js_unavailable";
            reasons.push("no_main_source".into());
            reasons.push("no_main_batches".into());
            reasons.push("no_fe_signals".into());
            reasons.push("no_server_side".into());
        }
    } else if main_batch_count == 0 && (signal_n == 0 || ua_only) {
        // fields may only mirror gateway UA — FE collectors never ran
        status = if has_server && !has_main_source {
            "server_only"
        } else {
            "js_unavailable"
        };
        reasons.push("no_main_batches".into());
        if ua_only {
            reasons.push("fe_fields_ua_only".into());
        }
        if signal_n == 0 {
            reasons.push("no_fe_signals".into());
        }
        if !has_main_source {
            reasons.push("no_main_source".into());
        }
    } else if pagehide && main_batch_count <= 2 && !truth.has_main_core {
        status = "js_early_exit";
        reasons.push("pagehide_before_core".into());
        reasons.push(format!("main_batch_count={main_batch_count}"));
        if !truth.has_main_core {
            reasons.push("main_core_incomplete".into());
        }
    } else if main_batch_count <= 2 && !truth.has_main_core && signal_n < 4 {
        // Fast bounce without explicit pagehide flag
        status = "js_early_exit";
        reasons.push("few_main_batches".into());
        reasons.push(format!("main_batch_count={main_batch_count}"));
        reasons.push("main_core_incomplete".into());
        if signal_n < 4 {
            reasons.push(format!("fe_signal_count={signal_n}"));
        }
    } else if !truth.has_main_core {
        status = "js_thin";
        reasons.push("main_core_incomplete".into());
        reasons.push(format!("main_batch_count={main_batch_count}"));
        reasons.push(format!("fe_signal_count={signal_n}"));
    } else {
        status = "js_ok";
        reasons.push("main_core_present".into());
        if pagehide {
            reasons.push("pagehide_after_core".into());
        }
    }

    let implication = match status {
        "js_unavailable" | "server_only" => "cannot_score_browser_fully",
        "js_early_exit" => "partial_probe_short_visit",
        "js_thin" => "needs_more_packs",
        _ => "probe_usable",
    };

    // Can we score each product axis with current evidence?
    let can_device = truth.has_main_core
        && fields
            .get("form_class")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty())
        && (fields.get("hw_curve_webgl").is_some()
            || fields.get("hw_curve_audio").is_some()
            || fields.get("webgl_unmasked_renderer").is_some());
    let can_os = signal_n >= 3 || truth.has_main_core;
    let can_br = signal_n >= 3
        || fields.get("automation").is_some()
        || fields.get("webdriver").is_some()
        || truth.has_main_core;
    let can_rpa = fields
        .get("behavior_early_bound")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || fields
            .get("behavior_events")
            .and_then(|v| v.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false)
        || fields
            .get("behavior_count")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
            > 0.0;

    let evidence_quality = match status {
        "js_ok" if truth.has_server_side => "rich",
        "js_ok" => "adequate",
        "js_thin" => "partial",
        "js_early_exit" => "thin",
        "server_only" | "js_unavailable" => "none",
        _ => "thin",
    };

    let mut blocking: Vec<String> = Vec::new();
    if matches!(status, "js_unavailable" | "server_only") {
        blocking.push("client_js_unavailable".into());
    }
    if matches!(status, "js_early_exit" | "js_thin") {
        blocking.push("client_probe_incomplete".into());
    }
    if !can_rpa {
        blocking.push("no_behavior_for_rpa".into());
    }
    if !can_device {
        blocking.push("no_hard_device_materials".into());
    }
    if !has_server {
        blocking.push("no_trusted_server_side".into());
    }

    json!({
        "status": status,
        "reasons": reasons,
        "implication": implication,
        "main_batch_count": main_batch_count,
        "main_batches": main_batches,
        "fe_signal_count": signal_n,
        "fe_signal_hits_sample": signal_hits.into_iter().take(12).collect::<Vec<_>>(),
        "pagehide_seen": pagehide,
        "has_main_source": has_main_source,
        "has_server_side": has_server,
        "has_ua_signal": has_ua,
        "ua_only_fields": ua_only,
        "has_main_core": truth.has_main_core,
        "probe_summary": {
            "evidence_quality": evidence_quality,
            "can_score_device": can_device,
            "can_score_os": can_os,
            "can_score_br": can_br,
            "can_score_rpa": can_rpa,
            "blocking_reasons": blocking,
        },
        // Human-readable product hint (zh + en codes)
        "product_hint": match status {
            "js_unavailable" => "frontend_js_did_not_run_or_no_collector_data",
            "server_only" => "only_server_edge_observation_no_browser_js",
            "js_early_exit" => "visitor_left_before_core_packs_completed",
            "js_thin" => "partial_frontend_probe_not_main_core",
            _ => "frontend_probe_ok",
        },
    })
}

/// Whether product scores must be capped / marked unknown due to client execution.
pub fn client_exec_forces_unknown(status: &str) -> bool {
    matches!(status, "js_unavailable" | "server_only")
}

/// Soft demote (early exit / thin) — lower coverage, keep partial scores.
pub fn client_exec_is_thin(status: &str) -> bool {
    matches!(status, "js_early_exit" | "js_thin")
}

/// Apply demotion to product / page / device based on client execution status.
/// Mutates in place. Safe when status is `js_ok` (annotation only).
pub fn apply_client_exec_demotion(
    product: &mut Value,
    page: &mut Option<Value>,
    device: &mut Value,
    client_exec: &Value,
) {
    let status = client_exec
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("js_ok");
    let product_hint = client_exec
        .get("product_hint")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let blocking: Vec<String> = client_exec
        .pointer("/probe_summary/blocking_reasons")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    if let Some(obj) = product.as_object_mut() {
        obj.insert("client_execution_status".into(), json!(status));
        if !product_hint.is_empty() {
            obj.insert("client_execution_hint".into(), json!(product_hint));
        }
    }

    if client_exec_forces_unknown(status) {
        demote_leg_unknown(product, "os", status, &blocking);
        demote_leg_unknown(product, "br", status, &blocking);
        demote_leg_unknown(product, "rpa", status, &blocking);
        if let Some(p) = page.as_mut() {
            demote_leg_unknown(p, "rpa", status, &blocking);
        }
        if let Some(obj) = device.as_object_mut() {
            let conf = obj.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let capped = (conf * 0.35).min(0.25);
            obj.insert("confidence".into(), json!(capped));
            let mut reasons = obj
                .get("no_id_reasons")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            reasons.push(json!(format!("client_exec:{status}")));
            for b in &blocking {
                reasons.push(json!(b));
            }
            obj.insert("no_id_reasons".into(), Value::Array(reasons));
        }
    } else if client_exec_is_thin(status) {
        demote_leg_thin(product, "os", status);
        demote_leg_thin(product, "br", status);
        demote_leg_thin(product, "rpa", status);
        if let Some(p) = page.as_mut() {
            demote_leg_thin(p, "rpa", status);
        }
        if let Some(obj) = device.as_object_mut() {
            let conf = obj.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let capped = (conf * 0.75).min(0.55);
            obj.insert("confidence".into(), json!(capped));
            let mut warnings = obj
                .get("id_warnings")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            warnings.push(json!(format!("client_exec_thin:{status}")));
            obj.insert("id_warnings".into(), Value::Array(warnings));
        }
    }
}

fn demote_leg_unknown(root: &mut Value, leg: &str, status: &str, blocking: &[String]) {
    let Some(obj) = root.as_object_mut() else {
        return;
    };
    let Some(leg_v) = obj.get_mut(leg) else {
        return;
    };
    let Some(leg_o) = leg_v.as_object_mut() else {
        return;
    };
    // Architecture always has a JS trigger path: empty/gateway-only evidence is
    // **missing probe data**, not a first-class "nojs" visitor class.
    let status_label = if matches!(status, "js_unavailable" | "server_only") {
        "missing_probe"
    } else {
        "unknown"
    };
    leg_o.insert("status".into(), json!(status_label));
    leg_o.insert("score".into(), json!(0.0));
    leg_o.insert("coverage".into(), json!(0.0));
    leg_o.insert("confidence".into(), json!(0.0));
    let mut reasons = leg_o
        .get("reasons")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    reasons.insert(0, json!(format!("client_exec:{status}")));
    reasons.insert(0, json!("missing_probe_data_zero_score"));
    for b in blocking {
        if !reasons.iter().any(|r| r.as_str() == Some(b.as_str())) {
            reasons.push(json!(b));
        }
    }
    leg_o.insert("reasons".into(), Value::Array(reasons));
}

fn demote_leg_thin(root: &mut Value, leg: &str, status: &str) {
    let Some(obj) = root.as_object_mut() else {
        return;
    };
    let Some(leg_v) = obj.get_mut(leg) else {
        return;
    };
    let Some(leg_o) = leg_v.as_object_mut() else {
        return;
    };
    let score = leg_o.get("score").and_then(|v| v.as_f64()).unwrap_or(0.5);
    leg_o.insert("score".into(), json!((score * 0.85).min(0.65)));
    let cov = leg_o.get("coverage").and_then(|v| v.as_f64()).unwrap_or(0.3);
    leg_o.insert("coverage".into(), json!((cov * 0.8).min(0.45)));
    let st = leg_o.get("status").and_then(|v| v.as_str()).unwrap_or("");
    if matches!(st, "real" | "human") {
        leg_o.insert("status".into(), json!("suspect"));
    }
    let mut reasons = leg_o
        .get("reasons")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    reasons.insert(0, json!(format!("client_exec:{status}")));
    leg_o.insert("reasons".into(), Value::Array(reasons));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xsrc::evaluate_xsrc;

    #[test]
    fn server_only_thin_ua() {
        let evidence = json!({
            "session_id": "t1",
            "sources": ["gateway"],
            "has_gateway": true,
            "batches": [{"batch_id": "B8_gateway", "source": "gateway"}],
            "gateway_fields": {"user_agent": "Mozilla/5.0"},
            "cf_fields": {"bot_score": 1},
            "fields": {"user_agent": "Mozilla/5.0", "ua": "Mozilla/5.0"}
        });
        let truth = evaluate_xsrc(&evidence, None).unwrap();
        let ce = assess_client_execution(&evidence, &truth);
        let st = ce["status"].as_str().unwrap();
        assert!(
            matches!(st, "server_only" | "js_unavailable"),
            "status={st}"
        );
        assert_eq!(ce["probe_summary"]["can_score_rpa"], false);
        assert!(ce["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r.as_str() == Some("no_main_batches") || r.as_str() == Some("fe_fields_ua_only")));
    }

    #[test]
    fn pagehide_early_exit() {
        let evidence = json!({
            "session_id": "t2",
            "sources": ["main"],
            "batches": [
                {"batch_id": "B0_bootstrap", "source": "main"}
            ],
            "fields": {
                "user_agent": "Mozilla/5.0",
                "screen_width": 1920,
                "pagehide_flush": true,
                "behavior_early_bound": true
            }
        });
        let truth = evaluate_xsrc(&evidence, None).unwrap();
        let ce = assess_client_execution(&evidence, &truth);
        assert_eq!(ce["status"], "js_early_exit");
        assert_eq!(ce["pagehide_seen"], true);
        assert_eq!(ce["product_hint"], "visitor_left_before_core_packs_completed");
    }

    #[test]
    fn demotion_forces_unknown_on_server_only() {
        let evidence = json!({
            "session_id": "t3",
            "sources": ["gateway"],
            "has_gateway": true,
            "batches": [{"batch_id": "B8_gateway", "source": "gateway"}],
            "fields": {"user_agent": "Mozilla/5.0"}
        });
        let truth = evaluate_xsrc(&evidence, None).unwrap();
        let ce = assess_client_execution(&evidence, &truth);
        let mut product = json!({
            "os": {"score": 0.7, "status": "real", "coverage": 0.5, "reasons": []},
            "br": {"score": 0.7, "status": "real", "coverage": 0.5, "reasons": []},
            "rpa": {"score": 0.6, "status": "human", "coverage": 0.4, "reasons": []}
        });
        let mut page = None;
        let mut device = json!({"confidence": 0.8, "no_id_reasons": []});
        apply_client_exec_demotion(&mut product, &mut page, &mut device, &ce);
        assert_eq!(product["os"]["status"], "missing_probe");
        assert_eq!(product["br"]["status"], "missing_probe");
        assert_eq!(product["os"]["score"].as_f64().unwrap(), 0.0);
        assert_eq!(product["rpa"]["score"].as_f64().unwrap(), 0.0);
        assert!(device["confidence"].as_f64().unwrap() <= 0.25);
        assert_eq!(product["client_execution_status"], ce["status"]);
    }

    #[test]
    fn few_batches_with_signals_classifies_thin_or_early() {
        let evidence = json!({
            "session_id": "t4",
            "sources": ["main", "gateway"],
            "has_gateway": true,
            "batches": [
                {"batch_id": "B0_bootstrap", "source": "main"},
                {"batch_id": "B1_conflict", "source": "main"},
                {"batch_id": "B3_system", "source": "main"},
                {"batch_id": "B8_gateway", "source": "gateway"}
            ],
            "fields": {
                "user_agent": "Mozilla/5.0",
                "screen_width": 1920,
                "screen_height": 1080,
                "hardware_concurrency": 8,
                "timezone": "Asia/Shanghai",
                "platform": "Win32",
                "webdriver": false,
                "plugins_length": 2
            }
        });
        let truth = evaluate_xsrc(&evidence, None).unwrap();
        let ce = assess_client_execution(&evidence, &truth);
        let st = ce["status"].as_str().unwrap();
        assert!(
            matches!(st, "js_thin" | "js_early_exit" | "js_ok"),
            "status={st} main_core={}",
            truth.has_main_core
        );
        assert!(!client_exec_forces_unknown(st));
    }

    #[test]
    fn thin_demotion_does_not_force_unknown() {
        let ce = json!({
            "status": "js_early_exit",
            "product_hint": "visitor_left_before_core_packs_completed",
            "probe_summary": {"blocking_reasons": ["client_probe_incomplete"]}
        });
        let mut product = json!({
            "os": {"score": 0.8, "status": "real", "coverage": 0.6, "reasons": []},
            "br": {"score": 0.8, "status": "real", "coverage": 0.6, "reasons": []},
            "rpa": {"score": 0.7, "status": "human", "coverage": 0.5, "reasons": []}
        });
        let mut page = Some(json!({
            "rpa": {"score": 0.7, "status": "human", "coverage": 0.5, "reasons": []}
        }));
        let mut device = json!({"confidence": 0.7, "id_warnings": []});
        apply_client_exec_demotion(&mut product, &mut page, &mut device, &ce);
        assert_eq!(product["os"]["status"], "suspect");
        assert!(product["os"]["score"].as_f64().unwrap() < 0.8);
        assert!(product["os"]["score"].as_f64().unwrap() > 0.35);
        assert_eq!(product["client_execution_status"], "js_early_exit");
    }

    #[test]
    fn js_ok_annotation_only() {
        let ce = json!({
            "status": "js_ok",
            "product_hint": "frontend_probe_ok",
            "probe_summary": {"blocking_reasons": []}
        });
        let mut product = json!({
            "os": {"score": 0.9, "status": "real", "coverage": 0.8, "reasons": ["main_core_ok"]}
        });
        let mut page = None;
        let mut device = json!({"confidence": 0.9});
        apply_client_exec_demotion(&mut product, &mut page, &mut device, &ce);
        assert_eq!(product["os"]["status"], "real");
        assert_eq!(product["os"]["score"], 0.9);
        assert_eq!(product["client_execution_status"], "js_ok");
        assert_eq!(device["confidence"], 0.9);
    }
}
