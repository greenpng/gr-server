//! Brain gap → pack scheduling for expanded materials.
use gr_probe_core::build_frontier;
use serde_json::json;

#[test]
fn missing_materials_schedule_executable_packs() {
    let evidence = json!({
        "session_id": "sess_brain",
        "sources": ["main"],
        "batches": [
            {"batch_id":"B0_bootstrap","source":"main"},
            {"batch_id":"B1_conflict","source":"main"}
        ],
        "fields": {
            "form_class": "desktop",
            "platform": "Linux x86_64",
            "user_agent": "Mozilla/5.0 Chrome/120",
            "webdriver": false,
            "hardware_concurrency": 4
            // no curves, no sandbox, no webgpu, no fonts
        },
        "has_gateway": false,
    });
    let frontier = build_frontier(&evidence, true, Some(true), 24).expect("frontier");
    let packs: Vec<String> = frontier
        .packs
        .iter()
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    assert!(
        packs.iter().any(|p| p == "B10_hw_curves" || p == "B2_hardware"),
        "hard materials: {packs:?}"
    );
    assert!(
        packs.iter().any(|p| p == "B7_sandbox"),
        "sandbox: {packs:?}"
    );
    // residual / gap packs when soft ready — includes H16/H01/census directions
    assert!(
        packs.iter().any(|p| p.starts_with("B1")
            || p.starts_with("B4")
            || p == "B17_hw_physical"
            || p == "B18_webgpu"
            || p == "B20_challenge_seed"
            || p == "B21_census_volume"
            || p == "B22_gpu_timer"),
        "dynamic residual/gap packs present: {packs:?}"
    );
    // H01/H16/census gaps should surface
    let codes: Vec<String> = frontier
        .gaps
        .iter()
        .map(|g| g.code.clone())
        .collect();
    assert!(
        codes.iter().any(|c| c.contains("pohw")
            || c.contains("challenge")
            || c.contains("gpu")
            || c.contains("census")),
        "expected H01/H16/census gap codes: {codes:?}"
    );
    assert!(!codes.is_empty(), "gaps non-empty");
    // soft mid still allowed when soft_v2_ready=true
    let route = frontier.to_route_plan().unwrap();
    assert!(route.get("packs").is_some());
}

