//! HTTP `Idempotency-Key` for open / ingest / analyze / complete.
//! Same key + same body → replay stored response. Same key + different body → 409.

use serde_json::{json, Value};
use std::collections::HashMap;

use crate::handlers::{ApiError, AppState};

fn header_ci<'a>(headers: &'a HashMap<String, String>, name: &str) -> &'a str {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
        .unwrap_or("")
}

fn site_of(headers: &HashMap<String, String>) -> String {
    headers
        .get("host")
        .or_else(|| headers.get("Host"))
        .map(|s| s.split(':').next().unwrap_or(s).to_ascii_lowercase())
        .unwrap_or_else(|| "_".into())
}

fn principal_of(headers: &HashMap<String, String>) -> String {
    let raw = header_ci(headers, "x-gr-sdk-key");
    let raw = if raw.is_empty() {
        let auth = header_ci(headers, "authorization");
        auth.strip_prefix("Bearer ")
            .or_else(|| auth.strip_prefix("bearer "))
            .unwrap_or("")
            .trim()
    } else {
        raw
    };
    if raw.is_empty() {
        "anon".into()
    } else {
        let h = gr_probe_core::sha256_hex(raw.as_bytes());
        format!("k:{}", &h[..12.min(h.len())])
    }
}

fn tenant_of(headers: &HashMap<String, String>) -> String {
    format!("{}|{}", site_of(headers), principal_of(headers))
}

fn sanitize_key(raw: &str) -> Result<Option<String>, ApiError> {
    let t = raw.trim();
    if t.is_empty() {
        return Ok(None);
    }
    if t.len() < 8 || t.len() > 128 {
        return Err(ApiError(400, "idempotency_key_invalid".into()));
    }
    if !t
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
    {
        return Err(ApiError(400, "idempotency_key_invalid".into()));
    }
    Ok(Some(t.to_string()))
}

fn require_key() -> bool {
    matches!(
        gr_abi::env::get("REQUIRE_IDEMPOTENCY_KEY").as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

pub enum IdemBegin {
    Fresh { tenant: String, key: String, body_hash: String },
    Replay { status: u16, body: Value },
}

pub fn begin(
    st: &AppState,
    headers: &HashMap<String, String>,
    route: &str,
    body: &[u8],
) -> Result<Option<IdemBegin>, ApiError> {
    let raw = header_ci(headers, "idempotency-key");
    let key = match sanitize_key(raw)? {
        Some(k) => k,
        None => {
            if require_key() {
                return Err(ApiError(400, "idempotency_key_required".into()));
            }
            return Ok(None);
        }
    };
    let tenant = tenant_of(headers);
    let body_hash = gr_probe_core::sha256_hex(body);
    match st
        .store
        .lookup_api_idempotency(&tenant, route, &key)
        .map_err(|e| ApiError(500, e.to_string()))?
    {
        Some(hit) => {
            let stored = hit.get("body_hash").and_then(|v| v.as_str()).unwrap_or("");
            if stored != body_hash {
                return Err(ApiError(409, "idempotency_conflict".into()));
            }
            let status = hit.get("status").and_then(|v| v.as_i64()).unwrap_or(200) as u16;
            let resp = hit.get("response").cloned().unwrap_or(json!({}));
            Ok(Some(IdemBegin::Replay {
                status,
                body: resp,
            }))
        }
        None => Ok(Some(IdemBegin::Fresh {
            tenant,
            key,
            body_hash,
        })),
    }
}

pub fn commit(
    st: &AppState,
    route: &str,
    begun: &IdemBegin,
    status: u16,
    response: &Value,
) {
    let IdemBegin::Fresh {
        tenant,
        key,
        body_hash,
    } = begun
    else {
        return;
    };
    if status >= 500 {
        return;
    }
    let _ = st.store.put_api_idempotency(
        tenant,
        route,
        key,
        body_hash,
        status as i64,
        response,
    );
}

pub fn wrap_json(
    st: &AppState,
    headers: &HashMap<String, String>,
    route: &str,
    body: &[u8],
    run: impl FnOnce() -> Result<Value, ApiError>,
) -> Result<(u16, Value), ApiError> {
    let begun = begin(st, headers, route, body)?;
    if let Some(IdemBegin::Replay { status, body }) = &begun {
        return Ok((*status, body.clone()));
    }
    match run() {
        Ok(v) => {
            if let Some(ref b) = begun {
                commit(st, route, b, 200, &v);
            }
            Ok((200, v))
        }
        Err(e) => {
            if e.0 < 500 && e.0 != 429 {
                if let Some(ref b) = begun {
                    let body = e.to_json();
                    commit(st, route, b, e.0, &body);
                }
            }
            Err(e)
        }
    }
}
