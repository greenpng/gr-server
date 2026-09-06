//! Per-material trust scoring + commercial device_id projection (machine_trust_v2).
//!
//! Design goals:
//! - Same physical host → same commercial `device_id` across kernels and proxy egress IPs
//! - Spoofable surface (UA, WebGL string, viewport) and egress IP alone cannot dominate
//! - Live hardware-noise curve digests + WebRTC host structure are primary anchors
//! - Soft cosine similarity remains diagnostic; commercial id requires trust gates

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

/// algo tag exposed on evaluate device projection
pub const COMMERCIAL_ALGO: &str = "machine_trust_v2";

/// Minimum trust for a material to enter the commercial digest.
pub const COMMERCIAL_TRUST_FLOOR: f64 = 0.55;
/// Minimum sum of included material trusts to emit a commercial device_id.
pub const COMMERCIAL_TRUST_SUM_MIN: f64 = 1.60;

/// Static trust prior per material (industry-informed; not a fixed sample library).
/// Values in \[0, 1\].
pub fn material_trust_prior(key: &str) -> f64 {
    match key {
        "hw_audio_stable" => 0.92,
        "hw_canvas_stable" => 0.70, // diagnostic; diverges by browser rasterizer
        "hw_webgl_stable" => 0.80,
        "hw_anti_collision" => 0.72,
        "hw_ensemble_digest" => 0.55,
        "hw_timing_phase_digest" => 0.48,
        "hw_silicon_fine" => 0.78,
        "hw_silicon_fusion" => 0.88,
        "hw_webgl_stable_fused" => 0.80,
        "webrtc_host_ip_hash" => 0.90,
        "os_instance_hash" => 0.85,
        "os_family" => 0.85,
        "form_class" => 0.70,
        "architecture" => 0.70,
        "gpu_vendor_class" => 0.45,
        "storage_quota_class" => 0.70,
        "audio_sample_rate" => 0.65,
        "color_depth" => 0.60,
        "max_touch_points" => 0.60,
        "cores_class" => 0.55,
        "mem_class" => 0.50,
        "soft_residual_bucket" => 0.80,
        "timezone" => 0.45,
        "device_memory" => 0.40,
        "speech_voices_count" => 0.50,
        "server_client_ip" | "server_asn" => 0.25,
        "user_agent" | "webgl_unmasked_renderer" | "screen_width" | "screen_height" => 0.15,
        _ => 0.30,
    }
}

/// Single commercial digest key order — **no exclusive alternate path**.
///
/// **F-11 SSOT**: live order comes from `field_product_matrix.commercial_device_id`
/// via [`crate::product_matrix::commercial_digest_order`]. Fallback constants match
/// the matrix default so offline tests without spec still work.
///
/// Live multi-kernel + feasibility-v9 track_B:
/// - Commercial **hw_webgl_stable** / **hw_audio_stable** use **coarse** digests so
///   Blink/Gecko/WebKit residual micro-diffs do not fork same-host machine id.
/// - Fine curves stay as `hw_*_fine` for conf / same-vendor separation.
/// - **PKG_GEOCPU** (screen class + timezone + cores) enters digest when present
///   (v9 sparse_safe geo path) — not canvas / engine_family / UA / server IP.
/// - WebRTC host hash: corroboration / soft separator only.
/// - Pingora JA3/JA4: **br/protocol**, never commercial machine digest.
const COMMERCIAL_DIGEST_ORDER_FALLBACK: &[&str] = &[
    "form_class",
    "hw_webgl_stable",
    "hw_audio_stable",
    "cores_class",
    "architecture",
    "screen_w_class",
    "screen_h_class",
    "timezone",
];

/// Hardware anchors — at least one curve required for commercial emission.
const HARDWARE_ANCHORS_FALLBACK: &[&str] = &["hw_webgl_stable", "hw_audio_stable"];

fn commercial_digest_order() -> Vec<&'static str> {
    let o = crate::product_matrix::commercial_digest_order();
    if o.is_empty() {
        COMMERCIAL_DIGEST_ORDER_FALLBACK.to_vec()
    } else {
        o
    }
}

fn hardware_anchors() -> Vec<&'static str> {
    let a = crate::product_matrix::commercial_hardware_anchors();
    if a.is_empty() {
        HARDWARE_ANCHORS_FALLBACK.to_vec()
    } else {
        a
    }
}

fn soft_commercial_extra() -> Vec<&'static str> {
    let e = crate::product_matrix::commercial_soft_extra();
    if e.is_empty() {
        SOFT_COMMERCIAL_EXTRA_FALLBACK.to_vec()
    } else {
        e
    }
}

fn backfill_os_family(fo: &Map<String, Value>) -> Option<String> {
    // Platform-first always: fingerprint injectors often spoof os_family/UA while
    // navigator.platform remains the host truth (Playwright WebKit Mac-UA + Linux platform).
    let platform = fo
        .get("platform")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let ua = fo
        .get("user_agent")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if platform.contains("android") {
        return Some("android".into());
    }
    if platform.contains("iphone") || platform.contains("ipad") {
        return Some("ios".into());
    }
    if platform.contains("win") {
        return Some("windows".into());
    }
    if platform.contains("linux") || platform.contains("x11") {
        return Some("linux".into());
    }
    if platform.contains("mac") {
        return Some("macos".into());
    }
    // Explicit FE os_family only when platform empty/generic
    if let Some(s) = fo
        .get("os_family")
        .and_then(|v| v.as_str())
        .map(|s| s.to_ascii_lowercase())
        .filter(|s| !s.is_empty())
    {
        return Some(s);
    }
    if ua.contains("android") {
        Some("android".into())
    } else if ua.contains("iphone") || ua.contains("ipad") {
        Some("ios".into())
    } else if ua.contains("windows") || ua.contains("win32") {
        Some("windows".into())
    } else if ua.contains("linux") || ua.contains("x11") {
        Some("linux".into())
    } else if ua.contains("mac") {
        Some("macos".into())
    } else {
        None
    }
}

/// Coarse cores class: log2 bucket so minor under-reporting (8 vs 12) often lands nearby.
/// Still not a fixed device table — pure function of concurrency.
/// Coarse architecture token from navigator.platform / ua arch fields.
pub fn arch_from_platform(platform: &str) -> Option<String> {
    let p = platform.to_ascii_lowercase();
    if p.is_empty() {
        return None;
    }
    if p.contains("aarch64") || p.contains("arm64") || p.contains("armv8") {
        return Some("arm64".into());
    }
    if p.contains("arm") {
        return Some("arm".into());
    }
    if p.contains("x86_64") || p.contains("x64") || p.contains("amd64") || p.contains("wow64") {
        return Some("x86_64".into());
    }
    // Blink often reports bare "x86" via UA-CH while Gecko reports "x86_64" on the same host.
    // Commercial arch class: treat IA-32-family labels as x86_64 unless clearly 32-bit-only.
    if p.contains("i686") || p.contains("i386") {
        return Some("x86".into());
    }
    if p.contains("x86") {
        return Some("x86_64".into());
    }
    // Browser legacy: platform "Win32" even on 64-bit Windows.
    if p.contains("win") {
        return Some("x86_64".into());
    }
    None
}

/// Coarse GPU vendor token from unmasked renderer (not full label — spoofable surface).
pub fn gpu_vendor_class(renderer: &str) -> Option<String> {
    let l = renderer.to_ascii_lowercase();
    if l.is_empty() {
        return None;
    }
    if l.contains("nvidia") || l.contains("geforce") || l.contains("quadro") || l.contains("rtx ")
    {
        return Some("nvidia".into());
    }
    if l.contains("amd") || l.contains("radeon") || l.contains("ati ") {
        return Some("amd".into());
    }
    if l.contains("intel") {
        return Some("intel".into());
    }
    if l.contains("apple") {
        return Some("apple".into());
    }
    if l.contains("mali") || l.contains("adreno") || l.contains("powervr") {
        return Some("mobile_gpu".into());
    }
    if l.contains("swiftshader") || l.contains("llvmpipe") || l.contains("softpipe") {
        return Some("software".into());
    }
    Some("other".into())
}

pub fn cores_class(n: i64) -> String {
    if n <= 0 {
        return "c0".into();
    }
    let b = ((n as f64).log2().floor() as i64).clamp(0, 8);
    format!("c{b}")
}

/// Stable digest of a live noise curve for commercial materials.
///
/// Multi-engine same-host audio often differs by **phase / analyser warmup** (leading zeros)
/// more than by spectral shape. Sequence-sensitive super-bins then fork `dv_*` across
/// Chromium/Firefox/WebKit on one machine.
///
/// `curve_v3` uses **order statistics** (sorted |x|/max abs quantiles): phase-invariant,
/// still separates machines with different amplitude distributions — no sample library.
///
/// **FE WebGL residual** is expected in ~[0,1] (histogram fractions). Do not mix large
/// precision integers into the same vector — max-abs would crush residual to ~0 and make
/// `hw_webgl_stable` a universal constant. Precision stays on `gl_*_float` fields.
pub fn curve_stable_digest(curve: &[f64]) -> Option<String> {
    if curve.len() < 4 {
        return None;
    }
    // Prefer residual-scale samples: if any |x|>2, treat large heads as non-residual
    // (legacy/bad FE prepend) and digest the remaining [0,1]-ish tail only.
    let residual: Vec<f64> = {
        let has_large = curve.iter().any(|x| x.abs() > 2.0);
        if has_large {
            let tail: Vec<f64> = curve.iter().copied().filter(|x| x.abs() <= 1.5).collect();
            if tail.len() >= 4 {
                tail
            } else {
                curve.to_vec()
            }
        } else {
            curve.to_vec()
        }
    };
    let max_abs = residual
        .iter()
        .map(|x| x.abs())
        .fold(0.0_f64, f64::max)
        .max(1e-12);
    // Drop near-silent flanks (Firefox analyser often pads leading zeros).
    let thr = (max_abs * 0.02).max(1e-9);
    let mut vals: Vec<f64> = residual
        .iter()
        .map(|x| x.abs() / max_abs)
        .filter(|&a| a >= thr / max_abs || a >= 0.02)
        .collect();
    if vals.len() < 4 {
        vals = residual.iter().map(|x| x.abs() / max_abs).collect();
    }
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    // curve_v4: more quantiles + finer levels + moment summary.
    // v3 (8×8) over-merged distinct machines that share similar residual shape
    // (prod: thousands of Win desktops → one dv_* despite different raw curves/GPUs).
    // Still order-stat based → phase-invariant for same-host multi-engine.
    let n_q = 16usize;
    let levels = 32.0_f64;
    let mut hasher = Sha256::new();
    hasher.update(b"curve_v4|ostats|");
    for k in 0..n_q {
        let idx = (((k + 1) as f64) / (n_q as f64) * ((vals.len() - 1) as f64)).round() as usize;
        let idx = idx.min(vals.len() - 1);
        let q = (vals[idx] * levels).round() as i64;
        hasher.update(k.to_string().as_bytes());
        hasher.update(b":");
        hasher.update(q.to_string().as_bytes());
        hasher.update(b";");
    }
    // Mean / std (phase-invariant moments) at 1e-3 relative precision
    let n = vals.len() as f64;
    let mean = vals.iter().sum::<f64>() / n;
    let var = vals.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n;
    let std = var.sqrt();
    hasher.update(b"|m");
    hasher.update(format!("{:.3}", mean).as_bytes());
    hasher.update(b"|s");
    hasher.update(format!("{:.3}", std).as_bytes());
    // Decimated sorted profile at 1e-2 — separates templates that share quantile buckets
    // but differ in body shape (still phase-invariant via sort).
    let step = (vals.len() / 16).max(1);
    hasher.update(b"|p");
    for (i, v) in vals.iter().enumerate().step_by(step) {
        if i > 0 {
            hasher.update(b",");
        }
        hasher.update(format!("{:.2}", v).as_bytes());
    }
    // Length class (log2) so tiny vs long captures do not always collide
    let len_class = ((residual.len() as f64).log2().floor() as i64).clamp(2, 16);
    hasher.update(b"L");
    hasher.update(len_class.to_string().as_bytes());
    let dig = format!("{:x}", hasher.finalize());
    Some(format!("c_{}", &dig[..16]))
}

