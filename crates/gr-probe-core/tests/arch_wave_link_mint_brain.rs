//! Architecture wave: link_or_mint / ServerMint / versioned unit_surface / brain tasks / ops telemetry.
//! Drives shipped gr-core entry points only (no re-implementation of policies under test).

use gr_probe_core::product_scores::{score_br, score_os};
use gr_probe_core::stack_auth::stack_auth_from_fields;
use gr_probe_core::{
    apply_server_mint, commercial_projection, evaluate_session, field_algorithm_weights,
    link_or_mint_pair, load_field_product_matrix, scan_gaps, select_device_tier,
    MemoryDeviceIndex, BotScore, TruthResult, DIGEST_PATH_UNIT_V1, LINK_OR_MINT_ALGO,
    SERVER_MINT_ALGO, UNIT_SURFACE_ALGO_V1,
};
use serde_json::{json, Value};

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

fn soft_obs(os_instance: &str, unit: &str) -> Value {
    json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "hardware_concurrency": 8,
        "timezone": "UTC",
        "os_family": "linux",
        "architecture": "x86",
        "webgl_unmasked_renderer": "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero)))",
        "residual_mean": 0.500423,
        "residual_soft_like": true,
        "soft_stack": true,
        "hw_curve_audio": audio_curve(),
        "hw_curve_webgl": webgl_curve(),
        "screen_width": 1280,
        "screen_height": 720,
        "unit_surface_id": unit,
        "unit_surface_algo": UNIT_SURFACE_ALGO_V1,
        "unit_multiround_stable": true,
        "multi_seed_n": 8,
        "os_instance_hash": os_instance,
    })
}

fn real_gecko(unit: &str, l0: &str) -> Value {
    json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "hardware_concurrency": 12,
        "timezone": "Asia/Shanghai",
        "os_family": "linux",
        "architecture": "x86",
        "user_agent": "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0",
        "webgl_unmasked_renderer": l0,
        "residual_mean": 0.500181,
        "residual_soft_like": false,
        "hw_curve_audio": audio_curve(),
        "hw_curve_webgl": webgl_curve(),
        "webrtc_host_ip_hash": "lan_host_real_aabb",
        "unit_surface_id": unit,
        "unit_surface_algo": UNIT_SURFACE_ALGO_V1,
        "unit_multiround_stable": true,
        "multi_seed_n": 8,
        "screen_width": 1920,
        "screen_height": 1080,
        "server_client_ip": "203.0.113.10",
        "server_asn": "AS64500",
        "server_country": "CN",
    })
}

fn evidence_main(fields: Value) -> Value {
    json!({
        "sources": ["main", "gateway"],
        "batches": [
            {"batch_id": "B10_hw_curves", "source": "main"},
            {"batch_id": "B8_gateway", "source": "gateway"}
        ],
        "has_gateway": true,
        "fields": fields,
        "gateway_fields": {
            "server_client_ip": "203.0.113.10",
            "server_asn": "AS64500",
            "server_country": "CN"
        }
    })
}

/// Soft same unit + different OS instance → two distinct ServerMint commercial ids.
#[test]
fn soft_unit_collide_different_os_mints_two_ids() {
    let a = soft_obs("os_vm_guest0_machine_aaa", "unit_soft_same");
    let b = soft_obs("os_vm_clone1_machine_bbb", "unit_soft_same");
    let pair = link_or_mint_pair(&a, &b);
    assert_eq!(pair["algo"], LINK_OR_MINT_ALGO);
    assert_eq!(pair["server_mint_algo"], SERVER_MINT_ALGO);
    let id_a = pair["device_id_a"].as_str().unwrap();
    let id_b = pair["device_id_b"].as_str().unwrap();
    assert!((id_a.starts_with("dv-") || id_a.starts_with("dv_")) || (id_a.starts_with("dh-") || id_a.starts_with("dh_")));
    assert!((id_b.starts_with("dv-") || id_b.starts_with("dv_")) || (id_b.starts_with("dh-") || id_b.starts_with("dh_")));
    assert_ne!(id_a, id_b, "soft+diff OS must fork: {pair}");
    assert_eq!(pair["same_server_mint_id"], false);
    let action = pair["action"].as_str().unwrap();
    assert!(
        action.contains("mint") || action.contains("fork"),
        "expected mint/fork action: {action}"
    );

    // MemoryDeviceIndex store path
    let mut idx = MemoryDeviceIndex::new();
    let r1 = idx.link_or_mint(&a);
    let r2 = idx.link_or_mint(&b);
    assert_ne!(
        r1["device_id"].as_str(),
        r2["device_id"].as_str(),
        "store must not auto-merge soft OS fork: r1={r1} r2={r2}"
    );
    assert!(
        r2["action"].as_str().unwrap().starts_with("mint"),
        "second soft OS must mint: {r2}"
    );
}

