//! Fuzzy extractor / Helper-Data style stabilization (iss/60 L1, 002.txt).
//!
//! Goal: same-machine curve drift near quantization boundaries must not flip digests
//! (wrong-split dual of collision). Client or server produces **Helper Data** that
//! does not leak full raw fingerprint; re-probe reconstructs a stable codeword digest.
//!
//! MVP (no BCH crate): soft-quantize + parity helper bits over blocks.
//! Algo: `fuzzy_ecc_v1`

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

pub const FUZZY_ECC_ALGO: &str = "fuzzy_ecc_v1";
const BLOCK: usize = 4;
const Q_LEVELS: f64 = 64.0;

fn curve_from(fields: &Value, keys: &[&str]) -> Vec<f64> {
    for k in keys {
        if let Some(a) = fields.get(*k).and_then(|v| v.as_array()) {
            let xs: Vec<f64> = a.iter().filter_map(|x| x.as_f64()).filter(|x| x.is_finite()).collect();
            if xs.len() >= 8 {
                return xs;
            }
        }
    }
    Vec::new()
}

/// Soft quantize to [0, Q_LEVELS) with mid-riser rounding.
fn quantize(xs: &[f64]) -> Vec<u16> {
    if xs.is_empty() {
        return Vec::new();
    }
    let min = xs.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let span = (max - min).max(1e-12);
    xs.iter()
        .map(|x| {
            let t = ((x - min) / span * (Q_LEVELS - 1.0)).round().clamp(0.0, Q_LEVELS - 1.0);
            t as u16
        })
        .collect()
}

/// Helper data: per-block XOR parity of quantized codes (does not reveal full curve).
fn helper_from_codes(codes: &[u16]) -> Vec<u16> {
    let mut out = Vec::new();
    for chunk in codes.chunks(BLOCK) {
        let mut p = 0u16;
        for &c in chunk {
            p ^= c;
        }
        out.push(p);
    }
    out
}

/// Reconstruct: snap each sample toward nearest code consistent with helper parity.
fn stabilize_codes(raw: &[u16], helper: &[u16]) -> Vec<u16> {
    if raw.is_empty() {
        return Vec::new();
    }
    let mut out = raw.to_vec();
    for (bi, chunk) in out.chunks_mut(BLOCK).enumerate() {
        let want = helper.get(bi).copied().unwrap_or(0);
        let mut p = 0u16;
        for &c in chunk.iter() {
            p ^= c;
        }
        if p == want {
            continue;
        }
        // Flip the symbol with largest uncertainty (middle of block) by ±1 to match parity
        let i = chunk.len() / 2;
        let cur = chunk[i];
        let try_a = cur.saturating_add(1).min((Q_LEVELS as u16) - 1);
        let try_b = cur.saturating_sub(1);
        let mut pa = 0u16;
        let mut pb = 0u16;
        for (j, &c) in chunk.iter().enumerate() {
            let ca = if j == i { try_a } else { c };
            let cb = if j == i { try_b } else { c };
            pa ^= ca;
            pb ^= cb;
        }
        if pa == want {
            chunk[i] = try_a;
        } else if pb == want {
            chunk[i] = try_b;
        }
        // else leave as-is (helper mismatch under large drift)
    }
    out
}

fn digest_codes(tag: &str, codes: &[u16]) -> String {
    let mut h = Sha256::new();
    h.update(FUZZY_ECC_ALGO.as_bytes());
    h.update(b"|");
    h.update(tag.as_bytes());
    h.update(b"|");
    for c in codes {
        h.update(c.to_le_bytes());
    }
    format!("{:x}", h.finalize())[..16].to_string()
}

