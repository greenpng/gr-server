//! Land iss/31–32: re_probe→brain, association ladder, task-critical conf, link/mint collision.

use gr_probe_core::{
    association_ladder, build_frontier, commercial_projection, evaluate_session,
    link_or_mint_pair, MemoryDeviceIndex, task_critical_fields,
};
use serde_json::{json, Value};

fn audio_curve() -> (Vec<f64>, Vec<f64>) {
    let a: Vec<f64> = (0..32)
        .map(|i| (i as f64 * 0.11).sin().abs() * 0.5 + 0.05)
        .collect();
    let w: Vec<f64> = (0..16)
        .map(|i| ((i * 13) % 200) as f64 / 200.0 + 0.01)
        .collect();
    (a, w)
}

fn residual_rich() -> Value {
    let (a, w) = audio_curve();
    json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "hardware_concurrency": 12,
        "timezone": "Asia/Shanghai",
        "os_family": "linux",
        "architecture": "x86",
        "webgl_unmasked_renderer": "ANGLE (NVIDIA, GTX 1050 Ti)",
        "residual_mean": 0.500181,
        "residual_soft_like": false,
        "hw_curve_audio": a,
        "hw_curve_webgl": w,
        "webrtc_host_ip_hash": "lan_rich",
        "unit_surface_id": "unit_rich",
        "unit_surface_algo": "gr_unit_v1",
        "unit_multiround_stable": true,
        "multi_seed_n": 8,
        "webgl2_support": true,
        "webgl_max_texture": 16384,
        "screen_width": 1920,
        "screen_height": 1080,
        "server_client_ip": "203.0.113.10",
        "server_asn": "AS64500",
        "server_country": "CN",
        "user_agent": "Mozilla/5.0 Chrome/120.0.0.0",
        "chrome_runtime": true,
        "behavior_early_bound": true,
        "behavior_count": 20,
        "behavior_events": [{"kind":"mousemove"},{"kind":"click"},{"kind":"scroll"}],
    })
}

fn soft_fields(os: &str, unit: &str) -> Value {
    let (a, w) = audio_curve();
    json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "hardware_concurrency": 8,
        "timezone": "UTC",
        "os_family": "linux",
        "architecture": "x86",
        "webgl_unmasked_renderer": "SwiftShader",
        "residual_mean": 0.500423,
        "residual_soft_like": true,
        "soft_stack": true,
        "stack_class": "soft_render",
        "hw_curve_audio": a,
        "hw_curve_webgl": w,
        "unit_surface_id": unit,
        "unit_surface_algo": "gr_unit_v1",
        "unit_multiround_stable": true,
        "os_instance_hash": os,
        "screen_width": 1280,
        "screen_height": 720,
    })
}

fn thin_gateway() -> Value {
    json!({
        "user_agent": "thin",
        "form_class": "desktop",
        "server_client_ip": "198.51.100.7",
        "server_asn": "AS1",
        "server_country": "US",
    })
}

fn evidence(fields: Value, sources: &[&str], gw: bool) -> Value {
    let batches: Vec<Value> = sources
        .iter()
        .map(|s| {
            json!({
                "batch_id": if *s == "gateway" { "B8_gateway" } else { "B10_hw_curves" },
                "source": s,
            })
        })
        .collect();
    let mut ev = json!({
        "sources": sources,
        "batches": batches,
        "has_gateway": gw,
        "fields": fields,
        "session_id": "iss31_32",
        "page_id": "p1",
    });
    if gw {
        ev.as_object_mut().unwrap().insert(
            "gateway_fields".into(),
            json!({"server_client_ip":"203.0.113.10","server_asn":"AS1","server_country":"CN"}),
        );
    }
    ev
}

/// Brain consumes re_probe_priority: soft/thin elevates verification packs; GPU digs demoted.
#[test]
fn brain_reprobe_priority_elevates_verification_packs() {
    let fields = json!({
        "form_class": "desktop",
        "residual_soft_like": true,
        "soft_stack": true,
        "stack_class": "soft_render",
        "residual_mean": 0.500423,
        "hw_curve_webgl": audio_curve().1,
        "hw_curve_audio": audio_curve().0,
        "webgl_unmasked_renderer": "GeForce GTX 980",
        "spoof_score": 0.55,
    });
    let ev = json!({
        "sources": ["main"],
        "batches": [{"batch_id":"B0_bootstrap","source":"main"}],
        "fields": fields,
        "session_id": "reprobe_brain",
    });
    let plan = build_frontier(&ev, true, None, 24).expect("frontier");
    let notes = plan.notes.join("|");
    assert!(
        notes.contains("re_probe_priority") || notes.contains("soft_stack_budget"),
        "notes must mention re_probe or soft budget: {notes}"
    );
    let cov = &plan.coverage;
    assert!(
        cov.get("re_probe_priority_codes").is_some()
            || cov.get("open_verification_gaps").is_some(),
        "coverage must carry re_probe / open gaps: {cov}"
    );
    // Elevated verification packs present
    let packs = &plan.packs;
    let elevated: Vec<_> = packs
        .iter()
        .filter(|p| p.get("re_probe_elevated").and_then(|v| v.as_bool()) == Some(true))
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()))
        .collect();
    assert!(
        !elevated.is_empty()
            || packs.iter().any(|p| {
                matches!(
                    p.get("pack_id").and_then(|v| v.as_str()),
                    Some("B10_hw_curves") | Some("B11_interaction") | Some("B2_hardware")
                )
            }),
        "verification packs must be scheduled: elevated={elevated:?} packs={packs:?}"
    );
    // Soft GPU digs demoted if present
    for p in packs {
        if p.get("soft_gpu_demoted").and_then(|v| v.as_bool()) == Some(true) {
            let ep = p
                .get("effective_priority")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            assert!(ep < 5000, "soft GPU dig should be demoted: {p}");
        }
    }
}

