//! iss/38 N1–N6 closure: ops routes (pure), rule weight, KPI threshold, index contract, brand/cal.

use gr_probe_core::{
    brand_to_engine_family, calibrate_offline, commercial_projection, FileDeviceIndex,
    pairs_from_json, report_collision_kpi, score_br, score_rpa, stack_auth_from_fields,
    BotScore, TruthResult,
};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

fn truth() -> TruthResult {
    TruthResult {
        xsrc_status: "ok".into(),
        real_band: "likely_real".into(),
        credibility: 0.5,
        fe_only: true,
        has_server_side: true,
        has_main_core: true,
        packages: json!({}),
        reasons: vec![],
        details: json!({}),
    }
}

fn bot() -> BotScore {
    BotScore {
        score: 10,
        verdict: "human".into(),
        flags: vec![],
        family: "none".into(),
        algo: "t".into(),
        details: json!({}),
        robot_name: None,
    }
}

fn reasons(v: &Value) -> Vec<String> {
    v.get("reasons")
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(|s| s.to_string()))
        .collect()
}

fn spec_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.join("spec")
}

#[test]
fn n3_rule_samples_apply_low_weight_on_axes() {
    let fields = json!({
        "antidetect_vendor_hint": true,
        "webdriver": true,
        "form_class": "desktop",
        "hardware_concurrency": 4,
        "user_agent": "Mozilla/5.0 Chrome/120",
    });
    let stack = stack_auth_from_fields(&fields);
    let br = score_br(&fields, &stack, &bot(), &truth(), &[]);
    let rpa = score_rpa(&fields, &bot(), Some("p1"));
    let rs_br = reasons(&br);
    let rs_rpa = reasons(&rpa);
    assert!(
        rs_br.iter().any(|r| r.contains("rule_sample")),
        "br should consume rule samples: {rs_br:?}"
    );
    assert!(
        rs_rpa.iter().any(|r| r.contains("rule_sample")),
        "rpa should consume rule samples: {rs_rpa:?}"
    );
    // Digest untouched
    let blob = commercial_projection(&fields).to_string();
    assert!(!blob.contains("rule_sample"));
}

#[test]
fn n4_fleet_collision_kpi_hard_thresholds() {
    // Fixed same-SKU soft pool without separator → expect class collide
    let base = json!({
        "form_class": "desktop",
        "residual_soft_like": true,
        "soft_stack": true,
        "unit_surface_id": "u_same_sku",
        "unit_surface_algo": "gr_unit_v1",
        "hw_curve_webgl": [0.1, 0.2, 0.3, 0.4],
        "hw_curve_audio": [0.11, 0.21, 0.31, 0.41],
        "hardware_concurrency": 8,
        "timezone": "UTC",
        "pool_label": "same_sku_a",
    });
    let mut pool = Vec::new();
    for i in 0..4 {
        let mut o = base.clone();
        if let Some(obj) = o.as_object_mut() {
            obj.insert("pool_label".into(), json!(format!("same_sku_{i}")));
            // no os_instance / webrtc separator
        }
        pool.push(o);
    }
    // Separated soft clones should split
    for i in 0..2 {
        let mut o = base.clone();
        if let Some(obj) = o.as_object_mut() {
            obj.insert("pool_label".into(), json!(format!("sep_{i}")));
            obj.insert("os_instance_hash".into(), json!(format!("os_inst_{i}")));
            obj.insert("webrtc_host_ip_hash".into(), json!(format!("rtc_{i}")));
        }
        pool.push(o);
    }
    let kpi = report_collision_kpi(&pool);
    assert_eq!(kpi["ok"], true, "{kpi}");
    let soft = &kpi["soft_no_separator_pool"];
    assert!(
        soft["n"].as_u64().unwrap_or(0) >= 2,
        "soft pool present: {soft}"
    );
    // Hard threshold: same soft class without separator must class_collide
    assert_eq!(
        soft["class_collides"], true,
        "same-SKU soft pool must collide without separator: {kpi}"
    );
    let rate = kpi["pairwise"]["collision_rate"].as_f64().unwrap_or(-1.0);
    assert!(
        (0.0..=1.0).contains(&rate),
        "collision_rate in [0,1]: {rate}"
    );
}

#[test]
fn n5_file_device_index_matches_store_contract() {
    let contract: Value = serde_json::from_str(
        &fs::read_to_string(spec_dir().join("device_index_store_contract.json")).unwrap(),
    )
    .unwrap();
    let required = contract["file_device_index_v1"]["required_top_keys"]
        .as_array()
        .unwrap();
    let dir = std::env::temp_dir().join(format!("gr_idx_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("device_index.json");
    {
        let mut idx = FileDeviceIndex::open_for_tenant(&path, "tenant_lab").unwrap();
        let fields = json!({
            "form_class": "desktop",
            "residual_soft_like": true,
            "unit_surface_id": "u_contract",
            "hw_curve_webgl": [0.2, 0.3],
            "hw_curve_audio": [0.2, 0.3],
            "hardware_concurrency": 4,
            "timezone": "UTC",
        });
        idx.link_or_mint(&fields).unwrap();
    }
    let raw: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    for k in required {
        let key = k.as_str().unwrap();
        assert!(raw.get(key).is_some(), "missing contract key {key} in {raw}");
    }
    assert_eq!(raw["version"], "file_device_index_v1");
    assert_eq!(raw["tenant_id"], "tenant_lab");
    // Contract advanced: file primary + sqlite skeleton (+ optional PG lab path).
    let status = contract["store_alignment"]["status"]
        .as_str()
        .unwrap_or("");
    assert!(
        status.starts_with("lab_file_primary"),
        "store_alignment.status must stay lab_file_primary*: got {status}"
    );
    assert_eq!(
        contract["store_alignment"]["pg_multi_tenant"],
        false,
        "production multi-tenant claim still false unless ops-certified"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn n6_brand_unknown_default_and_conf_cal_ingest() {
    assert_eq!(brand_to_engine_family(""), "unknown");
    assert_eq!(brand_to_engine_family("TotallyFakeBrowserXyz"), "unknown");
    assert_eq!(brand_to_engine_family("chrome"), "blink");
    assert_eq!(brand_to_engine_family("firefox"), "gecko");

    let pairs_path = spec_dir().join("conf_cal_labeled_pairs_v1.json");
    let doc: Value = serde_json::from_str(&fs::read_to_string(pairs_path).unwrap()).unwrap();
    let pairs = pairs_from_json(&doc["pairs"]);
    assert!(pairs.len() >= 4, "labeled pairs loaded");
    let report = calibrate_offline(&pairs);
    assert_eq!(report["ok"], true, "{report}");
    assert!(report.get("ece").is_some() || report.get("buckets").is_some());
}

#[test]
fn n1_n2_ops_handler_shapes_via_core() {
    // Service handlers wrap these; assert core payloads used by HTTP stay stable.
    let hub = gr_probe_core::aggregate_unknown_buckets(&[json!({
        "session_id": "s",
        "unknown_bucket": {"codes": ["out_of_envelope"], "present": true}
    })]);
    assert_eq!(hub["algo"], "unknown_hub_aggregate_v1");
    let cap = gr_probe_core::self_capability_json();
    assert_eq!(cap["algo"], "self_capability_v1");
    assert_eq!(cap["capabilities"]["unknown_hub_aggregate"], true);
}
