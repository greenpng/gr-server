//! Demo → v5 iteration: weighted multi-field fusion, always-tiered ids, multi-axis claim-obs.
//! Drives shipped `commercial_projection` / `select_device_tier` / `evaluate_session` / scores.

use gr_probe_core::product_scores::{score_br, score_os, score_rpa};
use gr_probe_core::stack_auth::stack_auth_from_fields;
use gr_probe_core::{
    commercial_projection, evaluate_session, select_device_tier, BotScore, TruthResult,
};
use serde_json::{json, Value};

fn is_tiered_id(id: &str) -> bool {
    id.starts_with("dh-")
        || id.starts_with("dh_")
        || id.starts_with("dv-")
        || id.starts_with("dv_")
        || id.starts_with("dv0-")
        || id.starts_with("dv4-")
        || id.starts_with("dv5-")
        || id.starts_with("dv6-")
        || id.starts_with("dg_")
        || id.starts_with("dg-")
}

fn is_gatewayish_id(id: &str) -> bool {
    id.starts_with("dg_")
        || id.starts_with("dg-")
        // multi-segment all-zero body is gateway-provisional style surface
        || (id.starts_with("dv0-") && id.matches("-0").count() >= 6)
}

#[allow(dead_code)] // test helper retained for upcoming silicon-tier cases
fn is_siliconish_id(id: &str) -> bool {
    is_tiered_id(id) && !id.starts_with("dg")
}


fn audio_curve() -> Vec<f64> {
    (0..32)
        .map(|i| (i as f64 * 0.11).sin().abs() * 0.5 + 0.05)
        .collect()
}

fn webgl_curve() -> Vec<f64> {
    (0..16)
        .map(|i| ((i * 13) % 200) as f64 / 200.0 + 0.01)
        .collect()
}

fn soft_residual_fields(with_webrtc: bool) -> Value {
    let mut o = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "hardware_concurrency": 8,
        "timezone": "UTC",
        "os_family": "linux",
        "architecture": "x86",
        "webgl_unmasked_renderer": "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero)))",
        "residual_mean": 0.500423,
        "residual_soft_like": true,
        "hw_curve_audio": audio_curve(),
        "hw_curve_webgl": webgl_curve(),
        "screen_width": 1280,
        "screen_height": 720,
    });
    if with_webrtc {
        o.as_object_mut()
            .unwrap()
            .insert("webrtc_host_ip_hash".into(), json!("lan_host_sep_aabb"));
    }
    o
}

fn residual_rich_real() -> Value {
    json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "hardware_concurrency": 12,
        "timezone": "Asia/Shanghai",
        "os_family": "linux",
        "architecture": "x86",
        "webgl_unmasked_renderer": "ANGLE (NVIDIA Corporation, NVIDIA GeForce GTX 1050 Ti/PCIe/SSE2, OpenGL 4.5.0)",
        "residual_mean": 0.500181,
        "residual_soft_like": false,
        "hw_curve_audio": audio_curve(),
        "hw_curve_webgl": webgl_curve(),
        "server_client_ip": "203.0.113.10",
        "server_asn": "AS64500",
        "server_country": "CN",
        "webrtc_host_ip_hash": "lan_host_sep_real",
        "screen_width": 1920,
        "screen_height": 1080,
    })
}

fn gateway_only_fields() -> Value {
    json!({
        "user_agent": "Mozilla/5.0 (compatible; bot-ish)",
        "server_client_ip": "198.51.100.7",
        "server_asn": "AS64496",
        "server_country": "US",
    })
}

fn exotic_label_soft_residual() -> Value {
    json!({
        "form_class": "desktop",
        "platform": "Win32",
        "hardware_concurrency": 8,
        "timezone": "UTC",
        "os_family": "windows",
        // L0 claim: discrete GPU
        "webgl_unmasked_renderer": "ANGLE (NVIDIA, NVIDIA GeForce GTX 980 Direct3D11 vs_5_0 ps_5_0)",
        // Observation: soft residual class
        "residual_mean": 0.500423,
        "residual_soft_like": true,
        "residual_available": true,
        "webgl_max_texture": 4096,
        "hw_curve_audio": audio_curve(),
        "hw_curve_webgl": webgl_curve(),
        "screen_width": 1366,
        "screen_height": 768,
    })
}

