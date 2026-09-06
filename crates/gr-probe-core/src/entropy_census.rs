//! Population entropy census for extended curve slots (iss/50 §7.2 H2 prerequisite).
//!
//! Per-sample `curve_slot_ok` is necessary but not sufficient: many machines can
//! share identical digests. This module surveys a batch of curve descriptor digests
//! and reports collision concentration so ops can demote slot_quality.

use serde_json::{json, Map, Value};
use std::collections::HashMap;

pub const ENTROPY_CENSUS_ALGO: &str = "curve_entropy_census_v1";

/// Survey LSH / short digests for one family across observations.
///
/// `samples`: list of digest strings (e.g. curve_lsh short hashes) for one slot.
pub fn census_slot_digests(slot: &str, samples: &[String]) -> Value {
    let n = samples.len() as f64;
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for s in samples {
        if s.is_empty() || s == "0" {
            continue;
        }
        *counts.entry(s.as_str()).or_default() += 1;
    }
    let distinct = counts.len() as f64;
    let present = counts.values().sum::<usize>() as f64;
    let max_bucket = counts.values().copied().max().unwrap_or(0) as f64;
    // Collision rate: fraction in the largest non-unique bucket mass
    let collision_mass: f64 = counts
        .values()
        .filter(|c| **c > 1)
        .map(|c| *c as f64)
        .sum();
    let collision_rate = if present > 0.0 {
        collision_mass / present
    } else {
        0.0
    };
    let top_share = if present > 0.0 {
        max_bucket / present
    } else {
        0.0
    };
    // Shannon entropy of digest distribution (bits)
    let mut h = 0.0_f64;
    if present > 0.0 {
        for c in counts.values() {
            let p = *c as f64 / present;
            if p > 0.0 {
                h -= p * p.log2();
            }
        }
    }
    let quality_suggest = if present < 8.0 {
        "insufficient_sample"
    } else if top_share >= 0.35 || collision_rate >= 0.5 {
        "demote_slot_quality"
    } else if top_share >= 0.15 {
        "watch"
    } else {
        "ok"
    };
    json!({
        "slot": slot,
        "n_total": n as u64,
        "n_present": present as u64,
        "n_distinct": distinct as u64,
        "max_bucket": max_bucket as u64,
        "top_share": (top_share * 10000.0).round() / 10000.0,
        "collision_rate": (collision_rate * 10000.0).round() / 10000.0,
        "shannon_bits": (h * 10000.0).round() / 10000.0,
        "quality_suggest": quality_suggest,
        "note": "iss/50 H2: population census — demote_slot_quality when top_share high (prod-178 pattern)",
    })
}

/// Run census for multiple slots from product-like curve_descriptors payloads.
pub fn census_from_curve_descriptor_batch(batch: &[Value]) -> Value {
    let mut by_slot: HashMap<String, Vec<String>> = HashMap::new();
    for item in batch {
        let slots = item
            .pointer("/curve_descriptors/slots")
            .or_else(|| item.get("slots"))
            .and_then(|v| v.as_object());
        if let Some(map) = slots {
            for (name, desc) in map {
                let dig = desc
                    .get("lsh")
                    .or_else(|| desc.get("digest"))
                    .or_else(|| desc.get("short"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if !dig.is_empty() {
                    by_slot.entry(name.clone()).or_default().push(dig);
                }
            }
        }
    }
    let mut reports = Map::new();
    for (slot, digs) in &by_slot {
        reports.insert(slot.clone(), census_slot_digests(slot, digs));
    }
    json!({
        "algo": ENTROPY_CENSUS_ALGO,
        "n_observations": batch.len(),
        "slots": reports,
        "policy": "feed quality_suggest into slot_quality weights; never mint raw curves",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_collision_suggests_demote() {
        let digs: Vec<String> = (0..20)
            .map(|i| if i < 12 { "same".into() } else { format!("u{i}") })
            .collect();
        let r = census_slot_digests("wg", &digs);
        assert_eq!(r["quality_suggest"], "demote_slot_quality");
        assert!(r["top_share"].as_f64().unwrap() >= 0.35);
    }

    #[test]
    fn diverse_ok() {
        let digs: Vec<String> = (0..20).map(|i| format!("d{i}")).collect();
        let r = census_slot_digests("au", &digs);
        assert_eq!(r["quality_suggest"], "ok");
    }
}