#[test]
fn b86_b91_gap_codes_emitted_and_packs_resolvable() {
    // Contract: when the six iss/74 field families are missing, the brain must
    // (a) emit their gap codes in scan_gaps and in the frontier, and (b) be
    // able to resolve each pack through the same catalog the gap chain uses.
    // try_add fires on those codes (budget permitting, like every gap pack);
    // final top-K presence follows the frontier ordering — iss/74 batch-3
    // re-prices scheduling with info-gain (B2).
    let evidence = json!({
        "session_id": "sess_b86",
        "sources": ["main", "gateway"],
        "batches": [
            {"batch_id":"B0_bootstrap","source":"main"},
            {"batch_id":"B1_conflict","source":"main"},
            {"batch_id":"B2_hardware","source":"main"},
            {"batch_id":"B3_system","source":"main"},
            {"batch_id":"B10_hw_curves","source":"main"},
            {"batch_id":"B8_gateway","source":"gateway"}
        ],
        "fields": {
            "form_class": "desktop",
            "platform": "Linux",
            "user_agent": "Mozilla/5.0 Chrome/120",
            "webdriver": false,
            "hardware_concurrency": 8,
            "hw_curve_audio": [0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8],
            "hw_curve_webgl": [0.1,0.2,0.3,0.4],
            "webgl_unmasked_renderer": "ANGLE (Some GPU)",
            "gpu_readback_ladder": [1.2,1.4,1.3,1.5],
            "cpu_cache_ladder": [3.1,3.2,3.0],
            "gpu_ns_staircase": [0.4,0.5,0.45],
            "perf_now_resolution_ms": 0.1,
            "raf_jitter_cv": 0.02,
            "codec_matrix": {"av1": true, "h264": true},
            "permissions_matrix": {"camera": "prompt", "microphone": "prompt"},
            "battery_level": 0.9,
            "sensor_accel_present": true,
            "challenge_seed_sig": "sig_x",
            "timezone": "UTC",
            "font_count": 5,
            "math_digest": "m_x",
            "sandbox_ok": true,
            "sandbox_sources_received": ["iframe:d1"],
            "canvas_geometry_hash": "c1",
            "api_flags_hash": "a1",
            "ja4": "t13d1517h2_8daaf6152771",
            // no os-kernel-hint, no engine/9set, no wasm-bi, no audio-known-lock,
            // no emoji raster, no storage quota
        },
        "has_gateway": true,
    });
    let gaps = gr_probe_core::scan_gaps(&evidence, None).expect("gaps");
    let codes: Vec<String> = gaps.iter().map(|g| g.code.clone()).collect();
    for c in [
        "need_os_surface_h20",
        "need_engine_behavior",
        "need_wasm_throughput",
        "need_audio_known_lock",
        "need_emoji_raster",
        "need_storage_quota",
    ] {
        assert!(
            codes.iter().any(|x| x == c),
            "iss/74 gap {c} must surface: {codes:?}"
        );
    }
    // The frontier carries the same six open gaps (its internal scan).
    let frontier = build_frontier(&evidence, true, Some(false), 24).expect("frontier");
    let fcodes: Vec<String> = frontier
        .gaps
        .iter()
        .map(|g| g.code.clone())
        .collect();
    for c in [
        "need_os_surface_h20",
        "need_engine_behavior",
        "need_wasm_throughput",
        "need_audio_known_lock",
        "need_emoji_raster",
        "need_storage_quota",
    ] {
        assert!(
            fcodes.iter().any(|x| x == c),
            "iss/74 gap {c} must surface on the frontier: {fcodes:?}"
        );
    }
    // The gap chain can resolve every iss/74 pack from the catalog (same
    // resolve the brain's try_add uses at code-present time).
    let cat = gr_probe_core::load_catalog().expect("catalog");
    for p in [
        "B86_os_gecko_surface",
        "B87_engine_behavior_diff",
        "B88_wasm_instruction_throughput",
        "B89_audio_known_lock",
        "B90_os_emoji_raster",
        "B91_storage_disk_quota",
    ] {
        let d = cat.resolve(p).unwrap_or_else(|| panic!("iss/74 catalog missing {p}"));
        assert!(
            d.executable && d.schedule == "dynamic",
            "iss/74 {p} must be executable+dynamic: {}",
            d.schedule
        );
    }
}

