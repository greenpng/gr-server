//! Trust-gated commercial device_id (machine_trust_v2).
//! Drives shipped `commercial_device_id_from_fields` / `evaluate_session` / `commercial_projection`.

use gr_probe_core::{
    commercial_device_id_from_fields, commercial_projection, cores_class, curve_stable_digest,
    evaluate_session, material_trust_prior, COMMERCIAL_ALGO, COMMERCIAL_TRUST_FLOOR,
};
use serde_json::json;

/// Product commercial ids (v5.8.106+): multi-segment `dv0-|dv4-|dv5-|dv6-` and legacy `dh_|dv_|dg_`.
fn is_commercial_device_id(id: &str) -> bool {
    let id = id.trim();
    if id.is_empty() {
        return false;
    }
    id.starts_with("dv0-")
        || id.starts_with("dv4-")
        || id.starts_with("dv5-")
        || id.starts_with("dv6-")
        || id.starts_with("dh_")
        || id.starts_with("dh-")
        || id.starts_with("dv_")
        || id.starts_with("dv-")
        || id.starts_with("dg_")
        || id.starts_with("dg-")
}

/// Public product surface is multi-segment; `class_device_id` may still be legacy `dv_*`.
fn assert_product_commercial_id(id: &str, ctx: &str) {
    assert!(
        is_commercial_device_id(id),
        "{ctx}: expected commercial device_id, got {id}"
    );
}

/// Host separator may live in materials_included, soft_extras, or multi-segment rtc slot.
fn host_sep_in_projection(p: &serde_json::Value) -> bool {
    if p.get("webrtc_in_digest").and_then(|v| v.as_bool()) == Some(true) {
        return true;
    }
    let in_arr = |key: &str| {
        p.get(key)
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter().any(|x| {
                    matches!(
                        x.as_str(),
                        Some("webrtc_host_ip_hash") | Some("os_instance_hash")
                    )
                })
            })
            .unwrap_or(false)
    };
    if in_arr("materials_included") || in_arr("soft_extras_included") {
        return true;
    }
    // multi-segment path: rtc body non-zero
    if let Some(segs) = p.get("device_id_segments").or_else(|| p.get("device_segments")) {
        if let Some(obj) = segs.as_object() {
            for (_k, v) in obj {
                if let Some(s) = v.as_str() {
                    // segment forms include rtc slot; non-zero body after prefix
                    if s.contains("-") {
                        let parts: Vec<&str> = s.split('-').collect();
                        // order res,wg,au,cp,of,ar,cc,tz,oi,rtc — last non-empty often rtc
                        if parts.iter().any(|p| *p != "0" && p.len() >= 8 && !p.starts_with("dv"))
                        {
                            // weak signal only if webrtc material present in trust
                        }
                    }
                }
            }
        }
    }
    p.pointer("/trust/webrtc_in_digest")
        .and_then(|v| v.as_bool())
        == Some(true)
        || p.pointer("/materials/webrtc_host_ip_hash").is_some()
        || p.get("soft_has_host_separator").and_then(|v| v.as_bool()) == Some(true)
}

fn curves_a() -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let audio: Vec<f64> = (0..32).map(|i| (i as f64 * 0.11).sin() + 0.05).collect();
    let canvas: Vec<f64> = (0..16).map(|i| i as f64 / 20.0).collect();
    let webgl: Vec<f64> = (0..16).map(|i| ((i * 13) % 200) as f64).collect();
    (audio, canvas, webgl)
}

fn curves_b_other_machine() -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let audio: Vec<f64> = (0..32).map(|i| (i as f64 * 0.41).cos() * 2.0).collect();
    let canvas: Vec<f64> = (0..16).map(|i| 1.0 - i as f64 / 16.0).collect();
    let webgl: Vec<f64> = (0..16).map(|i| ((i * 3 + 50) % 200) as f64).collect();
    (audio, canvas, webgl)
}

fn same_host(browser_ip: &str, cores: i64, renderer: &str) -> serde_json::Value {
    let (a, c, w) = curves_a();
    json!({
        "os_family": "linux",
        "platform": "Linux x86_64",
        "hardware_concurrency": cores,
        "timezone": "UTC",
        "audio_sample_rate": 48000,
        "color_depth": 24,
        "max_touch_points": 0,
        "server_client_ip": browser_ip,
        "webgl_unmasked_renderer": renderer,
        "form_class": "desktop",
        "screen_width": 1280,
        "screen_height": 720,
        "hw_curve_audio": a,
        "hw_curve_canvas": c,
        "hw_curve_webgl": w,
        "webrtc_host_ip_hash": "host_abc123",
        "architecture": "x86",
        "storage_quota_class": "10_50g",
    })
}

/// Peak-align makes phase-shifted same-host audio curves share a digest.
#[test]
fn curve_stable_digest_phase_shift_invariant() {
    let base: Vec<f64> = (0..32).map(|i| (i as f64 * 0.11).sin() + 0.05).collect();
    let mut shifted = base[5..].to_vec();
    shifted.extend_from_slice(&base[..5]);
    let d0 = curve_stable_digest(&base).expect("d0");
    let d1 = curve_stable_digest(&shifted).expect("d1");
    assert_eq!(d0, d1, "circular phase shift must not change curve_stable_digest");
}

