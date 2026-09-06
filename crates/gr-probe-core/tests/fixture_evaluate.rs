//! Fixture-driven tests against shipped `evaluate_session` / inject / redlines.
//! Loads real fixtures from the green-v5 tree — no oracle reimplementation.

use gr_probe_core::brain::build_frontier;
use gr_probe_core::evaluate::evaluate_session;
use gr_probe_core::experiment::apply_strategy;
use gr_probe_core::inject::{inject_deploy_gate, plan_inject, simulate_document_boot};
use gr_probe_core::soft_v2::{soft_pair_decision, SoftBlockingEngine, SoftMember, PROMOTE_TO_COMMERCIAL_ID};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    // crates/gr-core → ../../fixtures
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn load_fixture(name: &str) -> Value {
    let path = fixtures_dir().join(name);
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).expect("parse fixture")
}

#[test]
fn fe_only_not_confirmed_soft_false_defaults() {
    let evidence = load_fixture("fe_only.json");
    let out = evaluate_session(&evidence, None, None, None, true).expect("evaluate");
    let band = out["real_band"].as_str().unwrap();
    assert_ne!(band, "confirmed_real", "FE-only must never be confirmed_real");
    // Thin FE-only may be watch (locale soft) or insufficient (main_core incomplete) — never confirmed.
    assert!(
        matches!(band, "fe_only" | "insufficient" | "watch" | "insufficient_server_side"),
        "fe_only band unexpected: {band}"
    );
    assert_eq!(out["soft_promote"], json!(false));
    assert_eq!(out["defaults"]["bot_algo"], "balanced");
    assert_eq!(out["defaults"]["link_algo"], "sparse_safe");
    let packs = out["route_plan"]["packs"].as_array().expect("packs");
    assert!(!packs.is_empty(), "route_plan.packs must be present");
    assert!(
        packs.iter().any(|p| {
            matches!(
                p["pack_id"].as_str(),
                Some("B8_gateway_early") | Some("edge.cf") | Some("gateway.b8")
            )
        }),
        "expected gateway/cf pack, got {packs:?}"
    );
    assert_eq!(out["xsrc_status"], "missing_server");
    // Static wave exists without needing analyze first
    assert_eq!(out["static_wave"]["kick_without_analyze"], true);
    assert!(out["static_wave"]["packs"].as_array().unwrap().len() >= 6);
}

#[test]
fn confirmed_eligible_higher_band() {
    let evidence = load_fixture("confirmed_eligible.json");
    let out = evaluate_session(&evidence, None, None, None, true).expect("evaluate");
    assert_eq!(out["real_band"], "confirmed_real");
    assert_eq!(out["soft_promote"], json!(false));
    assert_eq!(out["defaults"]["bot_algo"], "balanced");
    assert_eq!(out["defaults"]["link_algo"], "sparse_safe");
    assert!(out["route_plan"]["packs"].is_array());
    assert_eq!(out["xsrc_status"], "consistent");
    assert!(out["credibility"].as_f64().unwrap() >= 0.55);
}

#[test]
fn thin_ua_server_insufficient() {
    let evidence = load_fixture("thin_ua_server.json");
    let out = evaluate_session(&evidence, None, None, None, true).expect("evaluate");
    assert_eq!(out["real_band"], "insufficient");
    assert_ne!(out["real_band"], "confirmed_real");
    // client_execution surface for gateway-only UA
    let ce = out["client_execution"]["status"].as_str().unwrap_or("");
    assert!(
        matches!(ce, "server_only" | "js_unavailable"),
        "thin_ua_server client_execution={ce}"
    );
    assert_eq!(out["short_visit"]["client_js_usable"], false);
    // Gateway-only / no FE: status is missing_probe (not a visitor class "unknown").
    let os_st = out["product"]["os"]["status"].as_str().unwrap_or("");
    assert!(
        matches!(os_st, "unknown" | "missing_probe"),
        "thin_ua_server os status={os_st}"
    );
}

#[test]
fn bot_crawler_band() {
    let evidence = load_fixture("bot_crawler.json");
    let out = evaluate_session(&evidence, None, None, None, true).expect("evaluate");
    assert_eq!(out["real_band"], "crawler");
    assert_eq!(out["bot"]["algo"], "balanced");
}

