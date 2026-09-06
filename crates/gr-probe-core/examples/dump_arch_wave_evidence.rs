
//! Dump architecture-wave evidence artifacts (shipped APIs only).
use gr_probe_core::{
    apply_server_mint, commercial_projection, evaluate_session, field_algorithm_weights,
    link_or_mint_pair, load_field_product_matrix, scan_gaps, MemoryDeviceIndex,
    UNIT_SURFACE_ALGO_V1,
};
use serde_json::{json, Value};
use std::env;
use std::fs;
use std::path::PathBuf;

fn curves() -> (Vec<f64>, Vec<f64>) {
    let a: Vec<f64> = (0..32).map(|i| (i as f64 * 0.11).sin().abs() * 0.5 + 0.05).collect();
    let w: Vec<f64> = (0..16).map(|i| ((i * 13) % 200) as f64 / 200.0 + 0.01).collect();
    (a, w)
}

fn soft(os: &str, unit: &str) -> Value {
    let (a, w) = curves();
    json!({
        "form_class": "desktop", "platform": "Linux x86_64", "hardware_concurrency": 8,
        "timezone": "UTC", "os_family": "linux", "architecture": "x86",
        "webgl_unmasked_renderer": "SwiftShader",
        "residual_mean": 0.500423, "residual_soft_like": true, "soft_stack": true,
        "hw_curve_audio": a, "hw_curve_webgl": w,
        "unit_surface_id": unit, "unit_surface_algo": UNIT_SURFACE_ALGO_V1,
        "unit_multiround_stable": true, "multi_seed_n": 8,
        "os_instance_hash": os, "screen_width": 1280, "screen_height": 720,
    })
}

