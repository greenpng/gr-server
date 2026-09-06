//! Stack / spoof authenticity from FE residual materials (demo→v5 integration).
//!
//! Authority is **server-side**: residual hist / soft-renderer labels / high-end claim
//! collision. FE may upload residual_mean and residual_hist but must not embed lab means
//! or digest-hex oracles as decision authority.
//!
//! Never claims host-silicon recovery; never promotes soft to commercial id.

use serde_json::{json, Map, Value};

/// Server residual-cluster algo version (not FE-exposed lab tables).
pub const STACK_AUTH_ALGO: &str = "gr_stack_auth_v3_p0_wire";

/// Soft renderer classes that must never emit collidable commercial `dv_*`.
const SOFT_RENDERER_CLASSES: &[&str] = &["swiftshader", "llvmpipe", "soft_other", "virt_gpu"];

/// Versioned **server-only** residual-mean cluster centers (algo v2).
/// Derived from live residual readback of the fixed FE residual shader across soft vs real
/// stacks — kept server-side so FE cannot embed oracles; unknown if far from both.
const CLUSTER_SOFT_MEAN: f64 = 0.5002774107689955;
const CLUSTER_REAL_MEAN: f64 = 0.5001811906403189;
const CLUSTER_MEAN_EPS: f64 = 4e-5;

#[derive(Debug, Clone)]
pub struct StackAuth {
    pub stack_class: String,
    pub residual_soft_like: Option<bool>,
    pub residual_mean: Option<f64>,
    pub spoof_score: f64,
    pub vm_score: f64,
    pub renderer_class: String,
    pub reasons: Vec<String>,
    pub authenticity_hint: String,
    pub claims_host_silicon_recovery: bool,
    /// When true, WebGL **label** must not strengthen commercial hard id.
    pub gpu_label_untrusted: bool,
    /// Soft render stack (software GL path) — soft digests must not promote / no commercial dv_*.
    pub soft_stack: bool,
}

impl StackAuth {
    pub fn to_value(&self) -> Value {
        json!({
            "stack_class": self.stack_class,
            "residual_soft_like": self.residual_soft_like,
            "residual_mean": self.residual_mean,
            "spoof_score": self.spoof_score,
            "vm_score": self.vm_score,
            "renderer_class": self.renderer_class,
            "reasons": self.reasons,
            "authenticity_hint": self.authenticity_hint,
            "claims_host_silicon_recovery": self.claims_host_silicon_recovery,
            "gpu_label_untrusted": self.gpu_label_untrusted,
            "soft_stack": self.soft_stack,
            "algo": STACK_AUTH_ALGO,
        })
    }
}

fn f64_field(fo: &Map<String, Value>, keys: &[&str]) -> Option<f64> {
    for k in keys {
        if let Some(v) = fo.get(*k) {
            if let Some(n) = v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)) {
                return Some(n);
            }
        }
    }
    None
}

fn str_field(fo: &Map<String, Value>, keys: &[&str]) -> String {
    for k in keys {
        if let Some(s) = fo.get(*k).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    String::new()
}

fn bool_field(fo: &Map<String, Value>, keys: &[&str]) -> Option<bool> {
    for k in keys {
        match fo.get(*k) {
            Some(Value::Bool(b)) => return Some(*b),
            Some(Value::String(s)) => {
                let t = s.trim().to_ascii_lowercase();
                if t == "true" || t == "1" {
                    return Some(true);
                }
                if t == "false" || t == "0" {
                    return Some(false);
                }
            }
            _ => {}
        }
    }
    None
}

fn json_f64_vec(v: Option<&Value>) -> Vec<f64> {
    match v {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_f64().or_else(|| x.as_i64().map(|i| i as f64)))
            .collect(),
        _ => Vec::new(),
    }
}

/// Explicit residual_hist from FE residual probe only.
/// Do **not** reuse `hw_curve_webgl` commercial digest vectors for soft classification —
/// those are always low-variance bin fractions and would false-flag real silicon.
fn residual_hist(fo: &Map<String, Value>) -> Vec<f64> {
    let h = json_f64_vec(fo.get("residual_hist"));
    if h.len() >= 8 {
        return h;
    }
    // B15 cross may upload main_residual_hist
    let h2 = json_f64_vec(fo.get("main_residual_hist"));
    if h2.len() >= 8 {
        return h2;
    }
    Vec::new()
}

pub fn renderer_class_from_label(renderer: &str) -> String {
    let s = renderer.to_ascii_lowercase();
    if s.is_empty() {
        return "unknown".into();
    }
    if s.contains("swiftshader") || s.contains("subzero") {
        return "swiftshader".into();
    }
    if s.contains("llvmpipe") || s.contains("softpipe") {
        return "llvmpipe".into();
    }
    if s.contains("virtio") || s.contains("vmware") || s.contains("virtualbox") {
        return "virt_gpu".into();
    }
    if s.contains("nvidia") || s.contains("geforce") || s.contains("quadro") {
        return "angle_nvidia".into();
    }
    if s.contains("amd") || s.contains("radeon") {
        return "angle_amd".into();
    }
    if s.contains("intel") {
        return "angle_intel".into();
    }
    if s.contains("apple") || s.contains("metal") || s.contains("m1") || s.contains("m2") {
        return "apple_gpu".into();
    }
    if s.contains("adreno") || s.contains("mali") {
        return "mobile_gpu".into();
    }
    if soft_label(renderer) {
        return "soft_other".into();
    }
    "other".into()
}

fn is_high_end(rc: &str) -> bool {
    matches!(
        rc,
        "angle_nvidia" | "angle_amd" | "apple_gpu" | "angle_intel" | "mobile_gpu"
    )
}

fn soft_label(renderer: &str) -> bool {
    let s = renderer.to_ascii_lowercase();
    s.contains("swiftshader")
        || s.contains("llvmpipe")
        || s.contains("softpipe")
        || s.contains("microsoft basic render")
        || s.contains("subzero")
        || s.contains("virtio")
}

pub fn is_soft_renderer_class(rc: &str) -> bool {
    SOFT_RENDERER_CLASSES.contains(&rc)
}

/// Shape features of residual hist for soft vs real without FE lab digests.
/// Soft GL residual often has lower mid-bin variance after normalization and
/// slightly higher edge-mass symmetry from fixed shader paths.
fn residual_shape_soft_like(hist: &[f64]) -> Option<bool> {
    if hist.len() < 8 {
        return None;
    }
    let sum: f64 = hist.iter().map(|x| x.abs()).sum::<f64>().max(1e-12);
    let p: Vec<f64> = hist.iter().map(|x| x.abs() / sum).collect();
    let mean = p.iter().sum::<f64>() / p.len() as f64;
    let var = p.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / p.len() as f64;
    let std = var.sqrt();
    // Edge mass (first+last two bins)
    let edge = p[0] + p[1] + p[p.len() - 1] + p[p.len() - 2];
    // Mid flatness: inverse of mid-bin std
    let mid = &p[2..p.len() - 2];
    let mid_mean = mid.iter().sum::<f64>() / mid.len() as f64;
    let mid_var = mid.iter().map(|x| (x - mid_mean).powi(2)).sum::<f64>() / mid.len() as f64;
    // Soft software paths tend to have very regular residual mass (low mid_var relative to edge).
    // Real GPUs introduce more mid-bin jitter. Thresholds are shape-based, not residual_mean lab anchors.
    if mid_var < 1.5e-4 && edge > 0.08 && std < 0.08 {
        return Some(true);
    }
    if mid_var > 4e-4 || (std > 0.12 && edge < 0.25) {
        return Some(false);
    }
    None
}

