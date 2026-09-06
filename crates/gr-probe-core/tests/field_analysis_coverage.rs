//! Commercial fields must map into field_product_matrix and feed analysis.

use gr_probe_core::evaluate::evaluate_session;
use gr_probe_core::product_matrix::load_field_product_matrix;
use gr_probe_core::product_scores::field_density;
use gr_probe_core::session_ticket::{
    issue_session_ticket, should_skip_session_probe, validate_session_ticket,
};
use gr_probe_core::brain::build_frontier;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

fn spec_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spec")
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn load_json(path: PathBuf) -> Value {
    let t = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&t).expect("json")
}

/// Load a session fixture with the RPA plan pinned explicit: the default free
/// the default product path includes RPA, but these tests exercise the full
/// control-plane product path.
fn load_fixture_with_rpa(name: &str) -> Value {
    let mut v = load_json(fixtures_dir().join(name));
    if let Some(fo) = v.get_mut("fields").and_then(|f| f.as_object_mut()) {
        fo.insert("rpa_analysis_enabled".into(), json!(true));
    }
    v
}

#[test]
fn all_coverage_commercial_fields_in_product_matrix() {
    let cov = load_json(spec_dir().join("field_coverage_matrix.json"));
    let mat = load_field_product_matrix().expect("matrix");
    let mfields: std::collections::HashSet<_> =
        mat.fields.iter().map(|f| f.field.as_str()).collect();
    let mut commercial = std::collections::HashSet::new();
    for section in ["static_packs", "dynamic_packs"] {
        if let Some(packs) = cov[section]["packs"].as_array() {
            for p in packs {
                for key in ["commercial_fields", "env_stack_fields_static"] {
                    if let Some(arr) = p[key].as_array() {
                        for f in arr {
                            if let Some(s) = f.as_str() {
                                commercial.insert(s.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    let missing: Vec<_> = commercial
        .iter()
        .filter(|f| !mfields.contains(f.as_str()))
        .cloned()
        .collect();
    assert!(
        missing.is_empty(),
        "commercial fields missing from field_product_matrix: {missing:?}"
    );
}

#[test]
fn multi_pack_human_has_field_hits_and_density() {
    let evidence = load_fixture_with_rpa("multi_pack_human.json");
    let out = evaluate_session(&evidence, None, None, None, true).expect("eval");
    let dens = out["diagnostics"]["field_density"]["ratio"]
        .as_f64()
        .unwrap_or(0.0);
    let present = out["diagnostics"]["field_density"]["present"]
        .as_u64()
        .unwrap_or(0);
    // Matrix grows with digests/SSOT (iss/18–19); ratio alone shrinks for fixed fixtures.
    // Multi-pack human must still show absolute breadth + non-zero critical density.
    assert!(
        present >= 10 && dens > 0.035,
        "expected rich multi-pack density present={present} dens={dens}"
    );
    let contrib = out["diagnostics"]["field_axis_contributions"]
        .as_object()
        .expect("contributions");
    for axis in ["os", "br", "rpa"] {
        let hits = contrib[axis].as_array().expect(axis);
        assert!(!hits.is_empty(), "axis {axis} should have field hits");
    }
    assert!(out["product"]["os"]["field_hits"].as_array().unwrap().len() >= 3);
    assert!(out["product"]["br"]["score"].as_f64().unwrap() > 0.4);
    assert!(out["product"]["rpa"]["score"].as_f64().unwrap() > 0.5);
    assert_eq!(out["page"]["page_id"], "page_checkout");
    assert!(out["session_ticket"].is_object() || out["session_ticket"].is_null());
}

#[test]
fn multi_pack_automation_lower_scores_than_human() {
    let human = evaluate_session(
        &load_fixture_with_rpa("multi_pack_human.json"),
        None,
        None,
        None,
        true,
    )
    .unwrap();
    let auto = evaluate_session(
        &load_fixture_with_rpa("multi_pack_automation.json"),
        None,
        None,
        None,
        true,
    )
    .unwrap();
    let hb = human["product"]["br"]["score"].as_f64().unwrap();
    let ab = auto["product"]["br"]["score"].as_f64().unwrap();
    let ho = human["product"]["os"]["score"].as_f64().unwrap();
    let ao = auto["product"]["os"]["score"].as_f64().unwrap();
    let hr = human["product"]["rpa"]["score"].as_f64().unwrap();
    let ar = auto["product"]["rpa"]["score"].as_f64().unwrap();
    assert!(ab < hb, "br auto {ab} >= human {hb}");
    assert!(ao < ho, "os auto {ao} >= human {ho}");
    assert!(ar < hr, "rpa auto {ar} >= human {hr}");
}

#[test]
fn session_ticket_skips_session_packs_keeps_rpa() {
    let evidence = load_json(fixtures_dir().join("multi_pack_human.json"));
    let out = evaluate_session(&evidence, None, None, None, true).unwrap();
    let ticket = out
        .get("session_ticket")
        .cloned()
        .filter(|t| t.is_object())
        .or_else(|| {
            issue_session_ticket(
                "sess_multi_human_001",
                &evidence["fields"],
                &out["product"],
                None,
            )
        })
        .expect("ticket");
    // Ticket-only validation (no meta skip flag) must succeed when session_id matches.
    assert!(validate_session_ticket(
        &ticket,
        "sess_multi_human_001",
        None,
        None
    ));
    // Rich fields recompute same fingerprint (device_id omitted on issue/validate).
    assert!(validate_session_ticket(
        &ticket,
        "sess_multi_human_001",
        Some(&evidence["fields"]),
        None
    ));

    // Cool-down + NEW page_id without behavior: must force-schedule B11 even if batch present.
    let mut cooled = evidence.clone();
    if let Some(obj) = cooled.as_object_mut() {
        obj.insert(
            "meta".into(),
            json!({"session_ticket": ticket.clone(), "skip_session_probe": true}),
        );
        obj.insert("session_ticket".into(), ticket.clone());
        obj.insert("page_id".into(), json!("page_new_after_cooldown"));
        // Mark B11 already collected (present) but strip page behavior for new page.
        let batches = obj
            .get("batches")
            .cloned()
            .unwrap_or(json!([]));
        obj.insert("batches".into(), batches);
        if let Some(fo) = obj.get_mut("fields").and_then(|f| f.as_object_mut()) {
            fo.insert("page_id".into(), json!("page_new_after_cooldown"));
            fo.remove("behavior_early_bound");
            fo.remove("behavior_events");
            fo.remove("behavior_count");
            fo.remove("pagehide_flush");
        }
    }
    // Ticket-only cool-down path (should_skip via ticket without relying only on meta flag)
    let mut ticket_only = cooled.clone();
    if let Some(obj) = ticket_only.as_object_mut() {
        obj.remove("skip_session_probe");
        if let Some(meta) = obj.get_mut("meta").and_then(|m| m.as_object_mut()) {
            meta.remove("skip_session_probe");
        }
    }
    assert!(
        should_skip_session_probe(&ticket_only),
        "ticket alone must enable cool-down"
    );

    let plan = build_frontier(&cooled, true, None, 12).expect("frontier");
    let packs: Vec<_> = plan
        .packs
        .iter()
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()))
        .collect();
    assert!(
        !packs.is_empty(),
        "cool-down + new page without behavior must schedule packs, got empty"
    );
    assert!(
        packs.iter().any(|p| *p == "B11_interaction"),
        "must schedule B11 for page rpa, got {packs:?}"
    );
    assert!(
        packs.iter().all(|p| {
            *p == "B11_interaction"
                || *p == "B1_conflict"
                || (p.starts_with("R") && p.contains("spotcheck"))
                || p.starts_with("B10x_") // cool-safe EDH silicon deepen
        }),
        "cool-down should only schedule rpa/conflict/verify/B10x packs, got {packs:?}"
    );
    assert!(
        packs.iter().any(|p| p.starts_with("B10x_silicon_")),
        "cool-down with B10 present must still schedule silicon B10x, got {packs:?}"
    );
    assert!(
        plan.notes.iter().any(|n| n.contains("cool_safe_b10x")),
        "notes should mention cool_safe_b10x, notes={:?}",
        plan.notes
    );
    let b11 = plan
        .packs
        .iter()
        .find(|p| p.get("pack_id").and_then(|v| v.as_str()) == Some("B11_interaction"))
        .expect("B11 pack");
    assert_eq!(
        b11.get("force_recollect").and_then(|v| v.as_bool()),
        Some(true),
        "B11 must be force_recollect on cool-down when page lacks behavior: {b11}"
    );
    assert!(
        plan.notes.iter().any(|n| n.contains("cool-down") || n.contains("skip_session")),
        "notes={:?}",
        plan.notes
    );
}

#[test]
fn score_rpa_accepts_kind_or_type_event_keys() {
    use gr_probe_core::bot::score_bot;
    use gr_probe_core::product_scores::score_rpa;
    let fields_kind = json!({
        "behavior_early_bound": true,
        "behavior_count": 0,
        "behavior_events": [
            {"t": 1, "kind": "pointermove"},
            {"t": 2, "kind": "click"},
            {"t": 3, "kind": "scroll"},
            {"t": 4, "kind": "keydown"}
        ]
    });
    let fields_type = json!({
        "behavior_early_bound": true,
        "behavior_count": 4,
        "behavior_events": [
            {"t": 1, "type": "pointermove"},
            {"t": 2, "type": "click"},
            {"t": 3, "type": "scroll"},
            {"t": 4, "type": "keydown"}
        ]
    });
    let bot = score_bot(&fields_kind, "balanced").unwrap();
    let sk = score_rpa(&fields_kind, &bot, Some("p1"));
    let st = score_rpa(&fields_type, &bot, Some("p1"));
    assert!(
        sk["score"].as_f64().unwrap() > 0.5,
        "kind events should score humanish: {sk}"
    );
    assert!(
        st["score"].as_f64().unwrap() > 0.5,
        "type events should score humanish: {st}"
    );
    assert!(
        sk["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r.as_str() == Some("behavior_type_diversity")),
        "kind diversity: {sk}"
    );
}

#[test]
fn field_density_pure_function() {
    let thin = json!({"user_agent": "x"});
    let rich = load_json(fixtures_dir().join("multi_pack_human.json"));
    let d1 = field_density(&thin);
    let d2 = field_density(&rich["fields"]);
    assert!(d2["ratio"].as_f64().unwrap() > d1["ratio"].as_f64().unwrap());
}