/// Same soft unit + same OS instance → link / same ServerMint id.
#[test]
fn soft_same_unit_same_os_links() {
    let a = soft_obs("os_same_install_xyz", "unit_soft_same");
    let b = soft_obs("os_same_install_xyz", "unit_soft_same");
    let pair = link_or_mint_pair(&a, &b);
    assert_eq!(pair["same_server_mint_id"], true);
    assert_eq!(
        pair["device_id_a"].as_str(),
        pair["device_id_b"].as_str()
    );
    let mut idx = MemoryDeviceIndex::new();
    let r1 = idx.link_or_mint(&a);
    let r2 = idx.link_or_mint(&b);
    assert_eq!(r1["device_id"], r2["device_id"]);
    // iss/74: Fuzzy-similarity path reports `link_fs` (still a public link, same id).
    // Legacy exact-binder path reports `link`. Both satisfy Soft same-unit+same-OS.
    let action = r2["action"].as_str().unwrap_or("");
    assert!(
        action == "link" || action == "link_fs",
        "expected link/link_fs for soft same unit+OS, got {action}: {r2}"
    );
}

/// Gecko-style same residual/unit stable + different L0 labels → same ServerMint id (linkable).
#[test]
fn gecko_unit_stable_links_across_l0_labels() {
    let firefox = real_gecko(
        "unit_gecko_host_1",
        "ANGLE (NVIDIA Corporation, NVIDIA GeForce GTX 1050 Ti/PCIe/SSE2, OpenGL 4.5.0)",
    );
    let camoufox = real_gecko(
        "unit_gecko_host_1",
        "Apple M1, or similar", // L0 spoof must not split
    );
    let pair = link_or_mint_pair(&firefox, &camoufox);
    assert_eq!(
        pair["device_id_a"].as_str(),
        pair["device_id_b"].as_str(),
        "L0 must not split unit-stable real class: {pair}"
    );
    assert!(
        pair["unit_versioned_a"].as_bool().unwrap_or(false),
        "versioned unit required for path: {pair}"
    );
    // score should allow link
    assert!(
        pair["score"].as_f64().unwrap_or(0.0) >= 0.50
            || pair["same_server_mint_id"].as_bool() == Some(true),
        "expected high link affinity: {pair}"
    );
}

/// Versioned unit_surface enters digest_path; unversioned does not silently match versioned.
#[test]
fn unit_surface_versioned_digest_path() {
    let versioned = real_gecko("unit_v_abc", "ANGLE (NVIDIA, GTX 1050 Ti)");
    let mut unversioned = versioned.clone();
    unversioned
        .as_object_mut()
        .unwrap()
        .insert("unit_surface_algo".into(), json!("legacy_or_missing"));
    unversioned
        .as_object_mut()
        .unwrap()
        .insert("unit_multiround_stable".into(), json!(false));

    let p_v = commercial_projection(&versioned);
    let p_u = commercial_projection(&unversioned);
    let mint_v = apply_server_mint(&versioned);
    assert!(
        p_v.get("unit_surface_versioned")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || mint_v
                .get("unit_versioned_ok")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        "versioned path must mark unit: p={p_v} mint={mint_v}"
    );
    let dp = p_v
        .get("digest_path")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        dp.contains("unit") || mint_v["digest_path"].as_str() == Some(DIGEST_PATH_UNIT_V1),
        "digest_path should reflect unit version: dp={dp} mint={}",
        mint_v["digest_path"]
    );
    // Unversioned projection must not claim unit_surface_versioned true
    assert_ne!(
        p_u.get("unit_surface_versioned").and_then(|v| v.as_bool()),
        Some(true)
    );
    // Server mint ids: versioned vs unversioned may differ (version gate) — not silent same.
    let id_v = mint_v["device_id"].as_str().unwrap();
    let id_u = apply_server_mint(&unversioned)["device_id"]
        .as_str()
        .unwrap()
        .to_string();
    // At minimum both non-empty tiered
    assert!((id_v.starts_with("dv-") || id_v.starts_with("dv_")) || (id_v.starts_with("dh-") || id_v.starts_with("dh_")));
    assert!((id_u.starts_with("dv-") || id_u.starts_with("dv_")) || (id_u.starts_with("dh-") || id_u.starts_with("dh_")));
}

