//! Product-axis contract: device_id / os / br / rpa / lifecycle / residual vs labels.
//!
//! Drives **real** `evaluate_session` (shipped entry). No hard-coded expected scores;
//! asserts structural contract and polarity/separation that must hold on any traffic.

use gr_probe_core::evaluate::evaluate_session;
use gr_probe_core::product_matrix::load_field_product_matrix;
use gr_probe_core::return_gate::{
    should_analyze_page_rpa, should_return_identity_to_sdk, SDK_RETURN_IDLE_MS,
};
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

fn score(out: &Value, axis: &str) -> f64 {
    out["product"][axis]["score"]
        .as_f64()
        .unwrap_or_else(|| panic!("missing product.{axis}.score"))
}

fn reasons(out: &Value, axis: &str) -> Vec<String> {
    out["product"][axis]["reasons"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// Criterion 1: evaluate product always carries device + os + br + rpa blocks.
#[test]
fn product_surface_has_four_axes() {
    let mut evidence = load_fixture("confirmed_eligible.json");
    // OPUS5 closure: pin paid entitlement so the RPA axis carries a score
    // The default product surface includes RPA with a populated score.
    if let Some(fo) = evidence["fields"].as_object_mut() {
        fo.insert("rpa_analysis_enabled".into(), json!(true));
    }
    let out = evaluate_session(&evidence, None, None, None, true)
        .expect("evaluate");
    let p = &out["product"];
    assert!(p.get("device_id").is_some() || p.get("device_tier").is_some());
    for axis in ["os", "br", "rpa"] {
        assert!(p[axis]["score"].as_f64().is_some(), "missing {axis}.score");
        assert!(p[axis]["status"].as_str().is_some(), "missing {axis}.status");
        assert!(p[axis]["reasons"].is_array(), "missing {axis}.reasons");
        assert!(p[axis]["coverage"].as_f64().is_some());
    }
    // Ensemble multi-algo present on evaluate (device residual architecture)
    let ens = out.get("device_ensemble");
    assert!(ens.is_some(), "device_ensemble must be on evaluate result");
    let n = ens
        .unwrap()
        .pointer("/single/strategies")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    assert!(n >= 2, "ensemble must list multiple digest strategies, got {n}");
}

/// Criterion 1 lifecycle: richer evidence under same shape upgrades device/os/br (not frozen first batch).
#[test]
fn richer_materials_can_change_device_os_br() {
    let mut thin = json!({
        "fields": {
            "user_agent": "Mozilla/5.0 (X11; Linux x86_64) Chrome/120.0.0.0",
            "platform": "Linux x86_64",
            "os_family": "linux",
            "form_class": "desktop",
            "hardware_concurrency": 12,
            "timezone": "Asia/Singapore",
            "screen_width": 1920,
            "screen_height": 1080,
            "server_client_ip": "10.0.0.8",
        },
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "gateway_fields": {"server_client_ip": "10.0.0.8"},
        "batches": [{"batch_id": "B0_bootstrap", "source": "main"}],
        "session_id": "axis_contract_thin",
    });
    let thin_out = evaluate_session(&thin, None, None, None, true).expect("thin");

    // Attach residual curves (same machine physics) — not device name strings.
    let curve: Vec<f64> = (0..32)
        .map(|i| (i as f64 * 0.17 + 0.3).sin().abs() * 0.4 + 0.05)
        .collect();
    let audio: Vec<f64> = (0..64)
        .map(|i| (i as f64 * 0.11 + 0.1).sin().abs() * 0.3 + 0.05)
        .collect();
    if let Some(fo) = thin["fields"].as_object_mut() {
        fo.insert(
            "webgl_unmasked_renderer".into(),
            json!("ANGLE (NVIDIA GeForce GTX 1050 Ti)"),
        );
        fo.insert("hw_curve_webgl".into(), json!(curve));
        fo.insert("hw_curve_audio".into(), json!(audio));
    }
    thin["batches"] = json!([
        {"batch_id": "B0_bootstrap", "source": "main"},
        {"batch_id": "B10_hw_curves", "source": "main"},
        {"batch_id": "B2_hardware", "source": "main"},
    ]);
    thin["session_id"] = json!("axis_contract_rich");
    let rich_out = evaluate_session(&thin, None, None, None, true).expect("rich");

    let thin_id = thin_out["product"]["device_id"].as_str().unwrap_or("");
    let rich_id = rich_out["product"]["device_id"].as_str().unwrap_or("");
    // IDs may both be non-empty; at least one of id/tier/confidence/os/br must move with materials.
    let os_t = score(&thin_out, "os");
    let os_r = score(&rich_out, "os");
    let br_t = score(&thin_out, "br");
    let br_r = score(&rich_out, "br");
    let moved = thin_id != rich_id
        || (os_r - os_t).abs() > 0.02
        || (br_r - br_t).abs() > 0.02
        || thin_out["product"]["device_tier"] != rich_out["product"]["device_tier"];
    assert!(
        moved,
        "richer residual must change identity product surface: thin_id={thin_id} rich_id={rich_id} os {os_t}->{os_r} br {br_t}->{br_r}"
    );
    // Residual path must not use raw API model as sole digest material: never_digest SSOT
    let m = load_field_product_matrix().expect("matrix");
    assert!(
        m.commercial_device_id
            .never_digest
            .iter()
            .any(|s| s == "webgl_unmasked_renderer")
    );
    assert!(m
        .commercial_device_id
        .never_digest
        .iter()
        .any(|s| s == "user_agent"));
}

/// Criterion 2: os = env abnormality; rpa = automation — reasons must not collapse to one field.
#[test]
fn soft_env_and_automation_separate_os_vs_rpa() {
    // Soft / software-render environment (os primary)
    let mut soft = json!({
        "fields": {
            "form_class": "desktop",
            "platform": "Linux x86_64",
            "os_family": "linux",
            "hardware_concurrency": 8,
            "timezone": "UTC",
            "webgl_unmasked_renderer": "Google SwiftShader",
            "residual_soft_like": true,
            "soft_stack": true,
            "stack_class": "soft_render",
            "vm_score": 0.55,
            "server_client_ip": "127.0.0.1",
            "hw_curve_webgl": [0.1, 0.2, 0.15, 0.12, 0.11, 0.1, 0.09, 0.1],
        },
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "gateway_fields": {"server_client_ip": "127.0.0.1"},
        "session_id": "soft_env",
        "page_id": "p_soft",
    });
    if let Some(fo) = soft["fields"].as_object_mut() {
        fo.insert("rpa_analysis_enabled".into(), json!(true));
    }
    let soft_out = evaluate_session(&soft, None, None, None, true).expect("soft");
    let os_rs = reasons(&soft_out, "os");
    let rpa_rs = reasons(&soft_out, "rpa");
    let os_env = os_rs.iter().any(|r| {
        r.contains("soft") || r.contains("residual") || r.contains("vm_score") || r.contains("stack_class")
    });
    assert!(os_env, "soft env must appear in os reasons: {os_rs:?}");
    // rpa without behavior should not be driven by soft_stack alone
    let rpa_soft_as_main = rpa_rs.iter().any(|r| r.contains("soft_stack") || r.contains("residual_soft"));
    assert!(
        !rpa_soft_as_main,
        "soft residual must not be primary rpa reason: {rpa_rs:?}"
    );

    // Automation control-plane (rpa primary)
    let mut auto = json!({
        "fields": {
            "form_class": "desktop",
            "platform": "Linux x86_64",
            "os_family": "linux",
            "hardware_concurrency": 12,
            "timezone": "UTC",
            "webdriver": true,
            "automation": {"webdriver": true, "playwright": true},
            "behavior_early_bound": true,
            "behavior_count": 4,
            "behavior_events": [
                {"t": 1, "type": "pointer", "x": 10, "y": 10},
                {"t": 2, "type": "pointer", "x": 11, "y": 10},
                {"t": 3, "type": "click", "x": 11, "y": 10},
                {"t": 4, "type": "keydown"}
            ],
            "server_client_ip": "127.0.0.1",
            "stack_class": "native",
            "page_id": "p_auto",
        },
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "gateway_fields": {"server_client_ip": "127.0.0.1"},
        "session_id": "auto_ctrl",
        "page_id": "p_auto",
    });
    if let Some(fo) = auto["fields"].as_object_mut() {
        fo.insert("rpa_analysis_enabled".into(), json!(true));
    }
    let auto_out = evaluate_session(&auto, None, None, None, true).expect("auto");
    let rpa_a = reasons(&auto_out, "rpa");
    assert!(
        rpa_a.iter().any(|r| r.contains("webdriver") || r.contains("automation")),
        "automation must hit rpa reasons: {rpa_a:?}"
    );
    // page-scoped rpa when page_id present
    assert!(
        auto_out.get("page").is_some(),
        "page block required when page_id set"
    );
    assert!(auto_out["page"]["rpa"]["score"].as_f64().is_some());
}

/// Criterion 1: identity vs rpa return gates are distinct.
#[test]
fn identity_and_rpa_gates_differ() {
    let id_gate = should_return_identity_to_sdk(true, Some(0), SDK_RETURN_IDLE_MS + 1);
    assert_eq!(id_gate["emit"], true);
    let id_early = should_return_identity_to_sdk(true, Some(0), 1_000);
    assert_eq!(id_early["emit"], false);

    let rpa_close = should_analyze_page_rpa(true, Some(0), 100, true);
    assert_eq!(rpa_close["analyze"], true);
    let rpa_wait = should_analyze_page_rpa(false, Some(0), 1_000, true);
    assert_eq!(rpa_wait["analyze"], false);
}

/// iss/25: control-plane alone (no bio) still scores rpa; webdriver primary on rpa not br.
#[test]
fn control_plane_only_scores_rpa_not_unknown() {
    let mut ev = json!({
        "fields": {
            "form_class": "desktop",
            "platform": "Linux x86_64",
            "os_family": "linux",
            "hardware_concurrency": 8,
            "timezone": "UTC",
            "webdriver": true,
            "automation": {"webdriver": true, "playwright": true},
            "outer_zero": true,
            "cdp_runtime_hint": 2.0,
            "server_client_ip": "127.0.0.1",
            "page_id": "p_ctrl",
        },
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "gateway_fields": {"server_client_ip": "127.0.0.1"},
        "session_id": "ctrl_only",
        "page_id": "p_ctrl",
    });
    if let Some(fo) = ev["fields"].as_object_mut() {
        fo.insert("rpa_analysis_enabled".into(), json!(true));
    }
    let out = evaluate_session(&ev, None, None, None, true).expect("eval");
    let rpa_status = out["product"]["rpa"]["status"].as_str().unwrap_or("");
    assert_ne!(
        rpa_status, "unknown",
        "control-plane without bio must not leave rpa unknown: {:?}",
        out["product"]["rpa"]
    );
    let rpa_rs = reasons(&out, "rpa");
    assert!(
        rpa_rs.iter().any(|r| r.contains("webdriver") || r.contains("cdp") || r.contains("outer_zero")),
        "rpa must cite control-plane reasons: {rpa_rs:?}"
    );
    // br may note weak webdriver but rpa score should be lower (less safe) than br when only automation
    let rpa_s = score(&out, "rpa");
    assert!(
        rpa_s < 0.45,
        "control-plane rpa safety should be low, got {rpa_s}"
    );
}

/// iss/25: soft residual + multi_source claim agreement → os claim_collusion (not rpa).
#[test]
fn claim_collusion_soft_hits_os_not_rpa() {
    let ev = json!({
        "fields": {
            "form_class": "desktop",
            "platform": "Win32",
            "os_family": "windows",
            "hardware_concurrency": 8,
            "timezone": "UTC",
            "webgl_unmasked_renderer": "ANGLE (NVIDIA GeForce GTX 980)",
            "residual_soft_like": true,
            "soft_stack": true,
            "stack_class": "soft_render",
            "spoof_score": 0.62,
            "multi_source_match_ratio": 0.92,
            "gpu_label_untrusted": true,
            "server_client_ip": "127.0.0.1",
            "page_id": "p_fp",
        },
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "gateway_fields": {"server_client_ip": "127.0.0.1"},
        "session_id": "fp_claim",
        "page_id": "p_fp",
    });
    let out = evaluate_session(&ev, None, None, None, true).expect("eval");
    let os_rs = reasons(&out, "os");
    assert!(
        os_rs.iter().any(|r| r.contains("claim_collusion") || r.contains("soft") || r.contains("residual")),
        "os must show claim collusion / soft env: {os_rs:?}"
    );
    let rpa_rs = reasons(&out, "rpa");
    assert!(
        !rpa_rs.iter().any(|r| r.contains("claim_collusion")),
        "claim collusion must not be rpa primary: {rpa_rs:?}"
    );
}

/// demo/iss2 forgeability: headless_likely is rpa control-plane primary (not os).
#[test]
fn demo_headless_likely_hits_rpa() {
    let ev = json!({
        "fields": {
            "form_class": "desktop",
            "platform": "Linux x86_64",
            "os_family": "linux",
            "hardware_concurrency": 8,
            "timezone": "UTC",
            "headless_likely": true,
            "rpa_analysis_enabled": true,
            "server_client_ip": "127.0.0.1",
            "page_id": "p_hl",
        },
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "gateway_fields": {"server_client_ip": "127.0.0.1"},
        "session_id": "hl1",
        "page_id": "p_hl",
    });
    let out = evaluate_session(&ev, None, None, None, true).expect("eval");
    let rpa_rs = reasons(&out, "rpa");
    assert!(
        rpa_rs.iter().any(|r| r.contains("headless")),
        "headless_likely must score rpa: {rpa_rs:?}"
    );
    assert_ne!(out["product"]["rpa"]["status"].as_str(), Some("unknown"));
}

/// demo env aliases: soft_stack_suspect raises os, not rpa.
#[test]
fn demo_soft_stack_suspect_hits_os() {
    let ev = json!({
        "fields": {
            "form_class": "desktop",
            "platform": "Linux x86_64",
            "os_family": "linux",
            "hardware_concurrency": 4,
            "timezone": "UTC",
            "soft_stack_suspect": true,
            "pc_vm_suspect": true,
            "residual_soft_like": true,
            "server_client_ip": "127.0.0.1",
            "page_id": "p_ss",
        },
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "gateway_fields": {"server_client_ip": "127.0.0.1"},
        "session_id": "ss1",
        "page_id": "p_ss",
    });
    let out = evaluate_session(&ev, None, None, None, true).expect("eval");
    let os_rs = reasons(&out, "os");
    assert!(
        os_rs.iter().any(|r| r.contains("soft") || r.contains("demo_env") || r.contains("residual")),
        "soft env aliases must hit os: {os_rs:?}"
    );
}

/// iss/25 r3: timezone_offset_min is os material — name vs zero-offset is env forgery signal.
#[test]
fn timezone_offset_material_consumed_on_os() {
    let mut base = json!({
        "fields": {
            "form_class": "desktop",
            "platform": "Linux x86_64",
            "os_family": "linux",
            "hardware_concurrency": 8,
            "timezone": "Asia/Singapore",
            "timezone_offset_min": 0,
            "server_client_ip": "127.0.0.1",
        },
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "gateway_fields": {"server_client_ip": "127.0.0.1"},
        "session_id": "tz1",
    });
    let bad = evaluate_session(&base, None, None, None, true).expect("bad");
    let os_bad = reasons(&bad, "os");
    assert!(
        os_bad.iter().any(|r| r.contains("timezone_offset")),
        "os must consume timezone_offset_min material: {os_bad:?}"
    );
    // Correct offset for named zone (approx +480 for Singapore) should not raise that reason
    if let Some(fo) = base["fields"].as_object_mut() {
        fo.insert("timezone_offset_min".into(), json!(480));
    }
    let ok = evaluate_session(&base, None, None, None, true).expect("ok");
    let os_ok = reasons(&ok, "os");
    assert!(
        !os_ok.iter().any(|r| r.contains("timezone_offset_vs_name_suspect")),
        "aligned offset must not flag name mismatch: {os_ok:?}"
    );
}

/// Criterion 2 device: commercial SSOT forbids label/network digests.
#[test]
fn commercial_never_digest_excludes_labels_and_network() {
    let m = load_field_product_matrix().expect("matrix");
    let never: Vec<&str> = m
        .commercial_device_id
        .never_digest
        .iter()
        .map(|s| s.as_str())
        .collect();
    for k in [
        "user_agent",
        "webgl_unmasked_renderer",
        "server_client_ip",
        "ja3_hash",
        "ja4",
    ] {
        assert!(never.contains(&k), "never_digest must include {k}, got {never:?}");
    }
    let order = &m.commercial_device_id.digest_order;
    assert!(
        order.iter().any(|s| s.contains("webgl") || s.contains("audio") || s == "hw_webgl_stable"),
        "digest_order must prefer residual anchors: {order:?}"
    );
    assert!(
        !order.iter().any(|s| s == "user_agent" || s == "webgl_unmasked_renderer"),
        "digest_order must not include forgeable labels"
    );
}
