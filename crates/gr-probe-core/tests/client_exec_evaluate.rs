//! Client-execution quality through evaluate_session (no-JS / early pagehide / thin FE).
//!
//! Requires fixtures under green-v5/fixtures/:
//!   js_unavailable.json, js_early_exit_pagehide.json, js_thin_partial.json,
//!   thin_ua_server.json, confirmed_eligible.json

use gr_probe_core::evaluate::evaluate_session;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn load_fixture(name: &str) -> Value {
    let path = fixtures_dir().join(name);
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).expect("parse fixture")
}

#[test]
fn js_unavailable_classifies_and_demotes() {
    let evidence = load_fixture("js_unavailable.json");
    let out = evaluate_session(&evidence, None, None, None, true).expect("evaluate");
    let ce = &out["client_execution"];
    let status = ce["status"].as_str().unwrap_or("");
    assert!(
        matches!(status, "server_only" | "js_unavailable"),
        "expected server_only|js_unavailable, got {status}; ce={ce}"
    );
    assert_eq!(ce["probe_summary"]["can_score_rpa"], false);
    assert!(
        matches!(
            ce["product_hint"].as_str(),
            Some("frontend_js_did_not_run_or_no_collector_data")
                | Some("only_server_edge_observation_no_browser_js")
        ),
        "hint={:?}",
        ce["product_hint"]
    );
    // Product legs must be unknown / missing_probe / heavily demoted
    let os_st = out["product"]["os"]["status"].as_str().unwrap_or("");
    let br_st = out["product"]["br"]["status"].as_str().unwrap_or("");
    assert!(
        matches!(os_st, "unknown" | "missing_probe" | "insufficient"),
        "os status={os_st}"
    );
    assert!(
        matches!(br_st, "unknown" | "missing_probe" | "insufficient"),
        "br status={br_st}"
    );
    assert!(
        out["product"]["os"]["score"].as_f64().unwrap_or(1.0) <= 0.35,
        "os score should be capped"
    );
    assert_eq!(out["product"]["client_execution_status"], status);
    assert_ne!(out["real_band"], "confirmed_real");
    assert_ne!(out["real_band"], "likely_real");
    assert_eq!(out["short_visit"]["client_js_usable"], false);
    // Diagnostics mirror
    assert_eq!(out["diagnostics"]["client_execution"]["status"], status);
}

#[test]
fn thin_ua_server_is_server_only_surface() {
    let evidence = load_fixture("thin_ua_server.json");
    let out = evaluate_session(&evidence, None, None, None, true).expect("evaluate");
    let status = out["client_execution"]["status"].as_str().unwrap_or("");
    assert!(
        matches!(status, "server_only" | "js_unavailable"),
        "thin_ua_server status={status}"
    );
    assert_eq!(out["real_band"], "insufficient");
    let os_st = out["product"]["os"]["status"].as_str().unwrap_or("");
    let br_st = out["product"]["br"]["status"].as_str().unwrap_or("");
    assert!(
        matches!(os_st, "unknown" | "missing_probe" | "insufficient"),
        "os status={os_st}"
    );
    assert!(
        matches!(br_st, "unknown" | "missing_probe" | "insufficient"),
        "br status={br_st}"
    );
    assert_eq!(out["short_visit"]["client_js_usable"], false);
    // Device conf capped when server-only demotion applies
    let conf = out["device"]["confidence"].as_f64().unwrap_or(1.0);
    assert!(conf <= 0.25, "device conf should be capped, got {conf}");
}

