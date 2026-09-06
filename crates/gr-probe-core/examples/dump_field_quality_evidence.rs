//! Dump field-quality wave evidence using shipped APIs only.
use gr_probe_core::{
    build_field_utilization_map, evaluate_session, open_verification_gaps, scan_gaps,
};
use serde_json::{json, Value};
use std::env;
use std::fs;
use std::path::PathBuf;

fn curves() -> (Vec<f64>, Vec<f64>) {
    let a: Vec<f64> = (0..32)
        .map(|i| (i as f64 * 0.11).sin().abs() * 0.5 + 0.05)
        .collect();
    let w: Vec<f64> = (0..16)
        .map(|i| ((i * 13) % 200) as f64 / 200.0 + 0.01)
        .collect();
    (a, w)
}

fn main() {
    let out = PathBuf::from(env::var("SCRATCH").unwrap_or_else(|_| ".".into()));
    let (au, w) = curves();

    let thin = json!({
        "fields": {
            "user_agent": "thin-bot",
            "form_class": "desktop",
            "server_client_ip": "198.51.100.7",
            "server_asn": "AS1",
            "server_country": "US",
        },
        "sources": ["gateway"],
        "batches": [{"batch_id":"B8_gateway","source":"gateway"}],
        "has_gateway": true,
        "gateway_fields": {"server_client_ip":"198.51.100.7","server_asn":"AS1","server_country":"US"},
        "session_id": "thin",
    });
    let rich = json!({
        "fields": {
            "form_class": "desktop", "platform": "Linux x86_64", "hardware_concurrency": 12,
            "timezone": "UTC", "os_family": "linux", "architecture": "x86",
            "webgl_unmasked_renderer": "NVIDIA GeForce GTX 1050 Ti",
            "residual_mean": 0.500181, "residual_soft_like": false, "residual_available": true,
            "hw_curve_audio": au, "hw_curve_webgl": w,
            "webrtc_host_ip_hash": "lan_rich",
            "unit_surface_id": "unit_r", "unit_surface_algo": "gr_unit_v1",
            "unit_multiround_stable": true, "multi_seed_n": 8,
            "webgl2_support": true, "webgl_max_texture": 16384,
            "behavior_early_bound": true, "behavior_count": 20,
            "behavior_events": [{"kind":"mousemove"},{"kind":"click"},{"kind":"scroll"}],
            "ja4": "t13d1516h2_demo", "chrome_runtime": true,
            "server_client_ip": "203.0.113.10", "server_asn": "AS2", "server_country": "CN",
            "user_agent": "Mozilla/5.0 Chrome/120.0.0.0",
        },
        "sources": ["main","gateway"],
        "batches": [
            {"batch_id":"B10_hw_curves","source":"main"},
            {"batch_id":"B8_gateway","source":"gateway"}
        ],
        "has_gateway": true,
        "gateway_fields": {"server_client_ip":"203.0.113.10","server_asn":"AS2","server_country":"CN"},
        "session_id": "rich", "page_id": "p1",
    });
    let claim = json!({
        "fields": {
            "form_class": "desktop", "platform": "Win32", "hardware_concurrency": 8,
            "timezone": "UTC", "os_family": "windows",
            "webgl_unmasked_renderer": "NVIDIA GeForce RTX 4090",
            "residual_mean": 0.500423, "residual_soft_like": true, "soft_stack": true,
            "webgl2_support": false, "webgl_max_texture": 4096,
            "hw_curve_webgl": w, "hw_curve_audio": au, "spoof_score": 0.5,
            "webdriver": true, "headless_likely": true,
        },
        "sources": ["main","gateway"],
        "batches": [
            {"batch_id":"B10_hw_curves","source":"main"},
            {"batch_id":"B8_gateway","source":"gateway"}
        ],
        "has_gateway": true,
        "gateway_fields": {"server_client_ip":"203.0.113.9","server_asn":"AS3","server_country":"US"},
        "session_id": "claim", "page_id": "p2",
    });

    let thin_out = evaluate_session(&thin, None, None, None, true).unwrap();
    let rich_out = evaluate_session(&rich, None, None, None, true).unwrap();
    let claim_out = evaluate_session(&claim, None, None, None, true).unwrap();
    let tp = thin_out.get("product").cloned().unwrap_or(thin_out.clone());
    let rp = rich_out.get("product").cloned().unwrap_or(rich_out.clone());
    let cp = claim_out.get("product").cloned().unwrap_or(claim_out.clone());

    let util = build_field_utilization_map(&rich["fields"]);
    fs::write(
        out.join("field_algorithm_utilization.json"),
        serde_json::to_string_pretty(&json!({
            "utilization_rich": util,
            "claim_product": {
                "device_id": cp.get("device_id"),
                "mutual_verification": cp.get("mutual_verification"),
                "os_reasons": cp.pointer("/os/reasons"),
                "br_reasons": cp.pointer("/br/reasons"),
                "rpa_reasons": cp.pointer("/rpa/reasons"),
                "analysis_quality": cp.get("analysis_quality"),
            },
            "never_digest_note": "L0/JA*/webdriver excluded from commercial digest still on os/br/rpa",
        }))
        .unwrap(),
    )
    .unwrap();

    let conf = |p: &Value, ax: &str| {
        p.pointer(&format!("/{ax}/confidence"))
            .or_else(|| p.pointer(&format!("/{ax}/confidence_report/confidence")))
            .and_then(|v| v.as_f64())
    };
    fs::write(
        out.join("confidence_completeness.json"),
        serde_json::to_string_pretty(&json!({
            "thin": {
                "os_conf": conf(&tp, "os"),
                "br_conf": conf(&tp, "br"),
                "device_confidence": tp.get("device_confidence"),
                "os_posture": tp.pointer("/os/confidence_posture"),
                "device_id": tp.get("device_id"),
            },
            "rich": {
                "os_conf": conf(&rp, "os"),
                "br_conf": conf(&rp, "br"),
                "rpa_conf": conf(&rp, "rpa"),
                "device_confidence": rp.get("device_confidence"),
                "os_posture": rp.pointer("/os/confidence_posture"),
                "device_confidence_posture": rp.get("device_confidence_posture"),
                "device_id": rp.get("device_id"),
            },
            "rich_higher_than_thin": {
                "os": conf(&rp, "os").unwrap_or(0.0) >= conf(&tp, "os").unwrap_or(0.0),
                "br": conf(&rp, "br").unwrap_or(0.0) >= conf(&tp, "br").unwrap_or(0.0),
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let soft_ev = json!({
        "sources": ["main"],
        "batches": [{"batch_id":"B0_bootstrap","source":"main"}],
        "fields": {
            "form_class": "desktop", "residual_soft_like": true, "soft_stack": true,
            "stack_class": "soft_render", "residual_mean": 0.500423,
            "hw_curve_webgl": w, "hw_curve_audio": au,
            "webgl_unmasked_renderer": "GeForce GTX 980", "spoof_score": 0.55
        },
        "session_id": "reprobe"
    });
    let gaps = scan_gaps(&soft_ev, None).unwrap();
    let open = open_verification_gaps(&soft_ev);
    fs::write(
        out.join("reprobe_task_targeted.json"),
        serde_json::to_string_pretty(&json!({
            "gaps": gaps.iter().map(|g| g.to_value()).collect::<Vec<_>>(),
            "open_verification_gaps": open,
            "gpu_low": gaps.iter().filter(|g| g.code.contains("gpu") && g.severity == "low")
                .map(|g| g.code.clone()).collect::<Vec<_>>(),
        }))
        .unwrap(),
    )
    .unwrap();

    fs::write(
        out.join("ops_field_quality_telemetry.json"),
        serde_json::to_string_pretty(&json!({
            "claim_ops": cp.get("ops_fusion_telemetry"),
            "claim_quality": cp.get("analysis_quality"),
            "claim_open_gaps": cp.get("open_verification_gaps"),
            "rich_ops": rp.get("ops_fusion_telemetry"),
            "rich_utilization_present": util.get("present_count"),
            "rich_missing_critical": util.get("missing_critical_count"),
        }))
        .unwrap(),
    )
    .unwrap();

    println!("wrote field quality evidence to {}", out.display());
}
