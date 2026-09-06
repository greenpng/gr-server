//! Product surface (norm/10) + task gaps (norm/11) against shipped evaluate_session.
//! No gold-env overrides: scores must move with fields only.

use gr_probe_core::brain::scan_gaps;
use gr_probe_core::evaluate::evaluate_session;
use gr_probe_core::product_matrix::{load_field_product_matrix, load_task_gap_map};
use gr_probe_core::product_scores::score_os;
use gr_probe_core::stack_auth::stack_auth_from_fields;
use gr_probe_core::xsrc::evaluate_xsrc;
use serde_json::{json, Value};
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

fn product_score(out: &Value, key: &str) -> f64 {
    out["product"][key]["score"]
        .as_f64()
        .unwrap_or_else(|| panic!("missing product.{key}.score: {}", out["product"]))
}

#[test]
fn product_has_device_os_br_rpa_and_diagnostics() {
    let mut evidence = load_fixture("confirmed_eligible.json");
    // Pin the RPA input so the full default product surface is exercised.
    if let Some(fo) = evidence["fields"].as_object_mut() {
        fo.insert("rpa_analysis_enabled".into(), json!(true));
    } else {
        evidence
            .as_object_mut()
            .unwrap()
            .insert("rpa_analysis_enabled".into(), json!(true));
    }
    let out = evaluate_session(&evidence, None, None, None, true).expect("evaluate");
    assert!(out["product"].get("device_id").is_some());
    assert!(out["product"]["os"]["score"].as_f64().is_some());
    assert!(out["product"]["br"]["score"].as_f64().is_some());
    assert!(out["product"]["rpa"]["score"].as_f64().is_some());
    for k in ["os", "br", "rpa"] {
        let s = product_score(&out, k);
        assert!((0.0..=1.0).contains(&s), "{k} score out of range: {s}");
        assert!(out["product"][k]["coverage"].as_f64().is_some());
        assert!(out["product"][k]["reasons"].is_array());
        assert!(out["product"][k]["status"].as_str().is_some());
    }
    // Diagnostics hold bot/real_band — not substitutes for public product scores
    assert!(out["diagnostics"]["real_band"].as_str().is_some() || out["real_band"].as_str().is_some());
    assert!(out["diagnostics"].get("authenticity").is_some());
    assert!(out["diagnostics"].get("decision").is_some());
    // Public product must not bury only real_band as the main contract
    assert!(out["product"].get("os").is_some());
    assert!(out["product"].get("br").is_some());
}

#[test]
fn automation_fixture_lower_br_than_confirmed_humanish() {
    let clean = evaluate_session(&load_fixture("confirmed_eligible.json"), None, None, None, true)
        .expect("clean");
    let crawler =
        evaluate_session(&load_fixture("bot_crawler.json"), None, None, None, true).expect("crawler");
    let br_clean = product_score(&clean, "br");
    let br_crawler = product_score(&crawler, "br");
    assert!(
        br_crawler < br_clean,
        "crawler br ({br_crawler}) should be lower than confirmed ({br_clean})"
    );
    assert!(
        br_crawler < 0.55,
        "crawler br should be clearly unsafe-ish, got {br_crawler}"
    );
}

#[test]
fn high_vm_score_lowers_os_polarity() {
    let mut base = load_fixture("confirmed_eligible.json");
    let low = evaluate_session(&base, None, None, None, true).unwrap();
    if let Some(fo) = base["fields"].as_object_mut() {
        fo.insert("vm_score".into(), json!(0.92));
        fo.insert("stack_class".into(), json!("vm"));
        fo.insert("residual_soft_like".into(), json!(true));
    }
    let high_vm = evaluate_session(&base, None, None, None, true).unwrap();
    let os_low_risk = product_score(&low, "os");
    let os_high_vm = product_score(&high_vm, "os");
    assert!(
        os_high_vm < os_low_risk,
        "high vm must lower os safety: high_vm={os_high_vm} base={os_low_risk}"
    );
}

#[test]
fn webdriver_lowers_br_and_rpa() {
    let mut base = load_fixture("confirmed_eligible.json");
    if let Some(fo) = base["fields"].as_object_mut() {
        fo.insert("rpa_analysis_enabled".into(), json!(true));
        fo.insert("page_id".into(), json!("p_test_1"));
        fo.insert("behavior_early_bound".into(), json!(true));
        fo.insert("behavior_count".into(), json!(12));
        fo.insert(
            "behavior_events".into(),
            json!([{"t": 1, "type": "pointer"}]),
        );
        fo.insert("webdriver".into(), json!(false));
        fo.insert("automation".into(), json!({"webdriver": false}));
    }
    let humanish = evaluate_session(&base, None, None, None, true).unwrap();
    if let Some(fo) = base["fields"].as_object_mut() {
        fo.insert("webdriver".into(), json!(true));
        fo.insert("automation".into(), json!({"webdriver": true, "playwright": true}));
    }
    let auto = evaluate_session(&base, None, None, None, true).unwrap();
    assert!(
        product_score(&auto, "br") < product_score(&humanish, "br"),
        "webdriver should lower br"
    );
    assert!(
        product_score(&auto, "rpa") < product_score(&humanish, "rpa"),
        "webdriver should lower rpa"
    );
    assert!(auto["page"]["page_id"].as_str() == Some("p_test_1"));
    assert!(auto["page"]["rpa"]["score"].as_f64().is_some());
}

