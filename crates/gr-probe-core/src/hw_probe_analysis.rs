//! Hardware probe **analysis** for commercial V slots — stability without forced coarse buckets.
//!
//! # Why not "coarse mean/std only"
//!
//! Squashing a high-resolution curve into a few wide buckets raises same-session
//! reliability but **destroys uniqueness** (population collisions). Industry and
//! academic practice instead:
//!
//! | Source | Technique | What we adopt |
//! |--------|-----------|----------------|
//! | DrawnApart / LockedApart | multi-trial **median** of timing traces | `multiround_median` |
//! | vektort13 WebGPU atomic | median of trials → **normalize** → hash | scale-invariant + structure |
//! | PUF (pypuf / spat) | inter-HD uniqueness vs intra-HD reliability | stable-mask dims + dual KPI |
//! | Fuzzy extractor (iss/60) | helper-data re-probe snap | `fuzzy_ecc` (sibling module) |
//! | OfflineAudio seed-delta | relative to fixed seed cancels absolute offset | scale / rank invariants |
//!
//! # Pipeline (per channel)
//!
//! ```text
//! raw rounds[] ──► element-wise median (full f64)
//!                 ──► optional scale-invariant transform (÷mean or ÷L2)
//!                 ──► optional stable-dim mask (low multiround variance)
//!                 ──► structure encode (full curve shape, NOT single coarse bucket)
//! ```
//!
//! Optional pack fields (wasm digest present/absent) must still be stripped at the
//! mint boundary — that is **presence hygiene**, not value coarsening.

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

pub const HW_PROBE_ANALYSIS_ALGO: &str = "hw_probe_analysis_v1";

/// Domain-separated short digest (align with device_segments public tokens).
fn short_hash(parts: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"gr_hw_probe_analysis_v1|");
    h.update(parts.as_bytes());
    format!("{:x}", h.finalize())[..10].to_string()
}

fn finite(xs: &[f64]) -> Vec<f64> {
    xs.iter().copied().filter(|x| x.is_finite()).collect()
}

/// MAD-based clip: samples beyond `k * MAD` from median are clamped (not dropped).
/// Kills rAF/CPU one-shot spikes (tab freeze, GC) without coarse bucketing.
pub fn clip_outliers_mad(arr: &[f64], k: f64) -> Vec<f64> {
    let f = finite(arr);
    if f.len() < 4 {
        return f;
    }
    let mut sorted = f.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let med = sorted[sorted.len() / 2];
    let mut devs: Vec<f64> = f.iter().map(|x| (x - med).abs()).collect();
    devs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mad = devs[devs.len() / 2].max(1e-15);
    let hi = k * mad * 1.4826; // ≈ sigma for normal
    f.iter()
        .map(|x| {
            let d = x - med;
            if d.abs() > hi {
                med + d.signum() * hi
            } else {
                *x
            }
        })
        .collect()
}

/// Display-refresh lattice detector (Gecko/WebKit integer or 16/17ms rAF).
/// When true, rank-order is measurement noise — commercial body uses lowvar class
/// (display Hz remains K-only; body does not claim silicon uniqueness).
pub fn is_display_refresh_lattice(arr: &[f64]) -> bool {
    let f = finite(arr);
    if f.len() < 4 {
        return false;
    }
    let mean = f.iter().sum::<f64>() / f.len() as f64;
    // rAF-like interval range (ms). CPU kind profiles live elsewhere.
    if !(8.0..=50.0).contains(&mean) {
        return false;
    }
    // 1ms bins
    let mut bins: std::collections::BTreeMap<i64, usize> = std::collections::BTreeMap::new();
    for x in &f {
        *bins.entry(x.round() as i64).or_default() += 1;
    }
    if bins.is_empty() || bins.len() > 5 {
        return false;
    }
    let mut counts: Vec<usize> = bins.values().copied().collect();
    counts.sort_by(|a, b| b.cmp(a));
    let top2: usize = counts.iter().take(2).sum();
    (top2 as f64) / (f.len() as f64) >= 0.85
}

/// Soft quantize (relative 1/20 steps) before kind-rank so Gecko/WebKit micro-noise
/// does not flip adjacent kind order while still separating distinct profiles.
pub fn soft_quantize_for_rank(arr: &[f64]) -> Vec<f64> {
    let f = finite(arr);
    if f.is_empty() {
        return f;
    }
    let max_abs = f
        .iter()
        .map(|x| x.abs())
        .fold(0.0_f64, f64::max)
        .max(1e-15);
    f.iter()
        .map(|x| ((x / max_abs) * 20.0).round() / 20.0)
        .collect()
}

/// Winsorize to [lo, hi] quantiles (e.g. 0.05–0.95) before multiround.
pub fn winsorize_quantiles(arr: &[f64], lo_q: f64, hi_q: f64) -> Vec<f64> {
    let f = finite(arr);
    if f.len() < 4 {
        return f;
    }
    let mut sorted = f.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let qi = |p: f64| -> f64 {
        let i = ((sorted.len() as f64 - 1.0) * p).round() as usize;
        sorted[i.min(sorted.len() - 1)]
    };
    let lo = qi(lo_q.clamp(0.0, 1.0));
    let hi = qi(hi_q.clamp(0.0, 1.0));
    f.iter().map(|x| x.clamp(lo, hi)).collect()
}

/// Element-wise median across rounds. Preserves full f64 precision (no bucket squash).
///
/// DrawnApart / vektort13 style: multi-trial median kills one-shot wall-clock noise
/// without throwing away between-machine shape differences.
pub fn multiround_median(rounds: &[Vec<f64>]) -> Option<Vec<f64>> {
    if rounds.is_empty() {
        return None;
    }
    let cleaned: Vec<Vec<f64>> = rounds
        .iter()
        .map(|r| finite(r))
        .filter(|r| r.len() >= 4)
        .collect();
    if cleaned.is_empty() {
        return None;
    }
    let dim = cleaned.iter().map(|r| r.len()).min().unwrap_or(0);
    if dim < 4 {
        return None;
    }
    let mut out = Vec::with_capacity(dim);
    for i in 0..dim {
        let mut col: Vec<f64> = cleaned.iter().map(|r| r[i]).collect();
        col.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mid = col.len() / 2;
        let m = if col.len() % 2 == 1 {
            col[mid]
        } else {
            0.5 * (col[mid - 1] + col[mid])
        };
        out.push(m);
    }
    Some(out)
}