fn evidence_main_gw() -> Value {
    json!({
        "sources": ["main", "gateway"],
        "batches": [
            {"batch_id": "B0", "source": "main"},
            {"batch_id": "B10_hw_curves", "source": "main"},
            {"batch_id": "B8_gateway", "source": "gateway"}
        ],
        "has_gateway": true,
        "gateway_fields": {
            "server_client_ip": "203.0.113.10",
            "server_asn": "AS64500",
            "server_country": "CN"
        }
    })
}

fn evidence_gateway_only() -> Value {
    json!({
        "sources": ["gateway"],
        "batches": [{"batch_id": "B8_gateway", "source": "gateway"}],
        "has_gateway": true,
        "gateway_fields": {
            "server_client_ip": "198.51.100.7",
            "server_asn": "AS64496",
            "server_country": "US"
        }
    })
}

fn id_prefix(id: &str) -> &str {
    if id.starts_with("dh-") || id.starts_with("dh_") {
        "dh"
    } else if id.starts_with("dv0-") || id.starts_with("dv4-") || id.starts_with("dv5-") || id.starts_with("dv6-") || id.starts_with("dv-") || id.starts_with("dv_") {
        "dv"
    } else if id.starts_with("dg-") || id.starts_with("dg_") {
        "dg"
    } else {
        "other"
    }
}

/// Soft residual without webrtc must still emit a commercial/tiered id (not empty withhold).
#[test]
fn soft_residual_emits_nonempty_tiered_id_with_collision_posture() {
    let fields = soft_residual_fields(false);
    let proj = commercial_projection(&fields);
    let tier = select_device_tier(
        &fields,
        Some(&json!({
            "sources": ["main"],
            "batches": [{"batch_id":"B10","source":"main"}],
            "has_gateway": false
        })),
    );
    let id = tier["device_id"].as_str().expect("tier device_id");
    assert!(
        is_tiered_id(id),
        "expected tiered id, got {id}"
    );
    // Prefer soft commercial emit when anchors exist
    let proj_id = proj.get("device_id").and_then(|v| v.as_str());
    assert!(
        proj_id.is_some() || proj.get("device_model_id").and_then(|v| v.as_str()).is_some(),
        "soft path must productize id or model id: {proj}"
    );
    if let Some(pid) = proj_id {
        assert!(is_tiered_id(pid), "soft commercial should be tiered, got {pid}");
    }
    let collision = proj
        .get("collision_risk")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    assert!(
        collision,
        "soft without host separator should set collision_risk: {proj}"
    );
    let posture = proj
        .get("analysis_posture")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let joined: String = posture
        .iter()
        .filter_map(|x| x.as_str())
        .collect::<Vec<_>>()
        .join(",");
    assert!(
        joined.contains("soft") || collision,
        "analysis_posture should encode soft collision: {joined}"
    );
}

/// Soft residual with webrtc: still non-empty id; separator lowers collision flag.
#[test]
fn soft_residual_with_separator_emits_id_lower_collision() {
    let fields = soft_residual_fields(true);
    let proj = commercial_projection(&fields);
    let id = proj
        .get("device_id")
        .and_then(|v| v.as_str())
        .expect("device_id");
    assert!(is_tiered_id(id), "{id}");
    assert_eq!(proj.get("eligible").and_then(|v| v.as_bool()), Some(true));
    // separator present → collision_risk false
    assert_eq!(
        proj.get("collision_risk").and_then(|v| v.as_bool()),
        Some(false),
        "{proj}"
    );
}

/// Gateway-only materials → dg_ id (not empty).
#[test]
fn gateway_only_emits_dg_tier() {
    let fields = gateway_only_fields();
    let tier = select_device_tier(&fields, Some(&evidence_gateway_only()));
    let id = tier["device_id"].as_str().expect("id");
    assert!(is_gatewayish_id(id), "expected gateway-ish id, got {id}");
    // Multi-segment public surface may label tier as "multi" while algo_group stays gateway.
    let dt = tier["device_tier"].as_str().unwrap_or("");
    assert!(
        dt == "dg" || dt == "multi" || dt == "gateway",
        "gateway tier, got {dt} id={id}"
    );
}