#[test]
fn bot_locale_noise_not_blocked_by_locale() {
    // balanced soft locale weights: still can confirm when multi-source ok
    let evidence = load_fixture("bot_locale_noise.json");
    let out = evaluate_session(&evidence, None, None, None, true).expect("evaluate");
    assert_eq!(out["real_band"], "confirmed_real");
    assert_eq!(out["bot"]["algo"], "balanced");
}

#[test]
fn soft_promote_always_false_constant() {
    assert!(!PROMOTE_TO_COMMERCIAL_ID);
    let evidence = load_fixture("fe_only.json");
    let out = evaluate_session(&evidence, None, None, None, true).unwrap();
    assert_eq!(out["soft_promote"], false);
}

#[test]
fn strategy_deny_soft_promote_and_fe_only_confirmed() {
    let mutations = load_fixture("strategy_deny.json");
    let res = apply_strategy(Some(&mutations), None, true).expect("apply");
    assert!(!res.ok, "denied constitutional mutations must fail closed");
    assert!(res.denied.iter().any(|k| k == "soft_promote"));
    assert!(res.denied.iter().any(|k| k == "fe_only_to_confirmed"));
    assert!(res.denied.iter().any(|k| k == "d2_hard_material_whitelist"));
    assert_eq!(res.base["soft_promote"], false);
    assert_eq!(res.base["fe_only_to_confirmed"], false);
    assert_eq!(res.base["bot_algo"], "balanced");
    assert_eq!(res.base["link_algo"], "sparse_safe");
    // end-to-end evaluate also keeps soft_promote false
    let evidence = load_fixture("confirmed_eligible.json");
    let out = evaluate_session(&evidence, Some(&mutations), None, None, true).unwrap();
    assert_eq!(out["soft_promote"], false);
    assert_eq!(out["strategy"]["ok"], false);
}

#[test]
fn strategy_allow_pack_order_ok() {
    let muts = load_fixture("strategy_allow.json");
    let mutations = if muts.get("pack_order").is_some() {
        muts
    } else {
        muts.get("mutations").cloned().unwrap_or(muts)
    };
    let res = apply_strategy(Some(&mutations), None, true).expect("apply");
    // constitutional algos fixed
    assert_eq!(res.base["bot_algo"], "balanced");
    assert_eq!(res.base["soft_promote"], false);
}

#[test]
fn brain_withholds_mid_without_soft_ready() {
    let evidence = load_fixture("fe_only.json");
    let plan = build_frontier(&evidence, false, None, 8).expect("frontier");
    // Soft-gated dynamic mid4 withheld; static hard B10 may still appear (layer=hard).
    let soft_mid = plan.packs.iter().any(|p| {
        p.get("schedule").and_then(|v| v.as_str()) == Some("dynamic")
            && p.get("layer").and_then(|v| v.as_str()) == Some("mid4")
    });
    assert!(
        !soft_mid,
        "dynamic mid4 packs must be withheld when soft_v2_ready=false; packs={:?}",
        plan.packs
    );
    assert!(plan
        .notes
        .iter()
        .any(|n| n.contains("mid withheld") || n.contains("soft_v2")));
}

#[test]
fn brain_amplify_mid_when_soft_ready() {
    // lite5-complete + gateway so high gaps clear and mid can be scheduled
    let evidence = load_fixture("confirmed_eligible.json");
    let plan = build_frontier(&evidence, true, None, 12).expect("frontier");
    let soft_mid = plan.packs.iter().any(|p| {
        p.get("schedule").and_then(|v| v.as_str()) == Some("dynamic")
            && p.get("layer").and_then(|v| v.as_str()) == Some("mid4")
    });
    let hard_curves = plan.packs.iter().any(|p| p["pack_id"] == "B10_hw_curves");
    assert!(
        soft_mid || hard_curves,
        "soft ready should plan soft mid and/or static hard curves; packs={:?}",
        plan.packs
    );
}