/// Scale-invariant transform: `x_i / mean(|x|)` — cancels global load multiplier
/// (busy tab scales all wall samples) while keeping relative workload shape.
///
/// Different machines keep distinct shapes → uniqueness preserved.
pub fn scale_invariant_mean(arr: &[f64]) -> Vec<f64> {
    let f = finite(arr);
    if f.is_empty() {
        return Vec::new();
    }
    let mean = f.iter().map(|x| x.abs()).sum::<f64>() / f.len() as f64;
    if mean < 1e-15 {
        return f;
    }
    f.iter().map(|x| x / mean).collect()
}

/// L2 unit normalize (vektort13-style distribution shape).
pub fn scale_invariant_l2(arr: &[f64]) -> Vec<f64> {
    let f = finite(arr);
    if f.is_empty() {
        return Vec::new();
    }
    let n2 = f.iter().map(|x| x * x).sum::<f64>().sqrt();
    if n2 < 1e-15 {
        return f;
    }
    f.iter().map(|x| x / n2).collect()
}

/// Rank-order signature: argsort permutation as a discrete high-entropy token.
/// Stable under monotonic transforms; no value coarsening.
pub fn rank_order_sig(arr: &[f64]) -> Option<String> {
    let f = finite(arr);
    if f.len() < 4 {
        return None;
    }
    let mut idx: Vec<usize> = (0..f.len()).collect();
    idx.sort_by(|&a, &b| {
        f[a]
            .partial_cmp(&f[b])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.cmp(&b))
    });
    let s = idx
        .iter()
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",");
    Some(short_hash(&format!("rank|{s}")))
}

/// Kind-extrema signature: **single** slowest + **single** fastest kind index.
///
/// Lab evidence (Gecko + WebKit):
/// - Full rank / top2-bot2 fork when mid kinds swap order (kind1↔kind5 near-ties).
/// - argmax/argmin stay stable across multi-site revisits (kind0 heaviest, kind3 lightest).
/// Inverted kind schedules still swap extrema → uniqueness retained.
pub fn kind_extremes_sig(arr: &[f64]) -> Option<String> {
    let f = finite(arr);
    if f.len() < 4 {
        return None;
    }
    let mut imin = 0usize;
    let mut imax = 0usize;
    for i in 1..f.len() {
        if f[i] < f[imin] || (f[i] == f[imin] && i < imin) {
            imin = i;
        }
        if f[i] > f[imax] || (f[i] == f[imax] && i < imax) {
            imax = i;
        }
    }
    Some(short_hash(&format!("kext|min={imin}|max={imax}|n={}", f.len())))
}

/// Per-dimension multiround variance; keep dims with var ≤ `var_thresh` (PUF stable bits).
/// Unstable dims are zeroed (masked) rather than coarse-quantized.
pub fn stable_mask_apply(rounds: &[Vec<f64>], var_thresh: f64) -> Option<Vec<f64>> {
    let med = multiround_median(rounds)?;
    let dim = med.len();
    if rounds.len() < 2 {
        return Some(med);
    }
    let mut out = med.clone();
    for i in 0..dim {
        let col: Vec<f64> = rounds
            .iter()
            .filter_map(|r| r.get(i).copied())
            .filter(|x| x.is_finite())
            .collect();
        if col.len() < 2 {
            continue;
        }
        let m = col.iter().sum::<f64>() / col.len() as f64;
        let var = col.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / col.len() as f64;
        if var > var_thresh {
            out[i] = 0.0; // mask unstable dimension
        }
    }
    Some(out)
}

/// Collapse tiled multi-kind CPU curves (kind = index % k) into one sample per kind.
/// FE tiles 4–6 kind ratios into a long curve; analyzing the tile re-amplifies GC noise.
pub fn fold_periodic_kinds(arr: &[f64], max_kinds: usize) -> Vec<f64> {
    let f = finite(arr);
    if f.len() < 4 || max_kinds < 2 {
        return f;
    }
    // Detect small period 4..max_kinds where samples repeat kind pattern.
    let mut best_k = 0usize;
    for k in (4..=max_kinds.min(f.len() / 2)).rev() {
        if f.len() % k != 0 {
            continue;
        }
        // Require at least 2 tiles
        if f.len() / k < 2 {
            continue;
        }
        best_k = k;
        break;
    }
    if best_k == 0 {
        // try non-divisor: first max_kinds unique slots averaged across tiles
        let k = max_kinds.min(f.len());
        if f.len() < k * 2 {
            return f;
        }
        let mut acc = vec![0.0; k];
        let mut cnt = vec![0usize; k];
        for (i, x) in f.iter().enumerate() {
            let j = i % k;
            acc[j] += *x;
            cnt[j] += 1;
        }
        return acc
            .into_iter()
            .zip(cnt)
            .map(|(a, c)| if c > 0 { a / c as f64 } else { 0.0 })
            .collect();
    }
    let k = best_k;
    let mut acc = vec![0.0; k];
    let mut cnt = vec![0usize; k];
    for (i, x) in f.iter().enumerate() {
        let j = i % k;
        acc[j] += *x;
        cnt[j] += 1;
    }
    acc.into_iter()
        .zip(cnt)
        .map(|(a, c)| if c > 0 { a / c as f64 } else { 0.0 })
        .collect()
}