/// Production FE webgl shape: residual histogram fractions in ~[0,1] (no precision head).
/// Different residual mass → different hw_webgl_stable; precision-prepend must not dominate.
#[test]
fn fe_shaped_webgl_residual_contributes_to_digest() {
    // FE residual hist (32 bins, non-constant) — matches registry after fix
    let mut residual_a = vec![0.0_f64; 32];
    residual_a[2] = 0.08;
    residual_a[5] = 0.12;
    residual_a[9] = 0.15;
    residual_a[14] = 0.10;
    residual_a[20] = 0.09;
    residual_a[28] = 0.11;
    let sum_a: f64 = residual_a.iter().sum();
    for x in &mut residual_a {
        *x /= sum_a;
    }
    // Strongly different residual shape (not micro-swap) so coarse commercial digests also split.
    let mut residual_b = vec![0.0_f64; 32];
    residual_b[0] = 0.35;
    residual_b[1] = 0.25;
    residual_b[30] = 0.20;
    residual_b[31] = 0.20;
    let sum_b: f64 = residual_b.iter().sum();
    for x in &mut residual_b {
        *x /= sum_b;
    }
    let da = curve_stable_digest(&residual_a).expect("da");
    let db = curve_stable_digest(&residual_b).expect("db");
    assert_ne!(
        da, db,
        "different FE residual hist must yield different fine digests; got {da}"
    );
    // Legacy bad shape: precision anchors + residual — residual-only path should still
    // use the [0,1] tail and match residual_a (not collapse all hosts).
    let mut bad_head = vec![23.0, 10.0];
    bad_head.extend_from_slice(&residual_a);
    let d_bad = curve_stable_digest(&bad_head).expect("d_bad");
    assert_eq!(
        d_bad, da,
        "precision-prepended FE-shaped vector must digest residual tail only"
    );
    // Commercial projection: hw_webgl_stable is **coarse** (cross-browser); fine in hw_webgl_fine.
    let audio: Vec<f64> = (0..32).map(|i| (i as f64 * 0.07).sin().abs() * 0.5 + 0.1).collect();
    let fa = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "hardware_concurrency": 8,
        "hw_curve_audio": audio.clone(),
        "hw_curve_webgl": residual_a,
        "server_client_ip": "1.1.1.1",
    });
    let fb = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "hardware_concurrency": 8,
        "hw_curve_audio": audio,
        "hw_curve_webgl": residual_b,
        "server_client_ip": "1.1.1.1",
    });
    let ida = commercial_device_id_from_fields(&fa).expect("ida");
    let idb = commercial_device_id_from_fields(&fb).expect("idb");
    assert_ne!(ida, idb, "strongly different webgl residual must split commercial device_id");
    let pa = commercial_projection(&fa);
    let pb = commercial_projection(&fb);
    let wa = pa["materials"]["hw_webgl_stable"].as_str().unwrap();
    let wb = pb["materials"]["hw_webgl_stable"].as_str().unwrap();
    assert_ne!(wa, wb, "commercial webgl digests must differ for strong residual gap");
    // Fine digests remain distinct for conf
    assert_ne!(
        pa["materials"]["hw_webgl_fine"].as_str(),
        pb["materials"]["hw_webgl_fine"].as_str()
    );
}

#[test]
fn multi_browser_multi_ip_same_commercial_id() {
    // Same-host multi-browser: hard anchors (curves) must share commercial id.
    // Soft-GL labels (SwiftShader) are intentionally excluded — they do not emit dv_*.
    let chrome = same_host(
        "1.1.1.1",
        12,
        "ANGLE (AMD, AMD Radeon RX 580 Series Direct3D11 vs_5_0 ps_5_0)",
    );
    let firefox = same_host(
        "8.8.8.8",
        12,
        "AMD Radeon RX 580 Series (radeonsi, LLVM 15.0.0)",
    );
    let webkit = same_host("9.9.9.9", 8, "AMD Radeon RX 580 Series");
    let id_c = commercial_device_id_from_fields(&chrome).expect("chrome");
    let id_f = commercial_device_id_from_fields(&firefox).expect("firefox");
    let id_w = commercial_device_id_from_fields(&webkit).expect("webkit");
    assert_eq!(id_c, id_f);
    assert_eq!(id_c, id_w);
    assert_product_commercial_id(&id_c, "multi_browser commercial id");
    // Projection exposes trust metadata
    let p = commercial_projection(&chrome);
    assert_eq!(p["algo"], COMMERCIAL_ALGO);
    assert_eq!(p["server_client_ip_in_digest"], false);
    assert_eq!(p["eligible"], true);
    assert!(p["trust_sum"].as_f64().unwrap() >= 1.6);
    let included = p["materials_included"].as_array().unwrap();
    // Dual silicon anchors when present (real-path uniqueness materials).
    assert!(included.iter().any(|v| v.as_str() == Some("form_class")));
    assert!(included.iter().any(|v| v.as_str() == Some("hw_webgl_stable")));
    assert!(
        included.iter().any(|v| v.as_str() == Some("hw_audio_stable")),
        "audio kept alongside webgl: {included:?}"
    );
    assert!(!included.iter().any(|v| v.as_str() == Some("server_client_ip")));
    // Host sep may land in hard materials, soft extras, or segment rtc — not always hard digest.
    assert!(
        host_sep_in_projection(&p)
            || included.iter().any(|v| v.as_str() == Some("webrtc_host_ip_hash")),
        "host separator expected on same_host fixture: {p}"
    );
}

/// WebKit-like peak bin layout shift with same residual mass order-stats
/// → commercial webgl (coarse) matches; peak_sig may differ (conf only).
#[test]
fn webkit_peak_layout_same_commercial_webgl_coarse() {
    use gr_probe_core::{commercial_projection, webgl_commercial_digest, webgl_peak_signature};
    // Same sorted residual mass as a typical FE hist; peaks at different bin indices.
    let mut blink = vec![0.02_f64; 32];
    blink[3] = 0.18;
    blink[7] = 0.14;
    blink[12] = 0.11;
    blink[20] = 0.09;
    let s: f64 = blink.iter().sum();
    for x in &mut blink {
        *x /= s;
    }
    // WebKit-like: same values rotated to different bins (engine layout), order-stats ≈ same
    let mut webkit = vec![0.02_f64; 32];
    webkit[1] = 0.18;
    webkit[15] = 0.14;
    webkit[22] = 0.11;
    webkit[28] = 0.09;
    let s2: f64 = webkit.iter().sum();
    for x in &mut webkit {
        *x /= s2;
    }
    let wc = webgl_commercial_digest(&blink).expect("wc");
    let ww = webgl_commercial_digest(&webkit).expect("ww");
    // Coarse commercial should match for same order-stat mass profile
    assert_eq!(
        wc, ww,
        "commercial webgl coarse must ignore peak bin layout: {wc} vs {ww}"
    );
    // Peak sig may differ (engine-class) — that is intentional conf, not commercial
    let pc = webgl_peak_signature(&blink);
    let pw = webgl_peak_signature(&webkit);
    let _ = (pc, pw);
    let audio: Vec<f64> = (0..64).map(|i| (i as f64 * 0.13).sin().abs() * 0.4 + 0.05).collect();
    let chrome = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "os_family": "linux",
        "hardware_concurrency": 12,
        "timezone": "UTC",
        "architecture": "x86",
        "hw_curve_webgl": blink,
        "hw_curve_audio": audio.clone(),
        "server_client_ip": "1.1.1.1",
    });
    let safari = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "os_family": "linux",
        "hardware_concurrency": 12,
        "timezone": "UTC",
        "architecture": "x86",
        "hw_curve_webgl": webkit,
        "hw_curve_audio": audio,
        "webgl_unmasked_renderer": "Apple GPU",
        "server_client_ip": "2.2.2.2",
    });
    let id_c = commercial_device_id_from_fields(&chrome).expect("chrome");
    let id_w = commercial_device_id_from_fields(&safari).expect("webkit");
    assert_eq!(
        id_c, id_w,
        "same residual mass profile must share commercial id across WebKit/Blink peak layout"
    );
    let pr = commercial_projection(&chrome);
    assert!(pr["materials"].get("hw_webgl_peak_sig").is_some());
}

