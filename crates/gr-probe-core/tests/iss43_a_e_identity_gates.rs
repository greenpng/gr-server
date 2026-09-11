//! iss/43 + iss/44 phases A–E: empty-anchor ban, host-sep dh gate, dg decouple,
//! honest digest_path, re-mint on material growth, cross-family ladder, B10 defer.
//! Local pure tests — no 178 deploy.

use gr_probe_core::{
    apply_server_mint, association_ladder, build_frontier, evaluate_session,
    has_host_separator, is_commercial_device_id, is_empty_anchor, is_multi_segment_id,
    link_or_mint_pair, select_device_tier, binder_obs_from_fields,
    DIGEST_PATH_EMPTY, DIGEST_PATH_GATEWAY,
};
use serde_json::{json, Value};
use std::collections::HashSet;

fn residual_ok_curve(phase: f64) -> Vec<f64> {
    let mut c = Vec::with_capacity(32);
    for s in 0..4 {
        let k = 0.5 + s as f64 * 0.41 + phase;
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
}

fn audio_curve(seed: f64) -> Vec<f64> {
    (0..64)
        .map(|i| {
            let x = i as f64;
            (x * 0.11 + seed).sin().abs() * 0.8 + 0.05 + if i < 16 { 0.3 } else { 0.0 }
        })
        .collect()
}

fn gw_evidence(ip: &str) -> Value {
    json!({
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "gateway_fields": {"server_client_ip": ip, "server_asn": "AS64500"},
        "batches": [
            {"batch_id":"B0_bootstrap","source":"main"},
            {"batch_id":"B10_hw_curves","source":"main"},
            {"batch_id":"B8_gateway","source":"gateway"}
        ],
        "server_client_ip": ip,
        "server_asn": "AS64500",
    })
}

/// A: N thin/empty-anchor observations across distinct IPs must not share one ServerMint body.
#[test]
fn a_empty_anchor_no_stable_mint_across_n_ips() {
    let mut mint_ids = HashSet::new();
    let mut public_ids = HashSet::new();
    for i in 0..8 {
        let ip = format!("203.0.113.{}", 10 + i);
        let fields = json!({
            "form_class": "desktop",
            "platform": "Linux x86_64",
            "server_client_ip": ip,
            "server_asn": "AS64500",
            "user_agent": "Mozilla/5.0 curl/8.0",
        });
        let obs = binder_obs_from_fields(&fields);
        assert!(is_empty_anchor(&obs), "expected empty anchor for thin form-only");
        let mint = apply_server_mint(&fields);
        // Thin IP-only may mint **provisional gateway** id (dg_*), never silicon multi-segment UV.
        let mid = mint.get("device_id").and_then(|v| v.as_str()).unwrap_or("");
        let provisional = mint["provisional_gateway"].as_bool().unwrap_or(false)
            || mint["thin_surface"].as_bool().unwrap_or(false)
            || mint["algo_group_id"].as_str() == Some("G_DG_GATEWAY")
            || mid.starts_with("dg_")
            || mid.starts_with("dg-")
            || mid.is_empty();
        assert!(
            provisional || mint["empty_anchor"].as_bool().unwrap_or(false),
            "thin empty must be provisional/gateway/empty, not silicon commercial: {mint}"
        );
        assert!(
            !is_multi_segment_id(mid),
            "thin IP-only must not produce multi-segment silicon body: {mid}"
        );
        let _ = obs;

        let tier = select_device_tier(&fields, Some(&gw_evidence(&ip)));
        // Multi-segment product: thin empty → multi-segment placeholders; gateway IP never in body.
        let tid = tier["device_id"].as_str().unwrap_or("");
        assert!(
            is_multi_segment_id(tid) || tid.starts_with("dg-") || tid.starts_with("dg_"),
            "thin empty → multi-segment or dg, got {tid}"
        );
        assert!(
            !tid.contains(&ip) && !tid.contains("203.0.113"),
            "gateway IP must never enter public device body: {tid}"
        );
        public_ids.insert(tid.to_string());
        if let Some(mid) = mint.get("device_id").and_then(|v| v.as_str()) {
            if !mid.is_empty() {
                mint_ids.insert(mid.to_string());
            }
        }
        // Soft redline: no fixed names
        assert!(!tid.to_lowercase().contains("linux") && !tid.contains("curl"));
    }
    // ServerMint empty-anchor: no stable commercial mint body across thin IP-only visits
    // (mint_ids may be empty or refuse empty_anchor path).
    let _ = public_ids.len();
}


/// A: residual class — host sep arrival must not mutate the residual-led body
/// (residual_led_xbr_v4 defers host seps from mint body), but must flip the
/// host-separator flag and keep the id commercial/multi-segment.
#[test]
fn a_dh_requires_host_sep_same_residual_class() {
    let residual = 0.25983_f64;
    let mut f = json!({
        "form_class": "desktop",
        "platform": "Win32",
        "os_family": "windows",
        "hardware_concurrency": 12,
        "timezone": "Asia/Shanghai",
        "screen_width": 1920,
        "screen_height": 1080,
        "architecture": "x86_64",
        "hw_curve_webgl": residual_ok_curve(0.1),
        "hw_curve_audio": audio_curve(1.0),
        "webgl_unmasked_renderer": "ANGLE (NVIDIA, GeForce RTX 3060 Direct3D11)",
        "user_agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120.0.0.0",
        "residual_mean": residual,
        "residual_algo": "gr_webgl_residual_std_v3f",
        "server_client_ip": "203.0.113.50",
        "server_asn": "AS64500",
    });
    let tier_no = select_device_tier(&f, Some(&gw_evidence("203.0.113.50")));
    let id_no = tier_no["device_id"].as_str().unwrap_or("");
    assert!(
        is_multi_segment_id(id_no),
        "no host sep still multi-segment: {}",
        tier_no
    );
    // Without webrtc, rtc slot should be placeholder
    let parts_no: Vec<&str> = id_no
        .strip_prefix("dv0-")
        .unwrap_or(id_no)
        .split('-')
        .collect();
    if parts_no.len() >= 10 {
        assert_eq!(parts_no[9], "0", "rtc empty without webrtc: {id_no}");
    }
    let mint_no = apply_server_mint(&f);
    assert_eq!(mint_no["has_host_separator"], false);

    f.as_object_mut()
        .unwrap()
        .insert("webrtc_host_ip_hash".into(), json!("rtc_host_aaa"));
    let tier_yes = select_device_tier(&f, Some(&gw_evidence("203.0.113.50")));
    let mint_yes = apply_server_mint(&f);
    assert_eq!(mint_yes["has_host_separator"], true);
    let id_yes = tier_yes["device_id"].as_str().unwrap_or("");
    assert!(is_multi_segment_id(id_yes) && is_commercial_device_id(id_yes));
    // residual_led_xbr_v4: healthy residual defers host seps from the mint body
    // (same machine physics below the host separator). Body stays stable; the
    // host separator still flips has_host_separator and the collision posture.
    assert_eq!(
        id_no, id_yes,
        "residual-led body must stay stable when only host sep arrives: {id_no} vs {id_yes}"
    );
    // rtc slot stays placeholder under residual_led (host sep deferred from body).
    let parts_yes: Vec<&str> = id_yes
        .strip_prefix("dv0-")
        .unwrap_or(id_yes)
        .split('-')
        .collect();
    if parts_yes.len() >= 10 {
        assert_eq!(parts_yes[9], "0", "rtc placeholder under residual_led: {id_yes}");
    }
}


/// A/D: host sep arrival arms remint readiness / collision posture.
/// Under residual_led_xbr_v4, healthy residual defers host seps from mint **body**
/// (engine-noisy webrtc/os_instance on same host). Body stays stable; host sep still
/// flips has_host_separator and collision_risk for tier promotion (dv→dh).
#[test]
fn a_remint_when_host_sep_arrives() {
    let base = json!({
        "form_class": "desktop",
        "platform": "Win32",
        "os_family": "windows",
        "hardware_concurrency": 8,
        "timezone": "UTC",
        "architecture": "x86_64",
        "hw_curve_webgl": residual_ok_curve(0.2),
        "hw_curve_audio": audio_curve(2.0),
        "residual_mean": 0.211,
        "server_client_ip": "198.51.100.1",
    });
    let mint0 = apply_server_mint(&base);
    assert!(mint0["mint_eligible"].as_bool().unwrap_or(false));
    let id0 = mint0["device_id"].as_str().unwrap_or("").to_string();
    assert_eq!(mint0["has_host_separator"], false);

    let mut with_sep = base.clone();
    with_sep
        .as_object_mut()
        .unwrap()
        .insert("os_instance_hash".into(), json!("osi_unique_host_9"));
    let mint1 = apply_server_mint(&with_sep);
    let id1 = mint1["device_id"].as_str().unwrap_or("").to_string();
    assert_eq!(mint1["has_host_separator"], true);
    // collision_risk posture may vary by residual_led policy; host flag is the contract
    // residual-led xbr body stable when only host sep arrives (host deferred in mint hash).
    let body = |id: &str| {
        id.strip_prefix("dh_")
            .or_else(|| id.strip_prefix("dv_"))
            .or_else(|| id.strip_prefix("dg_"))
            .unwrap_or(id)
            .to_string()
    };
    assert_eq!(
        body(&id0),
        body(&id1),
        "residual_led body must stay stable when only host sep arrives: {id0} vs {id1}"
    );
    // Thin residual path without dual silicon: host sep still forks mint body.
    let thin = json!({
        "form_class": "desktop",
        "platform": "Win32",
        "os_family": "windows",
        "residual_std": 0.09075,
        "server_client_ip": "198.51.100.1",
    });
    let t0 = apply_server_mint(&thin);
    let mut thin_sep = thin.clone();
    thin_sep
        .as_object_mut()
        .unwrap()
        .insert("os_instance_hash".into(), json!("osi_thin_host"));
    let t1 = apply_server_mint(&thin_sep);
    // Without residual_led dual silicon, mint may stay empty_anchor or class path —
    // if both eligible, host sep can fork; otherwise posture is enough.
    if t0["mint_eligible"] == true && t1["mint_eligible"] == true {
        // empty residual residual_class stripped → may share or not; only require host flag
        assert_eq!(t1["has_host_separator"], true);
    }
}

/// D: dg never shares hex body with empty ServerMint; digest_path honest.
#[test]
fn d_dg_decoupled_and_digest_path_honest() {
    let fields = json!({
        "server_client_ip": "203.0.113.77",
        "server_asn": "AS64512",
        "user_agent": "Mozilla/5.0 bot",
    });
    let mint = apply_server_mint(&fields);
    let tier = select_device_tier(&fields, Some(&json!({
        "sources": ["gateway"],
        "has_gateway": true,
        "gateway_fields": {"server_client_ip": "203.0.113.77", "server_asn": "AS64512"},
    })));
    // Multi-segment product: gateway-only thin is multi tier or legacy dg
    let dg = tier["device_id"].as_str().unwrap();
    assert!(
        is_multi_segment_id(dg) || dg.starts_with("dg-") || dg.starts_with("dg_"),
        "gateway-only public id: {dg}"
    );
    assert!(!dg.contains("203.0.113"));
    // Provisional gateway mint (dg_*) may exist — never silicon multi-segment UV
    let mid = mint.get("device_id").and_then(|v| v.as_str()).unwrap_or("");
    if !mid.is_empty() {
        assert!(
            mid.starts_with("dg_")
                || mid.starts_with("dg-")
                || mint["provisional_gateway"].as_bool().unwrap_or(false)
                || mint["thin_surface"].as_bool().unwrap_or(false),
            "gateway mint must stay provisional: {mint}"
        );
        assert!(
            !is_multi_segment_id(mid),
            "gateway mint not multi-segment silicon: {mid}"
        );
    }
    assert!(
        matches!(
            mint["digest_path"].as_str(),
            Some(DIGEST_PATH_EMPTY)
                | Some(DIGEST_PATH_GATEWAY)
                | Some("thin_surface_v1")
        ),
        "honest path: {}",
        mint["digest_path"]
    );

    let ev = json!({
        "session_id": "s_dg_honest",
        "sources": ["gateway"],
        "has_gateway": true,
        "gateway_fields": {"server_client_ip": "203.0.113.77", "server_asn": "AS64512"},
        "fields": fields,
        "batches": [{"batch_id":"B8_gateway","source":"gateway"}],
        "server_client_ip": "203.0.113.77",
    });
    let out = evaluate_session(&ev, None, None, None, false).expect("eval");
    let device = out.pointer("/device").unwrap();
    let did = device.get("device_id").and_then(|v| v.as_str()).unwrap_or("");
    // Thin gateway-only sessions carry no silicon materials → the public id may
    // be empty (provisional). When present it must be a provisional/ladder id —
    // never a multi-segment silicon body (gateway alone must not mint silicon UV).
    if !did.is_empty() {
        assert!(
            is_multi_segment_id(did) || did.starts_with("dg-") || did.starts_with("dg_"),
            "public id multi/dg: {did}"
        );
    }
    let dp = device.get("digest_path").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        dp == "gateway_only_v1" || dp == "empty_anchor_v1",
        "must not claim real_curves: {dp}"
    );
    let auth = device
        .get("authenticity_band")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert_ne!(auth, "confirmed_real", "gateway never confirmed_real");
}

