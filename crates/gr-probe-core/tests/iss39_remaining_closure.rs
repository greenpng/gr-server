//! iss/39 R1–R14 residual closure (lean single binary).

use gr_probe_core::{
    aggregate_unknown_buckets, brand_to_engine_family, commercial_projection, hub_promotion_drafts,
    load_field_product_matrix, next_rotation, report_collision_kpi, score_br, score_os,
    stack_auth_from_fields, BotScore, TruthResult,
};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

fn truth() -> TruthResult {
    TruthResult {
        xsrc_status: "ok".into(),
        real_band: "likely_real".into(),
        credibility: 0.5,
        fe_only: true,
        has_server_side: true,
        has_main_core: true,
        packages: json!({}),
        reasons: vec![],
        details: json!({}),
    }
}

fn bot() -> BotScore {
    BotScore {
        score: 5,
        verdict: "human".into(),
        flags: vec![],
        family: "none".into(),
        algo: "t".into(),
        details: json!({}),
        robot_name: None,
    }
}

fn reasons(v: &Value) -> Vec<String> {
    v.get("reasons")
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(|s| s.to_string()))
        .collect()
}

fn core_src_blob() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = String::new();
    fn walk(dir: &PathBuf, out: &mut String) {
        if let Ok(rd) = fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
                    if let Ok(t) = fs::read_to_string(&p) {
                        out.push_str(&t);
                        out.push('\n');
                    }
                }
            }
        }
    }
    walk(&root, &mut out);
    out
}

#[test]
fn r1_runtime_never_digest_excluded_from_materials_included() {
    let m = load_field_product_matrix().unwrap();
    let never: Vec<&str> = m
        .commercial_device_id
        .never_digest
        .iter()
        .map(|s| s.as_str())
        .collect();
    let fields = json!({
        "form_class": "desktop",
        "user_agent": "Mozilla/5.0 Chrome/120",
        "server_client_ip": "203.0.113.1",
        "server_asn": "AS1",
        "ja4": "t13d1516h2_8daaf6152771_b0da82dd1658",
        "webgl_unmasked_renderer": "ANGLE (NVIDIA GeForce)",
        "webdriver": true,
        "hw_curve_webgl": [0.1, 0.2, 0.3, 0.4, 0.5, 0.6],
        "hw_curve_audio": [0.1, 0.2, 0.3, 0.4],
        "hardware_concurrency": 8,
        "timezone": "UTC",
        "architecture": "x86",
    });
    let proj = commercial_projection(&fields);
    let included = proj
        .get("materials_included")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    for k in &never {
        assert!(
            !included.iter().any(|x| x.as_str() == Some(*k)),
            "never_digest {k} leaked into materials_included: {included:?}"
        );
    }
    // materials map may contain claim labels but must not be in included digest keys
    assert!(proj["server_client_ip_in_digest"] == false);
}

#[test]
fn r2_r3_cloud_vendor_and_ua_audit_not_on_score_main_gate() {
    let blob = core_src_blob();
    // Ban: cloud vendor name as hard gate strings in score_* style if-branches — allow comments/docs
    // Heuristic: forbid exact match patterns that look like decision gates.
    let forbidden_gates = [
        "if ua.contains(\"aws\")",
        "if ua.contains(\"alibaba\")",
        "if ua.contains(\"huawei cloud\")",
        "cloud_vendor_blacklist",
        "brand_allowlist_gate",
    ];
    for f in forbidden_gates {
        assert!(
            !blob.contains(f),
            "forbidden gate pattern present: {f}"
        );
    }
    // UA substrings allowed only as claim/aux — ensure brand_to_engine still defaults unknown
    assert_eq!(brand_to_engine_family("MyCloudPhoneVendor99"), "unknown");
}