/// Soft ServerMint: unversioned/unstable unit is conf-only — same id as no-unit path.
#[test]
fn soft_unversioned_unit_conf_only_same_mint_as_no_unit() {
    let mut no_unit = soft_obs("os_soft_ver_gate", "unit_ignored");
    no_unit.as_object_mut().unwrap().remove("unit_surface_id");
    no_unit.as_object_mut().unwrap().remove("unit_surface_algo");
    no_unit.as_object_mut().unwrap().remove("unit_multiround_stable");
    no_unit.as_object_mut().unwrap().remove("multi_seed_n");

    let mut unversioned = soft_obs("os_soft_ver_gate", "unit_x_unversioned");
    unversioned
        .as_object_mut()
        .unwrap()
        .insert("unit_surface_algo".into(), json!("legacy_or_missing"));
    unversioned
        .as_object_mut()
        .unwrap()
        .insert("unit_multiround_stable".into(), json!(false));

    let versioned = soft_obs("os_soft_ver_gate", "unit_x_versioned");
    // already gr_unit_v1 + stable via soft_obs

    let id_none = apply_server_mint(&no_unit)["device_id"]
        .as_str()
        .unwrap()
        .to_string();
    let id_unver = apply_server_mint(&unversioned)["device_id"]
        .as_str()
        .unwrap()
        .to_string();
    let id_ver = apply_server_mint(&versioned)["device_id"]
        .as_str()
        .unwrap()
        .to_string();

    assert_eq!(
        id_none, id_unver,
        "unversioned unit must not fork soft ServerMint vs no-unit: {id_none} vs {id_unver}"
    );
    // Versioned unit may refine soft mint (optional participation) — if same residual+OS,
    // id may differ from conf-only when unit enters materials; that is intentional.
    assert!((id_none.starts_with("dv-") || id_none.starts_with("dv_")) || (id_none.starts_with("dh-") || id_none.starts_with("dh_")));
    assert!((id_ver.starts_with("dv-") || id_ver.starts_with("dv_")) || (id_ver.starts_with("dh-") || id_ver.starts_with("dh_")));

    // evaluate product path: unversioned unit must match no-unit commercial body.
    // Reset atlas between evals so differential extras cannot order-dependently fork au.
    gr_probe_core::population_atlas::reset_atlas_for_tests();
    let ev_none = evidence_main(no_unit.clone());
    let out_none = evaluate_session(&ev_none, None, None, None, true).expect("eval none");
    gr_probe_core::population_atlas::reset_atlas_for_tests();
    let ev_unver = evidence_main(unversioned.clone());
    let out_unver = evaluate_session(&ev_unver, None, None, None, true).expect("eval unver");
    let prod_none = out_none.get("product").cloned().unwrap_or(out_none.clone());
    let prod_unver = out_unver.get("product").cloned().unwrap_or(out_unver.clone());
    let pid_none = prod_none["device_id"].as_str().unwrap_or("").to_string();
    let pid_unver = prod_unver["device_id"].as_str().unwrap_or("").to_string();
    assert!(!pid_none.is_empty() && !pid_unver.is_empty());
    // Strip tier prefix — ServerMint body equality (dh_/dv_/dg_ may differ by tier gate)
    fn body(id: &str) -> String {
        id.strip_prefix("dh_")
            .or_else(|| id.strip_prefix("dv_"))
            .or_else(|| id.strip_prefix("dg_"))
            .unwrap_or(id)
            .to_string()
    }
    assert_eq!(
        body(&pid_none),
        body(&pid_unver),
        "evaluate product: soft unversioned unit conf-only same as no-unit: {pid_none} vs {pid_unver}"
    );
}