#[test]
fn no_behavior_rpa_not_full_human() {
    let mut base = load_fixture("confirmed_eligible.json");
    if let Some(fo) = base["fields"].as_object_mut() {
        fo.insert("rpa_analysis_enabled".into(), json!(true));
        fo.insert("page_id".into(), json!("p_empty"));
        fo.remove("behavior_early_bound");
        fo.remove("behavior_events");
        fo.remove("behavior_count");
    }
    let out = evaluate_session(&base, None, None, None, true).unwrap();
    let rpa = product_score(&out, "rpa");
    assert!(
        rpa < 0.55,
        "no behavior must not claim human full score, got {rpa}"
    );
    let st = out["product"]["rpa"]["status"].as_str().unwrap_or("");
    assert!(
        st == "unknown" || st == "low",
        "no-behavior rpa status should stay unknown/low, got {st}"
    );
}

#[test]
fn env_hostname_does_not_force_product_scores() {
    // Setting lab-like env must not create allowlist overrides in scoring.
    std::env::set_var("GR_LAB_FORCE_PASS", "1");
    std::env::set_var("HOSTNAME", "lab-gold-host.example");
    let a = evaluate_session(&load_fixture("bot_crawler.json"), None, None, None, true).unwrap();
    std::env::remove_var("GR_LAB_FORCE_PASS");
    std::env::remove_var("HOSTNAME");
    let b = evaluate_session(&load_fixture("bot_crawler.json"), None, None, None, true).unwrap();
    assert_eq!(
        product_score(&a, "br"),
        product_score(&b, "br"),
        "env vars must not change product scores"
    );
    assert!(product_score(&a, "br") < 0.55);
}

#[test]
fn task_gaps_include_product_task_codes() {
    let evidence = load_fixture("fe_only.json");
    let gaps = scan_gaps(&evidence, None).expect("scan_gaps");
    let codes: Vec<_> = gaps.iter().map(|g| g.code.as_str()).collect();
    assert!(
        codes.iter().any(|c| c.starts_with("need_")),
        "expected product task need_* codes, got {codes:?}"
    );
    let tmap = load_task_gap_map().expect("task_gap_map");
    let packs = tmap.packs_for_gap_codes(codes.iter().copied(), true);
    assert!(
        !packs.is_empty(),
        "task_gap_map should map gaps to packs"
    );
    // Shared multi-axis: one pack family can serve multiple tasks
    assert!(
        packs.contains(&"B8_gateway_early")
            || packs.contains(&"B0_bootstrap")
            || packs.contains(&"B2_hardware")
            || packs.contains(&"B1_conflict")
            || packs.contains(&"B11_interaction"),
        "unexpected packs {packs:?}"
    );
}

#[test]
fn field_matrix_shared_multi_axis() {
    let m = load_field_product_matrix().expect("matrix");
    let multi = m.multi_axis_material_fields();
    assert!(
        !multi.is_empty(),
        "matrix should list fields serving multiple axes"
    );
    // residual_mean / hardware_concurrency style sharing
    assert!(
        multi.iter().any(|f| f.contains("residual") || f.contains("hardware") || f.contains("hw_curve")),
        "expected shared hardware-ish fields, got {multi:?}"
    );
    // G-ARCH-21: every field carries trust_tier T0–T5
    for row in &m.fields {
        assert!(
            matches!(
                row.trust_tier.as_str(),
                "T0" | "T1" | "T2" | "T3" | "T4" | "T5"
            ),
            "field {} bad trust_tier {}",
            row.field,
            row.trust_tier
        );
    }
    assert!(m.fields.iter().any(|r| r.trust_tier == "T1"));
}

#[test]
fn pure_score_os_polarity_without_fixture() {
    let fields_ok = json!({
        "vm_score": 0.05,
        "stack_class": "native",
        "hardware_concurrency": 8,
        "device_memory": 16,
        "hw_curve_webgl": [0.1, 0.2, 0.3, 0.4],
        "form_class": "desktop"
    });
    let fields_vm = json!({
        "vm_score": 0.95,
        "stack_class": "vm",
        "residual_soft_like": true,
        "hardware_concurrency": 2,
        "device_memory": 2
    });
    let st_ok = stack_auth_from_fields(&fields_ok);
    let st_vm = stack_auth_from_fields(&fields_vm);
    let ev = json!({"fields": fields_ok, "sources": ["main", "gateway"], "has_gateway": true});
    let truth = evaluate_xsrc(&ev, None).unwrap();
    let os_ok = score_os(&fields_ok, &st_ok, &truth, &[]);
    let os_vm = score_os(&fields_vm, &st_vm, &truth, &[]);
    assert!(
        os_vm["score"].as_f64().unwrap() < os_ok["score"].as_f64().unwrap()
    );
}