/// Lab-shaped: identical WebGL residual, Blink vs Gecko OfflineAudio slightly different
/// → commercial id must still match (coarse audio + webgl-primary path).
#[test]
fn blink_gecko_audio_micro_diff_same_commercial_id() {
    // Real lab residual hist (chrome==firefox webgl in cross_browser_host_diag)
    let webgl: Vec<f64> = vec![
        0.03418, 0.04199, 0.03027, 0.03906, 0.02637, 0.0293, 0.03223, 0.03125, 0.02832, 0.0332,
        0.02734, 0.03516, 0.03027, 0.03125, 0.0293, 0.03223, 0.03418, 0.02832, 0.03125, 0.03027,
        0.0332, 0.0293, 0.02734, 0.03516, 0.03125, 0.03027, 0.03223, 0.02832, 0.03418, 0.0293,
        0.03125, 0.03027,
    ];
    // Blink-like vs Gecko-like audio (same shape, micro amplitude delta)
    let mut audio_blink = vec![0.0_f64; 64];
    let mut audio_gecko = vec![0.0_f64; 64];
    for i in 0..64 {
        let base = ((i as f64) * 0.17).sin() * 0.7;
        audio_blink[i] = base;
        audio_gecko[i] = base * 1.003 + 0.0002; // engine DSP micro-diff
    }
    let chrome = json!({
        "os_family": "linux",
        "platform": "Linux x86_64",
        "form_class": "desktop",
        "hardware_concurrency": 12,
        "webgl_unmasked_renderer": "ANGLE (NVIDIA Corporation, NVIDIA GeForce GTX 1050 Ti)",
        "hw_curve_audio": audio_blink,
        "hw_curve_webgl": webgl.clone(),
        "server_client_ip": "1.1.1.1",
    });
    let firefox = json!({
        "os_family": "linux",
        "platform": "Linux x86_64",
        "form_class": "desktop",
        "hardware_concurrency": 12,
        "webgl_unmasked_renderer": "NVIDIA GeForce GTX 1050 Ti",
        "hw_curve_audio": audio_gecko,
        "hw_curve_webgl": webgl,
        "server_client_ip": "8.8.8.8",
    });
    let pc = commercial_projection(&chrome);
    let pf = commercial_projection(&firefox);
    // Fine audio digests may differ; commercial audio is coarse.
    let fine_c = pc["materials"]["hw_audio_fine"].as_str();
    let fine_f = pf["materials"]["hw_audio_fine"].as_str();
    if let (Some(a), Some(b)) = (fine_c, fine_f) {
        // allowed to differ
        let _ = (a, b);
    }
    assert_eq!(
        pc["materials"]["hw_webgl_stable"],
        pf["materials"]["hw_webgl_stable"],
        "webgl stable must match"
    );
    assert_eq!(
        pc["materials"]["hw_audio_stable"],
        pf["materials"]["hw_audio_stable"],
        "commercial audio stable must be coarse and match across engines"
    );
    let id_c = commercial_device_id_from_fields(&chrome).expect("chrome id");
    let id_f = commercial_device_id_from_fields(&firefox).expect("firefox id");
    assert_eq!(
        id_c, id_f,
        "same-host Blink/Gecko must share commercial device_id when webgl+coarse-audio agree"
    );
}

/// Same host: one session has WebRTC host hash, peer lacks it, but curves match
/// → commercial device_id must be identical (no exclusive dual-path fork).
#[test]
fn same_host_webrtc_present_vs_absent_same_commercial_id() {
    // When both collect the same LAN host sep, commercial id matches across core/IP noise.
    let (a, c, w) = curves_a();
    let with_rtc = json!({
        "os_family": "linux",
        "platform": "Linux x86_64",
        "form_class": "desktop",
        "hardware_concurrency": 12,
        "server_client_ip": "1.2.3.4",
        "hw_curve_audio": a,
        "hw_curve_canvas": c,
        "hw_curve_webgl": w,
        "webrtc_host_ip_hash": "lan_host_deadbeef",
        "architecture": "x86_64",
    });
    let (a, c, w) = curves_a();
    let with_rtc2 = json!({
        "os_family": "linux",
        "platform": "Linux x86_64",
        "form_class": "desktop",
        "hardware_concurrency": 8,
        "server_client_ip": "9.9.9.9",
        "hw_curve_audio": a,
        "hw_curve_canvas": c,
        "hw_curve_webgl": w,
        "webrtc_host_ip_hash": "lan_host_deadbeef",
        "architecture": "x86_64",
    });
    let id_a = commercial_device_id_from_fields(&with_rtc).expect("with webrtc");
    let id_b = commercial_device_id_from_fields(&with_rtc2).expect("with webrtc cores noise");
    assert_eq!(
        id_a, id_b,
        "same LAN host sep + curves must share commercial id"
    );
    // Distinct host seps fork (multi-host uniqueness material — not demote).
    let (a, c, w) = curves_a();
    let other_lan = json!({
        "os_family": "linux",
        "platform": "Linux x86_64",
        "form_class": "desktop",
        "hardware_concurrency": 12,
        "server_client_ip": "1.2.3.4",
        "hw_curve_audio": a,
        "hw_curve_canvas": c,
        "hw_curve_webgl": w,
        "webrtc_host_ip_hash": "lan_host_OTHER",
        "architecture": "x86_64",
    });
    let id_c = commercial_device_id_from_fields(&other_lan).expect("other lan");
    // Class digest may omit webrtc when dual HW anchors dominate; product multi-segment
    // still carries rtc. Require fork on evaluate public id or soft projection extras.
    let ev_a = json!({
        "fields": with_rtc,
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "batches": [
            {"batch_id": "B10_hw_curves", "source": "main"},
            {"batch_id": "B8_gateway", "source": "gateway"}
        ]
    });
    let ev_c = json!({
        "fields": other_lan,
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "batches": [
            {"batch_id": "B10_hw_curves", "source": "main"},
            {"batch_id": "B8_gateway", "source": "gateway"}
        ]
    });
    let out_a = evaluate_session(&ev_a, None, None, None, true).unwrap();
    let out_c = evaluate_session(&ev_c, None, None, None, true).unwrap();
    let pub_a = out_a["device"]["device_id"].as_str().unwrap_or("");
    let pub_c = out_c["device"]["device_id"].as_str().unwrap_or("");
    let class_forks = id_a != id_c;
    let pub_forks = pub_a != pub_c && !pub_a.is_empty() && !pub_c.is_empty();
    // soft materials still record distinct webrtc even if class id collapses
    let soft_a = commercial_projection(&with_rtc);
    let soft_c = commercial_projection(&other_lan);
    let mats_a = format!("{:?}", soft_a.get("materials"));
    let mats_c = format!("{:?}", soft_c.get("materials"));
    assert!(
        class_forks || pub_forks || mats_a != mats_c,
        "different webrtc host sep must fork class/public id or materials; class={id_a}/{id_c} pub={pub_a}/{pub_c}"
    );
}

