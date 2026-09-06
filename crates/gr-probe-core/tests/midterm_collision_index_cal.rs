//! Mid-term iss/32: collision KPI, durable FileDeviceIndex, ladder conf cal.

use gr_probe_core::{
    association_ladder, calibrate_offline_by_ladder, commercial_projection, evaluate_session,
    report_collision_kpi, FileDeviceIndex, LabeledPair, RUNTIME_CONFIDENCE_VERSION,
};
use serde_json::{json, Value};

fn curves() -> (Vec<f64>, Vec<f64>) {
    let a: Vec<f64> = (0..32)
        .map(|i| (i as f64 * 0.11).sin().abs() * 0.5 + 0.05)
        .collect();
    let w: Vec<f64> = (0..16)
        .map(|i| ((i * 13) % 200) as f64 / 200.0 + 0.01)
        .collect();
    (a, w)
}

fn soft_obs(label: &str, residual: f64, unit: &str, os: Option<&str>) -> Value {
    let (a, w) = curves();
    let mut o = json!({
        "pool_label": label,
        "form_class": "desktop",
        "platform": "Linux",
        "hardware_concurrency": 8,
        "timezone": "UTC",
        "os_family": "linux",
        "architecture": "x86",
        "webgl_unmasked_renderer": "SwiftShader",
        "residual_mean": residual,
        "residual_soft_like": true,
        "soft_stack": true,
        "stack_class": "soft_render",
        "hw_curve_audio": a,
        "hw_curve_webgl": w,
        "unit_surface_id": unit,
        "unit_surface_algo": "gr_unit_v1",
        "unit_multiround_stable": true,
        "screen_width": 1280,
        "screen_height": 720,
    });
    if let Some(os_h) = os {
        o.as_object_mut()
            .unwrap()
            .insert("os_instance_hash".into(), json!(os_h));
    }
    o
}

/// Same-SKU soft pool without separator collides; with separators splits.
#[test]
fn collision_kpi_same_sku_vs_separator_pools() {
    // Same residual/unit, no separator → class collision
    let no_sep: Vec<Value> = (0..4)
        .map(|i| soft_obs(&format!("n{i}"), 0.500423, "unit_sku_a", None))
        .collect();
    let kpi_ns = report_collision_kpi(&no_sep);
    assert_eq!(kpi_ns["ok"], true);
    assert_eq!(kpi_ns["sla"]["global_unique_promised"], false);
    assert!(
        kpi_ns["soft_no_separator_pool"]["class_collides"]
            .as_bool()
            .unwrap_or(false)
            || kpi_ns["pairwise"]["collision_rate"].as_f64().unwrap_or(0.0) >= 0.9,
        "no-sep pool should collide: {kpi_ns}"
    );
    assert!(
        kpi_ns["unique_commercial_ids"].as_u64().unwrap_or(99) <= 1,
        "same soft class no sep → one mint id: {kpi_ns}"
    );

    // Distinct OS separators → split
    let with_sep: Vec<Value> = (0..4)
        .map(|i| soft_obs(&format!("s{i}"), 0.500423, "unit_sku_a", Some(&format!("os_{i}"))))
        .collect();
    let kpi_sep = report_collision_kpi(&with_sep);
    assert_eq!(kpi_sep["ok"], true);
    assert!(
        kpi_sep["soft_with_separator_pool"]["split_ok"]
            .as_bool()
            .unwrap_or(false),
        "separator pool should split: {kpi_sep}"
    );
    assert!(
        kpi_sep["unique_commercial_ids"].as_u64().unwrap_or(0) >= 3,
        "4 distinct OS → nearly 4 ids: {kpi_sep}"
    );
    assert!(
        kpi_sep["pairwise"]["split_rate"].as_f64().unwrap_or(0.0) >= 0.9,
        "split rate high: {kpi_sep}"
    );
}

