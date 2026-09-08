//! Pingora session helpers: read body, write JSON/bytes, CORS.

use bytes::Bytes;
use pingora::prelude::*;
use pingora_http::ResponseHeader;
use pingora_proxy::Session;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, RwLock};

/// Hostnames registered via admin panel domains (and optional extras).
/// Multi-tenant production: **customer sites configure hostnames + SSL in admin** —
/// do not hardcode customer origins in env. Lab may still use GR_CORS_ORIGINS list.
static DYNAMIC_CORS_HOSTS: LazyLock<RwLock<HashSet<String>>> =
    LazyLock::new(|| RwLock::new(HashSet::new()));

/// Multi-worker: domain upsert hits admin process only; ingest/gateway need a way
/// to re-read admin SQLite when an Origin is denied. Registered at hub attach.
static CORS_LAZY_REFRESH: LazyLock<RwLock<Option<std::sync::Arc<dyn Fn() + Send + Sync>>>> =
    LazyLock::new(|| RwLock::new(None));
static CORS_LAZY_LAST_MS: LazyLock<std::sync::atomic::AtomicI64> =
    LazyLock::new(|| std::sync::atomic::AtomicI64::new(0));

/// Install a process-wide CORS reloader (typically reads admin domains table).
pub fn set_cors_lazy_refresh(f: impl Fn() + Send + Sync + 'static) {
    if let Ok(mut g) = CORS_LAZY_REFRESH.write() {
        *g = Some(std::sync::Arc::new(f));
    }
}

fn maybe_lazy_refresh_cors() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let last = CORS_LAZY_LAST_MS.load(std::sync::atomic::Ordering::Relaxed);
    // Throttle admin CORS host re-read (shared PostgreSQL admin store).
    if now.saturating_sub(last) < 1500 {
        return;
    }
    CORS_LAZY_LAST_MS.store(now, std::sync::atomic::Ordering::Relaxed);
    if let Ok(g) = CORS_LAZY_REFRESH.read() {
        if let Some(f) = g.as_ref() {
            f();
        }
    }
}

/// Replace dynamic CORS host allowlist (lowercase host only, no scheme/port).
pub fn set_dynamic_cors_hosts(hosts: impl IntoIterator<Item = String>) {
    let mut set = HashSet::new();
    for h in hosts {
        let h = normalize_cors_host(&h);
        if !h.is_empty() {
            set.insert(h);
        }
    }
    if let Ok(mut g) = DYNAMIC_CORS_HOSTS.write() {
        *g = set;
    }
}

/// Merge hostnames into dynamic CORS allowlist (e.g. after domain upsert).
pub fn add_dynamic_cors_hosts(hosts: impl IntoIterator<Item = String>) {
    if let Ok(mut g) = DYNAMIC_CORS_HOSTS.write() {
        for h in hosts {
            let h = normalize_cors_host(&h);
            if !h.is_empty() {
                g.insert(h);
            }
        }
    }
}

fn normalize_cors_host(raw: &str) -> String {
    let s = raw.trim().to_ascii_lowercase();
    if s.is_empty() {
        return String::new();
    }
    // Accept full origins or bare hostnames from admin domains table.
    if let Some(rest) = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")) {
        return rest.split('/').next().unwrap_or("").split(':').next().unwrap_or("").to_string();
    }
    s.split('/').next().unwrap_or("").split(':').next().unwrap_or("").to_string()
}

fn origin_host(origin: &str) -> String {
    normalize_cors_host(origin)
}

fn dynamic_host_allowed(origin: &str) -> bool {
    let host = origin_host(origin);
    if host.is_empty() {
        return false;
    }
    DYNAMIC_CORS_HOSTS
        .read()
        .map(|g| g.contains(&host))
        .unwrap_or(false)
}

pub async fn read_body(session: &mut Session, max: usize) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    loop {
        match session.read_request_body().await {
            Ok(Some(chunk)) => {
                if buf.len() < max {
                    let room = max - buf.len();
                    let take = chunk.len().min(room);
                    buf.extend_from_slice(&chunk[..take]);
                }
            }
            Ok(None) => break,
            Err(e) => return Err(e),
        }
        if buf.len() >= max {
            break;
        }
    }
    Ok(buf)
}