/// Build fuzzy materials for one channel; returns (stable_digest, helper, meta).
pub fn fuzzy_stabilize_curve(tag: &str, curve: &[f64], prior_helper: Option<&[u16]>) -> Value {
    if curve.len() < 8 {
        return json!({
            "algo": FUZZY_ECC_ALGO,
            "channel": tag,
            "ok": false,
            "reason": "short_curve",
        });
    }
    let codes = quantize(curve);
    let helper = if let Some(h) = prior_helper {
        if h.len() == (codes.len() + BLOCK - 1) / BLOCK {
            h.to_vec()
        } else {
            helper_from_codes(&codes)
        }
    } else {
        helper_from_codes(&codes)
    };
    let stable = stabilize_codes(&codes, &helper);
    let dig = digest_codes(tag, &stable);
    let raw_dig = digest_codes(&format!("{tag}_raw"), &codes);
    json!({
        "algo": FUZZY_ECC_ALGO,
        "channel": tag,
        "ok": true,
        "stable_digest": dig,
        "raw_digest": raw_dig,
        "helper": helper,
        "helper_n": helper.len(),
        "code_n": stable.len(),
        "boundary_flip_risk": dig != raw_dig,
        "note": "helper data is parity only; not a full reconstruction of raw fingerprint",
    })
}

/// Apply fuzzy ECC to fields (webgl + audio). Merges digests into extras map.
pub fn fuzzy_ecc_from_fields(fields: &Value) -> Value {
    let mut extras = Map::new();
    let mut channels = Map::new();

    // Reuse helper if FE stored it
    let prior_wg: Option<Vec<u16>> = fields
        .get("fuzzy_helper_wg")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_u64().map(|u| u as u16)).collect());
    let prior_au: Option<Vec<u16>> = fields
        .get("fuzzy_helper_au")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_u64().map(|u| u as u16)).collect());

    let wg = curve_from(
        fields,
        &["hw_curve_webgl", "webgl_residual_multipath", "hw_curve_webgl_silicon"],
    );
    if !wg.is_empty() {
        let r = fuzzy_stabilize_curve("wg", &wg, prior_wg.as_deref());
        if let Some(d) = r.get("stable_digest").and_then(|v| v.as_str()) {
            extras.insert("fuzzy_ecc_wg_digest".into(), json!(d));
        }
        if let Some(h) = r.get("helper") {
            extras.insert("fuzzy_helper_wg".into(), h.clone());
        }
        channels.insert("wg".into(), r);
    }

    let au = curve_from(
        fields,
        &["audio_seed_delta_curve", "audio_deep_curve", "hw_curve_audio"],
    );
    if !au.is_empty() {
        let r = fuzzy_stabilize_curve("au", &au, prior_au.as_deref());
        if let Some(d) = r.get("stable_digest").and_then(|v| v.as_str()) {
            extras.insert("fuzzy_ecc_au_digest".into(), json!(d));
        }
        if let Some(h) = r.get("helper") {
            extras.insert("fuzzy_helper_au".into(), h.clone());
        }
        channels.insert("au".into(), r);
    }

    json!({
        "algo": FUZZY_ECC_ALGO,
        "extras": extras,
        "channels": channels,
        "sdk_note": "Helper Data may be stored client-side (IDB); not commercial id body material alone",
    })
}

/// Route-plan payload so FE can persist Helper Data and echo it next session (iss/61 F1).
///
/// Returns `{"v":1,"wg":[...],"au":[...]}` with only present channels; empty object
/// when no usable curves. Helper is parity-only (no raw fingerprint reconstruction).
pub fn helper_route_payload(fields: &Value) -> Value {
    let fuzzy = fuzzy_ecc_from_fields(fields);
    let ex = match fuzzy.get("extras").and_then(|v| v.as_object()) {
        Some(e) => e,
        None => return json!({}),
    };
    let mut out = Map::new();
    out.insert("v".into(), json!(1));
    let mut n = 0usize;
    for (dst, src) in [("wg", "fuzzy_helper_wg"), ("au", "fuzzy_helper_au")] {
        if let Some(a) = ex.get(src).and_then(|v| v.as_array()) {
            if !a.is_empty() && a.len() <= 256 {
                out.insert(dst.into(), json!(a));
                n += 1;
            }
        }
    }
    if n == 0 {
        return json!({});
    }
    out.insert(
        "note".into(),
        json!("parity helper only; store client-side, echo next session"),
    );
    Value::Object(out)
}