/// Engine-aware residual **measurement calibration** (not force-association).
///
/// WebKit strip-std curves sometimes include a cold first-seed segment (near-zero
/// variance) after GL pipeline start. Dropping only a leading near-silent seed block
/// improves *measurement fidelity on WebKit* without collapsing distinct GPUs.
/// Blink/Gecko pass through unchanged.
pub fn residual_curve_for_engine(curve: &[f64], engine_family: Option<&str>) -> Vec<f64> {
    let eng = engine_family.unwrap_or("").to_ascii_lowercase();
    if eng != "webkit" || curve.len() < 16 {
        return curve.to_vec();
    }
    // v3f layout: 4 seeds × (6 strip std + 2 moments) = 32
    let seed_len = 8usize;
    if curve.len() < seed_len * 2 {
        return curve.to_vec();
    }
    let first: Vec<f64> = curve.iter().take(seed_len).copied().collect();
    let rest: Vec<f64> = curve.iter().skip(seed_len).copied().collect();
    let mean: f64 = first.iter().sum::<f64>() / first.len() as f64;
    let var: f64 = first.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / first.len() as f64;
    // Only drop cold first seed when it is near-flat (measurement artifact).
    if var < 1e-6 && mean < 0.05 {
        let mut out = rest;
        while out.len() < 32 {
            out.push(0.0);
        }
        if out.len() > 32 {
            out.truncate(32);
        }
        return out;
    }
    curve.to_vec()
}

/// Commercial WebGL digest for **machine-stable** multi-engine identity.
///
/// Lab: Blink/Gecko share residual; **WebKit** on same NVIDIA often puts mass in
/// **different bin indices** (GL stack layout) while order-stats + top **magnitudes**
/// stay similar. v1 hashed `bin:mag` → forked same-host `dh_*`.
///
/// `webgl_comm_v4` = coarse order-stats + **top magnitudes by rank** (no bin index)
/// + sorted-body moments at **0.001 mean/std** (do **not** coarsen to 0.01 — multi-machine
/// collision risk). Cross-engine drift (WebKit vs ANGLE ~0.001 residual mean) must be
/// reduced at **probe time** (multi-path residual / warmup / seed ensemble), not by
/// dropping commercial precision.
///
/// Bin-indexed peaks stay on `hw_webgl_peak_sig` (conf only).
/// Optional `engine_family` applies **measurement calibration only** (see
/// [`residual_curve_for_engine`]) — never invents materials or force-merges hosts.
/// Multipath commercial fuse: hash sorted per-path commercial digests + mode bitmask.
/// Returns None when fewer than 2 entropy-ok paths — callers fall back to single-curve digest.
pub fn multipath_commercial_fuse(
    fo: &Map<String, Value>,
    engine_family: Option<&str>,
) -> Option<Value> {
    let paths = fo.get("residual_paths")?.as_array()?;
    let mut digests: Vec<String> = Vec::new();
    let mut modes: Vec<String> = Vec::new();
    let mut path_ids: Vec<String> = Vec::new();
    let mut means: Vec<f64> = Vec::new();
    for entry in paths {
        let ok = entry.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        let curve = path_curve_from_entry(entry);
        if !ok || curve.len() < 8 {
            continue;
        }
        let entropy = entry
            .get("entropy_ok")
            .and_then(|v| v.as_bool())
            .unwrap_or_else(|| residual_curve_entropy_ok(&curve));
        if !entropy {
            continue;
        }
        // Prefer Lane-C paths for commercial fuse; still allow any entropy-ok path
        // so float+noderiv+rint ensembles always fuse when present.
        if residual_path_lane(entry) == "lane_s" {
            let m = entry.get("shader_mode").and_then(|v| v.as_str()).unwrap_or("");
            let pid = entry.get("path_id").and_then(|v| v.as_str()).unwrap_or("");
            // ULP pure lane stays fine-only; skip as commercial fuse member
            if m == "ulp" || pid.contains("ulp") {
                continue;
            }
        }
        if let Some(d) = webgl_commercial_digest_for_engine(&curve, engine_family) {
            digests.push(d);
            let pid = entry
                .get("path_id")
                .and_then(|v| v.as_str())
                .unwrap_or("path")
                .to_string();
            path_ids.push(pid.clone());
            let mode = entry
                .get("shader_mode")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    if pid.contains("noderiv") {
                        "noderiv".into()
                    } else if pid.contains("rint") {
                        "rint".into()
                    } else if pid.contains("ulp") {
                        "ulp".into()
                    } else {
                        "float".into()
                    }
                });
            if !modes.contains(&mode) {
                modes.push(mode);
            }
            if let Some(mean) = entry.get("mean").and_then(|v| v.as_f64()) {
                means.push(mean);
            }
        }
    }
    if digests.len() < 2 {
        return None;
    }
    digests.sort();
    modes.sort();
    path_ids.sort();
    let mut hasher = Sha256::new();
    hasher.update(b"webgl_comm_v5_mp_fuse|");
    hasher.update(format!("n={}", digests.len()).as_bytes());
    for d in &digests {
        hasher.update(b"|");
        hasher.update(d.as_bytes());
    }
    hasher.update(b"|modes:");
    hasher.update(modes.join(",").as_bytes());
    // mean-cluster quanta at 0.001 — documents floor without dominating fuse
    if !means.is_empty() {
        let msum: f64 = means.iter().sum();
        let mavg = msum / means.len() as f64;
        hasher.update(b"|muq");
        hasher.update(format!("{:.3}", mavg).as_bytes());
    }
    let dig = format!("{:x}", hasher.finalize());
    Some(json!({
        "algo": "webgl_comm_v5_multipath_fuse",
        "digest": format!("wg_{}", &dig[..16]),
        "n_paths_fused": digests.len(),
        "modes": modes,
        "path_ids": path_ids,
        "path_digests": digests,
    }))
}

pub fn webgl_commercial_digest(curve: &[f64]) -> Option<String> {
    webgl_commercial_digest_for_engine(curve, None)
}

pub fn webgl_commercial_digest_for_engine(
    curve: &[f64],
    engine_family: Option<&str>,
) -> Option<String> {
    let calibrated = residual_curve_for_engine(curve, engine_family);
    if calibrated.len() < 4 {
        return None;
    }
    let coarse = curve_stable_digest_coarse(&calibrated)?;
    let mut mags: Vec<f64> = calibrated.iter().map(|v| v.abs()).collect();
    mags.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let mut hasher = Sha256::new();
    hasher.update(b"webgl_comm_v4|coarse+magrank+mom|");
    hasher.update(coarse.as_bytes());
    hasher.update(b"|m:");
    let top_n = 8.min(mags.len());
    for (i, mag) in mags.iter().take(top_n).enumerate() {
        if i > 0 {
            hasher.update(b",");
        }
        // 0.02 quanta (v3 was 0.04): separates v3f strip-std templates better.
        let mq = ((*mag) / 0.02).round() as i64;
        hasher.update(format!("{i}={mq}").as_bytes());
    }
    // Body moments — keep 0.001 precision; probe-side multi-pass must reduce engine drift.
    let n = mags.len() as f64;
    let mean = mags.iter().sum::<f64>() / n;
    let var = mags.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n;
    let std = var.sqrt();
    hasher.update(b"|mu");
    hasher.update(format!("{:.3}", mean).as_bytes());
    hasher.update(b"|sd");
    hasher.update(format!("{:.3}", std).as_bytes());
    // Mid-rank sample (indexes 4..12) at 0.03 quanta — catches body shape forks
    // that top-only ranks can miss when two GPUs share strong peaks.
    hasher.update(b"|mid:");
    let mid_lo = 4.min(mags.len().saturating_sub(1));
    let mid_hi = 12.min(mags.len());
    for (j, mag) in mags.iter().take(mid_hi).skip(mid_lo).enumerate() {
        if j > 0 {
            hasher.update(b",");
        }
        let mq = ((*mag) / 0.03).round() as i64;
        hasher.update(mq.to_string().as_bytes());
    }
    let dig = format!("{:x}", hasher.finalize());
    Some(format!("wg_{}", &dig[..16]))
}

/// Engine-sensitive peak **bin ranks** — conf / same-engine only (never commercial digest).
pub fn webgl_peak_signature(curve: &[f64]) -> Option<String> {
    if curve.len() < 4 {
        return None;
    }
    let mut idx: Vec<(usize, f64)> = curve
        .iter()
        .enumerate()
        .map(|(i, v)| (i, v.abs()))
        .collect();
    idx.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let top_n = 4.min(idx.len());
    let mut hasher = Sha256::new();
    hasher.update(b"webgl_peaks_v1|");
    for (i, (bin, mag)) in idx.iter().take(top_n).enumerate() {
        if i > 0 {
            hasher.update(b",");
        }
        let mq = ((*mag) / 0.05).round() as i64;
        hasher.update(format!("{bin}:{mq}").as_bytes());
    }
    let dig = format!("{:x}", hasher.finalize());
    Some(format!("wp_{}", &dig[..16]))
}

/// Whether a residual / noise histogram has enough local entropy to act as a
/// **machine** anchor (not just a class/template constant).
///
/// Prod 178 evidence: ANGLE mediump sin/cos residual hist was **byte-identical**
/// across 37 distinct GPU renderers (RTX 5090 … HD 520) → one `dh_*` for 44 VTIDs.
/// When residual is dead, commercial path must not drop audio/canvas and must not
/// claim hardware-tier uniqueness (`dh`) without host separators.
///
/// Gate stack (tightened 2026-07-31):
/// 1. std / unique-level / peak concentration (base)
/// 2. **peak_sig structure** — top magnitude ranks must not be near-flat
/// 3. **fine↔coarse quanta** — fine (1e-4) must carry more unique levels than
///    coarse (1e-2); pure class templates collapse under both quanta equally
pub fn residual_curve_entropy_ok(curve: &[f64]) -> bool {
    if curve.len() < 4 {
        return false;
    }
    let mut vals: Vec<f64> = curve.iter().copied().filter(|x| x.is_finite()).collect();
    if vals.len() < 4 {
        return false;
    }
    let n = vals.len() as f64;
    let mean = vals.iter().sum::<f64>() / n;
    let var = vals.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n;
    let std = var.sqrt();
    // Near-constant residual (global 32-bin hist or flat tile means) → dead class.
    // Prod v3d hist: std≈0.005–0.006 across ~all sessions → must stay rejected.
    // Raised floor 0.008 → 0.010 so borderline class templates demote earlier.
    if std < 0.010 {
        return false;
    }
    // Count effective unique bins at 1e-4 quanta (hist fractions often share templates).
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut uniq_fine = 1usize;
    for w in vals.windows(2) {
        if (w[1] - w[0]).abs() > 1e-4 {
            uniq_fine += 1;
        }
    }
    // 32-bin residual templates that collapse to few distinct mass levels.
    if uniq_fine < 8 {
        return false;
    }
    // Shannon-ish: reject extremely peaked single-bin residuals.
    let max_v = vals.iter().copied().fold(0.0_f64, f64::max);
    let sum: f64 = vals.iter().map(|x| x.abs()).sum::<f64>().max(1e-12);
    if max_v / sum > 0.50 {
        return false;
    }
    // --- peak_sig structure: top-4 abs ranks must show real multi-peak mass ---
    let mut mags: Vec<f64> = curve.iter().map(|v| v.abs()).filter(|x| x.is_finite()).collect();
    if mags.len() < 4 {
        return false;
    }
    mags.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let top_n = 4.min(mags.len());
    let top_sum: f64 = mags.iter().take(top_n).sum::<f64>().max(1e-12);
    let top1 = mags[0];
    // Single peak dominates all mass → class template / solid clear.
    if top1 / top_sum > 0.72 {
        return false;
    }
    // Near-flat top ranks (all equal) → no peak structure (uniform hist class).
    // Dead prod hist has top-4 nearly equal; healthy v3f strip-std has clear spread.
    // Do not reject smooth multi-level sinusoids that still have fine structure.
    let top_mean = top_sum / top_n as f64;
    let top_var: f64 = mags
        .iter()
        .take(top_n)
        .map(|x| (x - top_mean) * (x - top_mean))
        .sum::<f64>()
        / top_n as f64;
    let top_cv = top_var.sqrt() / top_mean.max(1e-12);
    let top_spread = (mags[0] - mags[top_n - 1]).abs();
    if top_cv < 0.02 && top_spread < 0.0025 {
        return false;
    }
    // --- fine↔coarse quanta: fine must resolve more structure than coarse ---
    let mut uniq_coarse = 1usize;
    for w in vals.windows(2) {
        if (w[1] - w[0]).abs() > 1e-2 {
            uniq_coarse += 1;
        }
    }
    // Class hist templates often share few coarse levels AND few fine levels.
    // Healthy residual: fine levels strictly exceed coarse (micro-structure present).
    if uniq_fine <= uniq_coarse && uniq_fine < 12 {
        return false;
    }
    // Coarse collapse with low fine diversity → dead class.
    if uniq_coarse < 3 && uniq_fine < 10 {
        return false;
    }
    true
}

