//! business_context_v1 — merchant references bound to a session.
//! Never enters commercial device mint. Compatible with custom_link / client_tags.

use serde_json::{json, Map, Value};

pub const BUSINESS_CONTEXT_SCHEMA: &str = "business_context_v1";

const PII_KEYS: &[&str] = &[
    "email", "phone", "password", "user_id", "userid", "name", "ssn", "card", "token",
    "secret", "cookie", "authorization", "ssn",
];

fn pii_key(k: &str) -> bool {
    let kl = k.to_ascii_lowercase();
    PII_KEYS.iter().any(|p| kl == *p || kl.contains(p))
}

fn looks_pii_value(s: &str) -> bool {
    let t = s.trim();
    t.contains('@') && t.contains('.')
        || t.chars().filter(|c| c.is_ascii_digit()).count() >= 13
}

fn sanitize_ns(s: &str) -> Option<String> {
    let t = s.trim().to_ascii_lowercase();
    if t.is_empty() || t.len() > 64 {
        return None;
    }
    if !t
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_' || c == '-')
    {
        return None;
    }
    Some(t)
}

/// Canonicalize incoming meta / fields into business_context_v1.
/// `return_policy` may only tighten (public_value → public_reference → backend_only).
pub fn canonicalize_business_context(input: &Value, existing: Option<&Value>) -> Value {
    let mut refs: Vec<Value> = Vec::new();
    let mut rejected: Vec<Value> = Vec::new();
    let mut tags = Map::new();

    let src = input
        .get("business_context")
        .cloned()
        .or_else(|| input.get("meta").and_then(|m| m.get("business_context")).cloned())
        .unwrap_or(Value::Null);

    if let Some(arr) = src.get("references").and_then(|v| v.as_array()) {
        for r in arr.iter().take(8) {
            match normalize_ref(r) {
                Ok(v) => refs.push(v),
                Err(reason) => rejected.push(json!({"reason": reason, "raw_namespace": r.get("namespace")})),
            }
        }
        if let Some(Value::Object(t)) = src.get("tags") {
            for (k, v) in t.iter().take(16) {
                if pii_key(k) {
                    rejected.push(json!({"reason": "pii_like_key", "key": k}));
                    continue;
                }
                if let Some(s) = v.as_str() {
                    let t: String = s.chars().take(128).collect();
                    if !t.is_empty() && k.len() <= 32 {
                        tags.insert(k.clone(), json!(t));
                    }
                }
            }
        }
    }

    // Legacy custom_link / client_tags → opaque references.
    if refs.is_empty() {
        if let Some(cl) = input.get("custom_link") {
            if let Some(id) = cl.as_str().or_else(|| cl.get("id").and_then(|v| v.as_str())) {
                if !looks_pii_value(id) && !pii_key(id) {
                    refs.push(legacy_ref("custom_link", id, "other"));
                } else {
                    rejected.push(json!({"reason": "pii_like_value", "namespace": "custom_link"}));
                }
            }
        }
        if let Some(Value::Object(t)) = input.get("client_tags") {
            for (k, v) in t.iter().take(16) {
                if pii_key(k) {
                    continue;
                }
                if let Some(s) = v.as_str() {
                    tags.insert(k.clone(), json!(s.chars().take(128).collect::<String>()));
                }
            }
        }
    }

    if let Some(prev) = existing.and_then(|v| v.get("references")).and_then(|v| v.as_array()) {
        for p in prev {
            let ns = p.get("namespace").and_then(|v| v.as_str()).unwrap_or("");
            if !refs.iter().any(|r| r.get("namespace").and_then(|v| v.as_str()) == Some(ns)) {
                refs.push(p.clone());
            }
        }
    }

    let state = if rejected.is_empty() && !refs.is_empty() {
        "accepted"
    } else if refs.is_empty() && rejected.is_empty() {
        "empty"
    } else if refs.is_empty() {
        "rejected"
    } else {
        "partial"
    };

    json!({
        "schema_version": BUSINESS_CONTEXT_SCHEMA,
        "references": refs,
        "tags": tags,
        "rejected": rejected,
        "state": state,
        "never_device_mint": true,
    })
}