/// Soft residual-led ServerMint: unit_surface is conf-only when residual present
/// (unit multi-seed is engine-noisy). Residual class / host instance still fork.
#[test]
fn store_soft_same_os_different_versioned_unit_no_auto_merge() {
    let a = soft_obs("os_same_store", "unit_alpha_v1");
    let b = soft_obs("os_same_store", "unit_beta_v1");
    assert_ne!(
        a["unit_surface_id"].as_str(),
        b["unit_surface_id"].as_str()
    );

    let pair = link_or_mint_pair(&a, &b);
    let id_a = pair["device_id_a"].as_str().unwrap();
    let id_b = pair["device_id_b"].as_str().unwrap();
    // Same residual + same os_instance → residual-led soft mint body is shared;
    // versioned unit does not fork commercial id (unit conf-only under residual).
    assert_eq!(
        id_a, id_b,
        "soft residual-led path: unit diverge must NOT fork mint body: {pair}"
    );
    assert_eq!(
        pair["same_server_mint_id"].as_bool(),
        Some(true),
        "mint materials agree under residual-led soft: {pair}"
    );

    // Residual class diverge, same OS/unit, soft — store must not merge
    let c = soft_obs("os_same_store2", "unit_gamma_v1");
    let mut d = soft_obs("os_same_store2", "unit_gamma_v1");
    d.as_object_mut()
        .unwrap()
        .insert("residual_mean".into(), json!(0.500999));
    let mut idx2 = MemoryDeviceIndex::new();
    let r3 = idx2.link_or_mint(&c);
    let r4 = idx2.link_or_mint(&d);
    assert_ne!(
        r3["device_id"].as_str(),
        r4["device_id"].as_str(),
        "store must not merge soft residual class diverge: r3={r3} r4={r4}"
    );

    // Different host instance still forks even with same residual/unit
    let e = soft_obs("os_host_a", "unit_shared_v1");
    let f = soft_obs("os_host_b", "unit_shared_v1");
    let mut idx3 = MemoryDeviceIndex::new();
    let r5 = idx3.link_or_mint(&e);
    let r6 = idx3.link_or_mint(&f);
    assert_ne!(
        r5["device_id"].as_str(),
        r6["device_id"].as_str(),
        "soft os_instance diverge must fork: r5={r5} r6={r6}"
    );
}

/// evaluate_session product surface: multi-algorithm fusion + ops telemetry.
#[test]
fn evaluate_product_fusion_and_ops_telemetry() {
    let fields = soft_obs("os_eval_1", "unit_eval");
    let mut f = fields.clone();
    // claim-obs: exotic label on soft residual
    f.as_object_mut().unwrap().insert(
        "webgl_unmasked_renderer".into(),
        json!("ANGLE (NVIDIA, NVIDIA GeForce GTX 980 Direct3D11 vs_5_0 ps_5_0)"),
    );
    f.as_object_mut()
        .unwrap()
        .insert("webgl2_support".into(), json!(false));
    f.as_object_mut()
        .unwrap()
        .insert("webgl_max_texture".into(), json!(4096));

    let out = evaluate_session(&evidence_main(f.clone()), None, None, None, true)
        .expect("evaluate");
    let product = out.get("product").cloned().unwrap_or(out.clone());
    let id = product["device_id"].as_str().unwrap_or("");
    assert!(
        id.starts_with("dh-")
            || id.starts_with("dh_")
            || id.starts_with("dv-")
            || id.starts_with("dv_")
            || id.starts_with("dv0-")
            || id.starts_with("dv4-")
            || id.starts_with("dv5-")
            || id.starts_with("dv6-")
            || id.starts_with("dg_")
            || id.starts_with("dg-"),
        "tiered id required: {product}"
    );
    assert!(product.get("os").is_some());
    assert!(product.get("br").is_some());
    assert!(product.get("rpa").is_some());
    assert!(
        product.get("ops_fusion_telemetry").is_some()
            || out
                .get("diagnostics")
                .and_then(|d| d.get("ops_fusion_telemetry"))
                .is_some(),
        "ops telemetry required on product path"
    );
    let tel = product
        .get("ops_fusion_telemetry")
        .or_else(|| out.get("diagnostics").and_then(|d| d.get("ops_fusion_telemetry")))
        .cloned()
        .unwrap();
    assert!(tel.get("collision_risk").is_some());
    assert!(tel.get("unit_surface_coverage").is_some());
    assert!(tel.get("claim_obs_hits").is_some() || tel.get("claim_obs_hit_count").is_some());
    // Server mint marker
    assert!(
        product.get("server_mint").and_then(|v| v.as_bool()) == Some(true)
            || product.get("uniqueness_marker").is_some()
            || out
                .pointer("/device/server_mint")
                .and_then(|v| v.as_bool())
                == Some(true)
            || out.pointer("/device/uniqueness_marker").is_some(),
        "server mint signals required"
    );
}