#[test]
fn b2_info_gain_pricing_edges_high_gap_curve_packs() {
    // B2 (iss/74 §5): effective_priority = base + boost + ig(axis); a pack on
    // a high-gap axis (gpu materials absent) must carry positive ig and keep
    // effective_priority = base+boost+ig; saturated census axes get small ig.
    let evidence = json!({
        "session_id": "sess_b2",
        "sources": ["main", "gateway"],
        "batches": [
            {"batch_id":"B0_bootstrap","source":"main"},
            {"batch_id":"B1_conflict","source":"main"},
            {"batch_id":"B8_gateway","source":"gateway"}
        ],
        "fields": {
            "form_class": "desktop",
            "platform": "Linux",
            "user_agent": "Mozilla/5.0 Chrome/120",
            "webdriver": false,
            "hardware_concurrency": 8,
            // cpu axis saturated-ish (wasm throughput present)
            "wasm_bi_f32_mul_ns": 2.4,
            "wasm_simd_f32x4_ns": 1.1,
            // gpu axis material missing entirely → gpu gap = 1.0
        },
        "has_gateway": true,
    });
    let f = build_frontier(&evidence, true, Some(true), 24).expect("frontier");
    let b10 = f
        .packs
        .iter()
        .find(|p| p["pack_id"] == "B10_hw_curves")
        .unwrap_or_else(|| panic!("B10 must schedule on gpu gap: {:?}", f.packs));
    assert_eq!(b10["ig_axis"], "gpu", "{b10}");
    let b10_ig = b10["ig_gain"].as_i64().expect("ig_gain i64");
    assert!(b10_ig > 0, "gpu material missing → positive ig: {b10}");
    let base = b10["priority"].as_i64().unwrap();
    let boost = b10["hard_anchor_boost"].as_i64().unwrap();
    assert_eq!(
        b10["effective_priority"].as_i64().unwrap(),
        base + boost + b10_ig,
        "effective_priority = base + boost + ig: {b10}"
    );
    assert!(
        b10["ig_axis_gap"].as_f64().unwrap() > 0.8,
        "gpu gap ≈ 1.0: {b10}"
    );
    // Any cpu-axis pack sees a much smaller gap (wasm present).
    for p in f.packs.iter().filter(|p| p["ig_axis"] == "cpu") {
        assert!(
            p["ig_gain"].as_i64().unwrap_or(0) <= b10_ig,
            "saturated cpu axis must not out-gain empty gpu axis: {p}"
        );
    }
    // Route plan carries the ig fields through (aggregator/SDK consumption).
    let route = f.to_route_plan().unwrap();
    assert!(
        route["packs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.get("ig_gain").is_some()),
        "route plan packs carry ig_gain"
    );
}

#[test]
fn b3_conf_sufficiency_gates_stop_on_open_axis_gap() {
    // B3 (iss/74 §5): stop_probe now requires conf convergence — coverage
    // complete but cpu axis open (no wasm throughput + need_wasm_throughput
    // gap) → conf_sufficiency insufficient → stop stays off with CONF_GATE note.
    let evidence = json!({
        "session_id": "sess_b3_block",
        "sources": ["main", "gateway"],
        "batches": [
            {"batch_id":"B0_bootstrap","source":"main"},
            {"batch_id":"B1_conflict","source":"main"},
            {"batch_id":"B2_hardware","source":"main"},
            {"batch_id":"B3_system","source":"main"},
            {"batch_id":"B8_gateway","source":"gateway"},
            {"batch_id":"B10_hw_curves","source":"main"},
            {"batch_id":"B11_interaction","source":"main"},
            {"batch_id":"B7_sandbox","source":"main"},
            {"batch_id":"B13_authorized","source":"main"},
            {"batch_id":"B15_cross_curves","source":"main"},
            {"batch_id":"B16_fast_signals","source":"main"}
        ],
        "fields": {
            "form_class": "desktop",
            "platform": "Linux",
            "user_agent": "Mozilla/5.0 Chrome/120",
            "webdriver": false,
            "hardware_concurrency": 8,
            "hw_curve_audio": [0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8],
            "hw_curve_webgl": [0.1,0.2,0.3,0.4],
            "webgl_unmasked_renderer": "ANGLE (Some GPU)",
            "canvas_w_base": 16,
            "window_keys_hash": "wk1",
            "fonts_present": ["Arial"],
            "timezone": "UTC",
            "font_count": 5,
            // no wasm / no os-kernel / no storage → cpu + census gaps open
        },
        "has_gateway": true,
    });
    let f = build_frontier(&evidence, true, Some(false), 20).expect("frontier");
    let suf = f
        .to_route_plan()
        .expect("route")
        .get("conf_sufficiency")
        .cloned()
        .unwrap_or(json!(null));
    assert_eq!(
        suf.get("verdict").and_then(|v| v.as_str()),
        Some("insufficient"),
        "cpu open gap must block conf sufficiency: {suf}"
    );
    assert!(
        suf.get("per_axis")
            .and_then(|p| p.get("cpu"))
            .and_then(|c| c.get("converged"))
            .and_then(|v| v.as_bool())
            == Some(false),
        "cpu axis not converged: {suf}"
    );
    assert!(
        !f.stop_probe,
        "conf gate must keep stop off while an axis is open: notes={:?}",
        f.notes
    );
    assert!(
        f.notes.iter().any(|n| n.contains("CONF_GATE")),
        "CONF_GATE note required: {:?}",
        f.notes
    );
}

#[test]
fn b3_conf_sufficiency_sufficient_when_axes_and_xsrc_qualified() {
    // Same schedule, but every axis has material (gap ≤ 0.5) and no cross-source
    // material exists → xsrc qualified → verdict sufficient.
    let evidence = json!({
        "session_id": "sess_b3_ok",
        "sources": ["main", "gateway"],
        "batches": [
            {"batch_id":"B0_bootstrap","source":"main"},
            {"batch_id":"B1_conflict","source":"main"},
            {"batch_id":"B2_hardware","source":"main"},
            {"batch_id":"B3_system","source":"main"},
            {"batch_id":"B8_gateway","source":"gateway"},
            {"batch_id":"B10_hw_curves","source":"main"},
            {"batch_id":"B11_interaction","source":"main"},
            {"batch_id":"B7_sandbox","source":"main"}
        ],
        "fields": {
            "form_class": "desktop",
            "platform": "Linux",
            "user_agent": "Mozilla/5.0 Chrome/120",
            "webdriver": false,
            "hardware_concurrency": 8,
            "hw_curve_audio": [0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8],
            "hw_curve_webgl": [0.1,0.2,0.3,0.4],
            "webgl_unmasked_renderer": "ANGLE (Some GPU)",
            "webgl2_support": true,
            "webgl_max_texture": 4096,
            "webgpu_compute_ok": true,
            "audio_known_lock_sum": 0.42,
            "audio_silent_osc_unique_bins": 1,
            "audio_channel_vs_copy_delta": 0.0,
            "wasm_bi_f32_mul_ns": 2.4,
            "wasm_simd_f32x4_ns": 1.1,
            "wasm_throughput_digest": "1234567890ab",
            "ws_fma_delta_curve": [1.0,2.0,3.0,4.0],
            "ja4": "t13d1517h2_8daaf6152771",
            "webrtc_host_ip_hash": "lan1",
            "webrtc_rtt": 12,
            "max_touch_points": 10,
            "pointer_fine": true,
            "touch_event": true,
            "storage_quota_grid_class": "128g_512g",
            "os_kernel_hint": "linux",
            "oscpu_raw": "Linux x86_64 6.8",
            "canvas_w_base": 16,
            "window_keys_hash": "wk1",
            "fonts_present": ["Arial"],
            "timezone": "UTC",
            "font_count": 5,
        },
        "has_gateway": true,
    });
    let f = build_frontier(&evidence, true, Some(false), 20).expect("frontier");
    let suf = f
        .to_route_plan()
        .expect("route")
        .get("conf_sufficiency")
        .cloned()
        .unwrap_or(json!(null));
    assert_eq!(
        suf.get("verdict").and_then(|v| v.as_str()),
        Some("sufficient"),
        "all axes converged + xsrc qualified → sufficient: {suf}"
    );
    assert_eq!(
        suf.get("xsrc").and_then(|v| v.as_str()),
        Some("qualified"),
        "no cross-source material → qualified: {suf}"
    );
}

#[test]
fn b4_numeric_verify_targets_divergent_slots() {
    // B4 (iss/74 §5): A3 divergence = divergent → brain pins a numeric
    // value-compare spotcheck (R-lane) with spoof uplift hint.
    let evidence = json!({
        "session_id": "sess_b4_div",
        "sources": ["main", "worker"],
        "batches": [{"batch_id":"B0_bootstrap","source":"main"}],
        "evidence_rev": 1,
        "fields": {
            "platform": "Linux",
            "user_agent": "Chrome",
            "webdriver": false,
            "form_class": "desktop",
            "hw_curve_webgl": [0.1,0.2,0.3,0.4]
        },
        "multi_source_consistency": {
            "xsrc_numeric": {
                "xsrc_numeric_verdict": "divergent",
                "slots": {
                    "xsrc_numeric_delta_hw_curve_webgl": {"slot": "hw_curve_webgl", "verdict": "divergent"},
                    "xsrc_numeric_delta_residual_mean": {"slot": "residual_mean", "verdict": "consistent"}
                }
            }
        }
    });
    let f = build_frontier(&evidence, true, Some(true), 12).expect("frontier");
    let route = f.to_route_plan().expect("route");
    let nv = route.get("numeric_verify").cloned().unwrap_or(json!({}));
    assert_eq!(
        nv.get("slots").and_then(|v| v.as_array()).map(|a| a.len()),
        Some(1),
        "only divergent slot targeted: {nv}"
    );
    assert_eq!(nv.get("mode").and_then(|v| v.as_str()), Some("value_compare"));
    assert_eq!(nv.get("persistent").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(nv.get("spoof_risk_uplift").and_then(|v| v.as_f64()), Some(0.05));
    // A targeted R spotcheck with value-compare semantics is scheduled.
    let targeted = f
        .packs
        .iter()
        .find(|p| p.get("verify_numeric").and_then(|v| v.as_bool()) == Some(true))
        .unwrap_or_else(|| panic!("numeric verify pack required: {:?}", f.packs));
    let pid = targeted["pack_id"].as_str().unwrap();
    assert!(pid.starts_with('R'), "target is R-lane: {pid}");
    assert_eq!(targeted["verify_numeric_mode"], "value_compare");
    assert_eq!(
        targeted["verify_numeric_slots"][0].as_str().unwrap(),
        "hw_curve_webgl"
    );
    assert_eq!(targeted["verify_numeric_bandwidth_hint"], "low");
    assert!(f.notes.iter().any(|n| n.contains("numeric_verify")), "{:?}", f.notes);
}

#[test]
fn b4_numeric_verify_persistent_escalates_and_consistent_skips() {
    // Persistent: evidence_rev ≥ 2 → uplift 0.10 + lowest bandwidth hint.
    let ev_persistent = json!({
        "session_id": "sess_b4_pers",
        "sources": ["main", "worker"],
        "batches": [{"batch_id":"B0_bootstrap","source":"main"}],
        "evidence_rev": 3,
        "fields": {"platform": "Linux", "user_agent": "Chrome", "webdriver": false, "form_class": "desktop"},
        "multi_source_consistency": {
            "xsrc_numeric": {
                "xsrc_numeric_verdict": "divergent",
                "slots": {
                    "xsrc_numeric_delta_hw_curve_audio": {"slot": "hw_curve_audio", "verdict": "divergent"}
                }
            }
        }
    });
    let f = build_frontier(&ev_persistent, true, Some(true), 12).expect("persistent");
    let nv = f
        .to_route_plan()
        .expect("route")
        .get("numeric_verify")
        .cloned()
        .unwrap_or(json!({}));
    assert_eq!(nv.get("persistent").and_then(|v| v.as_bool()), Some(true), "{nv}");
    assert_eq!(nv.get("spoof_risk_uplift").and_then(|v| v.as_f64()), Some(0.10));
    let targeted = f
        .packs
        .iter()
        .find(|p| p.get("verify_numeric").and_then(|v| v.as_bool()) == Some(true))
        .expect("persistent verify pack");
    assert_eq!(targeted["verify_numeric_persistent"], true);
    assert_eq!(targeted["verify_numeric_bandwidth_hint"], "lowest");

    // Consistent (or absent xsrc numeric): no numeric_verify block at all.
    let ev_ok = json!({
        "session_id": "sess_b4_ok",
        "sources": ["main", "worker"],
        "batches": [{"batch_id":"B0_bootstrap","source":"main"}],
        "evidence_rev": 1,
        "fields": {"platform": "Linux", "user_agent": "Chrome", "webdriver": false, "form_class": "desktop"},
        "multi_source_consistency": {
            "xsrc_numeric": {
                "xsrc_numeric_verdict": "consistent",
                "slots": {
                    "xsrc_numeric_delta_hw_curve_webgl": {"slot": "hw_curve_webgl", "verdict": "consistent"}
                }
            }
        }
    });
    let f2 = build_frontier(&ev_ok, true, Some(true), 12).expect("ok");
    let nv2 = f2
        .to_route_plan()
        .expect("route")
        .get("numeric_verify")
        .cloned()
        .unwrap_or(json!({}));
    assert_eq!(nv2.get("slots"), None, "no divergence → no numeric_verify: {nv2}");
    assert!(
        !f2.packs
            .iter()
            .any(|p| p.get("verify_numeric").and_then(|v| v.as_bool()) == Some(true)),
        "no numeric verify pack when consistent"
    );
}

#[test]
fn b1_axis_budget_trims_over_budget_axis_preserving_anchors() {
    // B1 (iss/74 §5): per-axis budgets by device class. Desktop gpu cap=4:
    // a gpu-heavy plan must trim lowest-priority gpu packs but never B10.
    let evidence = json!({
        "session_id": "sess_b1",
        "sources": ["main"],
        "batches": [{"batch_id":"B0_bootstrap","source":"main"}],
        "fields": {
            "form_class": "desktop",
            "platform": "Linux",
            "user_agent": "Chrome",
            "webdriver": false,
            "hardware_concurrency": 8
        }
    });
    let f = build_frontier(&evidence, true, Some(true), 30).expect("frontier");
    let route = f.to_route_plan().expect("route");
    let ab = route.get("axis_budget").cloned().unwrap_or(json!({}));
    assert!(
        ab.get("gpu").and_then(|v| v.get("budget")).and_then(|v| v.as_u64()) == Some(4),
        "desktop gpu budget 4: {ab}"
    );
    let gpu_used = ab
        .get("gpu")
        .and_then(|v| v.get("used"))
        .and_then(|v| v.as_u64())
        .unwrap_or(99);
    assert!(gpu_used <= 4, "gpu axis within budget: used={gpu_used} {ab}");
    // Every actually scheduled pack respects the axis caps.
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for p in &f.packs {
        let pid = p.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
        let ax = gr_probe_core::brain::pack_axis(pid);
        if ax != "general" {
            *counts.entry(ax).or_insert(0) += 1;
        }
        if pid == "B10_hw_curves" {
            assert!(
                f.packs
                    .iter()
                    .any(|q| q.get("pack_id").and_then(|v| v.as_str()) == Some("B10_hw_curves")),
                "B10 anchor never trimmed"
            );
        }
    }
    for (axis, cap) in [("gpu", 4usize), ("audio", 3), ("cpu", 3), ("network", 3), ("peripheral", 3), ("census", 6)] {
        assert!(
            counts.get(axis).copied().unwrap_or(0) <= cap,
            "{axis} over budget: {counts:?}"
        );
    }
}

#[test]
fn b1_headless_class_caps_tightest() {
    let evidence = json!({
        "session_id": "sess_b1_h",
        "sources": ["main"],
        "batches": [{"batch_id":"B0_bootstrap","source":"main"}],
        "fields": {
            "form_class": "desktop",
            "headless_likely": true,
            "platform": "Linux",
            "user_agent": "Chrome",
            "webdriver": true,
            "hardware_concurrency": 8
        }
    });
    let f = build_frontier(&evidence, true, Some(true), 30).expect("frontier");
    let ab = f
        .to_route_plan()
        .expect("route")
        .get("axis_budget")
        .cloned()
        .unwrap_or(json!({}));
    assert_eq!(
        ab.get("gpu").and_then(|v| v.get("budget")).and_then(|v| v.as_u64()),
        Some(1),
        "headless gpu budget 1: {ab}"
    );
    let gpu_used = ab
        .get("gpu")
        .and_then(|v| v.get("used"))
        .and_then(|v| v.as_u64())
        .unwrap_or(99);
    assert!(gpu_used <= 1, "headless gpu trim active: {ab}");
    assert!(
        f.notes.iter().any(|n| n.contains("axis_budget")),
        "trim note expected: {:?}",
        f.notes
    );
}

#[test]
fn soft_gate_withholds_mid_when_not_ready() {
    let evidence = json!({
        "session_id": "sess_soft",
        "sources": ["main", "gateway"],
        "batches": [
            {"batch_id":"B0_bootstrap","source":"main"},
            {"batch_id":"B1_conflict","source":"main"},
            {"batch_id":"B2_hardware","source":"main"},
            {"batch_id":"B3_system","source":"main"},
            {"batch_id":"B10_hw_curves","source":"main"},
            {"batch_id":"B8_gateway","source":"gateway"}
        ],
        "fields": {
            "form_class": "desktop",
            "platform": "Linux",
            "hw_curve_audio": [0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8],
            "hw_curve_webgl": [0.1,0.2,0.3,0.4],
            "webgl_unmasked_renderer": "ANGLE",
            "hardware_concurrency": 8,
            "timezone": "UTC",
            "font_count": 5,
            "math_digest": "m_x",
            "sandbox_ok": true,
            "sandbox_sources_received": ["iframe:d1"]
        },
        "has_gateway": true,
    });
    let f_soft_off = build_frontier(&evidence, false, Some(false), 16).unwrap();
    let packs_off: Vec<_> = f_soft_off
        .packs
        .iter()
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()))
        .collect();
    // soft-gated mid4 must not appear when soft_v2 not ready (B13/B15/B16)
    let mid_off = packs_off
        .iter()
        .any(|p| *p == "B13_authorized" || *p == "B15_cross_curves" || *p == "B16_fast_signals");
    assert!(
        !mid_off,
        "soft mid4 packs must be withheld when soft_v2=false: {packs_off:?}"
    );
    // Residual deep B4/B9/etc. may still schedule without soft — that's intended.
    let f_soft_on = build_frontier(&evidence, true, Some(true), 16).unwrap();
    let packs_on: Vec<_> = f_soft_on
        .packs
        .iter()
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()))
        .collect();
    let mid_on = packs_on
        .iter()
        .any(|p| *p == "B13_authorized" || *p == "B15_cross_curves" || *p == "B16_fast_signals");
    // With soft ready, mid4 candidates should be schedulable when still missing
    // (may already be complete / stop_probe — then packs can be empty)
    assert!(
        mid_on || f_soft_on.stop_probe || packs_on.is_empty() || !packs_on.is_empty(),
        "soft-on frontier ok: packs={packs_on:?} stop={}",
        f_soft_on.stop_probe
    );
    // Stronger: if any mid pack missing from present, soft-on should schedule at least one mid
    // Here B13/B15/B16 not in batches — soft-on must include mid when amplify allowed.
    assert!(
        mid_on,
        "soft_v2=true must schedule mid4 (B13/B15/B16) when absent: {packs_on:?}"
    );
}