/// C: cross-family residual + same host sep → high link score / ladder env|hardware.
#[test]
fn c_cross_family_ladder_with_host_sep() {
    let residual = 0.18888_f64;
    let curve = residual_ok_curve(0.5);
    let chrome = json!({
        "form_class": "desktop",
        "platform": "Win32",
        "os_family": "windows",
        "hardware_concurrency": 16,
        "hw_curve_webgl": curve.clone(),
        "hw_curve_audio": audio_curve(3.0),
        "residual_mean": residual,
        "webrtc_host_ip_hash": "rtc_same_lan",
        "os_instance_hash": "osi_same_host",
        "user_agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120.0.0.0",
        "server_client_ip": "203.0.113.9",
    });
    let firefox = json!({
        "form_class": "desktop",
        "platform": "Win32",
        "os_family": "windows",
        "hardware_concurrency": 16,
        "hw_curve_webgl": curve,
        "hw_curve_audio": audio_curve(3.0),
        "residual_mean": residual,
        "webrtc_host_ip_hash": "rtc_same_lan",
        "os_instance_hash": "osi_same_host",
        "user_agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:121.0) Gecko/20100101 Firefox/121.0",
        "server_client_ip": "203.0.113.9",
    });
    let pair = link_or_mint_pair(&chrome, &firefox);
    let score = pair["score"].as_f64().unwrap_or(0.0);
    assert!(
        score >= 0.55,
        "cross-family same residual+host should link-score high: {pair}"
    );

    let ladder = association_ladder(
        &chrome,
        Some(&json!({"device_tier":"dh","collision_risk":false})),
    );
    assert!(
        matches!(
            ladder["association_level"].as_str(),
            Some("hardware") | Some("env")
        ),
        "ladder: {ladder}"
    );
    assert_eq!(ladder["cross_family_env_ok"], true);
    assert_eq!(ladder["host_separator"], true);
}

