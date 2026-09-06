//! Token-first embed gate for `/gr.js?grt=` (tasks/panel/02-site-token.md).
//!
//! Host is log-only. L2 reverse-proxy load presents Host=www, so Host-first
//! would 403 a legitimate request. Empty site tokens are legacy (no gate).

use crate::handlers::AppState;
use serde_json::{json, Value};
use std::collections::HashMap;

/// Pin URLs that mint the site script. `/dist/*` subresources skip this gate.
pub fn is_pin_path(path: &str) -> bool {
    matches!(
        path,
        "/gr.js" | "/dist/gr.js" | "/fe/gr.js" | "/gr.min.js" | "/dist/gr.min.js"
    )
}

/// `GR_EMBED_TOKEN_ENFORCE`: `enforce`/`1`/`true` → 403; `warn`/`0`/`false` → log.
/// Unset: lab/dev/test/local → enforce; otherwise warn.
pub fn embed_token_enforce() -> bool {
    match gr_abi::env::get("EMBED_TOKEN_ENFORCE")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "enforce" | "1" | "true" | "yes" | "on" => true,
        "warn" | "0" | "false" | "no" | "off" => false,
        _ => matches!(
            gr_abi::env::get("DEPLOY_ENV")
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "lab" | "dev" | "test" | "local"
        ),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinDecision {
    Allow,
    AllowWarn(&'static str),
    Deny(&'static str),
}

/// Pure decision used by the HTTP gate and unit tests.
///
/// `token_site_id`: site found by `grt=` (None = unknown/empty token).
/// `legacy_empty_token`: Host-matched site exists and has an empty embed_token.
pub fn decide_pin(
    grt: Option<&str>,
    token_site_id: Option<&str>,
    legacy_empty_token: bool,
    enforce: bool,
) -> PinDecision {
    let tok = grt.map(str::trim).filter(|s| !s.is_empty());
    match tok {
        None => {
            if legacy_empty_token {
                PinDecision::Allow
            } else if enforce {
                PinDecision::Deny("embed_token_required")
            } else {
                PinDecision::AllowWarn("embed_token_missing")
            }
        }
        Some(_) => match token_site_id.map(str::trim).filter(|s| !s.is_empty()) {
            Some(_) => PinDecision::Allow,
            None => {
                if enforce {
                    PinDecision::Deny("embed_token_invalid")
                } else {
                    PinDecision::AllowWarn("embed_token_invalid")
                }
            }
        },
    }
}

fn header_ci<'a>(headers: &'a HashMap<String, String>, name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn request_host(headers: &HashMap<String, String>) -> String {
    let raw = header_ci(headers, "host").unwrap_or("");
    raw.split(',')
        .next()
        .unwrap_or(raw)
        .trim()
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn origin_host(raw: &str) -> String {
    let t = raw.trim();
    let rest = t.split_once("://").map(|(_, r)| r).unwrap_or(t);
    rest.split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

fn site_hosts(site: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for key in ["pv_base", "gv_base"] {
        if let Some(h) = site.get(key).and_then(|v| v.as_str()).map(origin_host) {
            if !h.is_empty() {
                out.push(h);
            }
        }
    }
    if let Some(id) = site.get("site_id").and_then(|v| v.as_str()) {
        out.push(id.to_ascii_lowercase());
    }
    out
}

/// Gate `/gr.js`. `None` = serve the pin. `Some((status, json))` = reject.
pub fn check_pin(
    st: &AppState,
    query: &HashMap<String, String>,
    headers: &HashMap<String, String>,
) -> Option<(u16, Value)> {
    let enforce = embed_token_enforce();
    let grt = query
        .get("grt")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let admin = st.admin.as_ref();
    let token_site = grt.as_deref().and_then(|tok| {
        admin.and_then(|a| a.db.find_site_by_embed_token(tok).ok().flatten())
    });
    let token_site_id = token_site
        .as_ref()
        .and_then(|s| s.get("site_id").and_then(|v| v.as_str()))
        .map(|s| s.to_string());

    let host = request_host(headers);
    let mut legacy_empty = false;
    if grt.is_none() && !host.is_empty() {
        if let Some(admin) = admin {
            if let Ok(Some(dom)) = admin.db.get_domain_by_hostname(&host) {
                let sid = dom
                    .get("site_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !sid.is_empty() {
                    if let Ok(Some(site)) = admin.db.get_site(sid) {
                        let tok = site
                            .get("embed_token")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim();
                        legacy_empty = tok.is_empty();
                    }
                }
            }
        }
    }

    match decide_pin(
        grt.as_deref(),
        token_site_id.as_deref(),
        legacy_empty,
        enforce,
    ) {
        PinDecision::Allow => {
            if let Some(ref site) = token_site {
                let referer = header_ci(headers, "referer").unwrap_or("");
                let origin = header_ci(headers, "origin").unwrap_or("");
                let rh = origin_host(referer);
                let oh = origin_host(origin);
                let hosts = site_hosts(site);
                let host_hit = !host.is_empty() && hosts.iter().any(|h| h == &host);
                let ref_hit = (!rh.is_empty() && hosts.iter().any(|h| h == &rh))
                    || (!oh.is_empty() && hosts.iter().any(|h| h == &oh));
                if !host_hit && !ref_hit && (!host.is_empty() || !rh.is_empty() || !oh.is_empty())
                {
                    log::info!(
                        "embed pin host_log_only site={} host={} referer_host={} origin_host={}",
                        token_site_id.as_deref().unwrap_or(""),
                        host,
                        rh,
                        oh
                    );
                }
            }
            None
        }
        PinDecision::AllowWarn(code) => {
            log::warn!("embed pin warn={code} host={host} enforce=0");
            None
        }
        PinDecision::Deny(code) => {
            log::warn!("embed pin deny={code} host={host} enforce=1");
            Some((
                403,
                json!({"ok": false, "error": code, "hint": "pass ?grt=<site embed_token> on /gr.js"}),
            ))
        }
    }
}

/// `session/open` token↔site check. `None` = ok.
pub fn check_open_token(
    st: &AppState,
    site_id: &str,
    body_token: Option<&str>,
    query_token: Option<&str>,
) -> Option<(u16, Value)> {
    if site_id.is_empty() {
        return None;
    }
    let Some(admin) = st.admin.as_ref() else {
        return None;
    };
    let Ok(Some(site)) = admin.db.get_site(site_id) else {
        return None;
    };
    let expected = site
        .get("embed_token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if expected.is_empty() {
        return None;
    }
    let presented = body_token
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| query_token.map(str::trim).filter(|s| !s.is_empty()));
    match presented {
        Some(tok) if tok == expected => None,
        Some(_) => Some((
            403,
            json!({"ok": false, "error": "embed_token_site_mismatch"}),
        )),
        None => {
            if embed_token_enforce() {
                Some((
                    403,
                    json!({"ok": false, "error": "embed_token_required"}),
                ))
            } else {
                log::warn!("open missing embed_token site={site_id} (warn)");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_paths() {
        assert!(is_pin_path("/gr.js"));
        assert!(is_pin_path("/dist/gr.js"));
        assert!(!is_pin_path("/dist/a1b2c3d4e5f6.min.js"));
        assert!(!is_pin_path("/v1/session/open"));
    }

    #[test]
    fn missing_token_enforce() {
        assert_eq!(
            decide_pin(None, None, false, true),
            PinDecision::Deny("embed_token_required")
        );
        assert_eq!(
            decide_pin(None, None, false, false),
            PinDecision::AllowWarn("embed_token_missing")
        );
        assert_eq!(
            decide_pin(None, None, true, true),
            PinDecision::Allow
        );
    }

    #[test]
    fn invalid_token() {
        assert_eq!(
            decide_pin(Some("grst_nope"), None, false, true),
            PinDecision::Deny("embed_token_invalid")
        );
        assert_eq!(
            decide_pin(Some("grst_nope"), None, false, false),
            PinDecision::AllowWarn("embed_token_invalid")
        );
        assert_eq!(
            decide_pin(Some("grst_ok"), Some("site-a"), false, true),
            PinDecision::Allow
        );
    }
}