pub fn headers_map(session: &Session) -> HashMap<String, String> {
    let mut m = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for (k, v) in session.req_header().headers.iter() {
        if let Ok(s) = v.to_str() {
            let name = k.as_str().to_ascii_lowercase();
            // Preserve first-seen order of header names (fingerprint signal).
            if !order.iter().any(|x| x == &name) {
                order.push(name.clone());
            }
            m.insert(name, s.to_string());
        }
    }
    if !order.is_empty() {
        m.insert("x-gr-http-header-order".into(), order.join(","));
        // Short stable digest for mint/compare without huge strings
        let dig = {
            use sha2::{Digest, Sha256};
            let mut h = Sha256::new();
            h.update(order.join(",").as_bytes());
            format!("{:x}", h.finalize())[..16].to_string()
        };
        m.insert("x-gr-http-header-order-hash".into(), dig);
    }
    // Method + version for JA4H partial
    let method = session.req_header().method.as_str().to_string();
    if !method.is_empty() {
        m.insert("x-gr-http-method".into(), method);
    }
    let ver = format!("{:?}", session.req_header().version);
    // Debug format like Http2 / Http11 — normalize
    let ver_s = if ver.contains('2') {
        "HTTP/2"
    } else if ver.contains('3') {
        "HTTP/3"
    } else {
        "HTTP/1.1"
    };
    m.insert("x-gr-http-version".into(), ver_s.into());
    m
}

pub fn query_map(session: &Session) -> HashMap<String, String> {
    let mut m = HashMap::new();
    if let Some(q) = session.req_header().uri.query() {
        for pair in q.split('&') {
            if pair.is_empty() {
                continue;
            }
            let mut it = pair.splitn(2, '=');
            let k = it.next().unwrap_or("");
            let v = it.next().unwrap_or("");
            let k = urlencoding_decode(k);
            let v = urlencoding_decode(v);
            if !k.is_empty() {
                m.insert(k, v);
            }
        }
    }
    m
}