/// Infer residual_soft_like from residual materials (server authority).
/// Prefers soft renderer label + residual_hist shape; residual_mean uses **server**
/// versioned cluster centers (not FE-embedded oracles).
pub fn infer_residual_soft_like(
    mean: Option<f64>,
    hist: &[f64],
    fe_flag: Option<bool>,
    soft_lab: bool,
) -> Option<bool> {
    // Soft renderer label is honest software GL — residual path is soft.
    if soft_lab {
        return Some(true);
    }
    if let Some(b) = residual_shape_soft_like(hist) {
        return Some(b);
    }
    // Server residual-mean clusters (v2) — FE must not ship these constants.
    if let Some(m) = mean {
        let ds = (m - CLUSTER_SOFT_MEAN).abs();
        let dr = (m - CLUSTER_REAL_MEAN).abs();
        if ds < dr && ds < CLUSTER_MEAN_EPS {
            return Some(true);
        }
        if dr < ds && dr < CLUSTER_MEAN_EPS {
            return Some(false);
        }
    }
    // FE residual_soft_like only as last-resort hint (field-only after FE cleanup)
    fe_flag
}

/// Back-compat helper used by older unit tests (mean-only).
pub fn infer_residual_soft_like_mean(mean: Option<f64>, fe_flag: Option<bool>) -> Option<bool> {
    infer_residual_soft_like(mean, &[], fe_flag, false)
}

fn push_reason(reasons: &mut Vec<String>, code: &str) {
    if !reasons.iter().any(|r| r == code) {
        reasons.push(code.into());
    }
}

/// iss/opus5 03-P2-6: spoof/vm increments externalized to the signed weights
/// spec. Callers pass the current constant as default; the spec (if adopted)
/// overrides per code. Clamps to [0,1] as before.
fn sa_bump(spoof: &mut f64, vm: &mut f64, code: &str, d_spoof: f64, d_vm: f64) {
    let (ds, dv) = crate::bot_weights::active().stack_inc(code, d_spoof, d_vm);
    *spoof = (*spoof + ds).min(1.0);
    *vm = (*vm + dv).min(1.0);
}

/// Spoof-only variant for scopes where vm_score is declared later.
fn sa_bump_spoof(spoof: &mut f64, code: &str, d_spoof: f64) {
    let mut vm = 0.0_f64;
    sa_bump(spoof, &mut vm, code, d_spoof, 0.0);
}

