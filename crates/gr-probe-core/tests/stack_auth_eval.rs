//! Integration: stack_auth + evaluate_session for demo→v5 residual/spoof signals.
//! Drives shipped evaluate_session / stack_auth_from_fields — no reimplementation.

use gr_probe_core::brain::{build_frontier, scan_gaps};
use gr_probe_core::evaluate::evaluate_session;
use gr_probe_core::link::associate;
use gr_probe_core::stack_auth::{gpu_label_commercial_ok, stack_auth_from_fields};
use serde_json::{json, Value};

fn base_evidence(fields: Value) -> Value {
    json!({
        "session_id": "sess_stack_auth_test",
        "sources": ["main", "gateway"],
        "batches": [
            {"batch_id": "B0_bootstrap", "source": "main"},
            {"batch_id": "B1_conflict", "source": "main"},
            {"batch_id": "B2_hardware", "source": "main"},
            {"batch_id": "B3_system", "source": "main"},
            {"batch_id": "B12_anti_camouflage", "source": "main"},
            {"batch_id": "B8_gateway", "source": "gateway"},
            {"batch_id": "B10_hw_curves", "source": "main"}
        ],
        "has_gateway": true,
        "gateway_fields": {"user_agent": "Mozilla/5.0"},
        "fields": fields
    })
}

fn common_machine() -> Value {
    json!({
        "user_agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/120.0.0.0 Safari/537.36",
        "platform": "Linux x86_64",
        "os_family": "linux",
        "form_class": "desktop",
        "screen_width": 1920,
        "screen_height": 1080,
        "timezone": "Asia/Shanghai",
        "hardware_concurrency": 12,
        "device_memory": 16,
        "languages": ["en-US"],
        "site_locale": "en",
        "automation": {"webdriver": false},
        "hw_curve_audio": [0.1, 0.2, 0.15, 0.18, 0.22, 0.19, 0.17, 0.21, 0.16, 0.14, 0.2, 0.18],
        "hw_curve_webgl": [0.03, 0.04, 0.05, 0.06, 0.05, 0.04, 0.03, 0.05, 0.06, 0.04, 0.03, 0.05, 0.04, 0.06, 0.05, 0.04]
    })
}

#[test]
fn real_gpu_residual_stack_and_evaluate() {
    let mut f = common_machine();
    let obj = f.as_object_mut().unwrap();
    obj.insert(
        "webgl_unmasked_renderer".into(),
        json!("ANGLE (NVIDIA Corporation, NVIDIA GeForce GTX 1050 Ti/PCIe/SSE2, OpenGL 4.5.0)"),
    );
    obj.insert("residual_mean".into(), json!(0.5001811906403189));
    obj.insert("webgl_max_texture".into(), json!(32768));

    let auth = stack_auth_from_fields(&f);
    assert_eq!(auth.stack_class, "real_silicon");
    assert!(!auth.gpu_label_untrusted);
    assert!(gpu_label_commercial_ok(&auth));
    assert!(!auth.claims_host_silicon_recovery);

    let out = evaluate_session(&base_evidence(f), None, None, None, true).expect("eval");
    assert_eq!(out["soft_promote"], json!(false));
    assert_eq!(out["device"]["stack_auth"]["stack_class"], "real_silicon");
    assert_eq!(
        out["diagnostics"]["authenticity"]["claims_host_silicon_recovery"],
        false
    );
}

#[test]
fn soft_gl_residual_not_promote() {
    let mut f = common_machine();
    let obj = f.as_object_mut().unwrap();
    obj.insert(
        "webgl_unmasked_renderer".into(),
        json!("ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero)), SwiftShader driver)"),
    );
    obj.insert("residual_mean".into(), json!(0.5002766927083336));

    let auth = stack_auth_from_fields(&f);
    assert_eq!(auth.stack_class, "soft_render");
    assert!(auth.soft_stack);

    let out = evaluate_session(&base_evidence(f), None, None, None, true).expect("eval");
    assert_eq!(out["soft_promote"], json!(false));
    assert_eq!(out["device"]["stack_auth"]["stack_class"], "soft_render");
    // Soft still emits commercial id when form+curves present (multi-segment or legacy dv_/dg_)
    let id = out["device"]["device_id"].as_str().unwrap_or("");
    assert!(
        id.starts_with("dv0-")
            || id.starts_with("dv4-")
            || id.starts_with("dv5-")
            || id.starts_with("dv6-")
            || id.starts_with("dv_")
            || id.starts_with("dv-")
            || id.starts_with("dg_")
            || id.starts_with("dg-"),
        "soft stack should still emit commercial device_id: {id}"
    );
    assert!(!id.starts_with("dh_"), "soft stack must not exclusive dh: {id}");
}

