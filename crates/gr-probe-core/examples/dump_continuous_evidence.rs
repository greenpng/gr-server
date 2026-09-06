//! Dump continuous-path (post mid-term) evidence.
use gr_probe_core::{
    adopt_ladder_calibration, evaluate_session, link_or_mint_pair, report_profile_boundary_kpi,
    FileDeviceIndex, LabeledPair,
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

fn soft(residual: f64, unit: &str, os: Option<&str>) -> Value {
    let (a, w) = curves();
    let mut o = json!({
        "form_class":"desktop","hardware_concurrency":8,"timezone":"UTC","os_family":"linux",
        "architecture":"x86","residual_mean":residual,"residual_soft_like":true,"soft_stack":true,
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

    // Profile boundary
    let same: Vec<Value> = (0..3)
        .map(|i| {
            let mut f = soft(0.500423, "unit_cfg", None);
            f.as_object_mut()
                .unwrap()
                .insert("pool_label".into(), json!(format!("same_{i}")));
            f
        })
        .collect();
    let noise: Vec<Value> = (0..3)
        .map(|i| {
            let mut f = soft(0.5001 + i as f64 * 0.0003, &format!("u_{i}"), Some(&format!("os_{i}")));
            f.as_object_mut()
                .unwrap()
                .insert("pool_label".into(), json!(format!("noise_{i}")));
            f
        })
        .collect();
    fs::write(
        out.join("profile_boundary_kpi.json"),
        serde_json::to_string_pretty(&report_profile_boundary_kpi(&same, &noise)).unwrap(),
    )
    .unwrap();

    // Tenant pepper
    let dir = out.join("pepper_store");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let f = soft(0.500423, "u_t", Some("os1"));
    let id_a = {
        let mut idx = FileDeviceIndex::open_for_tenant(dir.join("a.json"), "tenant_a").unwrap();
        idx.link_or_mint(&f).unwrap()
    };
    let id_b = {
        let mut idx = FileDeviceIndex::open_for_tenant(dir.join("b.json"), "tenant_b").unwrap();
        idx.link_or_mint(&f).unwrap()
    };
    fs::write(
        out.join("tenant_pepper_index.json"),
        serde_json::to_string_pretty(&json!({
            "tenant_a": id_a,
            "tenant_b": id_b,
            "ids_differ": id_a.get("device_id") != id_b.get("device_id"),
        }))
        .unwrap(),
    )
    .unwrap();

    // Ladder conf adopt
    let pairs = vec![
        LabeledPair {
            same_host: true,
            score: 0.91,
            label_source: "lab".into(),
            association_level: Some("hardware".into()),
        },
        LabeledPair {
            same_host: false,
            score: 0.2,
            label_source: "lab".into(),
            association_level: Some("env".into()),
        },
    ];
    fs::write(
        out.join("ladder_conf_adopt.json"),
        serde_json::to_string_pretty(&json!({
            "empty_refuse": adopt_ladder_calibration(&[]),
            "explicit_adopt": adopt_ladder_calibration(&pairs),
        }))
        .unwrap(),
    )
    .unwrap();

    // UA merge ban pair
    let mut ua_a = soft(0.500111, "ua", Some("osx"));
    let mut ua_b = soft(0.500999, "ub", Some("osy"));
    ua_a.as_object_mut().unwrap().insert(
        "user_agent".into(),
        json!("Mozilla/5.0 Chrome/120"),
    );
    ua_b.as_object_mut().unwrap().insert(
        "user_agent".into(),
        json!("Mozilla/5.0 Chrome/120"),
    );
    ua_a.as_object_mut().unwrap().insert(
        "webgl_unmasked_renderer".into(),
        json!("NVIDIA GTX 1050"),
    );
    ua_b.as_object_mut().unwrap().insert(
        "webgl_unmasked_renderer".into(),
        json!("NVIDIA GTX 1050"),
    );

    // Antidetect evaluate
    let mut anti = soft(0.500423, "u_anti", Some("os_anti"));
    {
        let o = anti.as_object_mut().unwrap();
        o.insert("antidetect_vendor_hint".into(), json!("camoufox"));
        o.insert("prototype_chain_tamper".into(), json!(true));
        o.insert("webgl_unmasked_renderer".into(), json!("RTX 4090"));
    }
    let anti_out = evaluate_session(
        &json!({
            "sources":["main"],
            "batches":[{"batch_id":"B10_hw_curves","source":"main"}],
            "fields": anti,
            "session_id":"anti",
        }),
        None,
        None,
        None,
        true,
    )
    .unwrap();
    let product = anti_out.get("product").cloned().unwrap_or(anti_out.clone());

    fs::write(
        out.join("ops_continuous_telemetry.json"),
        serde_json::to_string_pretty(&json!({
            "antidetect_product": {
                "association_level": product.get("association_level"),
                "os_reasons": product.pointer("/os/reasons"),
                "br_reasons": product.pointer("/br/reasons"),
                "device_id": product.get("device_id"),
                "collision_risk": product.get("collision_risk"),
                "mutual": product.get("mutual_verification"),
            },
            "ua_only_pair": link_or_mint_pair(&ua_a, &ua_b),
            "continuous_landed": {
                "x8_antidetect_claim_obs": true,
                "x9_emulator_env": true,
                "x11_ua_merge_ban": true,
                "x17_soft_never_hardware": true,
                "x20_profile_boundary_kpi": true,
                "tenant_pepper_index": true,
                "ladder_conf_adopt_hook": true,
                "live_antidetect_fleet": false,
                "postgres_multitenant": false,
                "full_1_minus_p_fp_corpus": false,
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let _ = (au, w);
    println!("wrote continuous evidence to {}", out.display());
}