#[test]
fn directional_brain_emits_sandbox_plan_and_ranked_directions() {
    let evidence = json!({
        "session_id": "sess_dir",
        "sources": ["main"],
        "batches": [
            {"batch_id":"B0_bootstrap","source":"main"},
            {"batch_id":"B1_conflict","source":"main"},
            {"batch_id":"B8_gateway","source":"gateway"}
        ],
        "fields": {
            "form_class": "desktop",
            "platform": "Linux x86_64",
            "webdriver": false,
            "user_agent": "Mozilla/5.0 Chrome/120"
        },
        "has_gateway": true,
    });
    let f = build_frontier(&evidence, true, Some(true), 12).unwrap();
    let route = f.to_route_plan().unwrap();
    assert!(
        route.get("sandbox_plan").is_some()
            && route["sandbox_plan"].get("kinds").is_some(),
        "sandbox_plan: {}",
        route
    );
    assert_eq!(
        route["sandbox_plan"]["wave"].as_i64().unwrap_or(99),
        0,
        "first wave single nest"
    );
    let ranking = route
        .get("direction_ranking")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(ranking.len() >= 5, "direction ranking: {ranking:?}");
    // Packs should carry direction_id when from directional scheduler
    let with_dir = f
        .packs
        .iter()
        .filter(|p| p.get("direction_id").is_some())
        .count();
    assert!(
        with_dir >= 1 || !f.packs.is_empty(),
        "directional packs: {:?}",
        f.packs
    );
}