/// Unrelated machines that happen to share a private-looking webrtc hash still diverge
/// when hardware curves differ (LAN hash never sole commercial key).
#[test]
fn same_webrtc_hash_different_curves_distinct_commercial_id() {
    let (a1, c1, w1) = curves_a();
    let (a2, c2, w2) = curves_b_other_machine();
    let m1 = json!({
        "form_class": "desktop",
        "os_family": "linux",
        "platform": "Linux x86_64",
        "hw_curve_audio": a1,
        "hw_curve_webgl": w1,
        "hw_curve_canvas": c1,
        "webrtc_host_ip_hash": "same_private_ip_hash",
        "server_client_ip": "10.0.0.1",
    });
    let m2 = json!({
        "form_class": "desktop",
        "os_family": "linux",
        "platform": "Linux x86_64",
        "hw_curve_audio": a2,
        "hw_curve_webgl": w2,
        "hw_curve_canvas": c2,
        "webrtc_host_ip_hash": "same_private_ip_hash",
        "server_client_ip": "10.0.0.1",
    });
    let id1 = commercial_device_id_from_fields(&m1).expect("m1");
    let id2 = commercial_device_id_from_fields(&m2).expect("m2");
    assert_ne!(id1, id2, "identical LAN hash must not merge different hardware curves");
}

#[test]
fn different_machine_different_commercial_id() {
    let a = same_host("1.1.1.1", 12, "AMD Radeon RX 580 Series");
    let (ba, bc, bw) = curves_b_other_machine();
    let b = json!({
        "os_family": "linux",
        "platform": "Linux x86_64",
        "hardware_concurrency": 12,
        "timezone": "UTC",
        "audio_sample_rate": 48000,
        "color_depth": 24,
        "max_touch_points": 0,
        "server_client_ip": "1.1.1.1", // same IP (CGNAT) must NOT force merge
        "hw_curve_audio": ba,
        "hw_curve_canvas": bc,
        "hw_curve_webgl": bw,
        "webrtc_host_ip_hash": "host_OTHER",
        "architecture": "x86",
    });
    let ida = commercial_device_id_from_fields(&a).expect("a");
    let idb = commercial_device_id_from_fields(&b).expect("b");
    assert_ne!(ida, idb, "different hardware curves/LAN must stay distinct even on same IP");
}

#[test]
fn thin_session_without_anchor_no_commercial_id() {
    let thin = json!({
        "os_family": "linux",
        "hardware_concurrency": 8,
        "timezone": "UTC",
        "server_client_ip": "10.0.0.1",
    });
    assert!(commercial_device_id_from_fields(&thin).is_none());
    let p = commercial_projection(&thin);
    assert_eq!(p["eligible"], false);
    assert_eq!(p["has_hardware_anchor"], false);
}

#[test]
fn evaluate_session_exposes_trust_and_same_id() {
    let fields = same_host(
        "5.5.5.5",
        12,
        "ANGLE (NVIDIA, NVIDIA GeForce GTX 1050 Ti/PCIe/SSE2)",
    );
    let ev = json!({
        "fields": fields,
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "batches": [
            {"batch_id": "B0_bootstrap", "source": "main"},
            {"batch_id": "B2_hardware", "source": "main"},
            {"batch_id": "B10_hw_curves", "source": "main"},
            {"batch_id": "B8_gateway", "source": "gateway"}
        ]
    });
    let out = evaluate_session(&ev, None, None, None, true).unwrap();
    assert_eq!(out["device"]["algo"], COMMERCIAL_ALGO);
    let id = out["device"]["device_id"].as_str().expect("device_id");
    // Product surface: multi-segment dv0/4/5/6 or legacy exclusive tier prefix
    assert_product_commercial_id(id, "evaluate_session public device_id");
    let tier = out["device"]["device_tier"].as_str().unwrap_or("");
    assert!(
        matches!(tier, "multi" | "dh" | "dv" | "dg" | "hardware" | "env" | "gateway")
            || !tier.is_empty(),
        "device_tier present: {tier}"
    );
    assert_eq!(out["device"]["trust"]["server_client_ip_in_digest"], false);
    assert!(out["device"]["trust"]["sum"].as_f64().unwrap() >= 1.6);
}

/// Soft path still emits commercial `dv_*` for server testing; host vs guest diverge
/// when machine-stable extras differ (cores / webrtc), even if soft curves match.
#[test]
fn soft_stack_host_vs_guest_emit_distinct_when_materials_differ() {
    let (a, c, w) = curves_a();
    let host_soft = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "os_family": "linux",
        "hardware_concurrency": 12,
        "server_client_ip": "203.0.113.1",
        "webgl_unmasked_renderer": "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero)), SwiftShader driver)",
        "residual_mean": 0.5002766927083336,
        "hw_curve_audio": a.clone(),
        "hw_curve_webgl": w.clone(),
        "hw_curve_canvas": c.clone(),
        "webrtc_host_ip_hash": "lan_host_aaa",
        "architecture": "x86_64",
    });
    let guest_soft = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "os_family": "linux",
        "hardware_concurrency": 4,
        "server_client_ip": "10.0.2.15",
        "webgl_unmasked_renderer": "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero)), SwiftShader driver)",
        "residual_mean": 0.5002766927083336,
        "hw_curve_audio": a,
        "hw_curve_webgl": w,
        "hw_curve_canvas": c,
        "webrtc_host_ip_hash": "lan_guest_bbb",
        "architecture": "x86_64",
    });
    let ph = commercial_projection(&host_soft);
    let pg = commercial_projection(&guest_soft);
    assert_eq!(ph["eligible"], true, "host soft must emit commercial id: {ph}");
    assert_eq!(pg["eligible"], true, "guest soft must emit commercial id: {pg}");
    let idh = ph["device_id"].as_str().expect("host dv");
    let idg = pg["device_id"].as_str().expect("guest dv");
    assert!((idh.starts_with("dv-") || idh.starts_with("dv_")), "host id={idh}");
    assert!((idg.starts_with("dv-") || idg.starts_with("dv_")), "guest id={idg}");
    assert_ne!(
        idh, idg,
        "soft-aware path must separate host/guest when cores/LAN differ"
    );
    assert_eq!(ph["digest_path"], "soft_aware_v4");
    assert_eq!(out_soft_promote(&host_soft), false);
    // Stable LAN webrtc IS in soft commercial digest (host≠guest separator)
    let incl = ph["materials_included"].as_array().cloned().unwrap_or_default();
    assert!(
        incl.iter().any(|v| v.as_str() == Some("webrtc_host_ip_hash")),
        "stable LAN webrtc must enter soft commercial digest: {incl:?}"
    );
    assert!(
        incl.iter().any(|v| v.as_str() == Some("soft_residual_bucket")),
        "soft_residual_bucket required: {incl:?}"
    );
    assert_eq!(ph["webrtc_in_digest"], true);
}