/// Coarser order-stats digest for **soft-stack** commercial path only.
///
/// Soft-GL residual hist and OfflineAudio micro-noise still differ slightly across
/// Chromium channels on one guest (playwright headless vs system chrome). 4 quantiles
/// × 4 levels collapses same-host soft residual while remaining far from inventing
/// host silicon or fixed sample libraries.
pub fn curve_stable_digest_coarse(curve: &[f64]) -> Option<String> {
    if curve.len() < 4 {
        return None;
    }
    let residual: Vec<f64> = {
        let has_large = curve.iter().any(|x| x.abs() > 2.0);
        if has_large {
            let tail: Vec<f64> = curve.iter().copied().filter(|x| x.abs() <= 1.5).collect();
            if tail.len() >= 4 {
                tail
            } else {
                curve.to_vec()
            }
        } else {
            curve.to_vec()
        }
    };
    let max_abs = residual
        .iter()
        .map(|x| x.abs())
        .fold(0.0_f64, f64::max)
        .max(1e-12);
    let thr = (max_abs * 0.05).max(1e-9);
    let mut vals: Vec<f64> = residual
        .iter()
        .map(|x| x.abs() / max_abs)
        .filter(|&a| a >= thr / max_abs || a >= 0.05)
        .collect();
    if vals.len() < 4 {
        vals = residual.iter().map(|x| x.abs() / max_abs).collect();
    }
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n_q = 4usize;
    let levels = 4.0_f64;
    let mut hasher = Sha256::new();
    hasher.update(b"curve_v3c|ostats|");
    for k in 0..n_q {
        let idx = (((k + 1) as f64) / (n_q as f64) * ((vals.len() - 1) as f64)).round() as usize;
        let idx = idx.min(vals.len() - 1);
        let q = (vals[idx] * levels).round() as i64;
        hasher.update(k.to_string().as_bytes());
        hasher.update(b":");
        hasher.update(q.to_string().as_bytes());
        hasher.update(b";");
    }
    let len_class = ((residual.len() as f64).log2().floor() as i64).clamp(2, 16);
    hasher.update(b"L");
    hasher.update(len_class.to_string().as_bytes());
    let dig = format!("{:x}", hasher.finalize());
    Some(format!("cc_{}", &dig[..16]))
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

fn extract_curve(fo: &Map<String, Value>, flat: &str, nest: &str) -> Vec<f64> {
    let nested = fo.get("hw_noise_curves").and_then(|v| v.as_object());
    if let Some(v) = fo.get(flat) {
        let c = json_f64_vec(Some(v));
        if !c.is_empty() {
            return c;
        }
    }
    if let Some(n) = nested {
        return json_f64_vec(n.get(nest));
    }
    Vec::new()
}

fn i64_field(fo: &Map<String, Value>, key: &str) -> Option<i64> {
    fo.get(key)
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
}

fn str_field(fo: &Map<String, Value>, key: &str) -> Option<String> {
    fo.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
}

/// Resolve engine family from FE probe tags (measurement path, not identity).
pub fn engine_family_from_fields(fo: &Map<String, Value>) -> Option<String> {
    for k in ["engine_family", "engine_obs", "engine_claim"] {
        if let Some(s) = fo.get(k).and_then(|v| v.as_str()).map(|s| s.trim().to_ascii_lowercase()) {
            if matches!(s.as_str(), "blink" | "gecko" | "webkit") {
                return Some(s);
            }
        }
    }
    None
}

/// Magrank key (top-8 abs / 0.02) — path agreement without coarsening commercial digests.
pub fn residual_magrank_key(curve: &[f64]) -> String {
    let mut mags: Vec<f64> = curve.iter().map(|x| x.abs()).collect();
    mags.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    mags.into_iter()
        .take(8)
        .map(|x| ((x / 0.02).round() as i64).to_string())
        .collect::<Vec<_>>()
        .join(",")
}

fn path_curve_from_entry(entry: &Value) -> Vec<f64> {
    entry
        .get("curve")
        .map(|v| json_f64_vec(Some(v)))
        .unwrap_or_default()
}

/// Classify residual path into dual-lane:
/// - **lane_c** (commercial / cross-browser stability): rint, noderiv, classic float
/// - **lane_s** (silicon-level separability): ulp chain, eu timing
fn residual_path_lane(entry: &Value) -> &'static str {
    let pid = entry
        .get("path_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mode = entry
        .get("shader_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let has_eu_timing = entry
        .get("eu_timing_ms")
        .and_then(|v| v.as_array())
        .map(|a| a.len() >= 2)
        .unwrap_or(false);
    // Lane-S: ULP + iss/54 advanced silicon probes + any path carrying EU timing
    if mode == "ulp"
        || mode == "silicon_ulp"
        || mode == "fma_pair"
        || mode == "fma"
        || mode == "denorm"
        || mode == "ftz"
        || mode == "tex_lerp"
        || mode == "tex"
        || mode == "interp"
        || mode == "interp_diverge"
        || pid.contains("ulp_")
        || pid.contains("eu_timing")
        || pid.contains("fma_pair")
        || pid.contains("denorm")
        || pid.contains("tex_lerp")
        || pid.contains("interp_diverge")
        || has_eu_timing
    {
        return "lane_s";
    }
    if mode == "rint" || mode == "int" || mode == "noderiv" || pid.contains("rint_") || pid.contains("noderiv_")
    {
        return "lane_c";
    }
    "lane_c"
}

/// Server-side selection among FE `residual_paths` (authoritative for commercial curve).
/// Dual-lane (2026-08-02):
/// - **Lane-C** → commercial `hw_webgl_stable` (prefer rint/noderiv for cross-engine stability)
/// - **Lane-S** → silicon fine / eu_timing materials (prefer ulp)
/// Does **not** coarsen commercial digests — only picks which measurement path's curve to use.
/// Returns (lane_c_curve, residual_select meta).
pub fn select_residual_curve_from_paths(fo: &Map<String, Value>) -> (Vec<f64>, Value) {
    let paths = match fo.get("residual_paths").and_then(|v| v.as_array()) {
        Some(a) if !a.is_empty() => a,
        _ => {
            let c = extract_curve(fo, "hw_curve_webgl", "webgl");
            return (
                c,
                json!({
                    "policy": "single_hw_curve_webgl",
                    "algo": "residual_dual_lane_v1",
                    "lane_c": {"chosen_path_id": "hw_curve_webgl"},
                    "lane_s": Value::Null,
                    "chosen_path_id": "hw_curve_webgl",
                    "n_paths": 0,
                    "n_ok": if extract_curve(fo, "hw_curve_webgl", "webgl").len() >= 8 { 1 } else { 0 },
                    "note": "no residual_paths — use FE primary curve",
                }),
            );
        }
    };
    let mut key_count: std::collections::HashMap<String, i32> = std::collections::HashMap::new();
    for entry in paths {
        let ok = entry.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        let curve = path_curve_from_entry(entry);
        if !ok || curve.len() < 8 {
            continue;
        }
        let entropy = entry
            .get("entropy_ok")
            .and_then(|v| v.as_bool())
            .unwrap_or_else(|| residual_curve_entropy_ok(&curve));
        if !entropy {
            continue;
        }
        let key = entry
            .get("magrank_key")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| residual_magrank_key(&curve));
        *key_count.entry(key).or_insert(0) += 1;
    }
    // Dominant mean cluster among **lane_c** paths only (commercial stability).
    let mut mean_bucket_count: std::collections::HashMap<i64, (i32, f64)> =
        std::collections::HashMap::new();
    for entry in paths {
        if residual_path_lane(entry) != "lane_c" {
            continue;
        }
        let ok = entry.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        let curve = path_curve_from_entry(entry);
        if !ok || curve.len() < 8 {
            continue;
        }
        let entropy = entry
            .get("entropy_ok")
            .and_then(|v| v.as_bool())
            .unwrap_or_else(|| residual_curve_entropy_ok(&curve));
        if !entropy {
            continue;
        }
        let mean = entry
            .get("mean")
            .and_then(|v| v.as_f64())
            .unwrap_or_else(|| curve.iter().sum::<f64>() / curve.len() as f64);
        let b = (mean * 1000.0).round() as i64;
        let e = mean_bucket_count.entry(b).or_insert((0, mean));
        e.0 += 1;
        e.1 = mean;
    }
    let median_mean = mean_bucket_count
        .into_iter()
        .max_by(|a, b| {
            a.1 .0
                .cmp(&b.1 .0)
                .then_with(|| b.0.cmp(&a.0))
        })
        .map(|(_, (_, m))| m);

    // score paths for a given lane (strict: never mix Lane-S into commercial C)
    let score_for_lane = |lane: &str| -> Option<(f64, String, Vec<f64>, f64, f64, Value)> {
        let mut best: Option<(f64, String, Vec<f64>, f64, f64, Value)> = None;
        for entry in paths {
            if residual_path_lane(entry) != lane {
                continue;
            }
            let ok = entry.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            let curve = path_curve_from_entry(entry);
            if !ok || curve.len() < 8 {
                continue;
            }
            let entropy = entry
                .get("entropy_ok")
                .and_then(|v| v.as_bool())
                .unwrap_or_else(|| residual_curve_entropy_ok(&curve));
            if !entropy {
                continue;
            }
            let key = entry
                .get("magrank_key")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| residual_magrank_key(&curve));
            let mut score = (*key_count.get(&key).unwrap_or(&0) as f64) * 10.0;
            let size = entry.get("size").and_then(|v| v.as_i64()).unwrap_or(128);
            if size == 128 {
                score += 3.0;
            }
            let warm = entry
                .get("warm_frames")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            if warm >= 4 {
                score += 2.0;
            } else if warm >= 2 {
                score += 1.0;
            }
            let mean = entry
                .get("mean")
                .and_then(|v| v.as_f64())
                .unwrap_or_else(|| curve.iter().sum::<f64>() / curve.len() as f64);
            let std = entry.get("std").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if std > 0.02 {
                score += 1.0;
            }
            if mean > 0.15 && mean < 0.4 {
                score += 1.0;
            }
            if let Some(med) = median_mean {
                let d = (mean - med).abs();
                if d < 0.0005 {
                    score += 4.0;
                } else if d < 0.001 {
                    score += 2.0;
                } else if d > 0.002 {
                    score -= 2.0;
                }
            }
            let path_id = entry
                .get("path_id")
                .and_then(|v| v.as_str())
                .unwrap_or("path")
                .to_string();
            let mode = entry
                .get("shader_mode")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            // Lane-C: prefer rint/noderiv for cross-engine stability.
            // Must beat median-cluster bonus (+4) + classic warm path; hard noderiv
            // outranks float/unk warm paths so Blink does not mint a second dh_.
            if lane == "lane_c" {
                let has_noderiv_hard = paths.iter().any(|e| {
                    let pid = e.get("path_id").and_then(|v| v.as_str()).unwrap_or("");
                    let m = e.get("shader_mode").and_then(|v| v.as_str()).unwrap_or("");
                    let ok = e.get("ok").and_then(|v| v.as_bool()).unwrap_or(false)
                        || e.get("entropy_ok").and_then(|v| v.as_bool()).unwrap_or(false);
                    ok && (m == "noderiv" || pid.contains("noderiv_hard") || pid.contains("noderiv_"))
                });
                if mode == "rint" || path_id.contains("rint_") {
                    score += 9.0;
                } else if path_id.contains("noderiv_hard") {
                    score += 10.0;
                } else if mode == "noderiv" || path_id.contains("noderiv_") {
                    score += 8.5;
                } else if path_id.ends_with("_std") || path_id == "warm4_v3f_128" {
                    score += 1.0;
                    if has_noderiv_hard {
                        score -= 5.0;
                    }
                } else if path_id.contains("unk_") || mode == "float" || mode.is_empty() {
                    if has_noderiv_hard {
                        score -= 4.0;
                    }
                }
            }
            // Lane-S: prefer ulp + eu_timing richness
            if lane == "lane_s" {
                if path_id.contains("eu_timing") || entry.get("eu_timing_ms").is_some() {
                    score += 6.0;
                }
                if mode == "ulp" || path_id.contains("ulp_") {
                    score += 5.0;
                }
                // higher fine std is useful for silicon separation (bounded)
                if let Some(sf) = entry.get("std_fine").and_then(|v| v.as_f64()) {
                    score += (sf * 20.0).clamp(0.0, 3.0);
                }
            }
            if let Some(fe_id) = fo
                .get("residual_select")
                .and_then(|v| v.get("chosen_path_id"))
                .and_then(|v| v.as_str())
            {
                if entry.get("path_id").and_then(|v| v.as_str()) == Some(fe_id) {
                    score += 0.5;
                }
            }
            let entry_meta = json!({
                "path_id": path_id,
                "shader_mode": mode,
                "lane": lane,
                "mean_fine": entry.get("mean_fine").cloned().unwrap_or(Value::Null),
                "std_fine": entry.get("std_fine").cloned().unwrap_or(Value::Null),
                "eu_timing_ms": entry.get("eu_timing_ms").cloned().unwrap_or(Value::Null),
                "eu_timing_mean_ms": entry.get("eu_timing_mean_ms").cloned().unwrap_or(Value::Null),
            });
            match &best {
                None => best = Some((score, path_id, curve, mean, std, entry_meta)),
                Some((bs, ..)) if score > *bs => {
                    best = Some((score, path_id, curve, mean, std, entry_meta))
                }
                _ => {}
            }
        }
        best
    };

    let best_c = score_for_lane("lane_c");
    let best_s = score_for_lane("lane_s");
    let n_paths = paths.len();
    let n_ok = key_count.values().sum::<i32>();
    if let Some((score, path_id, curve, mean, std, lane_c_meta)) = best_c {
        let mut pairs = Vec::new();
        let chosen_key = residual_magrank_key(&curve);
        for entry in paths {
            let pid = entry.get("path_id").and_then(|v| v.as_str()).unwrap_or("");
            if pid == path_id {
                continue;
            }
            let c2 = path_curve_from_entry(entry);
            if c2.len() < 8 {
                continue;
            }
            let k2 = residual_magrank_key(&c2);
            let n = curve.len().min(c2.len()).max(1);
            let l2 = (0..n)
                .map(|i| {
                    let d = curve.get(i).copied().unwrap_or(0.0) - c2.get(i).copied().unwrap_or(0.0);
                    d * d
                })
                .sum::<f64>()
                .sqrt()
                / n as f64;
            let m2 = c2.iter().sum::<f64>() / c2.len() as f64;
            pairs.push(json!({
                "path_id": pid,
                "lane": residual_path_lane(entry),
                "magrank_eq": k2 == chosen_key,
                "l2": (l2 * 1e6).round() / 1e6,
                "mean_delta": ((mean - m2).abs() * 1e9).round() / 1e9,
            }));
        }
        let mut precision_pairs = Vec::new();
        for entry in paths {
            let pid = entry.get("path_id").and_then(|v| v.as_str()).unwrap_or("");
            if pid == path_id {
                continue;
            }
            let c2 = path_curve_from_entry(entry);
            if c2.len() < 8 {
                continue;
            }
            let mut pm = crate::engine_surface::residual_multi_precision_match(&curve, &c2);
            if let Some(obj) = pm.as_object_mut() {
                obj.insert("path_id".into(), json!(pid));
                obj.insert("lane".into(), json!(residual_path_lane(entry)));
            }
            precision_pairs.push(pm);
        }
        let (lane_s_json, lane_s_curve) =
            if let Some((s_score, s_pid, s_curve, s_mean, s_std, s_meta)) = best_s {
                (
                    json!({
                        "chosen_path_id": s_pid,
                        "score": s_score,
                        "mean": s_mean,
                        "std": s_std,
                        "mean_fine": s_meta.get("mean_fine").cloned().unwrap_or(json!(s_mean)),
                        "std_fine": s_meta.get("std_fine").cloned().unwrap_or(json!(s_std)),
                        "eu_timing_ms": s_meta.get("eu_timing_ms").cloned().unwrap_or(Value::Null),
                        "eu_timing_mean_ms": s_meta.get("eu_timing_mean_ms").cloned().unwrap_or(Value::Null),
                        "curve_len": s_curve.len(),
                        "has_curve": s_curve.len() >= 8,
                    }),
                    s_curve,
                )
            } else {
                (Value::Null, Vec::new())
            };
        let meta = json!({
            "policy": "residual_dual_lane_v1",
            "algo": "residual_dual_lane_v1",
            "chosen_path_id": path_id,
            "score": score,
            "mean": mean,
            "std": std,
            "n_paths": n_paths,
            "n_ok": n_ok,
            "lane_c": {
                "chosen_path_id": path_id,
                "score": score,
                "mean": mean,
                "std": std,
                "shader_mode": lane_c_meta.get("shader_mode").cloned().unwrap_or(Value::Null),
            },
            "lane_s": lane_s_json,
            "lane_s_curve": lane_s_curve,
            "pair_agreements": pairs,
            "pair_precision": precision_pairs,
            "commercial_floor_quanta": 0.001,
            "note": "Lane-C → hw_webgl_stable (prefer rint/noderiv); Lane-S → hw_webgl_fine / eu_timing (ulp); commercial digests keep 0.001",
        });
        return (curve, meta);
    }
    // Fallback single curve
    let c = extract_curve(fo, "hw_curve_webgl", "webgl");
    (
        c,
        json!({
            "policy": "fallback_hw_curve_webgl",
            "algo": "residual_dual_lane_v1",
            "chosen_path_id": "hw_curve_webgl",
            "lane_c": {"chosen_path_id": "hw_curve_webgl"},
            "lane_s": Value::Null,
            "n_paths": n_paths,
            "n_ok": 0,
            "note": "no entropy-ok residual_paths",
        }),
    )
}