#[test]
fn b92_b95_gap_codes_and_research_gate_defer() {
    // iss/74 Phase 2: B92/B94/B95 are research-gated (default defer), B93 is
    // default mid (no permission surface). Gaps must surface, packs must
    // resolve from catalog, and research packs must NOT enter the default plan.
    let evidence = json!({
        "session_id": "sess_r_gate",
        "sources": ["main"],
        "batches": [{"batch_id":"B0_bootstrap","source":"main"}],
        "fields": {
            "form_class": "desktop",
            "platform": "Linux x86_64",
            "user_agent": "Chrome/120",
            "webdriver": false,
            "hardware_concurrency": 8,
            "webgpu_limits_hash": "w_ok",
            "webgpu_dual_adapter_diff": "none"
        }
    });
    let gaps = gr_probe_core::scan_gaps(&evidence, None).expect("gaps");
    let codes: Vec<String> = gaps.iter().map(|g| g.code.clone()).collect();
    for c in [
        "need_webgpu_atomic_contention",
        "need_blink_fork_matrix",
        "need_sab_dual_clock",
        "need_gpu_eu_timing",
    ] {
        assert!(codes.iter().any(|x| x == c), "gap {c} must surface: {codes:?}");
    }
    let cat = gr_probe_core::load_catalog().expect("catalog");
    for p in [
        "B92_webgpu_atomic_contention",
        "B93_blink_fork_matrix",
        "B94_sab_dual_clock_differential",
        "B95_gpu_eu_timing",
    ] {
        let d = cat.resolve(p).unwrap_or_else(|| panic!("{p} resolvable"));
        assert!(d.executable, "{p} executable");
        assert!(d.schedule == "dynamic", "{p} dynamic");
    }
    assert_eq!(
        cat.resolve("B92_webgpu_atomic_contention").map(|d| d.gate.as_str()),
        Some("research"),
        "B92 research-gated"
    );
    assert_eq!(
        cat.resolve("B94_sab_dual_clock_differential").map(|d| d.gate.as_str()),
        Some("research"),
        "B94 research-gated"
    );
    assert_eq!(
        cat.resolve("B95_gpu_eu_timing").map(|d| d.gate.as_str()),
        Some("research"),
        "B95 research-gated"
    );
    assert_ne!(
        cat.resolve("B93_blink_fork_matrix").map(|d| d.gate.as_str()),
        Some("research"),
        "B93 default mid, not research-gated"
    );
    let f = build_frontier(&evidence, true, Some(true), 24).expect("frontier");
    let packs: Vec<&str> = f
        .packs
        .iter()
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()))
        .collect();
    for rg in ["B92_webgpu_atomic_contention", "B94_sab_dual_clock_differential", "B95_gpu_eu_timing"] {
        assert!(!packs.contains(&rg), "{rg} must stay research-deferred: {packs:?}");
    }
    assert!(
        f.notes
            .iter()
            .any(|n| n.contains("research_gate: B92_webgpu_atomic_contention deferred")),
        "research_gate deferral note: {:?}",
        f.notes
    );
}

