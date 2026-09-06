//! Green V7 results SDK (Rust) — backend only.
//!
//! Thin, read-only client: [`Client::get_result`], [`Client::wait_for_result`],
//! [`Client::query`] and the [`cookie_fields`] helper. It does **not** collect
//! or relay browser probes — probes keep flowing straight to the probe
//! server; this crate only reads stored analysis results per session.
//!
//! Auth: `X-Gr-Sdk-Key` with the site-scoped backend key, never in the browser.
//! Projection: `public` | `sdk` | `diagnostic` (diagnostic needs an ops key).

use std::time::Duration;

use serde_json::{json, Value};

/// Raised by [`Client::wait_for_result`] when the analysis is still pending
/// at the timeout.
#[derive(Debug)]
pub struct AnalysisPending {
    /// The last body observed before giving up.
    pub last_body: Option<Value>,
}

impl std::fmt::Display for AnalysisPending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "analysis_pending")
    }
}

impl std::error::Error for AnalysisPending {}

/// An API-level failure with the HTTP status and parsed error body.
#[derive(Debug)]
pub struct ApiError {
    pub status: Option<u16>,
    pub body: Value,
}

/// Optional v2 result projection controls. Empty values use server policy.
#[derive(Debug, Clone, Default)]
pub struct ResultOptions {
    pub strategy_id: Option<String>,
    pub response_profile: Option<String>,
    pub profile_cap: Option<String>,
    pub lang: Option<String>,
}

impl ApiError {
    pub fn code(&self) -> String {
        self.body
            .pointer("/error/code")
            .or_else(|| self.body.get("error"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string()
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.code())
    }
}

impl std::error::Error for ApiError {}

/// Thin result client bound to one probe server + one site key.
#[derive(Clone)]
pub struct Client {
    base_url: String,
    api_key: String,
    agent: ureq::Agent,
}

impl Client {
    /// `base_url` e.g. `https://probe.example.com`.
    pub fn new(base_url: &str, api_key: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(15))
                .build(),
        }
    }

    fn result_url(&self, session_id: &str, projection: &str) -> String {
        self.result_url_with_options(session_id, projection, &ResultOptions::default())
    }

    fn result_url_with_options(
        &self,
        session_id: &str,
        projection: &str,
        options: &ResultOptions,
    ) -> String {
        let mut query = format!("projection={}", urlencode(projection));
        for (key, value) in [
            ("strategy_id", options.strategy_id.as_deref()),
            ("response_profile", options.response_profile.as_deref()),
            ("profile_cap", options.profile_cap.as_deref()),
            ("lang", options.lang.as_deref()),
        ] {
            if let Some(value) = value.filter(|v| !v.is_empty()) {
                query.push('&');
                query.push_str(key);
                query.push('=');
                query.push_str(&urlencode(value));
            }
        }
        format!(
            "{}/v1/session/{}/result?{}",
            self.base_url,
            urlencode(session_id),
            query
        )
    }

    fn fetch(&self, url: &str) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let resp = self
            .agent
            .get(url)
            .set("Accept", "application/json")
            .set("X-Gr-Sdk-Key", &self.api_key)
            .set("X-Request-Id", &format!("req_{}", now_ms()))
            .call();
        let body = match resp {
            Ok(resp) => resp.into_json()?,
            Err(ureq::Error::Status(code, resp)) => {
                let body = resp.into_json().unwrap_or_else(|_| json!({}));
                return Err(Box::new(ApiError {
                    status: Some(code),
                    body,
                }));
            }
            Err(e) => return Err(Box::new(e)),
        };
        Ok(body)
    }

    /// Fetch one result snapshot (`projection`: public | sdk | diagnostic).
    pub fn get_result(
        &self,
        session_id: &str,
        projection: &str,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        self.fetch(&self.result_url(session_id, projection))
    }

    /// Fetch one result snapshot with explicit v2 projection controls.
    pub fn get_result_with_options(
        &self,
        session_id: &str,
        projection: &str,
        options: &ResultOptions,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        self.fetch(&self.result_url_with_options(session_id, projection, options))
    }

    fn pending(result: &Value) -> bool {
        result
            .pointer("/product_public/meta/analysis_pending")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// Poll until the analysis is no longer pending or the timeout elapses.
    pub fn wait_for_result(
        &self,
        session_id: &str,
        projection: &str,
        timeout: Duration,
        interval: Duration,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let deadline = std::time::Instant::now() + timeout;
        let mut last: Option<Value>;
        loop {
            match self.get_result(session_id, projection) {
                Ok(body) => {
                    last = Some(body.clone());
                    let ok = body.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
                    let has_body = body.get("product_public").is_some()
                        || body.get("sdk_projection").is_some();
                    if ok && has_body && !Self::pending(&body) {
                        return Ok(body);
                    }
                }
                Err(e) => {
                    // transient failures keep polling until the deadline
                    last = Some(Value::String(format!("{e}")));
                }
            }
            if std::time::Instant::now() >= deadline {
                return Err(Box::new(AnalysisPending { last_body: last }));
            }
            std::thread::sleep(interval);
        }
    }

    /// Poll with explicit v2 projection controls.
    pub fn wait_for_result_with_options(
        &self,
        session_id: &str,
        projection: &str,
        options: &ResultOptions,
        timeout: Duration,
        interval: Duration,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let deadline = std::time::Instant::now() + timeout;
        let mut last: Option<Value>;
        loop {
            match self.get_result_with_options(session_id, projection, options) {
                Ok(body) => {
                    last = Some(body.clone());
                    let ok = body.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
                    let has_body = body.get("product_public").is_some()
                        || body.get("sdk_projection").is_some();
                    if ok && has_body && !Self::pending(&body) {
                        return Ok(body);
                    }
                }
                Err(e) => last = Some(Value::String(format!("{e}"))),
            }
            if std::time::Instant::now() >= deadline {
                return Err(Box::new(AnalysisPending { last_body: last }));
            }
            std::thread::sleep(interval);
        }
    }

    /// Query helper: `wait=false` is a single fetch; `wait=true` polls.
    pub fn query(
        &self,
        session_id: &str,
        projection: &str,
        wait: bool,
        timeout: Duration,
        interval: Duration,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        if wait {
            self.wait_for_result(session_id, projection, timeout, interval)
        } else {
            self.get_result(session_id, projection)
        }
    }

    /// Query with explicit v2 projection controls.
    pub fn query_with_options(
        &self,
        session_id: &str,
        projection: &str,
        options: &ResultOptions,
        wait: bool,
        timeout: Duration,
        interval: Duration,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        if wait {
            self.wait_for_result_with_options(session_id, projection, options, timeout, interval)
        } else {
            self.get_result_with_options(session_id, projection, options)
        }
    }
}