fn urlencoding_decode(s: &str) -> String {
    // minimal %XX and + decode
    let s = s.replace('+', " ");
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let h = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(h) = h {
                if let Ok(b) = u8::from_str_radix(h, 16) {
                    out.push(b);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Optional Alt-Svc for H3 discovery (env `GR_H3_ALT_SVC` e.g. `h3=":28480"; ma=86400`).
fn insert_alt_svc(header: &mut ResponseHeader) {
    if let Some(v) = gr_abi::env::get("H3_ALT_SVC") {
        let v = v.trim();
        if !v.is_empty() {
            let _ = header.insert_header("Alt-Svc", v);
        }
    }
}

/// A-SVC-2: CORS origin resolution.
///
/// **Production (multi-tenant):** customer business origins are **not** hardcoded.
/// Hostnames come from admin panel `domains` (dynamic) plus optional env extras.
///
/// Policy (`GR_CORS_ORIGINS`):
/// - `*` or empty — lab open mode (reflect any Origin, or `*`)
/// - `admin` — only admin-registered domain hosts (+ sdk key origins if loaded)
/// - comma list — additional fixed origins (lab helpers / partners only)
///
/// Request Origin is allowed when:
/// 1. env is `*` / empty, or
/// 2. exact match in env comma list, or
/// 3. Origin host is in dynamic admin domain allowlist.
fn cors_allow_origin(session: &Session) -> String {
    let cfg = gr_abi::env::get("CORS_ORIGINS").unwrap_or_else(|| "*".into());
    let cfg = cfg.trim();
    let req_origin = session
        .req_header()
        .headers
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Lab open
    if cfg.is_empty() || cfg == "*" {
        if !req_origin.is_empty() {
            return req_origin;
        }
        return "*".into();
    }

    // Admin-only mode: no fixed customer domains in env
    let admin_only = cfg.eq_ignore_ascii_case("admin");

    if req_origin.is_empty() {
        if admin_only {
            return "null".into();
        }
        return cfg
            .split(',')
            .map(|s| s.trim())
            .find(|s| !s.is_empty() && !s.eq_ignore_ascii_case("admin"))
            .unwrap_or("*")
            .to_string();
    }

    // Dynamic admin domains (customer-configured hostnames)
    if dynamic_host_allowed(&req_origin) {
        return req_origin;
    }
    // Multi-worker: admin panel may have registered a domain after this process started
    // (or on another process). Lazy-reload allowlist once, then re-check.
    maybe_lazy_refresh_cors();
    if dynamic_host_allowed(&req_origin) {
        return req_origin;
    }

    if !admin_only {
        for o in cfg.split(',') {
            let o = o.trim();
            if o.is_empty() || o.eq_ignore_ascii_case("admin") {
                continue;
            }
            if o == "*" || o == req_origin {
                return req_origin.clone();
            }
            // Allow env entry as bare hostname matching Origin host
            if origin_host(o) == origin_host(&req_origin) && !origin_host(o).is_empty() {
                return req_origin.clone();
            }
        }
    }

    // Not allowed — return non-matching sentinel (browser blocks). Prefer not echoing attacker Origin.
    if admin_only {
        return "null".into();
    }
    cfg.split(',')
        .map(|s| s.trim())
        .find(|s| !s.is_empty() && !s.eq_ignore_ascii_case("admin") && *s != "*")
        .unwrap_or("null")
        .to_string()
}

fn insert_cors(header: &mut ResponseHeader, session: &Session) -> Result<()> {
    let origin = cors_allow_origin(session);
    header.insert_header("Access-Control-Allow-Origin", &origin)?;
    header.insert_header(
        "Access-Control-Allow-Methods",
        "GET, POST, PUT, OPTIONS",
    )?;
    header.insert_header(
        "Access-Control-Allow-Headers",
        "Content-Type, Authorization, X-Requested-With, X-Session-Ticket, X-Visitor-Terminal-Id, X-Inject-Path, X-Business-Token, Accept, Origin, Idempotency-Key, X-Request-Id, X-Gr-Sdk-Key, X-Gr-Diagnostic-Key, X-Gr-Ops-Token, X-Gr-Tenant, X-Gr-Result-Token",
    )?;
    // Cross-origin sendBeacon(/s0) and credentialed fetches from the business
    // site require Allow-Credentials + a concrete Origin (not *).
    // Without this Firefox logs CORS failures and B8 early beacons never land
    // → multi-tick never floors → probe looks like "infinite refresh".
    if origin != "*" && !origin.is_empty() {
        header.insert_header("Access-Control-Allow-Credentials", "true")?;
        header.insert_header("Vary", "Origin")?;
    } else if gr_abi::env::get("CORS_ORIGINS")
        .map(|s| s.trim() != "*" && !s.trim().is_empty())
        .unwrap_or(false)
    {
        header.insert_header("Vary", "Origin")?;
    }
    header.insert_header(
        "Access-Control-Expose-Headers",
        "X-Gr-Api-Version, X-Gr-Request-Id, X-Request-Id",
    )?;
    // Dual-domain pin/dist/API (pv/gv on a different origin than the business
    // page) must be embeddable even if the page sets COEP require-corp.
    let _ = header.insert_header("Cross-Origin-Resource-Policy", "cross-origin");
    Ok(())
}

pub async fn respond_json(session: &mut Session, status: u16, body: &Value) -> Result<bool> {
    let bytes = Bytes::from(serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec()));
    let mut header = ResponseHeader::build(status, Some(14))?;
    header.insert_header("Content-Type", "application/json; charset=utf-8")?;
    header.insert_header("Content-Length", bytes.len().to_string())?;
    header.insert_header("Cache-Control", "no-store")?;
    header.insert_header("X-Content-Type-Options", "nosniff")?;
    header.insert_header("X-Gr-Api-Version", "v1")?;
    let req_id = session
        .req_header()
        .headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            format!(
                "req_{:x}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            )
        });
    let _ = header.insert_header("X-Gr-Request-Id", req_id);
    insert_cors(&mut header, session)?;
    insert_alt_svc(&mut header);
    // Keepalive on by default (P-A6); set GR_NO_KEEPALIVE=1 to force close
    if gr_abi::env::get("NO_KEEPALIVE").as_deref() == Some("1") {
        session.set_keepalive(None);
    }
    // Client abort is normal (nav away / CF idle). Always finish as "responded"
    // (Ok(true)): Ok(false) falls through to upstream_peer → "no upstream" ERROR spam.
    if let Err(e) = session
        .write_response_header(Box::new(header), false)
        .await
    {
        if is_client_gone(&e) {
            return Ok(true);
        }
        return Err(e);
    }
    if let Err(e) = session.write_response_body(Some(bytes), true).await {
        if is_client_gone(&e) {
            return Ok(true);
        }
        return Err(e);
    }
    Ok(true)
}

pub async fn respond_bytes(
    session: &mut Session,
    status: u16,
    content_type: &str,
    body: Vec<u8>,
) -> Result<bool> {
    let bytes = Bytes::from(body);
    let mut header = ResponseHeader::build(status, Some(12))?;
    header.insert_header("Content-Type", content_type)?;
    header.insert_header("Content-Length", bytes.len().to_string())?;
    header.insert_header("Cache-Control", "no-store, no-cache")?;
    insert_cors(&mut header, session)?;
    insert_alt_svc(&mut header);
    if gr_abi::env::get("NO_KEEPALIVE").as_deref() == Some("1") {
        session.set_keepalive(None);
    }
    if let Err(e) = session
        .write_response_header(Box::new(header), false)
        .await
    {
        if is_client_gone(&e) {
            return Ok(true);
        }
        return Err(e);
    }
    if let Err(e) = session.write_response_body(Some(bytes), true).await {
        if is_client_gone(&e) {
            return Ok(true);
        }
        return Err(e);
    }
    Ok(true)
}

/// Browser/CF closed the socket before response fully written.
fn is_client_gone(err: &pingora_core::Error) -> bool {
    let s = format!("{err:?}");
    s.contains("Broken pipe")
        || s.contains("Connection reset")
        || s.contains("connection reset")
        || s.contains("ECONNRESET")
        || s.contains("EPIPE")
        || s.contains("WriteError") && (s.contains("pipe") || s.contains("reset"))
}

/// 302 redirect (LB redirect / internal-IP hop, docs/08). Optional sticky
/// cookie value; empty means no Set-Cookie header.
pub async fn respond_redirect(
    session: &mut Session,
    location: &str,
    set_cookie: Option<&str>,
) -> Result<bool> {
    let mut header = ResponseHeader::build(302, Some(10))?;
    header.insert_header("Location", location)?;
    if let Some(c) = set_cookie {
        if !c.is_empty() {
            header.insert_header(
                "Set-Cookie",
                format!("{c}; Path=/; Max-Age=86400; HttpOnly; SameSite=Lax"),
            )?;
        }
    }
    header.insert_header("Content-Length", "0")?;
    header.insert_header("Cache-Control", "no-store, no-cache")?;
    header.insert_header("X-Content-Type-Options", "nosniff")?;
    insert_cors(&mut header, session)?;
    insert_alt_svc(&mut header);
    if let Err(e) = session
        .write_response_header(Box::new(header), true)
        .await
    {
        if is_client_gone(&e) {
            return Ok(true);
        }
        return Err(e);
    }
    Ok(true)
}

pub async fn respond_options(session: &mut Session) -> Result<bool> {
    let mut header = ResponseHeader::build(204, Some(8))?;
    insert_cors(&mut header, session)?;
    header.insert_header("Access-Control-Max-Age", "86400")?;
    header.insert_header("Content-Length", "0")?;
    if let Err(e) = session
        .write_response_header(Box::new(header), true)
        .await
    {
        if is_client_gone(&e) {
            return Ok(true);
        }
        return Err(e);
    }
    Ok(true)
}

/// Serve a file from disk (FE static).
///
/// Cache policy (v5.8.123 — prod root-cause for sticky old FE packs):
/// - **loader** fixed name → `no-store` (bootstrap pulls product_version next).
/// - **?v=product_version** on min.js → long immutable (URL changes every release).
/// - **min.js without ?v=** → short revalidate only (never year-long immutable).
///   Previously all `.min.js` got `max-age=31536000, immutable`, so CDN/browser kept
///   pre-upgrade collectors for up to a year when `withVer` omitted the query.
pub async fn respond_file(session: &mut Session, path: &std::path::Path) -> Result<bool> {
    // P0 fix (2026-09-08, 178 prod wedge): asset reads ran as blocking
    // `std::fs::read` on the service runtime's single worker thread; under the
    // script/asset flood this joined PG round-trips in starving accepts.
    // Read on the blocking pool; the worker only writes the response.
    let read_path = path.to_path_buf();
    let data = match tokio::task::spawn_blocking(move || std::fs::read(&read_path)).await {
        Ok(Ok(d)) => d,
        Ok(Err(_)) => {
            return respond_json(
                session,
                404,
                &serde_json::json!({"ok": false, "error": "not_found"}),
            )
            .await;
        }
        Err(e) => {
            log::error!("respond_file task join failed: {e}");
            return respond_json(
                session,
                500,
                &serde_json::json!({"ok": false, "error": "file_read_task_failed"}),
            )
            .await;
        }
    };
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let ct = match ext {
        "js" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "html" | "htm" => "text/html; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "gif" => "image/gif",
        "map" => "application/json",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    };
    let req_uri = session.req_header().uri.to_string();
    let cache = static_cache_control(&req_uri, &name, ext);
    let busted_url = cache.contains("immutable");
    let bytes = Bytes::from(data);
    let mut header = ResponseHeader::build(200, Some(14))?;
    header.insert_header("Content-Type", ct)?;
    header.insert_header("Content-Length", bytes.len().to_string())?;
    header.insert_header("Cache-Control", cache)?;
    header.insert_header("CDN-Cache-Control", cache)?;
    // Help intermediaries not conflate hashed vs bare URLs.
    if busted_url {
        header.insert_header("Vary", "Accept-Encoding")?;
    }
    insert_cors(&mut header, session)?;
    insert_alt_svc(&mut header);
    if gr_abi::env::get("NO_KEEPALIVE").as_deref() == Some("1") {
        session.set_keepalive(None);
    }
    session
        .write_response_header(Box::new(header), false)
        .await?;
    if let Err(e) = session.write_response_body(Some(bytes), true).await {
        if is_client_gone(&e) {
            return Ok(true);
        }
        return Err(e);
    }
    Ok(true)
}

/// Pure cache policy for FE static responses (unit-tested).
///
/// - pin / bare loader → no-store
/// - request basename has content-hash (8–16 hex) or legacy version path → 1y immutable
/// - bare static without hash → short max-age
///
/// `disk_name` is the on-disk logical filename (after strip); `req_uri` is the client URL.
pub fn static_cache_control(req_uri: &str, disk_name: &str, ext: &str) -> &'static str {
    let name = disk_name.to_ascii_lowercase();
    let has_ver_query = req_uri.contains("?v=") || req_uri.contains("&v=");
    let has_ver_path = req_uri.contains("/dist/v/") || req_uri.contains("/fe/v/");
    let has_gen_path = has_ver_path && req_uri.contains("/g/");
    let req_leaf = {
        let path_only = req_uri.split('?').next().unwrap_or(req_uri);
        path_only
            .rsplit('/')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
    };
    let leaf = if req_leaf.is_empty() {
        name.as_str()
    } else {
        req_leaf.as_str()
    };
    let has_content_hash_name = {
        let stem = leaf
            .trim_end_matches(".min.js")
            .trim_end_matches(".js")
            .trim_end_matches(".wasm")
            .trim_end_matches(".html")
            .trim_end_matches(".css");
        // Standard C pure opaque: entire stem is 8–16 hex (`a1b2c3d4e5f6.min.js`).
        let pure_opaque = {
            let n = stem.len();
            (8..=16).contains(&n) && stem.bytes().all(|b| b.is_ascii_hexdigit())
        };
        // Embedded hash: `gr.race.a1b2c3d4e5f6.min.js` / `nest_frame.eb829e0c23e8.html`
        let embedded = stem
            .rsplit_once('.')
            .map(|(_, tok)| {
                let n = tok.len();
                (8..=16).contains(&n) && tok.bytes().all(|b| b.is_ascii_hexdigit())
            })
            .unwrap_or(false);
        pure_opaque || embedded
    };
    let busted_url = has_ver_query || has_ver_path || has_gen_path || has_content_hash_name;
    let is_pin = matches!(
        name.as_str(),
        "gr.js" | "gr.min.js" | "gr.pin.js" | "gr.pin.min.js"
    );
    let is_bare_loader =
        (name == "gr.loader.min.js" || name == "gr.loader.js") && !has_content_hash_name;
    let is_static_asset = matches!(ext, "js" | "css" | "map" | "wasm" | "html" | "htm");
    if is_pin || is_bare_loader {
        "no-store, no-cache, must-revalidate"
    } else if is_static_asset && busted_url {
        "public, max-age=31536000, immutable"
    } else if is_static_asset {
        "public, max-age=60, must-revalidate"
    } else {
        "public, max-age=300"
    }
}

