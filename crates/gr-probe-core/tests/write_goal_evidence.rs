//! One-shot evidence writer for goal harness (shipped functions only).
use gr_probe_core::product_scores::{score_br, score_os};
use gr_probe_core::stack_auth::stack_auth_from_fields;
use gr_probe_core::{commercial_projection, evaluate_session, select_device_tier, BotScore, TruthResult};
use serde_json::{json, Value};
use std::env;
use std::fs;
use std::path::PathBuf;

fn audio() -> Vec<f64> {
    (0..32).map(|i| (i as f64 * 0.11).sin().abs() * 0.5 + 0.05).collect()
}
fn webgl() -> Vec<f64> {
    (0..16).map(|i| ((i * 13) % 200) as f64 / 200.0 + 0.01).collect()
}

#[test]
fn write_scratch_evidence() {
    let scratch = env::var("GOAL_SCRATCH").unwrap_or_else(|_| "/tmp/grok-goal-5fd7b7b21143/implementer".into());
    let dir = PathBuf::from(&scratch);
    fs::create_dir_all(&dir).ok();

    let soft = json!({
        "form_class": "desktop", "platform": "Linux x86_64", "hardware_concurrency": 8,
        "timezone": "UTC", "os_family": "linux", "architecture": "x86",
        "webgl_unmasked_renderer": "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero)))",
        "residual_mean": 0.500423, "residual_soft_like": true,
        "hw_curve_audio": audio(), "hw_curve_webgl": webgl(),
        "screen_width": 1280, "screen_height": 720,
    });
    let rich = json!({
        "form_class": "desktop", "platform": "Linux x86_64", "hardware_concurrency": 12,
        "timezone": "Asia/Shanghai", "os_family": "linux", "architecture": "x86",
        "webgl_unmasked_renderer": "ANGLE (NVIDIA Corporation, NVIDIA GeForce GTX 1050 Ti/PCIe/SSE2, OpenGL 4.5.0)",
        "residual_mean": 0.500181, "residual_soft_like": false,
        "hw_curve_audio": audio(), "hw_curve_webgl": webgl(),
        "server_client_ip": "203.0.113.10", "server_asn": "AS64500", "server_country": "CN",
        "webrtc_host_ip_hash": "lan_host_sep_real", "screen_width": 1920, "screen_height": 1080,
    });
    let gw = json!({
        "user_agent": "Mozilla/5.0", "server_client_ip": "198.51.100.7",
        "server_asn": "AS64496", "server_country": "US",
    });
    let exotic = json!({
        "form_class": "desktop", "platform": "Win32", "hardware_concurrency": 8, "timezone": "UTC",
        "os_family": "windows",
        "webgl_unmasked_renderer": "ANGLE (NVIDIA, NVIDIA GeForce GTX 980 Direct3D11 vs_5_0 ps_5_0)",
        "residual_mean": 0.500423, "residual_soft_like": true, "residual_available": true,
        "webgl_max_texture": 4096, "hw_curve_audio": audio(), "hw_curve_webgl": webgl(),
        "screen_width": 1366, "screen_height": 768,
    });

    fn pack(name: &str, fields: &Value, sources: &[&str], has_gw: bool) -> Value {
        let batches: Vec<Value> = sources.iter().map(|s| json!({
            "batch_id": if *s == "gateway" { "B8_gateway" } else { "B10_hw_curves" },
            "source": s
        })).collect();
        let mut ev = json!({"sources": sources, "batches": batches, "has_gateway": has_gw, "fields": fields});
        if has_gw {
            ev.as_object_mut().unwrap().insert("gateway_fields".into(), json!({
                "server_client_ip": fields.get("server_client_ip").cloned().unwrap_or(json!("1.1.1.1")),
                "server_asn": fields.get("server_asn").cloned().unwrap_or(json!("AS1")),
                "server_country": fields.get("server_country").cloned().unwrap_or(json!("US")),
            }));
        }
        let proj = commercial_projection(fields);
        let tier = select_device_tier(fields, Some(&ev));
        let out = evaluate_session(&ev, None, None, None, true).expect("eval");
        let product = out.get("product").cloned().unwrap_or(json!({}));
        json!({
            "regime": name,
            "projection_device_id": proj.get("device_id"),
            "projection_eligible": proj.get("eligible"),
            "collision_risk": proj.get("collision_risk"),
            "analysis_posture": proj.get("analysis_posture"),
            "digest_path": proj.get("digest_path"),
            "tier_device_id": tier.get("device_id"),
            "tier": tier.get("device_tier"),
            "product_device_id": product.get("device_id"),
            "product_tier": product.get("device_tier"),
            "product_collision_risk": product.get("collision_risk"),
        })
    }

    let emission = json!({
        "gateway_only": pack("gateway_only", &gw, &["gateway"], true),
        "soft_residual": pack("soft_residual", &soft, &["main"], false),
        "residual_rich": pack("residual_rich", &rich, &["main", "gateway"], true),
    });
    fs::write(dir.join("device_tier_emission.json"), serde_json::to_string_pretty(&emission).unwrap()).unwrap();

    let stack = stack_auth_from_fields(&exotic);
    let truth = TruthResult {
        xsrc_status: "ok".into(), real_band: "unknown".into(), credibility: 0.5,
        fe_only: true, has_server_side: false, has_main_core: true,
        packages: json!({}), reasons: vec![], details: json!({}),
    };
    let bot = BotScore {
        score: 10, verdict: "human".into(), flags: vec![], family: "none".into(),
        algo: "test".into(), details: json!({}),
        robot_name: None,
    };
    let os = score_os(&exotic, &stack, &truth, &[]);
    let br = score_br(&exotic, &stack, &bot, &truth, &[]);
    let tier = select_device_tier(&exotic, Some(&json!({"sources":["main"],"batches":[{"batch_id":"B10","source":"main"}]})));
    let multi = json!({
        "fields_note": "exotic GTX980 label + soft residual + low max_texture",
        "os_reasons": os.get("reasons"),
        "os_score": os.get("score"),
        "br_reasons": br.get("reasons"),
        "br_score": br.get("score"),
        "device_tier_id": tier.get("device_id"),
        "device_tier": tier.get("device_tier"),
    });
    fs::write(dir.join("multi_axis_claim_obs.json"), serde_json::to_string_pretty(&multi).unwrap()).unwrap();
    let soft_id = emission["soft_residual"]["product_device_id"]
        .as_str()
        .unwrap_or("");
    let gw_id = emission["gateway_only"]["product_device_id"]
        .as_str()
        .unwrap_or("");
    assert!(
        soft_id.starts_with("dv_")
            || soft_id.starts_with("dv-")
            || soft_id.starts_with("dv0-")
            || soft_id.starts_with("dh_")
            || soft_id.starts_with("dh-"),
        "soft id {soft_id}"
    );
    // Thin gateway-only (no FE silicon materials) carries no public commercial
    // body — an empty id (provisional) is the honest outcome. When present it
    // must be a provisional/ladder id, never a silicon multi-segment body.
    if !gw_id.is_empty() {
        assert!(
            gw_id.starts_with("dg_")
                || gw_id.starts_with("dg-")
                || gw_id.starts_with("dv0-")
                || gw_id.starts_with("dv_"),
            "gateway id {gw_id}"
        );
    }
}
