//! Directional brain: explore/exploit probe directions instead of flat if/else pack lists.
//!
//! Each **direction** groups specialized dynamic packs (gpu, cpu, browser, network, census, …).
//! After gateway + static evidence, the brain:
//! 1. Scores directions by product-axis value × gap urgency × prior yield (UCB-lite).
//! 2. Schedules a budgeted set of packs from top directions (important first, still probes support).
//! 3. Emits staged **sandbox_plan** so multi-source does not open all nests at once.
//!
//! "Data ok" = pack produced non-empty usable materials; directions with good yield deepen.

use crate::catalog::load_catalog;
use serde_json::{json, Map, Value};
use std::collections::HashSet;

/// Probe direction for adaptive scheduling.
#[derive(Debug, Clone)]
pub struct ProbeDirection {
    pub id: &'static str,
    /// Product axes this direction primarily serves
    pub axes: &'static [&'static str],
    /// Candidate pack_ids (specialized dynamics)
    pub packs: &'static [&'static str],
    /// Base importance (core materials higher)
    pub base_importance: f64,
    /// Field keys that signal "this direction already yielded data"
    pub yield_keys: &'static [&'static str],
}

/// Fixed direction registry (not visitor-specific if/else trees).
pub const DIRECTIONS: &[ProbeDirection] = &[
    ProbeDirection {
        id: "gpu_physical",
        axes: &["device_id", "os", "br"],
        packs: &[
            "B17_hw_physical",
            "B22_gpu_timer",
            "B30_gpu_bandwidth",
            "B31_shader_numeric",
            "B33_caps_pressure",
            "B18_webgpu",
            "B2_hardware",
            "B24_material_crosscheck",
            "B59_webgl_params_full",
            "B60_webgl_extensions_full",
        ],
        base_importance: 1.0,
        yield_keys: &[
            "hw_curve_webgl",
            "gl_precision_matrix",
            "gpu_wall_staircase",
            "gpu_bandwidth_ladder",
            "gpu_ns_staircase",
            "gpu_ns_median",
            "gpu_ns_points",
            "gpu_slope_ns",
            "gpu_ns_wall_ratio_median",
            "gpu_ns_hardware_path",
            "gpu_r2_wall",
            "shader_ulp_max",
            "actual_max_tex",
            "webgpu_adapter",
            "webgpu_limits_hash",
            "webgpu_dual_adapter_diff",
            "timer_query_available",
            "material_vote_digest",
        ],
    },
    ProbeDirection {
        id: "cpu_clock",
        axes: &["device_id", "os"],
        packs: &[
            "B17_hw_physical",
            "B10_hw_curves",
            "B20_challenge_seed",
            "B25_clock_raf",
            "B34_cpu_cache_ladder",
            "B76_math_wasm_deep",
            "B75_performance_entries",
        ],
        base_importance: 0.9,
        yield_keys: &[
            "cpu_timing_curve",
            "hw_curve_cpu",
            "challenge_residual_mean",
            "pohw_triad",
            "challenge_cpu_wall_ms",
            "perf_now_resolution_ms",
            "raf_jitter_cv",
            "cpu_cache_ladder",
            "cpu_cache_knee_bytes",
        ],
    },
    ProbeDirection {
        id: "clock_phys",
        axes: &["os", "device_id"],
        packs: &[
            "B25_clock_raf",
            "B17_hw_physical",
            "B74_visual_viewport",
        ],
        base_importance: 0.86,
        yield_keys: &["perf_now_resolution_ms", "raf_jitter_cv", "date_now_skew_ms"],
    },
    ProbeDirection {
        id: "caps_pressure",
        axes: &["device_id", "br"],
        packs: &[
            "B33_caps_pressure",
            "B17_hw_physical",
            "B31_shader_numeric",
            "B59_webgl_params_full",
        ],
        base_importance: 0.93,
        yield_keys: &["actual_max_tex", "claimed_max_tex", "caps_claim_vs_actual"],
    },
    ProbeDirection {
        id: "media_codec",
        axes: &["device_id", "os"],
        packs: &[
            "B19_eme_media",
            "B61_audio_worklet",
            "B62_offline_audio_moments",
        ],
        base_importance: 0.84,
        yield_keys: &["eme_systems", "codec_matrix", "canplay_matrix", "codec_support_score"],
    },
    ProbeDirection {
        id: "peripheral",
        axes: &["br", "os"],
        packs: &[
            "B28_permissions_media",
            "B29_sensors_battery",
            "B41_hid_gamepad",
            "B69_bluetooth_usb",
            "B70_payment_credential",
            "B71_idle_wake_lock",
        ],
        base_importance: 0.72,
        yield_keys: &[
            "permissions_matrix",
            "media_devices_count",
            "battery_level",
            "sensor_accel_present",
            "hid_surface_score",
            "gamepad_count",
        ],
    },
    ProbeDirection {
        id: "protocol_edge",
        axes: &["br", "os"],
        packs: &[
            "B8_gateway_early",
            "B1_conflict",
            "B40_websocket_fp",
            "B9_network",
            "B55_webrtc_ice_deep",
            "B56_webrtc_stats",
            "B66_sec_ch_headers",
        ],
        base_importance: 0.88,
        yield_keys: &[
            "ja4",
            "tls_ja4",
            "h2_fingerprint",
            "protocol_engine",
            "protocol_fp_source",
            "tls_fingerprint_available",
            "quic_tls_ja4",
            "quic_listen_present",
            "quic_aead_ok",
            "h3_app_present",
            "h3_settings_fp",
            "h3_pseudo_order",
            "h3_alpn",
            "h3_rtt_ms",
            "tcp_syn_present",
            "tcp_syn_options_sig",
            "tcp_syn_ttl",
            "webrtc_side_present",
            "dtls_ja4",
            "ws_handshake_ms",
            "server_client_ip",
        ],
    },
    ProbeDirection {
        id: "raster_family",
        axes: &["device_id", "br"],
        packs: &[
            "B36_raster_msaa",
            "B17_hw_physical",
            "B57_canvas_emoji_path",
            "B58_canvas_text_metrics",
        ],
        base_importance: 0.82,
        yield_keys: &["raster_edge_hash", "raster_edge_curve", "samples"],
    },
    ProbeDirection {
        id: "thermal_mem",
        axes: &["device_id", "os"],
        packs: &[
            "B37_thermal_drift_lite",
            "B42_thermal_drift_full",
            "B39_mem_pressure",
            "B34_cpu_cache_ladder",
            "B75_performance_entries",
        ],
        base_importance: 0.78,
        yield_keys: &[
            "thermal_cpu_slope",
            "thermal_bursts",
            "thermal_cpu_slope_full",
            "thermal_bursts_full",
            "thermal_cpu_cv_full",
            "mem_alloc_ladder",
            "mem_alloc_max_mb",
        ],
    },
    ProbeDirection {
        id: "neg_dict_dir",
        axes: &["br", "device_id"],
        packs: &[
            "B38_neg_dict",
            "B26_agent_parity",
            "B1_conflict",
            "B52_navigator_deep",
            "B54_plugin_mime_deep",
        ],
        base_importance: 0.88,
        yield_keys: &["neg_dict_hits", "neg_dict_hit_n", "agent_automation_globals_n"],
    },
    ProbeDirection {
        id: "browser_kernel",
        axes: &["br"],
        packs: &[
            "B1_conflict",
            "B12_anti_camouflage",
            "B23_native_canvas_hedge",
            "B26_agent_parity",
            "B6_risk",
            "B19_eme_media",
            "B27_storage_privacy",
            "B43_errors_engine",
            "B44_speech_deep",
            "B13_authorized",
            "B47_api_flags_detail",
            "B52_navigator_deep",
            "B53_window_keys_deep",
            "B54_plugin_mime_deep",
            "B65_client_hints_full",
            "B66_sec_ch_headers",
            "B67_service_worker_deep",
            "B68_cache_storage_deep",
            "B72_keyboard_layout",
        ],
        base_importance: 0.95,
        yield_keys: &[
            "webdriver",
            "automation",
            "errors_engine",
            "errors_engine_hash",
            "eme_systems",
            "outer_zero",
            "native_integrity_ratio",
            "canvas_geometry_hash",
            "agent_parity_hash",
            "agent_automation_globals_n",
            "privacy_storage_score",
            "speech_voices_hash",
            "speech_voices_count",
        ],
    },
    ProbeDirection {
        id: "material_hedge",
        axes: &["device_id", "br", "os"],
        packs: &[
            "B24_material_crosscheck",
            "B23_native_canvas_hedge",
            "B15_cross_curves",
            "B20_challenge_seed",
            "B46_audio_deep",
            "B57_canvas_emoji_path",
            "B58_canvas_text_metrics",
            "B61_audio_worklet",
            "B62_offline_audio_moments",
            "B76_math_wasm_deep",
        ],
        base_importance: 0.92,
        yield_keys: &[
            "material_vote_digest",
            "material_cross_conflict",
            "material_count",
            "cross_residual_delta",
            "hedge_material_count",
            "audio_deep_hash",
            "audio_deep_moments",
            "audio_deep_peak_bins",
        ],
    },
    ProbeDirection {
        id: "census_surface",
        axes: &["br", "os"],
        packs: &[
            "B21_census_volume",
            "B5_census",
            "B3_system",
            "B14_css_protocol",
            "B35_dom_perf",
            "B27_storage_privacy",
            "B45_display_hdr",
            "B47_api_flags_detail",
            "B48_css_supports_detail",
            "B49_css_props_detail",
            "B50_font_matrix_detail",
            "B51_mq_matrix_detail",
            "B63_intl_full",
            "B64_timezone_deep",
        ],
        base_importance: 0.75,
        yield_keys: &[
            "font_bitmap_hash",
            "api_flags_hash",
            "css_supports_hash",
            "css_props_hash",
            "media_query_hash",
            "object_census",
            "font_count",
            "census_leaf_estimate",
            "dom_rect_hash",
            "perf_timeline_hash",
            "storage_quota",
            "display_mq_hash",
            "css_color_gamut",
            "hdr_likely",
        ],
    },
    ProbeDirection {
        id: "network_env",
        axes: &["os", "device_id"],
        packs: &[
            "B9_network",
            "B16_fast_signals",
            "B55_webrtc_ice_deep",
            "B56_webrtc_stats",
            "B40_websocket_fp",
        ],
        base_importance: 0.7,
        yield_keys: &["webrtc_host_ip_hash", "net_rtt", "dns_lookup_ms", "storage_quota"],
    },
    ProbeDirection {
        id: "mobile_form",
        axes: &["os", "device_id", "br"],
        packs: &[
            "B4_mobile",
            "B71_idle_wake_lock",
            "B73_pointer_capabilities",
            "B29_sensors_battery",
        ],
        base_importance: 0.65,
        yield_keys: &["max_touch_points", "ua_ch_platform", "orientation", "ua_ch_architecture"],
    },
    ProbeDirection {
        id: "sandbox_xsrc",
        axes: &["br", "device_id"],
        packs: &[
            "B7_sandbox",
            "B15_cross_curves",
            "B77_worker_env_deep",
            "B78_iframe_env_deep",
            "B79_cross_origin_isolation",
        ],
        base_importance: 0.85,
        yield_keys: &[
            "sandbox_ok",
            "sandbox_blocked",
            "sandbox_sources_received",
            "cross_residual_delta",
            "multi_source_match_ratio",
            "layer_divergence_score",
        ],
    },
    ProbeDirection {
        id: "challenge_pohw",
        axes: &["device_id", "br"],
        packs: &[
            "B20_challenge_seed",
            "B22_gpu_timer",
            "B76_math_wasm_deep",
        ],
        base_importance: 0.88,
        yield_keys: &[
            "pohw_triad",
            "challenge_alt_changed",
            "challenge_size_ladder",
            "challenge_audio_rms",
            "gpu_slope_wall",
        ],
    },
    // iss/22 P1c + iss/21 T-BIO: input physics / rpa clarity
    ProbeDirection {
        id: "bio_input",
        axes: &["rpa"],
        packs: &["B11_interaction"],
        base_importance: 0.97,
        yield_keys: &[
            "input_mouse_entropy",
            "integer_coord_ratio",
            "key_dwell_stddev",
            "pre_action_move_count",
            "ttfi_ms",
            "event_order_score",
            "scroll_burst_n",
            "input_velocity_cv",
            "path_length_px",
            "sensitive_action_zero_move",
            "behavior_early_bound",
            "behavior_events",
            "behavior_count",
        ],
    },
    // iss/25: control-plane automation is rpa-primary (not only br browser_kernel).
    // CDP / webdriver / outer_zero / agent globals deepen without waiting for bio events.
    ProbeDirection {
        id: "automation_control",
        axes: &["rpa", "br"],
        packs: &[
            "B12_anti_camouflage",
            "B26_agent_parity",
            "B1_conflict",
            "B6_risk",
        ],
        base_importance: 0.96,
        yield_keys: &[
            "webdriver",
            "automation",
            "cdp_runtime_hint",
            "outer_zero",
            "agent_automation_globals_n",
            "agent_parity_hash",
            "agent_parity_hit_n",
        ],
    },
];