/// FileDeviceIndex persists across reopen: same OS links; different OS mints.
#[test]
fn durable_file_device_index_reopen_link_fork() {
    let dir = std::env::temp_dir().join(format!(
        "gr_file_idx_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("device_index.json");

    let a = soft_obs("a", 0.500423, "unit_d", Some("os_same"));
    let b_same = soft_obs("b", 0.500423, "unit_d", Some("os_same"));
    let c_diff = soft_obs("c", 0.500423, "unit_d", Some("os_other"));

    {
        let mut idx = FileDeviceIndex::open(&path).expect("open1");
        let r1 = idx.link_or_mint(&a).expect("mint a");
        assert!(r1["action"].as_str().unwrap().starts_with("mint") || r1["action"] == "link");
        assert_eq!(r1["durable"], true);
        let id1 = r1["device_id"].as_str().unwrap().to_string();
        let r2 = idx.link_or_mint(&b_same).expect("link b");
        // identity_resolver_fs_v2 decision vocabulary: "link" (fold) | "link_fs"
        // (file-index fold with score/posture). Both are link decisions.
        assert!(
            r2["action"].as_str().is_some_and(|a| a == "link" || a == "link_fs"),
            "same soft unit+OS should link: {r2}"
        );
        assert_eq!(r2["device_id"].as_str().unwrap(), id1);
        assert!(idx.device_count() >= 1);
    }

    // Reopen — new process simulation
    {
        let mut idx2 = FileDeviceIndex::open(&path).expect("open2");
        assert!(
            idx2.device_count() >= 1,
            "reopen must load devices from disk"
        );
        let r3 = idx2.link_or_mint(&b_same).expect("relink");
        assert!(
            r3["action"].as_str().is_some_and(|a| a == "link" || a == "link_fs"),
            "reopen link same OS: {r3}"
        );
        let r4 = idx2.link_or_mint(&c_diff).expect("fork");
        assert!(
            r4["action"].as_str().unwrap().starts_with("mint"),
            "different OS must mint: {r4}"
        );
        assert_ne!(
            r3["device_id"].as_str(),
            r4["device_id"].as_str(),
            "OS fork distinct ids"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// Ladder conf cal: labeled pairs with association_level → strata; empty → refuse.
#[test]
fn ladder_conf_cal_stratified_and_empty_refuse() {
    let empty = calibrate_offline_by_ladder(&[]);
    assert_eq!(empty["ok"], false);
    assert_eq!(empty["confidence_version"], RUNTIME_CONFIDENCE_VERSION);
    assert_eq!(empty["silent_relabel_forbidden"], true);

    let pairs = vec![
        LabeledPair {
            same_host: true,
            score: 0.92,
            label_source: "lab".into(),
            association_level: Some("hardware".into()),
        },
        LabeledPair {
            same_host: true,
            score: 0.88,
            label_source: "lab".into(),
            association_level: Some("hardware".into()),
        },
        LabeledPair {
            same_host: false,
            score: 0.25,
            label_source: "lab".into(),
            association_level: Some("env".into()),
        },
        LabeledPair {
            same_host: true,
            score: 0.70,
            label_source: "lab".into(),
            association_level: Some("env".into()),
        },
        LabeledPair {
            same_host: false,
            score: 0.10,
            label_source: "lab".into(),
            association_level: Some("gateway".into()),
        },
    ];
    let r = calibrate_offline_by_ladder(&pairs);
    assert_eq!(r["ok"], true);
    assert_eq!(r["algo"], "ladder_conf_cal_v1");
    let strata = r["ladder_strata"].as_object().expect("strata");
    assert!(strata.contains_key("hardware"));
    assert!(strata.contains_key("env"));
    assert!(r["overall"]["ok"].as_bool().unwrap_or(false));
    assert_eq!(r["runtime_default_remains"], RUNTIME_CONFIDENCE_VERSION);
}

/// Continuous hooks still hold: soft≠hardware; residual-led L0 stable; product collision_posture.
#[test]
fn continuous_hooks_soft_never_hardware_and_product_surfaces() {
    let soft = soft_obs("s", 0.500423, "u", Some("os1"));
    let lad = association_ladder(&soft, Some(&json!({"device_tier":"dv"})));
    assert_ne!(lad["association_level"], "hardware");
    assert_eq!(lad["redlines"]["soft_never_hardware_level"], true);

    let (a, w) = curves();
    let rich = json!({
        "form_class":"desktop","platform":"Linux","hardware_concurrency":12,
        "timezone":"UTC","os_family":"linux","architecture":"x86",
        "residual_mean":0.500181,"residual_soft_like":false,
        "hw_curve_audio":a,"hw_curve_webgl":w,
        "webgl_unmasked_renderer":"NVIDIA GTX 1050 Ti",
        "webrtc_host_ip_hash":"lan",
        "server_client_ip":"203.0.113.1","server_asn":"AS1","server_country":"US",
    });
    let mut spoof = rich.clone();
    spoof.as_object_mut().unwrap().insert(
        "webgl_unmasked_renderer".into(),
        json!("Apple M1"),
    );
    let id_a = commercial_projection(&rich)["device_id"].as_str().unwrap().to_string();
    let id_b = commercial_projection(&spoof)["device_id"].as_str().unwrap().to_string();
    assert_eq!(id_a, id_b, "L0 flip must not redefine residual-led id");

    let out = evaluate_session(
        &json!({
            "sources":["main"],
            "batches":[{"batch_id":"B10_hw_curves","source":"main"}],
            "fields": soft,
            "session_id":"m",
        }),
        None,
        None,
        None,
        true,
    )
    .expect("eval");
    let product = out.get("product").cloned().unwrap_or(out.clone());
    assert!(product.get("collision_posture").is_some() || product.get("collision_risk").is_some());
    assert_eq!(
        product
            .pointer("/product_redlines/no_pure_fe_global_uv_promise")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
}