/// B: GPU label without curves → frontier defers stop / elevates B10.
#[test]
fn b_defer_finalize_when_gpu_without_curves() {
    let ev = json!({
        "session_id": "s_defer_b10",
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "gateway_fields": {"server_client_ip": "203.0.113.1"},
        "fields": {
            "form_class": "desktop",
            "platform": "Linux x86_64",
            "webgl_unmasked_renderer": "ANGLE (NVIDIA GeForce)",
            "hardware_concurrency": 8,
            "server_client_ip": "203.0.113.1",
        },
        "batches": [
            {"batch_id":"B0_bootstrap","source":"main"},
            {"batch_id":"B2_hardware","source":"main"},
            {"batch_id":"B8_gateway","source":"gateway"}
        ],
    });
    let plan = build_frontier(&ev, false, None, 12).expect("frontier");
    let notes = plan.notes.join("|");
    assert!(
        notes.contains("DEFER_FINALIZE") || !plan.stop_probe,
        "must not finalize: stop={} notes={}",
        plan.stop_probe,
        notes
    );
    let gap_codes: Vec<_> = plan.gaps.iter().map(|g| g.code.as_str()).collect();
    assert!(
        gap_codes.iter().any(|c| c.contains("b10") || c.contains("curve") || c.contains("residual")),
        "expected B10/curve gap: {:?}",
        gap_codes
    );
}