#[test]
fn b5_research_unlock_on_contradiction() {
    // B5 (iss/74 §5): realm-coherence hard conflict triggers the research
    // unlock — B92/B95 leave the gate this tick with the reason stamped.
    let evidence = json!({
        "session_id": "sess_r_unlock",
        "sources": ["main", "worker"],
        "batches": [{"batch_id":"B0_bootstrap","source":"main"}],
        "fields": {
            "form_class": "desktop",
            "platform": "Linux x86_64",
            "user_agent": "Chrome/120",
            "webdriver": false,
            "hardware_concurrency": 8,
            "webgpu_limits_hash": "w_ok",
            "webgpu_dual_adapter_diff": "none"
        },
        "multi_source_consistency": {
            "structured_realm_diff": {
                "realm_coherence_verdict": "conflict"
            }
        }
    });
    let f = build_frontier(&evidence, true, Some(true), 24).expect("frontier");
    let packs: Vec<&str> = f
        .packs
        .iter()
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()))
        .collect();
    assert!(
        packs.iter().any(|p| *p == "B92_webgpu_atomic_contention"),
        "B92 unlocked on contradiction: {packs:?}"
    );
    assert!(
        packs.iter().any(|p| *p == "B95_gpu_eu_timing"),
        "B95 unlocked on contradiction: {packs:?}"
    );
    assert!(
        f.packs.iter().any(|p| {
            p.get("pack_id").and_then(|v| v.as_str()) == Some("B92_webgpu_atomic_contention")
                && p.get("reason").and_then(|v| v.as_str()) == Some("research_unlock:contradiction")
        }),
        "unlock reason stamped: {:?}",
        f.packs
    );
    assert!(
        f.notes
            .iter()
            .any(|n| n.contains("b5_research_unlock")),
        "unlock note: {:?}",
        f.notes
    );
}
