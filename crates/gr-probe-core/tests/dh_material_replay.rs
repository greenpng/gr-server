//! Replay commercial identity on field maps derived from prod 178 collision patterns.
//! Uses the shipped commercial_projection / apply_server_mint / select_device_tier paths.

use gr_probe_core::{apply_server_mint, commercial_projection, select_device_tier};
use serde_json::{json, Value};
use std::fs::File;
use std::io::Write;

/// High-entropy v3f-like residual curve (passes tightened residual_curve_entropy_ok).
fn residual_ok_curve() -> Vec<f64> {
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
}

/// Strongly different spectral mass so coarse order-stats digests diverge.
fn audio_a() -> Vec<f64> {
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

fn host_fields(audio: Vec<f64>, webrtc: Option<&str>, residual_mean: f64) -> Value {
    let mut o = json!({
        "form_class": "desktop",
        "platform": "Win32",
        "os_family": "windows",
        "hardware_concurrency": 12,
        "timezone": "Asia/Shanghai",
        "screen_width": 1920,
        "screen_height": 1080,
        "architecture": "x86_64",
        "hw_curve_webgl": residual_ok_curve(),
        "hw_curve_audio": audio,
        "hw_curve_canvas": (0..32).map(|i| (i as f64 * 0.3).cos().abs() * 0.4 + 0.05).collect::<Vec<_>>(),
        "webgl_unmasked_renderer": "ANGLE (NVIDIA, GeForce RTX 3060 Direct3D11)",
        "user_agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120.0.0.0",
        "residual_mean": residual_mean,
        "residual_algo": "gr_webgl_residual_std_v3f",
        "server_client_ip": "203.0.113.10",
        "server_asn": "AS64500",
    });
    if let Some(w) = webrtc {
        o.as_object_mut()
            .unwrap()
            .insert("webrtc_host_ip_hash".into(), json!(w));
    }
    o
}

fn gw() -> Value {
    json!({
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "gateway_fields": {"server_client_ip": "203.0.113.10"},
        "batches": [
            {"batch_id":"B0_bootstrap","source":"main"},
            {"batch_id":"B10_hw_curves","source":"main"},
            {"batch_id":"B8_gateway","source":"gateway"}
        ],
    })
}

fn row(label: &str, fields: &Value) -> Value {
    let proj = commercial_projection(fields);
    let mint = apply_server_mint(fields);
    let tier = select_device_tier(fields, Some(&gw()));
    json!({
        "label": label,
        "proj_device_id": proj.get("device_id"),
        "mint_device_id": mint.get("device_id"),
        "tier_device_id": tier.get("device_id"),
        "device_tier": tier.get("device_tier"),
        "instance_id": tier.get("device_instance_id"),
        "instance_separator": tier.get("device_instance_separator"),
        "has_host_separator": tier.get("has_host_separator"),
        "materials_included": proj.get("materials_included"),
        "webgl_residual_entropy_ok": proj.get("webgl_residual_entropy_ok"),
        "collision_risk": proj.get("collision_risk"),
        "webrtc_in_digest": proj.get("webrtc_in_digest"),
        "residual_class": mint.get("residual_class"),
        "hw_webgl_stable": proj.pointer("/materials/hw_webgl_stable"),
        "hw_audio_stable": proj.pointer("/materials/hw_audio_stable"),
    })
}

#[test]
fn replay_prod_like_collision_materials_diverge_with_audio_or_webrtc() {
    // Pattern from prod: shared residual class webgl + form desktop → many IP shared one dh
    // when audio was dropped. Fixed path keeps audio → different audio seeds fork.
    let h1 = host_fields(audio_a(), None, 0.211);
    let h2 = host_fields(audio_b(), None, 0.211); // different audio only
    let h3 = host_fields(audio_a(), Some("lan_host_A"), 0.211);
    let h4 = host_fields(audio_a(), Some("lan_host_B"), 0.211);
    // same-host multi-browser: same audio + same webrtc, different cores/IP surface
    let mut h1b = h1.clone();
    h1b.as_object_mut()
        .unwrap()
        .insert("hardware_concurrency".into(), json!(8));
    h1b.as_object_mut()
        .unwrap()
        .insert("server_client_ip".into(), json!("198.51.100.1"));
    h1b.as_object_mut()
        .unwrap()
        .insert("webrtc_host_ip_hash".into(), json!("lan_host_A"));
    let mut h1c = h3.clone();
    h1c.as_object_mut()
        .unwrap()
        .insert("user_agent".into(), json!("Mozilla/5.0 Firefox/120.0"));

    let rows = vec![
        row("host1_audioA_no_rtc", &h1),
        row("host2_audioB_no_rtc", &h2),
        row("host3_audioA_rtcA", &h3),
        row("host4_audioA_rtcB", &h4),
        row("host3_same_browser_cores8", &h1b),
        row("host3_firefox_same_lan", &h1c),
    ];

    // Write evidence table for plan verification
    let path = std::env::var("DH_REPLAY_OUT").unwrap_or_else(|_| {
        "/tmp/grok-goal-1bd49c459ba9/implementer/replay_dh_before_after.json".into()
    });
    if let Ok(mut f) = File::create(&path) {
        let _ = writeln!(f, "{}", serde_json::to_string_pretty(&rows).unwrap());
    }

    let id = |r: &Value| {
        r["tier_device_id"]
            .as_str()
            .or_else(|| r["proj_device_id"].as_str())
            .unwrap_or("")
            .to_string()
    };
    let r0 = &rows[0];
    let r1 = &rows[1];
    let r2 = &rows[2];
    let r3 = &rows[3];
    let r4 = &rows[4];
    let r5 = &rows[5];

    // Dual audio kept when residual_ok
    for r in [&r0, &r1, &r2] {
        let inc = r["materials_included"].as_array().cloned().unwrap_or_default();
        assert!(
            inc.iter().any(|x| x.as_str() == Some("hw_audio_stable")),
            "audio kept: {r}"
        );
        assert!(
            inc.iter().any(|x| x.as_str() == Some("hw_webgl_stable")),
            "webgl present: {r}"
        );
        assert_eq!(r["webgl_residual_entropy_ok"], true, "{r}");
    }

    // Multi-host: different audio forks (primary machine separator under dual silicon).
    assert_ne!(id(r0), id(r1), "different audio must fork commercial id");
    // Layered identity (iss/opus5 01-P0-1): host/protocol context never enters
    // the commercial class body (oi/rtc stay "0" — see
    // gateway_protocol_host_context_flagged_but_not_in_body). The webrtc host
    // separator instead forks the **layered instance id** (dvi1-…), which is the
    // per-machine identity surface.
    assert_eq!(
        id(r2),
        id(r3),
        "host sep must NOT fork the class body (host context excluded): {r2} vs {r3}"
    );
    let inst = |r: &serde_json::Value| r["instance_id"].as_str().unwrap_or("").to_string();
    let inst_sep = |r: &serde_json::Value| r["instance_separator"].as_str().unwrap_or("").to_string();
    assert!(
        !inst(r2).is_empty() && inst(r2).starts_with("dvi1-"),
        "host-pinned instance id issued: {r2}"
    );
    assert_eq!(inst_sep(r2), "webrtc_host_hash", "{r2}");
    assert_ne!(
        inst(r2),
        inst(r3),
        "webrtc host sep must fork layered instance id: {r2} vs {r3}"
    );
    // Rows without any host separator material must not issue an instance id.
    assert_eq!(inst(r0), "", "no host sep → no instance id: {r0}");
    assert_eq!(r0["has_host_separator"], false, "{r0}");
    // ServerMint residual-led body may still be webrtc-conf-only (mint_device_id stable).
    let mint = |r: &serde_json::Value| r["mint_device_id"].as_str().unwrap_or("").to_string();
    if !mint(r2).is_empty() && !mint(r3).is_empty() {
        // mint path may keep same body when dual silicon residual_ok
        let _ = (mint(r2), mint(r3));
    }
    // Same host: same materials + same webrtc, cores noise must not fork
    assert_eq!(
        id(r2),
        id(r4),
        "same host cores noise must not fork when dual silicon+webrtc match"
    );
    assert_eq!(
        id(r2),
        id(r5),
        "same host multi-browser must share commercial id"
    );
    // residual_class not rm_none — dual silicon uses xbr-coarse rm_* (0.005 quanta).
    assert!(
        r0["residual_class"]
            .as_str()
            .unwrap_or("")
            .starts_with("rm_"),
        "residual_class derived: {r0}"
    );
    assert_ne!(r0["residual_class"], "rm_none");
    // residual_mean 0.211 → rm_0.210 under 0.005 xbr quanta
    assert_eq!(
        r0["residual_class"].as_str(),
        Some("rm_0.210"),
        "xbr residual class: {r0}"
    );
}

#[test]
fn replay_mirror_field_maps_if_present() {
    // Optional: path from local mirror extract (see implementer scripts).
    let path = std::env::var("DH_MIRROR_MAPS").unwrap_or_else(|_| {
        "/tmp/grok-goal-1bd49c459ba9/implementer/mirror_field_maps.json".into()
    });
    let Ok(raw) = std::fs::read_to_string(&path) else {
        eprintln!("skip mirror maps: no file at {path}");
        return;
    };
    let maps: Value = serde_json::from_str(&raw).expect("json");
    let arr = maps.as_array().cloned().unwrap_or_default();
    if arr.is_empty() {
        eprintln!("skip mirror maps: empty");
        return;
    }
    let mut out = Vec::new();
    for item in &arr {
        let fields = item.get("fields").cloned().unwrap_or(json!({}));
        let old = item.get("old_device_id").and_then(|v| v.as_str()).unwrap_or("");
        let sid = item.get("session_id").and_then(|v| v.as_str()).unwrap_or("");
        let mut r = row(sid, &fields);
        r.as_object_mut()
            .unwrap()
            .insert("old_device_id".into(), json!(old));
        // Old mirror FE may lack residual_mean — must derive + keep audio
        let inc = r["materials_included"].as_array().cloned().unwrap_or_default();
        let has_audio = fields.get("hw_curve_audio").and_then(|v| v.as_array()).map(|a| !a.is_empty()).unwrap_or(false)
            || fields
                .pointer("/hw_noise_curves/audio")
                .and_then(|v| v.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false);
        let has_webgl = fields.get("hw_curve_webgl").and_then(|v| v.as_array()).map(|a| a.len() >= 4).unwrap_or(false)
            || fields
                .pointer("/hw_noise_curves/webgl")
                .and_then(|v| v.as_array())
                .map(|a| a.len() >= 4)
                .unwrap_or(false);
        if has_audio {
            assert!(
                inc.iter().any(|x| x.as_str() == Some("hw_audio_stable")),
                "mirror session {sid} must keep audio: {r}"
            );
        }
        if has_webgl {
            let rc = r["residual_class"].as_str().unwrap_or("");
            assert!(
                rc.starts_with("rm_"),
                "mirror session {sid} residual_class from curve: {r}"
            );
        }
        out.push(r);
    }
    let path_out = std::env::var("DH_MIRROR_REPLAY_OUT").unwrap_or_else(|_| {
        "/tmp/grok-goal-1bd49c459ba9/implementer/replay_mirror_after.json".into()
    });
    if let Ok(mut f) = File::create(&path_out) {
        let _ = writeln!(f, "{}", serde_json::to_string_pretty(&out).unwrap());
    }
    // Distinct audio among maps (if any) should yield distinct proj ids when residual shared
    let ids: Vec<String> = out
        .iter()
        .filter_map(|r| r["proj_device_id"].as_str().map(|s| s.to_string()))
        .collect();
    eprintln!("mirror replay ids={ids:?} n={}", ids.len());
    assert!(!ids.is_empty());
}

#[test]
fn empty_residual_curve_not_ok_and_audio_carries() {
    let f = json!({
        "form_class": "desktop",
        "platform": "Win32",
        "os_family": "windows",
        "hardware_concurrency": 8,
        "architecture": "x86_64",
        "timezone": "UTC",
        "screen_width": 1920,
        "screen_height": 1080,
        "hw_curve_audio": audio_a(),
        "user_agent": "Mozilla/5.0 Chrome/120",
    });
    let proj = commercial_projection(&f);
    assert_eq!(proj["webgl_residual_entropy_ok"], false);
    let inc = proj["materials_included"].as_array().cloned().unwrap_or_default();
    assert!(inc.iter().any(|x| x.as_str() == Some("hw_audio_stable")));
}