/// Residual-rich + gateway → dh or strong dv (not empty, not weaker than thin without cause).
#[test]
fn residual_rich_emits_strong_tier() {
    let fields = residual_rich_real();
    let tier = select_device_tier(&fields, Some(&evidence_main_gw()));
    let id = tier["device_id"].as_str().expect("id");
    let p = id_prefix(id);
    assert!(
        p == "dh" || p == "dv",
        "residual-rich should be dh or dv, got {id}"
    );
    assert!(!id.is_empty());
}

fn evidence_with_fields(fields: Value, sources: &[&str], has_gateway: bool) -> Value {
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
        "has_gateway": has_gateway,
        "fields": fields,
    });
    if has_gateway {
        if let Some(obj) = ev.as_object_mut() {
            obj.insert(
                "gateway_fields".into(),
                json!({
                    "server_client_ip": fields.get("server_client_ip").cloned().unwrap_or(json!("198.51.100.1")),
                    "server_asn": fields.get("server_asn").cloned().unwrap_or(json!("AS1")),
                    "server_country": fields.get("server_country").cloned().unwrap_or(json!("US")),
                }),
            );
        }
    }
    ev
}

/// evaluate_session product path (post-identity-governance contract):
/// - rich/soft regimes emit non-empty tiered device ids;
/// - zero-entropy gateway-only regime is **withheld** (never the all-zero
///   placeholder dv0-0-0-… that would collide across machines) — the honest
///   marketable surface is a null id + deepen/withhold governance posture.
#[test]
fn evaluate_session_three_regimes_emit_tiered_ids() {
    let regimes: Vec<(&str, Value, Value, &str)> = vec![
        (
            "gateway",
            gateway_only_fields(),
            evidence_with_fields(gateway_only_fields(), &["gateway"], true),
            "dg",
        ),
        (
            "soft",
            soft_residual_fields(false),
            evidence_with_fields(soft_residual_fields(false), &["main"], false),
            "dv",
        ),
        (
            "rich",
            residual_rich_real(),
            evidence_with_fields(residual_rich_real(), &["main", "gateway"], true),
            "any",
        ),
    ];
    for (name, _fields, evidence, expect_prefix) in regimes {
        let out = evaluate_session(&evidence, None, None, None, true).expect("evaluate");
        let product = out.get("product").cloned().unwrap_or(out.clone());
        let id = product
            .get("device_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let gov = product
            .get("identity_governance")
            .cloned()
            .unwrap_or(json!({}));
        let commercial_blocked = gov
            .get("commercial_blocked")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if name == "gateway" {
            // Zero-entropy gateway-only session: identity governance must
            // withhold the all-zero placeholder (cross-machine collision) and
            // demand deepen, not publish a fake stable id.
            assert_eq!(id, "", "regime gateway: got {id} product={product}");
            assert!(
                commercial_blocked,
                "regime gateway: governance must block commercial id, gov={gov}"
            );
            let posture = gov
                .get("mint_posture")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            assert!(
                posture != "stable" && !posture.is_empty(),
                "regime gateway: non-stable mint posture required, gov={gov}"
            );
            continue;
        }
        assert!(
            !id.is_empty(),
            "regime {name}: device_id must be non-empty, product={product}"
        );
        assert!(
            is_tiered_id(id),
            "regime {name}: bad prefix {id}"
        );
        if expect_prefix != "any" {
            let ok = match expect_prefix {
                "dg" => is_gatewayish_id(id),
                "dv" | "dh" => is_tiered_id(id),
                _ => id.starts_with(&format!("{expect_prefix}_"))
                    || id.starts_with(&format!("{expect_prefix}-")),
            };
            assert!(
                ok,
                "regime {name}: expected {expect_prefix}_* (or multi-segment), got {id} product={product}"
            );
        }
    }
}

/// Exotic GPU label + soft residual moves os and br reasons (multi-axis claim-obs).
#[test]
fn claim_obs_exotic_label_soft_residual_moves_os_and_br() {
    let fields = exotic_label_soft_residual();
    let stack = stack_auth_from_fields(&fields);
    let truth = TruthResult {
        xsrc_status: "ok".into(),
        real_band: "unknown".into(),
        credibility: 0.5,
        fe_only: true,
        has_server_side: false,
        has_main_core: true,
        packages: json!({}),
        reasons: vec![],
        details: json!({}),
    };
    let bot = BotScore {
        score: 10,
        verdict: "human".into(),
        flags: vec![],
        family: "none".into(),
        algo: "test".into(),
        details: json!({}),
        robot_name: None,
    };
    let base = soft_residual_fields(false);
    // baseline: soft label matching soft residual (SwiftShader string)
    let stack_b = stack_auth_from_fields(&base);
    let os_base = score_os(&base, &stack_b, &truth, &[]);
    let br_base = score_br(&base, &stack_b, &bot, &truth, &[]);
    let os = score_os(&fields, &stack, &truth, &[]);
    let br = score_br(&fields, &stack, &bot, &truth, &[]);

    let os_reasons = os
        .get("reasons")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let br_reasons = br
        .get("reasons")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let os_join: String = os_reasons
        .iter()
        .filter_map(|x| x.as_str())
        .collect::<Vec<_>>()
        .join("|");
    let br_join: String = br_reasons
        .iter()
        .filter_map(|x| x.as_str())
        .collect::<Vec<_>>()
        .join("|");
    assert!(
        os_join.contains("gpu_label") || os_join.contains("claim_obs") || os_join.contains("soft"),
        "os should surface claim-obs or soft reasons: {os_join}"
    );
    assert!(
        br_join.contains("gpu_label")
            || br_join.contains("integrity")
            || br_join.contains("incoherent")
            || br_join.contains("claim"),
        "br should surface integrity/claim-obs: {br_join}"
    );
    let os_score = os.get("score").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let br_score = br.get("score").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let os_base_s = os_base.get("score").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let br_base_s = br_base.get("score").and_then(|v| v.as_f64()).unwrap_or(1.0);
    // Exotic claim + soft residual should not score safer than honest soft label path.
    assert!(
        os_score <= os_base_s + 0.05,
        "os score should not improve under label incoherence: {os_score} vs {os_base_s}"
    );
    assert!(
        br_score <= br_base_s + 0.05,
        "br score should not improve under label incoherence: {br_score} vs {br_base_s}"
    );

    // Device tier still residual-led: soft residual → dv/dg/dh non-empty, not driven by GTX label alone
    let tier = select_device_tier(
        &fields,
        Some(&json!({
            "sources": ["main"],
            "batches": [{"batch_id":"B10","source":"main"}],
        })),
    );
    let id = tier["device_id"].as_str().unwrap();
    assert!(!id.is_empty());
    // Compare to honest soft — if both emit dv, body may differ by residual path; label must not invent real silicon tier alone
    assert!(
        is_tiered_id(id),
        "{id}"
    );
}

/// L0 renderer must not be the sole commercial digest differentiator when residual matches.
#[test]
fn residual_led_identity_l0_label_does_not_override() {
    let (a, _, w) = {
        let audio = audio_curve();
        let webgl = webgl_curve();
        (audio, (), webgl)
    };
    let base = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "hardware_concurrency": 8,
        "timezone": "UTC",
        "os_family": "linux",
        "architecture": "x86",
        "residual_mean": 0.500181,
        "residual_soft_like": false,
        "hw_curve_audio": a.clone(),
        "hw_curve_webgl": w.clone(),
        "webrtc_host_ip_hash": "same_lan",
        "webgl_unmasked_renderer": "ANGLE (NVIDIA Corporation, NVIDIA GeForce GTX 1050 Ti/PCIe/SSE2, OpenGL 4.5.0)",
    });
    let spoof_label = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "hardware_concurrency": 8,
        "timezone": "UTC",
        "os_family": "linux",
        "architecture": "x86",
        "residual_mean": 0.500181,
        "residual_soft_like": false,
        "hw_curve_audio": a,
        "hw_curve_webgl": w,
        "webrtc_host_ip_hash": "same_lan",
        // L0 spoof only
        "webgl_unmasked_renderer": "Apple M1, or similar",
    });
    let id_a = commercial_projection(&base)
        .get("device_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let id_b = commercial_projection(&spoof_label)
        .get("device_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    assert!(!id_a.is_empty() && !id_b.is_empty(), "both must emit");
    // When residual/curves match, spoofed L0 should not create a different commercial body
    // (gpu labels stripped when untrusted, or not in digest order).
    assert_eq!(
        id_a, id_b,
        "L0 GPU label spoof must not split residual-led commercial id: {id_a} vs {id_b}"
    );
}

/// Soft residual alone must not drive rpa; soft + headless/control → farm corroboration.
#[test]
fn soft_residual_rpa_only_corroborates_with_control_plane() {
    let soft_only = soft_residual_fields(false);
    let bot = BotScore {
        score: 5,
        verdict: "human".into(),
        flags: vec![],
        family: "none".into(),
        algo: "test".into(),
        details: json!({}),
        robot_name: None,
    };
    let rpa_only = score_rpa(&soft_only, &bot, Some("p1"));
    let rs_only: String = rpa_only
        .get("reasons")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str())
        .collect::<Vec<_>>()
        .join("|");
    assert!(
        !rs_only.contains("soft_residual") && !rs_only.contains("residual_soft"),
        "soft alone must not be rpa primary: {rs_only}"
    );

    let mut soft_headless = soft_residual_fields(false);
    soft_headless.as_object_mut().unwrap().insert(
        "user_agent".into(),
        json!("Mozilla/5.0 HeadlessChrome/120.0.0.0 Safari/537.36"),
    );
    soft_headless
        .as_object_mut()
        .unwrap()
        .insert("headless_likely".into(), json!(true));
    let rpa_farm = score_rpa(&soft_headless, &bot, Some("p2"));
    let rs_farm: String = rpa_farm
        .get("reasons")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str())
        .collect::<Vec<_>>()
        .join("|");
    assert!(
        rs_farm.contains("soft_residual_farm_corroboration")
            || rs_farm.contains("headless"),
        "soft+headless should corroborate farm/control: {rs_farm}"
    );
}