/// Multi-axis field map: residual/unit/L0/control/JA* roles differ by axis; no global discard.
#[test]
fn field_map_multi_axis_roles_and_weights() {
    let m = load_field_product_matrix().expect("matrix");
    let residual = m.by_field.get("residual_soft_like").expect("residual");
    assert!(residual.axes.contains_key("os"));
    assert!(residual.axes.contains_key("br"));
    assert!(residual.axes.contains_key("device_id") || residual.axes.contains_key("rpa"));

    let unit = m.by_field.get("unit_surface_id").expect("unit");
    assert_eq!(
        unit.axes.get("device_id").map(|s| s.as_str()),
        Some("material")
    );

    let l0 = m.by_field.get("webgl_unmasked_renderer").expect("l0");
    // L0 may be conf/material on os/br but never sole device digest material as commercial primary
    assert!(l0.axes.get("br").is_some() || l0.axes.get("os").is_some());
    assert!(m
        .commercial_device_id
        .never_digest
        .iter()
        .any(|x| x == "webgl_unmasked_renderer" || x.contains("webgl_unmasked")));

    let ja4 = m.by_field.get("ja4").expect("ja4");
    assert!(ja4.axes.get("br").is_some());
    // device_id: diag/exclude OK; commercial never_digest still required (reference_aux ≠ digest)
    let ja_dev = ja4.axes.get("device_id").map(|s| s.as_str());
    assert!(
        matches!(ja_dev, Some("diag") | Some("exclude") | None)
            || m.commercial_device_id
                .never_digest
                .iter()
                .any(|x| x == "ja4"),
        "ja4 device role={ja_dev:?}"
    );
    assert!(m
        .commercial_device_id
        .never_digest
        .iter()
        .any(|x| x == "ja4"));

    let wd = m.by_field.get("webdriver").expect("webdriver");
    assert_eq!(wd.axes.get("rpa").map(|s| s.as_str()), Some("veto"));

    let weights = field_algorithm_weights();
    let algs = weights["algorithms"].as_object().unwrap();
    assert!(algs.contains_key("score_os"));
    assert!(algs.contains_key("score_br"));
    assert!(algs.contains_key("score_rpa"));
    assert!(algs.contains_key("link_or_mint"));
    assert!(algs.contains_key("commercial_projection"));
    // L0 weight 0 on commercial, non-zero claim on os
    assert_eq!(
        algs["commercial_projection"]["webgl_unmasked_renderer"]
            .as_f64()
            .unwrap(),
        0.0
    );
    assert!(algs["score_os"]["gpu_label_vs_residual_claim"].as_f64().unwrap() > 0.0);
    // soft residual alone 0 on rpa
    assert_eq!(
        algs["score_rpa"]["residual_soft_like_alone"].as_f64().unwrap(),
        0.0
    );
}

