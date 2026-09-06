//! iss/39 R5: Unknown Hub → rule/pack **draft** promotion (never auto-edit digest).

use serde_json::{json, Value};
use std::collections::HashMap;

/// From Hub aggregate JSON, emit review drafts for rule samples / packs.
/// Does **not** mutate matrix, never_digest, or Soft promote.
pub fn hub_promotion_drafts(hub: &Value) -> Value {
    let mut drafts = Vec::new();
    if let Some(codes) = hub.get("codes").and_then(|v| v.as_array()) {
        for c in codes {
            let code = c.get("code").and_then(|v| v.as_str()).unwrap_or("");
            let count = c.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
            if code.is_empty() || count < 1 {
                continue;
            }
            drafts.push(json!({
                "kind": "gap_or_envelope",
                "code": code,
                "count": count,
                "suggested_action": "review_open_gap_or_add_rule_sample",
                "suggested_pack": suggest_pack(code),
                "auto_apply": false,
            }));
        }
    }
    if let Some(tags) = hub.get("tags").and_then(|v| v.as_array()) {
        for t in tags {
            let tag = t.get("tag").and_then(|v| v.as_str()).unwrap_or("");
            let count = t.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
            if tag.is_empty() {
                continue;
            }
            drafts.push(json!({
                "kind": "tag",
                "tag": tag,
                "count": count,
                "suggested_action": "shadow_rule_sample_candidate",
                "rule_draft": {
                    "id": format!("R_hub_{}", sanitize(tag)),
                    "axis": "os",
                    "weight": 0.05,
                    "then_reasons": [format!("hub_draft_{}", sanitize(tag))],
                    "note": "human review required before active"
                },
                "auto_apply": false,
            }));
        }
    }
    // Cap
    drafts.truncate(48);
    let mut by_kind: HashMap<String, u64> = HashMap::new();
    for d in &drafts {
        let k = d.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
        *by_kind.entry(k.to_string()).or_default() += 1;
    }
    json!({
        "algo": "hub_promotion_drafts_v1",
        "draft_count": drafts.len(),
        "by_kind": by_kind,
        "drafts": drafts,
        "redlines": {
            "auto_edit_never_digest": false,
            "soft_promote_dh": false,
            "client_pack_expand": false
        },
        "note": "drafts only — Ops/lab must promote via PR/canary",
    })
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .take(32)
        .collect()
}

fn suggest_pack(code: &str) -> &'static str {
    let c = code.to_ascii_lowercase();
    if c.contains("cdp") || c.contains("webdriver") || c.contains("rpa") {
        "B12_anti_camouflage"
    } else if c.contains("unit") || c.contains("device") || c.contains("claim") {
        "B10_hw_curves"
    } else if c.contains("sensor") || c.contains("form") || c.contains("emu") {
        "B4_mobile"
    } else if c.contains("envelope") || c.contains("unknown") {
        "B3_system"
    } else {
        "B12_anti_camouflage"
    }
}