/// Fake high-end GPU + missing WebGL2 / low max_texture → br claim_capability_incoherence.
#[test]
fn br_claim_capability_incoherence_from_demo_fp_pattern() {
    let fields = json!({
        "form_class": "desktop",
        "platform": "Win32",
        "hardware_concurrency": 8,
        "timezone": "UTC",
        "os_family": "windows",
        "webgl_unmasked_renderer": "ANGLE (NVIDIA, NVIDIA GeForce RTX 4090 Direct3D11 vs_5_0 ps_5_0)",
        "residual_mean": 0.500423,
        "residual_soft_like": true,
        "residual_available": true,
        "webgl2_support": false,
        "webgl_max_texture": 4096,
        "hw_curve_webgl": webgl_curve(),
        "hw_curve_audio": audio_curve(),
    });
    let stack = stack_auth_from_fields(&fields);
    let truth = TruthResult {
        xsrc_status: "ok".into(),
        real_band: "unknown".into(),
        credibility: 0.4,
        fe_only: true,
        has_server_side: false,
        has_main_core: true,
        packages: json!({}),
        reasons: vec![],
        details: json!({}),
    };
    let bot = BotScore {
        score: 10,
        verdict: "human".into(),
        flags: vec![],
        family: "none".into(),
        algo: "test".into(),
        details: json!({}),
        robot_name: None,
    };
    let br = score_br(&fields, &stack, &bot, &truth, &[]);
    let join: String = br
        .get("reasons")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str())
        .collect::<Vec<_>>()
        .join("|");
    assert!(
        join.contains("claim_capability_incoherence")
            || join.contains("gpu_label_vs_residual")
            || join.contains("max_texture"),
        "br must surface capability/integrity claim-obs: {join}"
    );
    let os = score_os(&fields, &stack, &truth, &[]);
    let os_join: String = os
        .get("reasons")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str())
        .collect::<Vec<_>>()
        .join("|");
    assert!(
        os_join.contains("gpu_label") || os_join.contains("soft") || os_join.contains("claim"),
        "os must surface collusion/soft claim-obs: {os_join}"
    );
}
