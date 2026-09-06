//! Prod 178: dead ANGLE residual hist → universal dh_*; demote + multi-material fork.

use gr_probe_core::{commercial_projection, is_commercial_device_id, is_multi_segment_id, select_device_tier};
use serde_json::json;

/// Near-uniform 32-bin hist matching prod da6e residual (std≈0.005).
fn dead_webgl_hist() -> Vec<f64> {
    vec![
        0.03418, 0.04199, 0.03027, 0.03906, 0.02637, 0.0293, 0.03223, 0.03027, 0.03516, 0.03125,
        0.02832, 0.02051, 0.03223, 0.02539, 0.0332, 0.03223, 0.0293, 0.02344, 0.02637, 0.02734,
        0.02734, 0.03418, 0.0332, 0.0332, 0.02637, 0.02734, 0.02832, 0.04004, 0.02637, 0.04102,
        0.03418, 0.04004,
    ]
}

fn audio_a() -> Vec<f64> {
    // Distinct spectral mass so coarse order-stats digests diverge.
    (0..64)
        .map(|i| {
            let x = i as f64;
            (x * 0.11 + 0.2).sin().abs() * 0.8 + 0.05 + if i < 16 { 0.4 } else { 0.0 }
        })
        .collect()
}
fn audio_b() -> Vec<f64> {
    (0..64)
        .map(|i| {
            let x = i as f64;
            (x * 0.41 + 3.9).cos().abs() * 0.15 + 0.02 + if i > 48 { 0.9 } else { 0.0 }
        })
        .collect()
}
fn canvas_a() -> Vec<f64> {
    (0..32).map(|i| ((i as f64 * 0.31 + 0.5).cos().abs() * 0.4 + 0.05)).collect()
}
fn canvas_b() -> Vec<f64> {
    (0..32).map(|i| ((i as f64 * 0.47 + 2.1).cos().abs() * 0.4 + 0.05)).collect()
}

fn base_fields(audio: Vec<f64>, canvas: Vec<f64>) -> serde_json::Value {
    json!({
        "form_class": "desktop",
        "platform": "Win32",
        "os_family": "windows",
        "hardware_concurrency": 12,
        "timezone": "Asia/Shanghai",
        "screen_width": 1920,
        "screen_height": 1080,
        "architecture": "x86_64",
        "hw_curve_webgl": dead_webgl_hist(),
        "hw_curve_audio": audio,
        "hw_curve_canvas": canvas,
        "webgl_unmasked_renderer": "ANGLE (NVIDIA, GeForce RTX 3060 Direct3D11)",
        "user_agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120.0.0.0",
        "server_client_ip": "203.0.113.50",
        "server_asn": "AS64500",
    })
}

fn gw_evidence() -> serde_json::Value {
    json!({
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "gateway_fields": {"server_client_ip": "203.0.113.50", "server_asn": "AS64500"},
        "batches": [
            {"batch_id":"B0_bootstrap","source":"main"},
            {"batch_id":"B10_hw_curves","source":"main"},
            {"batch_id":"B8_gateway","source":"gateway"}
        ],
    })
}