#[test]
fn js_early_exit_pagehide_classifies() {
    let evidence = load_fixture("js_early_exit_pagehide.json");
    let out = evaluate_session(&evidence, None, None, None, true).expect("evaluate");
    let ce = &out["client_execution"];
    assert_eq!(ce["status"], "js_early_exit", "ce={ce}");
    assert_eq!(ce["pagehide_seen"], true);
    assert_eq!(ce["product_hint"], "visitor_left_before_core_packs_completed");
    assert_eq!(ce["implication"], "partial_probe_short_visit");
    // Thin demotion: client_exec reason present on product legs
    let os_reasons = out["product"]["os"]["reasons"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        os_reasons
            .iter()
            .any(|r| r.as_str().unwrap_or("").contains("client_exec:js_early_exit")),
        "expected client_exec reason on os; reasons={os_reasons:?}"
    );
    assert_eq!(out["product"]["client_execution_status"], "js_early_exit");
    assert_ne!(out["real_band"], "confirmed_real");
    assert_eq!(out["short_visit"]["client_execution_status"], "js_early_exit");
    // Soft demote path (not forces_unknown): confidence not hard-capped to server-only floor
    // when product already had low coverage, status may already be unknown — check score path.
    assert!(
        !out["device"]
            .get("no_id_reasons")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().any(|r| {
                r.as_str()
                    .unwrap_or("")
                    .contains("client_exec:js_unavailable")
                    || r.as_str().unwrap_or("").contains("client_exec:server_only")
            }))
            .unwrap_or(false),
        "early_exit must not apply unavailable/server_only hard demotion reasons"
    );
}

#[test]
fn js_thin_partial_classifies() {
    let evidence = load_fixture("js_thin_partial.json");
    let out = evaluate_session(&evidence, None, None, None, true).expect("evaluate");
    let ce = &out["client_execution"];
    let status = ce["status"].as_str().unwrap_or("");
    // Non-core mid packs + FE signals, no B0/B1/B2 core set → thin (not js_ok / not server_only)
    assert!(
        matches!(status, "js_thin" | "js_early_exit"),
        "expected js_thin|js_early_exit for partial non-core packs, got {status}; ce={ce}"
    );
    assert!(ce.get("probe_summary").is_some());
    assert_eq!(out["product"]["client_execution_status"], status);
    assert_ne!(out["real_band"], "confirmed_real");
    let os_reasons = out["product"]["os"]["reasons"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        os_reasons
            .iter()
            .any(|r| r.as_str().unwrap_or("").starts_with("client_exec:")),
        "expected client_exec demotion reason; reasons={os_reasons:?}"
    );
    assert_eq!(out["short_visit"]["client_js_usable"], status == "js_thin");
}

#[test]
fn confirmed_eligible_still_js_ok() {
    let evidence = load_fixture("confirmed_eligible.json");
    let out = evaluate_session(&evidence, None, None, None, true).expect("evaluate");
    let ce = &out["client_execution"];
    let status = ce["status"].as_str().unwrap_or("");
    assert!(
        matches!(status, "js_ok" | "js_thin"),
        "confirmed_eligible should not be unavailable; status={status} ce={ce}"
    );
    assert_eq!(
        out["short_visit"]["client_js_usable"],
        status == "js_ok" || status == "js_thin"
    );
    assert!(ce.get("probe_summary").is_some());
    // Must keep confirmed_real path for rich multi-source human fixture
    if status == "js_ok" {
        assert_eq!(out["real_band"], "confirmed_real");
        // No hard demotion to unknown on product legs
        assert_ne!(out["product"]["os"]["status"], "unknown");
        assert_ne!(out["product"]["br"]["status"], "unknown");
    }
}

#[test]
fn client_execution_present_on_all_evaluate_outputs() {
    for name in [
        "js_unavailable.json",
        "js_early_exit_pagehide.json",
        "thin_ua_server.json",
        "confirmed_eligible.json",
        "bot_crawler.json",
        "fe_only.json",
    ] {
        let evidence = load_fixture(name);
        let out = evaluate_session(&evidence, None, None, None, true)
            .unwrap_or_else(|e| panic!("evaluate {name}: {e}"));
        assert!(
            out.get("client_execution").is_some(),
            "{name}: missing client_execution"
        );
        assert!(
            out["client_execution"]["status"].as_str().is_some(),
            "{name}: missing status"
        );
        assert!(
            out["short_visit"]["client_execution_status"].as_str().is_some(),
            "{name}: short_visit missing client_execution_status"
        );
        assert!(
            out["product"]["client_execution_status"].as_str().is_some(),
            "{name}: product missing client_execution_status"
        );
    }
}