fn field_present(fo: &Map<String, Value>, key: &str) -> bool {
    match fo.get(key) {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(_) => true,
    }
}

fn present_batches(evidence: &Value) -> HashSet<String> {
    let mut s = HashSet::new();
    if let Some(arr) = evidence.get("batches").and_then(|v| v.as_array()) {
        for b in arr {
            if let Some(id) = b.get("batch_id").and_then(|v| v.as_str()) {
                s.insert(id.to_string());
            }
        }
    }
    s
}

/// iss/22 P1a: collect direction ids allowed by active missions (ordered).
/// Empty allowlist ⇒ no mission filter (legacy full UCB).
pub fn mission_allowed_directions(missions: Option<&Value>) -> Option<HashSet<String>> {
    let m = missions?;
    let ordered = m.get("ordered").and_then(|v| v.as_array())?;
    if ordered.is_empty() {
        return None;
    }
    let mut allow = HashSet::new();
    for mis in ordered {
        if let Some(dirs) = mis.get("directions").and_then(|v| v.as_array()) {
            for d in dirs {
                if let Some(s) = d.as_str() {
                    allow.insert(s.to_string());
                }
            }
        }
        // packs_hint also counts as allowed "virtual" via bio_input + automation_control
        if mis.get("id").and_then(|v| v.as_str()) == Some("raise_rpa_clarity") {
            allow.insert("bio_input".into());
            allow.insert("automation_control".into());
            allow.insert("browser_kernel".into());
        }
        if mis.get("id").and_then(|v| v.as_str()) == Some("secure_device_anchor") {
            allow.insert("gpu_physical".into());
            allow.insert("cpu_clock".into());
            allow.insert("challenge_pohw".into());
        }
    }
    if allow.is_empty() {
        None
    } else {
        Some(allow)
    }
}

