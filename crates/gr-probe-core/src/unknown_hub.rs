//! iss/38 P2-4: aggregate session unknown_bucket records into Hub-shaped summary.

use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// Input: list of session meta fragments each optionally containing `unknown_bucket`.
pub fn aggregate_unknown_buckets(sessions: &[Value]) -> Value {
    let mut by_code: HashMap<String, u64> = HashMap::new();
    let mut by_tag: HashMap<String, u64> = HashMap::new();
    let mut sessions_with = 0u64;
    let mut drafts: Vec<Value> = Vec::new();

    for s in sessions {
        let ub = s
            .get("unknown_bucket")
            .or_else(|| s.pointer("/meta/unknown_bucket"))
            .cloned()
            .unwrap_or(Value::Null);
        if ub.is_null() {
            continue;
        }
        sessions_with += 1;
        if let Some(codes) = ub.get("codes").and_then(|v| v.as_array()) {
            for c in codes {
                if let Some(code) = c.as_str() {
                    *by_code.entry(code.to_string()).or_default() += 1;
                }
            }
        }
        if let Some(code) = ub.get("code").and_then(|v| v.as_str()) {
            *by_code.entry(code.to_string()).or_default() += 1;
        }
        if let Some(tags) = ub.get("tags").and_then(|v| v.as_array()) {
            for t in tags {
                if let Some(tag) = t.as_str() {
                    *by_tag.entry(tag.to_string()).or_default() += 1;
                }
            }
        }
        if let Some(reason) = ub.get("reason").and_then(|v| v.as_str()) {
            *by_tag.entry(reason.to_string()).or_default() += 1;
        }
        // Draft iteration tags (not auto-applied)
        if let Some(obj) = ub.as_object() {
            if obj.get("out_of_envelope").and_then(|v| v.as_bool()).unwrap_or(false)
                || obj.get("present").and_then(|v| v.as_bool()).unwrap_or(false)
            {
                drafts.push(json!({
                    "session_id": s.get("session_id").cloned().unwrap_or(Value::Null),
                    "hint": "review_envelope_or_new_gap",
                    "bucket": ub,
                }));
            }
        }
    }

    let mut codes: Vec<Value> = by_code
        .into_iter()
        .map(|(k, n)| json!({"code": k, "count": n}))
        .collect();
    codes.sort_by(|a, b| {
        b.get("count")
            .and_then(|v| v.as_u64())
            .cmp(&a.get("count").and_then(|v| v.as_u64()))
    });
    let mut tags: Vec<Value> = by_tag
        .into_iter()
        .map(|(k, n)| json!({"tag": k, "count": n}))
        .collect();
    tags.sort_by(|a, b| {
        b.get("count")
            .and_then(|v| v.as_u64())
            .cmp(&a.get("count").and_then(|v| v.as_u64()))
    });

    drafts.truncate(32);
    // Also fold probe_coverage_gap when present on sessions (engine-aware catalog gaps).
    let probe_hub = crate::engine_surface::aggregate_probe_coverage_gaps(sessions);
    json!({
        "algo": "unknown_hub_aggregate_v1",
        "sessions_scanned": sessions.len(),
        "sessions_with_bucket": sessions_with,
        "codes": codes,
        "tags": tags,
        "draft_review": drafts,
        "probe_coverage_gap_hub": probe_hub,
        "note": "Hub draft only — does not auto-edit never_digest or Soft promote; probe_coverage_gap_hub drives B10x catalog work",
    })
}

/// Convenience: wrap single map of session_id → meta.
pub fn aggregate_unknown_from_meta_map(meta_by_sid: &Map<String, Value>) -> Value {
    let sessions: Vec<Value> = meta_by_sid
        .iter()
        .map(|(sid, meta)| {
            json!({
                "session_id": sid,
                "unknown_bucket": meta.get("unknown_bucket").cloned().unwrap_or(Value::Null),
                "meta": meta,
            })
        })
        .collect();
    aggregate_unknown_buckets(&sessions)
}