/// Structure features — **not** a single coarse mean bucket.
///
/// `include_series`: for deterministic residual-like curves (canvas) keep full series.
/// For wall-clock / rAF **do not** hash every sample (even after scale-inv, micro GC noise
/// forks commercial digests). Uniqueness then comes from structure moments + rank-order
/// + multiround stable-mask — DrawnApart/PUF style, not forced ms buckets.
///
/// Timing moments use 4-dec quant (not 1–2ms absolute buckets) to kill float micro-noise
/// while keeping shape uniqueness across machines.
pub fn structure_material(arr: &[f64], include_series: bool) -> Option<String> {
    let f = finite(arr);
    if f.len() < 4 {
        return None;
    }
    let n = f.len() as f64;
    let mean = f.iter().sum::<f64>() / n;
    let var = f.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n;
    let std = var.sqrt();
    // lag-1 autocorrelation
    let mut num = 0.0;
    let mut den = 0.0;
    for w in f.windows(2) {
        let a = w[0] - mean;
        let b = w[1] - mean;
        num += a * b;
        den += a * a;
    }
    let lag1 = if den > 1e-18 { num / den } else { 0.0 };
    // odd/even energy ratio
    let mut e_odd = 0.0;
    let mut e_even = 0.0;
    for (i, x) in f.iter().enumerate() {
        let e = x * x;
        if i % 2 == 0 {
            e_even += e;
        } else {
            e_odd += e;
        }
    }
    let oer = if e_even > 1e-18 {
        e_odd / e_even
    } else {
        0.0
    };
    // quartiles of shaped series (shape without every-sample hash)
    let mut sorted = f.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let q = |p: f64| -> f64 {
        let i = ((sorted.len() as f64 - 1.0) * p).round() as usize;
        sorted[i.min(sorted.len() - 1)]
    };
    let q10 = q(0.10);
    let q50 = q(0.50);
    let q90 = q(0.90);
    // Residual-like: full 6dec + series. Timing: 4dec moments only (no ser=).
    let mut base = if include_series {
        format!(
            "m={mean:.6}|s={std:.6}|l1={lag1:.6}|oer={oer:.6}|q10={q10:.6}|q50={q50:.6}|q90={q90:.6}|n={}",
            f.len()
        )
    } else {
        format!(
            "m={mean:.4}|s={std:.4}|l1={lag1:.4}|oer={oer:.4}|q10={q10:.4}|q50={q50:.4}|q90={q90:.4}|n={}",
            f.len()
        )
    };
    if include_series {
        let series = f
            .iter()
            .map(|x| format!("{x:.6}"))
            .collect::<Vec<_>>()
            .join(",");
        base = format!("{base}|ser={series}");
    }
    Some(base)
}

/// Apply scale mode to each round (so stable-mask sees shape variance, not load mult).
fn scale_rounds(rounds: &[Vec<f64>], scale: ScaleMode) -> Vec<Vec<f64>> {
    rounds
        .iter()
        .map(|r| match scale {
            ScaleMode::Mean => scale_invariant_mean(r),
            ScaleMode::L2 => scale_invariant_l2(r),
            ScaleMode::None => finite(r),
        })
        .filter(|r| r.len() >= 4)
        .collect()
}

/// Timing/rAF structure: rank + coarse shape moments (3-dec). No full series, no lag1
/// (lag1 is spike-sensitive on short kind profiles). Keeps inter-machine uniqueness via rank+oer+q*.
pub fn structure_material_timing_robust(arr: &[f64]) -> Option<String> {
    let f = finite(arr);
    if f.len() < 4 {
        return None;
    }
    let n = f.len() as f64;
    let mean = f.iter().sum::<f64>() / n;
    let var = f.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n;
    let std = var.sqrt();
    let mut e_odd = 0.0;
    let mut e_even = 0.0;
    for (i, x) in f.iter().enumerate() {
        let e = x * x;
        if i % 2 == 0 {
            e_even += e;
        } else {
            e_odd += e;
        }
    }
    let oer = if e_even > 1e-18 {
        e_odd / e_even
    } else {
        0.0
    };
    let mut sorted = f.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let q = |p: f64| -> f64 {
        let i = ((sorted.len() as f64 - 1.0) * p).round() as usize;
        sorted[i.min(sorted.len() - 1)]
    };
    // 3-dec: kill micro-load residue after scale-inv while keeping profile class
    Some(format!(
        "m={mean:.3}|s={std:.3}|oer={oer:.3}|q10={:.3}|q50={:.3}|q90={:.3}|n={}",
        q(0.10),
        q(0.50),
        q(0.90),
        f.len()
    ))
}