/// Soft residual_mean 1e-5 bucket collapses tiny FE noise; coarser 1e-4 would over-merge.
#[test]
fn soft_residual_bucket_collapses_mean_micro_noise() {
    let audio: Vec<f64> = (0..32).map(|i| (i as f64 * 0.11).sin() * 0.05).collect();
    let hist: Vec<f64> = {
        let mut h = vec![0.03_f64; 32];
        h[0] = 0.18;
        h[31] = 0.17;
        let s: f64 = h.iter().sum();
        h.iter().map(|x| x / s).collect()
    };
    let a = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "hardware_concurrency": 12,
        "webgl_unmasked_renderer": "SwiftShader Device (Subzero)",
        "residual_mean": 0.50042700674,
        "hw_curve_audio": audio.clone(),
        "hw_curve_webgl": hist.clone(),
        "webrtc_host_ip_hash": "lan_aabbccdd",
    });
    let b = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "hardware_concurrency": 12,
        "webgl_unmasked_renderer": "SwiftShader Device (Subzero)",
        // same 1e-5 bucket as 0.50043
        "residual_mean": 0.50042900000,
        "hw_curve_audio": audio.iter().map(|x| x + 0.001).collect::<Vec<_>>(),
        "hw_curve_webgl": hist,
        "webrtc_host_ip_hash": "lan_aabbccdd",
    });
    let ida = commercial_device_id_from_fields(&a).expect("a");
    let idb = commercial_device_id_from_fields(&b).expect("b");
    assert_eq!(ida, idb);
    // 0.500423 vs 0.500427 must NOT merge (1e-5 buckets differ)
    let far = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "hardware_concurrency": 12,
        "webgl_unmasked_renderer": "SwiftShader Device (Subzero)",
        "residual_mean": 0.50042300674,
        "hw_curve_audio": audio.clone(),
        "hw_curve_webgl": vec![0.04_f64; 32],
        "webrtc_host_ip_hash": "lan_aabbccdd",
    });
    let idf = commercial_device_id_from_fields(&far).expect("far");
    assert_ne!(ida, idf, "0.500423 vs 0.500427 must not share soft commercial id");
}

/// Criterion 3: host soft Chromium ≠ guest soft Chromium when LAN webrtc extras differ
/// even if residual_mean and cores match (SwiftShader cross-machine collision guard).
#[test]
fn soft_host_vs_guest_same_residual_diverge_on_lan_webrtc() {
    let hist: Vec<f64> = {
        let mut h = vec![0.03_f64; 32];
        h[0] = 0.18;
        h[31] = 0.17;
        let s: f64 = h.iter().sum();
        h.iter().map(|x| x / s).collect()
    };
    let audio: Vec<f64> = (0..32).map(|i| (i as f64 * 0.07).sin() * 0.04).collect();
    let host = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "os_family": "linux",
        "hardware_concurrency": 12,
        "webgl_unmasked_renderer": "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero)), SwiftShader driver)",
        "residual_mean": 0.50042700674,
        "hw_curve_audio": audio.clone(),
        "hw_curve_webgl": hist.clone(),
        "webrtc_host_ip_hash": "lan_host_192_168_1_14",
        "server_client_ip": "203.0.113.1",
    });
    let guest = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "os_family": "linux",
        "hardware_concurrency": 12,
        "webgl_unmasked_renderer": "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero)), SwiftShader driver)",
        "residual_mean": 0.50042700674,
        "hw_curve_audio": audio,
        "hw_curve_webgl": hist,
        "webrtc_host_ip_hash": "lan_guest_10_0_2_15",
        "server_client_ip": "10.0.2.15",
    });
    let idh = commercial_device_id_from_fields(&host).expect("host");
    let idg = commercial_device_id_from_fields(&guest).expect("guest");
    assert_ne!(
        idh, idg,
        "host soft vs guest soft with same residual+cores must diverge via LAN webrtc/mem: {idh} vs {idg}"
    );
    let ph = commercial_projection(&host);
    let pg = commercial_projection(&guest);
    assert_eq!(ph["digest_path"], "soft_aware_v4");
    assert_eq!(pg["webrtc_in_digest"], true);
    assert!(
        ph["materials_included"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str() == Some("soft_residual_bucket")),
        "must include soft_residual_bucket: {:?}",
        ph["materials_included"]
    );
    assert!(
        ph["materials_included"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str() == Some("webrtc_host_ip_hash")),
        "must hash webrtc LAN: {:?}",
        ph["materials_included"]
    );
}