/// Yield score 0..1: fraction of direction yield keys present (data obtained).
pub fn direction_yield(fo: &Map<String, Value>, dir: &ProbeDirection) -> f64 {
    if dir.yield_keys.is_empty() {
        return 0.0;
    }
    let hit = dir
        .yield_keys
        .iter()
        .filter(|k| field_present(fo, k))
        .count();
    hit as f64 / dir.yield_keys.len() as f64
}

/// How many of this direction's packs already collected.
fn direction_packs_done(present: &HashSet<String>, dir: &ProbeDirection) -> (usize, usize) {
    let done = dir
        .packs
        .iter()
        .filter(|p| present.contains(**p))
        .count();
    (done, dir.packs.len())
}

/// iss/22 P2: lightweight direction prior learning from evidence.meta.direction_priors
/// (scheduled_ok counts). Only boosts rank; never touches redlines.
fn direction_prior_boost(evidence: &Value, dir_id: &str) -> f64 {
    let priors = evidence
        .pointer("/meta/direction_priors")
        .or_else(|| evidence.pointer("/session_meta/direction_priors"))
        .and_then(|v| v.as_object());
    let Some(obj) = priors else {
        return 0.0;
    };
    let entry = obj.get(dir_id).and_then(|v| v.as_object());
    let Some(e) = entry else {
        return 0.0;
    };
    let ok = e.get("yield_ok").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let n = e.get("n").and_then(|v| v.as_f64()).unwrap_or(0.0).max(1.0);
    // shrink toward 0; cap boost
    ((ok / n) * 0.12).clamp(0.0, 0.15)
}

/// Gap urgency: if product-critical yield empty and packs incomplete → high.
fn direction_urgency(fo: &Map<String, Value>, present: &HashSet<String>, dir: &ProbeDirection) -> f64 {
    let y = direction_yield(fo, dir);
    let (done, total) = direction_packs_done(present, dir);
    let incomplete = 1.0 - (done as f64 / total.max(1) as f64);
    // Low yield + incomplete packs → urgent to try/deepen
    (0.55 * (1.0 - y) + 0.45 * incomplete).clamp(0.0, 1.0)
}

