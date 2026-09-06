//! v3f residual curve pairwise separability + class-collision posture.
//! Verifies webgl_comm_v4 + entropy gate + dual-silicon cores demote locally
//! before prod deploy.

use gr_probe_core::{
    apply_server_mint, commercial_projection, materials_detail, residual_curve_entropy_ok,
    residual_magrank_key, select_device_tier, select_residual_curve_from_paths,
    webgl_commercial_digest, webgl_commercial_digest_for_engine,
};
use serde_json::json;
use serde_json::Map;

/// Synthetic v3f-like curve: 4 seeds × (6 strip std + 2 global) = 32.
/// Distinct GPUs get different strip-std profiles.
fn v3f_curve(seed: f64, scale: f64) -> Vec<f64> {
    let mut c = Vec::with_capacity(32);
    for s in 0..4 {
        let k = seed + s as f64 * 0.37;
        for t in 0..6 {
            // strip pixel std — primary commercial signal (v3f)
            let v = ((k * 1.7 + t as f64 * 0.51).sin().abs() * 0.12 + 0.02) * scale
                + (t as f64 * 0.003);
            c.push((v * 1e5).round() / 1e5);
        }
        // global mean + std for seed
        let gm = ((k * 0.9).cos().abs() * 0.35 + 0.15) * scale;
        let gs = ((k * 1.1).sin().abs() * 0.08 + 0.03) * scale;
        c.push((gm * 1e5).round() / 1e5);
        c.push((gs * 1e5).round() / 1e5);
    }
    c
}

fn audio_curve(seed: f64) -> Vec<f64> {
    (0..64)
        .map(|i| {
            let x = i as f64;
            (x * (0.11 + seed * 0.07) + seed).sin().abs() * 0.7 + 0.05
                + if i < 12 { seed * 0.2 } else { 0.0 }
        })
        .collect()
}

fn host_fields(webgl: Vec<f64>, audio: Vec<f64>, cores: i64) -> serde_json::Value {
    json!({
        "form_class": "desktop",
        "platform": "Win32",
        "os_family": "windows",
        "hardware_concurrency": cores,
        "timezone": "Asia/Shanghai",
        "screen_width": 1920,
        "screen_height": 1080,
        "architecture": "x86_64",
        "hw_curve_webgl": webgl,
        "hw_curve_audio": audio,
        "residual_algo": "gr_webgl_residual_std_v3f",
        "residual_mean": 0.211,
        "webgl_unmasked_renderer": "ANGLE (NVIDIA, GeForce RTX 3060 Direct3D11)",
        "user_agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120.0.0.0",
        "server_client_ip": "203.0.113.50",
    })
}

fn dead_hist() -> Vec<f64> {
    vec![
        0.03418, 0.04199, 0.03027, 0.03906, 0.02637, 0.0293, 0.03223, 0.03027, 0.03516, 0.03125,
        0.02832, 0.02051, 0.03223, 0.02539, 0.0332, 0.03223, 0.0293, 0.02344, 0.02637, 0.02734,
        0.02734, 0.03418, 0.0332, 0.0332, 0.02637, 0.02734, 0.02832, 0.04004, 0.02637, 0.04102,
        0.03418, 0.04004,
    ]
}

#[test]
fn v3f_distinct_gpus_diverge_commercial_digest() {
    let a = v3f_curve(0.5, 1.0);
    let b = v3f_curve(2.1, 1.15);
    assert!(
        residual_curve_entropy_ok(&a),
        "v3f-like a must pass entropy gate"
    );
    assert!(
        residual_curve_entropy_ok(&b),
        "v3f-like b must pass entropy gate"
    );
    let da = webgl_commercial_digest(&a).expect("da");
    let db = webgl_commercial_digest(&b).expect("db");
    assert_ne!(
        da, db,
        "distinct v3f residual templates must fork webgl_comm_v4: {da} vs {db}"
    );
    assert!(da.starts_with("wg_") && db.starts_with("wg_"));

    let fa = host_fields(a, audio_curve(0.2), 12);
    let fb = host_fields(b, audio_curve(0.2), 12); // same audio, different webgl
    let pa = commercial_projection(&fa);
    let pb = commercial_projection(&fb);
    let ida = pa["device_id"].as_str().unwrap_or("");
    let idb = pb["device_id"].as_str().unwrap_or("");
    assert!(!ida.is_empty() && !idb.is_empty(), "both eligible: {pa} {pb}");
    assert_ne!(
        ida, idb,
        "distinct v3f webgl must fork commercial device_id"
    );
}

#[test]
fn v3f_same_host_micro_jitter_same_commercial() {
    let base = v3f_curve(0.5, 1.0);
    let mut jitter = base.clone();
    for (i, v) in jitter.iter_mut().enumerate() {
        *v *= 1.0 + if i % 5 == 0 { 0.002 } else { 0.0005 };
    }
    let da = webgl_commercial_digest(&base).expect("da");
    let db = webgl_commercial_digest(&jitter).expect("db");
    // Coarse commercial should absorb tiny multi-engine jitter
    assert_eq!(
        da, db,
        "same-host micro residual jitter must share webgl_comm_v5"
    );
}