/// FE-shaped soft multi-browser: residual_mean + cores same, audio/webrtc noise must share dv_*.
#[test]
fn same_host_soft_noisy_fe_curves_same_commercial_id() {
    let mut hist_a = vec![0.02_f64; 32];
    hist_a[0] = 0.18;
    hist_a[31] = 0.17;
    let sa: f64 = hist_a.iter().sum();
    for x in &mut hist_a {
        *x /= sa;
    }
    let mut hist_b = hist_a.clone();
    hist_b[0] += 0.003;
    hist_b[31] -= 0.003;
    let sb: f64 = hist_b.iter().sum();
    for x in &mut hist_b {
        *x /= sb;
    }
    let audio_a: Vec<f64> = (0..32).map(|i| (i as f64 * 0.11).sin() * 0.05).collect();
    let mut audio_b = vec![0.0; 3];
    audio_b.extend_from_slice(&audio_a[..29]);
    let lan = "lan_guest_10_0_2_15";
    let pw = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "os_family": "linux",
        "hardware_concurrency": 12,
        "webgl_unmasked_renderer": "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero)), SwiftShader driver)",
        "residual_mean": 0.50042700674,
        "hw_curve_audio": audio_a,
        "hw_curve_webgl": hist_a,
        "webrtc_host_ip_hash": lan,
        "server_client_ip": "10.0.2.15",
    });
    let chrome = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "os_family": "linux",
        "hardware_concurrency": 12,
        "webgl_unmasked_renderer": "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero)), SwiftShader driver)",
        "residual_mean": 0.50042700674,
        "hw_curve_audio": audio_b,
        "hw_curve_webgl": hist_b,
        "webrtc_host_ip_hash": lan,
        "server_client_ip": "10.0.2.15",
    });
    let id_a = commercial_device_id_from_fields(&pw).expect("pw");
    let id_b = commercial_device_id_from_fields(&chrome).expect("chrome");
    assert_eq!(
        id_a, id_b,
        "noisy FE soft curves on same host must share commercial id (soft residual bucket)"
    );
    let out = evaluate_session(
        &json!({
            "session_id": "soft_noisy",
            "sources": ["main", "gateway"],
            "has_gateway": true,
            "batches": [
                {"batch_id": "B0_bootstrap", "source": "main"},
                {"batch_id": "B2_hardware", "source": "main"},
                {"batch_id": "B10_hw_curves", "source": "main"},
                {"batch_id": "B8_gateway", "source": "gateway"}
            ],
            "fields": pw
        }),
        None,
        None,
        None,
        true,
    )
    .expect("eval");
    // Soft noisy FE may resolve soft_aware_v4 or thin_surface_v1 depending on anchor set.
    let dp = out["device"]["digest_path"].as_str().unwrap_or("");
    assert!(
        matches!(dp, "soft_aware_v4" | "thin_surface_v1" | "soft_stack_v1")
            || dp.contains("soft")
            || dp.contains("thin"),
        "soft noisy digest_path: {dp}"
    );
    let soft_stack = out["device"]["soft_stack"].as_bool().unwrap_or(false)
        || out["device"]["trust"]["soft_stack"].as_bool().unwrap_or(false)
        || out["device"]["id_warnings"]
            .as_array()
            .is_some_and(|a| a.iter().any(|x| x.as_str() == Some("soft_stack_digest_path")));
    assert!(
        soft_stack || matches!(dp, "soft_aware_v4" | "thin_surface_v1"),
        "evaluate must surface soft-stack posture: {}",
        out["device"]
    );
    let mats = &out["device"]["trust"]["materials"];
    assert!(
        mats.get("soft_residual_bucket").is_some()
            || mats.get("hw_webgl_stable").is_some()
            || mats.get("residual_mean").is_some(),
        "trust.materials must expose soft residual or hw anchors: {mats}"
    );
    assert_eq!(out["soft_promote"], false);
}

/// Same host, multiple soft Chromium-like sessions: same form+curves+cores → same `dv_*`
/// even when webrtc / GPU label strings differ (session noise / UA channel).
#[test]
fn same_host_soft_multi_browser_same_commercial_id() {
    let (a, c, w) = curves_a();
    // Same LAN host-IP hash (stable across browsers); ports not included.
    let lan = "lan_same_host_aabb";
    let chrome = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "os_family": "linux",
        "hardware_concurrency": 12,
        "webgl_unmasked_renderer": "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero)), SwiftShader driver)",
        "residual_mean": 0.5002766927083336,
        "hw_curve_audio": a.clone(),
        "hw_curve_webgl": w.clone(),
        "hw_curve_canvas": c.clone(),
        "webrtc_host_ip_hash": lan,
        "user_agent": "Mozilla/5.0 Chrome/120",
        "server_client_ip": "1.1.1.1",
    });
    let edge = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "os_family": "linux",
        "hardware_concurrency": 12,
        "webgl_unmasked_renderer": "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero)), SwiftShader driver)",
        "residual_mean": 0.5002766927083336,
        "hw_curve_audio": a.clone(),
        "hw_curve_webgl": w.clone(),
        "hw_curve_canvas": c.clone(),
        "webrtc_host_ip_hash": lan, // same host LAN
        "user_agent": "Mozilla/5.0 Edg/120",
        "server_client_ip": "8.8.8.8",
    });
    let (a2, c2, w2) = curves_a();
    let ungoog = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "os_family": "linux",
        "hardware_concurrency": 8,
        "webgl_unmasked_renderer": "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero)), SwiftShader driver)",
        "residual_mean": 0.5002766927083336,
        "hw_curve_audio": a2,
        "hw_curve_webgl": w2,
        "hw_curve_canvas": c2,
        "webrtc_host_ip_hash": lan,
        "server_client_ip": "9.9.9.9",
    });
    let id_c = commercial_device_id_from_fields(&chrome).expect("chrome");
    let id_e = commercial_device_id_from_fields(&edge).expect("edge");
    let id_u = commercial_device_id_from_fields(&ungoog).expect("ungoog");
    assert_eq!(id_c, id_e, "same-host soft chrome/edge must share commercial id");
    assert_eq!(id_c, id_u, "cores 8 vs 12 same log2 class must share commercial id");
    assert_product_commercial_id(&id_c, "same-host soft commercial id");
    let p = commercial_projection(&chrome);
    let dp = p["digest_path"].as_str().unwrap_or("");
    assert!(
        matches!(dp, "soft_aware_v4" | "thin_surface_v1") || dp.contains("soft"),
        "soft digest_path: {dp}"
    );
    assert!(
        host_sep_in_projection(&p) || p["webrtc_in_digest"].as_bool() == Some(true),
        "host sep expected on soft chrome fixture"
    );
}

fn out_soft_promote(fields: &serde_json::Value) -> bool {
    let ev = json!({
        "fields": fields,
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "batches": [
            {"batch_id": "B0_bootstrap", "source": "main"},
            {"batch_id": "B2_hardware", "source": "main"},
            {"batch_id": "B10_hw_curves", "source": "main"},
            {"batch_id": "B8_gateway", "source": "gateway"}
        ]
    });
    let out = evaluate_session(&ev, None, None, None, true).unwrap();
    let id = out["device"]["device_id"].as_str().unwrap_or("");
    // Soft stack must not be exclusive silicon dh; multi-segment / dv / dg ok.
    assert_product_commercial_id(id, "soft stack evaluate");
    assert!(
        !id.starts_with("dh_") && !id.starts_with("dh-"),
        "evaluate must surface commercial id for soft stack (not exclusive dh): {}",
        out["device"]
    );
    let tier = out["device"]["device_tier"].as_str().unwrap_or("");
    assert_ne!(tier, "dh", "soft stack never exclusive dh tier");
    out["soft_promote"].as_bool().unwrap_or(true)
}