/// P0-A: act on FE materials previously uploaded but ignored by stack_auth.
fn apply_p0_physical_materials(
    fo: &Map<String, Value>,
    high: bool,
    soft_lab: bool,
    spoof_score: &mut f64,
    vm_score: &mut f64,
    reasons: &mut Vec<String>,
) {
    // Challenge noise / over-stable (B20)
    let cv = f64_field(
        fo,
        &["challenge_avg_cv", "session_residual_cv", "challenge_cv"],
    );
    let unique_n = f64_field(fo, &["challenge_unique_digest_n"]);
    if let Some(c) = cv {
        if c > 0.05 {
            sa_bump(spoof_score, vm_score, "noise_injection_likely", 0.18, 0.00);
            push_reason(reasons, "noise_injection_likely");
        } else if c < 1e-9 && high {
            // Too stable with high-end label can be replay/constant spoof
            if unique_n == Some(1.0) {
                sa_bump(spoof_score, vm_score, "too_stable_with_label_conflict", 0.12, 0.00);
                push_reason(reasons, "too_stable_with_label_conflict");
            }
        }
        push_reason(reasons, "challenge_cv_present");
    }
    if unique_n == Some(1.0) && high && soft_lab {
        sa_bump(spoof_score, vm_score, "challenge_unique_n1_soft_label", 0.10, 0.00);
        push_reason(reasons, "challenge_unique_n1_soft_label");
    }

    // Residual hist alternate keys (main_residual_hist from B15)
    let alt_hist = residual_hist(fo);
    if alt_hist.is_empty() {
        let h = json_f64_vec(fo.get("main_residual_hist"));
        if h.len() >= 8 {
            if residual_shape_soft_like(&h) == Some(true) && high {
                sa_bump(spoof_score, vm_score, "main_residual_hist_soft_shape", 0.15, 0.00);
                push_reason(reasons, "main_residual_hist_soft_shape");
            } else {
                push_reason(reasons, "main_residual_hist_present");
            }
        }
    }

    // GPU staircase / slope (B22) — soft stacks often super-linear wall slope
    let slope = f64_field(fo, &["gpu_slope_wall", "gpu_slope"]);
    let stair_n = fo
        .get("gpu_wall_staircase")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .or_else(|| f64_field(fo, &["gpu_staircase_points"]).map(|n| n as usize));
    if stair_n.unwrap_or(0) >= 4 {
        push_reason(reasons, "gpu_staircase_present");
        if let Some(s) = slope {
            // Very high wall slope under high-end label → software-like fill
            if high && s > 1e-4 {
                sa_bump(spoof_score, vm_score, "gpu_slope_soft_like", 0.14, 0.08);
                push_reason(reasons, "gpu_slope_soft_like");
            } else if s > 0.0 && s < 1e-6 {
                push_reason(reasons, "gpu_slope_hardware_like");
            }
        }
    }
    if fo.get("gpu_bandwidth_ladder").is_some() {
        push_reason(reasons, "gpu_bandwidth_ladder_present");
    }
    if fo.get("gpu_r2_wall").and_then(|v| v.as_f64()).is_some()
        || fo.get("gpu_disjoint_rate").is_some()
        || fo.get("timer_query_available").is_some()
    {
        push_reason(reasons, "h01_timer_meta_present");
    }

    // Precision matrix + gl max (B17) — claim consistency
    let max_tex = f64_field(
        fo,
        &[
            "gl_max_texture_size",
            "webgl_max_texture",
            "max_texture_size",
            "claimed_max_tex",
        ],
    );
    let actual_tex = f64_field(fo, &["actual_max_tex", "caps_actual_max_tex"]);
    if let (Some(claim), Some(actual)) = (max_tex, actual_tex) {
        if claim >= 16384.0 && actual > 0.0 && actual < claim * 0.5 {
            sa_bump(spoof_score, vm_score, "caps_claim_vs_actual", 0.28, 0.00);
            push_reason(reasons, "caps_claim_vs_actual");
        }
    }
    if fo.get("gl_precision_matrix").is_some() || fo.get("precision_matrix").is_some() {
        push_reason(reasons, "gl_precision_matrix_present");
    }
    if let Some(ulp) = f64_field(fo, &["shader_ulp_max", "ulp_max"]) {
        // Software libm often shows higher ULP error on transcendentals
        if ulp > 8.0 && high {
            sa_bump(spoof_score, vm_score, "precision_claim_vs_behavior", 0.16, 0.00);
            push_reason(reasons, "precision_claim_vs_behavior");
        } else {
            push_reason(reasons, "shader_ulp_present");
        }
    }
    if bool_field(fo, &["mediump_diverges"]).unwrap_or(false) {
        push_reason(reasons, "mediump_divergence_present");
    }

    // Native integrity (B23)
    if let Some(r) = f64_field(fo, &["native_integrity_ratio"]) {
        if r < 0.7 {
            sa_bump(spoof_score, vm_score, "native_integrity_low", 0.22, 0.00);
            push_reason(reasons, "native_integrity_low");
        } else {
            push_reason(reasons, "native_integrity_ok");
        }
    }
    if let Some(n) = f64_field(fo, &["non_native_count"]) {
        if n >= 3.0 {
            sa_bump(spoof_score, vm_score, "non_native_count_high", 0.12, 0.00);
            push_reason(reasons, "non_native_count_high");
        }
    }

    // Clock / rAF (B25)
    if let Some(res) = f64_field(fo, &["perf_now_resolution_ms", "clock_resolution_ms"]) {
        if res >= 0.1 {
            sa_bump(spoof_score, vm_score, "clock_coarse_resolution", 0.00, 0.12);
            push_reason(reasons, "clock_coarse_resolution");
        } else {
            push_reason(reasons, "clock_fine_resolution");
        }
    }
    if let Some(j) = f64_field(fo, &["raf_jitter_cv", "raf_cv"]) {
        if j > 0.35 {
            sa_bump(spoof_score, vm_score, "raf_jitter_high", 0.00, 0.08);
            push_reason(reasons, "raf_jitter_high");
        } else {
            push_reason(reasons, "raf_jitter_present");
        }
    }
    if let Some(skew) = f64_field(fo, &["date_now_skew_ms"]) {
        if skew.abs() > 50.0 {
            sa_bump(spoof_score, vm_score, "date_now_skew_high", 0.00, 0.10);
            push_reason(reasons, "date_now_skew_high");
        }
    }

    // EME matrix (B19)
    if let Some(obj) = fo.get("eme_systems").and_then(|v| v.as_object()) {
        let supported = obj
            .values()
            .filter(|v| v.as_str() == Some("supported"))
            .count();
        if supported == 0 && obj.len() >= 2 {
            sa_bump(spoof_score, vm_score, "eme_all_unsupported", 0.00, 0.10);
            push_reason(reasons, "eme_all_unsupported");
        } else if supported > 0 {
            push_reason(reasons, "eme_matrix_present");
        }
    }
    if fo.get("codec_matrix").is_some() || fo.get("canplay_matrix").is_some() {
        push_reason(reasons, "codec_matrix_present");
    }
    // H10 object depth
    if let Some(score) = f64_field(fo, &["codec_support_score"]) {
        let n = f64_field(fo, &["codec_matrix_n"]).unwrap_or(0.0);
        if bool_field(fo, &["codec_virt_hint"]) == Some(true) || (n >= 8.0 && score <= 0.0) {
            sa_bump(spoof_score, vm_score, "codec_matrix_empty_virt_hint", 0.00, 0.12);
            push_reason(reasons, "codec_matrix_empty_virt_hint");
        } else if score >= 6.0 {
            push_reason(reasons, "codec_matrix_rich");
        }
    }
    if f64_field(fo, &["eme_supported_n"]).unwrap_or(0.0) >= 1.0 {
        push_reason(reasons, "eme_supported_n_present");
    } else if f64_field(fo, &["eme_unsupported_n"]).unwrap_or(0.0) >= 3.0 {
        sa_bump(spoof_score, vm_score, "eme_all_unsupported_depth", 0.00, 0.08);
        push_reason(reasons, "eme_all_unsupported_depth");
    }
    if bool_field(fo, &["eme_widevine"]) == Some(true) {
        push_reason(reasons, "eme_widevine_present");
    }
    if f64_field(fo, &["codec_probably_n"]).unwrap_or(0.0) >= 3.0 {
        push_reason(reasons, "codec_probably_dense");
    }

    // H01 GPU-ns path: wall/gpu consistency
    if fo.get("gpu_ns_staircase").is_some() || fo.get("gpu_ns_median").is_some() {
        push_reason(reasons, "gpu_ns_materials_present");
        if let (Some(wall), Some(gpu_ns)) = (
            f64_field(fo, &["gpu_wall_median_ms"]),
            f64_field(fo, &["gpu_ns_median"]),
        ) {
            // gpu_ns in nanoseconds; wall in ms → convert
            let gpu_ms = gpu_ns / 1_000_000.0;
            if wall > 0.0 && gpu_ms > 0.0 {
                let ratio = gpu_ms / wall;
                // Real GPU: GPU-ns << wall; ratio often << 1. Fake one-sided curves break this.
                if ratio > 2.0 {
                    sa_bump(spoof_score, vm_score, "gpu_ns_wall_ratio_inconsistent_v1", 0.15, 0.00);
                    push_reason(reasons, "gpu_ns_wall_ratio_inconsistent");
                } else if ratio < 0.95 {
                    push_reason(reasons, "gpu_ns_wall_ratio_hardware_like");
                }
            }
        }
        if let Some(dr) = f64_field(fo, &["gpu_disjoint_rate"]) {
            if dr > 0.5 {
                sa_bump(spoof_score, vm_score, "gpu_disjoint_rate_high", 0.00, 0.10);
                push_reason(reasons, "gpu_disjoint_rate_high");
            }
        }
    } else if bool_field(fo, &["timer_query_available"]) == Some(false) {
        push_reason(reasons, "timer_query_unavailable_honest");
    }

    // Signed challenge validation (when fields present)
    if fo.get("challenge_seed_sig").is_some() {
        let sid = fo
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let now = f64_field(fo, &["collected_at", "now_ms"])
            .map(|n| n as u64)
            .unwrap_or(0);
        let (ok, mut ch_rs) = crate::challenge_pow::evaluate_challenge_fields(
            sid,
            fo,
            crate::challenge_pow::DEFAULT_CHALLENGE_SECRET,
            now,
        );
        if ok {
            push_reason(reasons, "challenge_seed_valid");
        } else {
            sa_bump(spoof_score, vm_score, "challenge_seed_invalid", 0.20, 0.0);
            for r in ch_rs.drain(..) {
                push_reason(reasons, &r);
            }
        }
    }

    // Peripheral hedge
    if fo.get("permissions_matrix").is_some() || fo.get("media_devices_count").is_some() {
        push_reason(reasons, "peripheral_permissions_present");
    }
    if fo.get("battery_level").is_some() || fo.get("sensor_accel_present").is_some() {
        push_reason(reasons, "peripheral_sensors_present");
    }
    // CPU cache ladder
    if fo.get("cpu_cache_ladder").is_some() || fo.get("cpu_cache_knee_bytes").is_some() {
        push_reason(reasons, "cpu_cache_ladder_present");
    }

    // Agent parity / automation globals (B26)
    if let Some(n) = f64_field(fo, &["agent_automation_globals_n"]) {
        if n >= 1.0 {
            sa_bump(spoof_score, vm_score, "agent_parity_automation_globals", 0.25, 0.00);
            push_reason(reasons, "agent_parity_automation_globals");
        } else {
            push_reason(reasons, "agent_parity_clean");
        }
    } else if fo.get("agent_parity_hash").is_some() {
        push_reason(reasons, "agent_parity_present");
    }

    // Storage privacy (B27)
    if fo.get("storage_quota").is_some() || fo.get("privacy_storage_score").is_some() {
        push_reason(reasons, "storage_privacy_present");
        if bool_field(fo, &["local_storage"]) == Some(false)
            && bool_field(fo, &["cookie_enabled"]) == Some(false)
        {
            sa_bump(spoof_score, vm_score, "storage_heavily_restricted", 0.00, 0.05);
            push_reason(reasons, "storage_heavily_restricted");
        }
    }

    // DomRect / perf timeline (B35)
    if fo.get("dom_rect_hash").is_some() || fo.get("perf_timeline_hash").is_some() {
        push_reason(reasons, "dom_perf_present");
    }

    // Layer divergence H14 (B15)
    if let Some(score) = f64_field(fo, &["layer_divergence_score"]) {
        push_reason(reasons, "layer_divergence_present");
        if score < 0.7 {
            sa_bump(spoof_score, vm_score, "layer_divergence_high", 0.12, 0.00);
            push_reason(reasons, "layer_divergence_high");
        }
    } else if bool_field(fo, &["layer_divergence_match"]) == Some(false) {
        sa_bump(spoof_score, vm_score, "layer_divergence_mismatch", 0.10, 0.00);
        push_reason(reasons, "layer_divergence_mismatch");
    }

    // WebGPU deepen (B18)
    if fo.get("webgpu_limits_hash").is_some() || fo.get("webgpu_dual_adapter_diff").is_some() {
        push_reason(reasons, "webgpu_deep_present");
        if bool_field(fo, &["webgpu_dual_adapter_diff"]) == Some(true) {
            push_reason(reasons, "webgpu_dual_adapter");
        }
    }

    // H04 raster
    if fo.get("raster_edge_hash").is_some() || fo.get("raster_edge_curve").is_some() {
        push_reason(reasons, "raster_msaa_present");
    }
    // H09 thermal lite
    if let Some(slope) = f64_field(fo, &["thermal_cpu_slope"]) {
        push_reason(reasons, "thermal_drift_present");
        if slope > 0.35 {
            sa_bump(spoof_score, vm_score, "thermal_cpu_slope_high", 0.00, 0.08);
            push_reason(reasons, "thermal_cpu_slope_high");
        }
    }
    // H15 neg dictionary (server derive + FE hits)
    {
        let hits = crate::neg_dict::derive_neg_dict_hits(fo);
        if !hits.is_empty() {
            let boost = crate::neg_dict::neg_dict_spoof_boost(&hits);
            *spoof_score = (*spoof_score + boost).min(1.0);
            push_reason(reasons, &format!("neg_dict_hits={}", hits.len()));
            for h in hits.iter().take(6) {
                push_reason(reasons, &format!("neg:{h}"));
            }
        }
    }
    // D08 mem pressure
    if fo.get("mem_alloc_ladder").is_some() || fo.get("mem_alloc_max_mb").is_some() {
        push_reason(reasons, "mem_pressure_present");
    }
    // D21 websocket
    if fo.get("ws_handshake_ms").is_some() || bool_field(fo, &["websocket_present"]).is_some() {
        push_reason(reasons, "websocket_fp_present");
        if bool_field(fo, &["ws_constructor_native"]) == Some(false) {
            sa_bump(spoof_score, vm_score, "ws_constructor_not_native", 0.10, 0.00);
            push_reason(reasons, "ws_constructor_not_native");
        }
    }
    // D35 HID/gamepad
    if fo.get("hid_surface_score").is_some() || fo.get("gamepad_count").is_some() {
        push_reason(reasons, "hid_gamepad_present");
    }

    // H01 GPU-ns slope / wall ratio / depth metrics (real GPU path)
    let has_gpu_ns_depth = fo.get("gpu_slope_ns").is_some()
        || fo.get("gpu_ns_wall_ratio_median").is_some()
        || fo.get("gpu_r2_ns").is_some()
        || fo.get("gpu_ns_depth_score").is_some()
        || fo.get("gpu_ns_monotonic_ratio").is_some()
        || fo.get("gpu_ns_staircase").is_some()
        || fo.get("gpu_ns_median").is_some();
    if has_gpu_ns_depth {
        if fo.get("gpu_slope_ns").is_some() || fo.get("gpu_ns_wall_ratio_median").is_some() {
            push_reason(reasons, "gpu_ns_extended_present");
        }
        if let Some(ratio) = f64_field(fo, &["gpu_ns_wall_ratio_median"]) {
            // Real discrete GPU: GPU-ns << wall → ratio usually << 1
            if ratio > 1.5 {
                sa_bump(spoof_score, vm_score, "gpu_ns_wall_ratio_inconsistent_v2", 0.12, 0.00);
                push_reason(reasons, "gpu_ns_wall_ratio_inconsistent");
            } else if ratio > 0.0 && ratio < 0.8 {
                push_reason(reasons, "gpu_ns_wall_ratio_hardware_like");
            }
        }
        if f64_field(fo, &["gpu_ns_points"]).unwrap_or(0.0) >= 4.0 {
            push_reason(reasons, "gpu_ns_dense_staircase");
        }
        // H01 depth metrics
        if let Some(r2) = f64_field(fo, &["gpu_r2_ns"]) {
            if r2 < 0.3 && f64_field(fo, &["gpu_ns_points"]).unwrap_or(0.0) >= 4.0 {
                sa_bump(spoof_score, vm_score, "gpu_r2_ns_low", 0.10, 0.00);
                push_reason(reasons, "gpu_r2_ns_low");
            } else if r2 >= 0.85 {
                push_reason(reasons, "gpu_r2_ns_strong");
            }
        }
        if let Some(m) = f64_field(fo, &["gpu_ns_monotonic_ratio"]) {
            if m < 0.5 && f64_field(fo, &["gpu_ns_points"]).unwrap_or(0.0) >= 3.0 {
                sa_bump(spoof_score, vm_score, "gpu_ns_non_monotonic", 0.08, 0.00);
                push_reason(reasons, "gpu_ns_non_monotonic");
            } else if m >= 0.75 {
                push_reason(reasons, "gpu_ns_monotonic");
            }
        }
        if let Some(d) = f64_field(fo, &["gpu_ns_depth_score"]) {
            if d >= 0.7 {
                push_reason(reasons, "gpu_ns_depth_strong");
            } else if d > 0.0 && d < 0.35 {
                sa_bump(spoof_score, vm_score, "gpu_ns_depth_weak", 0.06, 0.00);
                push_reason(reasons, "gpu_ns_depth_weak");
            }
        }
    }

    // H09 full thermal
    if fo.get("thermal_cpu_slope_full").is_some() || fo.get("thermal_bursts_full").is_some() {
        push_reason(reasons, "thermal_full_present");
        if f64_field(fo, &["thermal_cpu_slope_full"]).unwrap_or(0.0) > 0.4 {
            sa_bump(spoof_score, vm_score, "thermal_full_slope_high", 0.00, 0.10);
            push_reason(reasons, "thermal_full_slope_high");
        }
    }

    // Errors engine / speech / display
    if fo.get("errors_engine_hash").is_some() {
        push_reason(reasons, "errors_engine_present");
    }
    if fo.get("speech_voices_hash").is_some() || fo.get("speech_voices_count").is_some() {
        push_reason(reasons, "speech_voices_present");
    }
    if fo.get("display_mq_hash").is_some() || fo.get("hdr_likely").is_some() {
        push_reason(reasons, "display_hdr_present");
    }
    // D03 audio deep (demo method port)
    if fo.get("audio_deep_hash").is_some() || fo.get("audio_deep_moments").is_some() {
        push_reason(reasons, "audio_deep_present");
        if fo.get("audio_deep_peak_bins").is_some() {
            push_reason(reasons, "audio_deep_peak_bins_present");
        }
    }
    // Real GPU-ns hardware path flag
    if bool_field(fo, &["gpu_ns_hardware_path"]) == Some(true)
        || f64_field(fo, &["gpu_ns_points"]).unwrap_or(0.0) >= 4.0
    {
        push_reason(reasons, "gpu_ns_hardware_path");
    }

    // Protocol / Pingora side-channel materials → used by authenticity (not just stored)
    if fo.get("ja4").is_some() || fo.get("tls_ja4").is_some() {
        push_reason(reasons, "protocol_ja4_present");
    }
    if fo.get("protocol_engine").is_some() {
        push_reason(reasons, "protocol_engine_present");
    }
    if fo.get("h2_fingerprint").is_some() || fo.get("h2_settings_available").is_some() {
        push_reason(reasons, "h2_fingerprint_present");
    }
    if fo.get("quic_tls_ja4").is_some() || bool_field(fo, &["quic_listen_present"]) == Some(true) {
        push_reason(reasons, "quic_side_present");
        if let (Some(tj), Some(qj)) = (
            fo.get("ja4").and_then(|v| v.as_str()),
            fo.get("quic_tls_ja4").and_then(|v| v.as_str()),
        ) {
            if !tj.is_empty() && !qj.is_empty() {
                if tj == qj {
                    push_reason(reasons, "protocol_tcp_quic_agree");
                } else {
                    sa_bump(spoof_score, vm_score, "protocol_tcp_quic_mismatch", 0.08, 0.00);
                    push_reason(reasons, "protocol_tcp_quic_mismatch");
                }
            }
        }
    }
    if bool_field(fo, &["tcp_syn_present"]) == Some(true) {
        push_reason(reasons, "tcp_syn_side_present");
        // Very low TTL can indicate remote DC / tunnel heuristics (soft only)
        if let Some(ttl) = f64_field(fo, &["tcp_syn_ttl"]) {
            if ttl > 0.0 && ttl < 40.0 {
                push_reason(reasons, "tcp_syn_ttl_low");
            }
        }
    }
    if bool_field(fo, &["webrtc_side_present"]) == Some(true) {
        push_reason(reasons, "webrtc_side_present");
    }
    if fo.get("challenge_eval").is_some() {
        push_reason(reasons, "challenge_eval_present");
        let ch_ok = fo
            .get("challenge_eval")
            .and_then(|v| v.get("ok"))
            .and_then(|v| v.as_bool());
        if ch_ok == Some(false) {
            sa_bump(spoof_score, vm_score, "challenge_eval_failed", 0.10, 0.00);
            push_reason(reasons, "challenge_eval_failed");
        } else if ch_ok == Some(true) {
            push_reason(reasons, "challenge_eval_ok");
        }
    }

    // --- Blackhole close-up: sibling digests FE already uploads ---
    if bool_field(fo, &["proxy_protocol_present"]) == Some(true) {
        push_reason(reasons, "proxy_protocol_edge_present");
    }
    if let Some(hit) = f64_field(fo, &["font_token_hit_count"]) {
        if hit >= 3.0 {
            push_reason(reasons, "font_token_dense");
        } else if hit > 0.0 {
            push_reason(reasons, "font_token_present");
        }
    } else if fo.get("font_present_sample").is_some() {
        push_reason(reasons, "font_present_sample");
    }
    if fo.get("codec_matrix_hash").is_some() {
        push_reason(reasons, "codec_matrix_hash_present");
    }
    if bool_field(fo, &["canvas_geometry_stable"]) == Some(false) {
        sa_bump(spoof_score, vm_score, "canvas_geometry_unstable", 0.06, 0.00);
        push_reason(reasons, "canvas_geometry_unstable");
    } else if fo.get("canvas_geometry_mean").is_some()
        || bool_field(fo, &["canvas_geometry_stable"]) == Some(true)
    {
        push_reason(reasons, "canvas_geometry_depth");
    }
    if let (Some(hit), Some(keys)) = (
        f64_field(fo, &["agent_parity_hit_n"]),
        f64_field(fo, &["agent_parity_keys_n"]),
    ) {
        if keys >= 8.0 {
            let ratio = hit / keys;
            if ratio < 0.5 {
                sa_bump(spoof_score, vm_score, "agent_parity_ratio_low", 0.12, 0.00);
                push_reason(reasons, "agent_parity_ratio_low");
            } else {
                push_reason(reasons, "agent_parity_ratio_ok");
            }
        }
    }
    if fo.get("permissions_hash").is_some() || fo.get("permissions_granted_n").is_some() {
        push_reason(reasons, "permissions_digest_present");
    }
    if bool_field(fo, &["battery_charging"]).is_some() {
        push_reason(reasons, "battery_charge_state_present");
    }
    if fo.get("media_devices_kinds").is_some() || fo.get("media_devices_enumerate").is_some() {
        push_reason(reasons, "media_devices_shape_present");
    }
    if let Some(d) = f64_field(fo, &["residual_challenge_delta"]) {
        if d.abs() > 0.05 {
            sa_bump(spoof_score, vm_score, "residual_challenge_delta_high", 0.12, 0.00);
            push_reason(reasons, "residual_challenge_delta_high");
        } else {
            push_reason(reasons, "residual_challenge_delta_ok");
        }
    }
    if fo.get("main_residual_mean").is_some() || fo.get("live_residual_mean").is_some() {
        push_reason(reasons, "residual_mean_present");
    }
    if fo.get("challenge_audio_mean").is_some()
        || fo.get("challenge_cpu_acc").is_some()
        || fo.get("challenge_alt_mean").is_some()
    {
        push_reason(reasons, "pohw_numeric_surfaces");
    }
    if fo.get("gpu_roundtrip_ladder").is_some() || fo.get("gpu_intercept_wall").is_some() {
        push_reason(reasons, "gpu_roundtrip_present");
    }
    if let Some(g) = f64_field(fo, &["caps_claim_vs_actual_gap"]) {
        if g > 0.5 {
            sa_bump(spoof_score, vm_score, "caps_claim_vs_actual_gap_high", 0.18, 0.00);
            push_reason(reasons, "caps_claim_vs_actual_gap_high");
        } else if g > 0.0 {
            push_reason(reasons, "caps_claim_gap_present");
        }
    }
    if let Some(m) = f64_field(fo, &["raf_mean_ms"]) {
        if m > 40.0 {
            sa_bump(spoof_score, vm_score, "raf_mean_coarse", 0.00, 0.08);
            push_reason(reasons, "raf_mean_coarse");
        } else if m > 0.0 {
            push_reason(reasons, "raf_mean_present");
        }
    }
    if fo.get("dom_rect").is_some() || bool_field(fo, &["dom_rect_subpixel"]).is_some() {
        push_reason(reasons, "dom_rect_depth_present");
    }
    if bool_field(fo, &["storage_persisted"]).is_some()
        || bool_field(fo, &["caches_api"]).is_some()
        || fo.get("storage_usage").is_some()
    {
        push_reason(reasons, "storage_privacy_depth");
    }
    if let Some(s) = f64_field(fo, &["thermal_gpu_slope_full"]) {
        if s > 0.4 {
            sa_bump(spoof_score, vm_score, "thermal_gpu_slope_full_high", 0.00, 0.08);
            push_reason(reasons, "thermal_gpu_slope_full_high");
        } else {
            push_reason(reasons, "thermal_gpu_full_present");
        }
    }
    if fo.get("css_supports_ok_count").is_some() || fo.get("css_supports_total").is_some() {
        push_reason(reasons, "css_supports_volume");
    }
    if fo.get("media_query_true_count").is_some() {
        push_reason(reasons, "media_query_volume");
    }
    if fo.get("gl_max_vertex_attribs").is_some()
        || fo.get("gl_max_texture_image_units").is_some()
        || fo.get("gl_max_renderbuffer").is_some()
    {
        push_reason(reasons, "gl_caps_depth_present");
    }
    if fo.get("webrtc_host_ips").is_some() || fo.get("webrtc_host_candidate").is_some() {
        push_reason(reasons, "webrtc_host_list_present");
    }
    if fo.get("perf_entry_counts").is_some()
        || fo.get("perf_nav_timing").is_some()
        || fo.get("perf_ttfb_ms").is_some()
    {
        push_reason(reasons, "perf_timeline_depth");
    }
    if bool_field(fo, &["sensor_gyro_present"]) == Some(true)
        || bool_field(fo, &["sensor_orient_present"]) == Some(true)
    {
        push_reason(reasons, "sensor_orient_gyro_present");
    }

    // N4 selective blackhole
    if let (Some(ok), Some(total)) = (
        f64_field(fo, &["api_flags_ok_count"]),
        f64_field(fo, &["api_flags_total"]),
    ) {
        if total >= 20.0 {
            let ratio = ok / total;
            if ratio < 0.25 {
                sa_bump(spoof_score, vm_score, "api_flags_sparse", 0.08, 0.00);
                push_reason(reasons, "api_flags_sparse");
            } else if ratio >= 0.6 {
                push_reason(reasons, "api_flags_dense");
            }
        }
    }
    if bool_field(fo, &["caps_internal_inconsistent"]) == Some(true) {
        sa_bump(spoof_score, vm_score, "caps_internal_inconsistent", 0.14, 0.00);
        push_reason(reasons, "caps_internal_inconsistent");
    }
    if fo.get("caps_probe_results").is_some() {
        push_reason(reasons, "caps_probe_results_present");
    }
    if fo.get("cross_canvas_mean").is_some() || fo.get("cross_audio_mean").is_some() {
        push_reason(reasons, "cross_context_means_present");
    }
    if let (Some(h), Some(m)) = (
        f64_field(fo, &["highp_mean"]),
        f64_field(fo, &["mediump_mean"]),
    ) {
        if h > 0.0 && m > 0.0 && (h - m).abs() / h.max(1e-9) > 0.5 {
            push_reason(reasons, "mediump_highp_divergence");
        }
    }
    if fo.get("automation_globals").is_some() {
        if let Some(Value::Array(a)) = fo.get("automation_globals") {
            if !a.is_empty() {
                sa_bump(spoof_score, vm_score, "automation_globals_nonempty", 0.18, 0.00);
                push_reason(reasons, "automation_globals_nonempty");
            }
        }
    }
    if fo.get("quic_fp").is_some() || fo.get("quic_fp_hash").is_some() {
        push_reason(reasons, "quic_fp_present");
    }

    // Remaining blackhole batch
    if fo.get("challenge_seed_digest").is_some() {
        push_reason(reasons, "challenge_seed_digest_present");
    }
    if fo.get("permission_notification").is_some() || fo.get("permissions_prompt_n").is_some() {
        push_reason(reasons, "permission_notification_present");
    }
    if fo.get("canplay_av1").is_some() || fo.get("canplay_hevc").is_some() {
        push_reason(reasons, "canplay_modern_codecs");
    }
    if let Some(empty) = f64_field(fo, &["codec_empty_n"]) {
        let total = f64_field(fo, &["codec_matrix_n"]).unwrap_or(0.0);
        if total >= 8.0 && empty / total >= 0.85 {
            sa_bump(spoof_score, vm_score, "codec_mostly_empty", 0.00, 0.08);
            push_reason(reasons, "codec_mostly_empty");
        }
    }
    if bool_field(fo, &["eme_clearkey"]) == Some(true) {
        push_reason(reasons, "eme_clearkey_present");
    }
    if fo.get("font_token_checked").is_some() {
        push_reason(reasons, "font_token_checked_present");
    }
    if fo.get("material_keys").is_some() || fo.get("material_consistency").is_some() {
        push_reason(reasons, "material_meta_present");
    }
    if fo.get("media_prefers_color_scheme").is_some()
        || fo.get("media_prefers_reduced_motion").is_some()
    {
        push_reason(reasons, "media_prefers_present");
    }
    if fo.get("orientation_type").is_some() || fo.get("orientation_angle").is_some() {
        push_reason(reasons, "orientation_present");
    }
    if let (Some(m), Some(s)) = (
        f64_field(fo, &["main_residual_mean"]),
        f64_field(fo, &["sandbox_residual_mean"]),
    ) {
        if (m - s).abs() > 0.02 {
            sa_bump(spoof_score, vm_score, "sandbox_main_residual_mismatch", 0.10, 0.00);
            push_reason(reasons, "sandbox_main_residual_mismatch");
        } else {
            push_reason(reasons, "sandbox_main_residual_ok");
        }
    } else if fo.get("sandbox_residual_mean").is_some() {
        push_reason(reasons, "sandbox_residual_mean_present");
    }
    if fo.get("screen_pixel_depth").is_some() || fo.get("screen_color_depth").is_some() {
        push_reason(reasons, "screen_depth_present");
        if f64_field(fo, &["screen_color_depth"]).unwrap_or(24.0) < 16.0 {
            sa_bump(spoof_score, vm_score, "screen_color_depth_low", 0.05, 0.00);
            push_reason(reasons, "screen_color_depth_low");
        }
    }
    if fo.get("session_residual_repeat").is_some() {
        push_reason(reasons, "session_residual_repeat_present");
    }
    if let Some(cv) = f64_field(fo, &["thermal_cpu_cv"]) {
        if cv > 0.5 {
            sa_bump(spoof_score, vm_score, "thermal_cpu_cv_high", 0.00, 0.05);
            push_reason(reasons, "thermal_cpu_cv_high");
        } else {
            push_reason(reasons, "thermal_cpu_cv_present");
        }
    }
    if fo.get("cookie_count").is_some() {
        push_reason(reasons, "cookie_count_present");
    }
    if fo.get("connection").is_some() {
        push_reason(reasons, "network_connection_present");
    }
    if bool_field(fo, &["device_motion"]) == Some(true) {
        push_reason(reasons, "device_motion_present");
    }
    if fo.get("device_orientation_alpha").is_some()
        || fo.get("device_motion_accel_x").is_some()
        || bool_field(fo, &["device_motion_sample_ok"]) == Some(true)
    {
        push_reason(reasons, "device_motion_vector_present");
    }
    if fo.get("ua_ch_platform_version").is_some()
        || fo.get("ua_ch_model").is_some()
        || fo.get("ua_ch_bitness").is_some()
        || fo.get("ua_ch_full_version_list").is_some()
    {
        push_reason(reasons, "ua_ch_high_entropy_present");
    }
    if fo.get("storage_quota_class").is_some() {
        push_reason(reasons, "storage_quota_class_present");
    }
    if fo.get("ice_candidate_types").is_some()
        || bool_field(fo, &["ice_has_host"]).is_some()
        || fo.get("ice_candidate_count").is_some()
    {
        push_reason(reasons, "ice_morphology_present");
    }
    if fo.get("ja4_r").is_some()
        || fo.get("tls_extensions_order").is_some()
        || fo.get("cipher_suites_order").is_some()
    {
        push_reason(reasons, "tls_hello_order_present");
    }
    if fo.get("dns_cache_timing_delta_ms").is_some() {
        push_reason(reasons, "dns_cache_delta_present");
    }
    if fo.get("gl_bandwidth_lite").is_some()
        || fo.get("gl_max_cube_map").is_some()
        || fo.get("gl_max_varying_vectors").is_some()
    {
        push_reason(reasons, "gl_caps_extra_present");
    }
    if fo.get("highp_precision").is_some() {
        push_reason(reasons, "highp_precision_present");
    }
    if fo.get("native_checked").is_some() {
        push_reason(reasons, "native_checked_present");
    }
    if fo.get("neg_dict_hash").is_some() {
        push_reason(reasons, "neg_dict_hash_present");
    }
    if bool_field(fo, &["offscreen_available"]).is_some() {
        push_reason(reasons, "offscreen_available_present");
    }
    if fo.get("perf_dom_content_loaded_ms").is_some()
        || fo.get("perf_load_event_ms").is_some()
        || fo.get("perf_time_origin").is_some()
    {
        push_reason(reasons, "perf_nav_depth_present");
    }
    if fo.get("roundtrip_slope").is_some() {
        push_reason(reasons, "roundtrip_slope_present");
    }
    if fo.get("total_js_heap_size").is_some() || fo.get("used_js_heap_size").is_some() {
        push_reason(reasons, "js_heap_runtime_present");
    }
    if fo.get("ws_protocol").is_some()
        || fo.get("ws_extensions").is_some()
        || fo.get("ws_ready_state").is_some()
    {
        push_reason(reasons, "websocket_depth_present");
    }
    if fo.get("challenge_audio_freq").is_some() || fo.get("challenge_repeat_means").is_some() {
        push_reason(reasons, "challenge_audio_repeat_present");
    }
    if bool_field(fo, &["quic_aead_ok"]) == Some(true) {
        push_reason(reasons, "quic_aead_clienthello");
    } else if fo.get("quic_aead_ok").is_some() {
        push_reason(reasons, "quic_aead_attempted");
    }
    // P-V3 TCP depth
    if bool_field(fo, &["tcp_info_available"]) == Some(true)
        || fo.get("tcp_info_rtt_us").is_some()
        || bool_field(fo, &["tcp_saved_syn"]) == Some(true)
    {
        push_reason(reasons, "tcp_depth_present");
        if bool_field(fo, &["tcp_saved_syn"]) == Some(true) {
            push_reason(reasons, "tcp_saved_syn_present");
        }
        if f64_field(fo, &["tcp_info_total_retrans"]).unwrap_or(0.0) >= 5.0 {
            sa_bump(spoof_score, vm_score, "tcp_retrans_high", 0.00, 0.04);
            push_reason(reasons, "tcp_retrans_high");
        }
    }

    // Full HTTP/3 application layer
    if bool_field(fo, &["h3_app_present"]) == Some(true)
        || fo.get("h3_settings_fp").is_some()
        || fo.get("h3_pseudo_order").is_some()
    {
        push_reason(reasons, "h3_app_layer_present");
        if fo.get("h3_pseudo_order").is_some() {
            push_reason(reasons, "h3_pseudo_order_present");
        }
        if let Some(rtt) = f64_field(fo, &["h3_rtt_ms"]) {
            if rtt > 0.0 {
                push_reason(reasons, "h3_rtt_present");
            }
        }
    }
}