#[cfg(test)]
mod static_cache_control_tests {
    use super::static_cache_control;

    #[test]
    fn pin_always_no_store() {
        let c = static_cache_control("/g5/gr.js", "gr.min.js", "js");
        assert!(c.contains("no-store"), "{c}");
    }

    #[test]
    fn content_hash_request_is_immutable_even_if_disk_unhashed() {
        let c = static_cache_control(
            "/dist/gr.race.a1b2c3d4e5f6.min.js",
            "gr.race.min.js",
            "js",
        );
        assert!(c.contains("immutable"), "{c}");
        assert!(c.contains("31536000"), "{c}");
    }

    #[test]
    fn pure_opaque_request_is_immutable() {
        let c = static_cache_control(
            "/g5/dist/a1b2c3d4e5f6.min.js",
            "gr.race.min.js",
            "js",
        );
        assert!(c.contains("immutable"), "{c}");
        assert!(c.contains("31536000"), "{c}");
    }

    #[test]
    fn pure_opaque_wasm_immutable() {
        let c = static_cache_control("/g5/dist/c5b97b9ad21c.wasm", "gr_seal_v2.wasm", "wasm");
        assert!(c.contains("immutable"), "{c}");
    }

    #[test]
    fn pure_opaque_html_immutable() {
        let c = static_cache_control("/g5/dist/eb829e0c23e8.html", "nest_frame.html", "html");
        assert!(c.contains("immutable"), "{c}");
    }

    #[test]
    fn bare_min_js_short_cache() {
        let c = static_cache_control("/dist/gr.race.min.js", "gr.race.min.js", "js");
        assert!(c.contains("max-age=60"), "{c}");
        assert!(!c.contains("immutable"), "{c}");
    }

    #[test]
    fn wasm_hashed_immutable() {
        let c = static_cache_control(
            "/g5/dist/gr_seal_v2.c5b97b9ad21c.wasm",
            "gr_seal_v2.wasm",
            "wasm",
        );
        assert!(c.contains("immutable"), "{c}");
    }

    #[test]
    fn nest_frame_hashed_immutable() {
        let c = static_cache_control(
            "/g5/dist/nest_frame.eb829e0c23e8.html",
            "nest_frame.html",
            "html",
        );
        assert!(c.contains("immutable"), "{c}");
    }

    #[test]
    fn legacy_version_path_immutable() {
        let c = static_cache_control(
            "/g5/dist/v/6.0.15/g/abc/gr.entry.min.js",
            "gr.entry.min.js",
            "js",
        );
        assert!(c.contains("immutable"), "{c}");
    }
}