#[test]
fn dead_hist_fails_tightened_entropy_gate() {
    let d = dead_hist();
    assert_eq!(
        residual_curve_entropy_ok(&d),
        false,
        "prod-like dead residual hist must fail tightened entropy"
    );
}

#[test]
fn dual_silicon_demotes_cores_from_digest() {
    let f12 = host_fields(v3f_curve(0.5, 1.0), audio_curve(0.3), 12);
    let mut f8 = f12.clone();
    f8.as_object_mut()
        .unwrap()
        .insert("hardware_concurrency".into(), json!(8));
    let p12 = commercial_projection(&f12);
    let p8 = commercial_projection(&f8);
    let inc = p12["materials_included"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        !inc.iter().any(|x| x.as_str() == Some("cores_class")),
        "cores must be demoted under dual silicon: {inc:?}"
    );
    assert_eq!(
        p12["device_id"], p8["device_id"],
        "cores noise must not fork commercial id under dual silicon"
    );
    let posture = p12["analysis_posture"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        posture
            .iter()
            .any(|x| x.as_str() == Some("dual_silicon_demote_cores_tz_screen")
                || x.as_str() == Some("real_dual_silicon_anchors")),
        "posture={posture:?}"
    );
}

#[test]
fn form_wg_class_only_sets_collision_risk() {
    // Webgl only — no audio, no host sep → form+wg_coarse class collision posture
    let mut f = host_fields(v3f_curve(0.5, 1.0), audio_curve(0.3), 12);
    f.as_object_mut().unwrap().remove("hw_curve_audio");
    let p = commercial_projection(&f);
    assert_eq!(
        p["collision_risk"].as_bool(),
        Some(true),
        "form+wg without second silicon/host → collision_risk: {p}"
    );
    let posture = p["analysis_posture"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        posture
            .iter()
            .any(|x| x.as_str() == Some("real_collision_risk_form_wg_coarse_class")),
        "posture={posture:?}"
    );
}

#[test]
fn materials_detail_exposes_keys() {
    let f = host_fields(v3f_curve(0.5, 1.0), audio_curve(0.3), 12);
    let d = materials_detail(&f);
    assert_eq!(d["webgl_comm_algo"], "webgl_comm_v4");
    assert!(d["keys"]["hw_webgl_stable"].as_str().is_some());
    assert!(d["keys"]["hw_audio_stable"].as_str().is_some());
    assert!(d["materials_included"].as_array().unwrap().len() >= 2);
}