#[test]
fn multi_tick_frontier_progress() {
    // Tick 1: thin main only → plan asks for static fills
    let thin = json!({
        "session_id": "mt1",
        "sources": ["main"],
        "batches": [{"batch_id": "B0_bootstrap", "source": "main"}],
        "fields": {
            "session_id": "mt1",
            "user_agent": "Mozilla/5.0 Chrome/120",
            "screen_width": 1920,
            "screen_height": 1080,
            "timezone": "Asia/Shanghai",
            "hardware_concurrency": 8
        }
    });
    let t1 = build_frontier(&thin, true, None, 12).expect("t1");
    assert!(!t1.stop_probe);
    assert!(t1.packs.iter().any(|p| p["pack_id"] == "B8_gateway_early"
        || p["pack_id"] == "B1_conflict"
        || p["pack_id"] == "B2_hardware"));

    // Tick 2: static wave completed → high gaps clear; mid dynamic may appear
    let full = load_fixture("confirmed_eligible.json");
    let t2 = build_frontier(&full, true, None, 12).expect("t2");
    let t2_ids: Vec<_> = t2
        .packs
        .iter()
        .filter_map(|p| p["pack_id"].as_str())
        .collect();
    // Should not re-request already-present lite5 batches as required fills
    assert!(
        !t2_ids.contains(&"B0_bootstrap"),
        "should not re-kick collected B0: {t2_ids:?}"
    );
    // Soft mid dynamic and/or static hard curves still missing
    assert!(
        t2.dynamic_pack_ids
            .iter()
            .any(|id| id.starts_with("B1") && id.as_str() != "B10_hw_curves")
            || t2.packs.iter().any(|p| p["pack_id"] == "B10_hw_curves")
            || t2.static_pack_ids.iter().any(|id| id == "B10_hw_curves"),
        "expected soft mid dynamic and/or static B10: dyn={:?} packs={:?}",
        t2.dynamic_pack_ids,
        t2.packs
    );

    // Tick 3: mid also present → stop or only residual low gaps
    let mut done = full.clone();
    if let Some(batches) = done.get_mut("batches").and_then(|b| b.as_array_mut()) {
        for bid in [
            "B10_hw_curves",
            "B16_fast_signals",
            "B13_authorized",
            "B15_cross_curves",
            "B7_sandbox",
            "B11_interaction",
            // static wave also requires fuzzy helper echo (catalog static_required).
            "B85_fuzzy_helper_echo",
        ] {
            batches.push(json!({"batch_id": bid, "source": "main"}));
        }
    }
    if let Some(sources) = done.get_mut("sources").and_then(|s| s.as_array_mut()) {
        sources.push(json!("worker:d1"));
    }
    let t3 = build_frontier(&done, true, None, 16).expect("t3");
    // maximize_probe: static+mid present does NOT force coverage_complete — dense
    // packs remain planned (B47/B50/…/B80). Terminal is static_complete + schedule, not
    // dense_empty. Assert static wave done and pipeline still schedules deepen/re-probe.
    let cov = &t3.coverage;
    let static_ok = cov
        .get("static_complete")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let dense_planned = cov
        .get("dense_planned")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let coverage_complete = cov
        .get("coverage_complete")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    assert!(
        static_ok,
        "static+mid fixture must mark static_complete; cov={cov:?}"
    );
    assert!(
        coverage_complete || dense_planned || !t3.packs.is_empty(),
        "after static+mid: coverage_complete OR dense still planned OR packs queued; cov={cov:?} packs={}",
        t3.packs.len()
    );
    // Coverage complete may still schedule re-probe packs when residual/host materials
    // are missing — missing fields demote scores, they must not freeze the pipeline.
    // stop_probe=true only when residual packs are optional or empty.
    if !t3.stop_probe {
        assert!(
            !t3.packs.is_empty(),
            "if not stop_probe after coverage, re-probe packs must remain; notes={:?}",
            t3.notes
        );
        let notes = t3.notes.join(" ");
        assert!(
            notes.contains("packs still queued")
                || notes.contains("re_probe")
                || notes.contains("host_sep")
                || notes.contains("DEFER_FINALIZE")
                || notes.contains("residual")
                || dense_planned
                || notes.contains("dense"),
            "expected continue-probe note when stop_probe=false; notes={:?}",
            t3.notes
        );
    }

    // confirmed_eligible alone is NOT terminal if mid still planned missing
    let conf_only = load_fixture("confirmed_eligible.json");
    let t_conf = build_frontier(&conf_only, true, None, 16).expect("conf");
    assert!(
        !t_conf.stop_probe,
        "real_band may be confirmed later in evaluate, but frontier must not stop while mid missing; packs={:?}",
        t_conf.packs
    );
    assert!(
        !t_conf
            .coverage
            .get("coverage_complete")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        "mid incomplete => coverage incomplete"
    );
}