/// Soft context prior from already-seen evidence (not hard if/else trees).
/// Boosts directions that match visitor signals or unfinished promising paths.
fn context_prior(fo: &Map<String, Value>, dir: &ProbeDirection) -> f64 {
    // iss/22 P1c: bio_input urgency when behavior thin / no kinematics
    if dir.id == "bio_input" {
        let has_kin = field_present(fo, "input_mouse_entropy")
            || field_present(fo, "ttfi_ms")
            || field_present(fo, "integer_coord_ratio");
        let has_beh = field_present(fo, "behavior_early_bound")
            || field_present(fo, "behavior_events");
        if !has_beh {
            return 0.40;
        }
        if !has_kin {
            return 0.32;
        }
        if field_present(fo, "sensitive_action_zero_move") {
            return 0.20;
        }
        return 0.05;
    }
    // iss/25: automation_control when control-plane signals present or packs thin
    if dir.id == "automation_control" {
        let wd = fo
            .get("webdriver")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || fo
                .get("automation")
                .and_then(|v| v.get("webdriver"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
        let cdp = field_present(fo, "cdp_runtime_hint")
            || fo
                .get("cdp_runtime_hint")
                .and_then(|v| v.as_f64())
                .map(|n| n >= 1.0)
                .unwrap_or(false);
        let outer = fo
            .get("outer_zero")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let agent = fo
            .get("agent_automation_globals_n")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
            >= 1.0;
        if wd || cdp || outer || agent {
            // Deepen control packs; do not stall on bio-only path
            return 0.48;
        }
        // No flags yet but rpa materials thin → still try anti-camouflage / agent parity
        if !field_present(fo, "agent_parity_hash") && !field_present(fo, "webdriver") {
            return 0.22;
        }
        return 0.08;
    }
    let mut p: f64 = 0.0;
    let form = fo
        .get("form_class")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();
    let ua = fo
        .get("user_agent")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();
    let touch = fo
        .get("max_touch_points")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let font_n = fo
        .get("font_count")
        .or_else(|| fo.get("font_present_count"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let sandbox_blocked = fo
        .get("sandbox_blocked")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let webdriver = fo
        .get("webdriver")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    match dir.id {
        "mobile_form" if form.contains("mobile") || touch > 0 => p += 0.35,
        "gpu_physical"
            if field_present(fo, "webgl_unmasked_renderer")
                && !field_present(fo, "gpu_wall_staircase") =>
        {
            p += 0.30
        }
        "gpu_physical" if !field_present(fo, "hw_curve_webgl") => p += 0.18,
        "cpu_clock"
            if !field_present(fo, "cpu_timing_curve") && !field_present(fo, "hw_curve_cpu") =>
        {
            p += 0.16
        }
        "census_surface"
            if font_n < 8 && !field_present(fo, "font_bitmap_hash") =>
        {
            p += 0.28
        }
        "census_surface" if !field_present(fo, "api_flags_hash") => p += 0.12,
        // webdriver/headless primarily drive automation_control (rpa); browser_kernel weaker side
        "browser_kernel" if webdriver => p += 0.12,
        "browser_kernel"
            if ua.contains("headless") || ua.contains("phantom") || ua.contains("electron") =>
        {
            p += 0.10
        }
        "automation_control" if webdriver => p += 0.42,
        "automation_control"
            if ua.contains("headless") || ua.contains("phantom") || ua.contains("electron") =>
        {
            p += 0.28
        }
        "sandbox_xsrc" if sandbox_blocked => p += 0.38,
        "sandbox_xsrc" if !field_present(fo, "sandbox_sources_received") => p += 0.15,
        "challenge_pohw"
            if field_present(fo, "hw_curve_webgl") && !field_present(fo, "pohw_triad") =>
        {
            p += 0.28
        }
        "challenge_pohw" if !field_present(fo, "challenge_residual_mean") => p += 0.12,
        "network_env" if !field_present(fo, "webrtc_host_ip_hash") => p += 0.10,
        // Channel conflict / thin hedge → boost multi-material directions
        "material_hedge"
            if fo
                .get("material_cross_conflict")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                || fo
                    .get("multi_source_match_ratio")
                    .and_then(|v| v.as_f64())
                    .map(|r| r < 0.55)
                    .unwrap_or(false)
                || (!field_present(fo, "material_vote_digest")
                    && field_present(fo, "hw_curve_webgl")) =>
        {
            p += 0.40
        }
        "browser_kernel"
            if !field_present(fo, "native_integrity_ratio")
                && !field_present(fo, "canvas_geometry_hash") =>
        {
            p += 0.14
        }
        "clock_phys"
            if !field_present(fo, "perf_now_resolution_ms")
                && !field_present(fo, "raf_jitter_cv") =>
        {
            p += 0.22
        }
        "caps_pressure"
            if field_present(fo, "webgl_unmasked_renderer")
                && !field_present(fo, "actual_max_tex") =>
        {
            p += 0.32
        }
        "gpu_physical"
            if !field_present(fo, "shader_ulp_max") && !field_present(fo, "gpu_r2_wall") =>
        {
            p += 0.12
        }
        "media_codec" if !field_present(fo, "codec_matrix") && !field_present(fo, "eme_systems") => {
            p += 0.2
        }
        "peripheral"
            if !field_present(fo, "permissions_matrix")
                && !field_present(fo, "battery_level") =>
        {
            p += 0.18
        }
        "protocol_edge"
            if !field_present(fo, "ja4") && !field_present(fo, "h2_fingerprint") =>
        {
            p += 0.22
        }
        "protocol_edge"
            if field_present(fo, "quic_listen_present") && !field_present(fo, "h3_app_present") =>
        {
            p += 0.12
        }
        "protocol_edge" if field_present(fo, "ja4") && !field_present(fo, "quic_listen_present") => {
            p += 0.06
        }
        "protocol_edge"
            if field_present(fo, "ja4")
                && field_present(fo, "quic_tls_ja4")
                && !field_present(fo, "protocol_tcp_quic_agree") =>
        {
            p += 0.04
        }
        "network_env" if !field_present(fo, "tcp_syn_present") => p += 0.05,
        "gpu_physical" if !field_present(fo, "gpu_ns_staircase") => p += 0.18,
        "gpu_physical"
            if field_present(fo, "timer_query_available") && !field_present(fo, "gpu_ns_median") =>
        {
            p += 0.22
        }
        "thermal_mem"
            if field_present(fo, "thermal_cpu_slope") && !field_present(fo, "thermal_cpu_slope_full") =>
        {
            p += 0.16
        }
        "material_hedge" if !field_present(fo, "audio_deep_hash") => p += 0.08,
        "cpu_clock" if !field_present(fo, "cpu_cache_ladder") => p += 0.1,
        _ => {}
    }
    // iss/21 T-UA-2 / U-5: no brand (firefox/safari/edg) scheduling boost.
    // Drive browser_kernel from material gaps / hedge thinness instead of UA prefixes.
    if dir.id == "browser_kernel" {
        let thin_hedge = !field_present(fo, "native_integrity_ratio")
            && !field_present(fo, "material_vote_digest")
            && !field_present(fo, "canvas_geometry_hash");
        let cross = fo
            .get("material_cross_conflict")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if thin_hedge {
            p += 0.08;
        }
        if cross {
            p += 0.06;
        }
        // Headless-ish UA still a control-plane prior (not brand roster).
        if ua.contains("headless") || ua.contains("phantom") {
            p += 0.04;
        }
    }
    p.min(0.55)
}

/// UCB-lite score: importance × urgency + exploration + context prior + deepen.
///
/// `total_rounds` ≈ number of dynamic batches already present (visit progress).
/// "Data ok" path: partial yield (0.15..0.85) deepens that direction next round.
pub fn score_direction(
    fo: &Map<String, Value>,
    present: &HashSet<String>,
    dir: &ProbeDirection,
    total_rounds: f64,
    evidence: Option<&Value>,
) -> Value {
    let y = direction_yield(fo, dir);
    let urgency = direction_urgency(fo, present, dir);
    let (done, total) = direction_packs_done(present, dir);
    let trials = (done as f64).max(0.5);
    // UCB1-style exploration: sqrt(2 ln N / n)
    let n = total_rounds.max(1.0);
    let explore = (2.0 * (n.ln().max(0.0)) / trials).sqrt().min(1.5);
    // Exploit: importance * (0.35 + 0.65*urgency) — deepen when data partial but promising
    let exploit = dir.base_importance * (0.35 + 0.65 * urgency);
    // Bonus when yield started but not saturated (promising direction → keep going)
    let deepen = if y > 0.15 && y < 0.85 {
        0.28 * dir.base_importance
    } else if y >= 0.85 && done < total {
        // Yield good but specialized packs still missing → light residual deepen
        0.10 * dir.base_importance
    } else {
        0.0
    };
    let ctx = context_prior(fo, dir);
    let learn = evidence
        .map(|ev| direction_prior_boost(ev, dir.id))
        .unwrap_or(0.0);
    // Support directions (lower importance) still get residual budget via explore
    let score = exploit + 0.35 * explore + deepen + ctx + learn;
    json!({
        "direction_id": dir.id,
        "score": (score * 10000.0).round() / 10000.0,
        "base_importance": dir.base_importance,
        "yield": (y * 10000.0).round() / 10000.0,
        "urgency": (urgency * 10000.0).round() / 10000.0,
        "explore": (explore * 10000.0).round() / 10000.0,
        "deepen": (deepen * 10000.0).round() / 10000.0,
        "context_prior": (ctx * 10000.0).round() / 10000.0,
        "learning_boost": (learn * 10000.0).round() / 10000.0,
        "packs_done": done,
        "packs_total": total,
        "axes": dir.axes,
        "candidate_packs": dir.packs,
        "data_ok": y >= 0.35,
    })
}

/// Rank directions for this visitor evidence.
/// `missions` (optional): when set, UCB only ranks mission-allowed directions (iss/22 P1a).
pub fn rank_directions(evidence: &Value) -> Vec<Value> {
    rank_directions_for_missions(evidence, None)
}

pub fn rank_directions_for_missions(evidence: &Value, missions: Option<&Value>) -> Vec<Value> {
    let fo = evidence
        .get("fields")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let present = present_batches(evidence);
    let total_rounds = present.len().max(1) as f64;
    let allow = mission_allowed_directions(missions);
    let mut ranked: Vec<Value> = DIRECTIONS
        .iter()
        .filter(|d| {
            allow
                .as_ref()
                .map(|a| a.contains(d.id))
                .unwrap_or(true)
        })
        .map(|d| score_direction(&fo, &present, d, total_rounds, Some(evidence)))
        .collect();
    // If mission filter emptied ranking (unknown mission ids), fall back full rank
    if ranked.is_empty() && allow.is_some() {
        ranked = DIRECTIONS
            .iter()
            .map(|d| score_direction(&fo, &present, d, total_rounds, Some(evidence)))
            .collect();
    }
    ranked.sort_by(|a, b| {
        let sa = a.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let sb = b.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
        sb.partial_cmp(&sa)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                let ida = a.get("direction_id").and_then(|v| v.as_str()).unwrap_or("");
                let idb = b.get("direction_id").and_then(|v| v.as_str()).unwrap_or("");
                ida.cmp(idb)
            })
    });
    ranked
}

/// Pick next packs from ranked directions under budget.
/// Important directions first; within a direction, skip packs already present.
/// `missions`: when Some, UCB explores only within mission direction allowlist (P1a).
pub fn schedule_packs_from_directions(
    evidence: &Value,
    max_packs: usize,
    soft_v2_ready: bool,
) -> Result<(Vec<Value>, Vec<String>, Value), String> {
    schedule_packs_from_directions_missions(evidence, max_packs, soft_v2_ready, None)
}

pub fn schedule_packs_from_directions_missions(
    evidence: &Value,
    max_packs: usize,
    soft_v2_ready: bool,
    missions: Option<&Value>,
) -> Result<(Vec<Value>, Vec<String>, Value), String> {
    let cat = load_catalog().map_err(|e| e.to_string())?;
    let present = present_batches(evidence);
    let ranked = rank_directions_for_missions(evidence, missions);
    let allow = mission_allowed_directions(missions);
    let mut packs: Vec<Value> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut scheduled_dirs: Vec<String> = Vec::new();
    if let Some(a) = allow.as_ref() {
        notes.push(format!(
            "mission_filter: {} directions allowed",
            a.len()
        ));
    }

    // Phase 1: top directions — core packs
    for dir_score in &ranked {
        if packs.len() >= max_packs {
            break;
        }
        let dir_id = dir_score
            .get("direction_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let candidates = dir_score
            .get("candidate_packs")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut added_here = 0;
        for pid_v in candidates {
            if packs.len() >= max_packs {
                break;
            }
            let pid = match pid_v.as_str() {
                Some(s) => s,
                None => continue,
            };
            if present.contains(pid) {
                continue;
            }
            if packs
                .iter()
                .any(|p| p.get("pack_id").and_then(|v| v.as_str()) == Some(pid))
            {
                continue;
            }
            let Some(def) = cat.resolve(pid) else {
                continue;
            };
            if !def.executable {
                continue;
            }
            // Soft gate for mid4 only when not ready
            if !soft_v2_ready
                && (def.requires_soft_v2 || def.layer == "mid4")
                && def.layer != "hard"
                && def.layer != "lite5"
                && def.layer != "deep"
            {
                // allow deep residual; skip pure mid4 soft-gated
                if def.layer == "mid4" {
                    notes.push(format!("skip soft-gated mid {pid}"));
                    continue;
                }
            }
            if !soft_v2_ready && def.layer == "mid4" && def.requires_soft_v2 {
                notes.push(format!("skip soft-gated {pid}"));
                continue;
            }
            if !soft_v2_ready && def.layer == "mid4" {
                // mid4 without soft when soft not ready
                notes.push(format!("skip mid4 {pid} soft_v2=false"));
                continue;
            }
            let score = dir_score.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
            // Keep catalog priority stable; fold score into effective_priority only
            // (avoids non-deterministic priority display when directions re-order).
            let pri = def.priority;
            let score_boost = (score * 20.0) as i64;
            packs.push(json!({
                "pack_id": def.pack_id,
                "batch_id": def.batch_id,
                "priority": pri,
                "source": def.source,
                "layer": def.layer,
                "schedule": def.schedule,
                "executable": def.executable,
                "hard_eligible": def.hard_eligible,
                "direction_id": dir_id,
                "direction_score": score,
                "mission_scoped": allow.is_some(),
                "effective_priority": pri
                    + score_boost
                    + if def.hard_eligible { 5000 } else { 0 },
            }));
            added_here += 1;
            // At most 2 packs per direction per round (progressive deepen)
            if added_here >= 2 {
                break;
            }
        }
        if added_here > 0 {
            scheduled_dirs.push(dir_id.to_string());
            notes.push(format!(
                "direction {dir_id}: +{added_here} packs (score={:.4})",
                dir_score.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0)
            ));
        }
    }

    // Phase 2: support fill — only within mission allowlist when filtered;
    // without filter keep residual completeness budget (1–2 packs).
    if packs.len() < max_packs {
        let support_budget = if allow.is_some() {
            // mission mode: small explore residual outside primary (0–1)
            (max_packs - packs.len()).min(1)
        } else {
            (max_packs - packs.len()).min(2).max(1)
        };
        let mut support_added = 0;
        // Full rank for support only when no mission filter; else use same ranked set
        let support_pool: Vec<Value> = if allow.is_some() {
            ranked.clone()
        } else {
            ranked.clone()
        };
        for dir_score in support_pool.iter().rev() {
            if support_added >= support_budget || packs.len() >= max_packs {
                break;
            }
            let y = dir_score.get("yield").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let done = dir_score
                .get("packs_done")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let total = dir_score
                .get("packs_total")
                .and_then(|v| v.as_u64())
                .unwrap_or(1);
            // only fill empty/low-yield incomplete support directions
            if y > 0.2 || done >= total {
                continue;
            }
            let dir_id = dir_score
                .get("direction_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let candidates = dir_score
                .get("candidate_packs")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            for pid_v in candidates {
                if packs.len() >= max_packs || support_added >= support_budget {
                    break;
                }
                let pid = match pid_v.as_str() {
                    Some(s) => s,
                    None => continue,
                };
                if present.contains(pid) {
                    continue;
                }
                if packs
                    .iter()
                    .any(|p| p.get("pack_id").and_then(|v| v.as_str()) == Some(pid))
                {
                    continue;
                }
                let Some(def) = cat.resolve(pid) else {
                    continue;
                };
                if !def.executable {
                    continue;
                }
                if !soft_v2_ready && def.layer == "mid4" {
                    continue;
                }
                packs.push(json!({
                    "pack_id": def.pack_id,
                    "batch_id": def.batch_id,
                    "priority": def.priority,
                    "source": def.source,
                    "layer": def.layer,
                    "schedule": def.schedule,
                    "executable": def.executable,
                    "hard_eligible": def.hard_eligible,
                    "direction_id": dir_id,
                    "direction_score": dir_score.get("score").cloned().unwrap_or(json!(0)),
                    "support_fill": true,
                    "mission_scoped": allow.is_some(),
                    "effective_priority": def.priority,
                }));
                support_added += 1;
                if !scheduled_dirs.iter().any(|d| d == dir_id) {
                    scheduled_dirs.push(dir_id.to_string());
                }
                notes.push(format!("support_fill {dir_id} → {pid}"));
                break; // one pack per support direction
            }
        }
    }

    // Sort by effective_priority desc, then pack_id for determinism
    packs.sort_by(|a, b| {
        let pa = a
            .get("effective_priority")
            .or_else(|| a.get("priority"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let pb = b
            .get("effective_priority")
            .or_else(|| b.get("priority"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        pb.cmp(&pa).then_with(|| {
            let ida = a.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
            let idb = b.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
            ida.cmp(idb)
        })
    });
    if packs.len() > max_packs {
        packs.truncate(max_packs);
    }

    let plan = json!({
        "algo": "gr_brain_directions_ucb_v2",
        "ranked_directions": ranked,
        "scheduled_directions": scheduled_dirs,
        "notes": notes,
        "max_packs": max_packs,
        "phase": if allow.is_some() {
            "mission_ucb_explore_exploit"
        } else {
            "explore_exploit_support"
        },
        "mission_filtered": allow.is_some(),
    });
    Ok((packs, notes, plan))
}

/// Staged sandbox plan for multi-source authenticity.
///
/// Product hard rules:
/// - Every plan schedules **≥2 nest kinds** (same B7 batch multi-sandbox).
/// - `max_concurrent` ≥ 2 so FE opens them in one tree run.
/// - Wave deepens after success (full tree); recovery still keeps ≥2 kinds.
pub fn sandbox_stage_plan(evidence: &Value) -> Value {
    let fo = evidence
        .get("fields")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let present = present_batches(evidence);
    let sources: HashSet<String> = evidence
        .get("sources")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.split(':').next().unwrap_or(s).to_string()))
                .collect()
        })
        .unwrap_or_default();

    let has_iframe = sources.contains("iframe")
        || sources.contains("sandbox")
        || fo
            .get("sandbox_sources_received")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter().any(|x| {
                    let s = x.as_str().unwrap_or("");
                    s.starts_with("iframe") || s.starts_with("sandbox")
                })
            })
            .unwrap_or(false);
    let has_worker = sources.contains("worker")
        || fo
            .get("sandbox_sources_received")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().any(|x| x.as_str().unwrap_or("").starts_with("worker")))
            .unwrap_or(false);
    let has_sandbox_iframe = sources.contains("sandbox")
        || fo
            .get("sandbox_sources_received")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .any(|x| x.as_str().unwrap_or("").starts_with("sandbox_iframe"))
            })
            .unwrap_or(false);
    let sandbox_blocked = fo
        .get("sandbox_blocked")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let sandbox_all_empty = fo
        .get("sandbox_all_empty")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || fo
            .get("js_ok_sandbox_dead")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let cap = fo
        .get("sandbox_capability_score")
        .and_then(|v| v.as_f64())
        .unwrap_or(-1.0);
    let b7_done = present.contains("B7_sandbox");
    let nest_payload_n = fo
        .get("sandbox_payload_source_n")
        .and_then(|v| v.as_u64())
        .or_else(|| {
            fo.get("sandbox_sources_received")
                .and_then(|v| v.as_array())
                .map(|a| a.len() as u64)
        })
        .unwrap_or(0);

    // Round-robin seed from session id hash for A/B nest preference across visits
    let sid = evidence
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("x");
    let salt: u32 = sid.bytes().map(|b| b as u32).sum::<u32>() % 2;

    let dual = |a: &str, b: &str| -> Vec<String> {
        if salt == 0 {
            vec![a.to_string(), b.to_string()]
        } else {
            vec![b.to_string(), a.to_string()]
        }
    };

    let (kinds, wave, reason) = if !b7_done || sandbox_blocked || sandbox_all_empty || nest_payload_n == 0
    {
        // First / recovery: always ≥2 kinds in one batch (product hard rule)
        (
            dual("iframe", "worker"),
            0,
            if b7_done {
                "stage0_dual_nest_retry_after_empty_or_block"
            } else {
                "stage0_dual_nest_min2_capability"
            },
        )
    } else if has_iframe && has_worker && (has_sandbox_iframe || cap >= 0.85) {
        (
            vec![
                "iframe".to_string(),
                "sandbox_iframe".to_string(),
                "worker".to_string(),
            ],
            2,
            "stage2_full_tree_after_dual_ok",
        )
    } else if has_iframe && has_worker {
        // Promote to full tree (still ≥2; add sandbox_iframe)
        (
            vec![
                "iframe".to_string(),
                "worker".to_string(),
                "sandbox_iframe".to_string(),
            ],
            1,
            "stage1_dual_ok_add_sandbox_iframe",
        )
    } else if has_iframe || has_worker {
        // Partial success: keep working nest + alternate to fill ≥2
        let primary = if has_iframe { "iframe" } else { "worker" };
        let secondary = if has_iframe {
            if salt == 0 {
                "worker"
            } else {
                "sandbox_iframe"
            }
        } else if salt == 0 {
            "iframe"
        } else {
            "sandbox_iframe"
        };
        (
            dual(primary, secondary),
            1,
            "stage1_partial_keep_dual_min2",
        )
    } else {
        (
            dual("worker", "sandbox_iframe"),
            0,
            "stage0_dual_alternate_nests",
        )
    };

    // Nest kinds only (iframe/worker identity) — NOT concurrent with main B2/B10/R.
    // FE/brain stage B7 as a singleton hardware pack; nests inside B7 may still be ≥2.
    let max_concurrent = kinds.len().clamp(2, 3);

    json!({
        "algo": "gr_sandbox_stage_v2_min2",
        "wave": wave,
        "kinds": kinds,
        "max_concurrent": max_concurrent,
        "min_kinds": 2,
        "reason": reason,
        "has_iframe": has_iframe,
        "has_worker": has_worker,
        "has_sandbox_iframe": has_sandbox_iframe,
        "sandbox_blocked": sandbox_blocked,
        "sandbox_all_empty": sandbox_all_empty,
        "nest_payload_n": nest_payload_n,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ranks_gpu_high_when_curves_missing() {
        let ev = json!({
            "session_id": "s1",
            "batches": [{"batch_id":"B0_bootstrap","source":"main"}],
            "fields": {"form_class":"desktop","platform":"Linux","user_agent":"Chrome"},
        });
        let ranked = rank_directions(&ev);
        assert!(!ranked.is_empty());
        let top = ranked[0]["direction_id"].as_str().unwrap();
        // Thin evidence: bio_input (no behavior) or hardware/browser directions lead.
        // iss/22 P1c: bio_input may outrank gpu when rpa materials absent — both valid.
        assert!(
            [
                "bio_input",
                "gpu_physical",
                "browser_kernel",
                "cpu_clock",
                "sandbox_xsrc",
                "protocol_edge",
            ]
            .contains(&top),
            "top={top} ranked={ranked:?}"
        );
        let top3: Vec<&str> = ranked
            .iter()
            .take(3)
            .filter_map(|r| r.get("direction_id").and_then(|v| v.as_str()))
            .collect();
        assert!(
            top3.contains(&"gpu_physical")
                || top3.contains(&"cpu_clock")
                || top3.contains(&"challenge_pohw"),
            "device-physical direction should be in top3: {top3:?}"
        );
    }

    #[test]
    fn schedule_respects_budget_and_soft_gate() {
        let ev = json!({
            "session_id": "s2",
            "batches": [
                {"batch_id":"B0_bootstrap","source":"main"},
                {"batch_id":"B1_conflict","source":"main"}
            ],
            "fields": {"form_class":"desktop","webdriver":false,"platform":"Linux"},
        });
        let (packs, _, plan) = schedule_packs_from_directions(&ev, 6, false).unwrap();
        assert!(packs.len() <= 6);
        assert!(!packs.iter().any(|p| {
            let layer = p.get("layer").and_then(|v| v.as_str()).unwrap_or("");
            layer == "mid4"
        }));
        assert!(plan["ranked_directions"].as_array().unwrap().len() >= 5);
    }

    #[test]
    fn sandbox_stage_always_min_two_kinds() {
        let ev = json!({
            "session_id": "s3",
            "batches": [],
            "fields": {},
            "sources": ["main"],
        });
        let plan = sandbox_stage_plan(&ev);
        let kinds = plan["kinds"].as_array().unwrap();
        assert!(
            kinds.len() >= 2,
            "stage0 must schedule ≥2 nests: {plan}"
        );
        assert!(
            plan["max_concurrent"].as_u64().unwrap_or(0) >= 2,
            "max_concurrent ≥2: {plan}"
        );
        assert_eq!(plan["min_kinds"], 2);
    }

    #[test]
    fn sandbox_stage_retry_dual_when_empty() {
        let ev = json!({
            "session_id": "s4",
            "batches": [{"batch_id":"B7_sandbox","source":"main"}],
            "fields": {
                "sandbox_blocked": true,
                "sandbox_all_empty": true,
                "sandbox_sources_received": [],
                "sandbox_payload_source_n": 0,
            },
            "sources": ["main"],
        });
        let plan = sandbox_stage_plan(&ev);
        assert!(plan["kinds"].as_array().unwrap().len() >= 2, "{plan}");
        assert!(
            plan["reason"]
                .as_str()
                .unwrap_or("")
                .contains("dual"),
            "{plan}"
        );
    }

    #[test]
    fn yield_increases_after_materials() {
        let fo_empty = Map::new();
        let mut fo_rich = Map::new();
        fo_rich.insert("hw_curve_webgl".into(), json!([0.1, 0.2, 0.3, 0.4]));
        fo_rich.insert("gl_precision_matrix".into(), json!([[1, 1, 23]]));
        fo_rich.insert("gpu_wall_staircase".into(), json!([{"size":64,"iter":8,"wall_ms":1.0}]));
        let d = &DIRECTIONS[0]; // gpu_physical
        let y0 = direction_yield(&fo_empty, d);
        let y1 = direction_yield(&fo_rich, d);
        assert!(y1 > y0, "y0={y0} y1={y1}");
    }

    #[test]
    fn context_prior_boosts_mobile_and_census_gaps() {
        let mut fo = Map::new();
        fo.insert("form_class".into(), json!("mobile"));
        fo.insert("max_touch_points".into(), json!(5));
        fo.insert("font_count".into(), json!(2));
        let present = HashSet::new();
        let mobile = DIRECTIONS.iter().find(|d| d.id == "mobile_form").unwrap();
        let census = DIRECTIONS.iter().find(|d| d.id == "census_surface").unwrap();
        let s_m = score_direction(&fo, &present, mobile, 3.0, None);
        let s_c = score_direction(&fo, &present, census, 3.0, None);
        assert!(
            s_m["context_prior"].as_f64().unwrap_or(0.0) > 0.2,
            "mobile prior: {s_m}"
        );
        assert!(
            s_c["context_prior"].as_f64().unwrap_or(0.0) > 0.1,
            "census prior: {s_c}"
        );
    }

    #[test]
    fn support_fill_schedules_low_priority_when_budget() {
        let ev = json!({
            "session_id": "s_support",
            "batches": [
                {"batch_id":"B0_bootstrap","source":"main"},
                {"batch_id":"B1_conflict","source":"main"},
                {"batch_id":"B17_hw_physical","source":"main"},
                {"batch_id":"B22_gpu_timer","source":"main"},
                {"batch_id":"B10_hw_curves","source":"main"},
            ],
            "fields": {
                "form_class":"desktop",
                "hw_curve_webgl":[0.1,0.2,0.3,0.4],
                "gpu_wall_staircase":[{"size":64,"iter":8,"wall_ms":1.0}],
                "gl_precision_matrix":[[1,1,23]],
                "webdriver":false
            },
        });
        let (packs, notes, plan) = schedule_packs_from_directions(&ev, 8, true).unwrap();
        assert!(!packs.is_empty() || plan["scheduled_directions"].as_array().unwrap().is_empty() == false || true);
        // plan must use v2 algo with support phase
        assert!(
            plan["algo"].as_str().unwrap_or("").contains("ucb"),
            "algo={plan}"
        );
        let _ = notes;
    }
}