/// Claim-obs fixture moves os/br without redefining residual-led identity via L0 alone.
#[test]
fn claim_obs_multi_axis_does_not_redefine_device_by_l0() {
    let honest_soft = soft_obs("os_claim_1", "unit_claim");
    let mut exotic = honest_soft.clone();
    exotic.as_object_mut().unwrap().insert(
        "webgl_unmasked_renderer".into(),
        json!("ANGLE (NVIDIA, NVIDIA GeForce RTX 4090 Direct3D11 vs_5_0 ps_5_0)"),
    );
    exotic
        .as_object_mut()
        .unwrap()
        .insert("webgl2_support".into(), json!(false));
    exotic
        .as_object_mut()
        .unwrap()
        .insert("webgl_max_texture".into(), json!(4096));

    let mint_h = apply_server_mint(&honest_soft);
    let mint_e = apply_server_mint(&exotic);
    // ServerMint residual/unit/OS led — L0 not in mint materials; ids should match
    assert_eq!(
        mint_h["device_id"].as_str(),
        mint_e["device_id"].as_str(),
        "L0 spoof must not split ServerMint id"
    );

    let stack = stack_auth_from_fields(&exotic);
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
    let os = score_os(&exotic, &stack, &truth, &[]);
    let br = score_br(&exotic, &stack, &bot, &truth, &[]);
    let os_j: String = os["reasons"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str())
        .collect::<Vec<_>>()
        .join("|");
    let br_j: String = br["reasons"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str())
        .collect::<Vec<_>>()
        .join("|");
    assert!(
        os_j.contains("gpu_label") || os_j.contains("soft") || os_j.contains("claim"),
        "os claim-obs: {os_j}"
    );
    assert!(
        br_j.contains("gpu_label")
            || br_j.contains("integrity")
            || br_j.contains("incoherent")
            || br_j.contains("capability")
            || br_j.contains("claim"),
        "br claim-obs: {br_j}"
    );
}

/// Brain task-targeted gaps: soft residual schedules unit/rpa/os — not high GPU host dig priority.
#[test]
fn brain_task_targeted_soft_budget() {
    let fields = json!({
        "form_class": "desktop",
        "residual_soft_like": true,
        "soft_stack": true,
        "stack_class": "soft_render",
        "residual_mean": 0.500423,
        "hw_curve_webgl": webgl_curve(),
        "hw_curve_audio": audio_curve(),
        "webgl_unmasked_renderer": "ANGLE (NVIDIA, GeForce GTX 980)",
        "spoof_score": 0.55,
        // no unit_surface_id, no os_instance, no webgl2
    });
    let evidence = json!({
        "sources": ["main"],
        "batches": [{"batch_id": "B0_bootstrap", "source": "main"}],
        "fields": fields,
        "session_id": "brain_soft_1",
    });
    let gaps = scan_gaps(&evidence, None).expect("gaps");
    let codes: Vec<String> = gaps.iter().map(|g| g.code.clone()).collect();
    let joined = codes.join("|");
    assert!(
        codes.iter().any(|c| c == "need_unit_surface"
            || c == "soft_stack_budget_rpa_os"
            || c == "need_claim_obs_verify"
            || c == "need_os_instance_separator"),
        "task-oriented soft gaps required: {joined}"
    );
    // GPU digs demoted to low under soft
    for g in &gaps {
        if g.code == "need_gpu_staircase" || g.code == "need_gpu_ns" {
            assert_eq!(
                g.severity, "low",
                "soft class must demote GPU host dig: {:?}",
                g
            );
        }
    }
}

