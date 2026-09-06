//! Collision KPI for same-SKU / soft-class pools (iss/32 X-12/X-19).
//!
//! Not a global-uniqueness claim: measures how often pure ServerMint commercial
//! ids **collide** within a fixture pool under shared residual/unit materials,
//! and how often **host separators** split them.

use crate::link_or_mint::{
    apply_server_mint, binder_obs_from_fields, server_mint_commercial_id, LINK_OR_MINT_ALGO,
};
use crate::trust::commercial_projection;
use serde_json::{json, Value};
use std::collections::HashMap;

pub const COLLISION_KPI_ALGO: &str = "collision_kpi_v1";

/// Report collision metrics over a pool of field maps (same-SKU simulation).
///
/// For each pair of observations, compares ServerMint commercial ids.
/// - Same residual/unit class **without** separator → expect collide / high risk
/// - Distinct os_instance or webrtc → expect split
pub fn report_collision_kpi(observations: &[Value]) -> Value {
    if observations.len() < 2 {
        return json!({
            "algo": COLLISION_KPI_ALGO,
            "ok": false,
            "error": "need_at_least_two_observations",
            "note": "not a global UV claim — fixture-pool collision measurement only",
        });
    }

    let mut mint_ids: Vec<String> = Vec::new();
    let mut risk_flags: Vec<bool> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    for (i, o) in observations.iter().enumerate() {
        let mint = apply_server_mint(o);
        let id = mint
            .get("device_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let cr = mint
            .get("collision_risk")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        mint_ids.push(id);
        risk_flags.push(cr);
        labels.push(
            o.get("pool_label")
                .and_then(|v| v.as_str())
                .unwrap_or(&format!("obs_{i}"))
                .to_string(),
        );
    }

    // Unique commercial id count vs n
    let mut id_counts: HashMap<String, usize> = HashMap::new();
    for id in &mint_ids {
        if !id.is_empty() {
            *id_counts.entry(id.clone()).or_insert(0) += 1;
        }
    }
    let n = mint_ids.len();
    let unique_ids = id_counts.len();
    let multi_member: usize = id_counts.values().filter(|c| **c >= 2).sum();
    // Pairwise collision rate among non-empty ids
    let mut pairs = 0usize;
    let mut collide_pairs = 0usize;
    let mut split_pairs = 0usize;
    for i in 0..n {
        for j in (i + 1)..n {
            if mint_ids[i].is_empty() || mint_ids[j].is_empty() {
                continue;
            }
            pairs += 1;
            if mint_ids[i] == mint_ids[j] {
                collide_pairs += 1;
            } else {
                split_pairs += 1;
            }
        }
    }
    let pair_collision_rate = if pairs > 0 {
        collide_pairs as f64 / pairs as f64
    } else {
        0.0
    };
    let pair_split_rate = if pairs > 0 {
        split_pairs as f64 / pairs as f64
    } else {
        0.0
    };

    // Class-level: observations sharing residual class with no separator should collide
    let mut soft_no_sep_ids: Vec<String> = Vec::new();
    let mut soft_with_sep_by_os: HashMap<String, Vec<String>> = HashMap::new();
    for o in observations {
        let obs = binder_obs_from_fields(o);
        if !obs.soft_class {
            continue;
        }
        let id = server_mint_commercial_id(&obs);
        let sep = obs.os_instance_hash.is_some() || obs.webrtc_host_ip_hash.is_some();
        if !sep {
            soft_no_sep_ids.push(id);
        } else {
            let k = obs
                .os_instance_hash
                .clone()
                .or(obs.webrtc_host_ip_hash.clone())
                .unwrap_or_else(|| "sep".into());
            soft_with_sep_by_os.entry(k).or_default().push(id);
        }
    }
    let soft_no_sep_unique = soft_no_sep_ids
        .iter()
        .filter(|s| !s.is_empty())
        .collect::<std::collections::HashSet<_>>()
        .len();
    let soft_no_sep_n = soft_no_sep_ids.len();
    let soft_no_sep_collides = soft_no_sep_n >= 2 && soft_no_sep_unique <= 1;

    // Separators: each OS key should have one mint id; different keys should differ
    let mut sep_ids: Vec<String> = Vec::new();
    for (_k, ids) in &soft_with_sep_by_os {
        if let Some(first) = ids.first() {
            sep_ids.push(first.clone());
        }
    }
    let sep_unique = sep_ids
        .iter()
        .filter(|s| !s.is_empty())
        .collect::<std::collections::HashSet<_>>()
        .len();
    let sep_split_ok = soft_with_sep_by_os.len() <= 1 || sep_unique >= soft_with_sep_by_os.len().min(sep_ids.len());

    let risk_true_n = risk_flags.iter().filter(|r| **r).count();

    json!({
        "algo": COLLISION_KPI_ALGO,
        "ok": true,
        "link_or_mint_algo": LINK_OR_MINT_ALGO,
        "n_observations": n,
        "unique_commercial_ids": unique_ids,
        "multi_member_observations": multi_member,
        "pairwise": {
            "pairs": pairs,
            "collide_pairs": collide_pairs,
            "split_pairs": split_pairs,
            "collision_rate": (pair_collision_rate * 10000.0).round() / 10000.0,
            "split_rate": (pair_split_rate * 10000.0).round() / 10000.0,
        },
        "soft_no_separator_pool": {
            "n": soft_no_sep_n,
            "unique_ids": soft_no_sep_unique,
            "class_collides": soft_no_sep_collides,
            "note": "same soft residual/unit without host separator expected to share mint class",
        },
        "soft_with_separator_pool": {
            "n_os_keys": soft_with_sep_by_os.len(),
            "unique_mint_ids": sep_unique,
            "split_ok": sep_split_ok,
            "note": "distinct os_instance/webrtc should split commercial ids",
        },
        "collision_risk_true_count": risk_true_n,
        "collision_risk_rate": if n > 0 {
            (risk_true_n as f64 / n as f64 * 10000.0).round() / 10000.0
        } else {
            0.0
        },
        "ids_sample": mint_ids.iter().zip(labels.iter()).map(|(id, lab)| json!({
            "label": lab,
            "device_id": id,
        })).collect::<Vec<_>>(),
        "sla": {
            "global_unique_promised": false,
            "goal": "minimize same-SKU collision when separators present; report risk when not",
            "product_use": "consume collision_risk + association_level + this KPI — never claim pure-FE UV",
        },
    })
}

/// Convenience: collision posture for one observation (product/diagnostics attach).
pub fn collision_posture_for_fields(fields: &Value) -> Value {
    let mint = apply_server_mint(fields);
    let proj = commercial_projection(fields);
    json!({
        "algo": COLLISION_KPI_ALGO,
        "server_mint_id": mint.get("device_id"),
        "collision_risk": mint.get("collision_risk").or_else(|| proj.get("collision_risk")),
        "soft_has_host_separator": proj.get("soft_has_host_separator"),
        "uniqueness_marker": mint.get("uniqueness_marker"),
    })
}

/// X-20 profile-boundary matrix: same-config residual/unit vs per-profile independent noise.
///
/// - `same_config`: observations share residual+unit (may collide as config cluster)
/// - `per_profile_noise`: each observation has distinct residual class (must split)
pub fn report_profile_boundary_kpi(same_config: &[Value], per_profile_noise: &[Value]) -> Value {
    let same = report_collision_kpi(same_config);
    let indep = report_collision_kpi(per_profile_noise);
    let same_collides = same
        .pointer("/soft_no_separator_pool/class_collides")
        .and_then(|v| v.as_bool())
        .or_else(|| {
            same.pointer("/pairwise/collision_rate")
                .and_then(|v| v.as_f64())
                .map(|r| r >= 0.8)
        })
        .unwrap_or(false);
    // For non-soft independent profiles, use unique id count / split rate
    let indep_unique = indep
        .get("unique_commercial_ids")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let indep_n = indep
        .get("n_observations")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let indep_splits = indep_n > 0 && indep_unique >= indep_n.saturating_sub(0).max(1)
        || indep
            .pointer("/pairwise/split_rate")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
            >= 0.85;

    json!({
        "algo": "profile_boundary_kpi_v1",
        "ok": true,
        "same_config_pool": same,
        "per_profile_noise_pool": indep,
        "boundary": {
            "same_config_may_share_class_id": same_collides
                || same.get("unique_commercial_ids").and_then(|v| v.as_u64()).unwrap_or(99) <= 2,
            "per_profile_must_split": indep_splits,
            "pass": (same_collides
                || same.get("unique_commercial_ids").and_then(|v| v.as_u64()).unwrap_or(99) <= 2)
                && indep_splits,
        },
        "sla": {
            "global_unique_promised": false,
            "note": "same-config cluster bind vs per-profile residual noise split — not pure-FE UV",
        },
    })
}