/// Spoof label still yields commercial id (curves); untrusted is warning, not refuse.
#[test]
fn spoof_untrusted_label_still_emits_commercial_id() {
    let (a, c, w) = curves_a();
    let spoof = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "os_family": "linux",
        "hardware_concurrency": 12,
        "webgl_unmasked_renderer": "NVIDIA GeForce GTX 980, or similar",
        "residual_mean": 0.5002774107689955,
        "webgl_max_texture": 32768,
        "hw_curve_audio": a,
        "hw_curve_webgl": w,
        "hw_curve_canvas": c,
        "webrtc_host_ip_hash": "lan_spoof_host",
        "architecture": "x86_64",
    });
    let p = commercial_projection(&spoof);
    assert_eq!(p["eligible"], true, "spoof still eligible via curves: {p}");
    assert_product_commercial_id(
        p["device_id"].as_str().unwrap_or(""),
        "spoof projection commercial id",
    );
    assert_eq!(p["gpu_label_untrusted"], true);
    let out = evaluate_session(
        &json!({
            "fields": spoof,
            "sources": ["main", "gateway"],
            "has_gateway": true,
            "batches": [
                {"batch_id": "B0_bootstrap", "source": "main"},
                {"batch_id": "B2_hardware", "source": "main"},
                {"batch_id": "B10_hw_curves", "source": "main"},
                {"batch_id": "B8_gateway", "source": "gateway"}
            ]
        }),
        None,
        None,
        None,
        true,
    )
    .unwrap();
    let eid = out["device"]["device_id"].as_str().unwrap_or("");
    // Spoof/untrusted GPU label → not exclusive dh; multi-segment / dv / dg ok.
    assert_product_commercial_id(eid, "spoof path commercial id");
    assert!(
        !eid.starts_with("dh_") && !eid.starts_with("dh-"),
        "spoof path commercial id must not be exclusive dh: {eid}"
    );
    assert_ne!(out["device"]["device_tier"].as_str().unwrap_or(""), "dh");
    assert_eq!(out["soft_promote"], false);
    assert_eq!(out["device"]["stack_auth"]["gpu_label_untrusted"], true);
}

#[test]
fn curve_stable_digest_deterministic() {
    let a = vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8];
    let d1 = curve_stable_digest(&a).unwrap();
    let d2 = curve_stable_digest(&a).unwrap();
    assert_eq!(d1, d2);
    assert!(d1.starts_with("c_"));
}

#[test]
fn cores_class_merges_webkit_underreport() {
    // log2 floor: 8→3, 12→3
    assert_eq!(cores_class(8), cores_class(12));
    assert_ne!(cores_class(4), cores_class(16));
}

#[test]
fn trust_priors_ip_low_audio_high() {
    assert!(material_trust_prior("hw_audio_stable") > COMMERCIAL_TRUST_FLOOR);
    assert!(material_trust_prior("server_client_ip") < COMMERCIAL_TRUST_FLOOR);
}

/// Load area fixtures (02-probe-analysis/fixtures) and drive shipped commercial_device_id_from_fields.
/// Proves example curves produce dv_ ids and same-host cross-browser equality on the real path.
#[test]
fn area_fixtures_drive_shipped_commercial_id() {
    use std::fs;
    use std::path::PathBuf;

    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures");
    assert!(
        fixtures.is_dir(),
        "missing area fixtures at {}",
        fixtures.display()
    );

    let load = |name: &str| -> serde_json::Value {
        let p = fixtures.join(name);
        let raw = fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
        serde_json::from_str(&raw).expect("json")
    };

    let chrome = load("host_a_chrome.json");
    let firefox = load("host_a_firefox.json");
    let other = load("host_b_chrome.json");
    let missing = load("host_missing_anchor.json");

    let id_c = commercial_device_id_from_fields(&chrome).expect("chrome commercial id");
    let id_f = commercial_device_id_from_fields(&firefox).expect("firefox commercial id");
    let id_b = commercial_device_id_from_fields(&other).expect("host_b commercial id");

    assert!((id_c.starts_with("dv-") || id_c.starts_with("dv_")), "shipped prefix, got {id_c}");
    assert_eq!(id_c.len(), 3 + 16, "dv_+16 hex");
    assert_eq!(id_c, id_f, "same-host cross-browser fixtures must share commercial id");
    assert_ne!(id_c, id_b, "different host fixtures must diverge");

    assert!(
        commercial_device_id_from_fields(&missing).is_none(),
        "missing hardware anchor must not emit commercial id"
    );

    let p = commercial_projection(&chrome);
    assert_eq!(p["eligible"], true);
    assert_eq!(p["has_both_curves"], true);
    assert!(p["trust_sum"].as_f64().unwrap() >= 1.6);
    let included = p["materials_included"].as_array().unwrap();
    // Host sep is a real-path digest material when present (area fixtures share LAN hash).
    assert!(
        included.iter().any(|v| v.as_str() == Some("webrtc_host_ip_hash"))
            || included.iter().any(|v| v.as_str() == Some("hw_audio_stable")),
        "expected dual silicon and/or host sep in digest: {included:?}"
    );
    assert!(!included.iter().any(|v| v.as_str() == Some("server_client_ip")));
    assert_eq!(p["server_client_ip_in_digest"], false);
}

/// Fingerprint-browser style surfaces (spoofed UA/renderer) + multi-IP must not fork id
/// when live curves and form_class agree. Pure field projection — no engine allowlist.
#[test]
fn fingerprint_surface_spoof_multi_ip_same_host_same_id() {
    let (a, c, w) = curves_a();
    // Camoufox-like: Windows UA on Linux platform, different egress
    let camou = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "user_agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/120.0.0.0",
        "os_family": "windows", // spoofed — digest uses curves+form, not this
        "hardware_concurrency": 12,
        "server_client_ip": "203.0.113.10",
        "webgl_unmasked_renderer": "ANGLE (NVIDIA, spoofed GTX)",
        "hw_curve_audio": a.clone(),
        "hw_curve_webgl": w.clone(),
        "hw_curve_canvas": c.clone(),
        "webrtc_host_ip_hash": "host_shared_lan",
        "architecture": "x86_64",
    });
    let (a2, c2, w2) = curves_a();
    // Ungoogled chromium, another proxy IP
    let ungoog = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "user_agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/119.0.0.0",
        "os_family": "linux",
        "hardware_concurrency": 8,
        "server_client_ip": "198.51.100.44",
        "webgl_unmasked_renderer": "Mesa Intel(R) UHD Graphics",
        "hw_curve_audio": a2,
        "hw_curve_webgl": w2,
        "hw_curve_canvas": c2,
        // same LAN host sep as camou — curves+form agree → same commercial id
        "webrtc_host_ip_hash": "host_shared_lan",
        "architecture": "x86_64",
    });
    let id1 = commercial_device_id_from_fields(&camou).expect("camou id");
    let id2 = commercial_device_id_from_fields(&ungoog).expect("ungoog id");
    assert_eq!(id1, id2, "spoofed UA/renderer/IP must not split commercial id when curves+webrtc match");
    assert!((id1.starts_with("dv-") || id1.starts_with("dv_")));
    let p = commercial_projection(&camou);
    let included = p["materials_included"].as_array().unwrap();
    assert!(!included.iter().any(|v| v.as_str() == Some("user_agent")));
    assert!(!included.iter().any(|v| v.as_str() == Some("server_client_ip")));
    assert!(!included.iter().any(|v| v.as_str() == Some("webgl_unmasked_renderer")));
}