#[test]
fn dead_residual_flags_collision_and_not_dh() {
    let f = base_fields(audio_a(), canvas_a());
    let proj = commercial_projection(&f);
    assert_eq!(
        proj["webgl_residual_entropy_ok"].as_bool(),
        Some(false),
        "prod-like residual must be low-entropy: {proj}"
    );
    assert_eq!(
        proj["collision_risk"].as_bool(),
        Some(true),
        "dead residual without host sep → collision_risk: {proj}"
    );
    let included = proj["materials_included"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let has_audio = included.iter().any(|x| {
        matches!(x.as_str(), Some("hw_audio_stable") | Some("hw_audio_fine"))
    });
    assert!(
        has_audio,
        "must keep audio in digest when residual dead: {included:?}"
    );

    let tier = select_device_tier(&f, Some(&gw_evidence()));
    let id = tier["device_id"].as_str().unwrap_or("");
    // iss/51 P0-1: exclusive dh/dv demotion retired → multi-segment commercial form
    assert!(
        is_multi_segment_id(id) || is_commercial_device_id(id),
        "collision path still yields commercial multi-segment id, got {id} tier={}",
        tier["device_tier"]
    );
    assert_ne!(
        tier["device_tier"].as_str(),
        Some("dh"),
        "must not claim exclusive dh under collision_risk: {}",
        tier["device_tier"]
    );
    // Public body never embeds fixed names
    assert!(!id.contains("windows") && !id.contains("Win32") && !id.contains("NVIDIA"));
}


#[test]
fn dead_residual_different_coarse_audio_fork_ids() {
    // Coarse audio kept under dead residual → machines with distinct OfflineAudio fork.
    // Canvas intentionally NOT in commercial (multi-browser same-host contract).
    let a = base_fields(audio_a(), canvas_a());
    let b = base_fields(audio_b(), canvas_b());
    let pa = commercial_projection(&a);
    let pb = commercial_projection(&b);
    let ida = pa["device_id"].as_str().unwrap_or("");
    let idb = pb["device_id"].as_str().unwrap_or("");
    assert!(!ida.is_empty() && !idb.is_empty(), "both eligible: {pa} {pb}");
    assert_ne!(
        ida, idb,
        "distinct coarse audio must fork commercial ids under dead webgl residual"
    );
    // Same audio, different canvas → still same commercial id (canvas not hashed)
    let c = base_fields(audio_a(), canvas_b());
    let pc = commercial_projection(&c);
    assert_eq!(
        pa["device_id"], pc["device_id"],
        "canvas must not fork commercial id under multi-browser contract"
    );
}

#[test]
fn dead_residual_same_coarse_audio_micro_diff_same_id() {
    // Blink/Gecko OfflineAudio micro-diff collapses under coarse digest.
    let mut audio_blink = vec![0.0_f64; 64];
    let mut audio_gecko = vec![0.0_f64; 64];
    for i in 0..64 {
        let base = ((i as f64) * 0.17).sin() * 0.7;
        audio_blink[i] = base;
        audio_gecko[i] = base * 1.003 + 0.0002;
    }
    let a = base_fields(audio_blink, canvas_a());
    let b = base_fields(audio_gecko, canvas_b());
    let pa = commercial_projection(&a);
    let pb = commercial_projection(&b);
    assert_eq!(
        pa["device_id"], pb["device_id"],
        "same-host Blink/Gecko micro-audio must share commercial id"
    );
    assert_eq!(pa["collision_risk"], true);
}

#[test]
fn dead_residual_with_webrtc_host_can_be_dh() {
    // Dead residual + host separator → collision_risk cleared; webrtc enters digest.
    let mut f = base_fields(audio_a(), canvas_a());
    f.as_object_mut()
        .unwrap()
        .insert("webrtc_host_ip_hash".into(), json!("lan_test_host_sep_01"));
    let proj = commercial_projection(&f);
    assert_eq!(
        proj["webgl_residual_entropy_ok"].as_bool(),
        Some(false),
        "still low-entropy residual: {proj}"
    );
    assert_eq!(
        proj["collision_risk"].as_bool(),
        Some(false),
        "host sep must clear collision_risk: {proj}"
    );
    let included = proj["materials_included"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        included.iter().any(|x| x.as_str() == Some("webrtc_host_ip_hash")),
        "webrtc must be in commercial materials when residual dead: {included:?}"
    );
    // Distinct host sep → distinct commercial ids under same dead residual+audio
    let mut f2 = f.clone();
    f2.as_object_mut()
        .unwrap()
        .insert("webrtc_host_ip_hash".into(), json!("lan_test_host_sep_02"));
    let p2 = commercial_projection(&f2);
    assert_ne!(
        proj["device_id"], p2["device_id"],
        "host sep must fork commercial id under dead residual"
    );
    let tier = select_device_tier(&f, Some(&gw_evidence()));
    let id = tier["device_id"].as_str().unwrap_or("");
    // Multi-segment: commercial multi-segment, not exclusive dh_
    assert!(
        is_multi_segment_id(id) && is_commercial_device_id(id),
        "dead residual + webrtc host → multi-segment commercial, got {id} reasons={:?}",
        tier["tier_reasons"]
    );
    // Host/protocol context NEVER enters the commercial body (same policy as
    // gateway_protocol_host_context_flagged_but_not_in_body): oi/rtc slots stay
    // placeholder. The host separator lives in the layered instance id instead
    // (iss/opus5 01-P0-1), which is what forks per-machine identity.
    let body = id.strip_prefix("dv0-").unwrap_or(id);
    let parts: Vec<&str> = body.split('-').collect();
    assert!(parts.len() >= 10, "10-part multi-segment body: {id}");
    assert_eq!(parts[8], "0", "oi slot must stay out of commercial body: {id}");
    assert_eq!(parts[9], "0", "rtc slot must stay out of commercial body: {id}");
    assert_eq!(
        tier["has_host_separator"].as_bool(),
        Some(true),
        "host separator still observed for ops/association: {tier}"
    );
    assert_eq!(
        tier["device_instance_issued"].as_bool(),
        Some(true),
        "layered instance id issued from webrtc host pin: {tier}"
    );
    assert_eq!(
        tier["device_instance_separator"].as_str(),
        Some("webrtc_host_hash"),
        "instance separator kind: {tier}"
    );
    let notes = tier["curve_selection_notes"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        notes
            .iter()
            .any(|n| n.as_str() == Some("rtc_from_webrtc_host_hash")),
        "host context recorded for ops in curve_selection_notes: {notes:?}"
    );
}


#[test]
fn healthy_residual_still_can_be_dh() {
    // High-entropy v3f-like residual + dual silicon anchors + host sep → dh.
    // Audio is KEPT in digest when residual_ok (no longer dropped).
    let curve: Vec<f64> = {
        let mut c = Vec::with_capacity(32);
        for s in 0..4 {
            let k = 0.7 + s as f64 * 0.41;
            for t in 0..6 {
                let v = (k * 2.3 + t as f64 * 0.7).sin().abs() * 0.18
                    + (k * 1.1 + t as f64).cos().abs() * 0.06
                    + t as f64 * 0.008;
                c.push((v * 1e5).round() / 1e5);
            }
            let gm = (k).sin().abs() * 0.4 + 0.1;
            let gs = (k * 1.3).cos().abs() * 0.12 + 0.04;
            c.push((gm * 1e5).round() / 1e5);
            c.push((gs * 1e5).round() / 1e5);
        }
        c
    };
    let f = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "os_family": "linux",
        "hardware_concurrency": 12,
        "device_memory": 16,
        "timezone": "Asia/Shanghai",
        "screen_width": 1920,
        "screen_height": 1080,
        "hw_curve_audio": (0..64).map(|i| ((i as f64 * 0.17 + 0.1).sin().abs() * 0.4 + 0.05)).collect::<Vec<_>>(),
        "hw_curve_webgl": curve,
        "webgl_unmasked_renderer": "ANGLE (NVIDIA, GeForce GTX 1660)",
        "webgl_unmasked_vendor": "Google Inc. (NVIDIA)",
        "user_agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/120.0.0.0",
        "gl_precision_matrix": [[1,1,23],[1,1,23],[1,1,23],[1,1,23],[1,1,23],[1,1,23],[1,1,23],[1,1,23],[1,1,23],[1,1,23],[1,1,23],[1,1,23]],
        "cpu_timing_curve": (0..24).map(|i| ((i as f64 * 0.17 + 0.5).sin().abs() * 0.4 + 0.05)).collect::<Vec<_>>(),
        "webrtc_host_ip_hash": "lan_abc123def456",
        "server_client_ip": "203.0.113.50",
        "server_asn": "AS64500",
        "residual_mean": 0.123456,
        "residual_algo": "gr_webgl_residual_std_v3f",
    });
    let proj = commercial_projection(&f);
    assert_eq!(proj["webgl_residual_entropy_ok"].as_bool(), Some(true), "{proj}");
    assert_eq!(proj["collision_risk"].as_bool(), Some(false), "{proj}");
    let included = proj["materials_included"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        included.iter().any(|x| x.as_str() == Some("hw_audio_stable")),
        "healthy residual must KEEP audio in digest: {included:?}"
    );
    // Dual silicon + residual_entropy_ok → host seps are conf-only (not commercial digest).
    // webrtc still counts as host_sep for dh tier promotion via fields_have_host_sep.
    assert!(
        !included
            .iter()
            .any(|x| x.as_str() == Some("webrtc_host_ip_hash")),
        "webrtc must be demoted from commercial digest under dual silicon residual_ok: {included:?}"
    );
    let tier = select_device_tier(&f, Some(&gw_evidence()));
    let id = tier["device_id"].as_str().unwrap_or("");
    assert!(
        is_multi_segment_id(id) && is_commercial_device_id(id),
        "healthy multi-dim → multi-segment commercial (not exclusive dh_), got {id} reasons={:?}",
        tier["tier_reasons"]
    );
    assert!(
        id.starts_with("dv0-") || id.starts_with("dv4-") || id.starts_with("dv5-") || id.starts_with("dv6-"),
        "precision-lane prefix expected: {id}"
    );
    assert!(!id.contains("NVIDIA") && !id.contains("linux"));
}