#[test]
fn filter_unresolved_skips_kicked() {
    use gr_probe_core::brain::filter_unresolved_packs;
    use std::collections::HashSet;
    let packs = vec![
        json!({"pack_id": "B0_bootstrap", "batch_id": "B0_bootstrap", "priority": 96}),
        json!({"pack_id": "B1_conflict", "batch_id": "B1_conflict", "priority": 95}),
    ];
    let mut already = HashSet::new();
    already.insert("B0_bootstrap".into());
    let left = filter_unresolved_packs(&packs, &already);
    assert_eq!(left.len(), 1);
    assert_eq!(left[0]["pack_id"], "B1_conflict");
}

#[test]
fn filter_unresolved_keeps_force_recollect() {
    use gr_probe_core::brain::filter_unresolved_packs;
    use std::collections::HashSet;
    // B2 already collected (batch present) but residual missing → brain marks force_recollect
    let packs = vec![
        json!({
            "pack_id": "B2_hardware",
            "batch_id": "B2_hardware",
            "priority": 96,
            "force_recollect": true,
            "reason": "stack_residual_missing_recollect"
        }),
        json!({"pack_id": "B0_bootstrap", "batch_id": "B0_bootstrap", "priority": 90}),
    ];
    let mut already = HashSet::new();
    already.insert("B2_hardware".into());
    already.insert("B0_bootstrap".into());
    let left = filter_unresolved_packs(&packs, &already);
    assert_eq!(left.len(), 1, "only force_recollect B2 should remain: {left:?}");
    assert_eq!(left[0]["pack_id"], "B2_hardware");
    assert_eq!(left[0]["force_recollect"], true);
}

#[test]
fn short_visit_thin_honest_and_usable() {
    let thin = load_fixture("thin_ua_server.json");
    let out = evaluate_session(&thin, None, None, None, true).expect("eval");
    assert_ne!(out["real_band"], "confirmed_real");
    assert_eq!(out["soft_promote"], false);
    assert_eq!(out["defaults"]["bot_algo"], "balanced");
    assert_eq!(out["defaults"]["link_algo"], "sparse_safe");
    assert_eq!(out["short_visit"]["usable_coarse"], true);
    assert_eq!(out["short_visit"]["has_ua_signal"], true);
}

#[test]
fn catalog_aliases_resolve_legacy_names() {
    use gr_probe_core::load_catalog;
    let c = load_catalog().expect("catalog");
    for alias in ["lite.surface", "gateway.b8", "mid.curves", "lite.auto", "conflict.reconcile"] {
        assert!(c.resolve(alias).is_some(), "alias {alias}");
    }
    // Every static kick pack is executable
    for p in c.static_packs() {
        assert!(p.executable, "{}", p.pack_id);
    }
}