/// Build stack/spoof authenticity from FE fields.
pub fn stack_auth_from_fields(fields: &Value) -> StackAuth {
    let fo = fields.as_object().cloned().unwrap_or_default();

    let residual_mean = f64_field(&fo, &["residual_mean"]);
    let hist = residual_hist(&fo);
    // If residual_mean missing but hist is residual-mean style, derive mean
    let residual_mean = residual_mean.or_else(|| {
        if hist.len() >= 8 {
            let m = hist.iter().sum::<f64>() / hist.len() as f64;
            if (0.4..0.6).contains(&m) {
                Some(m)
            } else {
                None
            }
        } else {
            None
        }
    });

    let renderer = str_field(
        &fo,
        &["webgl_unmasked_renderer", "webgl_renderer", "renderer"],
    );
    let rc = {
        let c = str_field(&fo, &["renderer_class"]);
        if !c.is_empty() {
            c
        } else {
            renderer_class_from_label(&renderer)
        }
    };
    let soft_lab = soft_label(&renderer) || is_soft_renderer_class(&rc);
    let high = is_high_end(&rc);

    let residual_soft_like = infer_residual_soft_like(
        residual_mean,
        &hist,
        bool_field(&fo, &["residual_soft_like"]),
        soft_lab,
    );

    // Prefer server residual inference over FE stack_class (FE may be incomplete).
    let fe_stack = str_field(&fo, &["stack_class"]);
    let mut stack_class = if residual_soft_like == Some(true) || soft_lab {
        "soft_render".into()
    } else if residual_soft_like == Some(false) {
        "real_silicon".into()
    } else if !fe_stack.is_empty() && fe_stack != "unknown" {
        fe_stack
    } else {
        "unknown".into()
    };

    let mut reasons: Vec<String> = fo
        .get("env_reasons")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    // iss/opus5 §3.1 (trust model): client-reported `spoof_score`/`vm_score`
    // used to short-circuit the entire server-side derivation — a client
    // reporting `spoof_score: 0` disabled the strongest anti-spoof criterion
    // (high-end GPU label vs measured soft rendering). The server now ALWAYS
    // derives its own scores; client values are only divergence hints, and a
    // client self-reporting far below the server derivation is itself
    // spoof-intent evidence.
    let fe_spoof_hint = f64_field(&fo, &["spoof_score"]);
    let fe_vm_hint = f64_field(&fo, &["vm_score"]);

    let mut spoof_score = 0.0_f64;
    if high && residual_soft_like == Some(true) {
        sa_bump_spoof(&mut spoof_score, "webgl_label_vs_residual", 0.45);
        if !reasons.iter().any(|r| r == "webgl_label_vs_residual") {
            reasons.push("webgl_label_vs_residual".into());
        }
        if !reasons.iter().any(|r| r == "gpu_label_vs_measured_cluster") {
            reasons.push("gpu_label_vs_measured_cluster".into());
        }
    }
    if high && soft_lab {
        sa_bump_spoof(&mut spoof_score, "webgl_label_vs_soft", 0.25);
        if !reasons.iter().any(|r| r == "webgl_label_vs_soft") {
            reasons.push("webgl_label_vs_soft".into());
        }
    }
    if residual_soft_like == Some(true) || soft_lab {
        if !reasons.iter().any(|r| r == "soft_gl") {
            reasons.push("soft_gl".into());
        }
    }
    let max_tex = f64_field(&fo, &["webgl_max_texture", "max_texture_size"]);
    if residual_soft_like == Some(true) && high && max_tex.is_some_and(|t| t >= 16384.0) {
        sa_bump_spoof(&mut spoof_score, "caps_claim_vs_soft_behavior", 0.15);
        if !reasons.iter().any(|r| r == "caps_claim_vs_soft_behavior") {
            reasons.push("caps_claim_vs_measured_soft".into());
        }
    }
    // Hist present with soft shape + high-end label
    if high && residual_shape_soft_like(&hist) == Some(true) {
        if spoof_score < 0.4 {
            spoof_score = 0.45;
        }
        if !reasons.iter().any(|r| r == "webgl_label_vs_hist_shape") {
            reasons.push("webgl_label_vs_hist_shape".into());
        }
    }
    if spoof_score > 1.0 {
        spoof_score = 1.0;
    }

    if residual_soft_like == Some(true) && stack_class == "real_silicon" {
        stack_class = "soft_render".into();
    }

    let mut vm_score = 0.0_f64;
    if stack_class == "soft_render" {
        sa_bump(&mut spoof_score, &mut vm_score, "stack_soft_render", 0.0, 0.25);
    }
    if rc == "virt_gpu" {
        sa_bump(&mut spoof_score, &mut vm_score, "vm_virt_gpu", 0.0, 0.35);
    }
    if vm_score > 1.0 {
        vm_score = 1.0;
    }

    // Client-vs-server divergence: self-reported clean while the server sees
    // spoof evidence is a positive signal, not an override.
    if let Some(fe) = fe_spoof_hint {
        if spoof_score - fe >= 0.3 {
            spoof_score = (spoof_score + 0.15).min(1.0);
            if !reasons.iter().any(|r| r == "fe_spoof_underreport") {
                reasons.push("fe_spoof_underreport".into());
            }
        }
    }
    if let Some(fe) = fe_vm_hint {
        if vm_score - fe >= 0.3 {
            vm_score = (vm_score + 0.15).min(1.0);
            if !reasons.iter().any(|r| r == "fe_vm_underreport") {
                reasons.push("fe_vm_underreport".into());
            }
        }
    }

    // --- P0-A wire: consume uploaded-but-unused FE materials (SSOT gap analysis) ---
    apply_p0_physical_materials(&fo, high, soft_lab, &mut spoof_score, &mut vm_score, &mut reasons);

    // --- P0-B: screen_fp_protection_suspect (RFP spoof pattern) ---
    // FE reports a classic RFP signature (avail==size window mask). Alone it is
    // an observation (privacy tools are legitimate); combined with a high-end
    // GPU claim while the residual says real silicon it is an inconsistency —
    // RFP-style masking is observed almost exclusively on soft/stable stacks.
    // Never takes digest-role materials; only spoof/reason posture.
    if bool_field(&fo, &["screen_fp_protection_suspect"]).unwrap_or(false) {
        push_reason(&mut reasons, "rfp_screen_suspect");
        if high && residual_soft_like == Some(false) && spoof_score < 0.35 {
            sa_bump_spoof(&mut spoof_score, "rfp_screen_vs_gpu_claim", 0.10);
            push_reason(&mut reasons, "rfp_screen_vs_gpu_claim");
        } else if high && residual_soft_like == Some(true) {
            // consistent with a soft stack — record pairing, no bump
            push_reason(&mut reasons, "rfp_screen_soft_consistent");
        }
    }

    let soft_stack = stack_class == "soft_render"
        || residual_soft_like == Some(true)
        || soft_lab
        || is_soft_renderer_class(&rc);
    let gpu_label_untrusted = spoof_score >= 0.4
        || (high && residual_soft_like == Some(true))
        || (high && soft_stack)
        || reasons.iter().any(|r| {
            r == "webgl_label_vs_residual"
                || r == "gpu_label_vs_measured_cluster"
                || r == "webgl_label_vs_hist_shape"
                || r == "gpu_slope_soft_like"
                || r == "precision_claim_vs_behavior"
                || r == "caps_claim_vs_actual"
        });

    let authenticity_hint = if spoof_score >= 0.4 {
        "fp_spoof_suspect"
    } else if soft_stack {
        "soft_stack"
    } else if stack_class == "real_silicon" {
        "real_stack"
    } else {
        "unknown"
    }
    .to_string();

    StackAuth {
        stack_class,
        residual_soft_like,
        residual_mean,
        spoof_score,
        vm_score,
        renderer_class: rc,
        reasons,
        authenticity_hint,
        claims_host_silicon_recovery: false,
        gpu_label_untrusted,
        soft_stack,
    }
}