#[test]
fn fp_spoof_high_end_label_soft_residual_strips_gpu_label() {
    let mut f = common_machine();
    let obj = f.as_object_mut().unwrap();
    // Camoufox-style: NVIDIA label + soft residual mean
    obj.insert(
        "webgl_unmasked_renderer".into(),
        json!("NVIDIA GeForce GTX 980, or similar"),
    );
    obj.insert("residual_mean".into(), json!(0.5002774107689955));
    obj.insert("webgl_max_texture".into(), json!(32768));

    let auth = stack_auth_from_fields(&f);
    assert_eq!(auth.stack_class, "soft_render");
    assert!(auth.spoof_score >= 0.4);
    assert!(auth.gpu_label_untrusted);
    assert!(!gpu_label_commercial_ok(&auth));

    let out = evaluate_session(&base_evidence(f), None, None, None, true).expect("eval");
    assert_eq!(out["soft_promote"], json!(false));
    assert_eq!(out["device"]["stack_auth"]["gpu_label_untrusted"], true);
    // GPU label must not appear as trusted hard material when untrusted
    let present = out["device"]["hard_materials_present"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        !present.iter().any(|v| v.as_str() == Some("webgl_unmasked_renderer")),
        "spoof GPU label must not be hard material present: {present:?}"
    );
    assert_eq!(out["diagnostics"]["authenticity"]["gpu_label_untrusted"], true);
    // Bot authenticity path may flag spoof
    let flags = out["bot"]["flags"].as_array().cloned().unwrap_or_default();
    assert!(
        flags.iter().any(|f| {
            f.as_str()
                .map(|s| s.contains("spoof") || s.contains("soft_stack") || s.contains("gpu_label"))
                .unwrap_or(false)
        }),
        "expected spoof/soft flag in bot: {flags:?}"
    );
}

#[test]
fn real_mobile_not_spoof() {
    let f = json!({
        "user_agent": "Mozilla/5.0 (Linux; Android 13; Pixel 7) AppleWebKit/537.36 Chrome/120.0.0.0 Mobile Safari/537.36",
        "platform": "Linux armv8l",
        "os_family": "android",
        "form_class": "mobile",
        "screen_width": 412,
        "screen_height": 915,
        "timezone": "Asia/Shanghai",
        "hardware_concurrency": 8,
        "webgl_unmasked_renderer": "Adreno (TM) 730",
        "residual_mean": 0.5001811906403189,
        "max_touch_points": 5,
        "automation": {"webdriver": false},
        "hw_curve_audio": [0.1, 0.2, 0.15, 0.18, 0.12, 0.14, 0.16, 0.19],
        "hw_curve_webgl": [0.04, 0.05, 0.06, 0.05, 0.04, 0.05, 0.06, 0.04]
    });
    let auth = stack_auth_from_fields(&f);
    assert_eq!(auth.stack_class, "real_silicon");
    assert!(auth.spoof_score < 0.4);
    assert!(!auth.gpu_label_untrusted);
}

#[test]
fn gap_when_gpu_without_residual() {
    let fields = json!({
        "user_agent": "Mozilla/5.0 Chrome/120",
        "webgl_unmasked_renderer": "ANGLE (NVIDIA, GeForce GTX 1050 Ti)",
        "platform": "Linux x86_64",
        "hardware_concurrency": 8,
        "timezone": "UTC"
    });
    let evidence = json!({
        "session_id": "gap_test",
        "sources": ["main"],
        "batches": [{"batch_id": "B2_hardware", "source": "main"}],
        "fields": fields
    });
    let gaps = scan_gaps(&evidence, None).expect("gaps");
    assert!(
        gaps.iter().any(|g| g.code == "stack_residual_missing"),
        "expected stack_residual_missing gap, got {:?}",
        gaps.iter().map(|g| &g.code).collect::<Vec<_>>()
    );
}