/// Spot-check: full lite5 + client-forged cloudflare source/stub must NOT become confirmed_real.
#[test]
fn forged_cloudflare_source_must_not_confirm() {
    let evidence = json!({
        "session_id": "forged_cf_001",
        "sources": ["main", "cloudflare"],
        "has_cloudflare": true,
        "cf_edge": true,
        // no cf_fields — client-only claim
        "batches": [
            {"batch_id": "B0_bootstrap", "source": "main"},
            {"batch_id": "B1_conflict", "source": "main"},
            {"batch_id": "B2_hardware", "source": "main"},
            {"batch_id": "B3_system", "source": "main"},
            {"batch_id": "B12_anti_camouflage", "source": "main"},
            {"batch_id": "B8_gateway", "source": "cloudflare"}
        ],
        "fields": {
            "session_id": "forged_cf_001",
            "user_agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120.0.0.0 Safari/537.36",
            "os_family": "windows",
            "form_class": "desktop",
            "platform": "Win32",
            "screen_width": 1920,
            "screen_height": 1080,
            "timezone": "Asia/Shanghai",
            "hardware_concurrency": 8,
            "device_memory": 8,
            "languages": ["zh-CN", "zh"],
            "site_locale": "zh",
            "webgl_unmasked_renderer": "ANGLE (Intel, Intel(R) UHD Graphics 620)",
            "webgl_unmasked_vendor": "Intel",
            "automation": {"webdriver": false},
            "webdriver": false,
            "cf_edge_stub": true
        }
    });
    let out = evaluate_session(&evidence, None, None, None, true).expect("evaluate");
    assert_ne!(
        out["real_band"], "confirmed_real",
        "forged CF must not confirm; got band={} truth={:?}",
        out["real_band"],
        out["truth"]
    );
    assert_eq!(out["soft_promote"], false);
    // Must still look like missing/untrusted server
    let xsrc = out["xsrc_status"].as_str().unwrap_or("");
    assert!(
        matches!(xsrc, "missing_server" | "partial"),
        "expected missing_server/partial, got {xsrc}"
    );
    assert_eq!(out["truth"]["has_server_side"], false);
}

/// Empty cf_fields {} or stub-only must not satisfy verified CF path.
#[test]
fn empty_cf_fields_not_server_side() {
    let evidence = json!({
        "session_id": "empty_cf",
        "sources": ["main", "cloudflare"],
        "cf_fields": {},
        "batches": [
            {"batch_id": "B0_bootstrap", "source": "main"},
            {"batch_id": "B1_conflict", "source": "main"},
            {"batch_id": "B2_hardware", "source": "main"},
            {"batch_id": "B3_system", "source": "main"},
            {"batch_id": "B12_anti_camouflage", "source": "main"}
        ],
        "fields": {
            "user_agent": "Mozilla/5.0 Chrome/120",
            "os_family": "windows",
            "form_class": "desktop",
            "screen_width": 1920,
            "screen_height": 1080,
            "timezone": "Asia/Shanghai",
            "hardware_concurrency": 8,
            "webgl_unmasked_renderer": "ANGLE",
            "automation": {"webdriver": false}
        }
    });
    let out = evaluate_session(&evidence, None, None, None, true).expect("evaluate");
    assert_ne!(out["real_band"], "confirmed_real");
    assert_eq!(out["truth"]["has_server_side"], false);
}

/// Real gateway + full lite5 still confirms (control: trusted server path).
#[test]
fn verified_gateway_with_lite5_can_confirm() {
    let evidence = load_fixture("confirmed_eligible.json");
    let out = evaluate_session(&evidence, None, None, None, true).expect("evaluate");
    assert_eq!(out["real_band"], "confirmed_real");
    assert_eq!(out["truth"]["has_server_side"], true);
}

#[test]
fn product_device_id_and_authenticity_projection() {
    let mut evidence = load_fixture("confirmed_eligible.json");
    // Pin RPA plan explicit: the default free plan stubs rpa analysis, but this
    // test exercises the full control-plane product surface (os/br/rpa norm/10).
    if let Some(fo) = evidence.get_mut("fields").and_then(|f| f.as_object_mut()) {
        fo.insert("rpa_analysis_enabled".into(), json!(true));
    }
    let out = evaluate_session(&evidence, None, None, None, true).expect("evaluate");
    assert!(out["device"]["linkable"].as_bool().unwrap());
    let did = out["device"]["device_id"].as_str().expect("device_id");
    // Multi-segment commercial surface: dv0|dv4|dv5|dv6 (legacy dh_/dv_ accepted).
    assert!(
        did.starts_with("dv0-")
            || did.starts_with("dv4-")
            || did.starts_with("dv5-")
            || did.starts_with("dv6-")
            || did.starts_with("dv-")
            || did.starts_with("dv_")
            || did.starts_with("dh-")
            || did.starts_with("dh_"),
        "device_id={did}"
    );
    // Multi-segment map when present
    if let Some(segs) = out["device"]["device_id_segments"].as_object() {
        for p in ["dv0", "dv4", "dv5", "dv6"] {
            assert!(segs.contains_key(p), "missing segment {p} in {segs:?}");
        }
    }
    // Public product: device_id + os/br/rpa (norm/10); diagnostics hold real_band/authenticity.
    assert!(out["product"]["os"]["score"].as_f64().is_some());
    assert!(out["product"]["br"]["score"].as_f64().is_some());
    assert!(out["product"]["rpa"]["score"].as_f64().is_some());
    assert_eq!(out["diagnostics"]["real_band"], "confirmed_real");
    assert_eq!(out["diagnostics"]["soft_promote"], false);
    assert_eq!(out["diagnostics"]["authenticity"]["multi_source_ok"], true);
    assert_eq!(out["defaults"]["bot_algo"], "balanced");
    assert_eq!(out["defaults"]["link_algo"], "sparse_safe");
}

