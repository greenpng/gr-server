//! Result projection ceiling: public / sdk / diagnostic.
//! Query parameters cannot raise privilege; site keys cap at sdk.

use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultProjection {
    Public,
    Sdk,
    Diagnostic,
}

impl ResultProjection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Sdk => "sdk",
            Self::Diagnostic => "diagnostic",
        }
    }

    pub fn parse(raw: &str) -> Result<Option<Self>, String> {
        let t = raw.trim().to_ascii_lowercase();
        if t.is_empty() {
            return Ok(None);
        }
        match t.as_str() {
            "public" | "product_public" | "merchant" => Ok(Some(Self::Public)),
            "sdk" | "slim" => Ok(Some(Self::Sdk)),
            "diagnostic" | "full" | "internal" | "ops" => Ok(Some(Self::Diagnostic)),
            _ => Err(format!("unknown projection `{raw}` (public|sdk|diagnostic)")),
        }
    }
}

/// iss/opus5 §3.2 P0-3: mirrors the fail-closed gate in
/// `handlers::require_result_token_bound` — open only via explicit escape
/// hatch (`GR_ALLOW_OPEN_RESULTS=1` or `GR_REQUIRE_RESULT_TOKEN=open`).
pub fn result_token_enforced() -> bool {
    let truthy = |v: &str| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on");
    let allow_open = gr_abi::env::get("ALLOW_OPEN_RESULTS")
        .map(|v| truthy(v.trim()))
        .unwrap_or(false);
    let legacy_open = gr_abi::env::get("REQUIRE_RESULT_TOKEN")
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off" | "open"
            )
        })
        .unwrap_or(false);
    !(allow_open || legacy_open)
}

pub fn default_result_projection() -> ResultProjection {
    if let Some(v) = gr_abi::env::get("RESULT_PROJECTION_DEFAULT") {
        if let Ok(Some(p)) = ResultProjection::parse(&v) {
            return p;
        }
    }
    if result_token_enforced() {
        ResultProjection::Public
    } else {
        ResultProjection::Diagnostic
    }
}

fn header_ci<'a>(headers: &'a HashMap<String, String>, name: &str) -> &'a str {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
        .unwrap_or("")
}

/// Ops/admin diagnostic scope. Site SDK / result token is not enough.
pub fn diagnostic_scope_from_headers(headers: &HashMap<String, String>) -> bool {
    if !result_token_enforced() {
        return true;
    }
    let ops = gr_abi::env::get("OPS_TOKEN").unwrap_or_default();
    let ops = ops.trim();
    let xops = header_ci(headers, "x-gr-ops-token").trim();
    let xdiag = header_ci(headers, "x-gr-diagnostic-key").trim();
    let auth = header_ci(headers, "authorization");
    let bearer = auth
        .strip_prefix("Bearer ")
        .or_else(|| auth.strip_prefix("bearer "))
        .unwrap_or("")
        .trim();
    if !ops.is_empty() && (xops == ops || bearer == ops || xdiag == ops) {
        return true;
    }
    if ops.is_empty() && !xops.is_empty() {
        let result = gr_abi::env::get("RESULT_TOKEN").unwrap_or_default();
        if xops == result.trim() && !result.trim().is_empty() {
            return true;
        }
    }
    false
}

pub fn resolve_result_projection(
    headers: &HashMap<String, String>,
    query: &HashMap<String, String>,
    admin_ok: bool,
) -> Result<ResultProjection, (u16, String)> {
    let requested = query
        .get("projection")
        .or_else(|| query.get("view"))
        .map(|s| s.as_str())
        .unwrap_or("");
    let proj = ResultProjection::parse(requested)
        .map_err(|e| (400, e))?
        .unwrap_or_else(default_result_projection);
    let diag_ok = diagnostic_scope_from_headers(headers) || admin_ok;
    if proj == ResultProjection::Diagnostic && !diag_ok {
        return Err((
            403,
            "diagnostic projection requires ops/admin token (site SDK key returns public|sdk only)"
                .into(),
        ));
    }
    Ok(proj)
}

pub fn assemble_result_response(
    session_id: &str,
    result: &Value,
    product_public: Value,
    projection: ResultProjection,
) -> Value {
    let emit_identity = product_public
        .pointer("/sdk_return/emit")
        .and_then(|v| v.as_bool())
        .or_else(|| {
            product_public
                .get("sdk_projection")
                .and_then(|s| s.get("emit_identity"))
                .and_then(|v| v.as_bool())
        })
        .unwrap_or(false);
    let sdk_projection = product_public
        .get("sdk_projection")
        .cloned()
        .unwrap_or_else(|| crate::sdk_slim_projection(&product_public, emit_identity));
    let mut body = json!({
        "ok": true,
        "schema_version": "product_public_v1",
        "session_id": session_id,
        "projection": projection.as_str(),
        "product_public": product_public,
    });
    let obj = body.as_object_mut().expect("object");
    match projection {
        ResultProjection::Public => {
            obj.insert(
                "note".into(),
                json!("product_public is merchant schema; diagnostic requires ops/admin scope"),
            );
        }
        ResultProjection::Sdk => {
            obj.insert("sdk_projection".into(), sdk_projection);
            obj.insert(
                "note".into(),
                json!("sdk_projection is the slim SDK contract"),
            );
        }
        ResultProjection::Diagnostic => {
            obj.insert("sdk_projection".into(), sdk_projection);
            obj.insert("result".into(), result.clone());
            obj.insert(
                "strategies".into(),
                crate::list_strategy_presets()
                    .get("strategies")
                    .cloned()
                    .unwrap_or(Value::Null),
            );
            obj.insert(
                "note".into(),
                json!("diagnostic: product_public + sdk_projection + full internal result"),
            );
        }
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_cannot_self_upgrade_when_token_required() {
        std::env::set_var("GR_REQUIRE_RESULT_TOKEN", "1");
        std::env::set_var("GR_OPS_TOKEN", "ops-secret");
        std::env::set_var("GR_RESULT_TOKEN", "site-secret");
        let mut q = HashMap::new();
        q.insert("projection".into(), "diagnostic".into());
        let mut h = HashMap::new();
        h.insert("x-gr-sdk-key".into(), "site-secret".into());
        let err = resolve_result_projection(&h, &q, false).unwrap_err();
        assert_eq!(err.0, 403);
        std::env::set_var("GR_REQUIRE_RESULT_TOKEN", "0");
    }
}