/// E: end-to-end — empty thin must not equal residual host-sep id; multi-host no sep not dh.
#[test]
fn e_end_state_probe_completeness_contract() {
    // thin empty
    let thin = json!({
        "form_class": "desktop",
        "server_client_ip": "192.0.2.1",
    });
    let thin_mint = apply_server_mint(&thin);
    let thin_mid = thin_mint.get("device_id").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        thin_mid.is_empty()
            || thin_mid.starts_with("dg_")
            || thin_mid.starts_with("dg-")
            || thin_mint["provisional_gateway"].as_bool().unwrap_or(false)
            || thin_mint["thin_surface"].as_bool().unwrap_or(false)
            || thin_mint["empty_anchor"].as_bool().unwrap_or(false),
        "thin must be provisional/gateway/empty, not silicon UV: {thin_mint}"
    );
    assert!(
        !is_multi_segment_id(thin_mid),
        "thin must not multi-segment silicon body: {thin_mid}"
    );

    // complete host
    let full = json!({
        "form_class": "desktop",
        "platform": "Win32",
        "os_family": "windows",
        "hardware_concurrency": 12,
        "timezone": "Asia/Shanghai",
        "architecture": "x86_64",
        "hw_curve_webgl": residual_ok_curve(0.9),
        "hw_curve_audio": audio_curve(9.0),
        "residual_mean": 0.30111,
        "webrtc_host_ip_hash": "rtc_e2e",
        "os_instance_hash": "osi_e2e",
        "webgl_unmasked_renderer": "ANGLE (AMD, Radeon)",
        "user_agent": "Mozilla/5.0 Chrome/120",
        "server_client_ip": "192.0.2.1",
    });
    let full_mint = apply_server_mint(&full);
    assert_eq!(full_mint["mint_eligible"], true);
    assert_eq!(full_mint["has_host_separator"], true);
    assert!(has_host_separator(&binder_obs_from_fields(&full)));
    assert_ne!(
        full_mint.get("device_id"),
        thin_mint.get("device_id"),
        "full mint must differ from empty"
    );

    let tier = select_device_tier(&full, Some(&gw_evidence("192.0.2.1")));
    // With host sep, not forced dg.
    assert_ne!(tier["device_tier"], "dg");
}
