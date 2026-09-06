//! Constrained LSH clustering helper for **SDK** consumption (iss/50 C2).
//!
//! V5 does **not** own the tenant account graph. This module clusters
//! session-level `curve_lsh` bit/digests into connected components so the SDK
//! can fuse with `subject_ref` — not a DBSCAN reimplementation of production graph.

use serde_json::{json, Value};
use std::collections::HashMap;

pub const CLUSTER_LSH_ALGO: &str = "curve_lsh_cluster_v1";

/// Hamming-like distance for hex digests (nibble XOR popcount).
pub fn hex_digest_distance(a: &str, b: &str) -> u32 {
    let a = a.as_bytes();
    let b = b.as_bytes();
    let n = a.len().min(b.len());
    let mut d = 0u32;
    for i in 0..n {
        let xa = hex_nibble(a[i]);
        let xb = hex_nibble(b[i]);
        d += (xa ^ xb).count_ones();
    }
    d + ((a.len().abs_diff(b.len()) as u32) * 4)
}

fn hex_nibble(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    }
}

#[derive(Debug, Clone)]
struct Uf {
    p: Vec<usize>,
}

impl Uf {
    fn new(n: usize) -> Self {
        Self {
            p: (0..n).collect(),
        }
    }
    fn find(&mut self, x: usize) -> usize {
        if self.p[x] != x {
            let r = self.find(self.p[x]);
            self.p[x] = r;
        }
        self.p[x]
    }
    fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.p[ra] = rb;
        }
    }
}

/// Cluster sessions by curve LSH digests within `max_distance`.
///
/// Input items: `{ "session_id": "...", "lsh": "hex...", "vtid": optional }`
pub fn cluster_by_curve_lsh(items: &[Value], max_distance: u32) -> Value {
    let n = items.len();
    let mut uf = Uf::new(n);
    let digs: Vec<String> = items
        .iter()
        .map(|v| {
            v.get("lsh")
                .or_else(|| v.get("curve_lsh"))
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string()
        })
        .collect();
    for i in 0..n {
        if digs[i].is_empty() {
            continue;
        }
        for j in (i + 1)..n {
            if digs[j].is_empty() {
                continue;
            }
            if hex_digest_distance(&digs[i], &digs[j]) <= max_distance {
                uf.union(i, j);
            }
        }
    }
    let mut comps: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        let r = uf.find(i);
        comps.entry(r).or_default().push(i);
    }
    let mut clusters = Vec::new();
    for (_root, members) in comps {
        if members.is_empty() {
            continue;
        }
        let sessions: Vec<Value> = members
            .iter()
            .map(|&i| {
                json!({
                    "session_id": items[i].get("session_id"),
                    "vtid": items[i].get("vtid"),
                    "lsh": digs[i],
                })
            })
            .collect();
        clusters.push(json!({
            "size": members.len(),
            "members": sessions,
            "promote_to_commercial_id": false,
        }));
    }
    clusters.sort_by(|a, b| {
        b["size"]
            .as_u64()
            .unwrap_or(0)
            .cmp(&a["size"].as_u64().unwrap_or(0))
    });
    json!({
        "algo": CLUSTER_LSH_ALGO,
        "max_distance": max_distance,
        "n_items": n,
        "n_clusters": clusters.len(),
        "clusters": clusters,
        "sdk_use": "fuse cluster_id with subject_ref; V5 never builds tenant account graph",
        "promote_to_commercial_id": false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn near_digests_cluster() {
        let items = vec![
            json!({"session_id":"s1","lsh":"aabbccdd"}),
            json!({"session_id":"s2","lsh":"aabbccde"}), // 1 nibble diff
            json!({"session_id":"s3","lsh":"ffffffff"}),
        ];
        let out = cluster_by_curve_lsh(&items, 4);
        assert!(out["n_clusters"].as_u64().unwrap() >= 2);
        // s1,s2 should share a component when max_distance allows
        let clusters = out["clusters"].as_array().unwrap();
        let big = clusters.iter().find(|c| c["size"].as_u64() == Some(2));
        assert!(big.is_some(), "{out}");
        assert_eq!(out["promote_to_commercial_id"], false);
    }
}