/// Build trust-scored commercial materials from raw session fields.
pub fn trust_materials(fields: &Value) -> Map<String, Value> {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let mut m = Map::new();
    let engine = engine_family_from_fields(&fo);
    // Diagnostic only — never hashed into commercial device_id.
    if let Some(ref e) = engine {
        m.insert("engine_family".into(), json!(e));
    }
    if let Some(pp) = fo
        .get("probe_profile")
        .or_else(|| fo.get("residual_probe_profile"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        m.insert("probe_profile".into(), json!(pp));
    }
    if let Some(os) = backfill_os_family(&fo) {
        m.insert("os_family".into(), json!(os));
    }
    // Commercial audio: **coarse** order-stats — fine OfflineAudio still forks Blink vs Gecko
    // on one host (lab evidence: chrome fine ≠ firefox fine, while coarse collapses).
    let audio_curve = extract_curve(&fo, "hw_curve_audio", "audio");
    if let Some(d) = curve_stable_digest_coarse(&audio_curve) {
        m.insert("hw_audio_stable".into(), json!(d));
    }
    if let Some(d) = curve_stable_digest(&audio_curve) {
        // Diagnostic / conf only — not in commercial digest_order.
        m.insert("hw_audio_fine".into(), json!(d));
    }
    if let Some(d) = curve_stable_digest(&extract_curve(&fo, "hw_curve_canvas", "canvas")) {
        m.insert("hw_canvas_stable".into(), json!(d));
    }
    // Multi-path residual: server picks best measurement path for commercial curve.
    let (webgl_curve, mut residual_select_meta) = select_residual_curve_from_paths(&fo);
    // Lane-S fine curve if present; else fine digest of commercial curve (diagnostic).
    // Full curve is stripped from stored residual_select (keep length only — avoid bloat).
    let lane_s_curve: Vec<f64> = residual_select_meta
        .get("lane_s_curve")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_f64().or_else(|| x.as_i64().map(|i| i as f64)))
                .collect()
        })
        .unwrap_or_default();
    if let Some(obj) = residual_select_meta.as_object_mut() {
        if !lane_s_curve.is_empty() {
            obj.insert("lane_s_curve_len".into(), json!(lane_s_curve.len()));
        }
        obj.remove("lane_s_curve");
    }
    m.insert("residual_select".into(), residual_select_meta.clone());
    if let Some(paths) = fo.get("residual_paths") {
        m.insert("residual_paths_n".into(), json!(paths.as_array().map(|a| a.len()).unwrap_or(0)));
    }
    // WebGL residual: commercial digests keep full precision (webgl_comm_v4 0.001 mean).
    // Peak bins → conf only; multi-path only chooses which curve to hash.
    let webgl_for_comm = residual_curve_for_engine(&webgl_curve, engine.as_deref());
    // Empty / missing residual is NEVER entropy-OK (do not default true later).
    let webgl_entropy_ok = !webgl_for_comm.is_empty() && residual_curve_entropy_ok(&webgl_for_comm);
    m.insert(
        "webgl_residual_entropy_ok".into(),
        json!(webgl_entropy_ok),
    );
    // Lane-C → commercial stable (cross-browser). Lane-S → silicon fine (same-model split).
    // When multipath has ≥2 entropy-ok paths, fuse per-path commercial digests + modes
    // bitmask so same residual_mean floor (0.260/0.261) still separates path structure.
    let mp_fuse = multipath_commercial_fuse(&fo, engine.as_deref());
    if let Some(ref fused) = mp_fuse {
        if let Some(d) = fused
            .get("digest")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            m.insert("hw_webgl_stable".into(), json!(d));
        } else if let Some(d) =
            webgl_commercial_digest_for_engine(&webgl_curve, engine.as_deref())
        {
            m.insert("hw_webgl_stable".into(), json!(d));
        }
        m.insert("hw_webgl_multipath_fuse".into(), fused.clone());
        if let Some(obj) = residual_select_meta.as_object_mut() {
            obj.insert("multipath_fuse".into(), fused.clone());
            obj.insert(
                "ensemble_algo".into(),
                json!("webgl_comm_v5_multipath_fuse"),
            );
            obj.insert(
                "n_ok_fused".into(),
                fused
                    .get("n_paths_fused")
                    .cloned()
                    .unwrap_or(json!(0)),
            );
        }
        m.insert("residual_select".into(), residual_select_meta.clone());
    } else if let Some(d) = webgl_commercial_digest_for_engine(&webgl_curve, engine.as_deref()) {
        m.insert("hw_webgl_stable".into(), json!(d));
    }
    if let Some(d) = webgl_peak_signature(&webgl_curve) {
        m.insert("hw_webgl_peak_sig".into(), json!(d));
    }
    let fine_src = if lane_s_curve.len() >= 8 {
        residual_curve_for_engine(&lane_s_curve, engine.as_deref())
    } else {
        webgl_for_comm.clone()
    };
    if let Some(d) = curve_stable_digest(&fine_src) {
        m.insert("hw_webgl_fine".into(), json!(d));
    }
    // EU timing silicon material (DrawnApart-inspired) — conf / secondary, not commercial digest_order
    if let Some(eu) = residual_select_meta
        .pointer("/lane_s/eu_timing_ms")
        .and_then(|v| v.as_array())
    {
        let xs: Vec<f64> = eu
            .iter()
            .filter_map(|x| x.as_f64().or_else(|| x.as_i64().map(|i| i as f64)))
            .collect();
        if xs.len() >= 4 {
            if let Some(d) = curve_stable_digest(&xs) {
                m.insert("hw_eu_timing_digest".into(), json!(d));
            }
            let mean = xs.iter().sum::<f64>() / xs.len() as f64;
            m.insert("hw_eu_timing_mean_ms".into(), json!((mean * 1000.0).round() / 1000.0));
        }
    }
    // WebGPU compute residual (B18) — second silicon source; conf / secondary only
    let webgpu_curve = extract_curve(&fo, "hw_curve_webgpu", "webgpu");
    if webgpu_curve.len() >= 8 {
        if let Some(d) = curve_stable_digest(&webgpu_curve) {
            m.insert("hw_webgpu_compute_digest".into(), json!(d));
        }
        let n = webgpu_curve.len() as f64;
        let mean = webgpu_curve.iter().sum::<f64>() / n;
        let var = webgpu_curve
            .iter()
            .map(|x| (x - mean) * (x - mean))
            .sum::<f64>()
            / n;
        m.insert(
            "hw_webgpu_compute_mean".into(),
            json!((mean * 1e6).round() / 1e6),
        );
        m.insert(
            "hw_webgpu_compute_std".into(),
            json!((var.sqrt() * 1e6).round() / 1e6),
        );
        m.insert(
            "webgpu_compute_entropy_ok".into(),
            json!(residual_curve_entropy_ok(&webgpu_curve)),
        );
    }
    if let Some(algo) = fo
        .get("webgpu_compute_algo")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        m.insert("webgpu_compute_algo".into(), json!(algo));
    }
    m.insert(
        "residual_lanes".into(),
        json!({
            "algo": "residual_dual_lane_v1",
            "lane_c": residual_select_meta.get("lane_c").cloned().unwrap_or(Value::Null),
            "lane_s": residual_select_meta.get("lane_s").cloned().unwrap_or(Value::Null),
            "commercial_from": "lane_c",
            "fine_from": if lane_s_curve.len() >= 8 { "lane_s" } else { "lane_c_fallback" },
            "webgpu_silicon": if webgpu_curve.len() >= 8 { "hw_webgpu_compute_digest" } else { "absent" },
            "note": "Lane-C=commercial digests; Lane-S+WebGPU=conf/secondary silicon; never coarsen commercial floor",
        }),
    );
    // Observability: missing host sep is a probe gap, not proof of different machine.
    let has_webrtc = fo
        .get("webrtc_host_ip_hash")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());
    if !has_webrtc {
        m.insert("webrtc_missing".into(), json!(true));
        if let Some(reason) = fo
            .get("webrtc_probe_failed")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            m.insert("webrtc_probe_failed".into(), json!(reason));
        }
    }
    // residual_mean / residual_algo: FE when present; else derive mean from curve so
    // ServerMint residual_class is not stuck on universal rm_none for complete JS sessions.
    if let Some(algo) = fo
        .get("residual_algo")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        m.insert("residual_algo".into(), json!(algo));
    }
    // Prefer mean of **server-selected** multi-path curve; FE residual_mean is provisional.
    let residual_mean = residual_select_meta
        .get("mean")
        .and_then(|v| v.as_f64())
        .or_else(|| {
            fo.get("residual_mean")
                .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
        })
        .or_else(|| {
            if webgl_curve.len() < 4 {
                return None;
            }
            let n = webgl_curve.len() as f64;
            Some(webgl_curve.iter().sum::<f64>() / n)
        });
    if let Some(mean) = residual_mean {
        m.insert("residual_mean".into(), json!(mean));
        let b = (mean * 100000.0).round() / 100000.0;
        m.insert("residual_mean_bucket".into(), json!(format!("rm_{b:.5}")));
    }
    if let Some(std) = residual_select_meta.get("std").and_then(|v| v.as_f64()) {
        m.insert("residual_std".into(), json!(std));
    }
    // GEO/CPU package (v9 PKG_GEOCPU) — machine-stable when WebGL residual thin/missing
    if let Some(w) = i64_field(&fo, "screen_width") {
        // bucket to reduce CSS zoom noise: 100px class
        let wb = (w / 100) * 100;
        m.insert("screen_w_class".into(), json!(format!("sw{wb}")));
    }
    if let Some(h) = i64_field(&fo, "screen_height") {
        let hb = (h / 100) * 100;
        m.insert("screen_h_class".into(), json!(format!("sh{hb}")));
    }
    if let Some(tz) = str_field(&fo, "timezone") {
        m.insert("timezone".into(), json!(tz));
    }
    if let Some(s) = str_field(&fo, "webrtc_host_ip_hash") {
        m.insert("webrtc_host_ip_hash".into(), json!(s));
    }
    if let Some(s) = str_field(&fo, "os_instance_hash") {
        m.insert("os_instance_hash".into(), json!(s));
    }
    if let Some(s) = str_field(&fo, "architecture") {
        // Normalize UA-CH "x86" vs Gecko "x86_64" (same host) for commercial class.
        let norm = arch_from_platform(&s).unwrap_or_else(|| s.to_ascii_lowercase());
        m.insert("architecture".into(), json!(norm));
    } else if let Some(arch) = arch_from_platform(
        fo.get("platform")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
    ) {
        // Derive arch from platform so x86_64 vs aarch64 soft hosts can separate.
        m.insert("architecture".into(), json!(arch));
    }
    if let Some(r) = fo
        .get("webgl_unmasked_renderer")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        if let Some(vc) = gpu_vendor_class(r) {
            m.insert("gpu_vendor_class".into(), json!(vc));
        }
    }
    if let Some(s) = str_field(&fo, "storage_quota_class") {
        m.insert("storage_quota_class".into(), json!(s));
    }
    if let Some(n) = i64_field(&fo, "audio_sample_rate") {
        m.insert("audio_sample_rate".into(), json!(n.to_string()));
    }
    if let Some(n) = i64_field(&fo, "color_depth") {
        m.insert("color_depth".into(), json!(n.to_string()));
    }
    if let Some(n) = i64_field(&fo, "max_touch_points") {
        m.insert("max_touch_points".into(), json!(n.to_string()));
    }
    if let Some(n) = i64_field(&fo, "hardware_concurrency") {
        m.insert("cores_class".into(), json!(cores_class(n)));
    }
    if let Some(s) = str_field(&fo, "form_class") {
        m.insert("form_class".into(), json!(s));
    } else {
        // Derive coarse form from screen/touch when FE omitted form_class
        let w = i64_field(&fo, "screen_width").unwrap_or(0);
        let touch = i64_field(&fo, "max_touch_points").unwrap_or(0);
        let fc = if w > 0 && w < 600 {
            "mobile"
        } else if touch > 1 && w > 0 && w < 900 {
            "mobile"
        } else {
            "desktop"
        };
        m.insert("form_class".into(), json!(fc));
    }
    // Low-trust retained for reporting only (not digest by default)
    if let Some(s) = str_field(&fo, "timezone") {
        m.insert("timezone".into(), json!(s));
    }
    if let Some(s) = str_field(&fo, "server_client_ip") {
        m.insert("server_client_ip".into(), json!(s));
    }
    // webrtc_host_ip_hash already inserted above when present (str_field path)
    // Diagnostics: canvas stable still scored but not in commercial order
    m
}