fn main() {
    let out = PathBuf::from(env::var("SCRATCH").unwrap_or_else(|_| ".".into()));
    let a = soft("os_vm_g0", "unit_soft_same");
    let b = soft("os_vm_g1", "unit_soft_same");
    let pair = link_or_mint_pair(&a, &b);
    let mut idx = MemoryDeviceIndex::new();
    let r1 = idx.link_or_mint(&a);
    let r2 = idx.link_or_mint(&b);
    let gecko_a = {
        let (au, w) = curves();
        json!({
            "form_class": "desktop", "platform": "Linux x86_64", "hardware_concurrency": 12,
            "timezone": "Asia/Shanghai", "os_family": "linux", "architecture": "x86",
            "user_agent": "Mozilla/5.0 Firefox/128.0",
            "webgl_unmasked_renderer": "NVIDIA GeForce GTX 1050 Ti",
            "residual_mean": 0.500181, "residual_soft_like": false,
            "hw_curve_audio": au, "hw_curve_webgl": w,
            "webrtc_host_ip_hash": "lan_real",
            "unit_surface_id": "unit_gecko", "unit_surface_algo": UNIT_SURFACE_ALGO_V1,
            "unit_multiround_stable": true, "multi_seed_n": 8,
            "screen_width": 1920, "screen_height": 1080,
        })
    };
    let mut gecko_b = gecko_a.clone();
    gecko_b.as_object_mut().unwrap().insert("webgl_unmasked_renderer".into(), json!("Apple M1 spoof"));
    let gecko_pair = link_or_mint_pair(&gecko_a, &gecko_b);
    let mint_a = apply_server_mint(&a);
    let mint_b = apply_server_mint(&b);
    let lom = json!({
        "soft_fork": pair,
        "store_mint_g0": r1,
        "store_mint_g1": r2,
        "gecko_l0_link": gecko_pair,
        "server_mint_ids": {"g0": mint_a["device_id"], "g1": mint_b["device_id"]},
        "ids_distinct_soft_os": mint_a["device_id"] != mint_b["device_id"],
        "gecko_same_id": gecko_pair["same_server_mint_id"],
    });
    fs::write(out.join("link_or_mint_server_mint.json"), serde_json::to_string_pretty(&lom).unwrap()).unwrap();

    // multi-axis
    let m = load_field_product_matrix().unwrap();
    let mut roles = serde_json::Map::new();
    for f in ["residual_soft_like", "unit_surface_id", "webgl_unmasked_renderer", "webdriver", "ja4", "ja3", "os_instance_hash"] {
        if let Some(row) = m.by_field.get(f) {
            roles.insert(f.into(), json!({"axes": row.axes, "trust_tier": row.trust_tier}));
        }
    }
    let mut exotic = soft("os_claim", "unit_claim");
    exotic.as_object_mut().unwrap().insert("webgl_unmasked_renderer".into(), json!("GeForce RTX 4090"));
    exotic.as_object_mut().unwrap().insert("webgl2_support".into(), json!(false));
    exotic.as_object_mut().unwrap().insert("webgl_max_texture".into(), json!(4096));
    let ev = json!({
        "sources": ["main","gateway"],
        "batches": [{"batch_id":"B10_hw_curves","source":"main"},{"batch_id":"B8_gateway","source":"gateway"}],
        "has_gateway": true, "fields": exotic.clone(),
        "gateway_fields": {"server_client_ip":"203.0.113.10","server_asn":"AS1","server_country":"US"}
    });
    let eval = evaluate_session(&ev, None, None, None, true).unwrap();
    let product = eval.get("product").cloned().unwrap_or(eval.clone());
    let multi = json!({
        "field_roles": roles,
        "algorithm_weights": field_algorithm_weights(),
        "never_digest": m.commercial_device_id.never_digest,
        "claim_obs_product": {
            "device_id": product.get("device_id"),
            "os_reasons": product.pointer("/os/reasons"),
            "br_reasons": product.pointer("/br/reasons"),
            "ops": product.get("ops_fusion_telemetry"),
        },
        "server_mint_id_stable_under_l0": apply_server_mint(&soft("os_claim","unit_claim"))["device_id"]
            == apply_server_mint(&exotic)["device_id"],
    });
    fs::write(out.join("multi_axis_weights_claim_obs.json"), serde_json::to_string_pretty(&multi).unwrap()).unwrap();

    // brain
    let soft_ev = json!({
        "sources": ["main"],
        "batches": [{"batch_id":"B0_bootstrap","source":"main"}],
        "fields": {
            "form_class": "desktop", "residual_soft_like": true, "soft_stack": true,
            "stack_class": "soft_render", "residual_mean": 0.500423,
            "hw_curve_webgl": curves().1, "hw_curve_audio": curves().0,
            "webgl_unmasked_renderer": "GeForce GTX 980", "spoof_score": 0.55
        },
        "session_id": "brain_soft"
    });
    let gaps = scan_gaps(&soft_ev, None).unwrap();
    let gap_json: Vec<Value> = gaps.iter().map(|g| g.to_value()).collect();
    let brain = json!({
        "gaps": gap_json,
        "task_oriented_codes": gaps.iter().map(|g| &g.code).filter(|c| {
            c.contains("unit") || c.contains("soft_stack") || c.contains("claim_obs")
                || c.contains("os_instance") || c.contains("rpa") || c.contains("br_capability")
                || c.contains("device") || c.contains("os") || c.contains("br")
        }).collect::<Vec<_>>(),
        "gpu_dig_severities": gaps.iter().filter(|g| g.code.contains("gpu")).map(|g| json!({
            "code": g.code, "severity": g.severity
        })).collect::<Vec<_>>(),
    });
    fs::write(out.join("brain_task_targeted_route.json"), serde_json::to_string_pretty(&brain).unwrap()).unwrap();

    // unit versioned
    let ver = {
        let (au, w) = curves();
        json!({
            "form_class": "desktop", "platform": "Linux x86_64", "hardware_concurrency": 12,
            "timezone": "UTC", "os_family": "linux", "architecture": "x86",
            "residual_mean": 0.500181, "residual_soft_like": false,
            "hw_curve_audio": au, "hw_curve_webgl": w,
            "unit_surface_id": "unit_v1", "unit_surface_algo": UNIT_SURFACE_ALGO_V1,
            "unit_multiround_stable": true, "webrtc_host_ip_hash": "lan",
        })
    };
    let mut unver = ver.clone();
    unver.as_object_mut().unwrap().insert("unit_surface_algo".into(), json!("old"));
    unver.as_object_mut().unwrap().insert("unit_multiround_stable".into(), json!(false));
    let unit_ev = json!({
        "versioned_projection": commercial_projection(&ver),
        "unversioned_projection": commercial_projection(&unver),
        "versioned_mint": apply_server_mint(&ver),
        "unversioned_mint": apply_server_mint(&unver),
    });
    fs::write(out.join("unit_surface_versioned_digest.json"), serde_json::to_string_pretty(&unit_ev).unwrap()).unwrap();

    // ops telemetry from evaluate
    let ops = product.get("ops_fusion_telemetry").cloned().unwrap_or(json!(null));
    let ops_full = json!({
        "ops_fusion_telemetry": ops,
        "product_collision_risk": product.get("collision_risk"),
        "product_server_mint": product.get("server_mint"),
        "product_uniqueness_marker": product.get("uniqueness_marker"),
        "sub_algorithms": product.get("sub_algorithms"),
        "diagnostics_ops": eval.pointer("/diagnostics/ops_fusion_telemetry"),
    });
    fs::write(out.join("ops_fusion_telemetry.json"), serde_json::to_string_pretty(&ops_full).unwrap()).unwrap();
    println!("wrote evidence to {}", out.display());
}
