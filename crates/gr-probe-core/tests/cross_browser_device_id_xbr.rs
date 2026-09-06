//! Cross-browser device_id stability (machine-stable residual + webrtc).
//! Lab regression: same residual_std + same webrtc must not fork mint body when
//! one browser has unit_surface and another does not (Opera vs Chrome fork).

use gr_probe_core::{
    apply_server_mint, evaluate_session, server_mint_commercial_id, binder_obs_from_fields,
};
use serde_json::{json, Value};

fn audio_curve() -> Vec<f64> {
    (0..32).map(|i| (i as f64) * 0.01).collect()
}

fn webgl_curve() -> Vec<f64> {
    (0..32).map(|i| 0.2 + (i as f64) * 0.003).collect()
}

fn base_fields(ua: &str, unit: Option<&str>) -> serde_json::Value {
    let mut f = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "os_family": "linux",
        "hardware_concurrency": 12,
        "timezone": "Asia/Singapore",
        "engine_family": if ua.contains("Firefox") { "gecko" } else { "blink" },
        "hw_curve_webgl": webgl_curve(),
        "hw_curve_audio": audio_curve(),
        "residual_mean": 0.2600390625,
        "residual_std": 0.090751774651,
        "residual_ok": true,
        "residual_available": true,
        "webrtc_host_ip_hash": "e6ba488c",
        "os_instance_hash": format!("osi_{}", if ua.contains("Firefox") { "ff" } else { "ch" }),
        "user_agent": ua,
        "server_client_ip": "127.0.0.1",
        "media_input_count": 1,
        "media_output_count": 2,
        "media_video_count": 1,
        "display_count": 1,
    });
    if let Some(u) = unit {
        f.as_object_mut().unwrap().insert("unit_surface_id".into(), json!(u));
        f.as_object_mut()
            .unwrap()
            .insert("unit_surface_algo".into(), json!("gr_unit_v1"));
        f.as_object_mut()
            .unwrap()
            .insert("unit_multiround_stable".into(), json!(true));
    }
    f
}

#[test]
fn residual_led_mint_ignores_unit_surface_fork() {
    let chrome = base_fields(
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/138.0.0.0 Safari/537.36",
        None,
    );
    let opera = base_fields(
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/138.0.0.0 Safari/537.36 OPR/120.0.0.0",
        Some("unit_opera_only_seed_abc"),
    );
    let firefox = base_fields(
        "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0",
        None,
    );

    let id_c = server_mint_commercial_id(&binder_obs_from_fields(&chrome));
    let id_o = server_mint_commercial_id(&binder_obs_from_fields(&opera));
    let id_f = server_mint_commercial_id(&binder_obs_from_fields(&firefox));

    assert!(!id_c.is_empty(), "chrome mint empty");
    assert_eq!(
        id_c, id_o,
        "unit surface must not fork residual-led mint: chrome={id_c} opera={id_o}"
    );
    // Strip prefix — body must match across engines (prefix may still be dv/dh from class).
    let body = |id: &str| {
        id.strip_prefix("dh_")
            .or_else(|| id.strip_prefix("dv_"))
            .or_else(|| id.strip_prefix("dg_"))
            .unwrap_or(id)
            .to_string()
    };
    assert_eq!(body(&id_c), body(&id_f), "chrome vs firefox body: {id_c} vs {id_f}");
}