#[test]
fn thin_has_no_strong_device_id() {
    let evidence = load_fixture("thin_ua_server.json");
    let out = evaluate_session(&evidence, None, None, None, true).expect("evaluate");
    assert_ne!(out["real_band"], "confirmed_real");
    // UA-only thin: not linkable; multi-segment may exist with silicon placeholders = 0
    assert_eq!(out["device"]["linkable"], false);
    let did = out["device"]["device_id"].as_str().unwrap_or("");
    assert!(
        !(did.starts_with("dh-") || did.starts_with("dh_")),
        "thin must not mint legacy dh commercial id, got {did:?}"
    );
    if let Some(body) = did.strip_prefix("dv0-") {
        let parts: Vec<&str> = body.split('-').collect();
        // res / wg / au / cp should be placeholder for thin UA-only
        for (i, name) in ["res", "wg", "au", "cp"].iter().enumerate() {
            assert_eq!(
                parts.get(i).copied().unwrap_or("?"),
                "0",
                "thin silicon part {name} must be 0, got {did}"
            );
        }
    }
}

/// Gateway bot UA vs main Chrome claim must not look consistent/confirmed when gateway_fields preserved.
#[test]
fn gateway_bot_vs_main_chrome_is_conflict_not_confirmed() {
    let evidence = json!({
        "session_id": "gw_conflict_1",
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "gateway_fields": {
            "user_agent": "Googlebot/2.1 (+http://www.google.com/bot.html)"
        },
        "batches": [
            {"batch_id": "B8_gateway", "source": "gateway"},
            {"batch_id": "B0_bootstrap", "source": "main"},
            {"batch_id": "B1_conflict", "source": "main"},
            {"batch_id": "B2_hardware", "source": "main"},
            {"batch_id": "B3_system", "source": "main"},
            {"batch_id": "B12_anti_camouflage", "source": "main"}
        ],
        "fields": {
            "user_agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120.0.0.0 Safari/537.36",
            "os_family": "windows",
            "form_class": "desktop",
            "platform": "Win32",
            "screen_width": 1920,
            "screen_height": 1080,
            "timezone": "Asia/Shanghai",
            "hardware_concurrency": 8,
            "webgl_unmasked_renderer": "ANGLE",
            "automation": {"webdriver": false},
            "webdriver": false
        }
    });
    let out = evaluate_session(&evidence, None, None, None, true).expect("evaluate");
    assert_ne!(out["real_band"], "confirmed_real", "band={}", out["real_band"]);
    assert_eq!(out["xsrc_status"], "conflict");
}

#[test]
fn inject_dual_primary_rejected() {
    let gate = inject_deploy_gate("nginx", "cfg_only", Some(true), Some(true));
    assert_eq!(gate["ok"], false);
    let errs = gate["errors"].as_array().unwrap();
    assert!(!errs.is_empty());
    assert!(errs.iter().any(|e| e.as_str().unwrap_or("").contains("both enabled")
        || e.as_str().unwrap_or("").contains("at_most_one")
        || e.as_str().unwrap_or("").contains("PRIMARY")));
}

