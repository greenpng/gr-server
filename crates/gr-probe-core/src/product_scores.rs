//! Product safety scores: os / br / rpa (norm/10).
//!
//! Higher score = safer (more like real OS / real browser / human).
//! Consumes shared commercial fields from field_product_matrix — polarity-flipped
//! risk signals. No host/env allowlists.

use crate::bot::BotScore;
use crate::corroborate::{count_material_families, fuse_axis_hedge};
use crate::product_matrix::load_field_product_matrix;
use crate::stack_auth::StackAuth;
use crate::xsrc::{TruthResult, XSRC_CONFLICT};
use serde_json::{json, Map, Value};

fn clamp01(x: f64) -> f64 {
    if x.is_nan() {
        0.0
    } else {
        x.clamp(0.0, 1.0)
    }
}

fn f_field(fields: &Map<String, Value>, key: &str) -> Option<f64> {
    fields.get(key).and_then(|v| {
        v.as_f64()
            .or_else(|| v.as_i64().map(|i| i as f64))
            .or_else(|| v.as_u64().map(|u| u as f64))
    })
}

fn bool_field(fields: &Map<String, Value>, key: &str) -> Option<bool> {
    fields.get(key).and_then(|v| v.as_bool())
}

fn nested_bool(fields: &Map<String, Value>, parent: &str, key: &str) -> Option<bool> {
    fields
        .get(parent)
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_bool())
}

fn has_nonempty(fields: &Map<String, Value>, key: &str) -> bool {
    match fields.get(key) {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(_) => true,
    }
}

fn str_field<'a>(fields: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    fields.get(key).and_then(|v| v.as_str()).filter(|s| !s.is_empty())
}

/// Device-id silicon materials re-used for os/br/rpa axes (session-local; not cross-vtid).
/// Full raw curves / residual = trust-boost; empty-curve desktop claim or pure-zero residual =
/// demote. Aligns product scores with multi-segment mint materials (iss/45–46).
fn apply_device_silicon_probe_evidence(
    fo: &Map<String, Value>,
    risk: &mut f64,
    reasons: &mut Vec<String>,
    hits: &mut Vec<String>,
    axis: &str,
) {
    let wg = fo
        .get("hw_curve_webgl")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let au = fo
        .get("hw_curve_audio")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let has_res = f_field(fo, "residual_mean").is_some() || f_field(fo, "residual_std").is_some();
    if wg >= 4 {
        hits.push(format!("{axis}_silicon_webgl_curve"));
        *risk = (*risk - 0.02).max(0.0);
        reasons.push(format!("{axis}_hw_curve_webgl_present"));
    }
    if au >= 8 {
        hits.push(format!("{axis}_silicon_audio_curve"));
        *risk = (*risk - 0.02).max(0.0);
        reasons.push(format!("{axis}_hw_curve_audio_present"));
    }
    if has_res {
        hits.push(format!("{axis}_residual_materials"));
    }
    // WebCodecs preference matrix (iss/45–46): conf/side only — not device_id material.
    if has_nonempty(fo, "webcodec_hw_matrix_hash") {
        hits.push(format!("{axis}_webcodec_pref_matrix"));
        *risk = (*risk - 0.015).max(0.0);
        reasons.push(format!("{axis}_webcodec_matrix_present"));
        // Soft stack + zero webcodec support → mild demote (VM often thin)
        let n = f_field(fo, "webcodec_supported_n").unwrap_or(0.0);
        if n < 1.0 {
            *risk = (*risk + 0.03).min(1.0);
            reasons.push(format!("{axis}_webcodec_empty_matrix"));
        }
    }
    // Gateway protocol observers (ja4t / h2 partial) — br/rpa conf assist only.
    // iss/46 H2: h2_priority / h3_pseudo are diagnostic-only — hit for coverage, never uniqueness.
    if axis == "br" || axis == "rpa" {
        if has_nonempty(fo, "ja4t") || has_nonempty(fo, "tcp_syn_ja4t") {
            hits.push(format!("{axis}_ja4t_observer"));
            *risk = (*risk - 0.01).max(0.0);
        }
        if has_nonempty(fo, "h2_priority_fingerprint") {
            hits.push(format!("{axis}_h2_priority_diagnostic"));
            reasons.push(format!("{axis}_h2_priority_diagnostic_only"));
            // no risk shift — not hard browser uniqueness
        }
        if has_nonempty(fo, "quic_tp_summary") || has_nonempty(fo, "quic_key_share_groups") {
            hits.push(format!("{axis}_quic_depth_observer"));
            *risk = (*risk - 0.01).max(0.0);
        }
    }
    // Desktop claim without any silicon curve → mild demote (soft/VM often thin).
    let form = str_field(fo, "form_class").unwrap_or("");
    if (form == "desktop" || form.is_empty()) && wg < 4 && au < 8 && !has_res {
        *risk = (*risk + 0.04).min(1.0);
        reasons.push(format!("{axis}_desktop_without_silicon_curves"));
    }
}

/// Apply high-value probe materials that were previously emitted but under-consumed.
/// Non-decorative: always touches risk and/or hits/reasons when material is present.
/// Missing/skip signals demote conf (increase risk slightly) — they do not halt probe.
fn apply_probe_surface_evidence_os(
    fo: &Map<String, Value>,
    risk: &mut f64,
    reasons: &mut Vec<String>,
    hits: &mut Vec<String>,
) {
    apply_device_silicon_probe_evidence(fo, risk, reasons, hits, "os");
    // Residual std — silicon noise / template detection
    if let Some(rs) = f_field(fo, "residual_std") {
        hits.push("residual_std".into());
        if rs < 1e-6 {
            *risk = (*risk + 0.08).min(1.0);
            reasons.push("residual_std_too_stable".into());
        } else if rs > 0.05 && rs < 0.5 {
            *risk = (*risk - 0.03).max(0.0);
            reasons.push("residual_std_present".into());
        } else {
            *risk = (*risk - 0.01).max(0.0);
            reasons.push("residual_std_observed".into());
        }
    }
    // Canvas / audio noise digests (previously silent)
    if has_nonempty(fo, "canvas_noise_hash") || has_nonempty(fo, "canvas_2d_hash") {
        hits.push("canvas_noise_hash".into());
        *risk = (*risk - 0.02).max(0.0);
        reasons.push("canvas_noise_materials".into());
    }
    if has_nonempty(fo, "challenge_pixel_sample") {
        hits.push("challenge_pixel_sample".into());
        *risk = (*risk - 0.01).max(0.0);
        reasons.push("challenge_pixel_sample".into());
    }
    if has_nonempty(fo, "audio_noise_energy")
        || has_nonempty(fo, "audio_noise_algo")
        || has_nonempty(fo, "audio_deep_curve")
    {
        hits.push("audio_noise_materials".into());
        *risk = (*risk - 0.02).max(0.0);
        if has_nonempty(fo, "audio_deep_skip") || has_nonempty(fo, "audio_deep_error") {
            *risk = (*risk + 0.03).min(1.0);
            reasons.push("audio_deep_gap".into());
        } else {
            reasons.push("audio_noise_present".into());
        }
    }
    if f_field(fo, "audio_base_latency").is_some() || f_field(fo, "audio_output_latency").is_some() {
        hits.push("audio_latency".into());
        *risk = (*risk - 0.01).max(0.0);
    }
    // H01 GPU-ns async / points (aliases + readback quality)
    if f_field(fo, "gpu_ns_async_frames").is_some()
        || f_field(fo, "h01_points").is_some()
        || bool_field(fo, "gpu_timer_query_available").is_some()
        || bool_field(fo, "timer_query_available").is_some()
    {
        hits.push("gpu_ns_async_h01".into());
        let tq = bool_field(fo, "gpu_timer_query_available")
            .or_else(|| bool_field(fo, "timer_query_available"))
            .unwrap_or(false);
        let readback = bool_field(fo, "gpu_ns_readback_ok").unwrap_or(false);
        let pts = f_field(fo, "h01_points")
            .or_else(|| f_field(fo, "gpu_staircase_points"))
            .unwrap_or(0.0);
        if tq && readback {
            *risk = (*risk - 0.04).max(0.0);
            reasons.push("gpu_ns_timer_readback_ok".into());
        } else if tq && !readback {
            *risk = (*risk + 0.05).min(1.0);
            reasons.push("gpu_ns_timer_available_no_readback".into());
        } else if !tq {
            *risk = (*risk + 0.02).min(1.0);
            reasons.push("gpu_ns_timer_unavailable".into());
        }
        if pts >= 4.0 {
            *risk = (*risk - 0.02).max(0.0);
            hits.push("h01_points".into());
        }
    }
    // Caps claim vs actual (aliases)
    if f_field(fo, "caps_actual_max_tex").is_some()
        || f_field(fo, "actual_max_tex").is_some()
        || f_field(fo, "caps_ok_n").is_some()
    {
        hits.push("caps_actual_max_tex".into());
        let claimed = f_field(fo, "caps_claimed_max_tex")
            .or_else(|| f_field(fo, "claimed_max_tex"))
            .or_else(|| f_field(fo, "gl_max_texture_size"))
            .unwrap_or(0.0);
        let actual = f_field(fo, "caps_actual_max_tex")
            .or_else(|| f_field(fo, "actual_max_tex"))
            .unwrap_or(0.0);
        let ok_n = f_field(fo, "caps_ok_n").unwrap_or(0.0);
        if claimed > 0.0 && actual > 0.0 && actual < claimed * 0.5 {
            *risk = (*risk + 0.12).min(1.0);
            reasons.push("caps_actual_far_below_claim".into());
        } else if ok_n >= 3.0 {
            *risk = (*risk - 0.03).max(0.0);
            reasons.push("caps_pressure_alloc_ok".into());
        } else if has_nonempty(fo, "caps_skip") || has_nonempty(fo, "caps_error") {
            *risk = (*risk + 0.03).min(1.0);
            reasons.push("caps_pressure_gap".into());
        }
    }
    // H08 clock aliases
    if f_field(fo, "clock_resolution_ms").is_some()
        || f_field(fo, "perf_now_resolution_ms").is_some()
        || f_field(fo, "raf_cv").is_some()
        || f_field(fo, "raf_jitter_cv").is_some()
    {
        hits.push("clock_raf_h08".into());
        let res = f_field(fo, "clock_resolution_ms")
            .or_else(|| f_field(fo, "perf_now_resolution_ms"))
            .unwrap_or(0.0);
        if res >= 0.5 {
            *risk = (*risk + 0.08).min(1.0);
            reasons.push("clock_resolution_coarse".into());
        } else if res > 0.0 && res < 0.05 {
            *risk = (*risk - 0.02).max(0.0);
            reasons.push("clock_resolution_fine".into());
        }
        if let Some(cv) = f_field(fo, "raf_cv").or_else(|| f_field(fo, "raf_jitter_cv")) {
            hits.push("raf_cv".into());
            if cv > 0.35 {
                *risk = (*risk + 0.04).min(1.0);
                reasons.push("raf_jitter_high".into());
            } else {
                *risk = (*risk - 0.01).max(0.0);
            }
        }
    }
    // PoHW / challenge completeness
    if bool_field(fo, "pohw_triad_ok").unwrap_or(false)
        || has_nonempty(fo, "pohw_triad")
        || f_field(fo, "challenge_cv").is_some()
        || f_field(fo, "challenge_avg_cv").is_some()
        || f_field(fo, "pohw_residual_mean").is_some()
        || f_field(fo, "pohw_unique_n").is_some()
        || bool_field(fo, "challenge_seed_present").is_some()
    {
        hits.push("pohw_challenge".into());
        if bool_field(fo, "pohw_triad_ok").unwrap_or(false) {
            *risk = (*risk - 0.03).max(0.0);
            reasons.push("pohw_triad_ok".into());
        }
        if f_field(fo, "pohw_residual_mean").is_some() {
            hits.push("pohw_residual_mean".into());
            *risk = (*risk - 0.01).max(0.0);
        }
        if let Some(un) = f_field(fo, "pohw_unique_n") {
            hits.push("pohw_unique_n".into());
            if un <= 1.0 {
                *risk = (*risk + 0.08).min(1.0);
                reasons.push("pohw_unique_n_low".into());
            }
        }
        if bool_field(fo, "pohw_alt_changed").unwrap_or(false) {
            hits.push("pohw_alt_changed".into());
            *risk = (*risk - 0.02).max(0.0);
            reasons.push("pohw_alt_changed".into());
        }
        if bool_field(fo, "challenge_seed_present") == Some(false) {
            *risk = (*risk + 0.03).min(1.0);
            reasons.push("challenge_seed_absent".into());
        } else if bool_field(fo, "challenge_seed_present") == Some(true) {
            hits.push("challenge_seed_present".into());
        }
        if let Some(cv) = f_field(fo, "challenge_cv").or_else(|| f_field(fo, "challenge_avg_cv")) {
            hits.push("challenge_cv".into());
            if cv < 1e-9 {
                *risk = (*risk + 0.1).min(1.0);
                reasons.push("challenge_cv_too_stable".into());
            } else if cv > 0.15 {
                *risk = (*risk + 0.06).min(1.0);
                reasons.push("challenge_cv_noisy".into());
            } else {
                *risk = (*risk - 0.02).max(0.0);
                reasons.push("challenge_cv_ok".into());
            }
        }
        if has_nonempty(fo, "challenge_skip") || has_nonempty(fo, "challenge_error") {
            *risk = (*risk + 0.04).min(1.0);
            reasons.push("challenge_seed_gap".into());
        }
    }
    // WebGL/WebGPU surface + timer query meta
    if bool_field(fo, "webgl_support").is_some()
        || has_nonempty(fo, "webgl_version")
        || has_nonempty(fo, "webgl2_version")
        || f_field(fo, "webgl_extensions_count").is_some()
        || f_field(fo, "webgl_ext_count").is_some()
        || f_field(fo, "webgl_max_renderbuffer").is_some()
        || f_field(fo, "webgl_max_viewport").is_some()
    {
        hits.push("webgl_surface_meta".into());
        if bool_field(fo, "webgl_support") == Some(false) {
            *risk = (*risk + 0.04).min(1.0);
            reasons.push("webgl_support_false".into());
        } else {
            *risk = (*risk - 0.01).max(0.0);
            reasons.push("webgl_surface_present".into());
        }
    }
    if has_nonempty(fo, "timer_query_ext")
        || f_field(fo, "timer_query_bits").is_some()
        || has_nonempty(fo, "timer_query_skip")
    {
        hits.push("timer_query_meta".into());
        if has_nonempty(fo, "timer_query_skip") {
            *risk = (*risk + 0.02).min(1.0);
            reasons.push("timer_query_skip".into());
        } else {
            *risk = (*risk - 0.01).max(0.0);
        }
    }
    if has_nonempty(fo, "webgpu_skip") || has_nonempty(fo, "webgpu_error") {
        hits.push("webgpu_gap".into());
        *risk = (*risk + 0.02).min(1.0);
        reasons.push("webgpu_probe_gap".into());
    }
    // thermal / unit surface
    if f_field(fo, "thermal_budget_ms").is_some()
        || f_field(fo, "thermal_elapsed_ms").is_some()
        || f_field(fo, "thermal_phases_done").is_some()
        || has_nonempty(fo, "thermal_error")
    {
        hits.push("thermal_surface".into());
        if has_nonempty(fo, "thermal_error") {
            *risk = (*risk + 0.02).min(1.0);
            reasons.push("thermal_error".into());
        } else {
            *risk = (*risk - 0.01).max(0.0);
            reasons.push("thermal_materials".into());
        }
    }
    if bool_field(fo, "unit_surface_available").is_some() {
        hits.push("unit_surface_available".into());
        *risk = (*risk - 0.01).max(0.0);
    }
    // audio noise detail
    if f_field(fo, "audio_noise_len").is_some()
        || f_field(fo, "audio_noise_seeds").is_some()
        || f_field(fo, "audio_noise_sr").is_some()
        || f_field(fo, "audio_deep_sr").is_some()
    {
        hits.push("audio_noise_detail".into());
        *risk = (*risk - 0.01).max(0.0);
    }
    // caps probe_n
    if f_field(fo, "caps_probe_n").is_some() {
        hits.push("caps_probe_n".into());
        *risk = (*risk - 0.005).max(0.0);
    }
    // webrtc probe engine tags
    if has_nonempty(fo, "webrtc_probe_engine") || has_nonempty(fo, "webrtc_probe_profile") {
        hits.push("webrtc_probe_profile".into());
        *risk = (*risk - 0.01).max(0.0);
    }
    // math/wasm accumulators
    if f_field(fo, "math_acc").is_some() || f_field(fo, "wasm_acc").is_some() {
        hits.push("math_wasm_acc".into());
        *risk = (*risk - 0.01).max(0.0);
    }
    // bitness / intl extras
    if has_nonempty(fo, "bitness") || has_nonempty(fo, "ua_ch_bitness") {
        hits.push("bitness".into());
        *risk = (*risk - 0.01).max(0.0);
    }
    if has_nonempty(fo, "intl_calendar") || has_nonempty(fo, "intl_numbering") {
        hits.push("intl_calendar".into());
        *risk = (*risk - 0.01).max(0.0);
    }
    // mediump / shader ulp points
    if f_field(fo, "mediump_precision").is_some()
        || f_field(fo, "shader_ulp_points_n").is_some()
        || f_field(fo, "gl_high_float").is_some()
        || f_field(fo, "gl_med_float").is_some()
    {
        hits.push("shader_precision_surface".into());
        *risk = (*risk - 0.02).max(0.0);
        reasons.push("shader_precision_materials".into());
    }
    // probe gap skip/error families (honest unavailability → conf demote, not bot)
    for key in [
        "audio_deep_error",
        "audio_deep_skip",
        "bw_error",
        "bw_skip",
        "canvas_hedge_error",
        "caps_error",
        "caps_skip",
        "challenge_audio_error",
        "challenge_error",
        "challenge_skip",
        "clock_error",
        "cpu_cache_error",
        "cross_error",
        "display_error",
        "dom_rect_error",
        "dom_rect_skip",
        "eme_skip",
        "font_error",
        "gamepad_error",
        "gpu_bandwidth_error",
        "gpu_timer_error",
        "layer_divergence_error",
        "native_error",
        "neg_error",
        "perf_error",
        "permissions_skip",
        "raster_error",
        "raster_skip",
        "shader_draw_error",
        "shader_error",
        "shader_skip",
        "battery_error",
        "thermal_stopped",
    ] {
        if has_nonempty(fo, key) || bool_field(fo, key).unwrap_or(false) {
            hits.push(key.into());
            *risk = (*risk + 0.02).min(1.0);
            reasons.push(format!("probe_gap_{key}"));
        }
    }
    // cpu_loop_median — timing surface present (B10 full staged CPU curve)
    if f_field(fo, "cpu_loop_median_ms").is_some() {
        hits.push("cpu_loop_median_ms".into());
        *risk = (*risk - 0.01).max(0.0);
    }
    // Full-richness tags (diag): algo + sample count + mobile yield policy flag
    if has_nonempty(fo, "cpu_loop_algo") {
        hits.push("cpu_loop_algo".into());
    }
    if f_field(fo, "cpu_loop_samples").is_some() {
        hits.push("cpu_loop_samples".into());
    }
    if bool_field(fo, "probe_mobile_like").is_some() || has_nonempty(fo, "probe_mobile_like") {
        hits.push("probe_mobile_like".into());
    }
    // gamepad sample
    if has_nonempty(fo, "gamepad_ids_sample") {
        hits.push("gamepad_ids_sample".into());
        *risk = (*risk - 0.01).max(0.0);
        reasons.push("gamepad_surface".into());
    }
}

fn apply_probe_surface_evidence_br(
    fo: &Map<String, Value>,
    risk: &mut f64,
    reasons: &mut Vec<String>,
    hits: &mut Vec<String>,
) {
    apply_device_silicon_probe_evidence(fo, risk, reasons, hits, "br");
    // Permission shape — silent readonly only (notifications + media counts).
    // Never treat geo/camera/mic as probe materials (not_probed / banned request path).
    if has_nonempty(fo, "permission_states")
        || has_nonempty(fo, "permissions_matrix")
        || has_nonempty(fo, "permissions_notifications")
        || has_nonempty(fo, "privacy_policy")
    {
        hits.push("permission_shape_readonly".into());
        let notif = str_field(fo, "permissions_notifications").unwrap_or("");
        if !notif.is_empty() && notif != "not_probed" && notif != "unknown" {
            hits.push("permissions_notifications".into());
            *risk = (*risk - 0.01).max(0.0);
        }
        // Explicit ignore of geo/camera/mic even if legacy fields present
        for ban in [
            "permissions_geolocation",
            "geolocation_permission",
            "permissions_camera",
            "permissions_microphone",
        ] {
            if let Some(v) = str_field(fo, ban) {
                if v != "not_probed" && !v.is_empty() {
                    reasons.push(format!("legacy_{ban}_ignored"));
                }
            }
        }
        if fo
            .get("privacy_guard_blocked_n")
            .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
            .unwrap_or(0.0)
            > 0.0
        {
            // Our probe tried a banned API — treat as self-integrity issue, not user risk
            hits.push("privacy_guard_block".into());
            reasons.push("privacy_guard_blocked_attempt".into());
        }
        if fo
            .get("site_media_access_observed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            hits.push("site_media_access_observed".into());
            *risk = (*risk - 0.02).max(0.0);
            reasons.push("site_opened_media_observed".into());
        }
        if has_nonempty(fo, "permissions_skip") {
            *risk = (*risk + 0.02).min(1.0);
            reasons.push("permissions_api_gap".into());
        }
    }
    // CSS / MQ samples for claim-obs
    if has_nonempty(fo, "css_props_sample") || has_nonempty(fo, "media_query_true_sample") {
        hits.push("css_mq_sample".into());
        *risk = (*risk - 0.02).max(0.0);
        reasons.push("css_mq_materials".into());
    }
    // Storage privacy flags
    if bool_field(fo, "service_worker").is_some()
        || bool_field(fo, "indexedDB").is_some()
        || bool_field(fo, "openDatabase").is_some()
        || bool_field(fo, "storage_estimate_pending").is_some()
    {
        hits.push("storage_privacy_flags".into());
        hits.push("indexedDB".into());
        hits.push("openDatabase".into());
        *risk = (*risk - 0.01).max(0.0);
    }
    // UA-CH / brands surface
    if has_nonempty(fo, "ua_brands")
        || has_nonempty(fo, "ua_platform")
        || has_nonempty(fo, "ua_model")
        || bool_field(fo, "ua_mobile").is_some()
        || bool_field(fo, "ua_ch_wow64").is_some()
    {
        hits.push("ua_ch_surface".into());
        *risk = (*risk - 0.02).max(0.0);
        reasons.push("ua_ch_surface".into());
    }
    // permission flat names — notifications + digest only (camera/mic never materials)
    for k in ["permissions_notifications", "permission_shape_digest", "privacy_policy"] {
        if has_nonempty(fo, k) {
            hits.push(k.into());
            *risk = (*risk - 0.01).max(0.0);
        }
    }
    // websocket / speech / native toString
    if has_nonempty(fo, "ws_error")
        || has_nonempty(fo, "ws_close_reason")
        || has_nonempty(fo, "ws_binary_types")
    {
        hits.push("websocket_fp_surface".into());
        if has_nonempty(fo, "ws_error") {
            *risk = (*risk + 0.02).min(1.0);
            reasons.push("ws_error".into());
        } else {
            *risk = (*risk - 0.01).max(0.0);
        }
    }
    if has_nonempty(fo, "speech_error") || has_nonempty(fo, "speech_skip") {
        hits.push("speech_gap".into());
        *risk = (*risk + 0.02).min(1.0);
        reasons.push("speech_probe_gap".into());
    }
    if has_nonempty(fo, "native_function_toString") {
        hits.push("native_function_toString".into());
        *risk = (*risk - 0.01).max(0.0);
    }
    if has_nonempty(fo, "struct_hash") || has_nonempty(fo, "type_hist") {
        hits.push("census_struct_hash".into());
        *risk = (*risk - 0.01).max(0.0);
    }
    // Agent parity flattened globals (br kernel)
    if bool_field(fo, "agent_has_webdriver").unwrap_or(false)
        || bool_field(fo, "agent_has_selenium").unwrap_or(false)
        || bool_field(fo, "agent_has_puppeteer").unwrap_or(false)
        || bool_field(fo, "agent_has_cdc").unwrap_or(false)
        || bool_field(fo, "agent_has_phantom").unwrap_or(false)
        || bool_field(fo, "agent_has_dom_automation").unwrap_or(false)
    {
        hits.push("agent_parity_flat".into());
        *risk = (*risk + 0.12).min(1.0);
        if bool_field(fo, "agent_has_webdriver").unwrap_or(false) {
            reasons.push("agent_has_webdriver".into());
        }
        if bool_field(fo, "agent_has_selenium").unwrap_or(false) {
            reasons.push("agent_has_selenium".into());
        }
        if bool_field(fo, "agent_has_puppeteer").unwrap_or(false) {
            reasons.push("agent_has_puppeteer".into());
        }
        if bool_field(fo, "agent_has_cdc").unwrap_or(false) {
            reasons.push("agent_has_cdc".into());
        }
        if bool_field(fo, "agent_has_phantom").unwrap_or(false) {
            reasons.push("agent_has_phantom".into());
        }
    } else if has_nonempty(fo, "agent_parity_matrix") || has_nonempty(fo, "agent_parity_hash") {
        hits.push("agent_parity_matrix".into());
        *risk = (*risk - 0.02).max(0.0);
        reasons.push("agent_parity_clean".into());
    }
    // native integrity sample
    if has_nonempty(fo, "native_integrity_sample") {
        hits.push("native_integrity_sample".into());
        *risk = (*risk - 0.01).max(0.0);
    }
    // net_save_data
    if bool_field(fo, "net_save_data").is_some() {
        hits.push("net_save_data".into());
        *risk = (*risk - 0.01).max(0.0);
    }
    // media device counts
    if f_field(fo, "media_input_count").is_some()
        || f_field(fo, "media_output_count").is_some()
        || f_field(fo, "media_video_count").is_some()
    {
        hits.push("media_device_counts".into());
        *risk = (*risk - 0.02).max(0.0);
        reasons.push("media_device_counts".into());
    }
    // sandbox sources present
    if has_nonempty(fo, "sandbox_sources") {
        hits.push("sandbox_sources".into());
        *risk = (*risk - 0.01).max(0.0);
    }
    // leaf census estimate
    if f_field(fo, "object_census_leaf_est").is_some() || f_field(fo, "leaf_est").is_some() {
        hits.push("object_census_leaf_est".into());
        *risk = (*risk - 0.01).max(0.0);
    }
    // inner_width present (viewport surface)
    if f_field(fo, "inner_width").is_some() {
        hits.push("inner_width".into());
        *risk = (*risk - 0.005).max(0.0);
    }
}

fn apply_probe_surface_evidence_rpa(
    fo: &Map<String, Value>,
    risk: &mut f64,
    reasons: &mut Vec<String>,
    hits: &mut Vec<String>,
) {
    // Silicon materials present without automation → mild human/env confidence.
    // Automation flags still dominate demotion below.
    apply_device_silicon_probe_evidence(fo, risk, reasons, hits, "rpa");
    if bool_field(fo, "agent_has_webdriver").unwrap_or(false)
        || bool_field(fo, "agent_has_selenium").unwrap_or(false)
        || bool_field(fo, "agent_has_puppeteer").unwrap_or(false)
        || bool_field(fo, "agent_has_cdc").unwrap_or(false)
        || bool_field(fo, "agent_has_phantom").unwrap_or(false)
        || bool_field(fo, "agent_has_dom_automation").unwrap_or(false)
    {
        hits.push("agent_parity_rpa".into());
        *risk = (*risk + 0.18).min(1.0);
        reasons.push("agent_parity_automation_surface".into());
    }
    if f_field(fo, "agent_parity_hit_ratio").unwrap_or(0.0) > 0.15
        && f_field(fo, "agent_automation_globals_n").unwrap_or(0.0) >= 1.0
    {
        hits.push("agent_parity_hit_ratio".into());
        *risk = (*risk + 0.06).min(1.0);
        reasons.push("agent_parity_hit_ratio_elevated".into());
    }
}

/// Score block: score in 0..1, status, coverage, reasons, optional field_hits.
pub fn score_block(
    score: f64,
    status: &str,
    coverage: f64,
    reasons: Vec<String>,
    field_hits: Vec<String>,
) -> Value {
    json!({
        "score": clamp01(score),
        "status": status,
        "coverage": clamp01(coverage),
        "reasons": reasons,
        "field_hits": field_hits,
    })
}

/// Copy selected device/trust materials onto fields so axis scorers share one projection.
/// Missing materials demote related axes only — they do not imply probe halt.
fn enrich_fields_with_device_materials(fields: &Value, device: &Value) -> Value {
    let mut out = fields.as_object().cloned().unwrap_or_default();
    let mat = device
        .get("materials")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    // Prefer explicit materials map; fall back to top-level device keys.
    let pick = |key: &str| -> Option<Value> {
        mat.get(key)
            .cloned()
            .or_else(|| device.get(key).cloned())
    };
    for key in [
        "webrtc_missing",
        "webrtc_probe_failed",
        "webrtc_host_ip_hash",
        "webgl_residual_entropy_ok",
        "residual_entropy_ok",
        "engine_family",
        "os_instance_hash",
        "has_host_separator",
    ] {
        if out.get(key).map(|v| !v.is_null()).unwrap_or(false) {
            continue;
        }
        if let Some(v) = pick(key) {
            if !v.is_null() {
                out.insert(key.to_string(), v);
            }
        }
    }
    // id_warnings → soft flag for OS residual demote when explicit entropy bool absent
    if !out.contains_key("webgl_residual_entropy_ok") {
        if let Some(warns) = device.get("id_warnings").and_then(|v| v.as_array()) {
            if warns.iter().any(|w| {
                w.as_str()
                    .is_some_and(|s| s.contains("residual_low_entropy") || s.contains("webgl_residual"))
            }) {
                out.insert("webgl_residual_entropy_ok".into(), json!(false));
            }
        }
    }
    Value::Object(out)
}

/// Fraction of product-critical matrix fields present — density diagnostic.
///
/// Only **T0–T2** rows with at least one `material` or `conf` role count.
/// T3+ conf/diag expansions and exclude must not tank density as the matrix grows.
pub fn field_density(fields: &Value) -> Value {
    let Ok(m) = load_field_product_matrix() else {
        return json!({"present": 0, "total": 0, "ratio": 0.0});
    };
    let fo = fields.as_object().cloned().unwrap_or_default();
    let mut present = 0u32;
    let mut total = 0u32;
    let mut missing = Vec::new();
    let mut used = Vec::new();
    let mut all_total = 0u32;
    for row in &m.fields {
        all_total += 1;
        // Tight set so matrix growth (diag conf T2+) cannot tank density.
        // material T0–T2 + conf/veto only at T0–T1.
        let product_critical = row.axes.values().any(|r| match (r.as_str(), row.trust_tier.as_str()) {
            ("material", "T0" | "T1" | "T2") => true,
            ("conf" | "veto", "T0" | "T1") => true,
            _ => false,
        });
        if !product_critical {
            continue;
        }
        total += 1;
        let ok = field_present(&fo, &row.field);
        if ok {
            present += 1;
            used.push(row.field.clone());
        } else {
            missing.push(row.field.clone());
        }
    }
    let ratio = if total == 0 {
        0.0
    } else {
        present as f64 / total as f64
    };
    json!({
        "present": present,
        "total": total,
        "matrix_all": all_total,
        "ratio": ratio,
        "missing_sample": missing.into_iter().take(12).collect::<Vec<_>>(),
        "present_sample": used.into_iter().take(12).collect::<Vec<_>>(),
    })
}

fn field_present(fo: &Map<String, Value>, field: &str) -> bool {
    if field.contains('.') {
        let parts: Vec<&str> = field.splitn(2, '.').collect();
        if fo.get(parts[0])
            .and_then(|v| v.get(parts[1]))
            .map(|v| !v.is_null())
            .unwrap_or(false)
        {
            return true;
        }
    }
    has_nonempty(fo, field)
}