#[test]
fn frontier_schedules_b2_b10_on_stack_residual_missing() {
    // GPU label present, no residual — brain must request B2 + B10 (force recollect ok)
    let evidence = json!({
        "session_id": "frontier_residual_gap",
        "sources": ["main", "gateway"],
        "batches": [
            {"batch_id": "B0_bootstrap", "source": "main"},
            {"batch_id": "B1_conflict", "source": "main"},
            {"batch_id": "B2_hardware", "source": "main"},
            {"batch_id": "B3_system", "source": "main"},
            {"batch_id": "B12_anti_camouflage", "source": "main"},
            {"batch_id": "B8_gateway", "source": "gateway"}
        ],
        "has_gateway": true,
        "fields": {
            "user_agent": "Mozilla/5.0 (X11; Linux x86_64) Chrome/120.0.0.0 Safari/537.36",
            "platform": "Linux x86_64",
            "os_family": "linux",
            "webgl_unmasked_renderer": "ANGLE (NVIDIA, GeForce GTX 1050 Ti)",
            "hardware_concurrency": 8,
            "timezone": "UTC",
            "screen_width": 1920,
            "screen_height": 1080,
            "form_class": "desktop",
            "automation": {"webdriver": false}
        }
    });
    let gaps = scan_gaps(&evidence, None).expect("gaps");
    assert!(
        gaps.iter().any(|g| g.code == "stack_residual_missing"),
        "gap codes: {:?}",
        gaps.iter().map(|g| &g.code).collect::<Vec<_>>()
    );
    let plan = build_frontier(&evidence, true, None, 16).expect("frontier");
    let pack_ids: Vec<String> = plan
        .packs
        .iter()
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    assert!(
        pack_ids.iter().any(|id| id == "B2_hardware" || id == "lite.gpu"),
        "expected B2_hardware in frontier packs: {pack_ids:?}"
    );
    assert!(
        pack_ids.iter().any(|id| id == "B10_hw_curves" || id == "mid.curves" || id == "hard.curves"),
        "expected B10_hw_curves in frontier packs: {pack_ids:?}"
    );
    // force_recollect because B2 already present without residual
    let b2 = plan
        .packs
        .iter()
        .find(|p| {
            matches!(
                p.get("pack_id").and_then(|v| v.as_str()),
                Some("B2_hardware") | Some("lite.gpu")
            )
        })
        .expect("b2 pack");
    assert_eq!(b2.get("force_recollect").and_then(|v| v.as_bool()), Some(true));
    let route = plan.to_route_plan().expect("route");
    assert!(route.get("packs").and_then(|p| p.as_array()).is_some_and(|a| !a.is_empty()));

    // filter_unresolved_packs must KEEP force_recollect packs even when batch already present
    use gr_probe_core::brain::filter_unresolved_packs;
    use std::collections::HashSet;
    let mut already = HashSet::new();
    already.insert("B2_hardware".into());
    already.insert("B10_hw_curves".into());
    already.insert("lite.gpu".into());
    already.insert("mid.curves".into());
    let route_packs = route
        .get("packs")
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default();
    let kept = filter_unresolved_packs(&route_packs, &already);
    let kept_ids: Vec<&str> = kept
        .iter()
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()))
        .collect();
    assert!(
        kept_ids
            .iter()
            .any(|id| *id == "B2_hardware" || *id == "lite.gpu"),
        "force_recollect B2 must survive filter_unresolved when already present: {kept_ids:?}"
    );
    assert!(
        kept.iter().any(|p| {
            matches!(
                p.get("pack_id").and_then(|v| v.as_str()),
                Some("B2_hardware") | Some("lite.gpu")
            ) && p.get("force_recollect").and_then(|v| v.as_bool()) == Some(true)
        }),
        "kept B2 must still carry force_recollect=true"
    );
}

#[test]
fn link_strips_spoof_gpu_label_before_associate() {
    // Same residual soft cluster, spoofed high-end labels must not hard-match on GPU string alone
    let a = json!({
        "os_family": "linux",
        "platform": "Linux x86_64",
        "hardware_concurrency": 12,
        "timezone": "Asia/Shanghai",
        "screen_width": 1920,
        "screen_height": 1080,
        "form_class": "desktop",
        "webgl_unmasked_renderer": "NVIDIA GeForce GTX 980, or similar",
        "residual_mean": 0.5002774107689955,
        "webgl_max_texture": 32768,
    });
    let b = json!({
        "os_family": "linux",
        "platform": "Linux x86_64",
        "hardware_concurrency": 12,
        "timezone": "Asia/Shanghai",
        "screen_width": 1920,
        "screen_height": 1080,
        "form_class": "desktop",
        "webgl_unmasked_renderer": "NVIDIA GeForce GTX 980, or similar",
        "residual_mean": 0.5002774107689955,
        "webgl_max_texture": 32768,
    });
    let auth = stack_auth_from_fields(&a);
    assert!(auth.gpu_label_untrusted);
    let r = associate(&a, &b, "sparse_safe").expect("associate");
    // GPU label stripped → webgl_unmasked_renderer should not appear as matched hard GPU
    assert!(
        !r.matched.iter().any(|m| m == "webgl_unmasked_renderer"),
        "spoof GPU label must not match in LINK materials: matched={:?}",
        r.matched
    );
}

#[test]
fn no_gap_when_residual_present() {
    let fields = json!({
        "user_agent": "Mozilla/5.0 Chrome/120",
        "webgl_unmasked_renderer": "ANGLE (NVIDIA, GeForce GTX 1050 Ti)",
        "residual_mean": 0.5001811906403189,
        "stack_class": "real_silicon",
        "platform": "Linux x86_64"
    });
    let evidence = json!({
        "session_id": "gap_test2",
        "sources": ["main", "gateway"],
        "batches": [
            {"batch_id": "B2_hardware", "source": "main"},
            {"batch_id": "B8_gateway", "source": "gateway"}
        ],
        "has_gateway": true,
        "fields": fields
    });
    let gaps = scan_gaps(&evidence, None).expect("gaps");
    assert!(
        !gaps.iter().any(|g| g.code == "stack_residual_missing"),
        "should not gap when residual present: {:?}",
        gaps.iter().map(|g| &g.code).collect::<Vec<_>>()
    );
}