#[test]
fn inject_single_flight_ok() {
    let plan = plan_inject("nginx", "cfg_only", false);
    assert!(plan.ok);
    assert_eq!(plan.boot_script_loaders.len(), 1);
    let sim = simulate_document_boot("nginx", "cfg_only");
    assert_eq!(sim["boot_runs"], 1);
    assert_eq!(sim["single_flight"], true);
    let gate = inject_deploy_gate("nginx", "cfg_only", None, None);
    assert_eq!(gate["ok"], true);
}

#[test]
fn soft_pair_never_promotes() {
    let a = load_fixture("peer_same_gpu.json");
    // use confirmed as b if peer is vector-like
    let b = load_fixture("confirmed_eligible.json");
    // build synthetic pair with same client_ref for P0
    let a_ev = json!({
        "session_id": "s1",
        "client_ref": "ref-1",
        "ts_ms": 1000,
        "fields": a.get("fields").cloned().unwrap_or(a.clone()),
        "env_class": "residential",
        "net_shard": "n1",
        "asn_class": "a1",
    });
    let b_ev = json!({
        "session_id": "s2",
        "client_ref": "ref-1",
        "ts_ms": 1100,
        "fields": b["fields"].clone(),
        "env_class": "residential",
        "net_shard": "n1",
        "asn_class": "a1",
    });
    let dec = soft_pair_decision(&a_ev, &b_ev, None);
    assert_eq!(dec["promote_to_commercial_id"], false);
    if let Some(edge) = dec.get("edge") {
        if !edge.is_null() {
            assert_eq!(edge["promote_to_commercial_id"], false);
        }
    }
}

#[test]
fn soft_engine_edges_never_promote() {
    let mut eng = SoftBlockingEngine::new(None);
    eng.observe(SoftMember {
        tenant: "t".into(),
        session_id: "a".into(),
        hs2: "hs2_abc".into(),
        net_shard: Some("net1".into()),
        asn_class: Some("asn1".into()),
        client_ref: None,
        ts_ms: 1_000,
        device_id_v2: None,
        env_class: "residential".into(),
        server_client_ip: Some("1.1.1.1".into()),
        hw_curves: Default::default(),
    });
    eng.observe(SoftMember {
        tenant: "t".into(),
        session_id: "b".into(),
        hs2: "hs2_abc".into(),
        net_shard: Some("net1".into()),
        asn_class: Some("asn1".into()),
        client_ref: None,
        ts_ms: 2_000,
        device_id_v2: None,
        env_class: "residential".into(),
        server_client_ip: Some("8.8.8.8".into()),
        hw_curves: Default::default(),
    });
    let edges = eng.edges(None);
    assert!(!edges.is_empty());
    assert!(edges.iter().all(|e| !e.promote_to_commercial_id));
}

#[test]
fn link_peer_same_gpu_sparse_safe() {
    let evidence = load_fixture("confirmed_eligible.json");
    let peer = load_fixture("peer_same_gpu.json");
    // peer_same_gpu is a vector
    let out = evaluate_session(&evidence, None, Some(&peer), None, true).expect("eval");
    assert!(out["link"].is_object());
    assert_eq!(out["link"]["algo"], "sparse_safe");
}

#[test]
fn evaluate_session_is_deterministic() {
    let evidence = load_fixture("fe_only.json");
    let a = evaluate_session(&evidence, None, None, None, true).unwrap();
    let b = evaluate_session(&evidence, None, None, None, true).unwrap();
    assert_eq!(a["real_band"], b["real_band"]);
    assert_eq!(a["soft_promote"], b["soft_promote"]);
    assert_eq!(a["defaults"], b["defaults"]);
    // Pack *set* must be stable (order may still vary under residual budget ties).
    let mut pa: Vec<_> = a["route_plan"]["packs"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["pack_id"].as_str().map(|s| s.to_string()))
        .collect();
    let mut pb: Vec<_> = b["route_plan"]["packs"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["pack_id"].as_str().map(|s| s.to_string()))
        .collect();
    pa.sort();
    pb.sort();
    assert_eq!(pa, pb, "route_plan pack_id set must be deterministic");
}
