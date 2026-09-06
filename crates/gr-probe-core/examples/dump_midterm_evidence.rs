//! Dump mid-term (X-12/13/14) evidence via shipped APIs.
use gr_probe_core::{
    association_ladder, build_frontier, calibrate_offline_by_ladder, evaluate_session,
    report_collision_kpi, FileDeviceIndex, LabeledPair,
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

fn soft(label: &str, unit: &str, os: Option<&str>) -> Value {
    let (a, w) = curves();
    let mut o = json!({
        "pool_label": label, "form_class":"desktop","hardware_concurrency":8,
        "timezone":"UTC","os_family":"linux","architecture":"x86",
        "residual_mean":0.500423,"residual_soft_like":true,"soft_stack":true,
        "hw_curve_audio":a,"hw_curve_webgl":w,"unit_surface_id":unit,
        "unit_surface_algo":"gr_unit_v1","unit_multiround_stable":true,
        "webgl_unmasked_renderer":"SwiftShader",
    });
    if let Some(h) = os {
        o.as_object_mut().unwrap().insert("os_instance_hash".into(), json!(h));
    }
    o
}

fn main() {
    let out = PathBuf::from(env::var("SCRATCH").unwrap_or_else(|_| ".".into()));
    let (au, w) = curves();

    // Collision KPI
    let no_sep: Vec<Value> = (0..4).map(|i| soft(&format!("n{i}"), "unit_a", None)).collect();
    let with_sep: Vec<Value> = (0..4)
        .map(|i| soft(&format!("s{i}"), "unit_a", Some(&format!("os_{i}"))))
        .collect();
    fs::write(
        out.join("collision_kpi.json"),
        serde_json::to_string_pretty(&json!({
            "no_separator_pool": report_collision_kpi(&no_sep),
            "with_separator_pool": report_collision_kpi(&with_sep),
        }))
        .unwrap(),
    )
    .unwrap();

    // Durable index
    let dir = out.join("device_index_store");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("index.json");
    let a = soft("a", "u", Some("os_same"));
    let b = soft("b", "u", Some("os_same"));
    let c = soft("c", "u", Some("os_diff"));
    let r1 = {
        let mut idx = FileDeviceIndex::open(&path).unwrap();
        let r = idx.link_or_mint(&a).unwrap();
        let _ = idx.link_or_mint(&b).unwrap();
        r
    };
    let (r_link, r_fork, count) = {
        let mut idx = FileDeviceIndex::open(&path).unwrap();
        let n = idx.device_count();
        let rl = idx.link_or_mint(&b).unwrap();
        let rf = idx.link_or_mint(&c).unwrap();
        (rl, rf, n)
    };
    fs::write(
        out.join("durable_device_index.json"),
        serde_json::to_string_pretty(&json!({
            "store_path": path.display().to_string(),
            "first_mint": r1,
            "reopen_device_count": count,
            "reopen_link_same_os": r_link,
            "reopen_mint_diff_os": r_fork,
            "ids_distinct_on_os_fork": r_link.get("device_id") != r_fork.get("device_id"),
        }))
        .unwrap(),
    )
    .unwrap();

    // Ladder conf cal
    let pairs = vec![
        LabeledPair {
            same_host: true,
            score: 0.9,
            label_source: "lab".into(),
            association_level: Some("hardware".into()),
        },
        LabeledPair {
            same_host: false,
            score: 0.2,
            label_source: "lab".into(),
            association_level: Some("env".into()),
        },
        LabeledPair {
            same_host: true,
            score: 0.75,
            label_source: "lab".into(),
            association_level: Some("env".into()),
        },
        LabeledPair {
            same_host: false,
            score: 0.1,
            label_source: "lab".into(),
            association_level: Some("gateway".into()),
        },
    ];
    fs::write(
        out.join("ladder_conf_cal.json"),
        serde_json::to_string_pretty(&json!({
            "empty": calibrate_offline_by_ladder(&[]),
            "labeled": calibrate_offline_by_ladder(&pairs),
        }))
        .unwrap(),
    )
    .unwrap();

    // Ops telemetry
    let rich = json!({
        "form_class":"desktop","platform":"Linux","hardware_concurrency":12,
        "timezone":"UTC","os_family":"linux","architecture":"x86",
        "residual_mean":0.500181,"residual_soft_like":false,
        "hw_curve_audio":au,"hw_curve_webgl":w,
        "webgl_unmasked_renderer":"NVIDIA","webrtc_host_ip_hash":"lan",
        "server_client_ip":"203.0.113.1","server_asn":"AS1","server_country":"US",
        "user_agent":"Chrome",
    });
    let soft_f = soft("ops", "u", Some("os1"));
    let rich_out = evaluate_session(
        &json!({
            "sources":["main","gateway"],
            "batches":[
                {"batch_id":"B10_hw_curves","source":"main"},
                {"batch_id":"B8_gateway","source":"gateway"}
            ],
            "has_gateway":true,
            "fields": rich,
            "gateway_fields":{"server_client_ip":"203.0.113.1","server_asn":"AS1","server_country":"US"},
            "session_id":"r",
        }),
        None,
        None,
        None,
        true,
    )
    .unwrap();
    let soft_out = evaluate_session(
        &json!({
            "sources":["main"],
            "batches":[{"batch_id":"B10_hw_curves","source":"main"}],
            "fields": soft_f,
            "session_id":"s",
        }),
        None,
        None,
        None,
        true,
    )
    .unwrap();
    let rp = rich_out.get("product").cloned().unwrap_or(rich_out.clone());
    let sp = soft_out.get("product").cloned().unwrap_or(soft_out.clone());
    let plan = build_frontier(
        &json!({
            "sources":["main"],
            "batches":[{"batch_id":"B0_bootstrap","source":"main"}],
            "fields": {
                "form_class":"desktop","residual_soft_like":true,"soft_stack":true,
                "residual_mean":0.500423,"hw_curve_webgl":w,"hw_curve_audio":au,
                "webgl_unmasked_renderer":"GeForce","spoof_score":0.5
            },
            "session_id":"b"
        }),
        true,
        None,
        20,
    )
    .unwrap();

    fs::write(
        out.join("ops_midterm_telemetry.json"),
        serde_json::to_string_pretty(&json!({
            "rich": {
                "association_level": rp.get("association_level"),
                "collision_risk": rp.get("collision_risk"),
                "collision_posture": rp.get("collision_posture"),
                "product_redlines": rp.get("product_redlines"),
            },
            "soft": {
                "association_level": sp.get("association_level"),
                "collision_risk": sp.get("collision_risk"),
                "collision_posture": sp.get("collision_posture"),
            },
            "brain_re_probe_elevated": plan.coverage.get("re_probe_elevated_packs"),
            "pure_ladder_soft": association_ladder(&soft("x","u",Some("os")), Some(&json!({"device_tier":"dv"}))),
            "midterm_status": {
                "x12_collision_kpi": true,
                "x13_file_device_index": true,
                "x14_ladder_conf_cal": true,
                "postgres_multitenant": false,
                "full_1_minus_p_fp_corpus": false,
            }
        }))
        .unwrap(),
    )
    .unwrap();

    println!("wrote midterm evidence to {}", out.display());
}
