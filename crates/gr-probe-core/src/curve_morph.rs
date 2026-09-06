//! A2 curve-morph summary consumption (iss/74 strategy §4-A2).
//!
//! Unified curve-shape summarizer: slope / knee / CV / p25-p75 span /
//! monotonic ratio / phase. Produces `curve_morph_{slot}` intermediate
//! fields so rule layers consume summary *buckets* instead of raw
//! fixed ±0.02–0.04 pointwise deltas. Bucket bounds come from
//! `spec/curve_morph_calibration.json` (lab-seeded SSOT; the loader
//! fails open to the built-in defaults below when the file is absent
//! so analysis never degrades on IO).

use serde_json::{json, Map, Value};
use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq)]
pub struct CurveMorph {
    /// Net slope per step: (last - first) / max(n-1, 1).
    pub slope: f64,
    /// Normalized position (0..1) of the maximum second-difference
    /// magnitude (curve "knee"). 0.0 when the curve is a clean line.
    pub knee_pos: f64,
    /// Coefficient of variation: std / max(|mean|, 1e-9); falls back to
    /// std / max(range, 1e-9) when mean ≈ 0.
    pub cv: f64,
    /// (p75 - p25) / max(|median|, 1e-6) — relative interquartile span.
    pub p25_p75_span: f64,
    /// Fraction of adjacent steps that follow the dominant sign.
    pub monotonic_ratio: f64,
    /// Phase estimate: dominant run length of same-sign steps
    /// (n / (1 + turning points)); 0 for an inert/flat curve.
    pub phase: f64,
}

/// Six shape metrics; each bucketed into 4 bands by calibration bounds.
pub const MORPH_METRICS: [&str; 6] = [
    "slope",
    "knee_pos",
    "cv",
    "p25_p75_span",
    "monotonic_ratio",
    "phase",
];

/// Built-in default bucket bounds (lab-seeded defaults; replace in
/// spec/curve_morph_calibration.json with fleet-calibrated tables).
/// Bounds are `[b1, b2, b3]` splitting each metric into:
/// band0 < b1 < band1 < b2 < band2 < b3 < band3.
const DEFAULT_BOUNDS: [(&str, [f64; 3]); 6] = [
    ("slope", [0.0005, 0.01, 0.05]),
    ("knee_pos", [0.15, 0.35, 0.65]),
    ("cv", [0.01, 0.08, 0.30]),
    ("p25_p75_span", [0.05, 0.30, 1.00]),
    ("monotonic_ratio", [0.20, 0.50, 0.85]),
    ("phase", [1.5, 3.0, 8.0]),
];

pub const BAND_NAMES: [&str; 4] = ["low", "mid", "high", "extreme"];