/// Per-material trust map for materials present (priors; can be demoted by conflict later).
pub fn trust_scores_for(materials: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (k, _) in materials {
        out.insert(k.clone(), json!(material_trust_prior(k)));
    }
    out
}

pub fn hash_selected(materials: &Map<String, Value>, keys: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"mt2|");
    for k in keys {
        if let Some(v) = materials.get(*k) {
            hasher.update(k.as_bytes());
            hasher.update(b"=");
            hasher.update(v.as_str().unwrap_or(&v.to_string()).as_bytes());
            hasher.update(b"|");
        }
    }
    format!("{:x}", hasher.finalize())
}

/// Machine-readable reasons when commercial `device_id` is not emitted (or empty when eligible).
///
/// Stable codes for ops / product paths — not prose. Order is deterministic.
pub fn eligibility_reason_codes(
    materials: &Map<String, Value>,
    scores: &Map<String, Value>,
    included: &[String],
    trust_sum: f64,
    eligible: bool,
) -> Vec<String> {
    if eligible {
        return Vec::new();
    }
    let mut reasons: Vec<String> = Vec::new();
    let mat_has = |k: &str| materials.contains_key(k);
    let score_ok = |k: &str| {
        scores
            .get(k)
            .and_then(|v| v.as_f64())
            .map(|t| t + f64::EPSILON >= COMMERCIAL_TRUST_FLOOR)
            .unwrap_or(false)
    };
    let in_included = |k: &str| included.iter().any(|x| x == k);

    if !in_included("form_class") {
        if !mat_has("form_class") {
            reasons.push("missing_form".into());
        } else if !score_ok("form_class") {
            reasons.push("form_below_trust_floor".into());
        } else {
            reasons.push("missing_form".into());
        }
    }
    if !in_included("hw_audio_stable") {
        if !mat_has("hw_audio_stable") {
            reasons.push("missing_curve_audio".into());
        } else if !score_ok("hw_audio_stable") {
            reasons.push("audio_below_trust_floor".into());
        } else {
            reasons.push("missing_curve_audio".into());
        }
    }
    if !in_included("hw_webgl_stable") {
        if !mat_has("hw_webgl_stable") {
            reasons.push("missing_curve_webgl".into());
        } else if !score_ok("hw_webgl_stable") {
            reasons.push("webgl_below_trust_floor".into());
        } else {
            reasons.push("missing_curve_webgl".into());
        }
    }
    let has_curve = in_included("hw_audio_stable") || in_included("hw_webgl_stable");
    let has_both = in_included("hw_audio_stable") && in_included("hw_webgl_stable");
    if !has_curve {
        reasons.push("missing_hardware_anchor".into());
    }
    if trust_sum + 1e-9 < COMMERCIAL_TRUST_SUM_MIN {
        reasons.push("trust_sum_below_min".into());
    } else if has_curve && !has_both && trust_sum + 1e-9 < COMMERCIAL_TRUST_SUM_MIN + 0.5 {
        // Single curve path requires elevated trust_sum (sum_min + 0.5).
        reasons.push("single_curve_trust_insufficient".into());
    }
    if reasons.is_empty() {
        reasons.push("eligible_false".into());
    }
    reasons
}

/// Soft-path commercial digest **extra** keys (hashed only when soft_stack).
///
/// Machine-stable across browsers on one host, but typically **differ host vs guest**:
/// - cores_class, os_family, architecture
/// - webrtc_host_ip_hash: sorted private host-IP set fingerprint (ports excluded)
///   **required** for commercial `dv_*` on soft path (host separator).
///
/// **Exclude**: GPU labels; device_memory (Chromium under-reports vs Edge on same host);
/// full ICE candidates (ports/session-volatile).
const SOFT_COMMERCIAL_EXTRA_FALLBACK: &[&str] =
    &["cores_class", "os_family", "architecture", "webrtc_host_ip_hash"];