/// Per-axis contribution map for diagnostics (which fields fed which product axes).
pub fn field_axis_contributions(fields: &Value) -> Value {
    let Ok(m) = load_field_product_matrix() else {
        return json!({});
    };
    let fo = fields.as_object().cloned().unwrap_or_default();
    let mut out = Map::new();
    for axis in ["device_id", "os", "br", "rpa"] {
        let mut hits = Vec::new();
        for row in &m.fields {
            if let Some(role) = row.axes.get(axis) {
                if field_present(&fo, &row.field) {
                    hits.push(json!({
                        "field": row.field,
                        "role": role,
                    }));
                }
            }
        }
        out.insert(axis.into(), json!(hits));
    }
    Value::Object(out)
}

fn xsrc_has_conflict(source_conflicts: &[String], truth: &TruthResult) -> bool {
    !source_conflicts.is_empty() || truth.xsrc_status == XSRC_CONFLICT
}

/// Cross-context identity mismatches (B15 / B7 multi-source) — string-literal
/// consumption of matrix veto/conf fields so T1 multi-source signals cannot be
/// silent (policy: matrix T0–T2 material|conf|veto must be referenced in core).
fn apply_cross_context_mismatch_axis(
    axis: &str,
    fo: &Map<String, Value>,
    risk: &mut f64,
    reasons: &mut Vec<String>,
    hits: &mut Vec<String>,
) {
    let br = axis == "br";
    let os = axis == "os";
    let rpa = axis == "rpa";
    // Hard identity mismatches → BR veto-class; OS conf; RPA when webdriver/toString
    let hard = [
        ("sandbox_ua_mismatch", true, false, false),
        ("iframe_ua_mismatch", true, false, true),
        ("sandbox_platform_mismatch", true, true, false),
        ("iframe_platform_mismatch", true, true, true),
        ("sandbox_webdriver_mismatch", true, false, true),
        ("sandbox_tostring_diverged", true, false, true),
        ("sandbox_hw_mismatch", true, true, false),
    ];
    for (key, hit_br, hit_os, hit_rpa) in hard {
        let fired = bool_field(fo, key).unwrap_or(false);
        if !field_present(fo, key) && !fired {
            continue;
        }
        if field_present(fo, key) {
            hits.push(key.into());
        }
        if !fired {
            continue;
        }
        if br && hit_br {
            *risk = (*risk + 0.10).min(1.0);
            reasons.push(format!("xsrc_mismatch_br:{key}"));
        }
        if os && hit_os {
            *risk = (*risk + 0.06).min(1.0);
            reasons.push(format!("xsrc_mismatch_os:{key}"));
        }
        if rpa && hit_rpa {
            *risk = (*risk + 0.08).min(1.0);
            reasons.push(format!("xsrc_mismatch_rpa:{key}"));
        }
    }
    // ICE morphology (B9) — conf surface for os/br when present
    if os || br {
        for k in ["has_host", "has_srflx", "has_relay", "ice_host_n", "ice_srflx_n", "ice_relay_n", "webrtc_relay_count"] {
            if field_present(fo, k) {
                hits.push(k.into());
                // host present is mild support; relay-only oddity is weak
                if k == "has_host" && bool_field(fo, k).unwrap_or(false) {
                    *risk = (*risk - 0.02).max(0.0);
                }
                if k == "ice_host_n" {
                    if let Some(n) = f_field(fo, k) {
                        if n <= 0.0 {
                            *risk = (*risk + 0.03).min(1.0);
                            reasons.push("ice_no_host_candidate".into());
                        }
                    }
                }
            }
        }
    }
    if has_nonempty(fo, "seed_digest") {
        hits.push("seed_digest".into());
    }
}

/// Sandbox multi-source capability demotion shared by OS / BR / RPA.
/// Normal browsers execute nests; fake shells often empty/thin/conflict.
fn apply_sandbox_capability_axis(
    axis: &str,
    fo: &Map<String, Value>,
    risk: &mut f64,
    reasons: &mut Vec<String>,
    hits: &mut Vec<String>,
) {
    let cap = f_field(fo, "sandbox_capability_score");
    let band = str_field(fo, "sandbox_capability_band").unwrap_or("");
    let dead = bool_field(fo, "js_ok_sandbox_dead").unwrap_or(false)
        || bool_field(fo, "sandbox_blocked").unwrap_or(false)
        || bool_field(fo, "sandbox_all_empty").unwrap_or(false)
        || band == "dead"
        || cap.is_some_and(|c| c <= 0.05);
    let thin = bool_field(fo, "sandbox_thin_vs_main").unwrap_or(false)
        || bool_field(fo, "sandbox_under_two_kinds").unwrap_or(false)
        || band == "thin"
        || band == "empty_or_conflict"
        || cap.is_some_and(|c| c > 0.05 && c < 0.55);
    let ok = bool_field(fo, "sandbox_ok").unwrap_or(false)
        || band == "ok"
        || cap.is_some_and(|c| c >= 0.85);

    if dead {
        hits.push("sandbox_capability_dead".into());
        match axis {
            "br" => {
                *risk = (*risk + 0.28).min(1.0);
                reasons.push("sandbox_capability_dead_br".into());
            }
            "os" => {
                *risk = (*risk + 0.14).min(1.0);
                reasons.push("sandbox_capability_dead_os".into());
            }
            "rpa" => {
                *risk = (*risk + 0.10).min(1.0);
                reasons.push("sandbox_capability_dead_rpa_conf".into());
            }
            _ => {}
        }
        return;
    }
    if thin {
        hits.push("sandbox_capability_thin".into());
        match axis {
            "br" => {
                *risk = (*risk + 0.14).min(1.0);
                reasons.push("sandbox_capability_thin_br".into());
            }
            "os" => {
                *risk = (*risk + 0.08).min(1.0);
                reasons.push("sandbox_capability_thin_os".into());
            }
            "rpa" => {
                *risk = (*risk + 0.06).min(1.0);
                reasons.push("sandbox_capability_thin_rpa".into());
            }
            _ => {}
        }
        return;
    }
    if ok {
        hits.push("sandbox_capability_ok".into());
        *risk = (*risk - 0.05).max(0.0);
        reasons.push(format!("sandbox_capability_ok_{axis}"));
    } else if bool_field(fo, "sandbox_ok").unwrap_or(false)
        || has_nonempty(fo, "sandbox_sources_received")
        || has_nonempty(fo, "sources_received")
    {
        hits.push("sandbox_ok".into());
        *risk = (*risk - 0.03).max(0.0);
        reasons.push(format!("sandbox_multi_source_partial_{axis}"));
    }
}

/// Font surface thin / claim mismatch — real-scenario first-difference (fake browsers often 0–1 fonts).
fn apply_font_surface_axis(
    axis: &str,
    fo: &Map<String, Value>,
    risk: &mut f64,
    reasons: &mut Vec<String>,
    hits: &mut Vec<String>,
) {
    if has_nonempty(fo, "font_probe_err") {
        hits.push("font_probe_err".into());
        if axis == "br" || axis == "os" {
            *risk = (*risk + 0.02).min(1.0);
            reasons.push("font_probe_err".into());
        }
    }
    if has_nonempty(fo, "gl_dense_error") {
        hits.push("gl_dense_error".into());
        if axis == "br" || axis == "os" {
            *risk = (*risk + 0.02).min(1.0);
            reasons.push("gl_dense_error".into());
        }
    }
    let font_count = f_field(fo, "font_count").or_else(|| {
        fo.get("fonts_present")
            .and_then(|v| v.as_array())
            .map(|a| a.len() as f64)
    });
    let ua = str_field(fo, "user_agent").unwrap_or("");
    let platform = str_field(fo, "platform").unwrap_or("");
    let ua_l = ua.to_ascii_lowercase();
    let plat_l = platform.to_ascii_lowercase();
    let mobileish = ua_l.contains("mobile")
        || ua_l.contains("android")
        || ua_l.contains("iphone")
        || ua_l.contains("ipad");
    let desktopish = (ua_l.contains("windows")
        || ua_l.contains("macintosh")
        || ua_l.contains("linux")
        || ua_l.contains("x11")
        || plat_l.contains("win")
        || plat_l.contains("mac")
        || plat_l.contains("linux"))
        && !mobileish;

    if let Some(n) = font_count {
        hits.push("font_count".into());
        if desktopish && n <= 1.0 {
            hits.push("font_surface_thin".into());
            match axis {
                "br" => {
                    *risk = (*risk + 0.12).min(1.0);
                    reasons.push("font_surface_thin_desktop_br".into());
                }
                "os" => {
                    *risk = (*risk + 0.10).min(1.0);
                    reasons.push("font_surface_thin_desktop_os".into());
                }
                _ => {}
            }
        } else if desktopish && n < 4.0 {
            hits.push("font_surface_sparse".into());
            if axis == "br" || axis == "os" {
                *risk = (*risk + 0.05).min(1.0);
                reasons.push(format!("font_surface_sparse_{axis}"));
            }
        } else if n >= 8.0 && (axis == "br" || axis == "os") {
            *risk = (*risk - 0.02).max(0.0);
        }
    } else if desktopish && (axis == "br" || axis == "os") {
        // Missing font materials on desktop claim is weak negative (gap, not veto)
        hits.push("font_surface_absent".into());
        *risk = (*risk + 0.03).min(1.0);
        reasons.push(format!("font_surface_absent_{axis}"));
    }
}

/// iss/38 P0-1: IP/ASN as os reference_aux (low-weight confirm/contradict; never digest).
fn apply_os_net_reference_aux(
    fo: &Map<String, Value>,
    soft_phys: bool,
    mobile_claim: bool,
    risk: &mut f64,
    reasons: &mut Vec<String>,
    hits: &mut Vec<String>,
) {
    let has_ip = has_nonempty(fo, "server_client_ip");
    let has_asn = has_nonempty(fo, "server_asn");
    if !has_ip && !has_asn {
        return;
    }
    if has_ip {
        hits.push("server_client_ip".into());
    }
    if has_asn {
        hits.push("server_asn".into());
    }
    *risk = (*risk - 0.02).max(0.0);
    reasons.push("net_egress_aux_present".into());
    let form = str_field(fo, "form_class").unwrap_or("");
    let mobile_form = matches!(form, "mobile" | "phone" | "tablet") || mobile_claim;
    if soft_phys && mobile_form {
        *risk = (*risk + 0.08).min(1.0);
        reasons.push("net_egress_aux_soft_mobile_context".into());
    }
    if form == "desktop" && !mobile_claim && !soft_phys {
        *risk = (*risk - 0.01).max(0.0);
        reasons.push("net_egress_aux_desktop_confirm_low_weight".into());
    }
}

/// iss/38 P1-2: joint emulator form × sensors × soft GPU × touch.
fn apply_emulator_form_cross(
    fo: &Map<String, Value>,
    soft_phys: bool,
    mobile_claim: bool,
    risk: &mut f64,
    reasons: &mut Vec<String>,
    hits: &mut Vec<String>,
) {
    let form = str_field(fo, "form_class").unwrap_or("");
    let mobile_form = matches!(form, "mobile" | "phone" | "tablet") || mobile_claim;
    if !mobile_form {
        return;
    }
    let accel = bool_field(fo, "sensor_accel_present").unwrap_or(false);
    let gyro = bool_field(fo, "sensor_gyro_present").unwrap_or(false);
    let orient = bool_field(fo, "sensor_orient_present").unwrap_or(false);
    let any_sensor = accel || gyro || orient;
    let sensors_known = fo.contains_key("sensor_accel_present")
        || fo.contains_key("sensor_gyro_present")
        || fo.contains_key("sensor_orient_present");
    let touch = f_field(fo, "max_touch")
        .or_else(|| f_field(fo, "max_touch_points"))
        .unwrap_or(-1.0);
    let touch_lie = touch == 0.0;
    let emu = bool_field(fo, "emulator_hint").unwrap_or(false)
        || fo
            .get("emulator_hint")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty() && s != "false");
    let virt_gpu = soft_phys
        || matches!(
            str_field(fo, "stack_class").unwrap_or(""),
            "virt" | "vm" | "emulator" | "soft_render" | "software"
        );
    if virt_gpu && sensors_known && !any_sensor && (touch_lie || emu) {
        hits.push("emulator_form_cross".into());
        *risk = (*risk + 0.16).min(1.0);
        reasons.push("emulator_form_cross_lie".into());
        if touch_lie {
            hits.push("max_touch".into());
        }
        if sensors_known {
            hits.push("sensors_battery".into());
        }
    }
}

/// iss/38 P2-2: worker parallel throughput vs hardware_concurrency (weak).
fn apply_worker_throughput_aux(
    fo: &Map<String, Value>,
    risk: &mut f64,
    reasons: &mut Vec<String>,
    hits: &mut Vec<String>,
) {
    let ratio = f_field(fo, "worker_throughput_vs_cores");
    let cores = f_field(fo, "hardware_concurrency").unwrap_or(0.0);
    if let Some(r) = ratio {
        hits.push("worker_throughput_vs_cores".into());
        if cores >= 4.0 && r < 0.25 {
            *risk = (*risk + 0.06).min(1.0);
            reasons.push("worker_throughput_vs_cores_low".into());
        } else if r >= 0.6 {
            *risk = (*risk - 0.02).max(0.0);
            reasons.push("worker_throughput_vs_cores_ok_low_weight".into());
        }
    }
}

/// Collect reference_aux reason codes from os/br (iss/38 P0-3).
pub fn collect_reference_aux_hits(os: &Value, br: &Value) -> Vec<String> {
    let collect = |block: &Value| -> Vec<String> {
        block
            .get("reasons")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect()
    };
    let markers = [
        "ua_form_aux",
        "net_egress_aux",
        "ja_aux",
        "ja4_vs_ua",
        "ja_protocol",
        "gpu_label",
        "claim_obs",
        "emulator_form_cross",
        "worker_throughput",
        "mobile_ua_claim",
    ];
    let mut out = Vec::new();
    for r in collect(os).into_iter().chain(collect(br)) {
        if markers.iter().any(|m| r.contains(m)) && !out.contains(&r) {
            out.push(r);
        }
    }
    out
}

/// R00–R99 authenticity spot-check: empty/non-exec rate demotes OS/BR authenticity.
/// Does **not** contribute to absolute identity score or device_id mint materials.
/// Prefers multi-tick aggregate (`verify_agg_*` / rewritten ratios) over single-pack last-write.
fn apply_random_verify_authenticity(
    fo: &Map<String, Value>,
    risk: &mut f64,
    reasons: &mut Vec<String>,
    hits: &mut Vec<String>,
) {
    // Multi-pack aggregate (store evidence_merge::apply_verify_spotcheck_aggregate).
    if f_field(fo, "verify_agg_pack_n").is_some() {
        hits.push("verify_agg_pack_n".into());
    }
    if has_nonempty(fo, "verify_aggregate_algo") {
        hits.push("verify_aggregate_algo".into());
    }
    // Fields may arrive flattened from any Rxx pack or multi-tick aggregated.
    let nonempty_ratio = f_field(fo, "verify_nonempty_ratio")
        .or_else(|| f_field(fo, "verify_exec_ratio"));
    let empty_n = f_field(fo, "verify_empty_n").or_else(|| f_field(fo, "verify_agg_empty_n"));
    let ops_n = f_field(fo, "verify_probe_ops_n")
        .or_else(|| f_field(fo, "verify_exec_n"))
        .or_else(|| f_field(fo, "verify_agg_ops_n"));
    let dim_fail_n = f_field(fo, "verify_dim_fail_n")
        .or_else(|| f_field(fo, "verify_agg_dim_fail_n_max"))
        .unwrap_or(0.0);
    let role = str_field(fo, "verify_role").unwrap_or("");
    let algo = str_field(fo, "verify_algo").unwrap_or("");
    let is_verify = role.contains("authenticity")
        || algo.contains("rand_verify")
        || algo.contains("gr_rand_verify")
        || nonempty_ratio.is_some()
        || ops_n.is_some();
    if !is_verify {
        // Scan nested verify_* keys presence as soft signal that packs ran
        let has_any = fo.keys().any(|k| k.starts_with("verify_"));
        if !has_any {
            return;
        }
    }
    hits.push("rand_verify_surface".into());
    if let Some(r) = nonempty_ratio {
        hits.push("verify_nonempty_ratio".into());
        // Real browsers: most arbitrary API probes yield non-empty observations.
        // Scripted / stripped / heavily blocked envs show elevated empty rates.
        if r < 0.35 {
            *risk = (*risk + 0.18).min(1.0);
            reasons.push(format!("rand_verify_empty_high ratio={r:.2}"));
        } else if r < 0.55 {
            *risk = (*risk + 0.10).min(1.0);
            reasons.push(format!("rand_verify_empty_mid ratio={r:.2}"));
        } else if r < 0.70 {
            *risk = (*risk + 0.04).min(1.0);
            reasons.push(format!("rand_verify_empty_soft ratio={r:.2}"));
        } else if r >= 0.85 {
            *risk = (*risk - 0.02).max(0.0);
            reasons.push("rand_verify_healthy".into());
        }
    }
    if dim_fail_n >= 4.0 {
        *risk = (*risk + 0.08).min(1.0);
        reasons.push(format!("rand_verify_dim_fail_n={dim_fail_n}"));
    } else if dim_fail_n >= 2.0 {
        *risk = (*risk + 0.04).min(1.0);
        reasons.push(format!("rand_verify_dim_fail_n={dim_fail_n}"));
    }
    if let (Some(e), Some(n)) = (empty_n, ops_n) {
        if n >= 20.0 {
            hits.push("verify_probe_ops_n".into());
            let ratio = e / n;
            if ratio > 0.6 {
                *risk = (*risk + 0.06).min(1.0);
                reasons.push(format!("rand_verify_empty_ops={e}/{n}"));
            }
        }
    }
    // Explicit marker: never treat as device material
    if bool_field(fo, "verify_not_score_material").unwrap_or(false)
        || str_field(fo, "verify_role")
            .map(|s| s.contains("spotcheck"))
            .unwrap_or(false)
    {
        hits.push("verify_not_score_material".into());
    }
}

/// Dense B47–B79 digests (gr_dense_v3): claim-obs hashes + min-field met + pad hedge.
/// Complements matrix_role_use for fields that densify emits as family digests.
fn apply_dense_digest_evidence(
    axis: &str,
    fo: &Map<String, Value>,
    risk: &mut f64,
    reasons: &mut Vec<String>,
    hits: &mut Vec<String>,
) {
    let algo = str_field(fo, "probe_density_algo").unwrap_or("");
    let dense_surface = algo.contains("gr_dense")
        || has_nonempty(fo, "dense_pack")
        || bool_field(fo, "probe_min_met").unwrap_or(false)
        || f_field(fo, "probe_specific_n").unwrap_or(0.0) >= 20.0;

    if has_nonempty(fo, "densify_error") || has_nonempty(fo, "family_dense_error") {
        hits.push("dense_error".into());
        *risk = (*risk + 0.05).min(1.0);
        reasons.push("dense_family_error".into());
    }

    // Aggregate claim-obs digests densify prefers over raw row dumps.
    let digest_keys = [
        "api_flags_hash",
        "api_probe_hash",
        "font_matrix_hash",
        "mq_hash",
        "gl_ext_full_hash",
        "css_supports_hash",
        "gl_unmasked_renderer_dense",
        // competitive expansion digests (v4 dense)
        "speech_voices_hash",
        "font_preferences_hash",
        "system_colors_hash",
        "dom_rect_hash",
        "svg_rect_hash",
        "math_fingerprint_hash",
        "text_metrics_hash",
        "codec_support_hash",
        "media_capabilities_hash",
        "webgl_params_digest",
        "webgl_extension_set_hash",
        "permissions_query_hash",
        "plugin_mime_hash",
        "window_keys_own_hash",
        "worker_navigator_hash",
        "iframe_navigator_hash",
        "offline_audio_fingerprint",
        "canvas_geometry_text_hash",
        "dom_blockers_hash",
        "keyboard_layout_map_hash",
        // async competitive follow-ups (gr_async_competitive_v1)
        "ua_ch_full_version_hash",
        "ua_ch_brands_hash",
        "webgpu_features_hash",
        "webgpu_limits_hash",
        "local_fonts_hash",
        "storage_quota_bytes",
    ];
    let mut digests = 0_u32;
    for k in digest_keys {
        if has_nonempty(fo, k) {
            digests += 1;
            if digests <= 8 {
                hits.push(k.into());
            }
        }
    }
    if digests >= 3 {
        *risk = (*risk - 0.04).max(0.0);
        reasons.push(format!("dense_digest_n={digests}"));
    } else if digests == 2 {
        *risk = (*risk - 0.02).max(0.0);
        reasons.push("dense_digest_n=2".into());
    } else if digests == 1 {
        *risk = (*risk - 0.01).max(0.0);
        reasons.push("dense_digest_partial".into());
    }

    if dense_surface {
        hits.push("gr_dense_surface".into());
        if !algo.is_empty() {
            hits.push("probe_density_algo".into());
        }
        if bool_field(fo, "probe_min_met").unwrap_or(false) {
            *risk = (*risk - 0.02).max(0.0);
            reasons.push("probe_min_met".into());
        }
        if let Some(n) = f_field(fo, "probe_specific_n") {
            hits.push("probe_specific_n".into());
            if n >= 30.0 {
                *risk = (*risk - 0.02).max(0.0);
                reasons.push("probe_specific_ge30".into());
            } else if n > 0.0 && n < 10.0 {
                *risk = (*risk + 0.03).min(1.0);
                reasons.push("probe_specific_thin".into());
            }
        }
        // Vanity pad dominance is not real depth.
        if let (Some(pad), Some(spec)) = (f_field(fo, "probe_pad_n"), f_field(fo, "probe_specific_n"))
        {
            if pad > 0.0 && spec > 0.0 && pad >= spec {
                *risk = (*risk + 0.04).min(1.0);
                reasons.push("dense_pad_dominates".into());
            }
        }
        if has_nonempty(fo, "dense_pack") {
            hits.push("dense_pack".into());
        }
    }

    // API surface ratio — headless often sparse; real Chromium richer.
    if axis == "br" || axis == "os" || axis == "rpa" {
        if let Some(r) = f_field(fo, "api_probe_ratio").or_else(|| {
            let hit = f_field(fo, "api_flags_hit").or_else(|| f_field(fo, "api_probe_hit"));
            let n = f_field(fo, "api_flags_n").or_else(|| f_field(fo, "api_probe_n"));
            match (hit, n) {
                (Some(h), Some(nn)) if nn > 0.0 => Some(h / nn),
                _ => None,
            }
        }) {
            hits.push("api_probe_ratio".into());
            if r < 0.15 {
                *risk = (*risk + 0.04).min(1.0);
                reasons.push("api_probe_ratio_low".into());
            } else if r >= 0.35 {
                *risk = (*risk - 0.02).max(0.0);
                reasons.push("api_probe_ratio_ok".into());
            }
        }
    }
}

/// Matrix-driven consumption: every useful (material|conf|veto) field present on this
/// axis is **used** in scoring — not density-only. Caps prevent conf bloat from
/// inventing high safety; veto bools demote only when true.
///
/// Algo id: part of `PROBE_SURFACE_ALGO` / product path (matrix SSOT walk).
fn apply_matrix_role_evidence(
    axis: &str,
    fo: &Map<String, Value>,
    risk: &mut f64,
    reasons: &mut Vec<String>,
    hits: &mut Vec<String>,
) {
    let Ok(m) = load_field_product_matrix() else {
        return;
    };
    let mut mat_boost = 0.0_f64;
    let mut conf_boost = 0.0_f64;
    let mut veto_hits = 0_u32;
    let mut mat_n = 0_u32;
    let mut conf_n = 0_u32;
    let mut hit_sample = 0_u32;
    for row in &m.fields {
        let role = match row.axes.get(axis).map(|s| s.as_str()) {
            Some("material") | Some("conf") | Some("veto") => {
                row.axes.get(axis).map(|s| s.as_str()).unwrap()
            }
            _ => continue,
        };
        if !field_present(fo, &row.field) {
            continue;
        }
        // Sample field_hits (avoid megabyte hit lists); always count.
        if hit_sample < 24 {
            hits.push(format!("mx:{axis}:{}", row.field));
            hit_sample += 1;
        }
        match role {
            "material" => {
                mat_n += 1;
                mat_boost += 0.012;
            }
            "conf" => {
                conf_n += 1;
                conf_boost += 0.004;
            }
            "veto" => {
                // Nested automation.* or flat bools: true = demote; false = observed clean.
                let bad = if row.field.contains('.') {
                    let parts: Vec<&str> = row.field.splitn(2, '.').collect();
                    fo.get(parts[0])
                        .and_then(|v| v.get(parts[1]))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                } else {
                    bool_field(fo, &row.field).unwrap_or(false)
                        || str_field(fo, &row.field)
                            .map(|s| {
                                let l = s.to_ascii_lowercase();
                                l.contains("swiftshader")
                                    || l.contains("llvmpipe")
                                    || l == "mismatch"
                                    || l.contains("soft")
                            })
                            .unwrap_or(false)
                };
                if bad {
                    veto_hits += 1;
                    *risk = (*risk + 0.07).min(1.0);
                    if veto_hits <= 6 {
                        reasons.push(format!("matrix_veto:{}", row.field));
                    }
                } else {
                    // Observed negative veto (e.g. webdriver:false) — mild conf.
                    conf_boost += 0.003;
                    conf_n += 1;
                }
            }
            _ => {}
        }
    }
    // Caps: materials strengthen more than conf; neither invents "real" alone.
    // When any veto fired, conf boost is suppressed (cannot average away automation).
    let mat_c = mat_boost.min(0.14);
    let conf_c = if veto_hits > 0 { 0.0 } else { conf_boost.min(0.10) };
    if mat_c > 0.0 || conf_c > 0.0 || veto_hits > 0 {
        *risk = (*risk - mat_c - conf_c).max(0.0);
        reasons.push(format!(
            "matrix_role_use axis={axis} mat={mat_n} conf={conf_n} veto_fire={veto_hits}"
        ));
    }
    hits.push(format!("matrix_mat_n={mat_n}"));
    hits.push(format!("matrix_conf_n={conf_n}"));
}