#[test]
fn evaluate_promotes_false_dg_when_silicon_present() {
    let fields = base_fields(
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/150.0.0.0 Safari/537.36",
        None,
    );
    // Thin evidence that might historically under-classify tier, but silicon is present.
    let evidence = json!({
        "session_id": "cycle_xbr_test",
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "gateway_fields": {"server_client_ip": "127.0.0.1"},
        "fields": fields,
        "batches": [
            {"batch_id": "B0_bootstrap", "source": "main"},
            {"batch_id": "B10_hw_curves", "source": "main"},
            {"batch_id": "B8_gateway", "source": "gateway"}
        ],
        "fields_by_source": {
            "main": fields,
            "gateway": {"server_client_ip": "127.0.0.1"}
        }
    });
    let out = evaluate_session(&evidence, None, None, None, false).expect("evaluate");
    let did = out
        .pointer("/product/device_id")
        .or_else(|| out.get("device_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let tier = out
        .pointer("/product/device_tier")
        .or_else(|| out.get("device_tier"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let digest = out
        .pointer("/product/digest_path")
        .or_else(|| out.get("digest_path"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        did.starts_with("dh-")
            || did.starts_with("dh_")
            || did.starts_with("dv-")
            || did.starts_with("dv_")
            || did.starts_with("dv0-")
            || did.starts_with("dv4-")
            || did.starts_with("dv5-")
            || did.starts_with("dv6-"),
        "expected silicon commercial id, got tier={tier} id={did} digest={digest} keys={:?}",
        out.as_object().map(|o| o.keys().collect::<Vec<_>>())
    );
    assert_ne!(tier, "dg", "must not stay gateway-only with residual+webrtc digest={digest}");
    assert!(
        digest.contains("real") || digest.contains("unit") || digest.contains("soft"),
        "digest_path should not be gateway_only: {digest}"
    );
}

#[test]
fn residual_std_preferred_for_residual_class() {
    let f = base_fields("Mozilla/5.0 Chrome/120", None);
    let mint = apply_server_mint(&f);
    let rc = mint
        .get("residual_class")
        .or_else(|| mint.pointer("/binders/residual_class"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    // Dual silicon → xbr mean bucket (rm_* at 0.005); thin path may use rs_*.
    assert!(
        rc.starts_with("rs_") || rc.starts_with("rm_"),
        "residual_class={rc} mint={mint}"
    );
    // Dual-channel (webgl+audio) must open residual mint gate.
    let gate = mint.get("multi_source_mint_gate").cloned().unwrap_or(json!({}));
    assert_eq!(
        gate.get("residual_mint_ok").and_then(|v| v.as_bool()),
        Some(true),
        "gate={gate}"
    );
    // base_fields has dual curves → dual-silicon xbr residual class rm_0.260 (0.260039 → 0.260).
    assert_eq!(
        rc, "rm_0.260",
        "dual silicon residual_class must be xbr-coarse mean: {rc}"
    );
}

/// residual_led_xbr_v5: dual HW (audio+webgl) both enter mint body.
/// - residual mean xbr-coarse still shared (rm_0.260)
/// - **same** webgl+audio curves → same mint body across engines (host seps deferred)
/// - **different** webgl digests (engine layout) → **may fork** (honest; not force-merge)
/// - different audio → must fork (second machine)
#[test]
fn dual_hw_mint_same_curves_share_body_across_engines() {
    let audio = audio_curve();
    let webgl = webgl_curve();

    let mut blink = base_fields(
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/150.0.0.0 Safari/537.36",
        None,
    );
    blink
        .as_object_mut()
        .unwrap()
        .insert("residual_mean".into(), json!(0.2600390625));
    blink
        .as_object_mut()
        .unwrap()
        .insert("hw_curve_webgl".into(), json!(webgl.clone()));
    blink
        .as_object_mut()
        .unwrap()
        .insert("hw_curve_audio".into(), json!(audio.clone()));
    blink
        .as_object_mut()
        .unwrap()
        .insert("engine_family".into(), json!("blink"));

    let mut gecko = base_fields(
        "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0",
        None,
    );
    gecko
        .as_object_mut()
        .unwrap()
        .insert("residual_mean".into(), json!(0.26100)); // xbr → same rm_0.260
    gecko
        .as_object_mut()
        .unwrap()
        .insert("hw_curve_webgl".into(), json!(webgl.clone()));
    gecko
        .as_object_mut()
        .unwrap()
        .insert("hw_curve_audio".into(), json!(audio.clone()));
    gecko
        .as_object_mut()
        .unwrap()
        .insert("engine_family".into(), json!("gecko"));
    gecko.as_object_mut().unwrap().insert(
        "webrtc_host_ip_hash".into(),
        json!("gecko_other_lan_hash"),
    );

    let body = |id: &str| {
        id.strip_prefix("dh_")
            .or_else(|| id.strip_prefix("dv_"))
            .or_else(|| id.strip_prefix("dg_"))
            .unwrap_or(id)
            .to_string()
    };
    let id_b = server_mint_commercial_id(&binder_obs_from_fields(&blink));
    let id_g = server_mint_commercial_id(&binder_obs_from_fields(&gecko));
    assert!(!id_b.is_empty() && !id_g.is_empty());
    assert_eq!(
        body(&id_b),
        body(&id_g),
        "same dual HW curves + residual xbr must share mint body: {id_b} vs {id_g}"
    );

    // Divergent webgl curve (WebKit layout) → dual HW digests differ → may fork (honest).
    let mut webgl_wk = webgl.clone();
    for (i, v) in webgl_wk.iter_mut().enumerate() {
        *v = (*v * 1.25 + 0.03 * (i as f64 % 5.0)).min(1.0);
    }
    let mut webkit = blink.clone();
    webkit
        .as_object_mut()
        .unwrap()
        .insert("engine_family".into(), json!("webkit"));
    webkit
        .as_object_mut()
        .unwrap()
        .insert("hw_curve_webgl".into(), json!(webgl_wk));
    let id_w = server_mint_commercial_id(&binder_obs_from_fields(&webkit));
    // Not asserting equality: probe calibration should reduce this; analysis must not force-merge.
    let _ = id_w;

    // Different audio → must fork even with same residual + webgl.
    let mut other = blink.clone();
    other.as_object_mut().unwrap().insert(
        "hw_curve_audio".into(),
        json!((0..32).map(|i| 0.9 - (i as f64) * 0.02).collect::<Vec<_>>()),
    );
    let id_o = server_mint_commercial_id(&binder_obs_from_fields(&other));
    assert_ne!(
        body(&id_b),
        body(&id_o),
        "different audio must fork residual_led_xbr_v5"
    );

    let mint_b = apply_server_mint(&blink);
    let mint_g = apply_server_mint(&gecko);
    assert_eq!(mint_b["residual_class"].as_str(), Some("rm_0.260"));
    assert_eq!(mint_g["residual_class"].as_str(), Some("rm_0.260"));
}

#[test]
fn single_hw_audio_cannot_qualify_dh() {
    use gr_probe_core::select_device_tier;
    let f = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "os_family": "linux",
        "hardware_concurrency": 12,
        "timezone": "UTC",
        "hw_curve_audio": audio_curve(),
        // no webgl curve
        "webrtc_host_ip_hash": "lan_abc",
        "server_client_ip": "127.0.0.1",
        "user_agent": "Mozilla/5.0 Chrome/150",
    });
    let evidence = json!({
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "gateway_fields": {"server_client_ip": "127.0.0.1"},
        "batches": [
            {"batch_id":"B0_bootstrap","source":"main"},
            {"batch_id":"B10_hw_curves","source":"main"},
            {"batch_id":"B8_gateway","source":"gateway"}
        ],
    });
    let tier = select_device_tier(&f, Some(&evidence));
    assert_ne!(
        tier.get("device_tier").and_then(|v| v.as_str()),
        Some("dh"),
        "single HW audio must not promote dh: {tier}"
    );
}

#[test]
fn single_source_residual_alone_not_mint_body() {
    // residual_std without dual silicon channel → conf-only, no residual_class in mint.
    let thin = json!({
        "form_class": "desktop",
        "residual_std": 0.09075,
        "server_client_ip": "127.0.0.1",
    });
    let mint = apply_server_mint(&thin);
    let gate = mint.get("multi_source_mint_gate").cloned().unwrap_or(json!({}));
    assert_eq!(gate["residual_mint_ok"], false);
    // residual_class stripped from mint obs
    assert!(
        mint.get("residual_class").is_none()
            || mint.get("residual_class") == Some(&Value::Null),
        "single-source residual must not enter mint body: {mint}"
    );
}
