//! analyze_mask_v1 — incremental analysis gate (dirty-mask bookkeeping).
//!
//! Session evidence is hashed per family (`fields` / `rest`) plus a product
//! version. When a stored mask matches the current input exactly, analyze skips
//! the full evaluate and returns the stored result; otherwise it recomputes and
//! records which families were dirty. First iteration: whole-analyze gating with
//! per-family dirty reporting — evaluate itself is not yet decomposable into
//! per-subgraph recompute units.

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

pub const ANALYZE_MASK_SCHEMA: &str = "analyze_mask_v1";

fn sha256_hex_short(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    let out = h.finalize();
    out.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()[..16].to_string()
}

/// Bookkeeping keys the analyzer itself writes into session meta. Including them
/// in the mask hash would create a feedback loop (every analyze mutates meta →
/// evidence changes → mask never matches). Stripped before hashing only; the
/// restored evidence is never used for analysis.
const MASK_BOOKKEEPING_KEYS: &[&str] = &[
    "analyzed_mask_v1",
    "analysis_rev",
    "skip_session_probe",
    "session_ticket",
    "pagehide_flush",
    "stop_reason",
    // Session touch timestamps: rewritten on every meta merge / ingest touch,
    // they are upload-clock noise, not evidence content.
    "updated_ms",
    "last_upload_ms",
    // Analyzer-written brain/control warm-start state (persist_brain_control):
    // derived deterministically from the evidence, so it must not dirty the mask.
    // battle_log_history is a rolling array that changes on every analyze.
    "plan_version",
    "plan_epoch",
    "belief",
    "missions",
    "capability_envelope",
    "battle_log",
    "battle_log_history",
    "policy_band",
    "direction_priors",
    "unknown_bucket",
    "prior_belief",
];

/// Deep-strip analyzer bookkeeping keys from an evidence view for hashing.
fn clean_bookkeeping(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut out = Map::new();
            for (k, val) in map {
                if MASK_BOOKKEEPING_KEYS.contains(&k.as_str()) {
                    continue;
                }
                out.insert(k.clone(), clean_bookkeeping(val));
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(clean_bookkeeping).collect()),
        other => other.clone(),
    }
}

/// Deterministic short hash of any JSON value (canonical form = serde to_string).
pub fn value_hash(v: &Value) -> String {
    let s = serde_json::to_string(v).unwrap_or_else(|_| "null".into());
    sha256_hex_short(s.as_bytes())
}

/// Per-family hashes: `fields` gets its own hash; everything else is `rest`.
pub fn family_hashes(evidence: &Value) -> Map<String, Value> {
    let cleaned = clean_bookkeeping(evidence);
    let mut m = Map::new();
    let fields = cleaned.get("fields").cloned().unwrap_or(Value::Null);
    m.insert("fields".into(), json!(value_hash(&fields)));
    let mut rest = cleaned;
    if let Some(o) = rest.as_object_mut() {
        o.remove("fields");
    }
    m.insert("rest".into(), json!(value_hash(&rest)));
    m
}

/// Whole-evidence input hash (fields + rest). Stored alongside the mask and in
/// selection provenance so a replay can prove which input produced a result.
pub fn evidence_input_hash(evidence: &Value) -> String {
    value_hash(&clean_bookkeeping(evidence))
}

/// Build the current mask value for persistence / comparison.
pub fn build_mask(evidence: &Value, product_version: &str) -> Value {
    json!({
        "schema": ANALYZE_MASK_SCHEMA,
        "product_version": product_version,
        "input_hash": evidence_input_hash(evidence),
        "families": Value::Object(family_hashes(evidence)),
    })
}

/// True when the stored mask matches the current input for this product version.
pub fn mask_matches(stored: Option<&Value>, current: &Value) -> bool {
    let Some(s) = stored else {
        return false;
    };
    if s.get("schema").and_then(|v| v.as_str()) != Some(ANALYZE_MASK_SCHEMA) {
        return false;
    }
    if s.get("product_version").and_then(|v| v.as_str())
        != current.get("product_version").and_then(|v| v.as_str())
    {
        return false;
    }
    s.get("families") == current.get("families")
}

/// Families whose hash differs between stored and current. Empty when identical;
/// all families when no stored mask exists.
pub fn dirty_families(stored: Option<&Value>, current: &Value) -> Vec<String> {
    let Some(sf) = stored
        .and_then(|s| s.get("families"))
        .and_then(|v| v.as_object())
    else {
        return current
            .get("families")
            .and_then(|v| v.as_object())
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
    };
    let Some(cf) = current.get("families").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (k, v) in cf {
        if sf.get(k) != Some(v) {
            out.push(k.clone());
        }
    }
    out.sort();
    out
}