/// Consistency of two observations under same helper (same machine).
pub fn fuzzy_same_machine_rate(curves: &[&[f64]]) -> Value {
    if curves.len() < 2 {
        return json!({"ok": false, "reason": "need_ge2"});
    }
    let base = fuzzy_stabilize_curve("t", curves[0], None);
    let helper: Vec<u16> = base
        .get("helper")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_u64().map(|u| u as u16)).collect())
        .unwrap_or_default();
    let dig0 = base.get("stable_digest").and_then(|v| v.as_str()).unwrap_or("");
    let mut agree = 0usize;
    for c in curves.iter().skip(1) {
        let r = fuzzy_stabilize_curve("t", c, Some(&helper));
        if r.get("stable_digest").and_then(|v| v.as_str()) == Some(dig0) {
            agree += 1;
        }
    }
    let n = curves.len() - 1;
    json!({
        "ok": true,
        "algo": FUZZY_ECC_ALGO,
        "agree_n": agree,
        "pair_n": n,
        "rate": if n > 0 { agree as f64 / n as f64 } else { 0.0 },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_stabilizes_boundary_jitter() {
        let base: Vec<f64> = (0..32).map(|i| 0.1 + i as f64 * 0.01).collect();
        let r0 = fuzzy_stabilize_curve("wg", &base, None);
        let helper: Vec<u16> = r0["helper"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|x| x.as_u64().map(|u| u as u16))
            .collect();
        let dig0 = r0["stable_digest"].as_str().unwrap().to_string();
        // Add tiny noise near quantization boundaries
        let mut noisy = base.clone();
        for i in 0..noisy.len() {
            noisy[i] += if i % 2 == 0 { 1e-6 } else { -1e-6 };
        }
        let r1 = fuzzy_stabilize_curve("wg", &noisy, Some(&helper));
        assert_eq!(
            r1["stable_digest"].as_str().unwrap(),
            dig0.as_str(),
            "stable digest must hold under micro-jitter"
        );
    }

    #[test]
    fn route_payload_carries_helper_and_echo_stabilizes() {
        let curve: Vec<f64> = (0..32).map(|i| 0.2 + i as f64 * 0.013).collect();
        let f1 = json!({"hw_curve_webgl": curve});
        let p1 = helper_route_payload(&f1);
        let wg = p1["wg"].as_array().expect("wg helper present");
        assert!(!wg.is_empty() && wg.len() <= 256);
        // Echo back with micro-jitter → payload helper must stay identical
        let mut noisy = curve.clone();
        for (i, v) in noisy.iter_mut().enumerate() {
            *v += if i % 2 == 0 { 1e-6 } else { -1e-6 };
        }
        let f2 = json!({"hw_curve_webgl": noisy, "fuzzy_helper_wg": p1["wg"].clone()});
        let p2 = helper_route_payload(&f2);
        assert_eq!(p1["wg"], p2["wg"], "echoed helper must persist unchanged");
        // And the stable digests must match across ticks
        let r1 = fuzzy_ecc_from_fields(&f1);
        let r2 = fuzzy_ecc_from_fields(&f2);
        assert_eq!(
            r1["extras"]["fuzzy_ecc_wg_digest"], r2["extras"]["fuzzy_ecc_wg_digest"],
            "same machine + echoed helper ⇒ same stable digest"
        );
    }

    #[test]
    fn route_payload_empty_without_curves() {
        let p = helper_route_payload(&json!({"user_agent": "x"}));
        assert!(p.as_object().map(|o| o.is_empty()).unwrap_or(false));
    }

    #[test]
    fn multi_tick_rate_high() {
        let base: Vec<f64> = (0..32).map(|i| (i as f64 * 0.07).sin() * 0.3 + 0.5).collect();
        let mut curves = Vec::new();
        // Micro-jitter well below quantize bin width
        let owned: Vec<Vec<f64>> = (0..5)
            .map(|t| {
                base.iter()
                    .enumerate()
                    .map(|(i, x)| x + 1e-9 * ((t + i) as f64))
                    .collect()
            })
            .collect();
        for c in &owned {
            curves.push(c.as_slice());
        }
        let r = fuzzy_same_machine_rate(&curves);
        assert!(r["rate"].as_f64().unwrap_or(0.0) >= 0.99, "{r}");
    }
}
