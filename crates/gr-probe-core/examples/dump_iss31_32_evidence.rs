//! Dump iss/31–32 landing evidence via shipped APIs.
use gr_probe_core::{
    association_ladder, build_frontier, evaluate_session, link_or_mint_pair, MemoryDeviceIndex,
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

    let rich = json!({
        "form_class":"desktop","platform":"Linux x86_64","hardware_concurrency":12,
        "timezone":"UTC","os_family":"linux","architecture":"x86",
        "webgl_unmasked_renderer":"NVIDIA GTX 1050 Ti","residual_mean":0.500181,
        "residual_soft_like":false,"hw_curve_audio":au,"hw_curve_webgl":w,
        "webrtc_host_ip_hash":"lan","unit_surface_id":"u1","unit_surface_algo":"gr_unit_v1",
        "unit_multiround_stable":true,"webgl2_support":true,"webgl_max_texture":16384,
        "server_client_ip":"203.0.113.10","server_asn":"AS1","server_country":"CN",
        "user_agent":"Mozilla/5.0 Chrome/120","chrome_runtime":true,
        "behavior_early_bound":true,"behavior_count":15,
        "behavior_events":[{"kind":"mousemove"},{"kind":"click"},{"kind":"scroll"}],
    });
    let soft = json!({
        "form_class":"desktop","platform":"Linux","hardware_concurrency":8,"timezone":"UTC",
        "os_family":"linux","residual_mean":0.500423,"residual_soft_like":true,"soft_stack":true,
        "stack_class":"soft_render","hw_curve_webgl":w,"hw_curve_audio":au,
        "unit_surface_id":"us","unit_surface_algo":"gr_unit_v1","unit_multiround_stable":true,
        "os_instance_hash":"os_vm0","webgl_unmasked_renderer":"SwiftShader",
    });
    let soft2 = {
        let mut s = soft.clone();
        s.as_object_mut().unwrap().insert("os_instance_hash".into(), json!("os_vm1"));
        s
    };
    let thin = json!({
        "form_class":"desktop","user_agent":"thin",
        "server_client_ip":"198.51.100.1","server_asn":"AS2","server_country":"US",
    });

    let ev = |fields: Value, sources: &[&str]| {
        let batches: Vec<Value> = sources
            .iter()
            .map(|s| {
                json!({
                    "batch_id": if *s=="gateway"{"B8_gateway"}else{"B10_hw_curves"},
                    "source": s
                })
            })
            .collect();
        json!({
            "sources": sources, "batches": batches, "has_gateway": sources.contains(&"gateway"),
            "fields": fields, "session_id":"e", "page_id":"p",
            "gateway_fields": {"server_client_ip":"203.0.113.10","server_asn":"AS1","server_country":"CN"},
        })
    };

    let rich_out = evaluate_session(&ev(rich.clone(), &["main", "gateway"]), None, None, None, true).unwrap();
    let soft_out = evaluate_session(&ev(soft.clone(), &["main"]), None, None, None, true).unwrap();
    let thin_out = evaluate_session(&ev(thin.clone(), &["gateway"]), None, None, None, true).unwrap();
    let rp = rich_out.get("product").cloned().unwrap_or(rich_out.clone());
    let sp = soft_out.get("product").cloned().unwrap_or(soft_out.clone());
    let tp = thin_out.get("product").cloned().unwrap_or(thin_out.clone());

    fs::write(
        out.join("association_ladder.json"),
        serde_json::to_string_pretty(&json!({
            "rich": {
                "level": rp.get("association_level"),
                "basis": rp.get("association_basis"),
                "ladder": rp.get("association_ladder"),
                "browser_surface_id": rp.get("browser_surface_id"),
                "device_id": rp.get("device_id"),
                "device_tier": rp.get("device_tier"),
            },
            "soft": {
                "level": sp.get("association_level"),
                "basis": sp.get("association_basis"),
                "claims_hardware": sp.pointer("/association_ladder/claims_host_silicon"),
            },
            "thin": {
                "level": tp.get("association_level"),
                "device_id": tp.get("device_id"),
            },
            "pure_ladder_api": {
                "rich": association_ladder(&rich, Some(&json!({"device_tier":"dh"}))),
                "soft": association_ladder(&soft, Some(&json!({"device_tier":"dv"}))),
                "thin": association_ladder(&thin, Some(&json!({"device_tier":"dg"}))),
            }
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
        out.join("task_critical_confidence.json"),
        serde_json::to_string_pretty(&json!({
            "thin": {
                "os": conf(&tp,"os"), "br": conf(&tp,"br"),
                "posture": tp.pointer("/os/confidence_posture"),
                "task_critical": tp.pointer("/os/confidence_report/task_critical_coverage"),
            },
            "rich": {
                "os": conf(&rp,"os"), "br": conf(&rp,"br"), "rpa": conf(&rp,"rpa"),
                "posture": rp.pointer("/os/confidence_posture"),
                "task_critical": rp.pointer("/os/confidence_report/task_critical_coverage"),
                "device_confidence": rp.get("device_confidence"),
            },
            "rich_higher": {
                "os": conf(&rp,"os").unwrap_or(0.0) > conf(&tp,"os").unwrap_or(0.0),
                "br": conf(&rp,"br").unwrap_or(0.0) > conf(&tp,"br").unwrap_or(0.0),
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let pair = link_or_mint_pair(&soft, &soft2);
    let mut idx = MemoryDeviceIndex::new();
    let m1 = idx.link_or_mint(&soft);
    let m2 = idx.link_or_mint(&soft2);
    fs::write(
        out.join("link_mint_collision.json"),
        serde_json::to_string_pretty(&json!({
            "pair": pair,
            "store": {"g0": m1, "g1": m2},
            "soft_product_collision_risk": sp.get("collision_risk"),
            "soft_uniqueness_marker": sp.get("uniqueness_marker"),
            "ids_distinct": pair.get("same_server_mint_id") == Some(&json!(false)),
        }))
        .unwrap(),
    )
    .unwrap();

    let soft_brain_fields = json!({
        "form_class":"desktop","residual_soft_like":true,"soft_stack":true,
        "stack_class":"soft_render","residual_mean":0.500423,
        "hw_curve_webgl":w,"hw_curve_audio":au,
        "webgl_unmasked_renderer":"GeForce GTX 980","spoof_score":0.55,
    });
    let plan = build_frontier(
        &json!({
            "sources":["main"],
            "batches":[{"batch_id":"B0_bootstrap","source":"main"}],
            "fields": soft_brain_fields,
            "session_id":"brain"
        }),
        true,
        None,
        24,
    )
    .unwrap();
    fs::write(
        out.join("ops_iss31_32_telemetry.json"),
        serde_json::to_string_pretty(&json!({
            "rich_product": {
                "association_level": rp.get("association_level"),
                "collision_risk": rp.get("collision_risk"),
                "mutual_verification": rp.get("mutual_verification"),
                "open_gaps": rp.get("open_verification_gaps"),
                "analysis_quality": rp.get("analysis_quality"),
                "product_redlines": rp.get("product_redlines"),
            },
            "soft_product": {
                "association_level": sp.get("association_level"),
                "collision_risk": sp.get("collision_risk"),
            },
            "brain_frontier": {
                "notes": plan.notes,
                "re_probe_codes": plan.coverage.get("re_probe_priority_codes"),
                "re_probe_elevated": plan.coverage.get("re_probe_elevated_packs"),
                "packs_head": plan.packs.iter().take(8).collect::<Vec<_>>(),
            }
        }))
        .unwrap(),
    )
    .unwrap();

    println!("wrote iss31-32 evidence to {}", out.display());
}