/// Association ladder: rich→hardware, soft→env/profile, gateway→gateway; soft never hardware.
#[test]
fn association_ladder_levels_and_soft_not_hardware() {
    let rich = residual_rich();
    let soft = soft_fields("os_a", "unit_a");
    let thin = thin_gateway();

    let lad_rich = association_ladder(&rich, Some(&json!({"device_tier":"dh","collision_risk":false})));
    let lad_soft = association_ladder(&soft, Some(&json!({"device_tier":"dv","collision_risk":false})));
    let lad_soft_n = association_ladder(
        &{
            let mut s = soft_fields("os_a", "unit_a");
            s.as_object_mut().unwrap().remove("os_instance_hash");
            s
        },
        Some(&json!({"device_tier":"dv","collision_risk":true})),
    );
    let lad_thin = association_ladder(&thin, Some(&json!({"device_tier":"dg"})));

    assert_eq!(lad_rich["association_level"], "hardware");
    assert_ne!(lad_soft["association_level"], "hardware");
    assert!(
        matches!(
            lad_soft["association_level"].as_str(),
            Some("env") | Some("profile")
        ),
        "soft with os_instance → env/profile: {lad_soft}"
    );
    assert!(
        matches!(
            lad_soft_n["association_level"].as_str(),
            Some("profile") | Some("gateway") | Some("env")
        ),
        "soft no separator: {lad_soft_n}"
    );
    assert_eq!(lad_thin["association_level"], "gateway");
    assert_eq!(lad_soft["redlines"]["soft_never_hardware_level"], true);
    assert_eq!(lad_rich["redlines"]["os_br_rpa_not_device_key"], true);

    // evaluate product surfaces ladder + browser_surface
    let out = evaluate_session(
        &evidence(residual_rich(), &["main", "gateway"], true),
        None,
        None,
        None,
        true,
    )
    .expect("eval");
    let product = out.get("product").cloned().unwrap_or(out.clone());
    let device = out.get("device").cloned().unwrap_or(json!({}));
    assert!(
        product.get("association_level").is_some() || device.get("association_level").is_some(),
        "association_level on product path"
    );
    assert!(
        product.get("browser_surface_id").is_some() || device.get("browser_surface_id").is_some(),
        "browser_surface_id dual-track"
    );
    assert_eq!(
        product
            .pointer("/product_redlines/os_br_rpa_not_device_key")
            .or_else(|| product.pointer("/association_ladder/redlines/os_br_rpa_not_device_key"))
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    // Soft evaluate must not claim hardware
    let soft_out = evaluate_session(
        &evidence(soft_fields("os_x", "u_x"), &["main"], false),
        None,
        None,
        None,
        true,
    )
    .expect("soft eval");
    let sp = soft_out.get("product").cloned().unwrap_or(soft_out.clone());
    let level = sp
        .get("association_level")
        .or_else(|| soft_out.pointer("/device/association_level"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert_ne!(level, "hardware", "soft must not claim hardware: {sp}");
}

/// Task-critical conf: rich > thin; critical subset used.
#[test]
fn task_critical_confidence_thin_vs_rich() {
    assert!(!task_critical_fields("device_id").is_empty());
    assert!(!task_critical_fields("os").is_empty());
    let thin = evaluate_session(
        &evidence(thin_gateway(), &["gateway"], true),
        None,
        None,
        None,
        true,
    )
    .expect("thin");
    let rich = evaluate_session(
        &evidence(residual_rich(), &["main", "gateway"], true),
        None,
        None,
        None,
        true,
    )
    .expect("rich");
    let tp = thin.get("product").cloned().unwrap_or(thin.clone());
    let rp = rich.get("product").cloned().unwrap_or(rich.clone());
    let conf = |p: &Value, ax: &str| {
        p.pointer(&format!("/{ax}/confidence"))
            .or_else(|| p.pointer(&format!("/{ax}/confidence_report/confidence")))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
    };
    let thin_os = conf(&tp, "os");
    let rich_os = conf(&rp, "os");
    let thin_br = conf(&tp, "br");
    let rich_br = conf(&rp, "br");
    assert!(
        rich_os > thin_os + 0.02 || rich_br > thin_br + 0.02,
        "rich conf must exceed thin: thin_os={thin_os} rich_os={rich_os} thin_br={thin_br} rich_br={rich_br}"
    );
    // Task-critical coverage field present on rich
    let tcc = rp
        .pointer("/os/confidence_report/task_critical_coverage")
        .and_then(|v| v.as_f64());
    assert!(
        tcc.is_some_and(|x| x > 0.2),
        "task_critical_coverage should be meaningful on rich: {tcc:?}"
    );
    let thin_posture = tp
        .pointer("/os/confidence_posture")
        .and_then(|v| v.as_str())
        .unwrap_or("thin");
    assert!(
        thin_posture == "thin" || thin_os < 0.45,
        "thin must not look fully certain: {thin_posture} conf={thin_os}"
    );
}

/// Soft same unit different OS → two ids; L0 flip same residual → same id; collision_risk exposed.
#[test]
fn link_mint_soft_os_fork_and_l0_stable() {
    let a = soft_fields("os_vm0", "unit_same");
    let b = soft_fields("os_vm1", "unit_same");
    let pair = link_or_mint_pair(&a, &b);
    assert_ne!(
        pair["device_id_a"].as_str(),
        pair["device_id_b"].as_str(),
        "soft OS fork: {pair}"
    );
    let mut idx = MemoryDeviceIndex::new();
    let r1 = idx.link_or_mint(&a);
    let r2 = idx.link_or_mint(&b);
    assert_ne!(r1["device_id"], r2["device_id"]);

    let rich = residual_rich();
    let mut spoof = rich.clone();
    spoof.as_object_mut().unwrap().insert(
        "webgl_unmasked_renderer".into(),
        json!("Apple M1 spoof"),
    );
    let id_a = commercial_projection(&rich)
        .get("device_id")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();
    let id_b = commercial_projection(&spoof)
        .get("device_id")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();
    assert_eq!(id_a, id_b, "L0 must not split residual-led id");

    // Soft WITH os_instance separator → product collision_risk must be false
    // (align trust/evaluate with ServerMint + association env ladder).
    let soft_sep = soft_fields("os_with_sep", "unit_sep");
    let out_sep =
        evaluate_session(&evidence(soft_sep, &["main"], false), None, None, None, true)
            .expect("eval sep");
    let product_sep = out_sep.get("product").cloned().unwrap_or(out_sep.clone());
    let cr_sep = product_sep
        .get("collision_risk")
        .or_else(|| out_sep.pointer("/device/collision_risk"))
        .and_then(|v| v.as_bool());
    assert_eq!(
        cr_sep,
        Some(false),
        "soft+os_instance_hash must clear product collision_risk: {product_sep}"
    );
    let level_sep = product_sep
        .get("association_level")
        .or_else(|| out_sep.pointer("/device/association_level"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert_ne!(level_sep, "hardware", "soft still not hardware: {level_sep}");
    assert!(
        level_sep == "env" || level_sep == "profile",
        "soft+os_instance expected env/profile: {level_sep}"
    );

    // Soft without separator → collision_risk on product
    let mut soft_n = soft_fields("os_x", "u");
    soft_n.as_object_mut().unwrap().remove("os_instance_hash");
    let out = evaluate_session(&evidence(soft_n, &["main"], false), None, None, None, true)
        .expect("eval");
    let product = out.get("product").cloned().unwrap_or(out.clone());
    let cr = product
        .get("collision_risk")
        .or_else(|| out.pointer("/device/collision_risk"))
        .and_then(|v| v.as_bool());
    assert_eq!(cr, Some(true), "soft no separator collision_risk: {product}");
    assert!(
        product.get("uniqueness_marker").is_some()
            || out.pointer("/device/uniqueness_marker").is_some()
            || product.get("server_mint").is_some(),
        "mint/uniqueness markers on product path"
    );

    // Soft + webrtc (no os_instance) also clears collision_risk
    let mut soft_rtc = soft_fields("os_drop", "u_rtc");
    soft_rtc.as_object_mut().unwrap().remove("os_instance_hash");
    soft_rtc
        .as_object_mut()
        .unwrap()
        .insert("webrtc_host_ip_hash".into(), json!("lan_soft_sep"));
    let out_rtc =
        evaluate_session(&evidence(soft_rtc, &["main"], false), None, None, None, true)
            .expect("eval rtc");
    let product_rtc = out_rtc.get("product").cloned().unwrap_or(out_rtc.clone());
    assert_eq!(
        product_rtc
            .get("collision_risk")
            .or_else(|| out_rtc.pointer("/device/collision_risk"))
            .and_then(|v| v.as_bool()),
        Some(false),
        "soft+webrtc must clear collision_risk: {product_rtc}"
    );
}
