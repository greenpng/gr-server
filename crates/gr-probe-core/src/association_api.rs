//! Association domain helpers (iss/48) — pure logic, no probe DB writes.
//!
//! Customer backend generates `subject_ref = sub_v1_ + base64url(HMAC(...))`.
//! V5 stores only the ref + probe context edges; never raw email/phone/user_id.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const SUBJECT_REF_PREFIX: &str = "sub_v1_";
pub const ASSOC_SCHEMA_VERSION: &str = "assoc_v1";

/// Customer-side subject_ref derivation (also used by Python SDK parity tests).
pub fn subject_ref_from_secret(tenant_secret: &str, canonical_subject_id: &str) -> String {
    let mut h = Sha256::new();
    h.update(tenant_secret.as_bytes());
    h.update(b"|");
    h.update(canonical_subject_id.as_bytes());
    let dig = h.finalize();
    // base64url without padding (simple hex for stable cross-lang tests)
    format!("{SUBJECT_REF_PREFIX}{:x}", dig)
}

pub fn validate_subject_ref(s: &str) -> Result<(), String> {
    let s = s.trim();
    if !s.starts_with(SUBJECT_REF_PREFIX) {
        return Err("subject_ref_must_start_with_sub_v1_".into());
    }
    let rest = &s[SUBJECT_REF_PREFIX.len()..];
    if rest.len() < 16 || rest.len() > 128 {
        return Err("subject_ref_length".into());
    }
    // Reject obvious plaintext PII patterns
    let lower = s.to_ascii_lowercase();
    for banned in ["@", "phone", "email", "user_id=", "mailto:"] {
        if lower.contains(banned) {
            return Err("subject_ref_looks_like_pii".into());
        }
    }
    if rest.chars().any(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '-')) {
        return Err("subject_ref_charset".into());
    }
    Ok(())
}

/// Reject client-supplied raw identity fields in association payloads.
pub fn reject_raw_pii_fields(attrs: &Value) -> Result<(), String> {
    let Some(obj) = attrs.as_object() else {
        return Ok(());
    };
    for k in [
        "email",
        "phone",
        "user_id",
        "password",
        "card_number",
        "ssn",
        "name",
        "full_name",
    ] {
        if obj.contains_key(k) {
            return Err(format!("forbidden_field:{k}"));
        }
    }
    Ok(())
}

pub fn assess_from_events(
    tenant_id: &str,
    subject_ref: &str,
    events: &[Value],
    labels: &[Value],
) -> Value {
    let n = events.len() as i64;
    let mut devices: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut last_identity = "not_linkable".to_string();
    let mut last_device = Value::Null;
    let mut last_session = Value::Null;
    for e in events {
        if let Some(d) = e
            .pointer("/probe_context/device_id")
            .or_else(|| e.get("device_id"))
            .and_then(|v| v.as_str())
        {
            if !d.is_empty() {
                devices.insert(d.to_string());
                last_device = json!(d);
            }
        }
        if let Some(s) = e
            .pointer("/probe_context/session_id")
            .or_else(|| e.get("session_id"))
            .and_then(|v| v.as_str())
        {
            last_session = json!(s);
        }
        if let Some(ist) = e
            .pointer("/probe_context/identity_state")
            .or_else(|| e.get("identity_state"))
            .and_then(|v| v.as_str())
        {
            last_identity = ist.to_string();
        }
    }
    // Only labels for THIS subject_ref count (never tenant-wide bleed).
    let fraud_labels = labels
        .iter()
        .filter(|l| {
            let label_subj = l.get("subject_ref").and_then(|v| v.as_str()).unwrap_or("");
            if label_subj != subject_ref {
                return false;
            }
            l.get("outcome")
                .and_then(|v| v.as_str())
                .map(|o| o.contains("fraud") || o.contains("chargeback") || o.contains("takeover"))
                .unwrap_or(false)
        })
        .count();
    let decision = if fraud_labels > 0 {
        "review"
    } else if last_identity == "linkable" && devices.len() <= 3 {
        "monitor"
    } else if last_identity == "not_linkable" {
        "allow"
    } else {
        "monitor"
    };
    // Heuristic risk only — not calibrated 1-P_fp (iss/48).
    let risk = (0.15 + (devices.len() as f64) * 0.05 + fraud_labels as f64 * 0.25).min(0.95);
    json!({
        "assessment_id": format!("asm_{}", &short_id(tenant_id, subject_ref)[..12]),
        "schema_version": ASSOC_SCHEMA_VERSION,
        "tenant_id": tenant_id,
        "subject_ref": subject_ref,
        "decision": decision,
        "risk_score": risk,
        "risk_score_version": "heuristic_v0",
        "association": {
            "identity_state": last_identity,
            "association_level": "env",
            "device_status": if devices.len() <= 1 { "single_device" } else { "multi_device" },
            "same_subject_devices": devices.len(),
            "event_count": n,
            "label_count": labels.len(),
            "collision_risk": devices.len() > 5,
            "last_device_id": last_device,
            "last_session_id": last_session,
            "evidence": ["backend_observe_events"],
        },
        "reasons": if fraud_labels > 0 {
            vec!["prior_fraud_label"]
        } else {
            vec!["heuristic_association_v0"]
        },
        "note": "risk_score is heuristic_v0 — not calibrated false-positive probability",
    })
}

fn short_id(a: &str, b: &str) -> String {
    let mut h = Sha256::new();
    h.update(a.as_bytes());
    h.update(b.as_bytes());
    format!("{:x}", h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_ref_hmac_shape_and_pii_reject() {
        let r = subject_ref_from_secret("tenant-secret", "user-42");
        assert!(r.starts_with("sub_v1_"));
        assert!(validate_subject_ref(&r).is_ok());
        assert!(validate_subject_ref("user@x.com").is_err());
        assert!(validate_subject_ref("sub_v1_a@b").is_err());
        assert!(reject_raw_pii_fields(&json!({"email": "a@b.c"})).is_err());
        assert!(reject_raw_pii_fields(&json!({"customer_segment": "seller"})).is_ok());
    }

    #[test]
    fn assess_idempotent_identity_fields() {
        let events = vec![json!({
            "probe_context": {
                "device_id": "dv0-aaaaaaaaaa-bbbbbbbbbb-cccccccccc-dddddddddd-0-0-0-0-0-0",
                "session_id": "cycle_x",
                "identity_state": "linkable"
            }
        })];
        let a = assess_from_events("site1", "sub_v1_abc", &events, &[]);
        assert_eq!(a["association"]["identity_state"], "linkable");
        assert_eq!(a["risk_score_version"], "heuristic_v0");
        assert_eq!(a["tenant_id"], "site1");
    }

    #[test]
    fn assess_ignores_other_subject_fraud_labels() {
        let events = vec![json!({
            "probe_context": {
                "device_id": "dv0-aaaaaaaaaa-bbbbbbbbbb-cccccccccc-dddddddddd-0-0-0-0-0-0",
                "identity_state": "linkable"
            }
        })];
        // Fraud on subject A must not pollute subject B
        let foreign = vec![json!({
            "subject_ref": "sub_v1_subject_a",
            "outcome": "fraud_chargeback"
        })];
        let b = assess_from_events("site1", "sub_v1_subject_b", &events, &foreign);
        assert_ne!(b["decision"], "review");
        let own = vec![json!({
            "subject_ref": "sub_v1_subject_b",
            "outcome": "fraud_chargeback"
        })];
        let b2 = assess_from_events("site1", "sub_v1_subject_b", &events, &own);
        assert_eq!(b2["decision"], "review");
    }
}