/// Multi-dimensional corroboration: fields from different packs/sources must
/// agree. Single-dimension cleanliness is not enough (hedge principle).
fn apply_cross_axis_corroboration(
    axis: &str,
    fo: &Map<String, Value>,
    risk: &mut f64,
    reasons: &mut Vec<String>,
    hits: &mut Vec<String>,
) {
    let os = axis == "os";
    let br = axis == "br";
    let rpa = axis == "rpa";

    // --- Form factor ↔ pointer / touch / orientation ---
    let form = str_field(fo, "form_class").unwrap_or("");
    let max_touch = f_field(fo, "max_touch")
        .or_else(|| f_field(fo, "max_touch_points"))
        .unwrap_or(0.0);
    let pointer_fine = bool_field(fo, "pointer_fine").unwrap_or(false);
    let pointer_coarse = bool_field(fo, "pointer_coarse")
        .or_else(|| bool_field(fo, "css_pointer_coarse"))
        .unwrap_or(false);
    let hover_hover = bool_field(fo, "hover_hover")
        .or_else(|| bool_field(fo, "css_hover_hover"))
        .unwrap_or(false);
    if (os || br || rpa) && form == "desktop" {
        if max_touch >= 5.0 && !pointer_fine {
            *risk = (*risk + 0.06).min(1.0);
            reasons.push("cross_form_desktop_touch_no_fine".into());
            hits.push("form_pointer_cross".into());
        }
        if pointer_coarse && !hover_hover {
            *risk = (*risk + 0.04).min(1.0);
            reasons.push("cross_desktop_coarse_no_hover".into());
            hits.push("form_hover_cross".into());
        }
    }
    if (os || br) && form == "mobile" {
        if pointer_fine && max_touch <= 0.0 {
            *risk = (*risk + 0.05).min(1.0);
            reasons.push("cross_form_mobile_fine_no_touch".into());
            hits.push("form_pointer_cross".into());
        }
        if str_field(fo, "ice_morphology") == Some("host_only") {
            // mobile often has srflx; host-only can be emulator/privacy
            *risk = (*risk + 0.03).min(1.0);
            reasons.push("cross_mobile_ice_host_only".into());
            hits.push("form_ice_cross".into());
        }
    }

    // --- Platform claim chain: platform ↔ UA-CH ↔ timezone / arch / flavors ---
    if os || br {
        let plat = str_field(fo, "platform").unwrap_or("").to_ascii_lowercase();
        let ua_ch = str_field(fo, "ua_ch_platform")
            .or_else(|| str_field(fo, "ua_platform"))
            .unwrap_or("")
            .to_ascii_lowercase();
        if !plat.is_empty() && !ua_ch.is_empty() {
            hits.push("platform_ua_ch_cross".into());
            let plat_win = plat.contains("win");
            let ch_win = ua_ch.contains("win");
            let plat_mac = plat.contains("mac");
            let ch_mac = ua_ch.contains("mac");
            let plat_lin = plat.contains("linux");
            let ch_lin = ua_ch.contains("linux") || ua_ch.contains("chrome os");
            if (plat_win && (ch_mac || ch_lin))
                || (plat_mac && (ch_win || ch_lin))
                || (plat_lin && (ch_win || ch_mac))
            {
                *risk = (*risk + 0.12).min(1.0);
                reasons.push("cross_platform_vs_ua_ch_mismatch".into());
            }
        }
        // High-entropy UA-CH architecture vs platform (async follow-up)
        let arch = str_field(fo, "ua_ch_architecture")
            .or_else(|| str_field(fo, "architecture_bitness"))
            .unwrap_or("")
            .to_ascii_lowercase();
        if !plat.is_empty() && !arch.is_empty() {
            hits.push("platform_arch_cross".into());
            let plat_win = plat.contains("win");
            // Win normally x86/arm; empty architecture after high-entropy promise is thin
            if bool_field(fo, "ua_ch_high_entropy_ok") == Some(true) && arch == ":" {
                *risk = (*risk + 0.04).min(1.0);
                reasons.push("cross_ua_ch_high_entropy_empty".into());
            }
            if plat_win && arch.contains("arm") {
                hits.push("win_on_arm".into());
            }
        }
        // Vendor flavors vs UA (chrome object without chrome brand is odd)
        if has_nonempty(fo, "vendor_flavors") || f_field(fo, "vendor_flavors_n").is_some() {
            hits.push("vendor_flavors_cross".into());
            let flavors = str_field(fo, "vendor_flavors").unwrap_or_default();
            let brands = str_field(fo, "ua_ch_brands_hash").unwrap_or_default();
            let has_chrome_obj = flavors.contains("chrome")
                || bool_field(fo, "window_chrome").unwrap_or(false)
                || bool_field(fo, "chrome_app").unwrap_or(false);
            if has_chrome_obj && brands.is_empty() && str_field(fo, "ua_ch_platform").is_none() {
                *risk = (*risk + 0.03).min(1.0);
                reasons.push("cross_chrome_obj_without_ua_ch".into());
            }
        }
        // timezone_offset_min material use + DST vs claim
        if f_field(fo, "timezone_offset_min").is_some() || has_nonempty(fo, "timezone") {
            hits.push("timezone_offset_min".into());
            hits.push("timezone_surface".into());
        }
        if let (Some(jan), Some(jul)) = (
            f_field(fo, "timezone_offset_jan").or_else(|| f_field(fo, "tz_offset_jan")),
            f_field(fo, "timezone_offset_jul").or_else(|| f_field(fo, "tz_offset_jul")),
        ) {
            hits.push("timezone_dst_cross".into());
            let dst = (jan - jul).abs();
            if let Some(delta) = f_field(fo, "timezone_dst_delta") {
                if (delta - dst).abs() > 1.0 {
                    *risk = (*risk + 0.04).min(1.0);
                    reasons.push("cross_timezone_dst_incoherent".into());
                }
            }
        }
    }

    // --- Residual soft ↔ GPU label / challenge noise (authenticity hedge) ---
    if os || br {
        let soft = bool_field(fo, "residual_soft_like").unwrap_or(false)
            || str_field(fo, "stack_class")
                .map(|s| s.contains("soft") || s == "virt" || s == "emulator")
                .unwrap_or(false);
        let label = str_field(fo, "webgl_unmasked_renderer")
            .or_else(|| str_field(fo, "gl_unmasked_renderer_dense"))
            .unwrap_or("")
            .to_ascii_lowercase();
        let fancy_gpu = label.contains("nvidia")
            || label.contains("geforce")
            || label.contains("radeon")
            || label.contains("apple m");
        if soft && fancy_gpu {
            *risk = (*risk + 0.14).min(1.0);
            reasons.push("cross_soft_residual_vs_fancy_gpu_label".into());
            hits.push("residual_label_cross".into());
        }
        if bool_field(fo, "noise_suspect").unwrap_or(false) && !soft && fancy_gpu {
            *risk = (*risk + 0.08).min(1.0);
            reasons.push("cross_challenge_noise_vs_clean_gpu_claim".into());
            hits.push("challenge_gpu_cross".into());
        }
        if has_nonempty(fo, "font_matrix_hash") || has_nonempty(fo, "font_bitmap_hash") {
            hits.push("font_matrix_cross".into());
            // fonts present with missing platform is thin
            if str_field(fo, "platform").is_none() && str_field(fo, "os_family").is_none() {
                *risk = (*risk + 0.03).min(1.0);
                reasons.push("cross_fonts_without_os_claim".into());
            }
        }
        // Font preferences vs soft stack: perfect zero widths often template
        if has_nonempty(fo, "font_preferences_hash") {
            hits.push("font_preferences_cross".into());
            let w_serif = f_field(fo, "font_pref_serif_width").unwrap_or(-1.0);
            let w_sans = f_field(fo, "font_pref_sans_width").unwrap_or(-1.0);
            if w_serif > 0.0 && w_sans > 0.0 && (w_serif - w_sans).abs() < 1e-6 && soft {
                *risk = (*risk + 0.05).min(1.0);
                reasons.push("cross_font_pref_identical_soft".into());
            }
        }
        // WebGPU adapter vs residual soft / fancy GPU label
        if bool_field(fo, "webgpu_adapter_ok") == Some(true) {
            hits.push("webgpu_adapter_cross".into());
            let gv = str_field(fo, "webgpu_adapter_vendor")
                .unwrap_or("")
                .to_ascii_lowercase();
            if soft && (gv.contains("nvidia") || gv.contains("amd") || gv.contains("apple")) {
                *risk = (*risk + 0.08).min(1.0);
                reasons.push("cross_soft_vs_webgpu_vendor".into());
            }
            if bool_field(fo, "webgpu_is_fallback").unwrap_or(false) && fancy_gpu {
                *risk = (*risk + 0.06).min(1.0);
                reasons.push("cross_webgpu_fallback_vs_fancy_gl_label".into());
            }
        }
        // Speech voices: zero voices on desktop claim is thin (async should usually fill)
        if let Some(n) = f_field(fo, "speech_voices_n") {
            hits.push("speech_voices_cross".into());
            if n <= 0.0 && form != "mobile" && form != "tablet" {
                if bool_field(fo, "speech_voices_async_ok") == Some(false)
                    || has_nonempty(fo, "speech_voices_async_skip")
                {
                    *risk = (*risk + 0.05).min(1.0);
                    reasons.push("cross_speech_voices_empty_desktop".into());
                }
            }
            if n >= 1.0 {
                *risk = (*risk - 0.01).max(0.0);
            }
        }
        // Storage quota: missing estimate after async skip vs present
        if bool_field(fo, "storage_estimate_ok") == Some(true) {
            hits.push("storage_estimate_cross".into());
            if let Some(q) = f_field(fo, "storage_quota_bytes") {
                if q > 0.0 && q < 1_000_000.0 {
                    // tiny quota often privacy / tracker-blocking profile
                    *risk = (*risk + 0.03).min(1.0);
                    reasons.push("cross_storage_quota_tiny".into());
                }
            }
        } else if has_nonempty(fo, "storage_estimate_skip")
            || bool_field(fo, "storage_estimate_api") == Some(false)
        {
            hits.push("storage_estimate_gap".into());
        }
        // Screen frame vs form_class
        if has_nonempty(fo, "screen_frame_inner") || has_nonempty(fo, "screen_frame_outer") {
            hits.push("screen_frame_cross".into());
            let outer = str_field(fo, "screen_frame_outer").unwrap_or_default();
            if form == "desktop" && outer.starts_with("0x") {
                *risk = (*risk + 0.04).min(1.0);
                reasons.push("cross_desktop_outer_zero".into());
            }
        }
        // Math fingerprint present with automation is weak positive only if clean
        if has_nonempty(fo, "math_fingerprint_hash") {
            hits.push("math_fingerprint_cross".into());
        }
        // Codec / mediaCapabilities: all-unsupported on desktop chrome-like is suspicious
        if let Some(n) = f_field(fo, "media_capabilities_supported_n") {
            hits.push("media_capabilities_cross".into());
            let total = f_field(fo, "media_capabilities_n").unwrap_or(0.0);
            if total >= 3.0 && n <= 0.0 {
                *risk = (*risk + 0.06).min(1.0);
                reasons.push("cross_media_capabilities_all_unsupported".into());
            }
        }
    }

    // --- Census / API depth ↔ automation (stealth often fakes APIs) ---
    if br || rpa {
        let auto = bool_field(fo, "webdriver").unwrap_or(false)
            || bool_field(fo, "agent_has_webdriver").unwrap_or(false)
            || bool_field(fo, "agent_has_playwright").unwrap_or(false)
            || bool_field(fo, "agent_has_selenium").unwrap_or(false)
            || bool_field(fo, "agent_has_cdc").unwrap_or(false);
        let census_rich = has_nonempty(fo, "api_flags_hash")
            || has_nonempty(fo, "css_supports_hash")
            || has_nonempty(fo, "font_matrix_hash")
            || f_field(fo, "api_flags_n").unwrap_or(0.0) >= 20.0;
        if auto && census_rich {
            // automation still "rich" census — control-plane is primary truth
            *risk = (*risk + 0.06).min(1.0);
            reasons.push("cross_automation_with_rich_census".into());
            hits.push("auto_census_cross".into());
        }
        if has_nonempty(fo, "agent_parity_matrix") || has_nonempty(fo, "agent_parity_hash") {
            hits.push("agent_parity_matrix".into());
        }
    }

    // --- RPA kinematics ↔ control-plane ---
    if rpa || br {
        let tr = f_field(fo, "trusted_ratio");
        let icv = f_field(fo, "interval_cv");
        let pe = f_field(fo, "path_entropy");
        let bound = bool_field(fo, "behavior_early_bound").unwrap_or(false);
        if bound {
            if let (Some(i), Some(p)) = (icv, pe) {
                hits.push("rpa_kinematics_cross".into());
                if i < 0.05 && p < 0.15 {
                    *risk = (*risk + 0.1).min(1.0);
                    reasons.push("cross_rpa_metronomic_low_entropy".into());
                }
            }
            if let Some(t) = tr {
                if t < 0.4 {
                    *risk = (*risk + 0.08).min(1.0);
                    reasons.push("cross_rpa_untrusted_events".into());
                }
            }
            if f_field(fo, "path_len").is_some() {
                hits.push("path_len".into());
            }
            if f_field(fo, "behavior_unique_kinds").is_some()
                || has_nonempty(fo, "behavior_unique_kinds")
            {
                hits.push("behavior_unique_kinds".into());
            }
        }
    }

    // --- Multi-source sandbox ↔ residual / challenge ---
    if os || br {
        if let Some(ratio) = f_field(fo, "multi_source_match_ratio") {
            hits.push("multi_source_cross".into());
            let soft = bool_field(fo, "residual_soft_like").unwrap_or(false);
            if ratio >= 0.95 && soft {
                // Perfect multi-source agree + soft residual → collusion class
                *risk = (*risk + 0.1).min(1.0);
                reasons.push("cross_multisource_agree_soft_residual".into());
            }
            if ratio < 0.6 {
                *risk = (*risk + 0.08).min(1.0);
                reasons.push("cross_multisource_diverge".into());
            }
        }
        if bool_field(fo, "has_challenge_seed").unwrap_or(false)
            || has_nonempty(fo, "challenge_seed")
        {
            hits.push("has_challenge_seed".into());
            if !has_nonempty(fo, "challenge_residual_mean")
                && !has_nonempty(fo, "pohw_triad")
            {
                *risk = (*risk + 0.04).min(1.0);
                reasons.push("cross_challenge_seed_without_material".into());
            }
        }
    }

    // --- Network path ↔ claim ---
    if os || br {
        if let Some(morph) = str_field(fo, "ice_morphology") {
            hits.push("ice_network_cross".into());
            let eng = str_field(fo, "protocol_engine")
                .or_else(|| str_field(fo, "claim_obs_ua_engine"))
                .unwrap_or("");
            if morph == "none" && !eng.is_empty() {
                *risk = (*risk + 0.04).min(1.0);
                reasons.push("cross_engine_claim_no_ice".into());
            }
        }
        if has_nonempty(fo, "net_rtt") || has_nonempty(fo, "tcp_rtt_us") {
            hits.push("net_rtt_cross".into());
        }
    }

    // --- Media / codec ↔ form ---
    if os || br {
        if has_nonempty(fo, "media_capabilities")
            || has_nonempty(fo, "codec_matrix_hash")
            || has_nonempty(fo, "eme_systems")
        {
            hits.push("media_codec_cross".into());
        }
        if has_nonempty(fo, "media_queries_lite") || has_nonempty(fo, "mq_hash") {
            hits.push("mq_cross".into());
        }
    }

    // --- Math/canvas/audio multi-material co-presence (device-ish hedge on br/os) ---
    if os || br {
        let math = has_nonempty(fo, "math_digest")
            || has_nonempty(fo, "math_hash")
            || has_nonempty(fo, "math_sin");
        let canvas = has_nonempty(fo, "canvas_geometry_hash")
            || has_nonempty(fo, "canvas_emoji_hash")
            || has_nonempty(fo, "canvas_data_hash");
        let audio = has_nonempty(fo, "hw_curve_audio")
            || has_nonempty(fo, "audio_worklet")
            || has_nonempty(fo, "offline_audio_ctor");
        let n = [math, canvas, audio].iter().filter(|x| **x).count();
        if n >= 2 {
            *risk = (*risk - 0.04).max(0.0);
            reasons.push(format!("cross_material_families_ok={n}"));
            hits.push("multi_material_cross".into());
        } else if n == 1 && has_nonempty(fo, "webdriver") {
            // single soft material + automation interest
            hits.push("single_material_with_auto_interest".into());
        }
    }

    // Explicit bag reads for previously never-in-core FE keys (audit + use).
    for k in [
        "cookieEnabled",
        "vendor",
        "css_pointer_coarse",
        "css_hover_hover",
        "media_queries_lite",
        "indexedDB",
        "font_hit_sample",
        "mq_probe_n",
        "mq_true_sample",
        "ice_error",
        "rtc_error",
        "webgl_err",
        "webgl_missing",
        "wasm_err",
        "audio_err",
        "vv_missing",
        "pack_error",
        "early_kick",
        "timezone_offset_min",
        "path_len",
        "behavior_unique_kinds",
        "has_challenge_seed",
    ] {
        if field_present(fo, k) {
            hits.push(k.into());
        }
    }
    // Probe errors demote mildly (failed useful probe ≠ silent skip).
    for (k, tag) in [
        ("ice_error", "ice_probe_error"),
        ("rtc_error", "rtc_probe_error"),
        ("webgl_err", "webgl_probe_error"),
        ("webgl_missing", "webgl_missing"),
        ("wasm_err", "wasm_probe_error"),
        ("audio_err", "audio_probe_error"),
        ("pack_error", "pack_probe_error"),
    ] {
        if has_nonempty(fo, k) {
            *risk = (*risk + 0.03).min(1.0);
            reasons.push(tag.into());
        }
    }
    if bool_field(fo, "vv_missing").unwrap_or(false) && rpa {
        *risk = (*risk + 0.04).min(1.0);
        reasons.push("visual_viewport_missing".into());
    }
}