#[test]
fn pairwise_matrix_distinct_pairs_split() {
    // 3 synthetic hosts × pairwise: all commercial ids must be unique when both
    // webgl and audio diverge (v3f separability matrix).
    let hosts: Vec<_> = (0..3)
        .map(|i| {
            let s = 0.4 + i as f64 * 0.9;
            host_fields(v3f_curve(s, 1.0 + i as f64 * 0.08), audio_curve(s), 12)
        })
        .collect();
    let ids: Vec<String> = hosts
        .iter()
        .map(|h| {
            commercial_projection(h)
                .get("device_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        })
        .collect();
    for id in &ids {
        assert!(!id.is_empty(), "eligible id required");
    }
    assert_ne!(ids[0], ids[1]);
    assert_ne!(ids[0], ids[2]);
    assert_ne!(ids[1], ids[2]);

    // Tier path still productizes (may be dh or dv depending on residual/host)
    let tier = select_device_tier(&hosts[0], None);
    let tid = tier["device_id"].as_str().unwrap_or("");
    assert!(
        (tid.starts_with("dh-") || tid.starts_with("dh_") || tid.starts_with("dv-") || tid.starts_with("dv_") || tid.starts_with("dv0-") || tid.starts_with("dv4-") || tid.starts_with("dv5-") || tid.starts_with("dv6-") || tid.starts_with("dg_") || tid.starts_with("dg-")),
        "tier id={tid} reasons={:?}",
        tier["tier_reasons"]
    );
}

#[test]
fn residual_multipath_server_selects_entropy_ok_path() {
    let good = v3f_curve(0.5, 1.0);
    let flat = vec![0.001; 32];
    let fo = json!({
        "residual_paths": [
            {"path_id":"cold_flat","ok":true,"curve": flat, "entropy_ok": false, "size":128, "warm_frames":0, "mean":0.001, "std":0.0},
            {"path_id":"warm4_v3f_128","ok":true,"curve": good, "entropy_ok": true, "size":128, "warm_frames":4, "mean":0.21, "std":0.09},
            {"path_id":"v3f_256","ok":true,"curve": good, "entropy_ok": true, "size":256, "warm_frames":2, "mean":0.21, "std":0.09}
        ],
        "residual_select": {"chosen_path_id": "warm4_v3f_128"},
        "hw_curve_webgl": flat,
    });
    let map = fo.as_object().cloned().unwrap_or_default();
    let (curve, meta) = select_residual_curve_from_paths(&map);
    assert_eq!(curve.len(), 32);
    assert_ne!(curve, flat);
    assert_eq!(meta["chosen_path_id"], "warm4_v3f_128");
    assert_eq!(meta["policy"], "residual_dual_lane_v1");
    assert_eq!(
        meta["lane_c"]["chosen_path_id"],
        "warm4_v3f_128",
        "classic float paths are Lane-C commercial candidates"
    );
    assert_eq!(residual_magrank_key(&good), residual_magrank_key(&curve));
}

/// Lab B10 curves (real host): blink residual_mean 0.26004 vs webkit 0.26100.
/// **Honest commercial digests may still fork** under webgl_comm_v4 (mu@0.001) —
/// we do **not** coarsen precision to force merge. Magrank@0.02 already agrees;
/// probe multi-pass must reduce mean drift so digests converge without 0.01 quanta.
#[test]
fn lab_blink_webkit_residual_magrank_agrees_comm_may_fork() {
    let blink: Vec<f64> = vec![
        0.2266, 0.22187, 0.2265, 0.2304, 0.22803, 0.22202, 0.49901, 0.22591, 0.22736,
        0.22615, 0.22759, 0.22602, 0.22734, 0.22552, 0.49941, 0.22668, 0.22456, 0.22527,
        0.23102, 0.22304, 0.22118, 0.22821, 0.50499, 0.22569, 0.21985, 0.22519, 0.22827,
        0.22436, 0.22295, 0.22849, 0.49676, 0.22501,
    ];
    let webkit: Vec<f64> = vec![
        0.22752, 0.22693, 0.22336, 0.22382, 0.2259, 0.2279, 0.50201, 0.22601, 0.22531,
        0.22849, 0.22965, 0.22367, 0.22508, 0.22824, 0.49842, 0.22687, 0.22938, 0.22075,
        0.22697, 0.22457, 0.22625, 0.2305, 0.50086, 0.22652, 0.2273, 0.22713, 0.23161,
        0.22684, 0.2264, 0.22829, 0.5016, 0.22794,
    ];
    // Magrank@0.02 quanta: shape agreement (probe target metric).
    let magrank = |c: &[f64]| -> Vec<i64> {
        let mut m: Vec<f64> = c.iter().map(|x| x.abs()).collect();
        m.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        m.into_iter()
            .take(8)
            .map(|x| (x / 0.02).round() as i64)
            .collect()
    };
    assert_eq!(
        magrank(&blink),
        magrank(&webkit),
        "lab same-host magrank must already agree — probe work targets mean/std only"
    );
    let db = webgl_commercial_digest(&blink).expect("blink");
    let dw = webgl_commercial_digest_for_engine(&webkit, Some("webkit")).expect("webkit");
    // Document current honest fork (mu 0.260 vs 0.261). When probe multi-pass closes
    // residual mean drift, digests may converge without coarsening commercial quanta.
    assert!(
        !db.is_empty() && !dw.is_empty(),
        "both digests present: {db} {dw}"
    );
    // residual_class xbr still shared (0.005 quanta on mean only for residual_led class).
    let audio: Vec<f64> = (0..64).map(|i| (i as f64 * 0.01).sin().abs()).collect();
    let mut fb = host_fields(blink.clone(), audio.clone(), 12);
    fb.as_object_mut()
        .unwrap()
        .insert("residual_mean".into(), json!(0.2600390625));
    let mut fw = host_fields(webkit.clone(), audio, 12);
    fw.as_object_mut()
        .unwrap()
        .insert("residual_mean".into(), json!(0.2610028125));
    let mb = apply_server_mint(&fb);
    let mw = apply_server_mint(&fw);
    assert_eq!(mb["residual_class"].as_str(), Some("rm_0.260"));
    assert_eq!(mw["residual_class"].as_str(), Some("rm_0.260"));
    // multi-path select prefers warm entropy-ok over flat FE primary
    let mut multipath = Map::new();
    multipath.insert(
        "residual_paths".into(),
        json!([
            {"path_id":"cold","ok":true,"curve": vec![0.001; 32], "entropy_ok": false, "size":128, "warm_frames":0},
            {"path_id":"warm_blink","ok":true,"curve": blink, "entropy_ok": true, "size":128, "warm_frames":8, "mean":0.260039, "std":0.09075}
        ]),
    );
    multipath.insert("hw_curve_webgl".into(), json!(vec![0.001; 32]));
    let (sel, meta) = select_residual_curve_from_paths(&multipath);
    assert_eq!(meta["chosen_path_id"], "warm_blink");
    assert_eq!(sel.len(), 32);
}