/// Whether WebGL label may enter commercial hard materials.
pub fn gpu_label_commercial_ok(auth: &StackAuth) -> bool {
    !auth.gpu_label_untrusted
}

/// Soft/untrusted stack: commercial id still emits, but is **soft-aware / warned**.
/// Name kept for callers that demote confidence or strip GPU labels — not a hard refuse.
pub fn commercial_id_blocked_by_stack(auth: &StackAuth) -> bool {
    auth.soft_stack || auth.gpu_label_untrusted || is_soft_renderer_class(&auth.renderer_class)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn real_residual_not_spoof() {
        let f = json!({
            "webgl_unmasked_renderer": "ANGLE (NVIDIA Corporation, NVIDIA GeForce GTX 1050 Ti/PCIe/SSE2, OpenGL 4.5.0)",
            "residual_mean": 0.5001811906403189,
            "webgl_max_texture": 32768,
        });
        let a = stack_auth_from_fields(&f);
        assert_eq!(a.stack_class, "real_silicon");
        assert_eq!(a.residual_soft_like, Some(false));
        assert!(a.spoof_score < 0.4);
        assert!(!a.gpu_label_untrusted);
        assert!(!a.claims_host_silicon_recovery);
        assert!(!commercial_id_blocked_by_stack(&a));
    }

    #[test]
    fn soft_swiftshader() {
        let f = json!({
            "webgl_unmasked_renderer": "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero)), SwiftShader driver)",
            "residual_mean": 0.5002766927083336,
        });
        let a = stack_auth_from_fields(&f);
        assert_eq!(a.stack_class, "soft_render");
        assert_eq!(a.residual_soft_like, Some(true));
        assert!(a.soft_stack);
        assert!(commercial_id_blocked_by_stack(&a));
    }

    #[test]
    fn camoufox_label_soft_residual_spoof() {
        let f = json!({
            "webgl_unmasked_renderer": "NVIDIA GeForce GTX 980, or similar",
            "residual_mean": 0.5002774107689955,
            "webgl_max_texture": 32768,
        });
        let a = stack_auth_from_fields(&f);
        assert_eq!(a.stack_class, "soft_render");
        assert!(a.spoof_score >= 0.4);
        assert!(a.gpu_label_untrusted);
        assert!(!gpu_label_commercial_ok(&a));
        assert!(commercial_id_blocked_by_stack(&a));
        assert!(a
            .reasons
            .iter()
            .any(|r| r.contains("residual") || r.contains("cluster") || r.contains("hist")));
    }

    #[test]
    fn client_reported_zero_spoof_score_does_not_shortcircuit_server() {
        // iss/opus5 §3.1: a client self-reporting spoof_score=0 must NOT skip
        // the server-side derivation (high-end label + soft residual evidence).
        let f = json!({
            "webgl_unmasked_renderer": "NVIDIA GeForce GTX 980, or similar",
            "residual_mean": 0.5002774107689955,
            "webgl_max_texture": 32768,
            "spoof_score": 0.0,
            "vm_score": 0.0,
        });
        let a = stack_auth_from_fields(&f);
        assert_eq!(a.stack_class, "soft_render");
        assert!(
            a.spoof_score >= 0.4,
            "client-reported 0 must not suppress server derivation: {}",
            a.spoof_score
        );
        assert!(
            a.reasons.iter().any(|r| r == "fe_spoof_underreport"),
            "divergence signal expected: {:?}",
            a.reasons
        );
        assert!(a.vm_score > 0.0, "server vm derivation must run");
    }

    #[test]
    fn client_reported_zero_vm_score_does_not_shortcircuit_server() {
        // virt_gpu label + soft residual → server vm_score = 0.25+0.35 = 0.6;
        // client self-report of 0 must not suppress it and must raise the
        // divergence reason (gap 0.6 >= 0.3).
        let f = json!({
            "webgl_unmasked_renderer": "virtio-gpu (VirGL)",
            "residual_mean": 0.5002,
            "vm_score": 0.0,
        });
        let a = stack_auth_from_fields(&f);
        assert_eq!(a.stack_class, "soft_render");
        assert!(a.vm_score >= 0.6, "vm_score={}", a.vm_score);
        assert!(
            a.reasons.iter().any(|r| r == "fe_vm_underreport"),
            "reasons: {:?}",
            a.reasons
        );
    }

    #[test]
    fn consistent_client_hint_no_divergence_reason() {
        let f = json!({
            "webgl_unmasked_renderer": "ANGLE (NVIDIA Corporation, NVIDIA GeForce GTX 1050 Ti/PCIe/SSE2, OpenGL 4.5.0)",
            "residual_mean": 0.5001811906403189,
            "spoof_score": 0.0,
            "vm_score": 0.0,
        });
        let a = stack_auth_from_fields(&f);
        assert_eq!(a.stack_class, "real_silicon");
        assert!(!a.reasons.iter().any(|r| r == "fe_spoof_underreport"));
        assert!(!a.reasons.iter().any(|r| r == "fe_vm_underreport"));
    }

    #[test]
    fn p0a_challenge_cv_and_native_change_reasons() {
        let base = json!({
            "webgl_unmasked_renderer": "ANGLE (NVIDIA, GeForce GTX 1050 Ti)",
            "residual_mean": 0.5001811906403189,
        });
        let a0 = stack_auth_from_fields(&base);
        let mut rich = base.as_object().cloned().unwrap();
        rich.insert("challenge_avg_cv".into(), json!(0.12));
        rich.insert("challenge_unique_digest_n".into(), json!(3));
        rich.insert("native_integrity_ratio".into(), json!(0.4));
        rich.insert("gpu_wall_staircase".into(), json!([
            {"size":64,"iter":8,"wall_ms":2.0},
            {"size":128,"iter":16,"wall_ms":8.0},
            {"size":256,"iter":32,"wall_ms":40.0},
            {"size":512,"iter":64,"wall_ms":200.0}
        ]));
        rich.insert("gpu_slope_wall".into(), json!(0.001));
        rich.insert("gl_precision_matrix".into(), json!([[1,1,23]]));
        let a1 = stack_auth_from_fields(&Value::Object(rich));
        assert!(
            a1.spoof_score > a0.spoof_score,
            "p0 materials must raise spoof: {} vs {}",
            a1.spoof_score,
            a0.spoof_score
        );
        assert!(
            a1.reasons.iter().any(|r| r.contains("noise") || r.contains("native") || r.contains("gpu_slope") || r.contains("challenge")),
            "reasons: {:?}",
            a1.reasons
        );
    }

    #[test]
    fn p0a_caps_claim_vs_actual_raises_spoof() {
        let f = json!({
            "webgl_unmasked_renderer": "NVIDIA GeForce RTX 3080",
            "residual_mean": 0.5001811906403189,
            "gl_max_texture_size": 32768,
            "actual_max_tex": 4096,
        });
        let a = stack_auth_from_fields(&f);
        assert!(a.spoof_score >= 0.28, "caps mismatch spoof={}", a.spoof_score);
        assert!(a.reasons.iter().any(|r| r.contains("caps_claim")));
    }

    #[test]
    fn real_mobile_adreno_not_spoof() {
        let f = json!({
            "webgl_unmasked_renderer": "Adreno (TM) 730",
            "residual_mean": 0.5001811906403189,
            "platform": "Linux armv8l",
            "user_agent": "Mozilla/5.0 (Linux; Android 13) AppleWebKit/537.36 Mobile Safari/537.36",
        });
        let a = stack_auth_from_fields(&f);
        assert_eq!(a.stack_class, "real_silicon");
        assert!(a.spoof_score < 0.4);
    }

    #[test]
    fn soft_label_alone_blocks_without_mean() {
        let f = json!({
            "webgl_unmasked_renderer": "llvmpipe (LLVM 15.0.0, 256 bits)",
        });
        let a = stack_auth_from_fields(&f);
        assert!(a.soft_stack);
        assert!(commercial_id_blocked_by_stack(&a));
    }
}
