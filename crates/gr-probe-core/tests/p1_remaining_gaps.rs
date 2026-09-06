//! P1 remaining SSOT gaps: GPU-ns wire, signed PoW, peripheral schedule, protocol xsrc.

use gr_probe_core::challenge_pow::{
    evaluate_challenge_fields, issue_challenge_seed, validate_challenge_seed,
    DEFAULT_CHALLENGE_SECRET,
};
use gr_probe_core::xsrc::{evaluate_xsrc, XSRC_CONFLICT};
use gr_probe_core::{build_frontier, score_bot, score_br, score_os, stack_auth_from_fields};
use serde_json::json;

#[test]
fn gpu_ns_fields_change_stack_auth() {
    let thin = json!({
        "webgl_unmasked_renderer": "ANGLE (NVIDIA GeForce GTX 1050 Ti)",
        "residual_mean": 0.5001811906403189,
    });
    let a0 = stack_auth_from_fields(&thin);
    let rich = json!({
        "webgl_unmasked_renderer": "ANGLE (NVIDIA GeForce GTX 1050 Ti)",
        "residual_mean": 0.5001811906403189,
        "gpu_ns_staircase": [
            {"size":64,"iter":8,"gpu_ns":12000,"disjoint":false},
            {"size":128,"iter":16,"gpu_ns":48000,"disjoint":false}
        ],
        "gpu_ns_median": 30000.0,
        "gpu_wall_median_ms": 2.5,
        "gpu_disjoint_rate": 0.0,
        "timer_query_available": true,
    });
    let a1 = stack_auth_from_fields(&rich);
    assert!(
        a1.reasons.iter().any(|r| r.contains("gpu_ns")),
        "must consume gpu_ns: {:?}",
        a1.reasons
    );
    // inconsistent ratio case
    let bad = json!({
        "webgl_unmasked_renderer": "NVIDIA GeForce RTX 3080",
        "residual_mean": 0.5001811906403189,
        "gpu_ns_staircase": [{"size":64,"iter":8,"gpu_ns":5_000_000_000.0,"disjoint":true}],
        "gpu_ns_median": 5_000_000_000.0,
        "gpu_wall_median_ms": 1.0,
        "gpu_disjoint_rate": 0.9,
    });
    let a2 = stack_auth_from_fields(&bad);
    assert!(
        a2.spoof_score > a0.spoof_score || a2.vm_score > a0.vm_score,
        "inconsistent gpu-ns should raise risk: spoof {} vm {} reasons {:?}",
        a2.spoof_score,
        a2.vm_score,
        a2.reasons
    );
}

#[test]
fn signed_challenge_issue_validate_and_stack_wire() {
    let sid = "pow_sess_p1";
    let now = 2_000_000_000_000u64;
    let mat = issue_challenge_seed(sid, now, 60_000, DEFAULT_CHALLENGE_SECRET);
    let seed = mat["challenge_seed"].as_str().unwrap();
    let exp = mat["challenge_seed_exp_ms"].as_u64().unwrap();
    let sig = mat["challenge_seed_sig"].as_str().unwrap();
    assert!(validate_challenge_seed(sid, seed, exp, sig, DEFAULT_CHALLENGE_SECRET, now).is_ok());
    assert!(
        validate_challenge_seed(sid, seed, exp, "00".repeat(32).as_str(), DEFAULT_CHALLENGE_SECRET, now)
            .is_err()
    );
    assert!(
        validate_challenge_seed(sid, seed, exp, sig, DEFAULT_CHALLENGE_SECRET, now + 120_000)
            .is_err()
    );

    let fields_ok = json!({
        "session_id": sid,
        "collected_at": now,
        "challenge_seed": seed,
        "challenge_seed_exp_ms": exp,
        "challenge_seed_sig": sig,
        "webgl_unmasked_renderer": "ANGLE (NVIDIA)",
        "residual_mean": 0.5001811906403189,
    });
    let a_ok = stack_auth_from_fields(&fields_ok);
    assert!(
        a_ok.reasons.iter().any(|r| r.contains("challenge_seed_valid")),
        "{:?}",
        a_ok.reasons
    );

    let fields_bad = json!({
        "session_id": sid,
        "collected_at": now,
        "challenge_seed": seed,
        "challenge_seed_exp_ms": exp,
        "challenge_seed_sig": "deadbeef",
        "webgl_unmasked_renderer": "ANGLE (NVIDIA)",
        "residual_mean": 0.5001811906403189,
    });
    let a_bad = stack_auth_from_fields(&fields_bad);
    assert!(a_bad.spoof_score > a_ok.spoof_score || a_bad.reasons.iter().any(|r| r.contains("mismatch")));
    let (ok, _) = evaluate_challenge_fields(
        sid,
        fields_ok.as_object().unwrap(),
        DEFAULT_CHALLENGE_SECRET,
        now,
    );
    assert!(ok);
}