/// Extract the site-owner allowlisted cookies captured server-side at session
/// open/ingest (business identifiers). Returns `None` when the site has no
/// cookie allowlist or nothing was captured.
pub fn cookie_fields(result: &Value) -> Option<&Value> {
    for path in ["sdk_projection", "product_public"] {
        if let Some(cf) = result
            .get(path)
            .and_then(|n| n.get("cookie_fields"))
            .filter(|cf| cf.as_object().map(|o| !o.is_empty()).unwrap_or(false))
        {
            return Some(cf);
        }
    }
    result
        .get("cookie_fields")
        .filter(|cf| cf.as_object().map(|o| !o.is_empty()).unwrap_or(false))
}

fn urlencode(s: &str) -> String {
    // Minimal RFC-3986 escape; sessions are `[a-zA-Z0-9_-]` in practice.
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencodes_session_ids() {
        assert_eq!(urlencode("sess_ab-1"), "sess_ab-1");
        assert_eq!(urlencode("a/b c"), "a%2Fb%20c");
    }

    #[test]
    fn pending_detection() {
        assert!(Client::pending(&json!({
            "product_public": {"meta": {"analysis_pending": true}}
        })));
        assert!(!Client::pending(&json!({
            "product_public": {"meta": {"analysis_pending": false}}
        })));
        assert!(!Client::pending(&json!({"sdk_projection": {}})));
    }

    #[test]
    fn cookie_fields_extraction() {
        let r = json!({
            "sdk_projection": {"cookie_fields": {"user_id": "u9"}},
            "product_public": {"cookie_fields": {"user_id": "u9"}},
        });
        assert_eq!(
            cookie_fields(&r).and_then(|c| c.get("user_id")),
            Some(&json!("u9"))
        );
        assert!(cookie_fields(&json!({"product_public": {}})).is_none());
        assert!(cookie_fields(&json!({"sdk_projection": {"cookie_fields": {}}})).is_none());
        let r2 = json!({"cookie_fields": {"cart": "c42"}});
        assert_eq!(
            cookie_fields(&r2).and_then(|c| c.get("cart")),
            Some(&json!("c42"))
        );
    }
}