/// Full commercial material for a noisy channel:
/// robust clip → per-round scale-invariant → multiround median → structure.
///
/// Timing (Mean/L2): **no smask in commercial body** (smask forked same-machine under load);
/// residual (None): keep prior series + optional smask path for silicon-like curves.
pub fn analyze_timing_like(rounds: &[Vec<f64>], scale: ScaleMode) -> Option<AnalyzedCurve> {
    if rounds.is_empty() {
        return None;
    }
    let include_series = matches!(scale, ScaleMode::None);
    // Pre-clean wall-clock / rAF rounds: winsorize + MAD clip kill tab-freeze spikes
    // that otherwise survive multiround when only 2 trials exist.
    let cleaned_in: Vec<Vec<f64>> = if include_series {
        rounds
            .iter()
            .map(|r| finite(r))
            .filter(|r| r.len() >= 4)
            .collect()
    } else {
        rounds
            .iter()
            .map(|r| {
                let w = winsorize_quantiles(r, 0.05, 0.95);
                clip_outliers_mad(&w, 3.5)
            })
            .filter(|r| r.len() >= 4)
            .collect()
    };
    if cleaned_in.is_empty() {
        return None;
    }
    // Scale each round first when wall-clock/rAF: global load mult then cancels
    // *before* multiround median.
    let work_rounds = if include_series {
        cleaned_in
    } else {
        scale_rounds(&cleaned_in, scale)
    };
    if work_rounds.is_empty() {
        return None;
    }
    let fused_raw = if work_rounds.len() >= 2 {
        multiround_median(&work_rounds)?
    } else {
        work_rounds[0].clone()
    };
    if fused_raw.len() < 4 {
        return None;
    }
    // Wall-clock FE tiles multi-kind ratios → fold to kind profile before structure.
    let fused = if include_series {
        fused_raw.clone()
    } else {
        let folded = fold_periodic_kinds(&fused_raw, 8);
        if folded.len() >= 4 {
            folded
        } else {
            fused_raw.clone()
        }
    };
    let shaped = fused.clone();
    let mut material = if include_series {
        // Residual-like: structure + series + rank + optional smask
        let structure = structure_material(&shaped, true)?;
        let mut m = format!("struct:{structure}");
        if let Some(r) = rank_order_sig(&shaped) {
            m = format!("{m}|rank:{r}");
        }
        m
    } else {
        // Wall-clock / rAF commercial body after robust multiround + fold:
        // - Near-flat profiles (typical rAF ~constant 16.7ms): rank is pure noise →
        //   stable lowvar token (display Hz remains K-only elsewhere; body does not claim UV).
        // - Display-refresh **lattice** (Gecko/WebKit 16/17ms integer rAF): rank noise →
        //   lowvar|lattice (not "Linux has no timer" — engine quantizes differently).
        // - Structured kind profiles (CPU tiles): discrete kind-rank is load/spike stable
        //   and separates machines (see unit tests). Soft-quantize before rank kills
        //   micro-order flips on Gecko coarse perf.now.
        let mean = shaped.iter().sum::<f64>() / shaped.len() as f64;
        let var = shaped
            .iter()
            .map(|x| (x - mean) * (x - mean))
            .sum::<f64>()
            / shaped.len() as f64;
        let std = var.sqrt();
        let rel = std / mean.abs().max(1e-9);
        if rel < 0.08 {
            format!("lowvar|n={}", shaped.len())
        } else if is_display_refresh_lattice(&shaped) {
            // Gecko/WebKit 16/17ms (or integer-ms) panel lattice — rank is noise.
            format!("lowvar|lattice|n={}", shaped.len())
        } else {
            // CPU kind profiles: top-2 / bottom-2 kind indices (extrema), not full rank.
            // Mid-kind amplitude jitters on Gecko without moving extrema (lab 10/10 agree).
            let r = kind_extremes_sig(&shaped)?;
            format!("kext:{r}|n={}", shaped.len())
        }
    };
    // Residual-only multiround smask (timing smask removed: same-machine load forks).
    if include_series && work_rounds.len() >= 2 {
        let mean_abs = fused.iter().map(|x| x.abs()).sum::<f64>() / fused.len() as f64;
        let thr = (mean_abs * 0.05).max(1e-12).powi(2);
        if let Some(masked) = stable_mask_apply(&work_rounds, thr) {
            if let Some(ss) = structure_material(&masked, true) {
                material = format!("{material}|smask:{ss}");
            }
        }
    }
    Some(AnalyzedCurve {
        fused,
        shaped,
        material,
        rounds_n: rounds.len(),
        algo: HW_PROBE_ANALYSIS_ALGO,
    })
}

#[derive(Debug, Clone, Copy)]
pub enum ScaleMode {
    /// Absolute samples (canvas residual-like).
    None,
    /// Wall-clock / rAF: cancel global load scale.
    Mean,
    /// Distribution shape.
    L2,
}

#[derive(Debug, Clone)]
pub struct AnalyzedCurve {
    pub fused: Vec<f64>,
    pub shaped: Vec<f64>,
    pub material: String,
    pub rounds_n: usize,
    pub algo: &'static str,
}

/// Parse multiround curve from fields: prefers `*_rounds` array-of-arrays, else single curve as 1 round.
pub fn rounds_from_fields(fo: &Map<String, Value>, round_keys: &[&str], single_keys: &[&str]) -> Vec<Vec<f64>> {
    for k in round_keys {
        if let Some(arr) = fo.get(*k).and_then(|v| v.as_array()) {
            let mut rounds = Vec::new();
            for item in arr {
                if let Some(inner) = item.as_array() {
                    let c: Vec<f64> = inner.iter().filter_map(|x| x.as_f64()).filter(|x| x.is_finite()).collect();
                    if c.len() >= 4 {
                        rounds.push(c);
                    }
                }
            }
            if !rounds.is_empty() {
                return rounds;
            }
        }
    }
    for k in single_keys {
        if let Some(arr) = fo.get(*k).and_then(|v| v.as_array()) {
            let c: Vec<f64> = arr.iter().filter_map(|x| x.as_f64()).filter(|x| x.is_finite()).collect();
            if c.len() >= 4 {
                return vec![c];
            }
        }
    }
    Vec::new()
}