#[test]
fn r4_r5_emulator_rules_and_hub_drafts() {
    let fields = json!({
        "form_class": "mobile",
        "mobile_ua_claim": true,
        "residual_soft_like": true,
        "sensor_accel_present": false,
        "max_touch_points": 0,
        "server_asn": "AS64500",
        "emulator_hint": "android_emu",
        "hardware_concurrency": 8,
        "user_agent": "Mozilla/5.0 (Linux; Android 12) Mobile",
    });
    let os = score_os(&fields, &stack_auth_from_fields(&fields), &truth(), &[]);
    let rs = reasons(&os);
    assert!(
        rs.iter().any(|r| r.contains("emulator_form_cross") || r.contains("rule_sample")),
        "{rs:?}"
    );
    let hub = aggregate_unknown_buckets(&[json!({
        "session_id": "s1",
        "unknown_bucket": {"codes": ["out_of_envelope"], "tags": ["thin"], "present": true}
    })]);
    let drafts = hub_promotion_drafts(&hub);
    assert!(drafts["draft_count"].as_u64().unwrap() >= 1);
    assert_eq!(drafts["redlines"]["auto_edit_never_digest"], false);
}

#[test]
fn r7_r8_challenge_rotate_and_ja_prior() {
    let rot = next_rotation("tenant_a", 3, "secret");
    assert_eq!(rot["previous_epoch"], 3);
    assert!(rot["epoch"].as_u64().unwrap() >= 4);
    let fields = json!({
        "form_class": "desktop",
        "user_agent": "Mozilla/5.0 TotallyWeirdShell/1.0",
        "ja4": "unknown_tls13",
        "hardware_concurrency": 4,
    });
    let br = score_br(
        &fields,
        &stack_auth_from_fields(&fields),
        &bot(),
        &truth(),
        &[],
    );
    let rs = reasons(&br);
    assert!(
        rs.iter().any(|r| r.contains("ja_pop_prior")),
        "ja population prior missing: {rs:?}"
    );
}

#[test]
fn r9_cloud_phone_fixture_no_vendor_name_gate() {
    // Cloud-phone shaped: soft + mobile + egress + no sensors — relation rules, not vendor catalog
    let fields = json!({
        "form_class": "mobile",
        "mobile_ua_claim": true,
        "residual_soft_like": true,
        "soft_stack": true,
        "server_asn": "AS64512",
        "server_client_ip": "198.51.100.9",
        "sensor_accel_present": false,
        "sensor_gyro_present": false,
        "max_touch_points": 0,
        "hardware_concurrency": 8,
        "user_agent": "Mozilla/5.0 (Linux; Android 13; Generic) Mobile Safari/537.36",
    });
    let os = score_os(&fields, &stack_auth_from_fields(&fields), &truth(), &[]);
    let rs = reasons(&os);
    assert!(
        rs.iter().any(|r| r.contains("net_egress_aux_soft_mobile")
            || r.contains("emulator_form_cross")
            || r.contains("rule_sample")),
        "cloud-phone relation signals missing: {rs:?}"
    );
    let blob = commercial_projection(&fields).to_string();
    assert!(!blob.to_ascii_lowercase().contains("aws"));
    assert!(!blob.to_ascii_lowercase().contains("alibaba"));
}

#[test]
fn r10_fleet_kpi_from_synthetic_pool() {
    let mut pool = Vec::new();
    for i in 0..3 {
        pool.push(json!({
            "form_class": "desktop",
            "residual_soft_like": true,
            "unit_surface_id": "u_fleet",
            "hw_curve_webgl": [0.2, 0.3, 0.4],
            "hw_curve_audio": [0.2, 0.3],
            "hardware_concurrency": 8,
            "timezone": "UTC",
            "pool_label": format!("fleet_{i}"),
        }));
    }
    let kpi = report_collision_kpi(&pool);
    assert_eq!(kpi["ok"], true);
    assert!(kpi["soft_no_separator_pool"]["class_collides"].as_bool().unwrap_or(false));
}

#[test]
fn r12_rpa_kinematics_path_and_click_uniform() {
    use gr_probe_core::score_rpa;
    let fields = json!({
        "behavior_count": 8,
        "behavior_early_bound": true,
        "path_straightness": 0.99,
        "click_interval_cv": 0.02,
        "input_velocity_cv": 0.03,
        "pre_action_move_count": 0,
        "hardware_concurrency": 4,
    });
    let rpa = score_rpa(&fields, &bot(), None);
    let rs = reasons(&rpa);
    assert!(
        rs.iter().any(|r| r.contains("path_too_straight") || r.contains("click_interval_too_uniform") || r.contains("velocity_too_uniform")),
        "{rs:?}"
    );
}