#[test]
fn frontier_schedules_p1_peripheral_and_physical() {
    let evidence = json!({
        "session_id": "p1_thin",
        "sources": ["main"],
        "batches": [
            {"batch_id":"B0_bootstrap","source":"main"},
            {"batch_id":"B1_conflict","source":"main"}
        ],
        "fields": {
            "form_class": "desktop",
            "platform": "Linux",
            "webdriver": false,
            "user_agent": "Mozilla/5.0 Chrome/120",
            "hardware_concurrency": 4
        },
        "has_gateway": false,
    });
    let f = build_frontier(&evidence, true, Some(true), 28).unwrap();
    let packs: Vec<String> = f
        .packs
        .iter()
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    let hit = packs.iter().any(|p| {
        matches!(
            p.as_str(),
            "B22_gpu_timer"
                | "B20_challenge_seed"
                | "B28_permissions_media"
                | "B29_sensors_battery"
                | "B30_gpu_bandwidth"
                | "B34_cpu_cache_ladder"
                | "B31_shader_numeric"
                | "B33_caps_pressure"
                | "B25_clock_raf"
                | "B8_gateway_early"
        )
    });
    assert!(hit, "p1 packs expected: {packs:?}");
    let codes: Vec<_> = f.gaps.iter().map(|g| g.code.clone()).collect();
    assert!(
        codes.iter().any(|c| {
            c.contains("gpu")
                || c.contains("challenge")
                || c.contains("permissions")
                || c.contains("sensors")
                || c.contains("bandwidth")
                || c.contains("cache")
                || c.contains("protocol")
                || c.contains("shader")
                || c.contains("caps")
        }),
        "gaps: {codes:?}"
    );
}

#[test]
fn protocol_ja4_vs_ua_conflict_demotes() {
    let fields = json!({
        "user_agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120.0.0.0 Safari/537.36",
        "platform": "Win32",
        "form_class": "desktop",
        "webdriver": false,
        "hw_curve_webgl": [0.1, 0.2, 0.3, 0.4],
        "sandbox_ok": true,
        "sandbox_sources_received": ["iframe:1"],
        "multi_source_match_ratio": 0.95,
        "native_integrity_ratio": 0.95,
        "canvas_geometry_hash": "c1",
        "api_flags_hash": "a1",
    });
    // With protocol mismatch (Chrome UA vs firefox protocol_engine + ja4)
    let ev = json!({
        "fields": fields,
        "gateway_fields": {
            "user_agent": "Mozilla/5.0 Chrome/120",
            "ja4": "ff_t13d1516h2_...firefox...",
            "protocol_engine": "firefox",
            "h2_fingerprint": "firefox-h2-settings"
        },
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "batches": [
            {"batch_id":"B0_bootstrap","source":"main"},
            {"batch_id":"B8_gateway","source":"gateway"}
        ]
    });
    let truth = evaluate_xsrc(&ev, None).unwrap();
    assert_eq!(
        truth.xsrc_status, XSRC_CONFLICT,
        "protocol mismatch must set xsrc conflict: status={} details={}",
        truth.xsrc_status, truth.details
    );
    let conflicts = truth
        .details
        .get("conflicts")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(
        conflicts.iter().any(|c| {
            let s = c.as_str().unwrap_or("");
            s.contains("ja4_vs_ua") || s.contains("h2_vs_ua")
        }),
        "details.conflicts must include ja4/h2 mismatch: {conflicts:?}"
    );
    // Demote via truth.xsrc_status alone — empty source_conflicts, no injected theater codes
    let stack = stack_auth_from_fields(&fields);
    let bot = score_bot(&fields, "balanced").unwrap();
    let empty_conflicts: Vec<String> = vec![];
    let br = score_br(&fields, &stack, &bot, &truth, &empty_conflicts);
    assert_ne!(br["status"].as_str(), Some("real"));
    assert!(br["score"].as_f64().unwrap() < 0.5, "br={}", br);

    // Absent protocol fields: no invented conflict from empty gateway
    let ev2 = json!({
        "fields": fields,
        "gateway_fields": {"user_agent": "Mozilla/5.0 Chrome/120"},
        "sources": ["main", "gateway"],
        "has_gateway": true,
        "batches": [
            {"batch_id":"B0_bootstrap","source":"main"},
            {"batch_id":"B8_gateway","source":"gateway"}
        ]
    });
    let truth2 = evaluate_xsrc(&ev2, None).unwrap();
    assert_ne!(truth2.xsrc_status, XSRC_CONFLICT, "no false protocol conflict");
    let conf2 = truth2
        .details
        .get("conflicts")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(
        !conf2.iter().any(|c| {
            let s = c.as_str().unwrap_or("");
            s.contains("ja4") || s.contains("h2_vs_ua")
        }),
        "no ja4/h2 in conflicts: {conf2:?}"
    );
}

#[test]
fn score_os_hits_peripheral_and_bandwidth_materials() {
    let fields = json!({
        "platform": "Linux",
        "hw_curve_webgl": [0.1, 0.2, 0.3, 0.4],
        "font_bitmap_hash": "f",
        "sandbox_ok": true,
        "sandbox_sources_received": ["iframe:1"],
        "multi_source_match_ratio": 0.95,
        "gpu_ns_median": 10000.0,
        "gpu_ns_staircase": [{"size":64,"iter":8,"gpu_ns":10000}],
        "gpu_readback_ladder": [{"size":256,"mib_s":100}],
        "roundtrip_intercept_ms": 0.5,
        "cpu_cache_ladder": [{"bytes":4096,"wall_ms":0.1}],
        "cpu_cache_knee_bytes": 262144,
        "permissions_matrix": {"camera":"prompt"},
        "media_devices_count": 2,
        "battery_level": 0.8,
    });
    let stack = stack_auth_from_fields(&fields);
    let truth = gr_probe_core::xsrc::TruthResult {
        xsrc_status: "ok".into(),
        real_band: "watch".into(),
        credibility: 0.8,
        fe_only: false,
        has_server_side: true,
        has_main_core: true,
        packages: json!([]),
        reasons: vec![],
        details: json!({}),
    };
    let os = score_os(&fields, &stack, &truth, &[]);
    let hits: Vec<String> = os["field_hits"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|h| h.as_str().map(|s| s.to_string()))
        .collect();
    assert!(
        hits.iter().any(|h| h.contains("gpu")
            || h.contains("cpu")
            || h.contains("permissions")
            || h.contains("sensors")
            || h.contains("mat:")),
        "hits={hits:?}"
    );
}