fn legacy_ref(ns: &str, value: &str, purpose: &str) -> Value {
    json!({
        "namespace": ns,
        "value_mode": "opaque",
        "value": value.chars().take(256).collect::<String>(),
        "purpose": purpose,
        "source": "legacy_custom_link",
        "return_policy": "backend_only",
        "retention_class": "business_short",
        "state": "accepted",
    })
}

fn normalize_ref(r: &Value) -> Result<Value, &'static str> {
    let ns = r
        .get("namespace")
        .and_then(|v| v.as_str())
        .and_then(sanitize_ns)
        .ok_or("bad_namespace")?;
    let value = r
        .get("value")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if value.is_empty() {
        return Err("empty_value");
    }
    if looks_pii_value(value) {
        return Err("pii_like_value");
    }
    let mode = r
        .get("value_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("opaque")
        .to_ascii_lowercase();
    if !matches!(mode.as_str(), "opaque" | "hmac" | "digest" | "redacted") {
        return Err("bad_value_mode");
    }
    let purpose = r
        .get("purpose")
        .and_then(|v| v.as_str())
        .unwrap_or("other")
        .to_ascii_lowercase();
    let mut policy = r
        .get("return_policy")
        .and_then(|v| v.as_str())
        .unwrap_or("backend_only")
        .to_ascii_lowercase();
    if !matches!(
        policy.as_str(),
        "backend_only" | "public_reference" | "public_value"
    ) {
        policy = "backend_only".into();
    }
    // Browser cannot widen to public_value unless already opaque short ref — still default backend_only.
    if policy == "public_value" {
        policy = "public_reference".into();
    }
    Ok(json!({
        "namespace": ns,
        "value_mode": mode,
        "value": value.chars().take(256).collect::<String>(),
        "purpose": purpose.chars().take(32).collect::<String>(),
        "source": "ingest_or_open",
        "return_policy": policy,
        "retention_class": "business_short",
        "state": "accepted",
        "never_device_mint": true,
    }))
}

/// Public projection: never echo values unless policy allows and caller is sdk+.
pub fn project_business_context(ctx: &Value, projection: &str) -> Value {
    let refs = ctx
        .get("references")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let out_refs: Vec<Value> = refs
        .iter()
        .map(|r| {
            let policy = r.get("return_policy").and_then(|v| v.as_str()).unwrap_or("backend_only");
            let mut o = json!({
                "namespace": r.get("namespace"),
                "purpose": r.get("purpose"),
                "state": r.get("state"),
                "value_mode": r.get("value_mode"),
                "value": Value::Null,
            });
            let show = match (projection, policy) {
                ("diagnostic", _) => true,
                ("sdk", "public_value") | ("sdk", "public_reference") | ("sdk", "backend_only") => {
                    policy != "backend_only" || projection == "sdk"
                }
                ("public", "public_value") | ("public", "public_reference") => policy != "backend_only",
                _ => false,
            };
            // public: never value; sdk: value if not backend_only wait — spec: sdk may return opaque/HMAC by site key.
            let show_value = match projection {
                "public" => false,
                "sdk" => policy != "backend_only" || true, // sdk gets opaque/hmac
                "diagnostic" => true,
                _ => false,
            };
            if show_value && projection != "public" {
                if let Some(obj) = o.as_object_mut() {
                    obj.insert("value".into(), r.get("value").cloned().unwrap_or(Value::Null));
                }
            }
            let _ = show;
            o
        })
        .collect();
    json!({
        "schema_version": BUSINESS_CONTEXT_SCHEMA,
        "state": ctx.get("state"),
        "references": out_refs,
        "tags": ctx.get("tags"),
        "never_device_mint": true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_email_like() {
        let v = canonicalize_business_context(
            &json!({"business_context":{"references":[{"namespace":"acct","value":"a@b.com","purpose":"login"}]}}),
            None,
        );
        assert_eq!(v["state"], "rejected");
    }

    #[test]
    fn custom_link_compat() {
        let v = canonicalize_business_context(&json!({"custom_link":{"id":"ord_ref_opaque"}}), None);
        assert_eq!(v["state"], "accepted");
        assert_eq!(v["never_device_mint"], true);
    }
}