fn calibration_bounds() -> &'static [(&'static str, [f64; 3]); 6] {
    static CAL: OnceLock<Option<[(&'static str, [f64; 3]); 6]>> = OnceLock::new();
    CAL.get_or_init(|| {
        let Ok(raw) = std::fs::read_to_string(
            "spec/curve_morph_calibration.json",
        ) else {
            return None;
        };
        let Ok(v) = serde_json::from_str::<Value>(&raw) else {
            return None;
        };
        let mut out: [(&'static str, [f64; 3]); 6] = DEFAULT_BOUNDS;
        for (name, def) in out.iter_mut() {
            if let Some(bs) = v
                .get("bounds")
                .and_then(|b| b.get(*name))
                .and_then(|b| b.as_array())
            {
                let mut got = [0.0f64; 3];
                let mut ok = true;
                for (i, b) in bs.iter().enumerate().take(3) {
                    match b.as_f64() {
                        Some(x) => got[i] = x,
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                if ok && got[0] < got[1] && got[1] < got[2] {
                    *def = got;
                }
            }
        }
        Some(out)
    })
    .as_ref()
    .unwrap_or(&DEFAULT_BOUNDS)
}

/// Bucket band name for one metric's value per calibration bounds.
pub fn morph_bucket(metric: &str, value: f64) -> &'static str {
    let Some((_, bounds)) = calibration_bounds().iter().find(|(m, _)| *m == metric) else {
        return "unknown";
    };
    if value < bounds[0] {
        BAND_NAMES[0]
    } else if value < bounds[1] {
        BAND_NAMES[1]
    } else if value < bounds[2] {
        BAND_NAMES[2]
    } else {
        BAND_NAMES[3]
    }
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted[idx]
}

/// Summarize a numeric curve into its shape description.
/// Returns None for <4 points or non-finite input (curve not usable).
pub fn summarize_curve(values: &[f64]) -> Option<CurveMorph> {
    if values.len() < 4 || values.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let n = values.len();
    let slope = (values[n - 1] - values[0]) / (n - 1) as f64;

    // Second differences → knee position
    let mut knee_pos = 0.0f64;
    if n >= 4 {
        let mut max_d2 = 0.0f64;
        let mut max_i = 0usize;
        for i in 1..n - 1 {
            let d2 = (values[i + 1] - 2.0 * values[i] + values[i - 1]).abs();
            if d2 > max_d2 {
                max_d2 = d2;
                max_i = i;
            }
        }
        if max_d2 > 1e-12 {
            knee_pos = max_i as f64 / (n as f64 - 1.0);
        }
    }

    let mean = values.iter().sum::<f64>() / n as f64;
    let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n as f64;
    let std = var.sqrt();
    let denom = if mean.abs() > 1e-9 {
        mean.abs()
    } else {
        let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        (max - min).max(1e-9)
    };
    let cv = std / denom;

    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = quantile(&sorted, 0.5);
    let p25 = quantile(&sorted, 0.25);
    let p75 = quantile(&sorted, 0.75);
    let p25_p75_span = (p75 - p25) / median.abs().max(1e-6);

    // Monotonic ratio: dominant-sign agreement of adjacent steps
    let mut pos = 0usize;
    let mut neg = 0usize;
    let mut diffs = Vec::with_capacity(n - 1);
    for w in values.windows(2) {
        let d = w[1] - w[0];
        if d > 1e-12 {
            pos += 1;
        } else if d < -1e-12 {
            neg += 1;
        }
        diffs.push(d);
    }
    let dom = pos.max(neg) as f64;
    let signed = (pos + neg) as f64;
    let monotonic_ratio = if signed == 0.0 {
        0.0
    } else {
        dom / signed
    };

    // Phase: mean run length of same-sign steps → dominant period estimate
    let turns = diffs
        .windows(2)
        .filter(|w| {
            (w[0] > 1e-12 && w[1] < -1e-12) || (w[0] < -1e-12 && w[1] > 1e-12)
        })
        .count();
    let phase = if turns == 0 {
        0.0
    } else {
        n as f64 / (1.0 + turns as f64)
    };

    Some(CurveMorph {
        slope,
        knee_pos,
        cv,
        p25_p75_span,
        monotonic_ratio,
        phase,
    })
}

fn morph_json(m: &CurveMorph, slot: &str) -> Value {
    let mut buckets = Map::new();
    for metric in MORPH_METRICS {
        let val = match metric {
            "slope" => m.slope,
            "knee_pos" => m.knee_pos,
            "cv" => m.cv,
            "p25_p75_span" => m.p25_p75_span,
            "monotonic_ratio" => m.monotonic_ratio,
            _ => m.phase,
        };
        buckets.insert(metric.to_string(), json!(morph_bucket(metric, val)));
    }
    json!({
        "slot": slot,
        "slope": (m.slope * 1e6).round() / 1e6,
        "knee_pos": (m.knee_pos * 1e6).round() / 1e6,
        "cv": (m.cv * 1e6).round() / 1e6,
        "p25_p75_span": (m.p25_p75_span * 1e6).round() / 1e6,
        "monotonic_ratio": (m.monotonic_ratio * 1e6).round() / 1e6,
        "phase": (m.phase * 1e6).round() / 1e6,
        "bucket": buckets,
    })
}

fn curve_array(fo: &Map<String, Value>, key: &str) -> Vec<f64> {
    fo.get(key)
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_f64())
                .collect::<Vec<f64>>()
        })
        .unwrap_or_default()
}

/// A2 consumption: `curve_morph_{webgl,audio,cpu}` intermediate fields
/// plus a same-surface morph coherence verdict (audio deep vs shallow).
pub fn curve_morph_of(fo: &Map<String, Value>) -> Value {
    let mut out = Map::new();
    let mut coherence = Map::new();
    for (slot, key) in [
        ("webgl", "hw_curve_webgl"),
        ("audio", "hw_curve_audio"),
        ("cpu", "hw_curve_cpu"),
    ] {
        let vals = curve_array(fo, key);
        if let Some(m) = summarize_curve(&vals) {
            out.insert(format!("curve_morph_{slot}"), morph_json(&m, slot));
        }
    }
    // Same-surface coherence: hw_curve_audio (shallow) vs audio_deep_curve (deep)
    // must share cv/span shape buckets; knee/phase may legitimately differ.
    let hw = curve_array(fo, "hw_curve_audio");
    let deep = curve_array(fo, "audio_deep_curve");
    let verdict = match (summarize_curve(&hw), summarize_curve(&deep)) {
        (Some(a), Some(b)) => {
            let agree = ["cv", "p25_p75_span"]
                .iter()
                .filter_map(|m| {
                    let va = match *m {
                        "cv" => a.cv,
                        _ => a.p25_p75_span,
                    };
                    let vb = match *m {
                        "cv" => b.cv,
                        _ => b.p25_p75_span,
                    };
                    Some(morph_bucket(m, va) == morph_bucket(m, vb)).filter(|eq| *eq)
                })
                .count();
            if agree >= 2 {
                "agree"
            } else if agree == 1 {
                "partial"
            } else {
                "disagree"
            }
        }
        _ => "missing",
    };
    coherence.insert("audio_deep_vs_hw".to_string(), json!(verdict));
    json!({
        "algo": "curve_morph_v1",
        "morphs": out,
        "coherence": coherence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_ramp_is_monotonic_flat_knee() {
        let v: Vec<f64> = (0..16).map(|i| 0.2 + i as f64 * 0.01).collect();
        let m = summarize_curve(&v).unwrap();
        assert!((m.monotonic_ratio - 1.0).abs() < 1e-9);
        assert!((m.slope - 0.01).abs() < 1e-12);
        assert_eq!(m.knee_pos, 0.0, "clean line has no knee");
        assert!(m.phase == 0.0, "no turns → inert");
    }

    #[test]
    fn knee_detected_at_known_position() {
        // Flat then steep: max second difference at the transition (index 8)
        let mut v = vec![0.1; 20];
        for (i, x) in v.iter_mut().enumerate().skip(8) {
            *x = 0.1 + (i - 8) as f64 * 0.5;
        }
        let m = summarize_curve(&v).unwrap();
        let expect = 8.0 / 19.0;
        assert!((m.knee_pos - expect).abs() < 0.06, "knee: {}", m.knee_pos);
    }

    #[test]
    fn high_frequency_curve_has_low_monotonicity_and_phase() {
        let v: Vec<f64> = (0..32).map(|i| ((i as f64) * 3.1).sin()).collect();
        let m = summarize_curve(&v).unwrap();
        assert!(m.monotonic_ratio < 0.9, "sine is not monotonic");
        assert!(m.phase > 0.0);
        assert!(m.phase < (32.0 / 3.0) + 1.0, "dominant period ≈ 2π/3.1");
    }

    #[test]
    fn flat_curve_is_tight() {
        let v: Vec<f64> = (0..8).map(|_| 0.25).collect();
        let m = summarize_curve(&v).unwrap();
        assert_eq!(m.cv, 0.0);
        assert_eq!(m.p25_p75_span, 0.0);
        assert_eq!(morph_bucket("cv", 0.0), "low");
    }

    #[test]
    fn buckets_follow_calibration_bounds() {
        let def = DEFAULT_BOUNDS;
        assert_eq!(morph_bucket("slope", 0.0001), BAND_NAMES[0]);
        assert_eq!(morph_bucket("slope", 0.001), BAND_NAMES[1]);
        assert_eq!(morph_bucket("cv", 0.1), BAND_NAMES[2]);
        assert_eq!(morph_bucket("monotonic_ratio", 0.7), BAND_NAMES[2]);
        assert_eq!(morph_bucket("phase", 20.0), BAND_NAMES[3]);
        let _ = def;
    }

    #[test]
    fn short_or_nonfinite_curves_rejected() {
        assert!(summarize_curve(&[0.1, 0.2, 0.3]).is_none());
        assert!(summarize_curve(&[f64::NAN, 0.2, 0.3, 0.4]).is_none());
    }

    #[test]
    fn curve_morph_of_emits_slots_and_coherence() {
        let hw: Vec<f64> = (0..16).map(|i| 0.15 + i as f64 * 0.002).collect();
        let deep: Vec<f64> = hw.iter().map(|x| x * 1.02).collect();
        let mut fo = Map::new();
        fo.insert("hw_curve_audio".into(), json!(hw));
        fo.insert("audio_deep_curve".into(), json!(deep));
        fo.insert("hw_curve_webgl".into(), json!((0..16).map(|i| 0.3 - i as f64 * 0.001).collect::<Vec<_>>()));
        let v = curve_morph_of(&fo);
        assert!(v["morphs"]["curve_morph_audio"].get("slope").is_some());
        assert!(v["morphs"]["curve_morph_webgl"].get("slope").is_some());
        assert!(v["morphs"].get("curve_morph_cpu").is_none());
        assert_eq!(v["coherence"]["audio_deep_vs_hw"], "agree");
        assert!(v["morphs"]["curve_morph_audio"]["bucket"]["cv"].is_string());
    }
}