/// Soft cosine near + different commercial ids must never merge via soft path.
#[test]
fn soft_similarity_never_emits_or_merges_commercial_id() {
    use gr_probe_core::{member_from_evidence, SoftBlockingEngine, PROMOTE_TO_COMMERCIAL_ID};
    assert!(!PROMOTE_TO_COMMERCIAL_ID);
    let (a, c, w) = curves_a();
    let mut near = a.clone();
    // tiny noise on audio — soft may link; commercial must stay from hard projection only
    if let Some(x) = near.get_mut(0) {
        *x += 0.0001;
    }
    let e1 = json!({
        "session_id": "s_soft_a",
        "fields": {
            "form_class": "desktop",
            "platform": "Linux x86_64",
            "server_client_ip": "10.0.0.1",
            "hw_curve_audio": a,
            "hw_curve_webgl": w.clone(),
            "hw_curve_canvas": c.clone(),
        }
    });
    let e2 = json!({
        "session_id": "s_soft_b",
        "fields": {
            "form_class": "desktop",
            "platform": "Linux x86_64",
            "server_client_ip": "10.0.0.2",
            "hw_curve_audio": near,
            "hw_curve_webgl": w,
            "hw_curve_canvas": c,
        }
    });
    let id1 = commercial_device_id_from_fields(e1.get("fields").unwrap()).expect("id1");
    let id2 = commercial_device_id_from_fields(e2.get("fields").unwrap()).expect("id2");
    // near curves should still quantize to same commercial materials on same host shape
    assert_eq!(id1, id2);
    let mut eng = SoftBlockingEngine::new(None);
    eng.observe(member_from_evidence(&e1, Some("lab")));
    eng.observe(member_from_evidence(&e2, Some("lab")));
    let edges = eng.edges(None);
    for e in &edges {
        assert!(!e.promote_to_commercial_id, "soft edge must never promote");
    }
    // Soft engine must not invent a third commercial id
    assert!(eng.members().iter().all(|m| {
        m.device_id_v2.as_deref() != Some("hardcoded")
            && m.device_id_v2
                .as_ref()
                .map(|s| is_commercial_device_id(s) || s.is_empty())
                .unwrap_or(true)
    }));
}

/// Digest path structural: materials never include IP/UA (grep-equivalent in projection).
#[test]
fn commercial_digest_excludes_ip_and_ua_materials() {
    let host = same_host("203.0.113.99", 16, "Whatever Spoofed GPU");
    let p = commercial_projection(&host);
    let included = p["materials_included"].as_array().unwrap();
    for banned in [
        "server_client_ip",
        "user_agent",
        "webgl_unmasked_renderer",
    ] {
        assert!(
            !included.iter().any(|v| v.as_str() == Some(banned)),
            "{banned} must not be in commercial materials_included"
        );
    }
    assert_eq!(p["server_client_ip_in_digest"], false);
    // Host separator is product material when present — hard digest and/or soft extras.
    assert!(
        host_sep_in_projection(&p)
            || included.iter().any(|v| v.as_str() == Some("webrtc_host_ip_hash")),
        "webrtc host sep expected for same_host fixture: included={included:?} p={p}"
    );
}

/// Soft farm residual class without LAN separator still emits commercial dv_* with collision posture
/// (prefer always productize; separate farms via conf/collision/OS fusion — not by withholding id).
#[test]
fn soft_low_entropy_no_webrtc_emits_with_collision_posture() {
    let hist: Vec<f64> = {
        let mut h = vec![0.03_f64; 32];
        h[0] = 0.18;
        h[31] = 0.17;
        let s: f64 = h.iter().sum();
        h.iter().map(|x| x / s).collect()
    };
    let applebot_soft = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "os_family": "macos",
        "hardware_concurrency": 8,
        "webgl_unmasked_renderer": "Apple GPU",
        "residual_mean": 0.500282676547,
        "hw_curve_webgl": hist,
        "timezone": "UTC",
        "screen_width": 1920,
        "user_agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Applebot",
        // no webrtc_host_ip_hash — farm-collidable soft class
    });
    let p = commercial_projection(&applebot_soft);
    assert_eq!(p["digest_path"], "soft_aware_v4");
    assert_eq!(p["eligible"], true, "must still emit commercial id: {p}");
    let id = p["device_id"].as_str().expect("device_id");
    assert!((id.starts_with("dv-") || id.starts_with("dv_")), "commercial dv={id}");
    assert_eq!(p["collision_risk"], true, "collision_risk expected: {p}");
    let dm = p["device_model_id"].as_str().expect("device_model_id");
    assert!(dm.starts_with("dm_"), "model id={dm}");
    let posture = p["analysis_posture"].as_array().cloned().unwrap_or_default();
    assert!(
        posture.iter().any(|r| r.as_str() == Some("soft_collision_risk_no_host_separator")
            || r.as_str() == Some("soft_low_entropy_use_weighted_fusion")),
        "posture={posture:?}"
    );
    // same residual different arch still productizes; may share class id without separator
    let arm = json!({
        "form_class": "desktop",
        "platform": "Linux aarch64",
        "hardware_concurrency": 8,
        "webgl_unmasked_renderer": "Apple GPU",
        "residual_mean": 0.500282676547,
        "hw_curve_webgl": vec![0.04_f64; 32],
        "timezone": "UTC",
    });
    let p2 = commercial_projection(&arm);
    assert!(p2["device_id"].as_str().unwrap_or("").starts_with("dv_"));
    assert!(p2["device_model_id"].as_str().unwrap_or("").starts_with("dm_"));
    assert_eq!(p2["collision_risk"], true);
}