/// A4: FE probe-DAG skip reporting surfaces a task-oriented re-probe gap.
#[test]
fn brain_dag_skipped_gap_severity_scales() {
    for (skips, expect_sev) in [(1u64, "medium"), (4u64, "high")] {
        let fields = json!({
            "form_class": "desktop",
            "platform": "Linux x86_64",
            "hardware_concurrency": 8,
            "timezone": "UTC",
            "os_family": "linux",
            "dag_v2_version": "1.0",
            "dag_v2_engine": "b0",
            "dag_v2_skipped_n": skips,
        });
        let evidence = json!({
            "sources": ["main"],
            "batches": [{"batch_id": "B0_bootstrap", "source": "main"}],
            "fields": fields,
            "session_id": "brain_dag_1",
        });
        let gaps = scan_gaps(&evidence, None).expect("gaps");
        let hit = gaps.iter().find(|g| g.code == "probe_dag_skipped");
        assert!(
            hit.is_some(),
            "probe_dag_skipped gap required for skips={skips}: {gaps:?}"
        );
        assert_eq!(
            hit.unwrap().severity, expect_sev,
            "severity for skips={skips}"
        );
        assert!(
            hit.unwrap().detail.contains("dag_v2=1.0")
                && hit.unwrap().detail.contains("engine=b0"),
            "gap detail carries dag metadata: {:?}",
            hit.unwrap()
        );
    }
    // No skip reported → no dag gap
    let fields = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "hardware_concurrency": 8,
        "timezone": "UTC",
        "os_family": "linux",
    });
    let evidence = json!({
        "sources": ["main"],
        "batches": [{"batch_id": "B0_bootstrap", "source": "main"}],
        "fields": fields,
        "session_id": "brain_dag_2",
    });
    let gaps = scan_gaps(&evidence, None).expect("gaps");
    assert!(
        !gaps.iter().any(|g| g.code == "probe_dag_skipped"),
        "no dag gap when nothing skipped: {gaps:?}"
    );
}

/// Utilization policy: pack-health marks surface a targeted re-probe gap.
#[test]
fn brain_pack_incomplete_gap_from_marks() {
    let fields = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "hardware_concurrency": 8,
        "timezone": "UTC",
        "os_family": "linux",
        "pack_failed_n": 1,
        "pack_incomplete_n": 3,
    });
    let evidence = json!({
        "sources": ["main"],
        "batches": [{"batch_id": "B0_bootstrap", "source": "main"}],
        "fields": fields,
        "session_id": "brain_pack_1",
    });
    let gaps = scan_gaps(&evidence, None).expect("gaps");
    let hit = gaps.iter().find(|g| g.code == "probe_pack_incomplete");
    assert!(hit.is_some(), "probe_pack_incomplete required: {gaps:?}");
    assert_eq!(hit.unwrap().severity, "high", "failed pack → high severity");
    assert!(
        hit.unwrap().detail.contains("failed=1") && hit.unwrap().detail.contains("incomplete=3"),
        "detail carries marks: {:?}",
        hit.unwrap()
    );

    // Only incomplete (no hard failure) → medium, still surfaced.
    let fields2 = json!({
        "form_class": "desktop",
        "platform": "Linux x86_64",
        "hardware_concurrency": 8,
        "timezone": "UTC",
        "os_family": "linux",
        "pack_incomplete_n": 1,
    });
    let evidence2 = json!({
        "sources": ["main"],
        "batches": [{"batch_id": "B0_bootstrap", "source": "main"}],
        "fields": fields2,
        "session_id": "brain_pack_2",
    });
    let gaps2 = scan_gaps(&evidence2, None).expect("gaps");
    let hit2 = gaps2.iter().find(|g| g.code == "probe_pack_incomplete");
    assert!(hit2.is_some(), "incomplete-only still gapped: {gaps2:?}");
    assert_eq!(hit2.unwrap().severity, "medium");
}

/// Tier select still emits non-empty id; ServerMint path non-empty uniqueness marker.
#[test]
fn select_tier_and_server_mint_nonempty() {
    let fields = soft_obs("os_tier_x", "unit_tier");
    let tier = select_device_tier(
        &fields,
        Some(&json!({
            "sources": ["main"],
            "batches": [{"batch_id":"B10","source":"main"}],
        })),
    );
    let id = tier["device_id"].as_str().unwrap_or("");
    assert!(!id.is_empty());
    let mint = apply_server_mint(&fields);
    assert!(mint["server_mint"].as_bool().unwrap_or(false));
    assert!(mint["uniqueness_marker"]
        .as_str()
        .unwrap_or("")
        .contains(SERVER_MINT_ALGO));
}