/// OS environment safety.
pub fn score_os(
    fields: &Value,
    stack: &StackAuth,
    truth: &TruthResult,
    source_conflicts: &[String],
) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let mut reasons = Vec::new();
    let mut hits = Vec::new();
    let coverage = load_field_product_matrix()
        .map(|m| m.axis_coverage("os", fields))
        .unwrap_or(0.35);

    let mut risk = 0.22_f64;
    let vm = f_field(&fo, "vm_score").unwrap_or(stack.vm_score).clamp(0.0, 1.0);
    if vm > 0.0 {
        risk = risk.max(vm);
        if vm >= 0.35 {
            reasons.push(format!("vm_score={vm:.2}"));
            hits.push("vm_score".into());
        }
    }
    // iss/21 T-OS-1: soft_stack + residual are primary; renderer strings only diag-boost.
    let soft_phys = stack.soft_stack
        || bool_field(&fo, "residual_soft_like").unwrap_or(false)
        || stack.residual_soft_like == Some(true);
    if stack.soft_stack {
        risk = (risk + 0.22).min(1.0);
        reasons.push("soft_stack".into());
        hits.push("soft_stack".into());
    }
    if bool_field(&fo, "residual_soft_like").unwrap_or(false)
        || stack.residual_soft_like == Some(true)
    {
        risk = (risk + 0.22).min(1.0);
        reasons.push("residual_soft_like".into());
        hits.push("residual_soft_like".into());
    }
    // Unit multi-seed surface: soft-class conf for OS environment (demo D41).
    if has_nonempty(&fo, "unit_surface_id") {
        hits.push("unit_surface_id".into());
        if has_nonempty(&fo, "unit_surface_algo") {
            hits.push("unit_surface_algo".into());
        }
        if soft_phys {
            risk = (risk + 0.04).min(1.0);
            reasons.push("unit_surface_soft_class_context".into());
        } else if bool_field(&fo, "unit_multiround_stable") == Some(true) {
            risk = (risk - 0.03).max(0.0);
            reasons.push("unit_multiround_stable_os".into());
            hits.push("unit_multiround_stable".into());
        }
    }
    // OS install instance (ServerMint fork material) — env authenticity conf.
    if has_nonempty(&fo, "os_instance_hash") {
        hits.push("os_instance_hash".into());
        reasons.push("os_instance_hash_present".into());
        // Presence of distinct OS instance is neutral-to-positive for real installs;
        // soft multi-VM still uses residual_soft_like for risk.
        if !soft_phys {
            risk = (risk - 0.04).max(0.0);
        }
    }
    if let Some(n) = f_field(&fo, "multi_seed_n") {
        hits.push("multi_seed_n".into());
        if n >= 8.0 {
            reasons.push(format!("multi_seed_n={n:.0}"));
        }
    }
    if let Some(uq) = f_field(&fo, "identical_draw_unique") {
        hits.push("identical_draw_unique".into());
        // Soft pure stacks often unique=1; high unique under soft claim can be anti-fp noise.
        if uq > 1.0 && soft_phys {
            risk = (risk + 0.05).min(1.0);
            reasons.push("identical_draw_noise_under_soft".into());
        }
    }
    if bool_field(&fo, "software_renderer_heuristic").unwrap_or(false) {
        hits.push("software_renderer_heuristic".into());
        // String heuristic is stealth-patchable; weak when physical soft path already fired.
        let bump = if soft_phys { 0.04 } else { 0.12 };
        risk = (risk + bump).min(1.0);
        reasons.push(if soft_phys {
            "software_renderer_diag".into()
        } else {
            "software_renderer".into()
        });
    }
    // Claim-obs: L0 GPU label vs residual/soft path — weighted OS collusion (not zero-weight ban).
    // Exotic discrete GPU strings with soft residual → claim_collusion (demo Camoufox pattern inverse
    // or spoofed labels on soft GL). Uses renderer as claim; residual as observation.
    {
        let ren = str_field(&fo, "webgl_unmasked_renderer")
            .or_else(|| str_field(&fo, "webgl_renderer"))
            .unwrap_or("")
            .to_ascii_lowercase();
        if !ren.is_empty() {
            hits.push("webgl_unmasked_renderer".into());
        }
        let exotic_claim = ren.contains("geforce")
            || ren.contains("radeon")
            || ren.contains("apple m")
            || ren.contains("adreno")
            || ren.contains("mali")
            || ren.contains("metal")
            || ren.contains("direct3d");
        let soft_obs = soft_phys
            || matches!(
                stack.renderer_class.as_str(),
                "swiftshader" | "llvmpipe" | "softpipe" | "soft_other" | "virt_gpu"
            );
        if exotic_claim && soft_obs {
            // Mid weight: verifies claim-vs-obs; does not alone decide device_id.
            risk = (risk + 0.16).min(1.0);
            reasons.push("gpu_label_vs_residual_soft_claim_obs".into());
            hits.push("residual_soft_like".into());
        } else if exotic_claim && stack.gpu_label_untrusted {
            risk = (risk + 0.12).min(1.0);
            reasons.push("gpu_label_untrusted_claim_obs".into());
        } else if exotic_claim {
            // Reliable-looking label still participates as low-weight environment context.
            risk = (risk + 0.03).min(1.0);
            reasons.push("gpu_label_claim_context_low_weight".into());
        }
    }
    if let Some(ra) = bool_field(&fo, "residual_available") {
        hits.push("residual_available".into());
        if !ra {
            risk = (risk + 0.08).min(1.0);
            reasons.push("residual_unavailable".into());
        }
    }
    // Residual present but low entropy: silicon conf soft demote (device also warns).
    // Does not stop mid/deep packs — score impact only.
    if bool_field(&fo, "webgl_residual_entropy_ok") == Some(false)
        || bool_field(&fo, "residual_entropy_ok") == Some(false)
    {
        hits.push("webgl_residual_entropy_ok".into());
        risk = (risk + 0.05).min(1.0);
        reasons.push("webgl_residual_low_entropy_os".into());
    }
    // X-8/X-9: antidetect / emulator — claim-obs / env only (never commercial digest).
    if bool_field(&fo, "antidetect_vendor_hint").unwrap_or(false)
        || bool_field(&fo, "fingerprint_vendor_lie").unwrap_or(false)
        || has_nonempty(&fo, "antidetect_vendor_hint")
            && fo.get("antidetect_vendor_hint").and_then(|v| v.as_str()).is_some_and(|s| !s.is_empty() && s != "false")
    {
        hits.push("antidetect_vendor_hint".into());
        risk = (risk + 0.14).min(1.0);
        reasons.push("antidetect_vendor_claim_obs".into());
    }
    if bool_field(&fo, "emulator_hint").unwrap_or(false)
        || fo
            .get("emulator_hint")
            .and_then(|v| v.as_str())
            .is_some_and(|s| matches!(s, "android_emu" | "ios_sim" | "qemu" | "ranchu" | "true"))
    {
        hits.push("emulator_hint".into());
        risk = (risk + 0.18).min(1.0);
        reasons.push("emulator_env_claim_obs".into());
    }
    let stack_class = fo
        .get("stack_class")
        .and_then(|v| v.as_str())
        .or(Some(stack.stack_class.as_str()))
        .unwrap_or("unknown");
    if has_nonempty(&fo, "stack_class") || !stack.stack_class.is_empty() {
        hits.push("stack_class".into());
    }
    if matches!(
        stack_class,
        "virt" | "vm" | "cloud" | "emulator" | "soft_render" | "software"
    ) {
        // Label path weaker when soft_stack/residual already applied (iss/21 T-OS-1).
        let bump = if soft_phys { 0.10 } else { 0.28 };
        risk = (risk + bump).min(1.0);
        reasons.push(format!("stack_class={stack_class}"));
    } else if stack_class == "real_silicon" || stack_class == "native" {
        risk = (risk - 0.1).max(0.0);
        reasons.push(format!("stack_class={stack_class}"));
    }
    if let Some(er) = fo.get("env_reasons") {
        if er.as_array().map(|a| !a.is_empty()).unwrap_or(false) || er.as_str().is_some() {
            hits.push("env_reasons".into());
            reasons.push("env_reasons_present".into());
            // env_reasons often list virt/cloud signals — slight risk
            risk = (risk + 0.06).min(1.0);
        }
    }
    if let Some(hint) = str_field(&fo, "authenticity_hint") {
        hits.push("authenticity_hint".into());
        if hint.contains("soft") || hint.contains("virt") || hint.contains("vm") {
            risk = (risk + 0.12).min(1.0);
            reasons.push(format!("authenticity_hint={hint}"));
        }
    }
    // iss/21 T-EMU-1 + iss/38 P0-2: Mobile UA claim vs capability form_class (obs).
    // Derive claim from user_agent when explicit mobile_ua_claim absent (reference_aux).
    let ua_raw = str_field(&fo, "user_agent").unwrap_or("");
    let mobile_claim = bool_field(&fo, "mobile_ua_claim")
        .or_else(|| bool_field(&fo, "mobile_ua_signals"))
        .unwrap_or_else(|| {
            let u = ua_raw.to_ascii_lowercase();
            u.contains("mobile")
                || u.contains("android")
                || u.contains("iphone")
                || u.contains("ipad")
        });
    if !ua_raw.is_empty() {
        hits.push("user_agent".into());
    }
    if let Some(form) = str_field(&fo, "form_class") {
        hits.push("form_class".into());
        if mobile_claim {
            hits.push("mobile_ua_claim".into());
            if form == "desktop" {
                risk = (risk + 0.14).min(1.0);
                reasons.push("mobile_ua_claim_vs_desktop_form".into());
                reasons.push("ua_form_aux_contradict".into());
            } else if form == "mobile" || form == "phone" || form == "tablet" {
                risk = (risk - 0.02).max(0.0);
                reasons.push("ua_form_aux_confirm_low_weight".into());
            }
        }
    }
    // iss/38 P0-1: IP/ASN reference_aux on os (never digest / never UV merge key).
    apply_os_net_reference_aux(&fo, soft_phys, mobile_claim, &mut risk, &mut reasons, &mut hits);
    // iss/38 P1-2: joint emulator form × sensors × GPU × touch.
    apply_emulator_form_cross(&fo, soft_phys, mobile_claim, &mut risk, &mut reasons, &mut hits);
    // iss/38 P2-2: worker throughput vs concurrency (weak).
    apply_worker_throughput_aux(&fo, &mut risk, &mut reasons, &mut hits);
    // Hardware curves & capacity — positive
    for k in [
        "hw_curve_webgl",
        "hw_curve_audio",
        "hw_curve_canvas",
        "hw_curve_cpu",
    ] {
        if has_nonempty(&fo, k) {
            risk = (risk - 0.04).max(0.0);
            hits.push(k.into());
        }
    }
    if has_nonempty(&fo, "hw_curve_webgl") || has_nonempty(&fo, "hw_curve_audio") {
        reasons.push("hw_curves_present".into());
        risk = (risk - 0.06).max(0.0);
    }
    // Residual deep materials (B4/B9/B17) strengthen os confidence when present.
    if has_nonempty(&fo, "gl_precision_matrix") || has_nonempty(&fo, "cpu_timing_curve") {
        hits.push("hw_physical".into());
        risk = (risk - 0.05).max(0.0);
        reasons.push("hw_physical_materials".into());
    }
    if has_nonempty(&fo, "gpu_wall_staircase")
        || has_nonempty(&fo, "gpu_slope_wall")
        || has_nonempty(&fo, "gpu_wall_staircase_digest")
    {
        hits.push("gpu_timer_h01".into());
        risk = (risk - 0.04).max(0.0);
        reasons.push("gpu_staircase_present".into());
        if has_nonempty(&fo, "gpu_wall_staircase_digest") {
            hits.push("gpu_wall_staircase_digest".into());
            risk = (risk - 0.02).max(0.0);
        }
        // Soft-like super-linear slope under high texture claim
        if let Some(s) = f_field(&fo, "gpu_slope_wall") {
            if s > 1e-4 {
                risk = (risk + 0.08).min(1.0);
                reasons.push("gpu_slope_soft_like".into());
            }
        }
    }
    if has_nonempty(&fo, "gpu_bandwidth_ladder") {
        hits.push("gpu_bandwidth".into());
        risk = (risk - 0.02).max(0.0);
        reasons.push("gpu_bandwidth_ladder".into());
    }
    if has_nonempty(&fo, "gpu_r2_wall") || has_nonempty(&fo, "gpu_disjoint_rate") {
        hits.push("h01_timer_meta".into());
        risk = (risk - 0.02).max(0.0);
    }
    if has_nonempty(&fo, "gpu_ns_staircase")
        || has_nonempty(&fo, "gpu_ns_staircase_digest")
        || has_nonempty(&fo, "gpu_ns_median")
        || bool_field(&fo, "gpu_ns_readback_ok").unwrap_or(false)
    {
        hits.push("gpu_ns_h01".into());
        risk = (risk - 0.04).max(0.0);
        reasons.push("gpu_ns_materials".into());
        if bool_field(&fo, "gpu_ns_readback_ok").unwrap_or(false)
            || has_nonempty(&fo, "gpu_ns_staircase_digest")
        {
            hits.push("gpu_ns_readback_ok".into());
            risk = (risk - 0.02).max(0.0);
        }
        if f_field(&fo, "gpu_disjoint_rate").unwrap_or(0.0) > 0.5 {
            risk = (risk + 0.08).min(1.0);
            reasons.push("gpu_disjoint_rate_high".into());
        }
    }
    if has_nonempty(&fo, "gpu_readback_ladder") || has_nonempty(&fo, "roundtrip_intercept_ms") {
        hits.push("gpu_bandwidth_h02".into());
        risk = (risk - 0.03).max(0.0);
    }
    if has_nonempty(&fo, "cpu_cache_ladder") || has_nonempty(&fo, "cpu_cache_knee_bytes") {
        hits.push("cpu_cache_h07".into());
        risk = (risk - 0.03).max(0.0);
    }
    if has_nonempty(&fo, "permissions_matrix") || has_nonempty(&fo, "media_devices_count") {
        hits.push("permissions_media".into());
        risk = (risk - 0.02).max(0.0);
    }
    if has_nonempty(&fo, "battery_level") || bool_field(&fo, "sensor_accel_present").unwrap_or(false)
    {
        hits.push("sensors_battery".into());
        risk = (risk - 0.02).max(0.0);
    }
    if has_nonempty(&fo, "challenge_seed_sig") {
        hits.push("signed_challenge".into());
        if bool_field(&fo, "challenge_seed_signed").unwrap_or(true) {
            risk = (risk - 0.02).max(0.0);
        }
    }
    if has_nonempty(&fo, "agent_parity_hash") || has_nonempty(&fo, "agent_automation_globals_n") {
        hits.push("agent_parity".into());
        if f_field(&fo, "agent_automation_globals_n").unwrap_or(0.0) >= 1.0 {
            risk = (risk + 0.15).min(1.0);
            reasons.push("agent_parity_automation_globals".into());
        } else {
            risk = (risk - 0.02).max(0.0);
        }
    }
    if has_nonempty(&fo, "privacy_storage_score") || has_nonempty(&fo, "storage_quota") {
        hits.push("storage_privacy".into());
        risk = (risk - 0.02).max(0.0);
    }
    if has_nonempty(&fo, "dom_rect_hash") || has_nonempty(&fo, "perf_timeline_hash") {
        hits.push("dom_perf".into());
        risk = (risk - 0.02).max(0.0);
    }
    if has_nonempty(&fo, "layer_divergence_score") {
        hits.push("layer_divergence".into());
        if f_field(&fo, "layer_divergence_score").unwrap_or(1.0) < 0.7 {
            risk = (risk + 0.1).min(1.0);
            reasons.push("layer_divergence_high".into());
        } else {
            risk = (risk - 0.03).max(0.0);
        }
    }
    if has_nonempty(&fo, "webgpu_limits_hash") || has_nonempty(&fo, "webgpu_dual_adapter_diff") {
        hits.push("webgpu_deep".into());
        risk = (risk - 0.02).max(0.0);
    }
    if has_nonempty(&fo, "raster_edge_hash") || has_nonempty(&fo, "raster_edge_curve") {
        hits.push("raster_msaa_h04".into());
        risk = (risk - 0.02).max(0.0);
    }
    if has_nonempty(&fo, "thermal_cpu_slope") || has_nonempty(&fo, "thermal_bursts") {
        hits.push("thermal_h09".into());
        if f_field(&fo, "thermal_cpu_slope").unwrap_or(0.0) > 0.35 {
            risk = (risk + 0.06).min(1.0);
            reasons.push("thermal_cpu_slope_high".into());
        }
    }
    if has_nonempty(&fo, "neg_dict_hits") || has_nonempty(&fo, "neg_dict_hit_n") {
        hits.push("neg_dict_h15".into());
        let n = f_field(&fo, "neg_dict_hit_n").unwrap_or(0.0);
        if n >= 2.0 {
            risk = (risk + 0.12).min(1.0);
            reasons.push("neg_dict_multi_hit".into());
        }
    }
    if has_nonempty(&fo, "mem_alloc_ladder") || has_nonempty(&fo, "mem_alloc_max_mb") {
        hits.push("mem_pressure".into());
        risk = (risk - 0.02).max(0.0);
    }
    if has_nonempty(&fo, "ws_handshake_ms") || bool_field(&fo, "websocket_present").is_some() {
        hits.push("websocket_fp".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "hid_surface_score") || has_nonempty(&fo, "gamepad_count") {
        hits.push("hid_gamepad".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "gpu_slope_ns") || has_nonempty(&fo, "gpu_ns_wall_ratio_median") {
        hits.push("gpu_ns_extended".into());
        risk = (risk - 0.03).max(0.0);
        if f_field(&fo, "gpu_ns_wall_ratio_median").unwrap_or(0.0) > 1.5 {
            risk = (risk + 0.1).min(1.0);
            reasons.push("gpu_ns_wall_ratio_inconsistent".into());
        }
    }
    if has_nonempty(&fo, "thermal_cpu_slope_full") || has_nonempty(&fo, "thermal_bursts_full") {
        hits.push("thermal_full_h09".into());
        if f_field(&fo, "thermal_cpu_slope_full").unwrap_or(0.0) > 0.4 {
            risk = (risk + 0.08).min(1.0);
            reasons.push("thermal_full_slope_high".into());
        }
    }
    if has_nonempty(&fo, "errors_engine_hash") {
        hits.push("errors_engine".into());
        risk = (risk - 0.02).max(0.0);
    }
    if has_nonempty(&fo, "speech_voices_hash") || has_nonempty(&fo, "speech_voices_count") {
        hits.push("speech_voices".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "display_mq_hash") || has_nonempty(&fo, "css_color_gamut") {
        hits.push("display_hdr".into());
        risk = (risk - 0.02).max(0.0);
    }
    // CSS hover capability (B14): env/surface conf for os — presence of real MQ support
    if bool_field(&fo, "css_hover_hover").is_some() || has_nonempty(&fo, "css_hover_hover") {
        hits.push("css_hover_hover".into());
        risk = (risk - 0.01).max(0.0);
        // Fine pointer + none hover on "desktop" form can be virt/thin shell (weak)
        if bool_field(&fo, "css_hover_hover") == Some(false)
            && str_field(&fo, "form_class") == Some("desktop")
        {
            risk = (risk + 0.04).min(1.0);
            reasons.push("css_hover_none_on_desktop_form".into());
        }
    }
    // timezone_offset_min is matrix os **material** (B0/B3): must be consumed, not density-only.
    // Without a full tz database: presence strengthens coverage; name vs zero-offset mismatch is forgery-ish.
    if let Some(off) = f_field(&fo, "timezone_offset_min") {
        hits.push("timezone_offset_min".into());
        risk = (risk - 0.02).max(0.0);
        if let Some(tz) = str_field(&fo, "timezone") {
            let tzl = tz.to_ascii_lowercase();
            let claims_non_utc = !(tzl.contains("utc")
                || tzl == "gmt"
                || tzl.starts_with("etc/utc")
                || tzl.starts_with("etc/gmt"));
            // Named non-UTC zone with offset exactly 0 is a common spoof/thin FE pattern
            if claims_non_utc && off.abs() < 0.5 {
                risk = (risk + 0.10).min(1.0);
                reasons.push("timezone_offset_vs_name_suspect".into());
            }
        }
    } else if str_field(&fo, "timezone").is_some() {
        // timezone name without offset material → thin os surface
        hits.push("timezone_without_offset".into());
        risk = (risk + 0.03).min(1.0);
    }
    if has_nonempty(&fo, "ja4") || has_nonempty(&fo, "protocol_engine") {
        hits.push("protocol_edge_fp".into());
        risk = (risk - 0.02).max(0.0);
    }
    if has_nonempty(&fo, "h2_fingerprint")
        || has_nonempty(&fo, "h2_settings_available")
        || has_nonempty(&fo, "h2_fingerprint_hash")
    {
        hits.push("h2_fingerprint_fp".into());
        risk = (risk - 0.02).max(0.0);
        if has_nonempty(&fo, "h2_fingerprint_hash") {
            hits.push("h2_fingerprint_hash".into());
        }
    }
    if has_nonempty(&fo, "quic_tls_ja4") || has_nonempty(&fo, "quic_listen_present") {
        hits.push("quic_side_fp".into());
        risk = (risk - 0.02).max(0.0);
        if let (Some(tj), Some(qj)) = (
            fo.get("ja4").and_then(|v| v.as_str()),
            fo.get("quic_tls_ja4").and_then(|v| v.as_str()),
        ) {
            if !tj.is_empty() && !qj.is_empty() && tj != qj {
                risk = (risk + 0.08).min(1.0);
                reasons.push("tcp_quic_ja4_mismatch".into());
            } else if !tj.is_empty() && tj == qj {
                hits.push("protocol_tcp_quic_agree".into());
                risk = (risk - 0.02).max(0.0);
            }
        }
    }
    if has_nonempty(&fo, "tcp_syn_present") || has_nonempty(&fo, "tcp_syn_options_sig") {
        hits.push("tcp_syn_side".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "webrtc_side_present") || has_nonempty(&fo, "dtls_ja4") {
        hits.push("webrtc_side_fp".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "challenge_eval") {
        hits.push("challenge_eval".into());
        let ch_ok = fo
            .get("challenge_eval")
            .and_then(|v| v.get("ok"))
            .and_then(|v| v.as_bool());
        if ch_ok == Some(true) {
            risk = (risk - 0.02).max(0.0);
        } else if ch_ok == Some(false) {
            risk = (risk + 0.1).min(1.0);
            reasons.push("challenge_eval_failed".into());
        }
    }
    if has_nonempty(&fo, "audio_deep_hash") || has_nonempty(&fo, "audio_deep_moments") {
        hits.push("audio_deep_d03".into());
        risk = (risk - 0.02).max(0.0);
    }
    if has_nonempty(&fo, "gpu_ns_hardware_path")
        || f_field(&fo, "gpu_ns_points").unwrap_or(0.0) >= 4.0
    {
        hits.push("gpu_ns_hardware".into());
        risk = (risk - 0.04).max(0.0);
    }
    // H01 depth metrics (R² / monotonic / CV / composite) — beyond points presence
    if has_nonempty(&fo, "gpu_r2_ns")
        || has_nonempty(&fo, "gpu_ns_depth_score")
        || has_nonempty(&fo, "gpu_ns_monotonic_ratio")
        || has_nonempty(&fo, "gpu_ns_cv")
    {
        hits.push("gpu_ns_depth".into());
        let depth = f_field(&fo, "gpu_ns_depth_score").unwrap_or(0.0);
        if depth >= 0.7 {
            risk = (risk - 0.04).max(0.0);
            reasons.push("gpu_ns_depth_strong".into());
        } else if depth > 0.0 && depth < 0.35 {
            risk = (risk + 0.06).min(1.0);
            reasons.push("gpu_ns_depth_weak".into());
        }
        if let Some(r2) = f_field(&fo, "gpu_r2_ns") {
            if r2 < 0.3 && f_field(&fo, "gpu_ns_points").unwrap_or(0.0) >= 4.0 {
                risk = (risk + 0.08).min(1.0);
                reasons.push("gpu_r2_ns_low".into());
            } else if r2 >= 0.85 {
                risk = (risk - 0.02).max(0.0);
            }
        }
        if let Some(m) = f_field(&fo, "gpu_ns_monotonic_ratio") {
            if m < 0.5 && f_field(&fo, "gpu_ns_points").unwrap_or(0.0) >= 3.0 {
                risk = (risk + 0.06).min(1.0);
                reasons.push("gpu_ns_non_monotonic".into());
            }
        }
    }
    // H10 object-depth codec/EME
    if has_nonempty(&fo, "codec_probably_n")
        || has_nonempty(&fo, "codec_matrix_n")
        || has_nonempty(&fo, "eme_supported_n")
        || has_nonempty(&fo, "canplay_matrix")
    {
        hits.push("codec_eme_depth".into());
        let score = f_field(&fo, "codec_support_score").unwrap_or(0.0);
        let n = f_field(&fo, "codec_matrix_n").unwrap_or(0.0);
        if bool_field(&fo, "codec_virt_hint").unwrap_or(false) || (n >= 8.0 && score <= 0.0) {
            risk = (risk + 0.12).min(1.0);
            reasons.push("codec_matrix_empty_virt_hint".into());
        } else if score >= 6.0 {
            risk = (risk - 0.03).max(0.0);
            reasons.push("codec_matrix_rich".into());
        }
        if f_field(&fo, "eme_supported_n").unwrap_or(0.0) >= 1.0 {
            hits.push("eme_supported".into());
            risk = (risk - 0.02).max(0.0);
        } else if has_nonempty(&fo, "eme_systems")
            && f_field(&fo, "eme_unsupported_n").unwrap_or(0.0) >= 3.0
        {
            risk = (risk + 0.06).min(1.0);
            reasons.push("eme_all_unsupported_depth".into());
        }
        if bool_field(&fo, "eme_widevine").unwrap_or(false) {
            hits.push("eme_widevine".into());
            risk = (risk - 0.01).max(0.0);
        }
    }
    if has_nonempty(&fo, "gl_precision_matrix") {
        hits.push("gl_precision_matrix".into());
        risk = (risk - 0.03).max(0.0);
    }
    if let (Some(claim), Some(actual)) = (
        f_field(&fo, "gl_max_texture_size").or_else(|| f_field(&fo, "claimed_max_tex")),
        f_field(&fo, "actual_max_tex"),
    ) {
        hits.push("caps_pressure".into());
        if claim >= 16384.0 && actual < claim * 0.5 {
            risk = (risk + 0.2).min(1.0);
            reasons.push("caps_claim_vs_actual".into());
        }
    }
    if has_nonempty(&fo, "shader_ulp_spectrum") || has_nonempty(&fo, "shader_ulp_mean") {
        hits.push("shader_ulp_spectrum".into());
        risk = (risk - 0.02).max(0.0);
        if bool_field(&fo, "mediump_diverges").unwrap_or(false) {
            hits.push("mediump_diverges".into());
        }
    }
    if let Some(ulp) = f_field(&fo, "shader_ulp_max") {
        hits.push("shader_ulp".into());
        if ulp > 8.0 {
            risk = (risk + 0.12).min(1.0);
            reasons.push("precision_claim_vs_behavior".into());
        }
    }
    if has_nonempty(&fo, "perf_now_resolution_ms") || has_nonempty(&fo, "raf_jitter_cv") {
        hits.push("clock_phys".into());
        if f_field(&fo, "perf_now_resolution_ms").unwrap_or(0.0) >= 0.1 {
            risk = (risk + 0.06).min(1.0);
            reasons.push("clock_coarse_resolution".into());
        }
    }
    if has_nonempty(&fo, "challenge_avg_cv") || has_nonempty(&fo, "session_residual_cv") {
        hits.push("challenge_cv".into());
        if f_field(&fo, "challenge_avg_cv")
            .or_else(|| f_field(&fo, "session_residual_cv"))
            .unwrap_or(0.0)
            > 0.05
        {
            risk = (risk + 0.1).min(1.0);
            reasons.push("noise_injection_likely".into());
        }
    }
    if has_nonempty(&fo, "font_bitmap_hash") || has_nonempty(&fo, "api_flags_hash") {
        hits.push("census_volume".into());
        risk = (risk - 0.03).max(0.0);
    }
    if fo
        .get("census_leaf_estimate")
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x as i64)))
        .unwrap_or(0)
        >= 2000
    {
        hits.push("census_leaf_dense".into());
        risk = (risk - 0.02).max(0.0);
        reasons.push("census_leaf_dense".into());
    }
    if has_nonempty(&fo, "webrtc_host_ip_hash") {
        hits.push("webrtc_host_ip_hash".into());
        risk = (risk - 0.02).max(0.0);
    } else {
        // Missing host sep: demote OS conf only — never halt other packs (probe continues).
        // Distinguish platform-unavailable (WebKitGTK no_rtc) vs engine-should-have-RTC gap.
        let fail = str_field(&fo, "webrtc_probe_failed").unwrap_or("");
        let missing_flag = bool_field(&fo, "webrtc_missing").unwrap_or(false)
            || fo.get("webrtc_missing").and_then(|v| v.as_bool()) == Some(true)
            || !fail.is_empty()
            || fo
                .get("webrtc_host_count")
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x as i64)))
                .unwrap_or(-1)
                == 0;
        if missing_flag || fo.contains_key("webrtc_probe_failed") {
            hits.push("webrtc_missing".into());
            if fail == "no_rtc" {
                // Platform cannot expose RTC — incomplete host-class evidence, not bot proof.
                risk = (risk + 0.03).min(1.0);
                reasons.push("webrtc_platform_unavailable".into());
            } else if !fail.is_empty() {
                risk = (risk + 0.06).min(1.0);
                reasons.push(format!("webrtc_evidence_gap={fail}"));
            } else if missing_flag {
                risk = (risk + 0.04).min(1.0);
                reasons.push("webrtc_host_absent".into());
            }
        }
    }
    if has_nonempty(&fo, "ua_ch_platform") || has_nonempty(&fo, "ua_ch_architecture") {
        hits.push("ua_ch".into());
        risk = (risk - 0.02).max(0.0);
    }
    // UA-CH high-entropy pack (iss/18 P0 F-5)
    if has_nonempty(&fo, "ua_ch_model")
        || has_nonempty(&fo, "ua_ch_platform_version")
        || has_nonempty(&fo, "ua_ch_bitness")
        || fo.get("ua_ch_mobile").is_some()
        || has_nonempty(&fo, "ua_ch_full_version_list")
    {
        hits.push("ua_ch_high_entropy".into());
        risk = (risk - 0.02).max(0.0);
        if has_nonempty(&fo, "ua_ch_platform_version") {
            hits.push("ua_ch_platform_version".into());
        }
        if has_nonempty(&fo, "ua_ch_model") {
            hits.push("ua_ch_model".into());
        }
    }
    if has_nonempty(&fo, "storage_quota_class") {
        hits.push("storage_quota_class".into());
        risk = (risk - 0.02).max(0.0);
    }
    // ICE morphology (iss/18 P0 F-6)
    if has_nonempty(&fo, "ice_candidate_types")
        || bool_field(&fo, "ice_has_host").is_some()
        || has_nonempty(&fo, "ice_candidate_count")
        || has_nonempty(&fo, "webrtc_host_count")
    {
        hits.push("ice_morphology".into());
        risk = (risk - 0.02).max(0.0);
        if bool_field(&fo, "ice_has_host") == Some(true) {
            hits.push("ice_has_host".into());
        }
        if bool_field(&fo, "ice_has_srflx") == Some(true) {
            hits.push("ice_has_srflx".into());
            risk = (risk - 0.01).max(0.0);
        }
        // host-only without srflx can indicate privacy / restricted ICE
        if bool_field(&fo, "ice_has_host") == Some(true)
            && bool_field(&fo, "ice_has_srflx") == Some(false)
            && bool_field(&fo, "ice_has_relay") != Some(true)
        {
            risk = (risk + 0.04).min(1.0);
            reasons.push("ice_host_only".into());
        }
        if bool_field(&fo, "ice_has_host") == Some(false)
            && bool_field(&fo, "ice_has_srflx") == Some(false)
            && f_field(&fo, "ice_candidate_count").unwrap_or(0.0) <= 0.0
            && f_field(&fo, "webrtc_host_count").unwrap_or(0.0) <= 0.0
        {
            risk = (risk + 0.05).min(1.0);
            reasons.push("ice_morphology_none".into());
        }
    }
    // Device orientation / motion vectors (iss/18 P0 F-4)
    if has_nonempty(&fo, "device_orientation_alpha")
        || has_nonempty(&fo, "device_orientation_beta")
        || has_nonempty(&fo, "device_orientation_gamma")
        || has_nonempty(&fo, "device_motion_accel_x")
        || has_nonempty(&fo, "device_motion_accel_y")
        || has_nonempty(&fo, "device_motion_accel_z")
        || bool_field(&fo, "device_motion_sample_ok").unwrap_or(false)
    {
        hits.push("device_motion_vector".into());
        risk = (risk - 0.02).max(0.0);
        if has_nonempty(&fo, "device_motion_accel_y") {
            hits.push("device_motion_accel_y".into());
        }
        if has_nonempty(&fo, "device_motion_accel_z") {
            hits.push("device_motion_accel_z".into());
        }
        if has_nonempty(&fo, "device_orientation_gamma") {
            hits.push("device_orientation_gamma".into());
        }
    }
    // DNS cache delta (iss/18 P0)
    if has_nonempty(&fo, "dns_cache_timing_delta_ms") {
        hits.push("dns_cache_timing_delta_ms".into());
        risk = (risk - 0.01).max(0.0);
    }
    // JA4_r + CH order digests (iss/18 F-7)
    if has_nonempty(&fo, "ja4_r")
        || has_nonempty(&fo, "tls_extensions_order")
        || has_nonempty(&fo, "cipher_suites_order")
    {
        hits.push("tls_hello_order".into());
        risk = (risk - 0.02).max(0.0);
        if has_nonempty(&fo, "ja4_r") {
            hits.push("ja4_r".into());
        }
    }
    if has_nonempty(&fo, "cross_residual_delta") {
        hits.push("cross_context".into());
        if bool_field(&fo, "cross_residual_match").unwrap_or(false) {
            risk = (risk - 0.03).max(0.0);
            reasons.push("cross_context_match".into());
        } else if let Some(d) = f_field(&fo, "cross_residual_delta") {
            if d > 1e-3 {
                risk = (risk + 0.08).min(1.0);
                reasons.push("cross_context_mismatch".into());
            }
        }
    }
    if f_field(&fo, "device_memory").unwrap_or(0.0) >= 4.0 {
        hits.push("device_memory".into());
        risk = (risk - 0.04).max(0.0);
    }
    if f_field(&fo, "hardware_concurrency").unwrap_or(0.0) >= 4.0 {
        hits.push("hardware_concurrency".into());
        risk = (risk - 0.03).max(0.0);
    }
    // Clock / timing surface
    if has_nonempty(&fo, "perf_now") {
        hits.push("perf_now".into());
        risk = (risk - 0.02).max(0.0);
    }
    if has_nonempty(&fo, "time_origin") {
        hits.push("time_origin".into());
    }
    // Network capacity mid signals (weak os conf)
    let downlink = f_field(&fo, "downlink").or_else(|| f_field(&fo, "net_downlink"));
    if let Some(d) = downlink {
        hits.push("downlink".into());
        if d <= 0.0 {
            risk = (risk + 0.03).min(1.0);
        }
    }
    if let Some(mt) = f_field(&fo, "max_touch").or_else(|| f_field(&fo, "max_touch_points")) {
        hits.push("max_touch".into());
        // mobile touch on "desktop" form can be env signal
        if mt > 0.0 && str_field(&fo, "form_class") == Some("desktop") {
            // not necessarily bad — mild conf only
            reasons.push("touch_on_desktop_form".into());
        }
    }
    if bool_field(&fo, "claims_host_silicon_recovery").unwrap_or(false) {
        hits.push("claims_host_silicon_recovery".into());
        // Never strengthen safety from this claim (redline-adjacent)
        reasons.push("host_silicon_claim_ignored".into());
    }
    // iss/25: claim-collusion channel (fp-browser / forged env).
    // Root-cause class: spoofable labels can be multi-source *consistent* while residual is soft.
    // General rule: high multi_source match of claims + soft residual → os risk (not br-only).
    let multi_match = f_field(&fo, "multi_source_match_ratio");
    if soft_phys {
        if let Some(r) = multi_match {
            hits.push("multi_source_match_ratio".into());
            if r >= 0.85 {
                risk = (risk + 0.18).min(1.0);
                reasons.push(format!("claim_collusion_multi_source_soft={r:.2}"));
            } else if r >= 0.70 {
                risk = (risk + 0.10).min(1.0);
                reasons.push(format!("claim_collusion_multi_source_soft_mid={r:.2}"));
            }
        }
        // Coordinated spoof score with soft residual (label path agrees with itself)
        let spoof = f_field(&fo, "spoof_score").unwrap_or(stack.spoof_score);
        if spoof >= 0.45 {
            hits.push("spoof_score".into());
            risk = (risk + 0.10).min(1.0);
            reasons.push(format!("claim_collusion_spoof_soft={spoof:.2}"));
        }
        if stack.gpu_label_untrusted {
            hits.push("gpu_label_untrusted".into());
            risk = (risk + 0.08).min(1.0);
            reasons.push("claim_collusion_gpu_label_soft".into());
        }
    }
    // --- T2 conf close-up (matrix scorer contract; forgeability L1/L2 conf only) ---
    // caps_rb_vs_claim: renderbuffer claim vs actual pressure (os conf)
    if let Some(g) = f_field(&fo, "caps_rb_vs_claim") {
        hits.push("caps_rb_vs_claim".into());
        if g > 0.5 {
            risk = (risk + 0.10).min(1.0);
            reasons.push("caps_rb_vs_claim_high".into());
        } else {
            risk = (risk - 0.01).max(0.0);
        }
    }
    // CSS pointer coarse / media queries lite — env surface conf
    if bool_field(&fo, "css_pointer_coarse").is_some() || has_nonempty(&fo, "css_pointer_coarse") {
        hits.push("css_pointer_coarse".into());
        risk = (risk - 0.01).max(0.0);
        if bool_field(&fo, "css_pointer_coarse") == Some(true)
            && str_field(&fo, "form_class") == Some("desktop")
        {
            // coarse pointer on desktop form can be thin/virt shell
            risk = (risk + 0.04).min(1.0);
            reasons.push("css_pointer_coarse_on_desktop".into());
        }
    }
    if has_nonempty(&fo, "media_queries_lite") {
        hits.push("media_queries_lite".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "intl_locale") {
        hits.push("intl_locale".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "platform_version") {
        hits.push("platform_version".into());
        risk = (risk - 0.01).max(0.0);
    }
    // WebRTC srflx surface (soft host separator conf; not commercial digest)
    if has_nonempty(&fo, "webrtc_srflx_count") || has_nonempty(&fo, "webrtc_srflx_ip_hash") {
        hits.push("webrtc_srflx".into());
        risk = (risk - 0.02).max(0.0);
        if has_nonempty(&fo, "webrtc_srflx_ip_hash") {
            hits.push("webrtc_srflx_ip_hash".into());
        }
        if has_nonempty(&fo, "webrtc_srflx_count") {
            hits.push("webrtc_srflx_count".into());
        }
    }
    if has_nonempty(&fo, "net_effective_type") {
        hits.push("net_effective_type".into());
        risk = (risk - 0.01).max(0.0);
    }
    // demo/iss2 forgeability aliases (may arrive from demo FE or mapped packs)
    if bool_field(&fo, "soft_stack_suspect").unwrap_or(false)
        || bool_field(&fo, "pc_vm_suspect").unwrap_or(false)
    {
        hits.push("demo_env_suspect".into());
        risk = (risk + 0.12).min(1.0);
        reasons.push("demo_env_suspect_alias".into());
    }
    if bool_field(&fo, "soft_only").unwrap_or(false) || has_nonempty(&fo, "residual_head") {
        hits.push("demo_residual_soft_alias".into());
        if soft_phys {
            risk = (risk + 0.06).min(1.0);
            reasons.push("demo_soft_residual_alias".into());
        } else {
            risk = (risk - 0.01).max(0.0);
        }
    }
    if !truth.has_server_side {
        risk = (risk + 0.05).min(1.0);
        reasons.push("no_server_side_cap".into());
    } else {
        hits.push("server_side".into());
    }
    // iss/38 N3: rule sample pack low-weight on os (never digest).
    crate::rule_sample_loader::apply_rule_samples_axis(
        fields, "os", &mut risk, &mut reasons, &mut hits,
    );
    // Close silent-unused FE materials into OS axis (why-use coded reasons).
    apply_probe_surface_evidence_os(&fo, &mut risk, &mut reasons, &mut hits);

    // Multi-source sandbox capability + cross-context mismatch + font first-difference
    apply_sandbox_capability_axis("os", &fo, &mut risk, &mut reasons, &mut hits);
    apply_cross_context_mismatch_axis("os", &fo, &mut risk, &mut reasons, &mut hits);
    apply_font_surface_axis("os", &fo, &mut risk, &mut reasons, &mut hits);
    // Identity multi-source conflicts demote OS (weights from product_policy / env).
    if xsrc_has_conflict(source_conflicts, truth) {
        let w = crate::policy::multi_source_conflict_weights();
        let id_conflicts = source_conflicts
            .iter()
            .filter(|c| {
                c.contains("platform")
                    || c.contains("hardware_concurrency")
                    || c.contains("os_family")
                    || c.contains("timezone")
                    || c.contains("device_memory")
            })
            .count();
        if id_conflicts > 0 {
            hits.push("identity_source_conflict_os".into());
            let n = id_conflicts.min(w.os_max_n) as f64;
            risk = (risk + w.os_base + w.os_per * n).min(1.0);
            reasons.push(format!("identity_source_conflict_n={id_conflicts}"));
        }
    }
    // Gate-stamped pressures (evaluate stamps mint_* onto fields)
    if let Some(cp) = f_field(&fo, "mint_conflict_pressure") {
        if cp > 0.0 {
            hits.push("mint_conflict_pressure".into());
            risk = (risk + 0.08 + 0.24 * cp).min(1.0);
            reasons.push(format!("mint_conflict_pressure={cp:.2}"));
        }
    }
    if let Some(sp) = f_field(&fo, "mint_single_source_pressure") {
        if sp > 0.25 {
            hits.push("mint_single_source_pressure".into());
            risk = (risk + 0.05 * sp).min(1.0);
            reasons.push(format!("mint_single_source_pressure={sp:.2}"));
        }
    }

    apply_matrix_role_evidence("os", &fo, &mut risk, &mut reasons, &mut hits);
    apply_dense_digest_evidence("os", &fo, &mut risk, &mut reasons, &mut hits);
    apply_random_verify_authenticity(&fo, &mut risk, &mut reasons, &mut hits);
    apply_cross_axis_corroboration("os", &fo, &mut risk, &mut reasons, &mut hits);

    // Material-family / hit breadth before coverage cap (matrix growth must not alone crush safety).
    // iss/50 H5: cap pure field-presence padding — hit_cov alone cannot dominate without materials.
    let (mat_n, mat_fams) = count_material_families("os", &fo);
    let mut hit_cov = (hits.len() as f64 / 18.0).clamp(0.0, 0.75);
    let presence_cap = if mat_n < 2 { 0.35 } else if mat_n < 4 { 0.55 } else { 0.75 };
    if hit_cov > presence_cap {
        reasons.push(format!("os_presence_pad_cap={presence_cap:.2}"));
        hit_cov = presence_cap;
    }
    let eff_cov = coverage
        .max((mat_n as f64 / 6.0) * 0.65)
        .max(hit_cov)
        .min(1.0);
    let mut safety = 1.0 - risk;
    let cov_cap = 0.40 + 0.60 * eff_cov;
    if safety > cov_cap {
        reasons.push(format!("coverage_cap={cov_cap:.2}"));
        safety = cov_cap;
    }
    if coverage < 0.2 && mat_n < 2 {
        reasons.push("os_coverage_thin".into());
        safety = safety.min(0.45);
    }

    if reasons.iter().any(|r| r.contains("cross_platform_vs_ua_ch")) {
        safety = (safety - 0.12).max(0.05);
        reasons.push("os_platform_ua_ch_post_cap".into());
    }
    if reasons.iter().any(|r| r.contains("cross_soft_residual_vs_fancy_gpu") || r.contains("cross_multisource_agree_soft")) {
        safety = (safety - 0.1).max(0.05);
    }
    if reasons.is_empty() {
        reasons.push("os_field_fusion".into());
    }

    // Multi-channel hedge: never single field / single analyzer alone.
    // Only hard env contradictions become Contradict (risk already baked into safety).
    let env_contradict = safety < 0.35
        || (bool_field(&fo, "residual_soft_like").unwrap_or(false) && stack.spoof_score >= 0.55);
    let (fused_safety, fused_status, fused_reasons, channel_diag) = fuse_axis_hedge(
        "os",
        &fo,
        safety,
        eff_cov.max(coverage),
        reasons,
        env_contradict,
        source_conflicts,
        xsrc_has_conflict(source_conflicts, truth),
    );
    for f in &mat_fams {
        hits.push(format!("mat:{f}"));
    }
    // Soft / cloud-phone / virt environment flags (V5 strengthen path).
    let env_flags = crate::peer_similarity::environment_flags(fields, stack.soft_stack);
    let mut adj_safety = fused_safety;
    let mut adj_reasons = fused_reasons;
    if let Some(boost) = env_flags.get("os_risk_boost").and_then(|v| v.as_f64()) {
        if boost > 0.0 {
            adj_safety = (adj_safety - boost).max(0.05);
            if env_flags
                .get("cloud_phone_suspect")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                adj_reasons.push("environment_cloud_phone_suspect".into());
                hits.push("environment_flags:cloud_phone".into());
            }
            if env_flags
                .get("emulator_or_virt")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                adj_reasons.push("environment_emulator_or_virt".into());
                hits.push("environment_flags:emulator_virt".into());
            }
            if env_flags
                .get("soft_stack")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                hits.push("environment_flags:soft".into());
            }
        }
    }
    // Unknown only when both thin matrix AND no independent material families
    let status = if coverage < 0.10 && mat_n == 0 {
        "unknown"
    } else if adj_safety < fused_safety - 0.01 {
        // Env demotion: keep hedge status if already low, else re-label
        if adj_safety >= 0.55 {
            fused_status
        } else if adj_safety >= 0.35 {
            "suspect"
        } else {
            "high_risk"
        }
    } else {
        fused_status
    };

    let mut block = score_block(adj_safety, status, coverage, adj_reasons, hits);
    let completen = os_br_completeness(fields);
    // Completeness ceiling: incomplete materials cannot claim high OS safety
    let comp = completen
        .get("completeness")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let consistent = completen
        .get("consistent")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    if let Some(obj) = block.as_object_mut() {
        obj.insert("hedge_channels".into(), json!(channel_diag));
        obj.insert("material_family_count".into(), json!(mat_n));
        obj.insert("material_families".into(), json!(mat_fams));
        obj.insert("fusion_algo".into(), json!("gr_axis_hedge_v1"));
        obj.insert("environment_flags".into(), env_flags);
        obj.insert("os_br_completeness".into(), completen.clone());
        if let Some(sc) = obj.get("score").and_then(|v| v.as_f64()) {
            let mut sc2 = sc;
            if comp < 0.45 {
                sc2 = sc2.min(0.55);
            }
            if !consistent {
                sc2 = (sc2 - 0.08).max(0.0);
            }
            obj.insert("score".into(), json!(clamp01(sc2)));
        }
    }
    block
}