/// Stamp selection provenance + dirty-mask observability onto an evaluated
/// result. Shared by the analyze path and the replay tool so both produce
/// byte-identical observability blocks for the same input.
pub fn stamp_observability(
    result: &mut Value,
    evidence: &Value,
    session_id: &str,
    product_version: &str,
    prev_mask: Option<&Value>,
    cur_mask: &Value,
) {
    let input_hash = evidence_input_hash(evidence);
    let sel = crate::selection_provenance::build_selection_record(
        result,
        session_id,
        product_version,
        &input_hash,
    );
    if let Some(obj) = result.as_object_mut() {
        obj.insert("selection".into(), sel);
        obj.insert(
            "analyze_dirty".into(),
            json!({
                "algo": ANALYZE_MASK_SCHEMA,
                "changed_families": dirty_families(prev_mask, cur_mask),
                "mask_matched": mask_matches(prev_mask, cur_mask),
                "input_hash": input_hash,
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deterministic_and_family_granular() {
        let e1 = json!({"fields": {"a": 1}, "visitor_terminal_id": "v"});
        let e2 = json!({"fields": {"a": 1}, "visitor_terminal_id": "v"});
        let e3 = json!({"fields": {"a": 2}, "visitor_terminal_id": "v"});
        assert_eq!(evidence_input_hash(&e1), evidence_input_hash(&e2));
        assert_ne!(evidence_input_hash(&e1), evidence_input_hash(&e3));
        let m1 = build_mask(&e1, "6.0.21");
        let m2 = build_mask(&e2, "6.0.21");
        let m3 = build_mask(&e3, "6.0.21");
        assert!(mask_matches(Some(&m1), &m2));
        assert!(!mask_matches(Some(&m1), &m3));
        let dirty = dirty_families(Some(&m1), &m3);
        assert!(dirty.contains(&"fields".to_string()), "dirty={dirty:?}");
        assert!(!dirty.contains(&"rest".to_string()), "dirty={dirty:?}");
        // Family-only changes are visible even when whole input differs elsewhere.
        let e4 = json!({"fields": {"a": 1}, "visitor_terminal_id": "w"});
        let m4 = build_mask(&e4, "6.0.21");
        assert!(!mask_matches(Some(&m1), &m4));
        let dirty4 = dirty_families(Some(&m1), &m4);
        assert!(
            !dirty4.contains(&"fields".to_string()) && dirty4.contains(&"rest".to_string()),
            "dirty4={dirty4:?}"
        );
        // Product version elevation cannot sneak past the mask.
        let m5 = build_mask(&e1, "6.0.22");
        assert!(!mask_matches(Some(&m1), &m5));
    }

    #[test]
    fn analyzer_bookkeeping_does_not_feed_back() {
        // The analyzer persists the mask itself into session meta; that must not
        // change the evidence hash (otherwise every analyze invalidates the mask).
        let e1 = json!({"fields": {"a": 1}, "meta": {"site_id": "s1"}});
        let e2 = json!({
            "fields": {"a": 1},
            "meta": {"site_id": "s1"},
            "analyzed_mask_v1": {"schema": "analyze_mask_v1"},
            "analysis_rev": 3,
            "skip_session_probe": false,
        });
        let m1 = build_mask(&e1, "6.0.21");
        let m2 = build_mask(&e2, "6.0.21");
        assert!(mask_matches(Some(&m1), &m2), "bookkeeping must not dirty mask");
        assert_eq!(evidence_input_hash(&e1), evidence_input_hash(&e2));
        // Legitimate evidence change still invalidates.
        let e3 = json!({"fields": {"a": 2}, "meta": {"site_id": "s1"}});
        assert!(!mask_matches(Some(&m1), &build_mask(&e3, "6.0.21")));
        // Session touch timestamps are upload-clock noise for the mask.
        let e4 = json!({
            "fields": {"a": 1},
            "meta": {"site_id": "s1"},
            "updated_ms": 1234567890,
            "last_upload_ms": 1234567890,
            "belief": {"p": 1},
            "plan_epoch": 7,
            "battle_log_history": [1, 2, 3],
        });
        assert!(mask_matches(Some(&m1), &build_mask(&e4, "6.0.21")));
    }
}