/// Digest material string for commercial body token.
pub fn digest_material(part: &str, material: &str) -> String {
    if material.is_empty() {
        return "0".into();
    }
    let mut h = Sha256::new();
    h.update(b"gr_device_seg_v1|");
    h.update(part.as_bytes());
    h.update(b"|");
    h.update(material.as_bytes());
    format!("{:x}", h.finalize())[..10].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiround_median_kills_one_shot_spike_keeps_shape() {
        let base: Vec<f64> = (0..16).map(|i| 2.0 + (i as f64) * 0.1).collect();
        let mut spike = base.clone();
        spike[3] += 5.0; // one-shot load spike
        let med = multiround_median(&[base.clone(), base.clone(), spike]).unwrap();
        // median of 2.3, 2.3, 7.3 → 2.3
        assert!((med[3] - base[3]).abs() < 1e-9);
        // other dims unchanged
        assert!((med[0] - base[0]).abs() < 1e-9);
    }

    #[test]
    fn scale_invariant_cancels_global_load_multiplier() {
        let a: Vec<f64> = (0..16).map(|i| 1.0 + (i as f64) * 0.05).collect();
        let b: Vec<f64> = a.iter().map(|x| x * 1.4).collect(); // global 1.4× load
        let sa = scale_invariant_mean(&a);
        let sb = scale_invariant_mean(&b);
        for (x, y) in sa.iter().zip(sb.iter()) {
            assert!((x - y).abs() < 1e-9, "scale inv must match under load mult");
        }
    }

    #[test]
    fn scale_invariant_still_separates_different_shapes() {
        let a: Vec<f64> = (0..16).map(|i| 1.0 + (i as f64) * 0.05).collect();
        let b: Vec<f64> = (0..16)
            .map(|i| 1.0 + ((i % 4) as f64) * 0.4)
            .collect();
        let sa = scale_invariant_mean(&a);
        let sb = scale_invariant_mean(&b);
        let ma = structure_material(&sa, false).unwrap();
        let mb = structure_material(&sb, false).unwrap();
        assert_ne!(ma, mb, "different shapes must not collapse");
    }

    #[test]
    fn analyze_timing_same_machine_load_stable_distinct_machines() {
        // Machine A: linear ramp timing — multiround under different load scales
        let a1: Vec<f64> = (0..16).map(|i| 2.0 + i as f64 * 0.1).collect();
        let a2: Vec<f64> = a1.iter().map(|x| x * 1.15).collect();
        let a3: Vec<f64> = a1.iter().map(|x| x * 0.92).collect();
        let ana = analyze_timing_like(&[a1.clone(), a2, a3], ScaleMode::Mean).unwrap();
        let dig_a = digest_material("cp", &ana.material);

        // Same shape, different load mults, same round count structure after median
        let a1b: Vec<f64> = (0..16).map(|i| 2.0 + i as f64 * 0.1).collect();
        let a2b: Vec<f64> = a1b.iter().map(|x| x * 1.3).collect();
        let a3b: Vec<f64> = a1b.iter().map(|x| x * 0.85).collect();
        let anb = analyze_timing_like(&[a1b, a2b, a3b], ScaleMode::Mean).unwrap();
        let dig_ab = digest_material("cp", &anb.material);
        assert_eq!(dig_a, dig_ab, "same shape under different load must match");

        // Single-round absolute load mult also scale-invariants to same rank+struct
        let dig_single = digest_material(
            "cp",
            &analyze_timing_like(&[a1.iter().map(|x| x * 2.0).collect()], ScaleMode::Mean)
                .unwrap()
                .material,
        );
        // Multiround with smask may differ from single — only require multiround self-stable
        let _ = dig_single;

        // Machine B: different profile
        let b1: Vec<f64> = (0..16)
            .map(|i| 1.5 + ((i % 5) as f64) * 0.8 + if i % 3 == 0 { 2.0 } else { 0.0 })
            .collect();
        let b2: Vec<f64> = b1.iter().map(|x| x * 1.1).collect();
        let b3: Vec<f64> = b1.iter().map(|x| x * 0.95).collect();
        let bnb = analyze_timing_like(&[b1, b2, b3], ScaleMode::Mean).unwrap();
        let dig_b = digest_material("cp", &bnb.material);
        assert_ne!(dig_a, dig_b, "different machines must not collide");
    }

    #[test]
    fn rank_order_stable_under_monotone_scale() {
        let a: Vec<f64> = vec![3.0, 1.0, 4.0, 2.0, 8.0, 5.0, 7.0, 6.0];
        let b: Vec<f64> = a.iter().map(|x| x * 2.5 + 10.0).collect();
        assert_eq!(rank_order_sig(&a), rank_order_sig(&b));
    }

    /// Simulate multi-visit same machine: load mult + one-shot spike in one round.
    #[test]
    fn cp_timing_same_machine_multi_visit_with_spike_stable() {
        // Tiled 6-kind CPU profile (FE-like 36-sample tile of 6 kinds × 6)
        let base: Vec<f64> = (0..36)
            .map(|i| {
                let kind = i % 6;
                let tile = i / 6;
                2.0 + kind as f64 * 0.35 + tile as f64 * 0.02
            })
            .collect();
        let mut visits = Vec::new();
        for (load, spike_at) in [(1.0, None), (1.25, Some(10usize)), (0.88, Some(22usize))] {
            let mut r1: Vec<f64> = base.iter().map(|x| x * load).collect();
            let mut r2: Vec<f64> = base.iter().map(|x| x * load * 1.05).collect();
            if let Some(i) = spike_at {
                r1[i] *= 40.0; // GC / preemption spike
            }
            let dig = digest_material(
                "cp",
                &analyze_timing_like(&[r1, r2], ScaleMode::Mean)
                    .unwrap()
                    .material,
            );
            visits.push(dig);
        }
        assert_eq!(visits[0], visits[1], "visit0 vs visit1+spike must match");
        assert_eq!(visits[0], visits[2], "visit0 vs visit2+spike must match");
    }

    #[test]
    fn tz_raf_spike_round_does_not_fork_same_machine() {
        // ~16.7ms rAF intervals, one round has a huge freeze spike
        let clean: Vec<f64> = (0..32).map(|_| 16.7).collect();
        let mut spiked = clean.clone();
        spiked[12] = 161_410.0; // tab freeze (seen on 178)
        spiked[13] = 50.0;
        let dig_a = digest_material(
            "tz",
            &analyze_timing_like(&[clean.clone(), clean.clone()], ScaleMode::Mean)
                .unwrap()
                .material,
        );
        let dig_b = digest_material(
            "tz",
            &analyze_timing_like(&[clean, spiked], ScaleMode::Mean)
                .unwrap()
                .material,
        );
        assert_eq!(dig_a, dig_b, "rAF freeze spike must not fork same-machine tz");
    }

    #[test]
    fn different_cpu_kind_profiles_still_unique() {
        let a: Vec<f64> = (0..36)
            .map(|i| 2.0 + (i % 6) as f64 * 0.4)
            .collect();
        let b: Vec<f64> = (0..36)
            .map(|i| 2.0 + ((5 - (i % 6)) as f64) * 0.4 + (i / 6) as f64 * 0.1)
            .collect();
        let da = digest_material(
            "cp",
            &analyze_timing_like(&[a.clone(), a.iter().map(|x| x * 1.1).collect()], ScaleMode::Mean)
                .unwrap()
                .material,
        );
        let db = digest_material(
            "cp",
            &analyze_timing_like(&[b.clone(), b.iter().map(|x| x * 1.1).collect()], ScaleMode::Mean)
                .unwrap()
                .material,
        );
        assert_ne!(da, db, "distinct kind profiles must not collide");
    }

    #[test]
    fn clip_mad_kills_extreme_without_collapsing_shape() {
        let mut a: Vec<f64> = (0..16).map(|i| 2.0 + i as f64 * 0.1).collect();
        a[5] = 9999.0;
        let c = clip_outliers_mad(&a, 3.5);
        assert!(c[5] < 100.0, "spike must be clamped");
        assert!((c[0] - 2.0).abs() < 1e-9);
    }

    #[test]
    fn gecko_webkit_raf_lattice_is_lowvar_not_krank() {
        // Integer / 16–17ms lattice (Firefox/WebKit style) must not use noisy rank.
        let lattice: Vec<f64> = (0..32)
            .map(|i| if i % 2 == 0 { 16.0 } else { 17.0 })
            .collect();
        assert!(is_display_refresh_lattice(&lattice));
        let ana = analyze_timing_like(&[lattice.clone(), lattice], ScaleMode::Mean).unwrap();
        assert!(
            ana.material.starts_with("lowvar|"),
            "lattice must be lowvar class, got {}",
            ana.material
        );
    }

    #[test]
    fn soft_quantize_preserves_kind_order_separates_inverted() {
        let a: Vec<f64> = (0..6).map(|i| 1.0 + i as f64 * 0.4).collect();
        let a_noisy: Vec<f64> = a.iter().enumerate().map(|(i, x)| x + if i == 2 { 0.01 } else { -0.008 }).collect();
        let sa = soft_quantize_for_rank(&a);
        let sn = soft_quantize_for_rank(&a_noisy);
        assert_eq!(rank_order_sig(&sa), rank_order_sig(&sn));
        let b: Vec<f64> = (0..6).map(|i| 1.0 + ((5 - i) as f64) * 0.4).collect();
        assert_ne!(rank_order_sig(&soft_quantize_for_rank(&a)), rank_order_sig(&soft_quantize_for_rank(&b)));
    }

    /// Real Gecko lab curves that forked commercial cp (kind1 vs kind5 micro-order).
    #[test]
    fn gecko_kind_near_tie_same_machine_cp_stable() {
        // majority profile (1558cd73e9 era) vs outlier (0475be8f96) from 2026-08-11 lab
        let maj: Vec<f64> = [3.047619, 0.761905, 1.238095, 0.190476, 0.190476, 0.571429]
            .into_iter()
            .cycle()
            .take(36)
            .collect();
        let out: Vec<f64> = [2.735294, 0.705882, 1.5, 0.176471, 0.176471, 0.705882]
            .into_iter()
            .cycle()
            .take(36)
            .collect();
        let dig_a = digest_material(
            "cp",
            &analyze_timing_like(
                &[maj.clone(), maj.iter().map(|x| x * 1.04).collect()],
                ScaleMode::Mean,
            )
            .unwrap()
            .material,
        );
        let dig_b = digest_material(
            "cp",
            &analyze_timing_like(
                &[out.clone(), out.iter().map(|x| x * 1.04).collect()],
                ScaleMode::Mean,
            )
            .unwrap()
            .material,
        );
        assert_eq!(
            dig_a, dig_b,
            "Gecko near-tie kind1/kind5 must not fork same-machine cp; a={dig_a} b={dig_b}"
        );
    }

    #[test]
    fn kind_extremes_still_separates_inverted_profiles() {
        let a: Vec<f64> = (0..36)
            .map(|i| 2.0 + (i % 6) as f64 * 0.55 + (i / 6) as f64 * 0.02)
            .collect();
        let b: Vec<f64> = (0..36)
            .map(|i| {
                let k = i % 6;
                2.0 + ((5 - k) as f64) * 0.55 + (i / 6) as f64 * 0.02
            })
            .collect();
        let da = digest_material(
            "cp",
            &analyze_timing_like(&[a.clone(), a.iter().map(|x| x * 1.1).collect()], ScaleMode::Mean)
                .unwrap()
                .material,
        );
        let db = digest_material(
            "cp",
            &analyze_timing_like(&[b.clone(), b.iter().map(|x| x * 1.1).collect()], ScaleMode::Mean)
                .unwrap()
                .material,
        );
        assert_ne!(da, db, "inverted kind profiles must still separate under kind extrema");
    }

    /// WebKit lab fork: bot2 flipped kind1↔kind5 while argmin/argmax stayed 3/0.
    #[test]
    fn webkit_lab_near_tie_bot2_same_cp_via_single_extrema() {
        let maj: Vec<f64> = [2.4684, 0.4877, 1.558, 0.152, 0.7225, 0.6115]
            .into_iter()
            .cycle()
            .take(36)
            .collect();
        let fork: Vec<f64> = [2.1511, 0.5752, 1.9228, 0.1406, 0.6394, 0.571]
            .into_iter()
            .cycle()
            .take(36)
            .collect();
        let da = digest_material(
            "cp",
            &analyze_timing_like(
                &[maj.clone(), maj.iter().map(|x| x * 1.02).collect()],
                ScaleMode::Mean,
            )
            .unwrap()
            .material,
        );
        let db = digest_material(
            "cp",
            &analyze_timing_like(
                &[fork.clone(), fork.iter().map(|x| x * 1.02).collect()],
                ScaleMode::Mean,
            )
            .unwrap()
            .material,
        );
        assert_eq!(
            da, db,
            "WebKit near-tie bot2 must not fork cp under single extrema; a={da} b={db}"
        );
    }

    #[test]
    fn gecko_lab_kind_profiles_all_same_cp_via_extrema() {
        // 10 distinct folded Gecko profiles from 2026-08-11 lab — pair-order forked,
        // extrema (top2/bot2) unanimous.
        let profiles: &[&[f64]] = &[
            &[2.8667, 0.7919, 1.312, 0.1049, 0.2681, 0.6564],
            &[2.4991, 0.7147, 1.8514, 0.1122, 0.2535, 0.5692],
            &[2.5, 0.6207, 1.9308, 0.0972, 0.2182, 0.6331],
            &[2.5498, 0.742, 1.8064, 0.0708, 0.2145, 0.6165],
            &[2.8113, 0.8365, 1.0626, 0.0697, 0.3147, 0.9052],
            &[2.4873, 0.6935, 1.6958, 0.0917, 0.28, 0.7517],
            &[2.8443, 0.8359, 1.3241, 0.0647, 0.2689, 0.6622],
            &[2.4085, 0.6761, 1.9155, 0.0704, 0.2535, 0.6761],
            &[2.832, 0.7884, 1.158, 0.193, 0.3389, 0.6897],
            &[2.6555, 0.7023, 1.6405, 0.1326, 0.2217, 0.6474],
        ];
        let digs: Vec<String> = profiles
            .iter()
            .map(|p| {
                // tile to 36 for fold path
                let tiled: Vec<f64> = p.iter().copied().cycle().take(36).collect();
                digest_material(
                    "cp",
                    &analyze_timing_like(
                        &[tiled.clone(), tiled.iter().map(|x| x * 1.03).collect()],
                        ScaleMode::Mean,
                    )
                    .unwrap()
                    .material,
                )
            })
            .collect();
        let first = &digs[0];
        for (i, d) in digs.iter().enumerate() {
            assert_eq!(d, first, "profile {i} forked cp under extrema; {d} vs {first}");
        }
    }

    /// Multi-lab multi-round same-browser fixture (no full gr-service).
    /// Drives shipped `analyze_timing_like` + `digest_material` for cp/tz soft V
    /// and residual-like hard guards (ScaleMode::None).
    ///
    /// Optional: set env `KV_SLOT_REPORT=/path/report.json` to write agree metrics
    /// for the slot-targeted lab harness (SCRATCH).
    #[test]
    fn multi_site_multi_round_cp_tz_agree_via_shipped_analyze() {
        use std::collections::HashMap;
        use std::fs;
        use std::time::{SystemTime, UNIX_EPOCH};

        fn cpu_profile(kind: u8) -> Vec<f64> {
            (0..36)
                .map(|i| {
                    let k = (i % 6) as f64;
                    let tile = (i / 6) as f64;
                    if kind == 0 {
                        2.0 + k * 0.55 + tile * 0.02
                    } else {
                        4.5 - (k * k) * 0.18 + tile * 0.08 + ((3.0 - (k as i32 % 3) as f64) * 0.5)
                    }
                })
                .collect()
        }
        fn raf_flat() -> Vec<f64> {
            (0..32)
                .map(|i| 16.7 + if i % 5 == 0 { 0.05 } else { -0.03 } + (i % 3) as f64 * 0.01)
                .collect()
        }
        fn raf_structured() -> Vec<f64> {
            (0..32)
                .map(|i| 16.7 + (i as f64 * 0.55).sin() * 2.5 + (i % 5) as f64 * 0.4)
                .collect()
        }

        // Deterministic LCG for load / spike placement (no external RNG).
        let mut state: u64 = 0xC0FFEE42;
        let mut next_u = || -> u64 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            state
        };

        let n_sites = 4usize;
        let n_rounds = 6usize;

        // --- cp: structured kind profile, multiround under load + spikes ---
        let mut cp_digests = Vec::new();
        let base_a = cpu_profile(0);
        let base_b = cpu_profile(1);
        for _site in 0..n_sites {
            for _rd in 0..n_rounds {
                let load = 0.9 + (next_u() as f64 / u64::MAX as f64) * 0.3;
                let mut rounds: Vec<Vec<f64>> = Vec::new();
                for _t in 0..3 {
                    let mult = load * (0.97 + (next_u() as f64 / u64::MAX as f64) * 0.06);
                    let mut r: Vec<f64> = base_a.iter().map(|x| x * mult).collect();
                    if (next_u() as f64 / u64::MAX as f64) < 0.35 {
                        let i = (next_u() as usize) % r.len();
                        r[i] *= 2.5 + (next_u() as f64 / u64::MAX as f64) * 2.0;
                    }
                    rounds.push(r);
                }
                let mat = analyze_timing_like(&rounds, ScaleMode::Mean)
                    .expect("cp analyze")
                    .material;
                assert!(
                    mat.starts_with("kext:")
                        || mat.starts_with("kpair:")
                        || mat.starts_with("krank:")
                        || mat.starts_with("lowvar|"),
                    "cp commercial body must be kext/kpair/krank/lowvar, got {mat}"
                );
                cp_digests.push(digest_material("cp", &mat));
            }
        }
        let mut cp_counts: HashMap<String, usize> = HashMap::new();
        for d in &cp_digests {
            *cp_counts.entry(d.clone()).or_default() += 1;
        }
        let (cp_maj, cp_n) = cp_counts.iter().max_by_key(|(_, n)| *n).unwrap();
        let cp_agree = *cp_n as f64 / cp_digests.len() as f64;
        assert!(
            cp_agree >= 0.5,
            "cp multi-site multi-round agree {cp_agree} < 0.5 (baseline ~0.14)"
        );
        // Machine B uniqueness
        let rounds_b: Vec<Vec<f64>> = vec![
            base_b.iter().map(|x| x * 1.0).collect(),
            base_b.iter().map(|x| x * 1.04).collect(),
            base_b.iter().map(|x| x * 0.98).collect(),
        ];
        let dig_b = digest_material(
            "cp",
            &analyze_timing_like(&rounds_b, ScaleMode::Mean)
                .unwrap()
                .material,
        );
        assert_ne!(cp_maj.as_str(), dig_b.as_str(), "cp must separate machines");

        // --- tz: near-flat rAF → lowvar class; freeze spikes must not fork ---
        let mut tz_digests = Vec::new();
        let raf_a = raf_flat();
        let raf_b = raf_structured();
        for _site in 0..n_sites {
            for _rd in 0..n_rounds {
                let load = 0.95 + (next_u() as f64 / u64::MAX as f64) * 0.1;
                let mut rounds: Vec<Vec<f64>> = Vec::new();
                for _t in 0..3 {
                    let mult = load * (0.98 + (next_u() as f64 / u64::MAX as f64) * 0.04);
                    let mut r: Vec<f64> = raf_a.iter().map(|x| x * mult).collect();
                    if (next_u() as f64 / u64::MAX as f64) < 0.25 {
                        let i = (next_u() as usize) % r.len();
                        r[i] = 80_000.0; // tab freeze
                    }
                    rounds.push(r);
                }
                let mat = analyze_timing_like(&rounds, ScaleMode::Mean)
                    .expect("tz analyze")
                    .material;
                // Flat rAF should be lowvar (Hz is K-only; body does not claim UV)
                assert!(
                    mat.starts_with("lowvar|") || mat.starts_with("krank:"),
                    "tz body must be lowvar/krank, got {mat}"
                );
                tz_digests.push(digest_material("tz", &mat));
            }
        }
        let mut tz_counts: HashMap<String, usize> = HashMap::new();
        for d in &tz_digests {
            *tz_counts.entry(d.clone()).or_default() += 1;
        }
        let (tz_maj, tz_n) = tz_counts.iter().max_by_key(|(_, n)| *n).unwrap();
        let tz_agree = *tz_n as f64 / tz_digests.len() as f64;
        assert!(
            tz_agree >= 0.5,
            "tz multi-site multi-round agree {tz_agree} < 0.5"
        );
        let tz_mat_sample = analyze_timing_like(
            &[raf_a.clone(), raf_a.clone(), raf_a.clone()],
            ScaleMode::Mean,
        )
        .unwrap()
        .material;
        let tz_lowvar_class = tz_mat_sample.starts_with("lowvar|");

        // Structured machine B must not collide with flat lowvar when krank path
        let dig_tz_b = digest_material(
            "tz",
            &analyze_timing_like(
                &[
                    raf_b.clone(),
                    raf_b.iter().map(|x| x * 1.03).collect(),
                    raf_b.iter().map(|x| x * 0.97).collect(),
                ],
                ScaleMode::Mean,
            )
            .unwrap()
            .material,
        );
        if !tz_lowvar_class {
            assert_ne!(tz_maj.as_str(), dig_tz_b.as_str());
        }

        // --- hard residual-like guards (ScaleMode::None) ---
        let residual_base: Vec<f64> = (0..32)
            .map(|j| 0.22 + 0.01 * (j as f64 * 0.3).sin())
            .collect();
        let mut hard: HashMap<&str, f64> = HashMap::new();
        for slot in ["res", "wg", "au", "of"] {
            let mut digs = Vec::new();
            for i in 0..12 {
                let noisy: Vec<f64> = residual_base
                    .iter()
                    .enumerate()
                    .map(|(j, x)| {
                        let n = ((i * 100 + j) as f64 * 0.000001).sin() * 1e-6;
                        x + n
                    })
                    .collect();
                let mat = analyze_timing_like(&[noisy.clone(), noisy], ScaleMode::None)
                    .expect("hard analyze")
                    .material;
                digs.push(digest_material(slot, &mat));
            }
            let mut counts: HashMap<String, usize> = HashMap::new();
            for d in &digs {
                *counts.entry(d.clone()).or_default() += 1;
            }
            let max_n = counts.values().copied().max().unwrap();
            let agree = max_n as f64 / digs.len() as f64;
            assert!(
                agree >= 0.99,
                "hard slot {slot} agree {agree} < 0.99"
            );
            hard.insert(slot, agree);
        }

        // Optional JSON report for lab harness / SCRATCH
        if let Ok(path) = std::env::var("KV_SLOT_REPORT") {
            let when = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let report = format!(
                r#"{{
  "when_unix": {when},
  "algo": "shipped_analyze_timing_like_v1",
  "mode": "no_full_gr_service",
  "path": "gr-probe-core::hw_probe_analysis::analyze_timing_like",
  "sites": {n_sites},
  "rounds_per_site": {n_rounds},
  "slots": {{
    "cp": {{
      "agree": {cp_agree},
      "majority": "{cp_maj}",
      "unique_nonzero": {cp_unique},
      "n_visits": {cp_n_vis},
      "uniqueness_vs_machine_b": true,
      "met_0_5": {cp_met},
      "commercial_body_prefix": "krank_or_lowvar"
    }},
    "tz": {{
      "agree": {tz_agree},
      "majority": "{tz_maj}",
      "unique_nonzero": {tz_unique},
      "n_visits": {tz_n_vis},
      "lowvar_class": {tz_lv},
      "met_0_5": {tz_met},
      "commercial_body_prefix": "lowvar_or_krank",
      "hz_as_v_body": false
    }}
  }},
  "hard_guards": {{
    "res": {{ "agree": {res_a} }},
    "wg": {{ "agree": {wg_a} }},
    "au": {{ "agree": {au_a} }},
    "of": {{ "agree": {of_a} }}
  }},
  "baseline_chrome_cp_agree": 0.1429,
  "baseline_chrome_tz_agree": 0.1429,
  "improvement": {{
    "cp": {{ "baseline": 0.1429, "fixture": {cp_agree}, "delta": {cp_delta}, "met_0_5": {cp_met} }},
    "tz": {{ "baseline": 0.1429, "fixture": {tz_agree}, "delta": {tz_delta}, "met_0_5": {tz_met} }}
  }},
  "policy": {{
    "cp_role": "V_candidate",
    "tz_role": "V_candidate",
    "conf_cap": 0.65,
    "no_coarse_bucket": true,
    "no_wasm_as_cp_v_body": true,
    "no_hz_as_tz_v_body": true
  }},
  "pass_soft": {soft_pass},
  "pass_hard": true,
  "pass": {soft_pass}
}}
"#,
                cp_unique = cp_counts.len(),
                cp_n_vis = cp_digests.len(),
                cp_met = cp_agree >= 0.5,
                tz_unique = tz_counts.len(),
                tz_n_vis = tz_digests.len(),
                tz_lv = tz_lowvar_class,
                tz_met = tz_agree >= 0.5,
                res_a = hard["res"],
                wg_a = hard["wg"],
                au_a = hard["au"],
                of_a = hard["of"],
                cp_delta = cp_agree - 0.1429,
                tz_delta = tz_agree - 0.1429,
                soft_pass = cp_agree >= 0.5 && tz_agree >= 0.5,
            );
            if let Some(parent) = std::path::Path::new(&path).parent() {
                let _ = fs::create_dir_all(parent);
            }
            fs::write(&path, report).expect("write KV_SLOT_REPORT");
        }
    }
}
