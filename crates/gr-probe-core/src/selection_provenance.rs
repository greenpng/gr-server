//! Selection provenance — audit record of how the final device identity was
//! chosen from candidates during analyze. Persisted with the analysis result;
//! the diagnostic projection exposes it for calibration and dispute review.
//!
//! Contract: `selected.device_id` MUST equal the result's final device_id; any
//! lane in `device_id_segments` that is not selected appears in `rejected`.

use serde_json::{json, Value};

pub const SELECTION_SCHEMA: &str = "selection_provenance_v1";

fn final_device_id(result: &Value) -> Option<String> {
    result
        .pointer("/device/device_id")
        .or_else(|| result.pointer("/product/device_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
}

fn segments(result: &Value) -> Value {
    result
        .pointer("/device/device_id_segments")
        .or_else(|| result.get("device_id_segments"))
        .cloned()
        .unwrap_or(json!({}))
}

/// Build the selection record for an evaluated result.
pub fn build_selection_record(
    result: &Value,
    session_id: &str,
    product_version: &str,
    input_hash: &str,
) -> Value {
    let winner = final_device_id(result);
    let mut candidates: Vec<Value> = Vec::new();
    let mut rejected: Vec<Value> = Vec::new();
    if let Some(obj) = segments(result).as_object() {
        for (lane, v) in obj {
            let Some(id) = v.as_str().map(|s| s.to_string()).filter(|s| !s.is_empty()) else {
                continue;
            };
            if winner.as_ref().map(String::as_str) == Some(id.as_str()) {
                candidates.push(json!({"lane": lane, "device_id": id, "role": "selected"}));
            } else {
                rejected.push(json!({
                    "lane": lane,
                    "device_id": id,
                    "reason": "not_selected_lane",
                }));
            }
        }
    }
    // Extra candidate surfaces for single-lane results that carry no segments.
    for (k, path) in [
        ("device_id_candidate", "/device/device_id_candidate"),
        ("class_device_id", "/device/class_device_id"),
        ("device_cluster_id", "/device/device_cluster_id"),
    ] {
        if let Some(s) = result.pointer(path).and_then(|v| v.as_str()) {
            if !s.is_empty()
                && !candidates
                    .iter()
                    .any(|c| c.get("device_id").and_then(|x| x.as_str()) == Some(s))
            {
                candidates.push(json!({
                    "source": k,
                    "device_id": s,
                    "role": if winner.as_ref().map(String::as_str) == Some(s) {
                        "selected"
                    } else {
                        "alternative"
                    },
                }));
            }
        }
    }
    let algo = result
        .pointer("/algo")
        .or_else(|| result.get("algo"))
        .and_then(|v| v.as_str())
        .unwrap_or("evaluate");
    json!({
        "schema": SELECTION_SCHEMA,
        "session_id": session_id,
        "product_version": product_version,
        "input_hash": input_hash,
        "decided_by": algo,
        "mint": {
            "server_mint": result.pointer("/device/server_mint").cloned().unwrap_or(Value::Null),
            "link_or_mint_algo": result
                .pointer("/device/link_or_mint_algo")
                .cloned()
                .unwrap_or(Value::Null),
            "multi_source_mint_gate": result
                .pointer("/device/multi_source_mint_gate")
                .cloned()
                .unwrap_or(Value::Null),
        },
        "selected": winner
            .as_ref()
            .map(|w| json!({"device_id": w}))
            .unwrap_or(Value::Null),
        "candidates": Value::Array(candidates),
        "rejected": Value::Array(rejected),
        "warnings": result
            .pointer("/device/id_warnings")
            .cloned()
            .unwrap_or(Value::Null),
    })
}

fn summary_of(v: Option<&Value>) -> Value {
    match v {
        None => json!({"type": "missing"}),
        Some(Value::Null) => json!({"type": "null"}),
        Some(Value::Object(o)) => json!({"type": "object", "keys": o.len()}),
        Some(Value::Array(a)) => json!({"type": "array", "len": a.len()}),
        Some(Value::String(s)) => {
            json!({"type": "string", "len": s.len(), "head": &s[..s.len().min(40)]})
        }
        Some(other) => json!({"type": "scalar", "value": other}),
    }
}

/// Decision-relevant paths for semantic replay equality. Handler-side volatile
/// attachments (peer_similarity / homogenization / meta / re-gated sdk_return)
/// are intentionally excluded: replay equality means "same decisions from the
/// same input", not "byte-identical JSON".
pub const SEMANTIC_PATHS: &[&str] = &[
    "/device/device_id",
    "/device/device_tier",
    "/device/collision_risk",
    "/device/digest_path",
    "/device/mint_eligible",
    "/selection/selected/device_id",
    "/selection/decided_by",
    "/real_band",
    "/os/score",
    "/os/status",
    "/br/score",
    "/br/status",
    "/rpa/score",
    "/rpa/status",
];

/// Compare two results on the semantic decision paths. Absent on both sides
/// counts as equal; absent on one side counts as a difference.
pub fn semantic_diff(recomputed: &Value, stored: &Value) -> Value {
    let mut out = Vec::new();
    for p in SEMANTIC_PATHS {
        let a = recomputed.pointer(p);
        let b = stored.pointer(p);
        if a != b {
            out.push(json!({
                "path": p,
                "stored": summary_of(b),
                "recomputed": summary_of(a),
            }));
        }
    }
    let changed: Vec<Value> = out.iter().map(|c| c.get("path").cloned().unwrap()).collect();
    json!({
        "identical": out.is_empty(),
        "changed_paths": Value::Array(changed),
        "changes": Value::Array(out),
    })
}

/// Top-level diff between a recomputed result and the stored one (replay tool).
/// `ignore` lists volatile keys (peer-dependent scores) that are allowed to
/// differ between runs without making the replay "changed".
pub fn diff_results(recomputed: &Value, stored: &Value, ignore: &[&str]) -> Value {
    let empty = serde_json::Map::new();
    let ro = recomputed.as_object().unwrap_or(&empty);
    let so = stored.as_object().unwrap_or(&empty);
    let mut keys: Vec<&String> = ro.keys().chain(so.keys()).collect();
    keys.sort();
    keys.dedup();
    let mut changes = Vec::new();
    for k in keys {
        if ignore.contains(&k.as_str()) {
            continue;
        }
        let a = ro.get(k);
        let b = so.get(k);
        if a != b {
            changes.push(json!({
                "key": k,
                "stored": summary_of(b),
                "recomputed": summary_of(a),
            }));
        }
    }
    let changed_keys: Vec<Value> = changes
        .iter()
        .filter_map(|c| c.get("key").cloned())
        .collect();
    json!({
        "identical": changes.is_empty(),
        "changed_keys": Value::Array(changed_keys),
        "changes": Value::Array(changes),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn winner_consistency_with_segments() {
        let result = json!({
            "algo": "evaluate.v6",
            "device": {
                "device_id": "dv4-abc",
                "device_id_segments": {"dv4": "dv4-abc", "dv5": "dv5-xyz", "dv6": ""},
                "id_warnings": ["collision_risk_medium"],
                "server_mint": true,
                "link_or_mint_algo": "link_or_mint_v1",
            }
        });
        let rec = build_selection_record(&result, "s1", "6.0.21", "sha256:deadbeef");
        assert_eq!(rec["schema"], "selection_provenance_v1");
        assert_eq!(rec["selected"]["device_id"], "dv4-abc");
        let lans: Vec<&str> = rec["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["lane"].as_str().unwrap())
            .collect();
        assert!(lans.contains(&"dv4"));
        let rejects: Vec<&str> = rec["rejected"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["lane"].as_str().unwrap())
            .collect();
        assert_eq!(rejects, vec!["dv5"]);
        // The winner must appear once as selected and never in rejected.
        for r in rec["rejected"].as_array().unwrap() {
            assert_ne!(r["device_id"], "dv4-abc");
        }
    }

    #[test]
    fn semantic_diff_tracks_decisions() {
        let a = json!({
            "device": {"device_id": "dv4-x", "device_tier": "k3", "digest_path": "p1"},
            "real_band": "b",
            "os": {"score": 80, "status": "ok"},
            "br": {"score": 70, "status": "ok"},
            "peer_similarity": {"x": 1},
        });
        let b = json!({
            "device": {"device_id": "dv4-x", "device_tier": "k3", "digest_path": "p1"},
            "real_band": "b",
            "os": {"score": 80, "status": "ok"},
            "br": {"score": 80, "status": "ok"},
            "peer_similarity": {"x": 2},
        });
        let d = semantic_diff(&a, &b);
        assert_eq!(d["identical"], false);
        let ck: Vec<&str> = d["changed_paths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(ck, vec!["/br/score"]);
        // Handler-only volatility must not count.
        let d2 = semantic_diff(&a, &a);
        assert_eq!(d2["identical"], true);
    }

    #[test]
    fn diff_detects_and_ignores() {
        let a = json!({"device_id": "x", "peer_similarity": {"b": 1}, "scores": {"os": 50}});
        let b = json!({"device_id": "x", "peer_similarity": {"b": 2}, "scores": {"os": 60}});
        let d = diff_results(&a, &b, &["peer_similarity"]);
        assert_eq!(d["identical"], false);
        let ck: Vec<&str> = d["changed_keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(ck, vec!["scores"]);
        let d2 = diff_results(&a, &b, &["peer_similarity", "scores"]);
        assert_eq!(d2["identical"], true);
        let d3 = diff_results(&a, &a, &[]);
        assert_eq!(d3["identical"], true);
    }
}