#[test]
fn empty_webgl_curve_is_not_entropy_ok() {
    let f = json!({
        "form_class": "desktop",
        "platform": "Win32",
        "os_family": "windows",
        "hardware_concurrency": 8,
        "timezone": "UTC",
        "screen_width": 1920,
        "screen_height": 1080,
        "hw_curve_audio": audio_a(),
        // no hw_curve_webgl
        "user_agent": "Mozilla/5.0 Chrome/120",
    });
    let proj = commercial_projection(&f);
    assert_eq!(
        proj["webgl_residual_entropy_ok"].as_bool(),
        Some(false),
        "missing residual must not default true: {proj}"
    );
    let included = proj["materials_included"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        included.iter().any(|x| x.as_str() == Some("hw_audio_stable")),
        "audio remains commercial when residual missing: {included:?}"
    );
}

#[test]
fn healthy_residual_dual_audio_or_webrtc_forks_hosts() {
    // Shared healthy residual shape (v3f-like strip stds) + different audio → distinct ids.
    // Uniqueness from materials — no force demote.
    let curve: Vec<f64> = {
        let mut c = Vec::with_capacity(32);
        for s in 0..4 {
            let k = 0.5 + s as f64 * 0.41;
            for t in 0..6 {
                let v = (k * 2.3 + t as f64 * 0.7).sin().abs() * 0.18
                    + (k * 1.1 + t as f64).cos().abs() * 0.06
                    + t as f64 * 0.008;
                c.push((v * 1e5).round() / 1e5);
            }
            let gm = (k).sin().abs() * 0.4 + 0.1;
            let gs = (k * 1.3).cos().abs() * 0.12 + 0.04;
            c.push((gm * 1e5).round() / 1e5);
            c.push((gs * 1e5).round() / 1e5);
        }
        c
    };
    let mut host_a = base_fields(audio_a(), canvas_a());
    host_a
        .as_object_mut()
        .unwrap()
        .insert("hw_curve_webgl".into(), json!(curve.clone()));
    host_a
        .as_object_mut()
        .unwrap()
        .insert("residual_mean".into(), json!(0.211));
    let mut host_b = base_fields(audio_b(), canvas_b());
    host_b
        .as_object_mut()
        .unwrap()
        .insert("hw_curve_webgl".into(), json!(curve));
    host_b
        .as_object_mut()
        .unwrap()
        .insert("residual_mean".into(), json!(0.211));
    let pa = commercial_projection(&host_a);
    let pb = commercial_projection(&host_b);
    assert_eq!(pa["webgl_residual_entropy_ok"].as_bool(), Some(true), "{pa}");
    assert_eq!(pb["webgl_residual_entropy_ok"].as_bool(), Some(true), "{pb}");
    // Both keep dual anchors
    for p in [&pa, &pb] {
        let inc = p["materials_included"].as_array().cloned().unwrap_or_default();
        assert!(
            inc.iter().any(|x| x.as_str() == Some("hw_webgl_stable")),
            "webgl in digest: {inc:?}"
        );
        assert!(
            inc.iter().any(|x| x.as_str() == Some("hw_audio_stable")),
            "audio kept when residual_ok: {inc:?}"
        );
    }
    assert_ne!(
        pa["device_id"], pb["device_id"],
        "different coarse audio must fork commercial id under shared residual curve"
    );

    // Same audio, different webrtc host sep → commercial body **same** under dual silicon
    // residual_ok (host seps conf-only). Multi-host fork is audio/webgl, not engine-noisy LAN.
    let mut h1 = host_a.clone();
    h1.as_object_mut()
        .unwrap()
        .insert("webrtc_host_ip_hash".into(), json!("host_sep_aaa"));
    let mut h2 = host_a.clone();
    h2.as_object_mut()
        .unwrap()
        .insert("webrtc_host_ip_hash".into(), json!("host_sep_bbb"));
    let p1 = commercial_projection(&h1);
    let p2 = commercial_projection(&h2);
    assert_eq!(
        p1["device_id"], p2["device_id"],
        "webrtc-only must not fork dual-silicon commercial id when residual_ok"
    );
    assert!(
        !p1["materials_included"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x.as_str() == Some("webrtc_host_ip_hash")),
        "webrtc conf-only under dual silicon residual_ok"
    );
}

#[test]
fn residual_mean_derived_for_server_mint() {
    use gr_probe_core::apply_server_mint;
    let curve: Vec<f64> = (0..32)
        .map(|i| ((i as f64 * 0.21 + 0.7).sin().abs() * 0.5 + 0.03))
        .collect();
    // No residual_mean field — mint must still get residual_class from curve mean
    let f = json!({
        "form_class": "desktop",
        "platform": "Win32",
        "os_family": "windows",
        "hardware_concurrency": 8,
        "timezone": "UTC",
        "screen_width": 1920,
        "screen_height": 1080,
        "architecture": "x86_64",
        "hw_curve_webgl": curve,
        "hw_curve_audio": audio_a(),
        "user_agent": "Mozilla/5.0 Chrome/120",
    });
    let mint = apply_server_mint(&f);
    let rc = mint["residual_class"].as_str().unwrap_or("");
    assert!(
        rc.starts_with("rm_") && rc != "rm_none",
        "ServerMint residual_class must derive from curve: {mint}"
    );
    assert!(!mint["device_id"].as_str().unwrap_or("").is_empty());
}