/// Browser authenticity safety.
pub fn score_br(
    fields: &Value,
    stack: &StackAuth,
    bot: &BotScore,
    truth: &TruthResult,
    source_conflicts: &[String],
) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let mut reasons = Vec::new();
    let mut hits = Vec::new();
    let coverage = load_field_product_matrix()
        .map(|m| m.axis_coverage("br", fields))
        .unwrap_or(0.35);

    let mut risk = 0.18_f64;
    // bot.verdict: crawler/bot can imply incomplete/rough kernel → keep strong on br.
    // automation|rpa is control-plane (rpa-primary) → weak side signal on br only.
    match bot.verdict.as_str() {
        "crawler" => {
            risk = 0.92;
            reasons.push("bot_verdict=crawler".into());
        }
        "bot" => {
            risk = risk.max(0.78);
            reasons.push("bot_verdict=bot".into());
        }
        "automation" | "rpa" => {
            risk = risk.max(0.38);
            reasons.push(format!("bot_verdict={}_weak_br", bot.verdict));
        }
        "human" => {
            risk = (risk - 0.08).max(0.0);
            reasons.push("bot_verdict=human".into());
        }
        other => reasons.push(format!("bot_verdict={other}")),
    }
    hits.push("bot".into());

    // Control-plane flags are **rpa-primary** (iss/25 axis ownership).
    // br keeps only weak conf — patchable static flags must not dominate kernel integrity.
    let wd = nested_bool(&fo, "automation", "webdriver")
        .or_else(|| bool_field(&fo, "webdriver"))
        .unwrap_or(false);
    if has_nonempty(&fo, "webdriver") || fo.get("automation").is_some() {
        hits.push("webdriver".into());
    }
    if wd {
        risk = (risk + 0.08).min(1.0);
        reasons.push("webdriver_weak_br".into());
    }
    // B0 `product_sub` / dense `nav_product_sub`: engine claim vs UA family (BotD).
    // Structural mismatch only — not a lab-tuned numeric threshold.
    {
        let ps = str_field(&fo, "product_sub")
            .or_else(|| str_field(&fo, "nav_product_sub"))
            .unwrap_or("");
        let ua = str_field(&fo, "user_agent")
            .or_else(|| str_field(&fo, "ua"))
            .unwrap_or("");
        if !ps.is_empty() {
            hits.push("product_sub".into());
            let chrome_like = ua.contains("Chrome/")
                || ua.contains("Chromium")
                || ua.contains("Edg/")
                || ua.contains("OPR/");
            let gecko = ua.contains("Firefox/");
            if chrome_like && !ua.contains("Firefox/") && ps != "20030107" {
                risk = (risk + 0.10).min(1.0);
                reasons.push("product_sub_not_chrome_engine".into());
            } else if gecko && !chrome_like && ps == "20030107" {
                risk = (risk + 0.10).min(1.0);
                reasons.push("product_sub_chrome_on_gecko_ua".into());
            }
        }
    }
    for key in ["playwright", "selenium", "cdc"] {
        if nested_bool(&fo, "automation", key).unwrap_or(false) {
            risk = (risk + 0.06).min(1.0);
            reasons.push(format!("automation.{key}_weak_br"));
            hits.push(format!("automation.{key}"));
        }
    }
    let spoof = f_field(&fo, "spoof_score").unwrap_or(stack.spoof_score);
    if spoof > 0.0 {
        hits.push("spoof_score".into());
    }
    if spoof >= 0.35 {
        risk = (risk + spoof * 0.55).min(1.0);
        reasons.push(format!("spoof_score={spoof:.2}"));
    }
    if stack.gpu_label_untrusted {
        risk = (risk + 0.18).min(1.0);
        reasons.push("gpu_label_untrusted".into());
        hits.push("webgl_unmasked_renderer".into());
    }
    // X-8: antidetect vendor / fingerprint lie → browser integrity only (not device digest).
    if bool_field(&fo, "antidetect_vendor_hint").unwrap_or(false)
        || bool_field(&fo, "fingerprint_vendor_lie").unwrap_or(false)
        || fo
            .get("antidetect_vendor_hint")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty() && s != "false")
    {
        hits.push("antidetect_vendor_hint".into());
        risk = (risk + 0.16).min(1.0);
        reasons.push("antidetect_browser_integrity".into());
    }
    if bool_field(&fo, "prototype_chain_tamper").unwrap_or(false) {
        hits.push("prototype_chain_tamper".into());
        risk = (risk + 0.14).min(1.0);
        reasons.push("prototype_chain_tamper_integrity".into());
    }
    // Claim-obs: declared GPU generation vs soft residual / missing capability — kernel integrity.
    {
        let ren = str_field(&fo, "webgl_unmasked_renderer")
            .or_else(|| str_field(&fo, "user_agent"))
            .unwrap_or("")
            .to_ascii_lowercase();
        let exotic_claim = ren.contains("geforce")
            || ren.contains("radeon")
            || ren.contains("apple m")
            || ren.contains("adreno")
            || ren.contains("mali")
            || ren.contains("metal");
        let soft_obs = stack.soft_stack
            || bool_field(&fo, "residual_soft_like").unwrap_or(false)
            || stack.residual_soft_like == Some(true)
            || matches!(
                stack.renderer_class.as_str(),
                "swiftshader" | "llvmpipe" | "softpipe" | "soft_other" | "virt_gpu"
            );
        let max_tex = f_field(&fo, "webgl_max_texture").or_else(|| f_field(&fo, "max_texture_size"));
        if exotic_claim {
            hits.push("webgl_unmasked_renderer".into());
        }
        if exotic_claim && soft_obs {
            risk = (risk + 0.20).min(1.0);
            reasons.push("gpu_label_vs_residual_soft_integrity".into());
            hits.push("residual_soft_like".into());
        }
        // High-end claim + low texture budget → capability incoherence (weighted verify).
        if exotic_claim {
            if let Some(mt) = max_tex {
                if mt > 0.0 && mt < 8192.0 {
                    risk = (risk + 0.14).min(1.0);
                    reasons.push("gpu_claim_vs_max_texture_incoherent".into());
                    reasons.push("claim_capability_incoherence".into());
                    hits.push("webgl_max_texture".into());
                }
            }
            // Claim discrete GPU but WebGL2 unavailable — fp-browser capability leak (demo/iss21).
            if bool_field(&fo, "webgl2_support") == Some(false) {
                risk = (risk + 0.16).min(1.0);
                reasons.push("claim_capability_incoherence_webgl2".into());
                reasons.push("claim_capability_incoherence".into());
                hits.push("webgl2_support".into());
            }
        }
        // Missing WebGL when UA/renderer claims desktop GPU — integrity hedge (low weight).
        if bool_field(&fo, "residual_available") == Some(false) && exotic_claim {
            risk = (risk + 0.10).min(1.0);
            reasons.push("gpu_claim_residual_unavailable".into());
            reasons.push("claim_capability_incoherence".into());
            hits.push("residual_available".into());
        }
        // Unit multiround + identical-draw: integrity conf for fingerprint noise / soft stacks.
        if bool_field(&fo, "unit_multiround_stable") == Some(false) {
            risk = (risk + 0.06).min(1.0);
            reasons.push("unit_multiround_unstable_integrity".into());
            hits.push("unit_multiround_stable".into());
        } else if bool_field(&fo, "unit_multiround_stable") == Some(true) {
            hits.push("unit_multiround_stable".into());
            risk = (risk - 0.02).max(0.0);
        }
        if let Some(uq) = f_field(&fo, "identical_draw_unique") {
            hits.push("identical_draw_unique".into());
            if uq > 2.0 {
                // High per-draw uniqueness under claimed hardware → anti-fp / noise injector.
                risk = (risk + 0.08).min(1.0);
                reasons.push("identical_draw_unique_high".into());
            }
        }
    }
    // outer_zero / headless window → rpa control-plane primary; weak br conf only
    if bool_field(&fo, "outer_zero").unwrap_or(false) {
        risk = (risk + 0.07).min(1.0);
        reasons.push("outer_zero_weak_br".into());
        hits.push("outer_zero".into());
    }
    if let Some(pl) = f_field(&fo, "plugins_length") {
        hits.push("plugins_length".into());
        if pl <= 0.0 && bot.verdict != "crawler" {
            risk = (risk + 0.06).min(1.0);
            reasons.push("plugins_empty".into());
        }
    }
    // iss/21 U-3: Chrome UA × missing chrome.runtime is brand-fragile (WebView/Arc/Brave shells).
    // Only risk when headless/automation context also present; otherwise conf hit only.
    if let Some(cr) = bool_field(&fo, "chrome_runtime") {
        hits.push("chrome_runtime".into());
        let ua = str_field(&fo, "user_agent").unwrap_or("");
        let claims_chrome = ua.contains("Chrome") || ua.contains("Chromium");
        let headlessish = ua.contains("HeadlessChrome")
            || ua.to_ascii_lowercase().contains("headless")
            || wd;
        let webviewish = ua.contains("; wv)") || ua.contains("Version/") && ua.contains("Mobile");
        if !cr && claims_chrome && !webviewish {
            if headlessish {
                risk = (risk + 0.10).min(1.0);
                reasons.push("chrome_ua_without_runtime_headless".into());
            } else {
                // conf-only: do not tank unknown Chromium shells
                reasons.push("chrome_ua_without_runtime_claim".into());
            }
        }
    }
    // CDP runtime → rpa primary; br weak conf only (not sole veto)
    if let Some(cdp) = f_field(&fo, "cdp_runtime_hint") {
        hits.push("cdp_runtime_hint".into());
        if cdp >= 1.0 {
            risk = (risk + (0.05_f64).min(cdp * 0.04)).min(1.0);
            reasons.push(format!("cdp_runtime_hint_weak_br={cdp:.0}"));
        }
    } else if bool_field(&fo, "cdp_runtime_hint").unwrap_or(false) {
        hits.push("cdp_runtime_hint".into());
        risk = (risk + 0.05).min(1.0);
        reasons.push("cdp_runtime_hint_weak_br".into());
    }
    // iss/21 T-BIO-2 joint band: sensitive form + zero pre-move also hits br
    if bool_field(&fo, "sensitive_action_zero_move").unwrap_or(false) {
        hits.push("sensitive_action_zero_move".into());
        risk = (risk + 0.14).min(1.0);
        reasons.push("sensitive_action_zero_move".into());
    }
    // iss/21 T-ENG-1: engine_claim vs engine_obs family mismatch
    if let (Some(claim), Some(obs)) = (
        str_field(&fo, "engine_claim"),
        str_field(&fo, "engine_obs"),
    ) {
        hits.push("engine_claim".into());
        hits.push("engine_obs".into());
        // Surface tag so br scores are not identical across engines when materials equal.
        hits.push(format!("engine_surface_{obs}"));
        reasons.push(format!("engine_surface={obs}"));
        if claim != "unknown"
            && obs != "unknown"
            && claim != obs
        {
            risk = (risk + 0.16).min(1.0);
            reasons.push(format!("engine_claim_obs_mismatch={claim}/{obs}"));
        } else if obs == "gecko" {
            // Gecko often lacks blink-only CH / plugins depth — mild authenticity caution.
            risk = (risk + 0.03).min(1.0);
            reasons.push("engine_gecko_surface_caution".into());
        } else if obs == "webkit" {
            risk = (risk + 0.02).min(1.0);
            reasons.push("engine_webkit_surface_caution".into());
        }
    } else if let Some(obs) = str_field(&fo, "engine_family").or_else(|| str_field(&fo, "engine_obs"))
    {
        hits.push(format!("engine_surface_{obs}"));
        reasons.push(format!("engine_surface={obs}"));
    }
    // iss/36 D-3 · atlas E-6: JA*/protocol_engine as **low-weight br aux** (not UV, not digest).
    // Record claim-obs reasons here; safety delta applied **after coverage_cap** so rich
    // materials cannot erase the small aux demotion (same pattern as sandbox post-cap).
    let mut ja_aux_safety_delta: f64 = 0.0;
    {
        use crate::protocol_edge::{brand_to_engine_family, derive_engine_claim_obs};
        let (claim_fam, _) = derive_engine_claim_obs(&fo);
        let proto = str_field(&fo, "protocol_engine").unwrap_or_default();
        let ja4 = str_field(&fo, "ja4")
            .or_else(|| str_field(&fo, "tls_ja4"))
            .unwrap_or_default();
        if !proto.is_empty() || !ja4.is_empty() {
            hits.push("ja_protocol_aux_br".into());
        }
        if !proto.is_empty() {
            hits.push("protocol_engine".into());
            let pf = brand_to_engine_family(&proto);
            if claim_fam != "unknown" && pf != "unknown" && pf != claim_fam {
                // Low weight: aux reference only (≪ hard residual path)
                ja_aux_safety_delta -= 0.06;
                reasons.push(format!(
                    "ja_protocol_vs_ua_claim_low_weight={pf}/{claim_fam}"
                ));
            } else if claim_fam != "unknown" && pf == claim_fam {
                ja_aux_safety_delta += 0.02;
                reasons.push("ja_protocol_confirms_claim_low_weight".into());
            }
        }
        if !ja4.is_empty() {
            hits.push("ja4".into());
            let jf = brand_to_engine_family(&ja4);
            if claim_fam != "unknown" && jf != "unknown" && jf != claim_fam {
                ja_aux_safety_delta -= 0.05;
                reasons.push(format!("ja4_vs_ua_claim_low_weight={jf}/{claim_fam}"));
            } else if source_conflicts
                .iter()
                .any(|c| c.contains("ja4_vs_ua") || c.contains("h2_vs_ua"))
            {
                ja_aux_safety_delta -= 0.04;
                reasons.push("ja4_xsrc_conflict_aux_br".into());
            }
        } else if source_conflicts
            .iter()
            .any(|c| c.contains("ja4_vs_ua") || c.contains("h2_vs_ua"))
        {
            hits.push("ja_protocol_aux_br".into());
            ja_aux_safety_delta -= 0.04;
            reasons.push("ja4_xsrc_conflict_aux_br".into());
        }
        // Clamp total aux influence (low weight by design)
        ja_aux_safety_delta = ja_aux_safety_delta.clamp(-0.12, 0.03);
    }
    // Anti-camouflage / languages consistency
    if has_nonempty(&fo, "languages") {
        hits.push("languages".into());
    }
    if has_nonempty(&fo, "language") {
        hits.push("language".into());
    }
    if has_nonempty(&fo, "platform") {
        hits.push("platform".into());
    }
    // Sandbox multi-source capability + B15/B7 identity mismatches
    apply_sandbox_capability_axis("br", &fo, &mut risk, &mut reasons, &mut hits);
    apply_cross_context_mismatch_axis("br", &fo, &mut risk, &mut reasons, &mut hits);
    if has_nonempty(&fo, "sandbox_kind")
        && !hits.iter().any(|h| h.starts_with("sandbox_capability"))
    {
        hits.push("sandbox_kind".into());
        risk = (risk - 0.02).max(0.0);
        reasons.push("sandbox_surface_present".into());
    }
    // Soft multi-source match ratio (class-level consistency across main/iframe/worker)
    if let Some(mr) = f_field(&fo, "multi_source_match_ratio") {
        hits.push("multi_source_match_ratio".into());
        if mr >= 0.85 {
            risk = (risk - 0.05).max(0.0);
            reasons.push(format!("multi_source_match={mr:.2}"));
        } else if mr < 0.5 {
            risk = (risk + 0.12).min(1.0);
            reasons.push(format!("multi_source_mismatch={mr:.2}"));
        }
    }
    // Identity multi-source conflicts (platform/cores/UA etc.) demote BR (policy weights).
    if xsrc_has_conflict(source_conflicts, truth) {
        let w = crate::policy::multi_source_conflict_weights();
        let id_conflicts = source_conflicts
            .iter()
            .filter(|c| {
                c.contains("platform")
                    || c.contains("hardware_concurrency")
                    || c.contains("user_agent")
                    || c.contains("os_family")
                    || c.contains("timezone")
                    || c.contains("webdriver")
            })
            .count();
        if id_conflicts > 0 {
            hits.push("identity_source_conflict_br".into());
            let n = id_conflicts.min(w.br_max_n) as f64;
            risk = (risk + w.br_base + w.br_per * n).min(1.0);
            reasons.push(format!("identity_source_conflict_n={id_conflicts}"));
        }
    }
    // Edge-authoritative UA / JA4 cross (Pingora gateway_user_agent vs FE)
    if bool_field(&fo, "gateway_fe_ua_mismatch").unwrap_or(false)
        || bool_field(&fo, "gateway_ua_vs_ja4_mismatch").unwrap_or(false)
    {
        hits.push("gateway_ua_edge_mismatch".into());
        risk = (risk + 0.12).min(1.0);
        if bool_field(&fo, "gateway_fe_ua_mismatch").unwrap_or(false) {
            reasons.push("gateway_fe_ua_mismatch".into());
        }
        if bool_field(&fo, "gateway_ua_vs_ja4_mismatch").unwrap_or(false) {
            reasons.push("gateway_ua_vs_ja4_mismatch".into());
        }
    }
    // Prefer gateway_user_agent presence as positive edge corroboration
    if has_nonempty(&fo, "gateway_user_agent") {
        hits.push("gateway_user_agent".into());
        if has_nonempty(&fo, "ja4") || has_nonempty(&fo, "protocol_engine") {
            hits.push("edge_protocol_fp".into());
            risk = (risk - 0.02).max(0.0);
            reasons.push("edge_ua_and_protocol_present".into());
        }
    }
    if has_nonempty(&fo, "http_header_order_hash") {
        hits.push("http_header_order_hash".into());
        risk = (risk - 0.01).max(0.0);
    }
    // Mint-gate single-source pressure when present on fields
    if let Some(sp) = f_field(&fo, "mint_single_source_pressure") {
        let w = crate::policy::multi_source_conflict_weights();
        if sp > w.single_source_pressure_lt {
            hits.push("mint_single_source_pressure".into());
            risk = (risk + w.single_source_scale * sp).min(1.0);
            reasons.push(format!("mint_single_source_pressure={sp:.2}"));
        }
    }
    // Mint-gate multi-source conflict pressure (harder-to-forge resolve still demotes scores)
    if let Some(cp) = f_field(&fo, "mint_conflict_pressure") {
        if cp > 0.0 {
            hits.push("mint_conflict_pressure".into());
            risk = (risk + 0.10 + 0.28 * cp).min(1.0);
            reasons.push(format!("mint_conflict_pressure={cp:.2}"));
        }
    }
    // Font surface first-difference (thin desktop fonts → fake shell signal)
    apply_font_surface_axis("br", &fo, &mut risk, &mut reasons, &mut hits);
    // Expanded materials: fonts/math/canvas soft conf
    if has_nonempty(&fo, "font_count") || has_nonempty(&fo, "fonts_present") {
        if !hits.iter().any(|h| h.starts_with("font_")) {
            hits.push("fonts".into());
            risk = (risk - 0.02).max(0.0);
        }
    }
    if has_nonempty(&fo, "math_digest") {
        hits.push("math_digest".into());
    }
    if has_nonempty(&fo, "webgl_extensions_hash") || bool_field(&fo, "webgl2_support").is_some() {
        hits.push("webgl_caps".into());
        risk = (risk - 0.02).max(0.0);
    }
    // Protocol fingerprints: br material only (never device digest).
    if has_nonempty(&fo, "ja4") {
        hits.push("ja4".into());
        risk = (risk - 0.03).max(0.0);
        reasons.push("ja4_protocol_present".into());
    }
    if has_nonempty(&fo, "ja3") || has_nonempty(&fo, "ja3_hash") {
        hits.push("ja3".into());
        if has_nonempty(&fo, "ja3_hash") {
            hits.push("ja3_hash".into());
        }
        risk = (risk - 0.02).max(0.0);
        reasons.push("ja3_protocol_present".into());
    }
    if has_nonempty(&fo, "native_function_toString") || has_nonempty(&fo, "errors_engine") {
        hits.push("native_integrity".into());
    }
    if has_nonempty(&fo, "pohw_triad") || bool_field(&fo, "challenge_alt_changed").unwrap_or(false) {
        hits.push("pohw_h16".into());
        risk = (risk - 0.03).max(0.0);
        reasons.push("pohw_challenge_materials".into());
    }
    // Multi-material hedge (B23/B24) — never single-field authenticity
    if has_nonempty(&fo, "native_integrity_ratio") || has_nonempty(&fo, "non_native_count") {
        hits.push("native_integrity_hedge".into());
        if let Some(r) = f_field(&fo, "native_integrity_ratio") {
            if r < 0.7 {
                risk = (risk + 0.12).min(1.0);
                reasons.push(format!("native_integrity_low={r:.2}"));
            } else {
                risk = (risk - 0.04).max(0.0);
                reasons.push("native_integrity_ok".into());
            }
        }
    }
    if has_nonempty(&fo, "canvas_geometry_hash") {
        hits.push("canvas_geometry_hedge".into());
        risk = (risk - 0.03).max(0.0);
    }
    if bool_field(&fo, "material_cross_conflict").unwrap_or(false) {
        hits.push("material_cross_conflict".into());
        risk = (risk + 0.28).min(1.0);
        reasons.push("material_cross_conflict".into());
    } else if has_nonempty(&fo, "material_vote_digest") {
        hits.push("material_vote".into());
        risk = (risk - 0.04).max(0.0);
        reasons.push("material_vote_digest".into());
    }
    if has_nonempty(&fo, "challenge_audio_rms") || has_nonempty(&fo, "challenge_cpu_wall_ms") {
        hits.push("pohw_multi_surface".into());
        risk = (risk - 0.02).max(0.0);
        reasons.push("pohw_audio_cpu_surfaces".into());
    }
    if has_nonempty(&fo, "font_bitmap_hash")
        || has_nonempty(&fo, "css_supports_hash")
        || has_nonempty(&fo, "media_query_hash")
        || has_nonempty(&fo, "css_props_hash")
    {
        hits.push("census_volume".into());
        risk = (risk - 0.03).max(0.0);
    }
    if has_nonempty(&fo, "object_census") {
        hits.push("object_census".into());
        risk = (risk - 0.02).max(0.0);
    }
    // Privacy / storage surface
    if let Some(ce) = bool_field(&fo, "cookie_enabled").or_else(|| bool_field(&fo, "cookieEnabled"))
    {
        hits.push("cookie_enabled".into());
        if !ce {
            risk = (risk + 0.04).min(1.0);
            reasons.push("cookies_disabled".into());
        }
    }
    if let Some(ls) = bool_field(&fo, "local_storage") {
        hits.push("local_storage".into());
        if !ls {
            risk = (risk + 0.04).min(1.0);
            reasons.push("local_storage_blocked".into());
        }
    }
    if has_nonempty(&fo, "authorized_surface") {
        hits.push("authorized_surface".into());
        risk = (risk - 0.03).max(0.0);
    }
    if has_nonempty(&fo, "mid_tag") {
        hits.push("mid_tag".into());
    }
    // Residual deep: css protocol + census + cross-context
    if has_nonempty(&fo, "css_color_gamut") || has_nonempty(&fo, "css_prefers_color_scheme") {
        hits.push("css_protocol".into());
        risk = (risk - 0.02).max(0.0);
    }
    if has_nonempty(&fo, "api_flags") || has_nonempty(&fo, "css_supports") {
        hits.push("census_lite".into());
        risk = (risk - 0.02).max(0.0);
    }
    if has_nonempty(&fo, "gl_precision_matrix") {
        hits.push("gl_precision_matrix".into());
        risk = (risk - 0.03).max(0.0);
    }
    if has_nonempty(&fo, "session_residual_cv") {
        hits.push("session_residual_cv".into());
        if let Some(cv) = f_field(&fo, "session_residual_cv") {
            if cv > 0.05 {
                risk = (risk + 0.1).min(1.0);
                reasons.push("residual_noise_injection_suspect".into());
            }
        }
    }

    // --- Blackhole close-up (FE emits these digests; analysis must read them) ---
    // Pingora edge: PROXY protocol real-client presence
    if bool_field(&fo, "proxy_protocol_present").unwrap_or(false)
        || has_nonempty(&fo, "proxy_protocol_version")
    {
        hits.push("proxy_protocol_edge".into());
        risk = (risk - 0.02).max(0.0);
        reasons.push("proxy_protocol_present".into());
    }
    // H2 fingerprint hash (gateway B8) — br conf material, not density-only
    if has_nonempty(&fo, "h2_fingerprint_hash") || has_nonempty(&fo, "h2_fingerprint") {
        hits.push("h2_fingerprint_hash".into());
        risk = (risk - 0.02).max(0.0);
    }
    // --- T2 br conf close-up (matrix contract) ---
    if bool_field(&fo, "has_challenge_seed").is_some() || has_nonempty(&fo, "has_challenge_seed") {
        hits.push("has_challenge_seed".into());
        if bool_field(&fo, "has_challenge_seed") == Some(true) {
            risk = (risk - 0.02).max(0.0);
        }
    }
    if has_nonempty(&fo, "intl_locale") {
        hits.push("intl_locale".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "media_device_count") || has_nonempty(&fo, "media_video_count") {
        hits.push("media_device_count".into());
        risk = (risk - 0.01).max(0.0);
        if has_nonempty(&fo, "media_video_count") {
            hits.push("media_video_count".into());
        }
    }
    if has_nonempty(&fo, "media_queries_lite") {
        hits.push("media_queries_lite".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "net_effective_type") {
        hits.push("net_effective_type".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "outer_width") {
        hits.push("outer_width".into());
        // zero outer is control-plane (rpa); here only presence as browser surface conf
        if f_field(&fo, "outer_width").unwrap_or(-1.0) == 0.0 {
            risk = (risk + 0.04).min(1.0);
            reasons.push("outer_width_zero_weak_br".into());
        } else {
            risk = (risk - 0.01).max(0.0);
        }
    }
    if has_nonempty(&fo, "vendor") {
        hits.push("vendor".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "webrtc_srflx_count") {
        hits.push("webrtc_srflx_count".into());
        risk = (risk - 0.01).max(0.0);
    }
    if bool_field(&fo, "worker_available").is_some() || has_nonempty(&fo, "worker_available") {
        hits.push("worker_available".into());
        if bool_field(&fo, "worker_available") == Some(false) {
            // missing workers on desktop can indicate rough/truncated kernel
            risk = (risk + 0.08).min(1.0);
            reasons.push("worker_unavailable".into());
        } else {
            risk = (risk - 0.02).max(0.0);
        }
    }
    if bool_field(&fo, "css_pointer_coarse").is_some() || has_nonempty(&fo, "css_pointer_coarse") {
        hits.push("css_pointer_coarse".into());
        risk = (risk - 0.01).max(0.0);
    }
    // demo forgeability L5 automation alias (d28)
    if bool_field(&fo, "headless_likely").unwrap_or(false) {
        hits.push("headless_likely".into());
        risk = (risk + 0.06).min(1.0); // weak br; rpa owns headless primary
        reasons.push("headless_likely_weak_br".into());
    }
    // Font OS-family surface (token hits, not just count)
    if has_nonempty(&fo, "font_token_hit_count") || has_nonempty(&fo, "font_present_sample") {
        hits.push("font_token_surface".into());
        risk = (risk - 0.02).max(0.0);
        if f_field(&fo, "font_token_hit_count").unwrap_or(0.0) >= 3.0 {
            hits.push("font_token_dense".into());
            risk = (risk - 0.01).max(0.0);
        }
    }
    // codec digest alias (B19 may emit hash without full matrix object)
    if has_nonempty(&fo, "codec_matrix_hash") {
        hits.push("codec_matrix_hash".into());
        risk = (risk - 0.02).max(0.0);
    }
    // Canvas geometry stability (B23 hedge depth)
    if has_nonempty(&fo, "canvas_geometry_stable") || has_nonempty(&fo, "canvas_geometry_mean") {
        hits.push("canvas_geometry_depth".into());
        if bool_field(&fo, "canvas_geometry_stable").unwrap_or(true) {
            risk = (risk - 0.02).max(0.0);
            reasons.push("canvas_geometry_stable".into());
        } else {
            risk = (risk + 0.06).min(1.0);
            reasons.push("canvas_geometry_unstable".into());
        }
    }
    // Agent parity ratio (hit/keys) — thin parity is weaker material
    if has_nonempty(&fo, "agent_parity_hit_n") || has_nonempty(&fo, "agent_parity_keys_n") {
        hits.push("agent_parity_ratio".into());
        let hit = f_field(&fo, "agent_parity_hit_n").unwrap_or(0.0);
        let keys = f_field(&fo, "agent_parity_keys_n").unwrap_or(0.0);
        if keys >= 8.0 {
            let ratio = hit / keys;
            if ratio < 0.5 {
                risk = (risk + 0.1).min(1.0);
                reasons.push("agent_parity_ratio_low".into());
            } else {
                risk = (risk - 0.02).max(0.0);
                reasons.push("agent_parity_ratio_ok".into());
            }
        }
    }
    // Permissions digest depth
    if has_nonempty(&fo, "permissions_hash") || has_nonempty(&fo, "permissions_granted_n") {
        hits.push("permissions_digest".into());
        risk = (risk - 0.02).max(0.0);
    }
    // Battery charging state (mobile conf, not spoof by itself)
    if bool_field(&fo, "battery_charging").is_some()
        || has_nonempty(&fo, "battery_charging_time")
        || has_nonempty(&fo, "battery_discharging_time")
    {
        hits.push("battery_charge_state".into());
        risk = (risk - 0.01).max(0.0);
    }
    // Media devices kinds / enumerate shape
    if has_nonempty(&fo, "media_devices_kinds") || has_nonempty(&fo, "media_devices_enumerate") {
        hits.push("media_devices_shape".into());
        risk = (risk - 0.02).max(0.0);
    }
    // CSS / MQ volume (counts beyond hash)
    if has_nonempty(&fo, "css_supports_ok_count") || has_nonempty(&fo, "css_supports_total") {
        hits.push("css_supports_volume".into());
        let ok = f_field(&fo, "css_supports_ok_count").unwrap_or(0.0);
        let total = f_field(&fo, "css_supports_total").unwrap_or(0.0);
        if total >= 10.0 && ok / total < 0.2 {
            risk = (risk + 0.05).min(1.0);
            reasons.push("css_supports_sparse".into());
        } else if ok >= 5.0 {
            risk = (risk - 0.01).max(0.0);
        }
    }
    if has_nonempty(&fo, "media_query_true_count") || has_nonempty(&fo, "media_query_total") {
        hits.push("media_query_volume".into());
        risk = (risk - 0.01).max(0.0);
    }
    // Residual means (hist already used; means enable challenge-vs-main delta)
    if has_nonempty(&fo, "main_residual_mean") || has_nonempty(&fo, "live_residual_mean") {
        hits.push("residual_mean_surface".into());
        risk = (risk - 0.01).max(0.0);
    }
    if let Some(d) = f_field(&fo, "residual_challenge_delta") {
        hits.push("residual_challenge_delta".into());
        if d.abs() > 0.05 {
            risk = (risk + 0.1).min(1.0);
            reasons.push("residual_challenge_delta_high".into());
        } else {
            risk = (risk - 0.02).max(0.0);
            reasons.push("residual_challenge_delta_ok".into());
        }
    }
    // PoHW multi-surface numeric (beyond rms/wall aliases)
    if has_nonempty(&fo, "challenge_audio_mean")
        || has_nonempty(&fo, "challenge_cpu_acc")
        || has_nonempty(&fo, "challenge_alt_mean")
        || has_nonempty(&fo, "challenge_wall_ms")
    {
        hits.push("pohw_numeric_surfaces".into());
        risk = (risk - 0.02).max(0.0);
    }
    // H02 roundtrip ladder alias
    if has_nonempty(&fo, "gpu_roundtrip_ladder") || has_nonempty(&fo, "gpu_intercept_wall") {
        hits.push("gpu_roundtrip_h02".into());
        risk = (risk - 0.02).max(0.0);
    }
    // H05 claim-vs-actual gap score from FE
    if let Some(g) = f_field(&fo, "caps_claim_vs_actual_gap") {
        hits.push("caps_claim_gap_score".into());
        if g > 0.5 {
            risk = (risk + 0.15).min(1.0);
            reasons.push("caps_claim_vs_actual_gap_high".into());
        } else if g > 0.0 {
            risk = (risk - 0.01).max(0.0);
        }
    }
    if has_nonempty(&fo, "caps_alloc_fail_at") || has_nonempty(&fo, "caps_failed_at") {
        hits.push("caps_alloc_fail_at".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "caps_rb_fail_at") || has_nonempty(&fo, "caps_rb_actual") {
        hits.push("caps_rb_pressure".into());
        risk = (risk - 0.01).max(0.0);
    }
    // rAF mean (clock_phys sibling)
    if has_nonempty(&fo, "raf_mean_ms") || has_nonempty(&fo, "raf_samples") {
        hits.push("raf_mean_surface".into());
        if let Some(m) = f_field(&fo, "raf_mean_ms") {
            // ~16.7ms expected at 60Hz; coarse virtualization often >> 40ms mean
            if m > 40.0 {
                risk = (risk + 0.06).min(1.0);
                reasons.push("raf_mean_coarse".into());
            } else if m > 0.0 {
                risk = (risk - 0.01).max(0.0);
            }
        }
    }
    // DomRect subpixel / layout spoof hedge
    if has_nonempty(&fo, "dom_rect") || bool_field(&fo, "dom_rect_subpixel").is_some() {
        hits.push("dom_rect_depth".into());
        risk = (risk - 0.01).max(0.0);
    }
    // Storage privacy siblings
    if bool_field(&fo, "storage_persisted").is_some()
        || bool_field(&fo, "caches_api").is_some()
        || has_nonempty(&fo, "storage_usage")
    {
        hits.push("storage_privacy_depth".into());
        risk = (risk - 0.01).max(0.0);
    }
    // H09 full GPU thermal slope sibling
    if has_nonempty(&fo, "thermal_gpu_slope_full") {
        hits.push("thermal_gpu_full".into());
        if f_field(&fo, "thermal_gpu_slope_full").unwrap_or(0.0) > 0.4 {
            risk = (risk + 0.06).min(1.0);
            reasons.push("thermal_gpu_slope_full_high".into());
        } else {
            risk = (risk - 0.01).max(0.0);
        }
    }
    // Engine error shape
    if bool_field(&fo, "errors_engine_chrome_like").is_some() {
        hits.push("errors_engine_shape".into());
        risk = (risk - 0.01).max(0.0);
    }
    // Speech language list / display MQ raw
    if has_nonempty(&fo, "speech_langs") || has_nonempty(&fo, "speech_voices_sample") {
        hits.push("speech_lang_surface".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "display_mq") {
        hits.push("display_mq_raw".into());
        risk = (risk - 0.01).max(0.0);
    }
    // WebGPU feature/limit counts
    if has_nonempty(&fo, "webgpu_features") || has_nonempty(&fo, "webgpu_limits_n") {
        hits.push("webgpu_feature_surface".into());
        risk = (risk - 0.01).max(0.0);
    }
    // WebRTC host IP list (network hedge beyond hash)
    if has_nonempty(&fo, "webrtc_host_ips") || has_nonempty(&fo, "webrtc_host_candidate") {
        hits.push("webrtc_host_list".into());
        risk = (risk - 0.01).max(0.0);
    }
    // GL caps depth (max_* siblings of label)
    if has_nonempty(&fo, "gl_max_vertex_attribs")
        || has_nonempty(&fo, "gl_max_texture_image_units")
        || has_nonempty(&fo, "gl_max_renderbuffer")
    {
        hits.push("gl_caps_depth".into());
        risk = (risk - 0.02).max(0.0);
    }
    // GPU-ns poll meta (honest async path)
    if has_nonempty(&fo, "gpu_ns_poll_frames") || has_nonempty(&fo, "gpu_query_meta") {
        hits.push("gpu_ns_poll_meta".into());
        risk = (risk - 0.01).max(0.0);
    }
    // Layer divergence count
    if has_nonempty(&fo, "layer_divergence_n") || has_nonempty(&fo, "layer_divergence_compared") {
        hits.push("layer_divergence_volume".into());
        risk = (risk - 0.01).max(0.0);
    }
    // Perf timeline depth
    if has_nonempty(&fo, "perf_entry_counts")
        || has_nonempty(&fo, "perf_nav_timing")
        || has_nonempty(&fo, "perf_ttfb_ms")
        || bool_field(&fo, "perf_longtask_observer").is_some()
    {
        hits.push("perf_timeline_depth".into());
        risk = (risk - 0.01).max(0.0);
    }
    // Sensor orientation / gyro siblings
    if bool_field(&fo, "sensor_gyro_present").unwrap_or(false)
        || bool_field(&fo, "sensor_orient_present").unwrap_or(false)
    {
        hits.push("sensor_orient_gyro".into());
        risk = (risk - 0.01).max(0.0);
    }

    // --- N4 selective blackhole: discriminative samples only ---
    if has_nonempty(&fo, "api_flags_ok_count") || has_nonempty(&fo, "api_flags_total") {
        hits.push("api_flags_volume".into());
        let ok = f_field(&fo, "api_flags_ok_count").unwrap_or(0.0);
        let total = f_field(&fo, "api_flags_total").unwrap_or(0.0);
        if total >= 20.0 {
            let ratio = ok / total;
            if ratio < 0.25 {
                risk = (risk + 0.08).min(1.0);
                reasons.push("api_flags_sparse".into());
            } else if ratio >= 0.6 {
                risk = (risk - 0.02).max(0.0);
                reasons.push("api_flags_dense".into());
            }
        }
    }
    if has_nonempty(&fo, "css_props_resolved_count") || has_nonempty(&fo, "css_props_total") {
        hits.push("css_props_volume".into());
        let ok = f_field(&fo, "css_props_resolved_count").unwrap_or(0.0);
        let total = f_field(&fo, "css_props_total").unwrap_or(0.0);
        if total >= 10.0 && ok / total < 0.2 {
            risk = (risk + 0.05).min(1.0);
            reasons.push("css_props_sparse".into());
        } else if ok >= 5.0 {
            risk = (risk - 0.01).max(0.0);
        }
    }
    if bool_field(&fo, "caps_internal_inconsistent").unwrap_or(false) {
        hits.push("caps_internal_inconsistent".into());
        risk = (risk + 0.12).min(1.0);
        reasons.push("caps_internal_inconsistent".into());
    }
    if has_nonempty(&fo, "caps_probe_results") {
        hits.push("caps_probe_results".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "cross_canvas_mean") || has_nonempty(&fo, "cross_audio_mean") {
        hits.push("cross_context_means".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "highp_mean") || has_nonempty(&fo, "mediump_mean") {
        hits.push("shader_precision_means".into());
        if let (Some(h), Some(m)) = (f_field(&fo, "highp_mean"), f_field(&fo, "mediump_mean")) {
            if h > 0.0 && m > 0.0 && (h - m).abs() / h.max(1e-9) > 0.5 {
                hits.push("mediump_highp_divergence".into());
                risk = (risk - 0.02).max(0.0); // real GPU often diverges mediump
                reasons.push("mediump_highp_divergence".into());
            }
        }
    }
    if has_nonempty(&fo, "canplay_mp4") || has_nonempty(&fo, "canplay_webm") {
        hits.push("canplay_scalars".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "automation_globals") {
        hits.push("automation_globals_list".into());
        if let Some(Value::Array(a)) = fo.get("automation_globals") {
            if !a.is_empty() {
                risk = (risk + 0.15).min(1.0);
                reasons.push("automation_globals_nonempty".into());
            }
        } else if has_nonempty(&fo, "automation_globals") {
            risk = (risk + 0.08).min(1.0);
        }
    }
    if has_nonempty(&fo, "js_heap_size_limit") {
        hits.push("js_heap_limit".into());
        // Extremely tiny heap limit can indicate constrained/soft envs
        if let Some(h) = f_field(&fo, "js_heap_size_limit") {
            if h > 0.0 && h < 64.0 * 1024.0 * 1024.0 {
                risk = (risk + 0.04).min(1.0);
                reasons.push("js_heap_limit_low".into());
            }
        }
    }
    if has_nonempty(&fo, "dns_probe_wall_ms") || has_nonempty(&fo, "connect_ms") {
        hits.push("net_timing_probes".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "device_pixel_ratio") {
        hits.push("device_pixel_ratio".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "quic_fp") || has_nonempty(&fo, "quic_fp_hash") {
        hits.push("quic_fp_header".into());
        risk = (risk - 0.01).max(0.0);
    }

    // --- Remaining blackhole batch (wire_now + selective wire_if_easy) ---
    if has_nonempty(&fo, "challenge_seed_digest") {
        hits.push("challenge_seed_digest".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "permission_notification")
        || has_nonempty(&fo, "permissions_prompt_n")
    {
        hits.push("permission_notification_surface".into());
        risk = (risk - 0.01).max(0.0);
        if f_field(&fo, "permissions_prompt_n").unwrap_or(0.0) >= 3.0 {
            risk = (risk - 0.01).max(0.0);
            reasons.push("permissions_prompt_rich".into());
        }
    }
    if has_nonempty(&fo, "canplay_av1") || has_nonempty(&fo, "canplay_hevc") {
        hits.push("canplay_modern_codecs".into());
        risk = (risk - 0.01).max(0.0);
        // empty string on both while matrix present is already covered by virt_hint
    }
    if has_nonempty(&fo, "codec_empty_n") || has_nonempty(&fo, "codec_maybe_n") {
        hits.push("codec_empty_maybe_counts".into());
        let empty = f_field(&fo, "codec_empty_n").unwrap_or(0.0);
        let total = f_field(&fo, "codec_matrix_n").unwrap_or(0.0);
        if total >= 8.0 && empty / total >= 0.85 {
            risk = (risk + 0.06).min(1.0);
            reasons.push("codec_mostly_empty".into());
        }
    }
    if bool_field(&fo, "eme_clearkey").unwrap_or(false) {
        hits.push("eme_clearkey".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "font_token_checked") {
        hits.push("font_token_checked".into());
        if f_field(&fo, "font_token_checked").unwrap_or(0.0) >= 8.0 {
            risk = (risk - 0.01).max(0.0);
        }
    }
    if has_nonempty(&fo, "gpu_ns_poll_ms") {
        hits.push("gpu_ns_poll_ms".into());
        // very short poll with many points can be soft-injected; long poll honest async
        if f_field(&fo, "gpu_ns_points").unwrap_or(0.0) >= 4.0
            && f_field(&fo, "gpu_ns_poll_ms").unwrap_or(0.0) >= 50.0
        {
            risk = (risk - 0.01).max(0.0);
            reasons.push("gpu_ns_poll_async_honest".into());
        }
    }
    if has_nonempty(&fo, "material_cross_reasons") || has_nonempty(&fo, "material_consistency") {
        hits.push("material_cross_meta".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "material_keys") {
        hits.push("material_keys".into());
        if let Some(Value::Array(a)) = fo.get("material_keys") {
            if a.len() >= 3 {
                risk = (risk - 0.02).max(0.0);
                reasons.push("material_keys_multi".into());
            }
        }
    }
    if has_nonempty(&fo, "media_prefers_color_scheme")
        || has_nonempty(&fo, "media_prefers_reduced_motion")
    {
        hits.push("media_prefers".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "orientation_angle") || has_nonempty(&fo, "orientation_type") {
        hits.push("orientation_surface".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "perf_now_delta_n") {
        hits.push("perf_now_delta_n".into());
        // resolution staircase sample count
        if f_field(&fo, "perf_now_delta_n").unwrap_or(0.0) >= 8.0 {
            risk = (risk - 0.01).max(0.0);
        }
    }
    if has_nonempty(&fo, "sandbox_residual_mean") {
        hits.push("sandbox_residual_mean".into());
        risk = (risk - 0.01).max(0.0);
        if let (Some(m), Some(s)) = (
            f_field(&fo, "main_residual_mean"),
            f_field(&fo, "sandbox_residual_mean"),
        ) {
            if (m - s).abs() > 0.02 {
                risk = (risk + 0.08).min(1.0);
                reasons.push("sandbox_main_residual_mismatch".into());
            }
        }
    }
    if has_nonempty(&fo, "screen_pixel_depth") || has_nonempty(&fo, "screen_color_depth") {
        hits.push("screen_depth".into());
        risk = (risk - 0.01).max(0.0);
        if f_field(&fo, "screen_color_depth").unwrap_or(24.0) < 16.0 {
            risk = (risk + 0.04).min(1.0);
            reasons.push("screen_color_depth_low".into());
        }
    }
    if has_nonempty(&fo, "session_residual_repeat") {
        hits.push("session_residual_repeat".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "thermal_cpu_cv") {
        hits.push("thermal_cpu_cv".into());
        if f_field(&fo, "thermal_cpu_cv").unwrap_or(0.0) > 0.5 {
            risk = (risk + 0.04).min(1.0);
            reasons.push("thermal_cpu_cv_high".into());
        }
    }
    if has_nonempty(&fo, "cookie_count") {
        hits.push("cookie_count".into());
        risk = (risk - 0.01).max(0.0);
    }
    // wire_if_easy selective
    if has_nonempty(&fo, "connection") || has_nonempty(&fo, "net_type") {
        hits.push("network_connection".into());
        risk = (risk - 0.01).max(0.0);
    }
    if bool_field(&fo, "device_motion").unwrap_or(false) {
        hits.push("device_motion".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "gl_bandwidth_lite")
        || has_nonempty(&fo, "gl_max_cube_map")
        || has_nonempty(&fo, "gl_max_varying_vectors")
    {
        hits.push("gl_caps_extra".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "highp_precision") {
        hits.push("highp_precision".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "native_checked") {
        hits.push("native_checked".into());
        if f_field(&fo, "native_checked").unwrap_or(0.0) >= 10.0 {
            risk = (risk - 0.01).max(0.0);
        }
    }
    if has_nonempty(&fo, "neg_dict_hash") {
        hits.push("neg_dict_hash".into());
        risk = (risk - 0.01).max(0.0);
    }
    if bool_field(&fo, "offscreen_available").is_some() {
        hits.push("offscreen_available".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "perf_dom_content_loaded_ms")
        || has_nonempty(&fo, "perf_load_event_ms")
        || has_nonempty(&fo, "perf_time_origin")
    {
        hits.push("perf_nav_depth".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "roundtrip_slope") {
        hits.push("roundtrip_slope".into());
        risk = (risk - 0.01).max(0.0);
    }
    if bool_field(&fo, "storage_manager").is_some()
        || has_nonempty(&fo, "total_js_heap_size")
        || has_nonempty(&fo, "used_js_heap_size")
    {
        hits.push("heap_storage_runtime".into());
        risk = (risk - 0.01).max(0.0);
        if let (Some(used), Some(total)) = (
            f_field(&fo, "used_js_heap_size"),
            f_field(&fo, "total_js_heap_size"),
        ) {
            if total > 0.0 && used / total > 0.95 {
                risk = (risk + 0.03).min(1.0);
                reasons.push("js_heap_pressure".into());
            }
        }
    }
    if has_nonempty(&fo, "ws_protocol")
        || has_nonempty(&fo, "ws_extensions")
        || has_nonempty(&fo, "ws_binary_type")
        || has_nonempty(&fo, "ws_ready_state")
        || has_nonempty(&fo, "ws_url_host")
    {
        hits.push("websocket_depth".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "challenge_audio_freq") || has_nonempty(&fo, "challenge_repeat_means") {
        hits.push("challenge_audio_repeat".into());
        risk = (risk - 0.01).max(0.0);
    }
    // demo/iss2 L3 challenge residual (high forge cost) — conf when FE emits digests
    if has_nonempty(&fo, "challenge_hist_digest")
        || has_nonempty(&fo, "challenge_hist")
        || has_nonempty(&fo, "challenge_lowbit16_digest")
    {
        hits.push("challenge_hist_digest".into());
        risk = (risk - 0.03).max(0.0);
        reasons.push("challenge_residual_hist".into());
        if has_nonempty(&fo, "challenge_lowbit16_digest") {
            hits.push("challenge_lowbit16_digest".into());
        }
        if has_nonempty(&fo, "challenge_hist") {
            hits.push("challenge_hist".into());
        }
    }
    if has_nonempty(&fo, "hw_webgl_peak_sig") {
        hits.push("hw_webgl_peak_sig".into());
        // peaks are engine-sensitive conf only (not commercial)
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "layer_divergence_sample") {
        hits.push("layer_divergence_sample".into());
        risk = (risk - 0.01).max(0.0);
    }
    if has_nonempty(&fo, "quic_aead_ok") {
        hits.push("quic_aead".into());
        if bool_field(&fo, "quic_aead_ok").unwrap_or(false) {
            risk = (risk - 0.03).max(0.0);
            reasons.push("quic_aead_clienthello".into());
        }
    }
    // P-V3 TCP_INFO / SAVED_SYN depth (+ client_tcp_rtt alias for xsrc)
    if bool_field(&fo, "tcp_info_available").unwrap_or(false)
        || has_nonempty(&fo, "tcp_info_rtt_us")
        || has_nonempty(&fo, "client_tcp_rtt_us")
        || has_nonempty(&fo, "client_tcp_rtt")
        || bool_field(&fo, "tcp_saved_syn").unwrap_or(false)
    {
        hits.push("tcp_depth".into());
        risk = (risk - 0.02).max(0.0);
        if bool_field(&fo, "tcp_saved_syn").unwrap_or(false) {
            hits.push("tcp_saved_syn".into());
            risk = (risk - 0.01).max(0.0);
        }
        if has_nonempty(&fo, "tcp_info_rtt_us")
            || has_nonempty(&fo, "client_tcp_rtt_us")
            || has_nonempty(&fo, "client_tcp_rtt")
        {
            hits.push("client_tcp_rtt".into());
        }
        if let Some(r) = f_field(&fo, "tcp_info_total_retrans") {
            if r >= 5.0 {
                risk = (risk + 0.04).min(1.0);
                reasons.push("tcp_retrans_high".into());
            }
        }
    }

    // Full HTTP/3 application layer (Quinn+h3 server)
    if bool_field(&fo, "h3_app_present").unwrap_or(false)
        || has_nonempty(&fo, "h3_settings_fp")
        || has_nonempty(&fo, "h3_pseudo_order")
    {
        hits.push("h3_app_layer".into());
        risk = (risk - 0.03).max(0.0);
        reasons.push("h3_app_present".into());
        if has_nonempty(&fo, "h3_pseudo_order") {
            // iss/46 H2: pseudo order may be padded — diagnostic only
            hits.push("h3_pseudo_order_diagnostic".into());
            reasons.push("h3_pseudo_order_diagnostic_only".into());
        }
        if let Some(rtt) = f_field(&fo, "h3_rtt_ms") {
            hits.push("h3_rtt".into());
            if rtt > 0.0 && rtt < 500.0 {
                risk = (risk - 0.01).max(0.0);
            }
        }
        // H3 app without TCP JA4 is still strong protocol material
        if fo.get("ja4").is_none() && fo.get("quic_tls_ja4").is_none() {
            risk = (risk - 0.01).max(0.0);
            reasons.push("h3_app_without_tcp_ja4".into());
        }
    }

    // iss/50 H4: B34 cache ladder is L2–L3 timing ladder, not L3 silicon mint material.
    if has_nonempty(&fo, "cpu_cache_ladder") || has_nonempty(&fo, "cpu_cache_knee_bytes") {
        hits.push("cpu_cache_ladder_diagnostic".into());
        reasons.push("b34_cache_ladder_not_commercial_mint".into());
        // mild conf assist only — never treated as mint key
        risk = (risk - 0.005).max(0.0);
    }

    // iss/46 H3: governed WebGL renderer must not pollute native unmasked semantics
    if has_nonempty(&fo, "governed_webgl_renderer")
        || bool_field(&fo, "webgl_unmasked_is_governed").unwrap_or(false)
        || bool_field(&fo, "gl_governor_active").unwrap_or(false)
    {
        hits.push("gl_governor_isolated".into());
        reasons.push("webgl_governed_vs_unmasked_isolated".into());
    }

    if truth.has_main_core && !wd && bot.verdict != "crawler" && bot.verdict != "bot" {
        risk = (risk - 0.1).max(0.0);
        reasons.push("main_core_ok".into());
    }
    if !truth.has_server_side {
        risk = (risk + 0.04).min(1.0);
        reasons.push("no_server_side_cap".into());
    }
    // Fake CF: untrusted cloudflare without verified fields
    if fo.get("cf_bot_score").is_some() && !truth.has_server_side {
        risk = (risk + 0.1).min(1.0);
        reasons.push("unverified_cf_fields".into());
    }
    crate::rule_sample_loader::apply_rule_samples_axis(
        fields, "br", &mut risk, &mut reasons, &mut hits,
    );
    crate::ja_population_prior::apply_ja_population_prior_br(
        &fo, &mut risk, &mut reasons, &mut hits,
    );
    // Close silent-unused FE materials into BR axis.
    apply_probe_surface_evidence_br(&fo, &mut risk, &mut reasons, &mut hits);

    apply_matrix_role_evidence("br", &fo, &mut risk, &mut reasons, &mut hits);
    apply_dense_digest_evidence("br", &fo, &mut risk, &mut reasons, &mut hits);
    apply_random_verify_authenticity(&fo, &mut risk, &mut reasons, &mut hits);
    apply_cross_axis_corroboration("br", &fo, &mut risk, &mut reasons, &mut hits);

    // Material-family / hit breadth before coverage cap (matrix growth must not alone crush safety).
    // iss/50 H5: cap pure field-presence padding on br axis.
    let (mat_n, mat_fams) = count_material_families("br", &fo);
    let mut hit_cov = (hits.len() as f64 / 20.0).clamp(0.0, 0.80);
    let presence_cap = if mat_n < 2 { 0.35 } else if mat_n < 4 { 0.55 } else { 0.80 };
    if hit_cov > presence_cap {
        reasons.push(format!("br_presence_pad_cap={presence_cap:.2}"));
        hit_cov = presence_cap;
    }
    let eff_cov = coverage
        .max((mat_n as f64 / 7.0) * 0.70)
        .max(hit_cov)
        .min(1.0);
    let mut safety = 1.0 - risk;
    let cov_cap = 0.40 + 0.60 * eff_cov;
    if safety > cov_cap {
        reasons.push(format!("coverage_cap={cov_cap:.2}"));
        safety = cov_cap;
    }
    // JA aux (low weight): apply after coverage_cap so rich surfaces still show family mismatch.
    if ja_aux_safety_delta.abs() > 1e-9 {
        safety = (safety + ja_aux_safety_delta).clamp(0.05, 1.0);
        reasons.push(format!("ja_aux_post_cap_delta={ja_aux_safety_delta:.2}"));
    }
    // Sandbox capability demotion applies after coverage cap so it is not erased by thin-cap.
    if bool_field(&fo, "js_ok_sandbox_dead").unwrap_or(false)
        || bool_field(&fo, "sandbox_blocked").unwrap_or(false)
        || bool_field(&fo, "sandbox_all_empty").unwrap_or(false)
        || f_field(&fo, "sandbox_capability_score").is_some_and(|c| c <= 0.05)
    {
        safety = (safety - 0.18).max(0.05);
        if !reasons.iter().any(|r| r.contains("sandbox")) {
            reasons.push("js_ok_but_sandbox_fully_blocked".into());
        }
    } else if bool_field(&fo, "sandbox_thin_vs_main").unwrap_or(false)
        || bool_field(&fo, "sandbox_under_two_kinds").unwrap_or(false)
        || f_field(&fo, "sandbox_capability_score").is_some_and(|c| c > 0.05 && c < 0.55)
    {
        safety = (safety - 0.10).max(0.05);
        if !reasons.iter().any(|r| r.contains("sandbox_capability_thin")) {
            reasons.push("sandbox_capability_thin_post_cap".into());
        }
    }
    if coverage < 0.15 && mat_n < 2 {
        safety = safety.min(0.5);
        reasons.push("br_coverage_thin".into());
    }

    if reasons.iter().any(|r| r.contains("cross_platform_vs_ua_ch")) {
        safety = (safety - 0.12).max(0.05);
        reasons.push("br_platform_ua_ch_post_cap".into());
    }
    // Agent control-plane after cap: matrix conf/hits must not erase automation demotion.
    if bool_field(&fo, "agent_has_webdriver").unwrap_or(false)
        || bool_field(&fo, "agent_has_selenium").unwrap_or(false)
        || bool_field(&fo, "agent_has_playwright").unwrap_or(false)
        || bool_field(&fo, "agent_has_cdc").unwrap_or(false)
        || bool_field(&fo, "webdriver").unwrap_or(false)
    {
        safety = (safety - 0.18).max(0.05);
        if !reasons.iter().any(|r| r.contains("agent_has") || r.contains("webdriver")) {
            reasons.push("br_agent_post_cap".into());
        }
    }

    if reasons.iter().any(|r| r.contains("cross_context") || r.contains("iframe_ua") || r.contains("sandbox_tostring") || r.contains("multi_source_match")) {
        safety = (safety - 0.08).max(0.05);
    }
    if reasons.iter().any(|r| r.contains("noise_suspect") || r.contains("challenge_noise") || r.contains("challenge_avg_cv")) {
        safety = (safety - 0.08).max(0.05);
        reasons.push("br_challenge_noise_post_cap".into());
    }
    if reasons.is_empty() {
        reasons.push("br_field_fusion".into());
    }

    // Multi-channel hedge for browser authenticity
    let agent_veto = [
        "agent_has_webdriver",
        "agent_has_domautomation",
        "agent_has_cdc",
        "agent_has_playwright",
        "agent_has_selenium",
        "agent_has_nightmare",
        "agent_has_phantom",
        "agent_has_callphantom",
    ]
    .iter()
    .any(|k| bool_field(&fo, k).unwrap_or(false));
    let env_contradict = safety < 0.35
        || wd
        || agent_veto
        || bot.verdict == "bot"
        || bot.verdict == "crawler"
        || (bool_field(&fo, "sandbox_blocked").unwrap_or(false)
            && bool_field(&fo, "js_ok_sandbox_dead").unwrap_or(true));
    let (fused_safety, fused_status, fused_reasons, channel_diag) = fuse_axis_hedge(
        "br",
        &fo,
        safety,
        eff_cov.max(coverage),
        reasons,
        env_contradict,
        source_conflicts,
        xsrc_has_conflict(source_conflicts, truth),
    );
    for f in &mat_fams {
        hits.push(format!("mat:{f}"));
    }
    let status = if coverage < 0.08 && mat_n == 0 {
        "unknown"
    } else {
        fused_status
    };
    let mut block = score_block(fused_safety, status, coverage, fused_reasons, hits);
    let completen = os_br_completeness(fields);
    let comp = completen
        .get("completeness")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let consistent = completen
        .get("consistent")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    if let Some(obj) = block.as_object_mut() {
        obj.insert("hedge_channels".into(), json!(channel_diag));
        obj.insert("material_family_count".into(), json!(mat_n));
        obj.insert("material_families".into(), json!(mat_fams));
        obj.insert("fusion_algo".into(), json!("gr_axis_hedge_v1"));
        obj.insert("os_br_completeness".into(), completen.clone());
        if let Some(sc) = obj.get("score").and_then(|v| v.as_f64()) {
            let mut sc2 = sc;
            if comp < 0.45 {
                sc2 = sc2.min(0.55);
            }
            if !consistent {
                sc2 = (sc2 - 0.1).max(0.0);
            }
            obj.insert("score".into(), json!(clamp01(sc2)));
        }
    }
    block
}

/// Page-level RPA safety.
/// iss/21 T-BIO-1: apply FE-emitted kinematics summaries (and/or derive from events).
fn apply_rpa_kinematics_fields_only(
    fo: &Map<String, Value>,
    risk: &mut f64,
    reasons: &mut Vec<String>,
    hits: &mut Vec<String>,
) {
    if let Some(ent) = f_field(fo, "input_mouse_entropy") {
        hits.push("input_mouse_entropy".into());
        if ent >= 0.35 {
            *risk = (*risk - 0.12).max(0.0);
            reasons.push(format!("input_mouse_entropy={ent:.2}"));
        } else if ent < 0.12 {
            *risk = (*risk + 0.14).min(1.0);
            reasons.push(format!("input_mouse_entropy_low={ent:.2}"));
        }
    }
    if let Some(icr) = f_field(fo, "integer_coord_ratio") {
        hits.push("integer_coord_ratio".into());
        // iss/50 D2: after FE fix this is subpixel-near-int ratio, not 4px-grid collapse.
        // Only extreme pure-int streams pad automation; mid band is neutral.
        if icr >= 0.995 {
            *risk = (*risk + 0.08).min(1.0);
            reasons.push(format!("integer_coord_ratio_pure={icr:.3}"));
        } else if icr <= 0.75 {
            *risk = (*risk - 0.06).max(0.0);
            reasons.push(format!("integer_coord_ratio_humanish={icr:.2}"));
        } else {
            reasons.push(format!("integer_coord_ratio_neutral={icr:.2}"));
        }
    }
    if let Some(dwell) = f_field(fo, "key_dwell_stddev") {
        hits.push("key_dwell_stddev".into());
        if dwell >= 12.0 {
            *risk = (*risk - 0.08).max(0.0);
            reasons.push(format!("key_dwell_stddev={dwell:.1}"));
        } else if dwell > 0.0 && dwell < 3.0 {
            *risk = (*risk + 0.10).min(1.0);
            reasons.push(format!("key_dwell_too_uniform={dwell:.1}"));
        }
    }
    if let Some(pre) = f_field(fo, "pre_action_move_count") {
        hits.push("pre_action_move_count".into());
        if pre <= 0.0 {
            *risk = (*risk + 0.16).min(1.0);
            reasons.push("pre_action_move_zero".into());
        } else if pre >= 3.0 {
            *risk = (*risk - 0.06).max(0.0);
            reasons.push(format!("pre_action_move_count={pre:.0}"));
        }
    }
    // T-BIO-1 finish: ttfi / event_order / scroll / velocity / path
    if let Some(ttfi) = f_field(fo, "ttfi_ms") {
        hits.push("ttfi_ms".into());
        if ttfi < 30.0 {
            *risk = (*risk + 0.10).min(1.0);
            reasons.push(format!("ttfi_too_fast={ttfi:.0}"));
        } else if ttfi >= 120.0 && ttfi < 120_000.0 {
            *risk = (*risk - 0.04).max(0.0);
            reasons.push(format!("ttfi_ms={ttfi:.0}"));
        }
    }
    if let Some(eo) = f_field(fo, "event_order_score") {
        hits.push("event_order_score".into());
        if eo >= 0.75 {
            *risk = (*risk - 0.05).max(0.0);
            reasons.push(format!("event_order_score={eo:.2}"));
        } else if eo < 0.35 {
            *risk = (*risk + 0.08).min(1.0);
            reasons.push(format!("event_order_low={eo:.2}"));
        }
    }
    if let Some(sb) = f_field(fo, "scroll_burst_n") {
        hits.push("scroll_burst_n".into());
        if sb >= 1.0 {
            *risk = (*risk - 0.03).max(0.0);
            reasons.push(format!("scroll_burst_n={sb:.0}"));
        }
    }
    if let Some(vcv) = f_field(fo, "input_velocity_cv") {
        hits.push("input_velocity_cv".into());
        // Humans: moderate CV; scripted constant velocity → very low CV
        if vcv < 0.08 {
            *risk = (*risk + 0.12).min(1.0);
            reasons.push(format!("velocity_too_uniform={vcv:.2}"));
        } else if vcv >= 0.25 && vcv < 3.0 {
            *risk = (*risk - 0.05).max(0.0);
            reasons.push(format!("input_velocity_cv={vcv:.2}"));
        }
    }
    if let Some(pl) = f_field(fo, "path_length_px") {
        hits.push("path_length_px".into());
        if pl >= 80.0 {
            *risk = (*risk - 0.03).max(0.0);
            reasons.push(format!("path_length_px={pl:.0}"));
        } else if pl > 0.0 && pl < 8.0 {
            *risk = (*risk + 0.06).min(1.0);
            reasons.push("path_length_tiny".into());
        }
    }
    // iss/39 R12: path straightness + click interval uniformity (weak, never sole veto)
    if let Some(ps) = f_field(fo, "path_straightness") {
        hits.push("path_straightness".into());
        if ps >= 0.97 {
            *risk = (*risk + 0.08).min(1.0);
            reasons.push(format!("path_too_straight={ps:.2}"));
        } else if ps > 0.0 && ps <= 0.85 {
            *risk = (*risk - 0.03).max(0.0);
            reasons.push(format!("path_straightness={ps:.2}"));
        }
    }
    if let Some(cicv) = f_field(fo, "click_interval_cv") {
        hits.push("click_interval_cv".into());
        if cicv > 0.0 && cicv < 0.06 {
            *risk = (*risk + 0.09).min(1.0);
            reasons.push(format!("click_interval_too_uniform={cicv:.2}"));
        } else if cicv >= 0.2 && cicv < 4.0 {
            *risk = (*risk - 0.03).max(0.0);
            reasons.push(format!("click_interval_cv={cicv:.2}"));
        }
    }
    // T-BIO-2: sensitive form with zero pre-move
    if bool_field(fo, "sensitive_action_zero_move").unwrap_or(false) {
        hits.push("sensitive_action_zero_move".into());
        *risk = (*risk + 0.22).min(1.0);
        reasons.push("sensitive_action_zero_move".into());
    }
    // B11 `rpa_coalesced_stats` — corroboration only (never sole deny).
    if let Some(cs) = fo
        .get("rpa_coalesced_stats")
        .and_then(|v| v.as_object())
        .or_else(|| {
            fo.get("rpa_features_v2")
                .and_then(|v| v.get("rpa_coalesced_stats"))
                .and_then(|v| v.as_object())
        })
    {
        hits.push("rpa_coalesced_stats".into());
        let no_coal = cs
            .get("no_coalesced_observed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let pointer_n = f_field(fo, "pointer_n").unwrap_or(0.0);
        if no_coal && pointer_n >= 6.0 {
            *risk = (*risk + 0.05).min(1.0);
            reasons.push("rpa_no_coalesced_on_pointer_stream".into());
        } else if !no_coal {
            *risk = (*risk - 0.02).max(0.0);
            reasons.push("rpa_coalesced_present".into());
        }
    }
}

fn apply_rpa_kinematics(
    fo: &Map<String, Value>,
    arr: &[Value],
    risk: &mut f64,
    reasons: &mut Vec<String>,
    hits: &mut Vec<String>,
) {
    // Prefer FE summaries when present (still derive missing keys from stream)
    apply_rpa_kinematics_fields_only(fo, risk, reasons, hits);
    let has_full = f_field(fo, "ttfi_ms").is_some()
        && f_field(fo, "event_order_score").is_some()
        && f_field(fo, "input_velocity_cv").is_some();
    if has_full {
        return;
    }
    // Derive remaining kinematics from raw events
    let mut coords = 0u32;
    let mut int_coords = 0u32;
    let mut cells = std::collections::HashSet::new();
    let mut pre_moves = 0u32;
    let mut saw_action = false;
    let mut path_len = 0.0_f64;
    let mut prev: Option<(f64, f64, f64)> = None;
    let mut speeds: Vec<f64> = Vec::new();
    let mut scroll_burst = 0u32;
    let mut scroll_window = 0u32;
    let mut scroll_start = 0.0_f64;
    let mut order_hits = 0u32;
    let mut order_total = 0u32;
    let mut first_t: Option<f64> = None;
    for e in arr {
        let k = e
            .get("kind")
            .or_else(|| e.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let t = e.get("t").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let is_move = matches!(k, "mousemove" | "pointermove" | "touchmove" | "wheel");
        let is_action = matches!(
            k,
            "click" | "pointerdown" | "pointerup" | "keydown" | "touchstart"
        );
        if first_t.is_none() && (is_move || is_action || k == "scroll") {
            first_t = Some(t);
        }
        if !saw_action && is_move {
            pre_moves += 1;
        }
        if is_action {
            order_total += 1;
            if pre_moves > 0 || saw_action {
                order_hits += 1;
            }
            saw_action = true;
        }
        if k == "scroll" || k == "wheel" {
            if scroll_start <= 0.0 || t - scroll_start > 400.0 {
                scroll_start = t;
                scroll_window = 1;
            } else {
                scroll_window += 1;
                if scroll_window >= 4 {
                    scroll_burst += 1;
                }
            }
        }
        // Prefer full-precision _xf/_yf when present (FE local kinematics); never trust 4px grid alone
        if let (Some(x), Some(y)) = (
            e.get("_xf")
                .and_then(|v| v.as_f64())
                .or_else(|| e.get("x").and_then(|v| v.as_f64())),
            e.get("_yf")
                .and_then(|v| v.as_f64())
                .or_else(|| e.get("y").and_then(|v| v.as_f64())),
        ) {
            if is_move || matches!(k, "click" | "pointerdown") {
                coords += 1;
                // Near-integer at ~1px — aligns with FE iss/50 D2
                if (x - x.round()).abs() < 0.02 && (y - y.round()).abs() < 0.02 {
                    int_coords += 1;
                }
                cells.insert(((x / 8.0).floor() as i64, (y / 8.0).floor() as i64));
                if let Some((px, py, pt)) = prev {
                    let dist = ((x - px).powi(2) + (y - py).powi(2)).sqrt();
                    path_len += dist;
                    if t > pt {
                        speeds.push(dist / (t - pt).max(1.0));
                    }
                }
                prev = Some((x, y, t));
            }
        }
    }
    if f_field(fo, "pre_action_move_count").is_none() && saw_action {
        hits.push("pre_action_move_count".into());
        if pre_moves == 0 {
            *risk = (*risk + 0.16).min(1.0);
            reasons.push("pre_action_move_zero".into());
        } else if pre_moves >= 3 {
            *risk = (*risk - 0.06).max(0.0);
            reasons.push(format!("pre_action_move_count={pre_moves}"));
        }
    }
    if f_field(fo, "input_mouse_entropy").is_none() && coords >= 4 {
        let ent = (cells.len() as f64 / coords as f64).min(1.0);
        hits.push("input_mouse_entropy".into());
        if ent >= 0.35 {
            *risk = (*risk - 0.12).max(0.0);
            reasons.push(format!("input_mouse_entropy={ent:.2}"));
        } else if ent < 0.12 {
            *risk = (*risk + 0.14).min(1.0);
            reasons.push(format!("input_mouse_entropy_low={ent:.2}"));
        }
    }
    if f_field(fo, "integer_coord_ratio").is_none() && coords >= 4 {
        let icr = int_coords as f64 / coords as f64;
        hits.push("integer_coord_ratio".into());
        // Align with FE full-precision near-int (iss/50 D2) — no perpetual +0.12 on humans
        if icr >= 0.995 {
            *risk = (*risk + 0.08).min(1.0);
            reasons.push(format!("integer_coord_ratio_pure={icr:.3}"));
        } else if icr <= 0.75 {
            *risk = (*risk - 0.06).max(0.0);
            reasons.push(format!("integer_coord_ratio_humanish={icr:.2}"));
        }
    }
    if f_field(fo, "event_order_score").is_none() && order_total > 0 {
        let eo = order_hits as f64 / order_total as f64;
        hits.push("event_order_score".into());
        if eo >= 0.75 {
            *risk = (*risk - 0.05).max(0.0);
            reasons.push(format!("event_order_score={eo:.2}"));
        } else if eo < 0.35 {
            *risk = (*risk + 0.08).min(1.0);
            reasons.push(format!("event_order_low={eo:.2}"));
        }
    }
    if f_field(fo, "scroll_burst_n").is_none() && scroll_burst > 0 {
        hits.push("scroll_burst_n".into());
        *risk = (*risk - 0.03).max(0.0);
        reasons.push(format!("scroll_burst_n={scroll_burst}"));
    }
    if f_field(fo, "path_length_px").is_none() && path_len > 0.0 {
        hits.push("path_length_px".into());
        if path_len >= 80.0 {
            *risk = (*risk - 0.03).max(0.0);
            reasons.push(format!("path_length_px={path_len:.0}"));
        }
    }
    if f_field(fo, "input_velocity_cv").is_none() && speeds.len() >= 3 {
        let mean = speeds.iter().sum::<f64>() / speeds.len() as f64;
        let var = speeds.iter().map(|s| (s - mean).powi(2)).sum::<f64>() / speeds.len() as f64;
        let cv = if mean > 1e-9 {
            var.sqrt() / mean
        } else {
            0.0
        };
        hits.push("input_velocity_cv".into());
        if cv < 0.08 {
            *risk = (*risk + 0.12).min(1.0);
            reasons.push(format!("velocity_too_uniform={cv:.2}"));
        } else if cv >= 0.25 && cv < 3.0 {
            *risk = (*risk - 0.05).max(0.0);
            reasons.push(format!("input_velocity_cv={cv:.2}"));
        }
    }
}

/// Consume 001-aligned RPA kinematics from rpa_features_v2 / top-level fields.
fn apply_rpa_kinematics_v3(
    fo: &Map<String, Value>,
    risk: &mut f64,
    reasons: &mut Vec<String>,
    hits: &mut Vec<String>,
) {
    // Resolve nested features.features or top-level
    let nest = fo
        .get("rpa_features_v2")
        .and_then(|v| v.get("features"))
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let g = |k: &str| -> Option<f64> {
        f_field(fo, k).or_else(|| nest.get(k).and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64))))
    };
    let gb = |k: &str| -> Option<bool> {
        bool_field(fo, k).or_else(|| nest.get(k).and_then(|v| v.as_bool()))
    };
    let gs = |k: &str| -> Option<String> {
        str_field(fo, k)
            .map(|s| s.to_string())
            .or_else(|| nest.get(k).and_then(|v| v.as_str()).map(|s| s.to_string()))
    };
    if let Some(sig) = gs("rpa_signal") {
        hits.push("rpa_signal".into());
        if sig == "none" {
            reasons.push("rpa_signal_none".into());
            // no_signal: do not invent human safety
            return;
        }
        if sig == "present" {
            *risk = (*risk - 0.04).max(0.0);
            reasons.push("rpa_signal_present".into());
        }
    }
    if gb("rpa_rule_teleport") == Some(true) {
        *risk = (*risk + 0.14).min(1.0);
        reasons.push("rpa_rule_teleport".into());
        hits.push("rpa_rule_teleport".into());
    }
    if gb("rpa_rule_metronomic") == Some(true) {
        *risk = (*risk + 0.12).min(1.0);
        reasons.push("rpa_rule_metronomic".into());
        hits.push("rpa_rule_metronomic".into());
    }
    if gb("rpa_rule_zero_jerk") == Some(true) {
        *risk = (*risk + 0.12).min(1.0);
        reasons.push("rpa_rule_zero_jerk".into());
        hits.push("rpa_rule_zero_jerk".into());
    }
    if let Some(jcv) = g("input_jerk_cv") {
        hits.push("input_jerk_cv".into());
        if jcv < 0.03 {
            *risk = (*risk + 0.1).min(1.0);
            reasons.push(format!("jerk_too_low={jcv:.3}"));
        } else if jcv >= 0.15 && jcv < 8.0 {
            *risk = (*risk - 0.05).max(0.0);
            reasons.push(format!("input_jerk_cv={jcv:.2}"));
        }
    }
    if let Some(fc) = g("path_curvature_mean") {
        hits.push("path_curvature_mean".into());
        if fc < 0.02 {
            *risk = (*risk + 0.06).min(1.0);
            reasons.push(format!("path_almost_linear_curv={fc:.3}"));
        } else if fc >= 0.08 {
            *risk = (*risk - 0.03).max(0.0);
        }
    }
    if let Some(fr) = g("fitts_residual_mean") {
        hits.push("fitts_residual_mean".into());
        // bots often perfect Fitts or absurd residual
        if fr < 0.05 {
            *risk = (*risk + 0.05).min(1.0);
            reasons.push(format!("fitts_too_perfect={fr:.3}"));
        } else if fr > 2.5 {
            *risk = (*risk + 0.04).min(1.0);
            reasons.push(format!("fitts_absurd={fr:.2}"));
        } else {
            *risk = (*risk - 0.03).max(0.0);
        }
    }
    if let Some(on) = g("overshoot_n") {
        hits.push("overshoot_n".into());
        if on >= 1.0 {
            *risk = (*risk - 0.04).max(0.0);
            reasons.push(format!("overshoot_n={on:.0}"));
        }
    }
    if let Some(kfc) = g("key_flight_cv") {
        hits.push("key_flight_cv".into());
        if kfc < 0.08 {
            *risk = (*risk + 0.1).min(1.0);
            reasons.push(format!("key_flight_metronomic={kfc:.2}"));
        } else if kfc >= 0.2 {
            *risk = (*risk - 0.04).max(0.0);
        }
    }
    if let Some(wr) = g("window_risk_max") {
        hits.push("window_risk_max".into());
        if wr >= 0.6 {
            *risk = (*risk + 0.08).min(1.0);
            reasons.push(format!("window_risk_max={wr:.2}"));
        }
    }
    // Cross-layer: behavior humanish but JA4 vs UA conflict → keep risk
    let ja4_eng = str_field(fo, "protocol_engine")
        .or_else(|| str_field(fo, "gateway_ua_engine"))
        .or_else(|| str_field(fo, "ja4h_lite"));
    let claim_eng = str_field(fo, "engine_family")
        .or_else(|| str_field(fo, "claim_obs_ua_engine"));
    if let (Some(j), Some(c)) = (ja4_eng, claim_eng) {
        let j_l = j.to_ascii_lowercase();
        let c_l = c.to_ascii_lowercase();
        if (c_l.contains("blink") || c_l.contains("chrome"))
            && (j_l.contains("gecko") || j_l.contains("firefox") || j_l.contains("curl") || j_l.contains("python"))
        {
            *risk = (*risk + 0.12).min(1.0);
            reasons.push("rpa_ja4_vs_ua_engine_conflict".into());
            hits.push("ja4_vs_ua_rpa".into());
        }
    }
}

/// Completeness + consistency structure for OS/BR axes (001 cross-check; never mint labels).
pub fn os_br_completeness(fields: &Value) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let has = |k: &str| has_nonempty(&fo, k);
    let platform = has("platform") || has("ua_ch_platform");
    let ua = has("user_agent") || has("sec_ch_ua");
    let tz = has("timezone") || has("timezone_offset") || has("timezone_offset_min");
    let form = has("form_class") || has("max_touch_points");
    let fonts = has("font_matrix_hash") || has("font_presence_hash") || has("fonts_count");
    let webgl = has("webgl_unmasked_renderer")
        || has("hw_curve_webgl")
        || has("residual_mean")
        || has("webgl_vendor");
    let speech = has("speech_voices_n") || has("speech_voices_hash");
    let ja4 = has("ja4h") || has("ja4h_lite") || has("ja4l") || has("ja4l_lite");
    let webrtc = has("webrtc_host_ip_hash") || has("webrtc_host_ip_hash_v2");
    let keys = [
        ("platform", platform),
        ("ua", ua),
        ("timezone", tz),
        ("form", form),
        ("fonts", fonts),
        ("webgl", webgl),
        ("speech", speech),
        ("ja4", ja4),
        ("webrtc", webrtc),
    ];
    let present_n = keys.iter().filter(|(_, v)| *v).count() as f64;
    let total = keys.len() as f64;
    let completeness = present_n / total;

    let mut contradictions: Vec<String> = Vec::new();
    // mem vs cores
    if let (Some(mem), Some(cores)) = (
        f_field(&fo, "device_memory"),
        f_field(&fo, "hardware_concurrency"),
    ) {
        if mem <= 2.0 && cores >= 32.0 {
            contradictions.push("mem_low_cores_high".into());
        }
        if mem >= 16.0 && cores <= 1.0 {
            contradictions.push("mem_high_cores_low".into());
        }
    }
    // form vs touch
    let form_s = str_field(&fo, "form_class").unwrap_or("");
    let touch = f_field(&fo, "max_touch_points").unwrap_or(0.0);
    if form_s == "desktop" && touch >= 5.0 {
        contradictions.push("desktop_claim_high_touch".into());
    }
    if (form_s == "mobile" || form_s == "phone") && touch <= 0.0 {
        contradictions.push("mobile_claim_no_touch".into());
    }
    // UA engine vs JA4 / protocol engine
    let eng = str_field(&fo, "engine_family")
        .or_else(|| str_field(&fo, "claim_obs_ua_engine"))
        .unwrap_or("");
    let peng = str_field(&fo, "protocol_engine")
        .or_else(|| str_field(&fo, "gateway_ua_engine"))
        .unwrap_or("");
    if !eng.is_empty() && !peng.is_empty() {
        let e = eng.to_ascii_lowercase();
        let p = peng.to_ascii_lowercase();
        if (e.contains("blink") && (p.contains("gecko") || p.contains("firefox")))
            || (e.contains("gecko") && (p.contains("blink") || p.contains("chrome")))
        {
            contradictions.push("ua_engine_vs_protocol_engine".into());
        }
    }
    // webdriver vs rich materials
    if bool_field(&fo, "webdriver").unwrap_or(false) && webgl {
        contradictions.push("webdriver_with_webgl_materials".into());
    }
    // soft residual vs hard renderer claim
    if bool_field(&fo, "residual_soft_like").unwrap_or(false) {
        if let Some(r) = str_field(&fo, "webgl_unmasked_renderer") {
            let rl = r.to_ascii_lowercase();
            if rl.contains("nvidia") || rl.contains("radeon") || rl.contains("geforce") {
                contradictions.push("soft_residual_hard_gpu_label".into());
            }
        }
    }

    let consistent = contradictions.is_empty();
    json!({
        "algo": "os_br_completeness_v1",
        "completeness": (completeness * 1000.0).round() / 1000.0,
        "present_n": present_n as u32,
        "total_n": total as u32,
        "parts": {
            "platform": platform,
            "ua": ua,
            "timezone": tz,
            "form": form,
            "fonts": fonts,
            "webgl": webgl,
            "speech": speech,
            "ja4": ja4,
            "webrtc": webrtc,
        },
        "consistent": consistent,
        "contradictions": contradictions,
        "note": "completeness/consistency only — never feeds commercial device body labels",
    })
}

pub fn score_rpa(fields: &Value, bot: &BotScore, page_id: Option<&str>) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let mut reasons = Vec::new();
    let mut hits = Vec::new();
    let coverage = load_field_product_matrix()
        .map(|m| m.axis_coverage("rpa", fields))
        .unwrap_or(0.0);

    let bound = bool_field(&fo, "behavior_early_bound").unwrap_or(false);
    // Prefer rpa_features_v2 aggregates (iss/47); fall back to legacy event stream.
    let feat = fo
        .get("rpa_features_v2")
        .cloned()
        .or_else(|| fo.get("features").cloned());
    let feat_obj = feat.as_ref().and_then(|v| v.as_object());
    let feat_features = feat_obj
        .and_then(|o| o.get("features"))
        .and_then(|v| v.as_object());
    let feat_n = feat_features
        .and_then(|f| f.get("n_events").and_then(|v| v.as_f64()))
        .or_else(|| {
            feat_obj
                .and_then(|o| o.get("segment"))
                .and_then(|s| s.get("n_events"))
                .and_then(|v| v.as_f64())
        })
        .unwrap_or(0.0);
    let has_v2 = feat_obj
        .and_then(|o| o.get("schema").and_then(|v| v.as_str()))
        .map(|s| s == "rpa_features_v2")
        .unwrap_or(false)
        || feat_features.is_some();
    if has_v2 {
        hits.push("rpa_features_v2".into());
        reasons.push("rpa_features_v2_present".into());
    }
    // Prefer explicit count; else v2 n_events; else legacy events array length.
    let event_len = fo
        .get("behavior_events")
        .and_then(|v| v.as_array())
        .map(|a| a.len() as f64)
        .unwrap_or(0.0);
    let count = f_field(&fo, "behavior_count")
        .filter(|c| *c > 0.0)
        .or_else(|| if feat_n > 0.0 { Some(feat_n) } else { None })
        .unwrap_or(event_len);
    let has_events = event_len > 0.0 || count > 0.0 || has_v2;
    let wd = nested_bool(&fo, "automation", "webdriver")
        .or_else(|| bool_field(&fo, "webdriver"))
        .unwrap_or(false);
    let auto_pw = nested_bool(&fo, "automation", "playwright").unwrap_or(false);
    let auto_se = nested_bool(&fo, "automation", "selenium").unwrap_or(false);
    let auto_cdc = nested_bool(&fo, "automation", "cdc").unwrap_or(false);
    let outer_zero = bool_field(&fo, "outer_zero").unwrap_or(false);
    let cdp_n = f_field(&fo, "cdp_runtime_hint").unwrap_or(0.0);
    let cdp_b = bool_field(&fo, "cdp_runtime_hint").unwrap_or(false);
    let agent_auto_n = f_field(&fo, "agent_automation_globals_n").unwrap_or(0.0);
    let ua = str_field(&fo, "user_agent").unwrap_or("");
    let headlessish = ua.contains("HeadlessChrome")
        || ua.to_ascii_lowercase().contains("headless")
        || ua.to_ascii_lowercase().contains("phantom");
    // Control-plane materials (iss/25): score rpa even without bio events.
    let headless_likely = bool_field(&fo, "headless_likely").unwrap_or(false);
    let has_control = wd
        || auto_pw
        || auto_se
        || auto_cdc
        || outer_zero
        || cdp_n >= 1.0
        || cdp_b
        || agent_auto_n >= 1.0
        || headlessish
        || headless_likely
        || matches!(bot.verdict.as_str(), "bot" | "crawler" | "automation" | "rpa");

    // pagehide / flush markers
    let pagehide = bool_field(&fo, "pagehide_flush")
        .or_else(|| bool_field(&fo, "behavior_pagehide"))
        .unwrap_or(false);

    let mut risk = 0.35_f64;
    // 001 RPA kinematics layer (jerk / Fitts / key flight / window / rules)
    apply_rpa_kinematics_v3(&fo, &mut risk, &mut reasons, &mut hits);
    // Unknown only when neither bio nor control-plane evidence exists.
    if !bound && !has_events && !has_control {
        reasons.push("no_behavior_data".into());
        if page_id.is_some() {
            hits.push("page_id".into());
        }
        let safety: f64 = if page_id.is_some() { 0.38 } else { 0.4 };
        return score_block(
            safety.min(0.45_f64),
            "unknown",
            coverage.max(0.05_f64),
            reasons,
            hits,
        );
    }
    if !bound && !has_events && has_control {
        reasons.push("no_behavior_control_plane_only".into());
        hits.push("rpa_control_plane_family".into());
    }
    if has_control {
        hits.push("rpa_control_plane_family".into());
        reasons.push("control_plane_scored_separately_from_behavior".into());
    }
    if bound {
        risk = (risk - 0.1).max(0.0);
        reasons.push("behavior_early_bound".into());
        hits.push("behavior_early_bound".into());
    }
    if has_events {
        // Count MUST NOT invent "human" safety (iss/47): scripts pad event length.
        // Coverage-only bump; kinematics / control-plane dominate risk.
        reasons.push(format!("behavior_count={count}_coverage_only"));
        hits.push("behavior_count".into());
        // Tiny coverage presence only when sample_quality not insufficient
        let sq = feat_obj
            .and_then(|o| o.get("segment"))
            .and_then(|s| s.get("sample_quality"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if sq != "insufficient" && count >= 4.0 {
            risk = (risk - 0.03).max(0.0);
            reasons.push("behavior_presence_coverage_only".into());
        }
        if event_len > 0.0 {
            hits.push("behavior_events".into());
        }
        // Split control-plane vs behavior evidence families (decouple axes)
        hits.push("rpa_behavior_family".into());
        // Sample quality gate from v2 (insufficient → demote confidence, not invent safety)
        if let Some(q) = feat_obj
            .and_then(|o| o.get("segment"))
            .and_then(|s| s.get("sample_quality"))
            .and_then(|v| v.as_str())
        {
            hits.push("sample_quality".into());
            if q == "insufficient" {
                risk = (risk + 0.08).min(1.0);
                reasons.push("rpa_sample_quality_insufficient".into());
            } else if q == "adequate" {
                risk = (risk - 0.04).max(0.0);
                reasons.push("rpa_sample_quality_adequate".into());
            }
        }
        // Event variety: accept FE `kind` or `type` (registry emits kind; fixtures may use type)
        if let Some(arr) = fo.get("behavior_events").and_then(|v| v.as_array()) {
            let mut types = std::collections::HashSet::new();
            for e in arr {
                if let Some(t) = e
                    .get("type")
                    .or_else(|| e.get("kind"))
                    .and_then(|v| v.as_str())
                {
                    types.insert(t.to_string());
                }
            }
            if types.len() >= 3 {
                risk = (risk - 0.08).max(0.0);
                reasons.push("behavior_type_diversity".into());
            } else if types.len() == 1 {
                risk = (risk + 0.06).min(1.0);
                reasons.push("behavior_type_monotone".into());
            }
            // Server-side kinematics from event stream when FE summary absent
            apply_rpa_kinematics(&fo, arr, &mut risk, &mut reasons, &mut hits);
        } else {
            // v2 aggregate path: flatten features so uploaded kinematics are consumed
            let mut kin_fo = fo.clone();
            if let Some(ff) = feat_features {
                for (k, v) in ff {
                    if !kin_fo.contains_key(k) {
                        kin_fo.insert(k.clone(), v.clone());
                    }
                }
            }
            apply_rpa_kinematics_fields_only(&kin_fo, &mut risk, &mut reasons, &mut hits);
            if let Some(ff) = feat_features {
                if let Some(n) = ff.get("type_diversity_n").and_then(|v| v.as_f64()) {
                    hits.push("type_diversity_n".into());
                    if n >= 3.0 {
                        risk = (risk - 0.06).max(0.0);
                        reasons.push(format!("v2_type_diversity_n={n:.0}"));
                    } else if n <= 1.0 {
                        risk = (risk + 0.05).min(1.0);
                        reasons.push("v2_type_diversity_low".into());
                    }
                }
                if ff
                    .get("sensitive_action_zero_move")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    risk = (risk + 0.12).min(1.0);
                    reasons.push("v2_sensitive_action_zero_move".into());
                }
            }
        }
    }
    // FE-precomputed diversity count (matrix T2 conf) — must be read, not only re-derived
    if let Some(n) = f_field(&fo, "behavior_type_diversity_n") {
        hits.push("behavior_type_diversity_n".into());
        if n >= 3.0 {
            risk = (risk - 0.05).max(0.0);
            reasons.push(format!("behavior_type_diversity_n={n:.0}"));
        } else if n <= 1.0 && (has_events || bound) {
            risk = (risk + 0.05).min(1.0);
            reasons.push("behavior_type_diversity_n_low".into());
        }
    }
    if bool_field(&fo, "sensitive_action_seen").unwrap_or(false)
        || has_nonempty(&fo, "sensitive_action_seen")
    {
        hits.push("sensitive_action_seen".into());
        // Seen without pre-move is already covered by sensitive_action_zero_move;
        // presence alone is weak positive (user reached sensitive surface).
        if bool_field(&fo, "sensitive_action_seen") == Some(true)
            && !bool_field(&fo, "sensitive_action_zero_move").unwrap_or(false)
        {
            risk = (risk - 0.02).max(0.0);
        }
    }
    if pagehide {
        hits.push("pagehide_flush".into());
        reasons.push("pagehide_flush".into());
        // short-visit flush is good signal that FE completed bind path
        risk = (risk - 0.04).max(0.0);
    }
    if bool_field(&fo, "rpa_idle_flush").unwrap_or(false) {
        hits.push("rpa_idle_flush".into());
        reasons.push("rpa_idle_flush".into());
        risk = (risk - 0.02).max(0.0);
    }
    if has_nonempty(&fo, "page_url") {
        hits.push("page_url".into());
    }
    // Multi-source nest capability + webdriver/toString nest mismatches
    apply_sandbox_capability_axis("rpa", &fo, &mut risk, &mut reasons, &mut hits);
    apply_cross_context_mismatch_axis("rpa", &fo, &mut risk, &mut reasons, &mut hits);
    // Source conflicts / multi-source mismatch demote RPA (policy weights).
    {
        let w = crate::policy::multi_source_conflict_weights();
        if let Some(mr) = f_field(&fo, "multi_source_match_ratio") {
            if mr < w.rpa_match_ratio_lt {
                hits.push("multi_source_mismatch_rpa".into());
                risk = (risk + w.rpa_mismatch_add).min(1.0);
                reasons.push(format!("multi_source_mismatch_rpa={mr:.2}"));
            }
        }
        if bool_field(&fo, "sandbox_ua_mismatch").unwrap_or(false)
            || bool_field(&fo, "iframe_ua_mismatch").unwrap_or(false)
            || bool_field(&fo, "sandbox_webdriver_mismatch").unwrap_or(false)
        {
            hits.push("sandbox_identity_mismatch_rpa".into());
            risk = (risk + w.rpa_base + 0.04).min(1.0);
            reasons.push("sandbox_identity_mismatch_rpa".into());
        }
    }
    // --- Control plane (primary rpa materials; not br-owned) ---
    if wd {
        risk = (risk + 0.32).min(1.0);
        reasons.push("webdriver".into());
        hits.push("webdriver".into());
    }
    if auto_pw {
        risk = (risk + 0.22).min(1.0);
        reasons.push("automation.playwright".into());
        hits.push("automation.playwright".into());
    }
    if auto_se {
        risk = (risk + 0.22).min(1.0);
        reasons.push("automation.selenium".into());
        hits.push("automation.selenium".into());
    }
    if auto_cdc {
        risk = (risk + 0.20).min(1.0);
        reasons.push("automation.cdc".into());
        hits.push("automation.cdc".into());
    }
    if outer_zero {
        risk = (risk + 0.22).min(1.0);
        reasons.push("outer_zero".into());
        hits.push("outer_zero".into());
    }
    if cdp_n >= 1.0 || cdp_b {
        hits.push("cdp_runtime_hint".into());
        risk = (risk + 0.24).min(1.0);
        reasons.push(if cdp_n >= 1.0 {
            format!("cdp_runtime_hint={cdp_n:.0}")
        } else {
            "cdp_runtime_hint".into()
        });
    }
    if agent_auto_n >= 1.0 {
        hits.push("agent_automation_globals_n".into());
        risk = (risk + (0.12 + 0.04 * agent_auto_n.min(5.0)).min(0.28)).min(1.0);
        reasons.push(format!("agent_automation_globals_n={agent_auto_n:.0}"));
    }
    if headlessish {
        hits.push("headless_ua".into());
        risk = (risk + 0.18).min(1.0);
        reasons.push("headless_ua".into());
    }
    // demo d28 forgeability L5 alias
    if bool_field(&fo, "headless_likely").unwrap_or(false) {
        hits.push("headless_likely".into());
        risk = (risk + 0.16).min(1.0);
        reasons.push("headless_likely".into());
    }
    // Soft residual is **os-primary**; only corroborates rpa when control-plane is also present
    // (demo multi-VM soft farm / headless cluster). Never sole rpa driver (product_axis_contract).
    {
        let soft_obs = bool_field(&fo, "residual_soft_like").unwrap_or(false)
            || bool_field(&fo, "soft_stack").unwrap_or(false)
            || fo
                .get("stack_class")
                .and_then(|v| v.as_str())
                .is_some_and(|s| matches!(s, "soft_render" | "software" | "virt" | "emulator"));
        let emu = bool_field(&fo, "emulator_hint").unwrap_or(false)
            || fo
                .get("emulator_hint")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty() && s != "false");
        let control_present = wd
            || auto_pw
            || auto_se
            || auto_cdc
            || outer_zero
            || cdp_n >= 1.0
            || cdp_b
            || agent_auto_n >= 1.0
            || headlessish
            || headless_likely;
        if soft_obs && control_present {
            hits.push("residual_soft_like".into());
            risk = (risk + 0.10).min(1.0);
            reasons.push("soft_residual_farm_corroboration".into());
        }
        // X-9: emulator + control plane → rpa farm corroboration (not device key).
        if emu && control_present {
            hits.push("emulator_hint".into());
            risk = (risk + 0.08).min(1.0);
            reasons.push("emulator_control_plane_corroboration".into());
        }
    }
    // --- Environment-plane RPA (aligned with OS/BR; bio remains primary when present) ---
    // Fingerprint / antidetect browsers: strong control-surface suspicion (verify via packs, score now).
    let antidetect = bool_field(&fo, "antidetect_vendor_hint").unwrap_or(false)
        || bool_field(&fo, "fingerprint_vendor_lie").unwrap_or(false)
        || fo
            .get("antidetect_vendor_hint")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty() && s != "false");
    if antidetect {
        hits.push("antidetect_vendor_hint".into());
        risk = (risk + 0.16).min(1.0);
        reasons.push("antidetect_rpa_env".into());
    }
    if bool_field(&fo, "prototype_chain_tamper").unwrap_or(false) {
        hits.push("prototype_chain_tamper".into());
        risk = (risk + 0.12).min(1.0);
        reasons.push("prototype_tamper_rpa_env".into());
    }
    let spoof_r = f_field(&fo, "spoof_score").unwrap_or(0.0);
    if spoof_r >= 0.45 {
        hits.push("spoof_score".into());
        risk = (risk + 0.10).min(1.0);
        reasons.push(format!("spoof_score_rpa_env={spoof_r:.2}"));
    }
    // Incomplete OS/BR surface (stripped / custom browser toolkit) without rich bio:
    // raise RPA suspicion — custom tools often suppress UA-CH, memory, RTC, etc.
    {
        let no_ua_ch = !has_nonempty(&fo, "ua_ch_platform")
            && !has_nonempty(&fo, "ua_ch_architecture")
            && !has_nonempty(&fo, "ua_ch_full_version_list")
            && fo.get("ua_ch").and_then(|v| v.as_bool()) != Some(true);
        let no_mem = f_field(&fo, "device_memory").unwrap_or(0.0) <= 0.0
            && !has_nonempty(&fo, "device_memory");
        let webrtc_fail = str_field(&fo, "webrtc_probe_failed").unwrap_or("");
        let no_webrtc = !has_nonempty(&fo, "webrtc_host_ip_hash")
            && (bool_field(&fo, "webrtc_missing").unwrap_or(false)
                || !webrtc_fail.is_empty()
                || fo
                    .get("webrtc_host_count")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(-1)
                    == 0);
        // Blink/Chrome normally has UA-CH; missing on Chrome UA is more suspicious than WebKit.
        let ua = str_field(&fo, "user_agent").unwrap_or("");
        let claims_blink = ua.contains("Chrome/")
            && !ua.contains("Edg/")
            && (ua.contains("Chrome/") || ua.contains("Chromium/"));
        let incomplete_n =
            (no_ua_ch as u8) + (no_mem as u8) + (no_webrtc as u8);
        if incomplete_n >= 2 {
            hits.push("env_incomplete_surface".into());
            let mut add = 0.05 + 0.03 * (incomplete_n as f64 - 1.0);
            if claims_blink && no_ua_ch {
                add += 0.06;
                reasons.push("blink_missing_ua_ch_rpa_env".into());
            }
            // Platform no_rtc alone is weaker (expected on some WebKit) — don't double-count.
            if webrtc_fail == "no_rtc" && !claims_blink {
                add = (add - 0.03).max(0.03);
                reasons.push("env_incomplete_webkit_partial".into());
            } else {
                reasons.push("env_incomplete_custom_surface".into());
            }
            // Stronger when bio is thin — custom automation often has sparse events.
            if !has_events || count < 3.0 {
                add += 0.05;
                reasons.push("env_incomplete_thin_bio".into());
            }
            risk = (risk + add).min(1.0);
        }
    }
    if matches!(bot.verdict.as_str(), "bot" | "crawler" | "automation" | "rpa") {
        risk = (risk + 0.15).min(1.0);
        reasons.push(format!("session_bot={}", bot.verdict));
    }
    if has_events && count < 3.0 {
        risk = (risk + 0.1).min(1.0);
        reasons.push("sparse_events".into());
    }
    // Timing fields as weak rpa conf
    if has_nonempty(&fo, "perf_now") {
        hits.push("perf_now".into());
    }
    crate::rule_sample_loader::apply_rule_samples_axis(
        fields, "rpa", &mut risk, &mut reasons, &mut hits,
    );
    apply_probe_surface_evidence_rpa(&fo, &mut risk, &mut reasons, &mut hits);

    apply_matrix_role_evidence("rpa", &fo, &mut risk, &mut reasons, &mut hits);
    apply_dense_digest_evidence("rpa", &fo, &mut risk, &mut reasons, &mut hits);
    apply_cross_axis_corroboration("rpa", &fo, &mut risk, &mut reasons, &mut hits);

    let mut safety = 1.0 - risk;
    let cov_floor = if has_events {
        0.45
    } else if has_control {
        0.35
    } else {
        0.15
    };
    let cov_cap = 0.25 + 0.75 * coverage.max(cov_floor);
    if safety > cov_cap {
        safety = cov_cap;
        reasons.push("coverage_cap".into());
    }
    // Control-plane-only must never claim "human"
    let status = if !has_events && !bound && !has_control {
        "unknown"
    } else if !has_events && !bound && has_control {
        if safety >= 0.4 {
            "automation_suspect"
        } else {
            "bot"
        }
    } else if safety >= 0.7 {
        "human"
    } else if safety >= 0.4 {
        "automation_suspect"
    } else {
        "bot"
    };
    let mut rpa_block = score_block(
        safety,
        status,
        coverage.max(if has_events {
            0.45
        } else if has_control {
            0.35
        } else {
            0.2
        }),
        reasons,
        hits,
    );
    let bands = crate::rpa_bands::rpa_behavior_bands(fields);
    if let Some(obj) = rpa_block.as_object_mut() {
        obj.insert(
            "evidence_families".into(),
            json!({
                "control_plane": has_control,
                "behavior": has_events || bound,
                "count_does_not_boost_human": true,
                "schema": if has_v2 { "rpa_features_v2" } else { "legacy_or_control" },
            }),
        );
        // iss/50 R5/R6 dual-axis bands
        obj.insert("rpa_bands".into(), bands.clone());
        obj.insert(
            "human_likeness".into(),
            bands.get("human_likeness").cloned().unwrap_or(json!(null)),
        );
        obj.insert(
            "automation_evidence".into(),
            bands
                .get("automation_evidence")
                .cloned()
                .unwrap_or(json!(null)),
        );
        // Explicit dual scores for product consumers (0=human-like, 1=bot-like for bot_score)
        let human = safety.clamp(0.0, 1.0);
        let bot = (1.0 - human).clamp(0.0, 1.0);
        obj.insert("human_score".into(), json!(human));
        obj.insert("bot_score".into(), json!(bot));
        obj.insert(
            "bot_score_algo".into(),
            json!("gr_rpa_bot_score_v1_from_safety"),
        );
    }
    rpa_block
}

/// Ops-facing fusion telemetry: collision_risk, claim-obs hits, unit_surface coverage.
/// Machine-readable on product path (no separate dashboard required).
pub fn build_ops_fusion_telemetry(
    device: &Value,
    fields: &Value,
    stack: &StackAuth,
    os: &Value,
    br: &Value,
    rpa: &Value,
) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let collect_reasons = |block: &Value| -> Vec<String> {
        block
            .get("reasons")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect()
    };
    let os_rs = collect_reasons(os);
    let br_rs = collect_reasons(br);
    let rpa_rs = collect_reasons(rpa);
    let is_claim_obs = |r: &str| {
        r.contains("claim")
            || r.contains("gpu_label")
            || r.contains("integrity")
            || r.contains("incoherent")
            || r.contains("capability")
            || r.contains("collusion")
    };
    let mut claim_obs_hits: Vec<String> = Vec::new();
    for r in os_rs.iter().chain(br_rs.iter()) {
        if is_claim_obs(r) && !claim_obs_hits.contains(r) {
            claim_obs_hits.push(r.clone());
        }
    }
    let unit_present = fo
        .get("unit_surface_id")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());
    let unit_algo = fo
        .get("unit_surface_algo")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let unit_stable = fo
        .get("unit_multiround_stable")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let multi_seed_n = fo
        .get("multi_seed_n")
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)));
    let collision_risk = device
        .get("collision_risk")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    json!({
        "collision_risk": collision_risk,
        "claim_obs_hit_count": claim_obs_hits.len(),
        "claim_obs_hits": claim_obs_hits,
        "claim_obs_rate_proxy": if claim_obs_hits.is_empty() { 0.0 } else { 1.0 },
        "unit_surface_coverage": {
            "present": unit_present,
            "algo": unit_algo,
            "stable": unit_stable,
            "versioned_ok": unit_present
                && unit_stable
                && (unit_algo == "gr_unit_v1" || unit_algo.starts_with("gr_unit_v")),
            "multi_seed_n": multi_seed_n,
            "coverage_flag": if unit_present { "covered" } else { "missing" },
        },
        "soft_stack": stack.soft_stack,
        "gpu_label_untrusted": stack.gpu_label_untrusted,
        "server_mint": device.get("server_mint").cloned().unwrap_or(json!(false)),
        "uniqueness_marker": device.get("uniqueness_marker").cloned().unwrap_or(json!(null)),
        "digest_path": device.get("digest_path").cloned().unwrap_or(json!(null)),
        "rpa_control_reasons": rpa_rs.iter().filter(|r| {
            r.contains("webdriver") || r.contains("cdp") || r.contains("headless")
                || r.contains("playwright") || r.contains("farm")
        }).cloned().collect::<Vec<_>>(),
        "field_algorithm_weights_ref": "link_or_mint::field_algorithm_weights",
    })
}

/// Build norm/10 product + diagnostics split.
///
/// `evidence` optional: when present, open re-probe gaps are derived from full session
/// evidence (batches/sources); otherwise synthetic field-only evidence.
pub fn build_product_surface(
    device: &Value,
    fields: &Value,
    stack: &StackAuth,
    bot: &BotScore,
    truth: &TruthResult,
    page_id: Option<&str>,
    decision_nonsensitive: &Value,
    decision_sensitive: &Value,
    bot_algo: &str,
    link_algo: &str,
    coverage_complete: Option<bool>,
    analysis_terminal: bool,
    source_conflicts: &[String],
) -> (Value, Value, Option<Value>) {
    build_product_surface_with_evidence(
        device,
        fields,
        stack,
        bot,
        truth,
        page_id,
        decision_nonsensitive,
        decision_sensitive,
        bot_algo,
        link_algo,
        coverage_complete,
        analysis_terminal,
        source_conflicts,
        None,
    )
}

/// Same as [`build_product_surface`] but attaches open verification gaps from full evidence.
pub fn build_product_surface_with_evidence(
    device: &Value,
    fields: &Value,
    stack: &StackAuth,
    bot: &BotScore,
    truth: &TruthResult,
    page_id: Option<&str>,
    decision_nonsensitive: &Value,
    decision_sensitive: &Value,
    bot_algo: &str,
    link_algo: &str,
    coverage_complete: Option<bool>,
    analysis_terminal: bool,
    source_conflicts: &[String],
    evidence: Option<&Value>,
) -> (Value, Value, Option<Value>) {
    // Shared projections: device materials (webrtc_missing, residual entropy, …) re-enter
    // OS/BR/RPA scorers so one probe field feeds multiple product axes without re-probing.
    let fields_enriched = enrich_fields_with_device_materials(fields, device);
    let mut os = score_os(&fields_enriched, stack, truth, source_conflicts);
    let mut br = score_br(&fields_enriched, stack, bot, truth, source_conflicts);
    let mut rpa = score_rpa(&fields_enriched, bot, page_id);
    // Shared algorithm-group registry: probe materials boost os/br/rpa conf only
    // (never rewrite commercial device_id).
    let score_boost = crate::algo_groups::score_materials_boost(&fields_enriched);
    let apply_boost = |score: &mut Value, axis: &str| {
        let b = score_boost
            .get(axis)
            .and_then(|a| a.get("boost"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        if b <= 0.0 {
            return;
        }
        if let Some(obj) = score.as_object_mut() {
            if let Some(s) = obj.get("score").and_then(|v| v.as_f64()) {
                obj.insert("score".into(), json!(clamp01(s + b * 0.5)));
            }
            if let Some(c) = obj.get("confidence").and_then(|v| v.as_f64()) {
                obj.insert("confidence".into(), json!(clamp01(c + b)));
            } else {
                obj.insert("confidence".into(), json!(clamp01(0.5 + b)));
            }
            obj.insert("algo_group_materials_boost".into(), json!(b));
            if let Some(fields_used) = score_boost.get(axis).and_then(|a| a.get("fields")) {
                obj.insert("algo_group_fields".into(), fields_used.clone());
            }
        }
    };
    apply_boost(&mut os, "os");
    apply_boost(&mut br, "br");
    apply_boost(&mut rpa, "rpa");
    // Multi-source conflict / single-source pressure demotion (shared os/br/rpa).
    // Never rewrites commercial device_id — score risk only.
    let gate = fields_enriched
        .get("multi_source_mint_gate")
        .cloned()
        .or_else(|| device.get("multi_source_mint_gate").cloned())
        .unwrap_or(json!({}));
    let (ms_risk, ms_reasons) =
        crate::multi_source_mint::conflict_score_demotion(&gate, source_conflicts);
    let apply_ms_demote = |score: &mut Value, axis: &str| {
        if ms_risk <= 0.0 {
            return;
        }
        if let Some(obj) = score.as_object_mut() {
            if let Some(s) = obj.get("score").and_then(|v| v.as_f64()) {
                // safety = 1 - risk elsewhere; here scores are already safety-oriented.
                obj.insert("score".into(), json!(clamp01(s * (1.0 - ms_risk * 0.85))));
            }
            if let Some(c) = obj.get("confidence").and_then(|v| v.as_f64()) {
                obj.insert("confidence".into(), json!(clamp01(c * (1.0 - ms_risk * 0.6))));
            }
            obj.insert("multi_source_demotion_risk".into(), json!(ms_risk));
            obj.insert(
                "multi_source_demotion_reasons".into(),
                json!(ms_reasons),
            );
            obj.insert("multi_source_demotion_axis".into(), json!(axis));
        }
    };
    apply_ms_demote(&mut os, "os");
    apply_ms_demote(&mut br, "br");
    apply_ms_demote(&mut rpa, "rpa");
    // Also apply mint_conflict_pressure when stamped on fields (evaluate path).
    if let Some(cp) = fields_enriched
        .get("mint_conflict_pressure")
        .and_then(|v| v.as_f64())
    {
        if cp > 0.0 {
            for score in [&mut os, &mut br, &mut rpa] {
                if let Some(obj) = score.as_object_mut() {
                    if let Some(s) = obj.get("score").and_then(|v| v.as_f64()) {
                        obj.insert(
                            "score".into(),
                            json!(clamp01(s - 0.10 * cp - 0.05 * cp.min(1.0))),
                        );
                    }
                    obj.insert("mint_conflict_pressure_applied".into(), json!(cp));
                }
            }
        }
    }
    let density = field_density(&fields_enriched);
    let contributions = field_axis_contributions(fields);

    // Confidence: completeness × importance (matrix roles/tiers) + score quality.
    let os = attach_score_confidence_axis(os, Some("os"), fields);
    let br = attach_score_confidence_axis(br, Some("br"), fields);
    let rpa = attach_score_confidence_axis(rpa, Some("rpa"), fields);

    let page_url = fields
        .get("page_url")
        .or_else(|| fields.get("href"))
        .cloned()
        .unwrap_or(Value::Null);

    let ops_telemetry = build_ops_fusion_telemetry(device, fields, stack, &os, &br, &rpa);
    let reference_aux_hits = collect_reference_aux_hits(&os, &br);
    let platform_honesty = crate::analysis_quality::classify_platform_honesty(fields);

    let mut product = json!({
        "device_id": device.get("device_id"),
        "device_id_reserved_compare": device.get("device_id_reserved_compare").cloned().unwrap_or(json!(null)),
        "device_id_segments": device.get("device_id_segments").cloned().unwrap_or(json!(null)),
        // multi-segment commercial mint flags (device_segments_v1) — keep on product for
        // SDK/lab matrix readers that do not walk device.* (return_gate also whitelists).
        "multi_segment": device.get("multi_segment").cloned().unwrap_or_else(|| {
            let tier = device.get("device_tier").and_then(|v| v.as_str()).unwrap_or("");
            json!(tier == "multi")
        }),
        "device_algo_group": device.get("device_algo_group").cloned()
            .or_else(|| device.get("algo_group").cloned())
            .unwrap_or(json!(null)),
        "slot_scheme": device.get("slot_scheme").cloned().unwrap_or(json!("extended_curve_v2")),
        "slot_aliases": device.get("slot_aliases").cloned().unwrap_or(json!({})),
        "curve_descriptors": device.get("curve_descriptors").cloned().unwrap_or(json!({})),
        "conf_ceiling": device.get("conf_ceiling").cloned().unwrap_or(json!(null)),
        "conf_reserved_compare": device.get("conf_reserved_compare").cloned().unwrap_or(json!(null)),
        "material_slots_addressable": device.get("material_slots_addressable").cloned().unwrap_or(json!(10)),
        "slot_quality": device.get("slot_quality").cloned().unwrap_or(json!({})),
        "device_tier": device.get("device_tier"),
        "device_confidence": device.get("device_confidence").cloned()
            .or_else(|| device.get("confidence").cloned()),
        "confidence_version": device.get("confidence_version").cloned().unwrap_or(json!(null)),
        "collision_risk": device.get("collision_risk").cloned().unwrap_or(json!(false)),
        "analysis_posture": device.get("analysis_posture").cloned().unwrap_or(json!([])),
        "no_id_reasons": device.get("no_id_reasons").cloned().unwrap_or(json!([])),
        "tier_reasons": device.get("tier_reasons").cloned().unwrap_or(json!([])),
        "server_mint": device.get("server_mint").cloned().unwrap_or(json!(false)),
        "server_mint_algo": device.get("server_mint_algo").cloned().unwrap_or(json!(null)),
        "uniqueness_marker": device.get("uniqueness_marker").cloned().unwrap_or(json!(null)),
        "digest_path": device.get("digest_path").cloned().unwrap_or(json!(null)),
        "browser_surface_id": device.get("browser_surface_id").cloned().unwrap_or(json!(null)),
        "association_level": device.get("association_level").cloned().unwrap_or(json!(null)),
        "association_basis": device.get("association_basis").cloned().unwrap_or(json!([])),
        "association_ladder": device.get("association_ladder").cloned().unwrap_or(json!(null)),
        "peer_similarity": device.get("peer_similarity").cloned().unwrap_or(json!(null)),
        "homogenization": device.get("homogenization").cloned().unwrap_or(json!(null)),
        "environment_flags": os.get("environment_flags").cloned().unwrap_or(json!(null)),
        "algo_group": device.get("algo_group").cloned().unwrap_or(json!(null)),
        "algo_group_id": device.get("algo_group_id").cloned().unwrap_or(json!(null)),
        "provisional_gateway": device.get("provisional_gateway").cloned().unwrap_or(json!(false)),
        "score_materials_boost": score_boost.clone(),
        "os": os.clone(),
        "br": br.clone(),
        "rpa": rpa.clone(),
        "ops_fusion_telemetry": ops_telemetry.clone(),
        "reference_aux_hits": reference_aux_hits,
        "platform_honesty": platform_honesty,
        "rule_samples_shadow": crate::rule_sample_loader::evaluate_rule_samples_shadow(fields),
        "self_capability": crate::self_capability::self_capability_json(),
        "brain_hypotheses": crate::brain_hypotheses::hypothesis_coverage_json(fields, &[]),
        "sub_algorithms": {
            "stack_auth": stack.stack_class,
            "commercial_projection": device.get("algo").cloned().unwrap_or(json!("machine_trust_v2")),
            "server_mint": device.get("server_mint_algo").cloned().unwrap_or(json!("server_mint_v1")),
            "link_or_mint": device.get("link_or_mint_algo").cloned().unwrap_or(json!("link_or_mint_v1")),
            "score_os": "product_scores.score_os",
            "score_br": "product_scores.score_br",
            "score_rpa": "product_scores.score_rpa",
            "mutual_verification": "analysis_quality.mutual_verification_v1",
            "field_utilization": "analysis_quality.field_utilization_v1",
            "association_ladder": "association_ladder_v1",
            "peer_similarity": crate::peer_similarity::PEER_SIMILARITY_ALGO,
            "homogenization": "homogenization_v1",
            "environment_homogeneity": "environment_homogeneity_v1",
            "rpa_bands": crate::rpa_bands::RPA_BANDS_ALGO,
            "config_snapshot": crate::config_snapshot::CONFIG_SNAPSHOT_ALGO,
            "behavior_self_sim": crate::behavior_self_sim::BEHAVIOR_SELF_SIM_ALGO,
            "claim_obs_graph": crate::claim_obs_graph::CLAIM_OBS_GRAPH_ALGO,
            "shadow_score": crate::claim_obs_graph::SHADOW_SCORE_ALGO,
            "canary_model": "canary_model_v1",
            "ja4h_lite": crate::ja4h_lite::JA4H_LITE_ALGO,
            "hw_silicon_fusion": crate::hw_silicon_fusion::HW_SILICON_FUSION_ALGO,
            "hw_anti_collision": crate::hw_anti_collision::HW_ANTI_COLLISION_ALGO,
            "hw_channel_census": crate::hw_channel_census::HW_CHANNEL_CENSUS_ALGO,
            "hw_channel_drift": crate::hw_channel_drift::HW_CHANNEL_DRIFT_ALGO,
            "silicon_fusion_weights": crate::hw_fusion_weights::SILICON_FUSION_WEIGHTS_ALGO,
            "lane_s_engine_norm": crate::hw_engine_norm::LANE_S_ENGINE_NORM_ALGO,
            "environment_flags": "environment_flags_v1",
            "algo_groups": crate::algo_groups::ALGO_GROUPS_ALGO,
            "device_tier": device.get("tier_reasons").cloned().unwrap_or(json!([])),
        },
        "product_redlines": {
            "os_br_rpa_not_device_key": true,
            "os_br_rpa_scope": "current_browser_or_page_only",
            "device_id_cross_browser_best_effort": true,
            "no_pure_fe_global_uv_promise": true,
            "ip_ua_ja_never_commercial_digest": true,
            "reference_aux_ok": true,
            "ja_aux_low_weight_br_ok": true,
            "ua_ip_asn_aux_ok": true,
            "l0_renderer_claim_obs_ok": true,
            "ja_policy": "JA3/JA4/protocol_engine = low-weight br/os claim-obs aux (confirm or contradict); never commercial device digest key or UV",
            "ua_ip_policy": "UA/IP/ASN = reference_aux on os/br (and device_id diag); low weight / contradict; never digest or sole merge key",
            "never_digest_means": "commercial digest_order exclusion only — not a global product ban",
            "soft_never_promote_to_silicon_dh": true,
            "peer_similarity_not_account_graph": true,
            "homogenization_is_product_signal": true,
            "environment_homogeneity_is_c3_alias": true,
            "b34_cache_ladder_not_mint": true,
            "h2h3_pseudo_priority_diagnostic_only": true,
            "shadow_never_drives_production_gate": true,
            "canary_never_auto_promote": true,
            "behavior_self_sim_not_commercial_id": true,
            "ja4h_lite_not_commercial_mint": true,
            "hw_silicon_fusion_never_alone_uv": true,
            "residual_alone_not_silicon_uv": true,
            "gpu_model_string_never_commercial_body": true,
            "fusion_weights_never_silent_adopt": true,
            "engine_norm_provisional_until_calibrated": true,
            // iss/67 C2: when GR_CHANNEL_DRIFT_GATE=1, drift may demote conf (still no hard-block).
            // Default off → redline true (not production gate); gate on → redline false.
            "channel_drift_not_production_gate": !crate::hw_channel_drift::drift_gate_enabled(),
            "channel_drift_hard_block": false,
            "gl_governor_isolated_from_unmasked": true,
            "cross_account_association_on_sdk": true,
            "silent_probe_no_permission_request": true,
            "banned_permission_apis": [
                "getUserMedia",
                "getDisplayMedia",
                "geolocation",
                "clipboard.read",
                "Notification.requestPermission",
                "DeviceOrientationEvent.requestPermission",
                "DeviceMotionEvent.requestPermission",
                "queryLocalFonts"
            ],
            "passive_site_media_ok": true,
            "collision_sla": "report collision_risk + KPI; never promise global unique pure-FE",
            "catalog_gate_forbidden": "global brand/UA/cloud-vendor whitelist as hard decision gate still forbidden",
        },
        "collision_posture": crate::collision_kpi::collision_posture_for_fields(fields),
        "note": "os/br scores: higher = better quality; NOT cross-browser device keys. device_id = env/machine ladder (dh_|dv_|dg_) + association_level. browser_surface_id = this browser. multi-algorithm confirm/contradict; never_digest ≠ unused — UA/IP/JA*/L0 are reference_aux (low weight / contradict). product.env = OS/browser names+versions, IP, CF (context only).",
    });
    // iss/50 C3 alias + R5/R6 bands + iss/49 config snapshot (top-level product keys)
    if let Some(obj) = product.as_object_mut() {
        let mut homo = obj.get("homogenization").cloned().unwrap_or(json!(null));
        if homo.is_null() {
            let env_class = fields
                .get("env_class")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let env_signals = fields.get("env_signals").cloned().unwrap_or_else(|| {
                json!({
                    "bucket_member_count": fields.get("bucket_member_count").cloned().unwrap_or(json!(0)),
                    "supercluster": fields.get("supercluster").cloned().unwrap_or(json!(false)),
                })
            });
            let cfg = crate::soft_v2::SoftConfig::default();
            homo = crate::soft_v2::homogenization_product_signal(
                env_class,
                &env_signals,
                cfg.hot_bucket_threshold,
                cfg.p1_window_ms,
            );
            obj.insert("homogenization".into(), homo.clone());
        }
        let env_h = homo
            .get("environment_homogeneity")
            .cloned()
            .unwrap_or(json!(null));
        obj.insert("environment_homogeneity".into(), env_h);
        obj.insert(
            "rpa_bands".into(),
            rpa.get("rpa_bands")
                .cloned()
                .unwrap_or_else(|| crate::rpa_bands::rpa_behavior_bands(fields)),
        );
        // iss/50 R7: behavior self-sim vec32 for SDK offline compare
        let mut fields_for_profile = fields.clone();
        if let Some(fo) = fields_for_profile.as_object_mut() {
            if let Some(rb) = obj.get("rpa_bands") {
                fo.insert("rpa_bands".into(), rb.clone());
            }
        }
        obj.insert(
            "behavior_self_sim".into(),
            crate::behavior_self_sim::behavior_profile_vec32(&fields_for_profile),
        );
        // iss/47 P2–P4: claim-obs graph + shadow + canary (never production gate)
        let cos = crate::claim_obs_graph::claim_obs_shadow_canary(fields, &rpa, &os);
        obj.insert(
            "claim_obs_graph".into(),
            cos.get("claim_obs_graph").cloned().unwrap_or(json!({})),
        );
        obj.insert(
            "shadow_score".into(),
            cos.get("shadow_score").cloned().unwrap_or(json!({})),
        );
        obj.insert(
            "canary_model".into(),
            cos.get("canary_model").cloned().unwrap_or(json!({})),
        );
        // iss/45 B8: JA4H-lite product surface (order digest only)
        obj.insert(
            "ja4h_lite".into(),
            crate::ja4h_lite::ja4h_lite_product(fields),
        );
        // iss/53–54: multi-path silicon fusion + census + F3/F5/F6 honesty surfaces
        let fusion = crate::hw_silicon_fusion::fuse_silicon_channels(fields);
        let fw = crate::hw_fusion_weights::fusion_weights_status();
        let en = crate::hw_engine_norm::engine_norm_status();
        obj.insert(
            "hw_silicon".into(),
            json!({
                "algo": crate::hw_silicon_fusion::HW_SILICON_FUSION_ALGO,
                "grade": fusion.get("grade"),
                "same_sku_separability": fusion.get("same_sku_separability"),
                "hw_silicon_fine": fusion.get("hw_silicon_fine"),
                "hw_silicon_fusion": fusion.get("hw_silicon_fusion"),
                "lane_c": fusion.get("lane_c"),
                "lane_s": fusion.get("lane_s"),
                "channels": fusion.get("channels"),
                "engine_norm": fusion.get("engine_norm"),
                "fusion_weights_source": fusion.get("fusion_weights_source"),
                "fusion_weights_learned": fusion.get("fusion_weights_learned"),
                "promote_to_silicon_uv": false,
            }),
        );
        obj.insert(
            "hw_channel_census".into(),
            crate::hw_channel_census::hw_channel_census(fields),
        );
        obj.insert(
            "hw_anti_collision".into(),
            crate::hw_anti_collision::build_anti_collision_surface(fields),
        );
        obj.insert(
            "hw_channel_drift".into(),
            crate::hw_channel_drift::channel_drift_from_fields(fields),
        );
        obj.insert("silicon_fusion_weights".into(), fw);
        obj.insert("lane_s_engine_norm".into(), en);
        let snap = crate::config_snapshot::current_config_snapshot();
        obj.insert(
            "config_version".into(),
            snap.get("config_version").cloned().unwrap_or(json!(null)),
        );
        obj.insert("config_snapshot".into(), snap);
    }

    // Engine-aware client context + multi-precision residual + specialized probe gaps.
    // Present pack ids from evidence when available (best-effort).
    let mut present_packs: Vec<String> = Vec::new();
    if let Some(ev) = evidence {
        if let Some(arr) = ev
            .get("present_packs")
            .or_else(|| ev.get("pack_ids"))
            .and_then(|v| v.as_array())
        {
            for x in arr {
                if let Some(s) = x.as_str() {
                    present_packs.push(s.to_string());
                }
            }
        }
        // batches may list pack/batch ids
        if let Some(batches) = ev.get("batches").and_then(|v| v.as_array()) {
            for b in batches {
                if let Some(s) = b.get("pack_id").or_else(|| b.get("batch_id")).and_then(|v| v.as_str())
                {
                    present_packs.push(s.to_string());
                }
            }
        }
    }
    if fields
        .get("residual_paths")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty())
        || fields.get("hw_curve_webgl").is_some()
    {
        present_packs.push("B10_hw_curves".into());
    }
    crate::engine_surface::attach_engine_surface_to_product(
        &mut product,
        fields,
        evidence,
        &present_packs,
    );

    // Field utilization + mutual verification + confidence posture + open re-probe gaps.
    let quality = crate::analysis_quality::attach_analysis_quality(
        &mut product,
        fields,
        stack,
        device,
        truth,
        source_conflicts,
        evidence,
    );

    // iss/36 D-2: recommended_action (envelope/stop refined later in evaluate_session).
    let rec = crate::decision::derive_recommended_action(
        &product,
        device,
        bot.verdict.as_str(),
        None,
        None,
    );
    if let Some(obj) = product.as_object_mut() {
        obj.insert("recommended_action".into(), rec);
    }

    let diagnostics = json!({
        "real_band": truth.real_band,
        "authenticity": {
            "real_user_candidate": matches!(
                truth.real_band.as_str(),
                "confirmed_real" | "likely_real"
            ) && bot.verdict != "crawler" && bot.verdict != "bot"
                && !stack.gpu_label_untrusted,
            "real_browser_candidate": truth.has_main_core && bot.verdict != "crawler",
            "multi_source_ok": truth.has_server_side && truth.has_main_core,
            "xsrc_status": truth.xsrc_status,
            "credibility": truth.credibility,
            "stack_class": stack.stack_class,
            "spoof_score": stack.spoof_score,
            "gpu_label_untrusted": stack.gpu_label_untrusted,
            "residual_soft_like": stack.residual_soft_like,
            "claims_host_silicon_recovery": false,
        },
        "bot_algo": bot_algo,
        "link_algo": link_algo,
        "soft_promote": false,
        "coverage_complete": coverage_complete,
        "analysis_terminal": analysis_terminal,
        "decision": {
            "nonsensitive": decision_nonsensitive,
            "sensitive": decision_sensitive,
            "order": ["bot_veto", "has_dv", "soft_corroboration", "sensitivity"],
        },
        "stack_auth": {
            "stack_class": stack.stack_class,
            "spoof_score": stack.spoof_score,
            "vm_hint": fields.get("vm_score"),
        },
        "field_density": density,
        "field_axis_contributions": contributions,
        "ops_fusion_telemetry": ops_telemetry,
        "analysis_quality": quality,
        "mutual_verification": product.get("mutual_verification").cloned(),
        "field_utilization": product.get("field_utilization").cloned(),
        "open_verification_gaps": product.get("open_verification_gaps").cloned(),
        "sub_algorithms": {
            "os_reasons": os.get("reasons").cloned().unwrap_or(json!([])),
            "br_reasons": br.get("reasons").cloned().unwrap_or(json!([])),
            "rpa_reasons": rpa.get("reasons").cloned().unwrap_or(json!([])),
            "field_algorithm_weights": crate::link_or_mint::field_algorithm_weights(),
        },
    });

    let page_rev = fields
        .get("page_rev")
        .and_then(|v| v.as_u64())
        .or_else(|| fields.get("page_rev").and_then(|v| v.as_i64()).map(|i| i as u64))
        .unwrap_or(1);

    let page = page_id.map(|pid| {
        json!({
            "page_id": pid,
            "page_url": page_url,
            "page_rev": page_rev,
            "rpa": rpa,
            "session_id": fields.get("session_id"),
        })
    });

    (product, diagnostics, page)
}

/// Importance weight by matrix role + trust tier (material > conf > veto > diag).
pub fn role_importance(role: &str, trust_tier: &str) -> f64 {
    let role_w = match role {
        "material" => 1.0,
        "veto" => 0.9,
        "conf" => 0.65,
        "diag" => 0.25,
        "exclude" => 0.0,
        _ => 0.4,
    };
    let tier_w = match trust_tier {
        "T0" => 0.55,
        "T1" => 1.0,
        "T2" => 0.85,
        "T3" => 0.55,
        "T4" => 0.7,
        "T5" => 0.6,
        _ => 0.7,
    };
    role_w * tier_w
}

/// Completeness × importance confidence for a product axis (0..1).
pub fn axis_confidence(axis: &str, fields: &Value, score: f64, field_hits: &[String]) -> f64 {
    let cov = load_field_product_matrix()
        .map(|m| m.axis_coverage(axis, fields))
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let (imp_have, imp_need) = load_field_product_matrix()
        .map(|m| {
            let fo = fields.as_object().cloned().unwrap_or_default();
            let mut need = 0.0;
            let mut have = 0.0;
            for row in &m.fields {
                let Some(role) = row.axes.get(axis) else {
                    continue;
                };
                // Density/confidence: material T0–T2; conf/veto T0–T1 only
                if role == "exclude" || role == "diag" {
                    continue;
                }
                let tier = row.trust_tier.as_str();
                let ok = match (role.as_str(), tier) {
                    ("material", "T0" | "T1" | "T2") => true,
                    ("conf" | "veto", "T0" | "T1") => true,
                    _ => false,
                };
                if !ok {
                    continue;
                }
                let w = role_importance(role, &row.trust_tier);
                if w <= 0.0 {
                    continue;
                }
                need += w;
                if field_present(&fo, &row.field) {
                    have += w;
                }
            }
            (have, need)
        })
        .unwrap_or((0.0, 0.0));
    let imp_cov = if imp_need > 0.0 {
        (imp_have / imp_need).clamp(0.0, 1.0)
    } else {
        cov
    };
    let hit_boost = (field_hits.len() as f64 / 12.0).clamp(0.0, 0.15);
    let score = score.clamp(0.0, 1.0);
    // Completeness (importance-weighted) primary; coverage secondary; score tertiary.
    let conf =
        (0.15 + 0.50 * imp_cov + 0.20 * cov + 0.15 * score + hit_boost).clamp(0.0, 1.0);
    (conf * 10000.0).round() / 10000.0
}

fn attach_score_confidence_axis(block: Value, axis: Option<&str>, fields: &Value) -> Value {
    let mut b = block;
    let cov = b
        .get("coverage")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let score = b
        .get("score")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let hits: Vec<String> = b
        .get("field_hits")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let conf = if let Some(ax) = axis {
        axis_confidence(ax, fields, score, &hits)
    } else {
        // Fallback when axis unknown
        ((0.25 + 0.55 * cov + 0.20 * score) * 10000.0).round() / 10000.0
    };
    if let Some(obj) = b.as_object_mut() {
        obj.insert("confidence".into(), json!(conf));
        obj.insert(
            "confidence_algo".into(),
            json!("completeness_x_importance_v1"),
        );
        obj.entry("score_polarity".to_string())
            .or_insert_with(|| json!("higher_is_better"));
    }
    b
}

#[cfg(test)]
mod rpa_v2_tests {
    use super::*;
    use crate::bot::BotScore;
    use serde_json::json;

    fn bot_human() -> BotScore {
        BotScore {
            score: 10,
            verdict: "human".into(),
            flags: vec![],
            family: "none".into(),
            algo: "test".into(),
            details: json!({}),
            robot_name: None,
        }
    }

    #[test]
    fn score_rpa_consumes_features_v2_without_raw_events() {
        let fields = json!({
            "behavior_early_bound": true,
            "behavior_count": 20,
            "page_id": "p_abc",
            "rpa_features_v2": {
                "schema": "rpa_features_v2",
                "segment": {"n_events": 20, "sample_quality": "adequate", "kind": "rolling_window"},
                "features": {
                    "n_events": 20,
                    "type_diversity_n": 4,
                    "input_mouse_entropy": 0.7,
                    "pre_action_move_count": 5,
                    "sensitive_action_zero_move": false
                },
                "privacy": {"raw_events_uploaded": false, "raw_keys_uploaded": false}
            },
            // intentionally no behavior_events array
        });
        let out = score_rpa(&fields, &bot_human(), Some("p_abc"));
        assert!(out.get("score").and_then(|v| v.as_f64()).is_some());
        let reasons = out.get("reasons").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let joined = reasons.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(",");
        assert!(
            joined.contains("rpa_features_v2") || joined.contains("behavior_count"),
            "{joined}"
        );
        assert!(fields.get("behavior_events").is_none());
    }

    #[test]
    fn score_rpa_consumes_coalesced_stats_and_nested_straightness() {
        let fields = json!({
            "behavior_count": 24,
            "pointer_n": 12,
            "rpa_coalesced_stats": {
                "events_with_coalesced": 0,
                "coalesced_total": 0,
                "no_coalesced_observed": true
            },
            "rpa_features_v2": {
                "schema": "rpa_features_v2",
                "segment": {"n_events": 24, "sample_quality": "adequate"},
                "features": {
                    "n_events": 24,
                    "pointer_n": 12,
                    "path_straightness": 0.99,
                    "input_velocity_cv": 0.04
                }
            }
        });
        let out = score_rpa(&fields, &bot_human(), Some("p1"));
        let hits = out
            .get("field_hits")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str())
            .collect::<Vec<_>>();
        let reasons = out
            .get("reasons")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str())
            .collect::<Vec<_>>();
        assert!(
            hits.iter().any(|h| *h == "rpa_coalesced_stats"),
            "coalesced stats must be consumed hits={hits:?}"
        );
        assert!(
            hits.iter().any(|h| *h == "path_straightness")
                || reasons.iter().any(|r| r.contains("path_straight") || r.contains("path_too_straight")),
            "nested path_straightness must be consumed hits={hits:?} reasons={reasons:?}"
        );
        assert!(
            reasons.iter().any(|r| r.contains("coalesced")),
            "coalesced reason missing {reasons:?}"
        );
    }
}