/// Full commercial projection: device_id + trust diagnostics.
///
/// - Real curves: emit `dv_*` when form + curve anchors clear the floor.
/// - Soft: soft-aware digest; always prefer commercial emit when form+residual/curve
///   anchors exist. Host separator (`webrtc_host_ip_hash`) **raises fusion weight** and
///   clears collision_risk — it is not an absolute withhold gate.
/// - Low-reliability fields stay available as corroboration / weighted extras.
pub fn commercial_projection(fields: &Value) -> Value {
    let mut materials = trust_materials(fields);
    let stack = crate::stack_auth::stack_auth_from_fields(fields);
    let soft_path = stack.soft_stack
        || crate::stack_auth::is_soft_renderer_class(&stack.renderer_class);
    // Spoofed GPU labels must not split commercial id (fingerprint injectors).
    if stack.gpu_label_untrusted {
        materials.remove("gpu_vendor_class");
    }
    let fo = fields.as_object().cloned().unwrap_or_default();

    // Soft path commercial anchors (generic, live measurements — not sample libraries):
    // residual_mean 1e-5 bucket (stable across Chromium channels on one soft stack;
    // separates 0.500423 vs 0.500427-class FE noise) + form + machine extras.
    // Audio OfflineAudio diverges across engines — soft path does not hash fine audio.
    // Machine extras (cores/os/LAN webrtc/mem) separate host soft ≠ guest soft when residual
    // alone would collide (SwiftShader cross-machine identity).
    let mut soft_residual_bucket: Option<String> = None;
    if soft_path {
        if let Some(m) = fo
            .get("residual_mean")
            .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
        {
            // 1e-5 bucket: same-host FE micro-noise collapses; finer than 1e-4 so
            // 0.500423 vs 0.500427 do not merge.
            let b = (m * 100000.0).round() / 100000.0;
            soft_residual_bucket = Some(format!("rm_{b:.5}"));
            materials.insert(
                "soft_residual_bucket".into(),
                json!(soft_residual_bucket.as_ref().unwrap()),
            );
        }
        if let Some(d) = curve_stable_digest_coarse(&extract_curve(&fo, "hw_curve_webgl", "webgl")) {
            materials.insert("hw_webgl_stable".into(), json!(d));
        }
        materials.remove("hw_audio_stable");
        if let Some(d) = curve_stable_digest_coarse(&extract_curve(&fo, "hw_curve_audio", "audio")) {
            materials.insert("hw_audio_soft_diag".into(), json!(d));
        }
        // mem_class diagnostic only (not commercial — under-reports across engines)
        if let Some(mem) = fo
            .get("device_memory")
            .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
        {
            let mc = if mem <= 4.0 {
                "m0"
            } else if mem <= 16.0 {
                "m1"
            } else {
                "m2"
            };
            materials.insert("mem_class".into(), json!(mc));
        }
        // Demo D41 unit surface — versioned path only (gr_unit_v1 + stable) may enter digest.
        // Unversioned / unstable: conf corroboration only (avoids silent FE upgrade fork).
        if let Some(us) = fo.get("unit_surface_id").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
        {
            materials.insert("unit_surface_id".into(), json!(us));
            if unit_surface_versioned_ok(&fo) {
                materials.insert("unit_surface_versioned".into(), json!(true));
            }
        }
    } else {
        // Real residual path: versioned multi-seed unit binds class across browsers.
        if let Some(us) = fo.get("unit_surface_id").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
        {
            materials.insert("unit_surface_id".into(), json!(us));
            if unit_surface_versioned_ok(&fo) {
                materials.insert("unit_surface_versioned".into(), json!(true));
            }
        }
    }
    // Multi-fn residual ensemble + timing phase (anti same-SKU single-curve collision)
    for (k, v) in crate::hw_anti_collision::anti_collision_materials(&Value::Object(fo.clone())) {
        materials.insert(k, v);
    }
    // Commercial multi-path silicon fusion (lane_c + lane_s + audio/webgpu/timing)
    for (k, v) in crate::hw_silicon_fusion::silicon_fusion_materials(&Value::Object(fo.clone())) {
        // Do not overwrite already-set hw_webgl_stable with fused tag; only fill gaps
        if k == "hw_webgl_stable_fused" {
            if !materials.contains_key("hw_webgl_stable") {
                if let Some(s) = v.as_str() {
                    materials.insert("hw_webgl_stable".into(), json!(s));
                }
            }
            materials.insert(k, v);
            continue;
        }
        if k == "hw_timing_phase_digest" && materials.contains_key("hw_timing_phase_digest") {
            continue;
        }
        materials.insert(k, v);
    }
    // Per-channel census (ops + fusion health)
    for (k, v) in crate::hw_channel_census::channel_census_materials(&Value::Object(fo.clone())) {
        materials.insert(k, v);
    }
    let scores = trust_scores_for(&materials);

    // Core digest path: form_class + curve anchors (real silicon primary path).
    let mut included: Vec<String> = Vec::new();
    let mut trust_sum = 0.0;
    // Deferred posture codes (filled during digest assembly, merged into analysis_posture).
    let mut digest_posture: Vec<String> = Vec::new();
    if soft_path {
        // Soft commercial order: form + residual_bucket **and** coarse webgl when both
        // exist (host soft ≠ guest soft more often than residual alone). Not exclusive OR.
        if materials.contains_key("form_class") {
            included.push("form_class".into());
            trust_sum += material_trust_prior("form_class");
        }
        if materials.contains_key("soft_residual_bucket") {
            included.push("soft_residual_bucket".into());
            trust_sum += 0.80; // residual measurement as soft hard-anchor
        }
        if materials.contains_key("hw_webgl_stable") {
            included.push("hw_webgl_stable".into());
            trust_sum += material_trust_prior("hw_webgl_stable") * 0.85;
        }
    } else {
        for k in commercial_digest_order() {
            if !materials.contains_key(k) {
                continue;
            }
            let t = scores
                .get(k)
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            // Secondary keys use a lower floor so they enter digest when present.
            let floor = if matches!(
                k,
                "cores_class" | "architecture" | "screen_w_class" | "screen_h_class" | "timezone"
            ) {
                0.35
            } else {
                COMMERCIAL_TRUST_FLOOR
            };
            if t + f64::EPSILON < floor {
                continue;
            }
            included.push(k.to_string());
            trust_sum += t;
        }
        // Real-path dual silicon anchors (2026-07-31 prod):
        // - Do NOT drop coarse audio solely because residual_entropy_ok — dropping
        //   audio left only class-level webgl digests (one dh_* across many IP/VT).
        // - Coarse audio stays cross-browser-stable enough for commercial; fine audio
        //   remains conf-only (never in commercial_digest_order).
        // - Host separators (webrtc / os_instance) always fold into digest when present
        //   (not only on dead residual).
        let webgl_entropy_ok = materials
            .get("webgl_residual_entropy_ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let has_webgl = included.iter().any(|k| k == "hw_webgl_stable");
        let has_audio = included.iter().any(|k| k == "hw_audio_stable");
        if has_webgl && has_audio {
            // Keep both anchors; slight extra mass when residual is healthy.
            trust_sum += if webgl_entropy_ok {
                material_trust_prior("hw_audio_stable") * 0.10
            } else {
                material_trust_prior("hw_audio_stable") * 0.15
            };
        } else if has_webgl && !webgl_entropy_ok {
            // Dead residual, audio missing: canvas may help class separation (conf mass).
            if materials.contains_key("hw_canvas_stable")
                && !included.iter().any(|k| k == "hw_canvas_stable")
            {
                let t = scores
                    .get("hw_canvas_stable")
                    .and_then(|v| v.as_f64())
                    .unwrap_or_else(|| material_trust_prior("hw_canvas_stable"));
                if t + f64::EPSILON >= 0.35 {
                    included.push("hw_canvas_stable".into());
                    trust_sum += t * 0.75;
                }
            }
        }
        // Real path: always hash host separators when collected (LAN / OS instance).
        for sep in ["webrtc_host_ip_hash", "os_instance_hash"] {
            if !materials.contains_key(sep) {
                continue;
            }
            if included.iter().any(|k| k == sep) {
                continue;
            }
            let t = scores
                .get(sep)
                .and_then(|v| v.as_f64())
                .unwrap_or_else(|| material_trust_prior(sep));
            if t + f64::EPSILON >= 0.20 {
                included.push(sep.to_string());
                trust_sum += if sep == "webrtc_host_ip_hash" {
                    0.70
                } else {
                    0.55
                };
            }
        }
        // Multi-path silicon fusion + anti-collision secondary (near-silicon same-SKU).
        // Never residual-alone UV. Prefer fusion composite when separability is high.
        let fusion_ok = materials
            .get("hw_silicon_fusion_secondary_ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || materials
                .get("hw_anti_collision_secondary_ok")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
        let sep = materials
            .get("same_sku_separability")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        if fusion_ok {
            for k in [
                "hw_silicon_fusion",
                "hw_silicon_fine",
                "hw_anti_collision",
                "hw_ensemble_digest",
                "hw_timing_phase_digest",
            ] {
                if !materials.contains_key(k) || included.iter().any(|x| x == k) {
                    continue;
                }
                let multi_fn = materials
                    .get("hw_anti_collision_surface")
                    .and_then(|v| v.get("multi_fn_disagrees"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let allow = match k {
                    "hw_silicon_fusion" => sep >= 0.35 || !webgl_entropy_ok,
                    "hw_silicon_fine" => sep >= 0.4 || !webgl_entropy_ok,
                    "hw_anti_collision" => true,
                    "hw_ensemble_digest" => !webgl_entropy_ok || multi_fn,
                    "hw_timing_phase_digest" => !webgl_entropy_ok || sep >= 0.5,
                    _ => false,
                };
                if allow {
                    included.push(k.into());
                    trust_sum += match k {
                        "hw_silicon_fusion" => 0.88,
                        "hw_silicon_fine" => 0.78,
                        "hw_anti_collision" => 0.72,
                        "hw_ensemble_digest" => 0.55,
                        "hw_timing_phase_digest" => 0.48,
                        _ => 0.4,
                    };
                    digest_posture.push(format!("silicon_fusion_include_{k}"));
                }
            }
        }
        // Dual silicon anchors present → drop soft surface secondaries that fork same-host
        // multi-browser (timezone, viewport, cores under-report, **architecture UA-CH noise**,
        // **engine-noisy host seps**). Host seps remain conf materials for ladder / remint,
        // but must NOT enter commercial digest when residual entropy is healthy — lab evidence:
        // blink/gecko share wg_+cc_ but fork dh_ on webrtc/os_instance/architecture alone.
        // Safety: only demote host seps when residual_entropy_ok (dead residual needs host sep).
        let has_webgl2 = included.iter().any(|k| k == "hw_webgl_stable");
        let has_audio2 = included.iter().any(|k| k == "hw_audio_stable");
        let residual_entropy_ok = materials
            .get("webgl_residual_entropy_ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if has_webgl2 && has_audio2 {
            let before = included.len();
            let demote_host_seps = residual_entropy_ok;
            included.retain(|k| {
                !matches!(
                    k.as_str(),
                    "timezone" | "screen_w_class" | "screen_h_class" | "cores_class" | "architecture"
                ) && !(demote_host_seps
                    && matches!(k.as_str(), "webrtc_host_ip_hash" | "os_instance_hash"))
            });
            if included.len() != before {
                trust_sum = included
                    .iter()
                    .map(|k| {
                        scores
                            .get(k)
                            .and_then(|v| v.as_f64())
                            .unwrap_or_else(|| material_trust_prior(k))
                    })
                    .sum::<f64>();
                // Host seps still contribute conf mass when kept; when demoted, note posture.
                if included.iter().any(|k| k == "webrtc_host_ip_hash") {
                    trust_sum += 0.15;
                }
                if included.iter().any(|k| k == "os_instance_hash") {
                    trust_sum += 0.10;
                }
            }
            digest_posture.push("dual_silicon_demote_cores_tz_screen".into());
            if demote_host_seps {
                digest_posture.push("dual_silicon_host_sep_conf_only".into());
            }
            digest_posture.push("dual_silicon_architecture_conf_only".into());
        }
    }
    // Soft path: hash machine-stable extras (cores/os/LAN webrtc/mem) — not GPU labels.
    let mut soft_extras_included: Vec<String> = Vec::new();
    if soft_path {
        for k in soft_commercial_extra() {
            if !materials.contains_key(k) {
                continue;
            }
            // webrtc_host_ip_hash prior is high; still include when present (LAN separator).
            let t = scores.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let floor = if k == "webrtc_host_ip_hash" {
                0.20
            } else if k == "architecture" {
                0.30
            } else {
                0.40
            };
            if t + f64::EPSILON < floor {
                continue;
            }
            if !included.iter().any(|x| x == k) {
                included.push(k.to_string());
                soft_extras_included.push(k.to_string());
            }
            trust_sum += if k == "webrtc_host_ip_hash" {
                0.70
            } else {
                t * 0.85
            };
        }
        // unit_surface: conf always; hashed only when versioned (gr_unit_v1 + stable).
        if materials.contains_key("unit_surface_id") {
            trust_sum += 0.20;
            if materials.get("unit_surface_versioned").and_then(|v| v.as_bool()) == Some(true)
                && !included.iter().any(|x| x == "unit_surface_id")
            {
                included.push("unit_surface_id".into());
                soft_extras_included.push("unit_surface_id".into());
                trust_sum += 0.25;
            }
        }
        trust_sum += 0.15;
    } else if materials.contains_key("unit_surface_id") {
        trust_sum += 0.25;
        // Real path versioned unit → secondary digest material (cross-browser class bind).
        if materials.get("unit_surface_versioned").and_then(|v| v.as_bool()) == Some(true)
            && !included.iter().any(|x| x == "unit_surface_id")
        {
            included.push("unit_surface_id".into());
            trust_sum += 0.20;
        }
    }
    // Corroboration only (not in commercial digest unless already soft-extra hashed):
    let mut corroboration: Vec<String> = Vec::new();
    if materials.contains_key("unit_surface_id") {
        corroboration.push("unit_surface_id".into());
    }
    for k in [
        "webrtc_host_ip_hash",
        "architecture",
        "storage_quota_class",
        "os_family",
        "cores_class",
        "mem_class",
    ] {
        if materials.contains_key(k) {
            if soft_path && soft_extras_included.iter().any(|x| x == k) {
                continue;
            }
            let t = scores.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
            trust_sum += t * 0.20;
            corroboration.push(k.to_string());
        }
    }
    let anchors = hardware_anchors();
    let has_curve_anchor = included.iter().any(|k| {
        anchors.iter().any(|a| *a == k.as_str()) || k == "soft_residual_bucket"
    });
    let has_form = included.iter().any(|k| k == "form_class");
    // Presence in materials (not only digest keys): webgl-primary may drop audio from included.
    let has_both_curves = materials.contains_key("hw_audio_stable")
        && materials.contains_key("hw_webgl_stable");
    // Soft commercial: host separator improves entropy / clears collision_risk.
    // Align with ServerMint / association ladder: **webrtc_host_ip_hash OR os_instance_hash**.
    // Without either we still emit commercial id with collision_risk posture (demo multi-VM).
    let soft_has_host_separator = included.iter().any(|k| k == "webrtc_host_ip_hash")
        || soft_extras_included
            .iter()
            .any(|k| k == "webrtc_host_ip_hash")
        || fo
            .get("webrtc_host_ip_hash")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty())
        || fo
            .get("os_instance_hash")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty());
    // Emit commercial id when form + curve/residual anchors present (always prefer emit).
    // Soft path no longer withholds id when LAN separator missing — marks collision instead.
    // Pure empty bag: never mint commercial id (no form, no curve anchors).
    // Pure empty bag: never mint commercial id without form + curve/residual anchors.
    let empty_anchor = !has_form || !has_curve_anchor;
    let eligible = if empty_anchor {
        false
    } else if soft_path {
        has_form && has_curve_anchor && trust_sum + 1e-9 >= 1.0
    } else {
        has_form
            && has_curve_anchor
            && (has_both_curves || trust_sum + 1e-9 >= COMMERCIAL_TRUST_SUM_MIN + 0.5)
            && trust_sum + 1e-9 >= COMMERCIAL_TRUST_SUM_MIN
    };
    // Lightweight real path: form + single strong anchor still productizes (dv-class materials).
    let eligible = eligible
        || (!empty_anchor
            && !soft_path
            && has_form
            && has_curve_anchor
            && trust_sum + 1e-9 >= COMMERCIAL_TRUST_FLOOR + 0.5);
    let mut no_id_reasons =
        eligibility_reason_codes(&materials, &scores, &included, trust_sum, eligible);
    if empty_anchor && !no_id_reasons.iter().any(|r| r == "empty_anchor_no_mint") {
        no_id_reasons.push("empty_anchor_no_mint".into());
    }
    // Soft/analysis posture codes (always annotated when relevant — not only when ineligible).
    let mut analysis_posture: Vec<String> = Vec::new();
    if soft_path {
        analysis_posture.push("soft_stack_digest_path".into());
        if !soft_has_host_separator {
            analysis_posture.push("soft_collision_risk_no_host_separator".into());
            analysis_posture.push("soft_low_entropy_use_weighted_fusion".into());
        } else {
            analysis_posture.push("soft_host_separator_present".into());
        }
    }
    if stack.gpu_label_untrusted {
        analysis_posture.push("gpu_label_untrusted_weighted_not_digest_primary".into());
    }
    if soft_path {
        if !no_id_reasons.iter().any(|r| r == "soft_stack_digest_path") {
            no_id_reasons.push("soft_stack_digest_path".into());
        }
        // Keep legacy code name for ops dashboards that still filter this string —
        // meaning is now posture/collision, not "id withheld".
        if !soft_has_host_separator
            && !no_id_reasons
                .iter()
                .any(|r| r == "soft_low_entropy_no_host_separator")
        {
            no_id_reasons.push("soft_low_entropy_no_host_separator".into());
        }
    }
    if stack.gpu_label_untrusted {
        if !no_id_reasons
            .iter()
            .any(|r| r == "gpu_label_untrusted_spoof_or_soft_residual")
        {
            no_id_reasons.push("gpu_label_untrusted_spoof_or_soft_residual".into());
        }
    }
    // Soft: collision without host separator.
    // Real:
    //  - dead residual without host sep → class risk
    //  - form + class-level wg_coarse only (no second silicon, no host sep) → class risk
    //    even when residual entropy looks OK (shared commercial webgl template).
    // Does NOT force demote-as-uniqueness — uniqueness still comes from materials.
    let webgl_entropy_flag = materials
        .get("webgl_residual_entropy_ok")
        .and_then(|v| v.as_bool());
    let webgl_low_entropy = webgl_entropy_flag != Some(true);
    let real_host_sep = materials.contains_key("webrtc_host_ip_hash")
        || fo
            .get("webrtc_host_ip_hash")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty())
        || fo
            .get("os_instance_hash")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty())
        || included.iter().any(|k| k == "webrtc_host_ip_hash" || k == "os_instance_hash");
    let has_wg_in = included.iter().any(|k| k == "hw_webgl_stable");
    let has_au_in = included.iter().any(|k| k == "hw_audio_stable");
    // form + wg_coarse class path: only one silicon anchor and no host sep.
    let form_wg_class_only = has_form
        && has_wg_in
        && !has_au_in
        && !real_host_sep
        && !included.iter().any(|k| k == "os_instance_hash");
    let ac_prone = materials
        .get("hw_single_curve_collision_prone")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let ac_secondary = materials
        .get("hw_anti_collision_secondary_ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let collision_risk = if soft_path {
        !soft_has_host_separator
    } else {
        // Dead residual / form-class without host sep → risk.
        // Single-curve prone without anti-collision secondary → risk.
        // When multi-fn ensemble or timing secondary is present, risk may clear for digest
        // uniqueness but association still reports class risk via surface.
        ((webgl_low_entropy && !real_host_sep) || form_wg_class_only)
            || (ac_prone && !ac_secondary && !real_host_sep)
    };
    analysis_posture.extend(digest_posture.iter().cloned());
    if webgl_low_entropy {
        analysis_posture.push("real_webgl_residual_low_entropy".into());
        if !real_host_sep {
            analysis_posture.push("real_collision_risk_dead_residual_no_host_sep".into());
        }
    }
    if form_wg_class_only {
        analysis_posture.push("real_collision_risk_form_wg_coarse_class".into());
    }
    if has_au_in && has_wg_in {
        analysis_posture.push("real_dual_silicon_anchors".into());
    }
    if real_host_sep && !soft_path {
        analysis_posture.push("real_host_separator_in_digest".into());
    }
    // Soft model signature: always available when soft anchors exist (diagnostics /
    // model-level clustering — NOT commercial UV).
    let soft_model_hash = if soft_path && has_form && has_curve_anchor {
        Some(hash_selected_soft(&materials, &included))
    } else {
        None
    };
    let device_model_id = soft_model_hash
        .as_ref()
        .map(|dig| format!("dm_{}", &dig[..16.min(dig.len())]));
    let device_id = if eligible {
        let dig = if soft_path {
            // soft_aware_v4: same soft hash namespace; commercial only with host separator
            soft_model_hash
                .clone()
                .unwrap_or_else(|| hash_selected_soft(&materials, &included))
        } else {
            let order = commercial_digest_order();
            let mut keys: Vec<&str> = order
                .iter()
                .copied()
                .filter(|k| included.iter().any(|x| x == *k))
                .collect();
            // Versioned unit surface: secondary digest material (real_curves_unit_v1).
            if included.iter().any(|x| x == "unit_surface_id")
                && materials.contains_key("unit_surface_id")
                && !keys.contains(&"unit_surface_id")
            {
                keys.push("unit_surface_id");
            }
            // Dead-residual host separators (not in default commercial order).
            for sep in ["webrtc_host_ip_hash", "os_instance_hash"] {
                if included.iter().any(|x| x == sep)
                    && materials.contains_key(sep)
                    && !keys.contains(&sep)
                {
                    keys.push(sep);
                }
            }
            hash_selected(&materials, &keys)
        };
        Some(format!("dv_{}", &dig[..16.min(dig.len())]))
    } else {
        None
    };
    // Candidate: same core order for diagnostics
    let order = commercial_digest_order();
    let cand_keys: Vec<&str> = order
        .iter()
        .copied()
        .filter(|k| {
            materials.contains_key(*k)
                && scores.get(*k).and_then(|v| v.as_f64()).unwrap_or(0.0) >= COMMERCIAL_TRUST_FLOOR
        })
        .collect();
    let candidate = if cand_keys.len() >= 2 {
        let dig = hash_selected(&materials, &cand_keys);
        Some(format!("dc_{}", &dig[..16]))
    } else {
        None
    };
    let mut id_warnings: Vec<String> = Vec::new();
    if soft_path {
        id_warnings.push("soft_stack_digest_path".into());
    }
    if stack.gpu_label_untrusted {
        id_warnings.push("gpu_label_untrusted".into());
        id_warnings.push("gpu_label_used_as_weighted_claim_not_digest_primary".into());
    }
    if soft_path && !soft_has_host_separator {
        id_warnings.push("soft_low_entropy_no_host_separator".into());
        id_warnings.push("collision_risk_soft_class".into());
        // Prefer emit commercial id + model id; do not suppress product identity.
        id_warnings.push("commercial_id_emitted_with_collision_posture".into());
    }
    if webgl_low_entropy {
        id_warnings.push("webgl_residual_low_entropy".into());
        if collision_risk {
            id_warnings.push("collision_risk_real_dead_residual".into());
        }
    }
    if form_wg_class_only {
        id_warnings.push("collision_risk_form_wg_coarse_class".into());
        id_warnings.push("commercial_id_emitted_with_collision_posture".into());
    }
    let unit_in_digest = included.iter().any(|k| k == "unit_surface_id");
    let digest_path_label = if soft_path && unit_in_digest {
        "soft_aware_v4_unit_v1"
    } else if soft_path {
        "soft_aware_v4"
    } else if unit_in_digest {
        "real_curves_unit_v1"
    } else {
        "real_curves_v1"
    };
    json!({
        "device_id": device_id,
        "device_model_id": device_model_id,
        "device_id_candidate": candidate,
        "algo": COMMERCIAL_ALGO,
        "trust_sum": (trust_sum * 10000.0).round() / 10000.0,
        "trust_floor": COMMERCIAL_TRUST_FLOOR,
        "trust_sum_min": COMMERCIAL_TRUST_SUM_MIN,
        "corroboration": corroboration,
        "webrtc_in_digest": included.iter().any(|k| k == "webrtc_host_ip_hash")
            || (soft_path
                && soft_extras_included
                    .iter()
                    .any(|k| k == "webrtc_host_ip_hash")),
        "soft_has_host_separator": soft_path && soft_has_host_separator,
        "collision_risk": collision_risk,
        "webgl_residual_entropy_ok": materials
            .get("webgl_residual_entropy_ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        "analysis_posture": analysis_posture,
        "eligible": eligible,
        "has_hardware_anchor": has_curve_anchor,
        "has_both_curves": has_both_curves,
        "has_os_family": materials.contains_key("os_family"),
        "materials_included": included,
        "soft_extras_included": soft_extras_included,
        "materials": materials,
        "trust_scores": scores,
        "server_client_ip_in_digest": false,
        // When eligible, keep posture codes in eligibility_reasons for ops; no_id empty.
        "no_id_reasons": if eligible { Vec::<String>::new() } else { no_id_reasons.clone() },
        "eligibility_reasons": if eligible {
            analysis_posture.clone()
        } else {
            no_id_reasons
        },
        "soft_stack": stack.soft_stack,
        "gpu_label_untrusted": stack.gpu_label_untrusted,
        "stack_class": stack.stack_class,
        "renderer_class": stack.renderer_class,
        // Versioned unit path marker when multi-seed unit entered digest materials.
        "digest_path": digest_path_label,
        "unit_surface_versioned": materials
            .get("unit_surface_versioned")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        "unit_surface_algo": fo
            .get("unit_surface_algo")
            .cloned()
            .unwrap_or(Value::Null),
        "id_warnings": id_warnings,
    })
}

/// FE unit surface may enter commercial digest only when algo version matches and stable.
fn unit_surface_versioned_ok(fo: &Map<String, Value>) -> bool {
    let algo = fo
        .get("unit_surface_algo")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let stable = fo
        .get("unit_multiround_stable")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    stable && (algo == "gr_unit_v1" || algo.starts_with("gr_unit_v"))
}

/// Soft-aware hash: soft namespace + ordered included materials only.
/// Does **not** mix GPU labels. WebRTC host hash is included when present (separator).
fn hash_selected_soft(materials: &Map<String, Value>, included: &[String]) -> String {
    let mut hasher = Sha256::new();
    // v4_unit namespace when versioned unit_surface participates; else v3 soft path.
    let unit_in = included.iter().any(|x| x == "unit_surface_id");
    if unit_in {
        hasher.update(b"mt2|soft_v4_unit_v1|");
    } else {
        // v3 namespace: architecture added to order; commercial gate is separate.
        hasher.update(b"mt2|soft_v3|");
    }
    // Deterministic key order for soft commercial / model materials
    const SOFT_HASH_ORDER: &[&str] = &[
        "form_class",
        "soft_residual_bucket",
        "hw_webgl_stable",
        "cores_class",
        "os_family",
        "architecture",
        "webrtc_host_ip_hash",
        "unit_surface_id",
    ];
    for k in SOFT_HASH_ORDER {
        if !included.iter().any(|x| x == *k) {
            continue;
        }
        if let Some(v) = materials.get(*k) {
            hasher.update(k.as_bytes());
            hasher.update(b"=");
            hasher.update(v.as_str().unwrap_or(&v.to_string()).as_bytes());
            hasher.update(b"|");
        }
    }
    format!("{:x}", hasher.finalize())
}

/// Commercial device_id only (Option) — replaces edge-IP digest path for multi-env uniqueness.
pub fn commercial_device_id_trusted(fields: &Value) -> Option<String> {
    commercial_projection(fields)
        .get("device_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Ops-facing materials breakdown (digest keys + key material values + posture).
/// Safe for `/v1/ops/device?materials=1` and offline replay — no secrets / IP hashing.
pub fn materials_detail(fields: &Value) -> Value {
    let proj = commercial_projection(fields);
    let mats = proj
        .get("materials")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let included = proj
        .get("materials_included")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let pick = |k: &str| mats.get(k).cloned().unwrap_or(Value::Null);
    json!({
        "algo": COMMERCIAL_ALGO,
        "webgl_comm_algo": "webgl_comm_v4",
        "device_id": proj.get("device_id").cloned().unwrap_or(Value::Null),
        "device_model_id": proj.get("device_model_id").cloned().unwrap_or(Value::Null),
        "eligible": proj.get("eligible").cloned().unwrap_or(json!(false)),
        "collision_risk": proj.get("collision_risk").cloned().unwrap_or(json!(false)),
        "webgl_residual_entropy_ok": proj.get("webgl_residual_entropy_ok").cloned().unwrap_or(json!(false)),
        "digest_path": proj.get("digest_path").cloned().unwrap_or(Value::Null),
        "trust_sum": proj.get("trust_sum").cloned().unwrap_or(Value::Null),
        "materials_included": included,
        "soft_extras_included": proj.get("soft_extras_included").cloned().unwrap_or_else(|| json!([])),
        "analysis_posture": proj.get("analysis_posture").cloned().unwrap_or_else(|| json!([])),
        "id_warnings": proj.get("id_warnings").cloned().unwrap_or_else(|| json!([])),
        "webrtc_in_digest": proj.get("webrtc_in_digest").cloned().unwrap_or(json!(false)),
        "has_both_curves": proj.get("has_both_curves").cloned().unwrap_or(json!(false)),
        "keys": {
            "form_class": pick("form_class"),
            "hw_webgl_stable": pick("hw_webgl_stable"),
            "hw_webgl_fine": pick("hw_webgl_fine"),
            "hw_webgl_peak_sig": pick("hw_webgl_peak_sig"),
            "hw_eu_timing_digest": pick("hw_eu_timing_digest"),
            "hw_webgpu_compute_digest": pick("hw_webgpu_compute_digest"),
            "hw_audio_stable": pick("hw_audio_stable"),
            "hw_audio_fine": pick("hw_audio_fine"),
            "hw_canvas_stable": pick("hw_canvas_stable"),
            "hw_silicon_fine": pick("hw_silicon_fine"),
            "hw_silicon_fusion": pick("hw_silicon_fusion"),
            "hw_anti_collision": pick("hw_anti_collision"),
            "hw_timing_phase_digest": pick("hw_timing_phase_digest"),
            "same_sku_separability": pick("same_sku_separability"),
            "hw_silicon_grade": pick("hw_silicon_grade"),
            "residual_mean": pick("residual_mean"),
            "residual_mean_bucket": pick("residual_mean_bucket"),
            "soft_residual_bucket": pick("soft_residual_bucket"),
            "residual_lanes": pick("residual_lanes"),
            "cores_class": pick("cores_class"),
            "architecture": pick("architecture"),
            "os_family": pick("os_family"),
            "os_instance_hash": pick("os_instance_hash"),
            "webrtc_host_ip_hash": pick("webrtc_host_ip_hash"),
            "unit_surface_id": pick("unit_surface_id"),
            "timezone": pick("timezone"),
            "screen_w_class": pick("screen_w_class"),
            "screen_h_class": pick("screen_h_class"),
        },
        "hw_channel_census": pick("hw_channel_census"),
        "hw_silicon_fusion_surface": pick("hw_silicon_fusion_surface"),
        "note": "Lane-C commercial + Lane-S/fusion near-silicon; census compares every HW channel",
    })
}

#[cfg(test)]
mod engine_aware_tests {
    use super::*;

    fn synth_curve(base: f64, n: usize) -> Vec<f64> {
        (0..n)
            .map(|i| {
                let t = i as f64 * 0.17 + base;
                (t.sin().abs() * 0.35 + 0.12 + (i % 5) as f64 * 0.01).min(0.99)
            })
            .collect()
    }

    #[test]
    fn multipath_fuse_splits_same_mean_different_modes() {
        let float_c = synth_curve(0.260, 32);
        let noderiv_c = synth_curve(0.260, 32)
            .into_iter()
            .enumerate()
            .map(|(i, v)| v + if i % 7 == 0 { 0.004 } else { 0.0 })
            .collect::<Vec<_>>();
        let rint_c = synth_curve(0.261, 32);
        let mut fo = Map::new();
        fo.insert(
            "residual_paths".into(),
            json!([
                {
                    "path_id": "v3f_128_std",
                    "ok": true,
                    "entropy_ok": true,
                    "mean": 0.260,
                    "std": 0.09,
                    "shader_mode": "float",
                    "curve": float_c,
                },
                {
                    "path_id": "v3f_128_noderiv",
                    "ok": true,
                    "entropy_ok": true,
                    "mean": 0.260,
                    "std": 0.09,
                    "shader_mode": "noderiv",
                    "curve": noderiv_c,
                },
                {
                    "path_id": "rint_warm2_128",
                    "ok": true,
                    "entropy_ok": true,
                    "mean": 0.261,
                    "std": 0.09,
                    "shader_mode": "rint",
                    "curve": rint_c,
                }
            ]),
        );
        let fuse = multipath_commercial_fuse(&fo, Some("blink")).expect("fuse");
        assert_eq!(fuse["algo"], "webgl_comm_v5_multipath_fuse");
        assert!(fuse["n_paths_fused"].as_u64().unwrap_or(0) >= 2);
        let dig = fuse["digest"].as_str().unwrap_or("");
        assert!(dig.starts_with("wg_"), "digest={dig}");
        // single path must NOT fuse
        let mut fo1 = Map::new();
        fo1.insert(
            "residual_paths".into(),
            json!([{
                "path_id": "only",
                "ok": true,
                "entropy_ok": true,
                "mean": 0.26,
                "shader_mode": "float",
                "curve": synth_curve(0.26, 32),
            }]),
        );
        assert!(multipath_commercial_fuse(&fo1, None).is_none());
    }

    #[test]
    fn dual_lane_prefers_rint_for_c_and_ulp_for_s() {
        let float_c = synth_curve(0.26, 32);
        let rint_c = synth_curve(0.261, 32);
        let ulp_s = synth_curve(0.33, 32);
        let mut fo = Map::new();
        fo.insert(
            "residual_paths".into(),
            json!([
                {
                    "path_id": "warm4_v3f_128",
                    "ok": true,
                    "entropy_ok": true,
                    "size": 128,
                    "warm_frames": 4,
                    "mean": 0.260,
                    "std": 0.09,
                    "shader_mode": "float",
                    "curve": float_c,
                },
                {
                    "path_id": "rint_warm4_128",
                    "ok": true,
                    "entropy_ok": true,
                    "size": 128,
                    "warm_frames": 4,
                    "mean": 0.260,
                    "std": 0.09,
                    "shader_mode": "rint",
                    "curve": rint_c.clone(),
                },
                {
                    "path_id": "ulp_eu_timing",
                    "ok": true,
                    "entropy_ok": true,
                    "size": 128,
                    "warm_frames": 0,
                    "mean": 0.33,
                    "std": 0.12,
                    "std_fine": 0.08,
                    "shader_mode": "ulp",
                    "eu_timing_ms": [1.1, 1.2, 1.15, 1.18, 1.22, 1.19, 1.21, 1.17],
                    "curve": ulp_s.clone(),
                }
            ]),
        );
        let (c_curve, meta) = select_residual_curve_from_paths(&fo);
        assert_eq!(meta["policy"], "residual_dual_lane_v1");
        assert_eq!(meta["lane_c"]["chosen_path_id"], "rint_warm4_128");
        assert_eq!(meta["lane_s"]["chosen_path_id"], "ulp_eu_timing");
        assert_eq!(c_curve, rint_c);
        assert!(
            meta.get("lane_s_curve")
                .and_then(|v| v.as_array())
                .map(|a| a.len() >= 8)
                .unwrap_or(false),
            "lane_s_curve present for fine material mint"
        );
        // Commercial materials: Lane-C digest; fine/eu from S; no full curve in residual_select store path
        let mats = trust_materials(&json!({
            "residual_paths": fo.get("residual_paths").cloned().unwrap(),
            "hw_curve_webgl": synth_curve(0.1, 32),
            "hw_curve_audio": synth_curve(0.5, 64),
        }));
        assert!(mats.get("hw_webgl_stable").is_some());
        assert!(mats.get("hw_webgl_fine").is_some());
        assert!(mats.get("hw_eu_timing_digest").is_some());
        assert_eq!(
            mats["residual_lanes"]["commercial_from"],
            "lane_c"
        );
        assert_eq!(mats["residual_lanes"]["fine_from"], "lane_s");
        assert!(
            mats.get("residual_select")
                .and_then(|v| v.get("lane_s_curve"))
                .is_none(),
            "full lane_s_curve must not bloat materials residual_select"
        );
    }

    #[test]
    fn dual_lane_prefers_noderiv_hard_over_float_warm() {
        let float_c = synth_curve(0.260039, 32);
        let noderiv_c = synth_curve(0.260035, 32);
        let mut fo = Map::new();
        fo.insert(
            "residual_paths".into(),
            json!([
                {
                    "path_id": "unk_warm4_std",
                    "ok": true,
                    "entropy_ok": true,
                    "size": 128,
                    "warm_frames": 4,
                    "mean": 0.260039,
                    "std": 0.09,
                    "shader_mode": "float",
                    "curve": float_c,
                },
                {
                    "path_id": "noderiv_hard_warm4",
                    "ok": true,
                    "entropy_ok": true,
                    "size": 128,
                    "warm_frames": 4,
                    "mean": 0.260035,
                    "std": 0.09,
                    "shader_mode": "noderiv",
                    "curve": noderiv_c.clone(),
                }
            ]),
        );
        let (c_curve, meta) = select_residual_curve_from_paths(&fo);
        assert_eq!(meta["lane_c"]["chosen_path_id"], "noderiv_hard_warm4");
        assert_eq!(c_curve, noderiv_c);
    }

    #[test]
    fn dual_lane_never_picks_ulp_for_commercial_lane_c() {
        let ulp = synth_curve(0.4, 32);
        let float_c = synth_curve(0.26, 32);
        let mut fo = Map::new();
        fo.insert(
            "residual_paths".into(),
            json!([
                {
                    "path_id": "ulp_chain_128",
                    "ok": true,
                    "entropy_ok": true,
                    "size": 128,
                    "warm_frames": 4,
                    "mean": 0.4,
                    "std": 0.15,
                    "shader_mode": "ulp",
                    "curve": ulp,
                },
                {
                    "path_id": "v3f_128_std",
                    "ok": true,
                    "entropy_ok": true,
                    "size": 128,
                    "warm_frames": 2,
                    "mean": 0.26,
                    "std": 0.09,
                    "shader_mode": "float",
                    "curve": float_c.clone(),
                }
            ]),
        );
        let (c_curve, meta) = select_residual_curve_from_paths(&fo);
        assert_eq!(meta["lane_c"]["chosen_path_id"], "v3f_128_std");
        assert_eq!(meta["lane_s"]["chosen_path_id"], "ulp_chain_128");
        assert_eq!(c_curve, float_c);
    }

    #[test]
    fn residual_curve_webkit_drops_cold_first_seed_only() {
        let mut curve = vec![0.0_f64; 8];
        // hot seeds
        for i in 0..24 {
            curve.push(0.2 + (i as f64) * 0.001);
        }
        let out = residual_curve_for_engine(&curve, Some("webkit"));
        assert_eq!(out.len(), 32);
        // first seed dropped → first value from second seed
        assert!((out[0] - 0.2).abs() < 1e-9);
        // blink unchanged
        let blink = residual_curve_for_engine(&curve, Some("blink"));
        assert_eq!(blink, curve);
    }

    #[test]
    fn engine_family_prefers_probe_tags() {
        let fo = serde_json::json!({"engine_family":"webkit","engine_claim":"blink"})
            .as_object()
            .cloned()
            .unwrap();
        assert_eq!(engine_family_from_fields(&fo).as_deref(), Some("webkit"));
    }
}
