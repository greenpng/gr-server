//! Application-layer clock skew estimate (iss/58 A1).
//!
//! FE attaches `t_perf` (performance.now) per batch; server pairs with recv time.
//! Theil–Sen slope → ppm bucket for **tz** extra / fusion material.
//! No permission surface. VM/synthetic clocks marked class=synthetic when slope≈0
//! with tiny residual (shared host clock).

use serde_json::{json, Value};

pub const HW_CLOCK_SKEW_ALGO: &str = "hw_clock_skew_v1";

/// One (server_recv_ms, t_perf_ms) sample.
#[derive(Debug, Clone, Copy)]
pub struct SkewSample {
    pub t_server_ms: f64,
    pub t_perf_ms: f64,
}

/// Estimate skew from samples. Returns JSON report.
pub fn estimate_clock_skew(samples: &[SkewSample]) -> Value {
    if samples.len() < 3 {
        return json!({
            "algo": HW_CLOCK_SKEW_ALGO,
            "ok": false,
            "reason": "need_ge_3_samples",
            "n": samples.len(),
        });
    }
    // Use first sample as origin
    let t0s = samples[0].t_server_ms;
    let t0p = samples[0].t_perf_ms;
    let pts: Vec<(f64, f64)> = samples
        .iter()
        .map(|s| (s.t_server_ms - t0s, s.t_perf_ms - t0p))
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .collect();
    if pts.len() < 3 {
        return json!({
            "algo": HW_CLOCK_SKEW_ALGO,
            "ok": false,
            "reason": "insufficient_finite",
            "n": pts.len(),
        });
    }
    // Theil–Sen: median of pairwise slopes dy/dx (skip dx≈0)
    let mut slopes = Vec::new();
    for i in 0..pts.len() {
        for j in (i + 1)..pts.len() {
            let dx = pts[j].0 - pts[i].0;
            if dx.abs() < 1.0 {
                continue;
            }
            let dy = pts[j].1 - pts[i].1;
            slopes.push(dy / dx);
        }
    }
    if slopes.is_empty() {
        return json!({
            "algo": HW_CLOCK_SKEW_ALGO,
            "ok": false,
            "reason": "no_pairwise_slopes",
            "n": pts.len(),
        });
    }
    slopes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = slopes.len() / 2;
    let slope = if slopes.len() % 2 == 1 {
        slopes[mid]
    } else {
        (slopes[mid - 1] + slopes[mid]) / 2.0
    };
    // slope ≈ d(perf)/d(server) ; skew_ppm = (slope - 1) * 1e6
    let skew_ppm = (slope - 1.0) * 1_000_000.0;
    // Residual MAD
    let mut resids: Vec<f64> = pts
        .iter()
        .map(|(x, y)| (y - slope * x).abs())
        .collect();
    resids.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mad = resids[resids.len() / 2];
    // Bucket 0.5 ppm
    let bucket = (skew_ppm * 2.0).round() / 2.0;
    // synthetic heuristic: near-zero skew + very small mad + many samples from "perfect" host
    let class = if skew_ppm.abs() < 0.25 && mad < 0.5 && samples.len() >= 5 {
        "synthetic_or_locked"
    } else if mad > 50.0 {
        "noisy"
    } else {
        "native"
    };
    let conf = if class == "noisy" {
        0.3
    } else if samples.len() >= 8 && mad < 5.0 {
        0.9
    } else if samples.len() >= 5 {
        0.7
    } else {
        0.5
    };
    json!({
        "algo": HW_CLOCK_SKEW_ALGO,
        "ok": true,
        "n": samples.len(),
        "slope": slope,
        "clock_skew_ppm": (skew_ppm * 100.0).round() / 100.0,
        "clock_skew_ppm_bucket": bucket,
        "clock_skew_class": class,
        "residual_mad_ms": (mad * 1000.0).round() / 1000.0,
        "confidence": conf,
        "tz_extra": format!(
            "csk={bucket:.1}|cls={class}|n={}",
            samples.len()
        ),
    })
}

/// Extract samples from fields if FE attached `clock_skew_samples` or single t_perf+server.
pub fn estimate_from_fields(fields: &Value) -> Value {
    let mut samples = Vec::new();
    if let Some(arr) = fields.get("clock_skew_samples").and_then(|v| v.as_array()) {
        for s in arr {
            let ts = s
                .get("t_server_ms")
                .or_else(|| s.get("server_recv_ms"))
                .and_then(|v| v.as_f64());
            let tp = s.get("t_perf").or_else(|| s.get("t_perf_ms")).and_then(|v| v.as_f64());
            if let (Some(a), Some(b)) = (ts, tp) {
                samples.push(SkewSample {
                    t_server_ms: a,
                    t_perf_ms: b,
                });
            }
        }
    }
    // Single-point insufficient; still report presence of t_perf for ops
    if samples.is_empty() {
        if let Some(tp) = fields.get("t_perf").and_then(|v| v.as_f64()) {
            return json!({
                "algo": HW_CLOCK_SKEW_ALGO,
                "ok": false,
                "reason": "single_t_perf_need_series",
                "t_perf_seen": tp,
                "n": 1,
            });
        }
        return json!({
            "algo": HW_CLOCK_SKEW_ALGO,
            "ok": false,
            "reason": "no_samples",
            "n": 0,
        });
    }
    estimate_clock_skew(&samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovers_known_ppm() {
        // perf runs 10ppm fast: t_perf = t_server * (1 + 10e-6)
        let mut samples = Vec::new();
        for i in 0..12 {
            let ts = 1_000_000.0 + i as f64 * 5000.0;
            let tp = ts * (1.0 + 10e-6);
            samples.push(SkewSample {
                t_server_ms: ts,
                t_perf_ms: tp,
            });
        }
        let r = estimate_clock_skew(&samples);
        assert_eq!(r["ok"], true);
        let ppm = r["clock_skew_ppm"].as_f64().unwrap();
        assert!((ppm - 10.0).abs() < 1.0, "ppm={ppm}");
    }
}
