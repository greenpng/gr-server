//! Pingora `ProxyHttp` gateway + API surface for green-v5.
//!
//! Replaces axum Router: all HTTP is terminated here (optionally TLS with ClientHello JA4).

use crate::handlers::{self, AppState, ApiError};
use crate::http_util::{
    headers_map, query_map, read_body, respond_bytes, respond_file, respond_json, respond_options,
    respond_redirect,
};
use crate::upstream::{self, ProxyMode, UpstreamConfig};
use async_trait::async_trait;
use gr_lb::{LbPool, LbTarget};
use pingora::prelude::*;
use pingora_core::upstreams::peer::HttpPeer;
use pingora_proxy::{ProxyHttp, Session};
use serde_json::{json, Value};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;

pub struct GrCtx {
    pub responded: bool,
    /// Transparent mode: forward this request to upstream after edge capture.
    pub forward: bool,
    /// LB mode (docs/guides/08-LB-MODULE.md): chosen routing target; Forward → upstream_peer.
    pub lb_target: Option<LbTarget>,
}

pub struct GrService {
    pub state: Arc<AppState>,
    pub static_dir: PathBuf,
    pub mode: ProxyMode,
    pub upstream: Option<UpstreamConfig>,
    /// Shared LB pool (wired by gr-service control plane; paid feature gate
    /// enforced there). Mode `Lb` without a pool falls back to origin.
    pub lb_pool: Option<Arc<LbPool>>,
}

#[async_trait]
impl ProxyHttp for GrService {
    type CTX = GrCtx;

    fn new_ctx(&self) -> Self::CTX {
        GrCtx {
            responded: false,
            forward: false,
            lb_target: None,
        }
    }

    async fn request_filter(&self, session: &mut Session, ctx: &mut Self::CTX) -> Result<bool>
    where
        Self::CTX: Send + Sync,
    {
        let method = session.req_header().method.as_str().to_string();
        let path = session.req_header().uri.path().to_string();
        let mut headers = headers_map(session);
        // LB mode (docs/guides/08-LB-MODULE.md): PURE header passthrough — the forwarded stream
        // keeps every header byte-identical (visitor's XFF/CF/JA4 family etc.).
        // The local map below is only for routing decisions on this node; the
        // wire request is never rewritten.
        if self.mode != ProxyMode::Lb {
            // Pingora downstream peer — real TCP peer for gateway_fields
            if let Some(addr) = session.client_addr() {
                headers
                    .entry("x-gr-peer-addr".into())
                    .or_insert_with(|| addr.to_string());
                if let Some(inet) = addr.as_inet() {
                    headers.insert("x-gr-peer-ip".into(), inet.ip().to_string());
                    headers.insert("x-gr-peer-port".into(), inet.port().to_string());
                }
                // CF / True-Client-IP always retained (CDN priority).
                // XFF/X-Real only when peer is trusted or loopback (local nginx→pingora).
                let peer_ip = addr.as_inet().map(|a| a.ip().to_string());
                if !handlers::peer_is_trusted_proxy(peer_ip.as_deref()) {
                    headers.remove("x-forwarded-for");
                    headers.remove("x-real-ip");
                    // Do NOT strip cf-connecting-ip / true-client-ip
                    if let Some(ip) = peer_ip {
                        headers.insert("x-real-ip".into(), ip);
                    }
                } else if !headers.contains_key("x-forwarded-for")
                    && !headers.contains_key("x-real-ip")
                    && !headers.contains_key("cf-connecting-ip")
                {
                    if let Some(inet) = addr.as_inet() {
                        headers
                            .entry("x-real-ip".into())
                            .or_insert_with(|| inet.ip().to_string());
                    }
                }
            }
        }
        let query = query_map(session);
        if self.mode != ProxyMode::Lb {
            // PROXY protocol real client (PreTls) — all routes
            crate::listen::inject_proxy_into_headers(&mut headers, session);
            // Connection-scoped TLS ClientHello JA4 (POST gateway + GET /s0)
            crate::tls_fp::inject_into_headers(session, &mut headers);
            // P-V3: TCP_SAVED_SYN + TCP_INFO (Linux)
            crate::listen::tcp_depth::inject_tcp_depth(session, &mut headers);
            // H2 SETTINGS capture
            crate::listen::inject_h2_into_headers(&mut headers, session);
        }

        // Transparent: forward non-local paths after edge capture
        if self.mode == ProxyMode::Transparent
            && self.upstream.is_some()
            && !upstream::is_local_path(&path)
        {
            ctx.forward = true;
            ctx.responded = false;
            // Ok(false) → continue to upstream_peer
            return Ok(false);
        }

        // LB: balance every non-gateway-local path (docs/guides/08-LB-MODULE.md). Pure passthrough:
        // forward stream untouched; 302 for redirect/internal-IP modes with
        // optional sticky cookie; 503 when enabled but no healthy node.
        if self.mode == ProxyMode::Lb {
            if let Some(ref pool) = self.lb_pool {
                if upstream::is_lb_path(&path) {
                    if pool.is_enabled() {
                        let sticky = cookie_value(&headers, "cookie", gr_lb::STICKY_COOKIE);
                        let visitor = lb_visitor_ip(&headers, session);
                        let raw_uri = session.req_header().uri.to_string();
                        match pool.select(visitor, sticky.as_deref(), &raw_uri) {
                            Some(LbTarget::Forward { addr, tls, sni }) => {
                                let target_addr = addr.clone();
                                ctx.lb_target = Some(LbTarget::Forward { addr, tls, sni });
                                log::info!(
                                    "lb forward (passthrough) mode={} target={target_addr} path={path} visitor={visitor:?}",
                                    pool.config().mode
                                );
                                return Ok(false);
                            }
                            Some(LbTarget::Redirect { location, sticky }) => {
                                ctx.responded = true;
                                let cookie = sticky
                                    .clone()
                                    .map(|id| format!("{}={id}", gr_lb::STICKY_COOKIE));
                                log::info!(
                                    "lb redirect mode={} location={location} path={path}",
                                    pool.config().mode
                                );
                                return respond_redirect(session, &location, cookie.as_deref()).await;
                            }
                            None => {
                                ctx.responded = true;
                                log::warn!("lb enabled but no healthy upstream — 503 path={path}");
                                return respond_json(
                                    session,
                                    503,
                                    &json!({"ok": false, "error": {
                                        "code": "lb_no_healthy_upstream",
                                        "retryable": true,
                                    }}),
                                )
                                .await;
                            }
                        }
                    }
                    // Pool present but disabled (no paid license / panel off):
                    // fall through to origin processing on this node.
                }
            } else if !upstream::is_local_path(&path) {
                log::warn!("lb mode without lb_pool — origin fallback path={path}");
            }
        }

        if method == "OPTIONS" {
            ctx.responded = true;
            return respond_options(session).await;
        }

        // Health + FE sdk bootstrap
        // Public /health|/healthz|/v1/health: minimal LB/DNS probe (no DB/recipe leak).
        // /v1/health/detail: loopback-only ops snapshot (still no DSN).
        if (method == "GET" || method == "POST")
            && (path == "/health"
                || path == "/healthz"
                || path == "/livez"
                || path == "/readyz"
                || path == "/v1/health"
                || path == "/v1/health/detail"
                || path == "/v1/livez"
                || path == "/v1/readyz"
                || path == "/v1/sdk/bootstrap")
        {
            ctx.responded = true;
            let body = if path == "/v1/sdk/bootstrap" {
                handlers::sdk_bootstrap(&self.state, &headers, &query, &self.static_dir)
            } else if path == "/v1/health/detail" {
                let peer = headers
                    .get("x-real-ip")
                    .map(|s| s.as_str())
                    .or_else(|| headers.get("x-forwarded-for").map(|s| s.split(',').next().unwrap_or("").trim()))
                    .unwrap_or("");
                let loopback = peer.is_empty()
                    || peer == "127.0.0.1"
                    || peer == "::1"
                    || peer.starts_with("127.");
                if loopback {
                    handlers::health_detail(&self.state)
                } else {
                    // Do not leak recipe to remote clients — same as public health.
                    handlers::health_public(&self.state)
                }
            } else if path == "/livez" || path == "/v1/livez" {
                handlers::health_livez(&self.state)
            } else if path == "/readyz" || path == "/v1/readyz" {
                handlers::health_readyz(&self.state)
            } else {
                handlers::health_public(&self.state)
            };
            let code = if (path == "/readyz" || path == "/v1/readyz")
                && body.get("ok").and_then(|v| v.as_bool()) == Some(false)
            {
                503
            } else {
                200
            };
            return respond_json(session, code, &body).await;
        }

        // Static FE
        if method == "GET" && self.state.role.serves_ingest() {
            if crate::embed_gate::is_pin_path(&path) {
                if let Some((status, body)) =
                    crate::embed_gate::check_pin(&self.state, &query, &headers)
                {
                    ctx.responded = true;
                    return respond_json(session, status, &body).await;
                }
            }
            if let Some(rel) = static_rel(&path, &self.static_dir) {
                let file = resolve_static(&self.static_dir, &rel);
                if file.is_file() {
                    ctx.responded = true;
                    return respond_file(session, &file).await;
                }
            }
            if path == "/ops" || path == "/ops.html" {
                let f = self.static_dir.join("ops.html");
                if f.is_file() {
                    ctx.responded = true;
                    return respond_file(session, &f).await;
                }
            }
            // Completeness board (also under /g5/… via nginx strip → /ops_probe_completeness.html)
            if path == "/ops_probe_completeness"
                || path == "/ops_probe_completeness.html"
                || path == "/ops/probe_completeness"
            {
                let f = self.static_dir.join("ops_probe_completeness.html");
                if f.is_file() {
                    ctx.responded = true;
                    return respond_file(session, &f).await;
                }
            }
        }

        // API dispatch
        let body_needed = matches!(method.as_str(), "POST" | "PUT" | "PATCH");
        let body = if body_needed {
            read_body(session, 8 * 1024 * 1024).await?
        } else {
            Vec::new()
        };

        // Admin console / ACME (before role-gated business APIs).
        //
        // P0 fix (2026-09-08, 178 prod wedge): every sync handler below does
        // blocking work — PG pool round-trips inside `Store::run()`, admin-DB
        // queries, file IO. The Pingora proxy-service runtime has ONE worker
        // thread by default (`ServerConf.threads: 1`); blocking it inside
        // `request_filter` starves accepts and every other request on that
        // listener. Under the 178 crawler flood this surfaced as Recv-Q
        // pileup, nginx 60s timeouts → CLOSE_WAIT/FD accumulation and a full
        // wedge of the probe plane (healthz included) while the gateway plane
        // on its own runtime stayed fast. All sync handler work now runs on
        // the service runtime's blocking pool via `spawn_blocking`; the
        // worker stays free for accept/read/write.
        let admin_disp = {
            let st = self.state.clone();
            let (m, p, h, q) = (method.clone(), path.clone(), headers.clone(), query.clone());
            let b = body.clone();
            match tokio::task::spawn_blocking(move || {
                crate::admin::try_dispatch(&st, &m, &p, &h, &q, &b)
            })
            .await
            {
                Ok(d) => d,
                Err(e) => {
                    log::error!("admin dispatch task join failed: {e}");
                    Some(crate::admin::AdminDispatch::Json(
                        500,
                        json!({"ok": false, "error": "admin_dispatch_task_failed"}),
                    ))
                }
            }
        };
        if let Some(disp) = admin_disp {
            ctx.responded = true;
            // iss/audit LOG-02: admin-plane non-2xx join the same aggregate.
            let admin_status = match &disp {
                crate::admin::AdminDispatch::Json(s, _) => *s,
                crate::admin::AdminDispatch::Bytes(s, ..) => *s,
                crate::admin::AdminDispatch::File(_) => 200,
            };
            note_dispatch_status(&method, &path, admin_status);
            return respond_admin(session, disp).await;
        }

        let result = {
            let st = self.state.clone();
            let (m, p, h, q) = (method.clone(), path.clone(), headers.clone(), query.clone());
            let b = body.clone();
            match tokio::task::spawn_blocking(move || dispatch(&st, &m, &p, &h, &q, &b)).await {
                Ok(d) => d,
                Err(e) => {
                    log::error!("dispatch task join failed: {e}");
                    Dispatch::Json(500, json!({"ok": false, "error": "dispatch_task_failed"}))
                }
            }
        };
        ctx.responded = true;
        // iss/audit LOG-02: aggregated journal visibility for data-plane
        // non-2xx (was: zero access logging at the Pingora entry).
        let disp_status = match &result {
            Dispatch::Json(s, _) => *s,
            Dispatch::Bytes(s, ..) => *s,
        };
        note_dispatch_status(&method, &path, disp_status);
        match result {
            Dispatch::Json(status, v) => respond_json(session, status, &v).await,
            Dispatch::Bytes(status, ct, bytes) => respond_bytes(session, status, ct, bytes).await,
        }
    }

    async fn upstream_peer(
        &self,
        _session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>> {
        if let Some(LbTarget::Forward { addr, tls, sni }) = ctx.lb_target.clone() {
            // LB proxy mode: forward to the chosen node, original request
            // stream untouched (pure passthrough, docs/08).
            let peer = HttpPeer::new(addr.as_str(), tls, sni.clone());
            return Ok(Box::new(peer));
        }
        if ctx.forward {
            if let Some(ref up) = self.upstream {
                let peer = HttpPeer::new(up.addr.as_str(), up.tls, up.sni.clone());
                return Ok(Box::new(peer));
            }
        }
        // Origin mode / local path already responded
        Err(Error::new_str("gr origin mode: no upstream"))
    }

    async fn logging(
        &self,
        session: &mut Session,
        _e: Option<&Error>,
        ctx: &mut Self::CTX,
    ) {
        if !ctx.responded {
            let _ = respond_json(
                session,
                500,
                &json!({"ok": false, "error": "unhandled"}),
            )
            .await;
        }
    }
}

enum Dispatch {
    Json(u16, serde_json::Value),
    Bytes(u16, &'static str, Vec<u8>),
}

async fn respond_admin(
    session: &mut Session,
    disp: crate::admin::AdminDispatch,
) -> Result<bool> {
    match disp {
        crate::admin::AdminDispatch::Json(status, v) => respond_json(session, status, &v).await,
        crate::admin::AdminDispatch::Bytes(status, ct, bytes) => {
            respond_bytes(session, status, ct, bytes).await
        }
        crate::admin::AdminDispatch::File(path) => respond_file(session, &path).await,
    }
}

fn map_err(e: ApiError) -> Dispatch {
    Dispatch::Json(e.status(), e.to_json())
}

/// iss/audit LOG-02: journal visibility for non-2xx dispatch responses.
/// The Pingora data plane previously had zero access logging — 4xx/5xx were
/// only visible as per-request ops DB events on the ingest arms. A per-request
/// INFO line would flood the journal during crawler storms, so aggregate
/// instead: one WARN per 30s window with count + sample (method/path/status),
/// lazily flushed on the next non-2xx after the window closes.
fn note_dispatch_status(method: &str, path: &str, status: u16) {
    if (200..400).contains(&status) {
        return;
    }
    use std::sync::Mutex;
    static WINDOW: std::sync::OnceLock<Mutex<(i64, u64, String, u16)>> =
        std::sync::OnceLock::new();
    let now = gr_probe_store::hot_now_ms();
    let Some(g) = WINDOW.get_or_init(|| Mutex::new((0, 0, String::new(), 0))).lock().ok() else {
        return;
    };
    let mut w = g;
    if now.saturating_sub(w.0) >= 30_000 {
        if w.1 > 0 {
            log::warn!(
                "http_4xx_5xx_30s n={} sample=\"{} -> {}\" (aggregated window; per-request access logging stays off by design)",
                w.1,
                w.2,
                w.3
            );
        }
        *w = (now, 1, format!("{method} {path}"), status);
    } else {
        w.1 += 1;
        w.3 = status; // most recent status wins the histogram slot
    }
}

/// Record ingest 4xx/5xx as ops events with a reason class (P0-2, iss/grok4.6/05).
/// The `/v1/ingest/sealed` arm previously had no event emission, so the browser
/// ingest 401 outage (sdk_key_required) ran unnoticed; both arms now feed this.
/// Client-disconnect / soft-close noise is filtered identically for both paths.
fn ingest_err_event(st: &AppState, path: &str, e: &ApiError) {
    let code = e.0;
    let msg = e.1.clone();
    let noise = code == 410
        || code == 200 && msg.contains("SOFT_CYCLE_CLOSED")
        || msg.contains("session_expired")
        || msg.contains("session_inactive")
        || msg.contains("Broken pipe")
        || msg.contains("connection reset");
    if code < 400 || noise {
        return;
    }
    let short = msg.chars().take(400).collect::<String>();
    let msg_class = if short.contains("sdk_key_required") || short.contains("sdk_key_invalid") {
        "sdk_key"
    } else if short.contains("invalid signature")
        || short.contains("seal_grant_expired")
        || short.contains("seal_policy")
        || short.contains("unseal")
    {
        "seal"
    } else if short.contains("postgres") || short.contains("pg ") || short.contains("sqlite") {
        "store"
    } else if short.contains("invalid") || short.contains("batch") {
        "validation"
    } else {
        "other"
    };
    let _ = st.store.insert_ops_server_event(json!({
        "code": if code >= 500 { "ingest_http_5xx" } else { "ingest_http_4xx" },
        "severity": if code >= 500 { "error" } else { "warn" },
        "stage": "ingest",
        "detail_json": {
            "http": code,
            "path": path,
            "msg": short,
            "msg_class": msg_class,
        },
    }));
}

fn rate_dispatch(
    st: &AppState,
    headers: &std::collections::HashMap<String, String>,
    route: &str,
) -> Option<Dispatch> {
    let site = headers
        .get("host")
        .or_else(|| headers.get("Host"))
        .map(|s| s.split(':').next().unwrap_or(s).to_ascii_lowercase())
        .unwrap_or_else(|| "_".into());
    match crate::rate_limit::check(st, &site, route) {
        Ok(()) => None,
        Err(e) => Some(Dispatch::Json(
            429,
            json!({
                "ok": false,
                "error": {
                    "code": "rate_limited",
                    "message": e,
                    "retryable": true
                }
            }),
        )),
    }
}

fn dispatch(
    st: &AppState,
    method: &str,
    path: &str,
    headers: &std::collections::HashMap<String, String>,
    query: &std::collections::HashMap<String, String>,
    body: &[u8],
) -> Dispatch {
    // Drain gate: while the ops flag is set, refuse new probe traffic (including
    // gateway early) so the LB can finish draining before the process exits.
    // Health/ops/webhook paths stay reachable; this runs before any data handler.
    if st.draining.load(std::sync::atomic::Ordering::Relaxed)
        && !path.starts_with("/v1/ops/")
        && !path.starts_with("/v1/webhook/")
        && !path.starts_with("/livez")
        && !path.starts_with("/readyz")
        && !path.starts_with("/v1/health")
    {
        return Dispatch::Json(
            503,
            json!({"ok": false, "error": {"code": "draining", "retryable": true}}),
        );
    }
    // Gateway early / B8 beacon:
    // - Dedicated gateway role (TLS JA4 / edge) is preferred.
    // - Ingest also accepts these paths: FE dual-posts to apiBase as CDN fallback
    //   (see fe fireEarlyB8). Multi-worker formal/prod with public ingest only was
    //   returning 404 on /v1/gateway/early + /s0 → b8_first_miss forever.
    if path == "/v1/gateway/early" || path == "/s0" {
        if st.role.serves_gateway() || st.role.serves_ingest() {
            // TLS JA4 is injected in request_filter for ALL methods (POST + GET /s0|/early)
            // GET beacon (img/s0) must land B8 — accept both /s0 and /v1/gateway/early.
            if method == "GET" {
                return match handlers::gateway_early_get_compat(st, headers, query) {
                    Ok(v) => Dispatch::Json(200, v),
                    Err(e) => map_err(e),
                };
            }
            if method == "POST" {
                // Empty-body sendBeacon(/s0?session_id=…) must still land B8 — merge query.
                let mut parsed: handlers::GatewayEarlyBody = serde_json::from_slice(body)
                    .unwrap_or(handlers::GatewayEarlyBody {
                        session_id: None,
                        visitor_terminal_id: None,
                        inject_path: Some("gateway".into()),
                        fields: json!({}),
                        analyze: false,
                        site_id: None,
                        session_ticket: None,
                    });
                handlers::gateway_early_fill_from_query(&mut parsed, headers, query);
                return match handlers::gateway_early(st, headers, parsed) {
                    Ok(v) => Dispatch::Json(200, v),
                    Err(e) => map_err(e),
                };
            }
        }
    }

    if !st.role.serves_ingest() {
        return Dispatch::Json(
            404,
            json!({"ok": false, "error": "role_not_served"}),
        );
    }

    // P0: gate sensitive ops + webhook control plane (FE client_event stays open).
    let ops_gated = (path.starts_with("/v1/ops/") && path != "/v1/ops/client_event")
        || path.starts_with("/v1/webhook/");
    if ops_gated {
        if let Err(e) = handlers::require_ops_auth(st, headers) {
            return map_err(e);
        }
    }

    match (method, path) {
        ("POST", "/v1/session/open") => {
            if let Some(d) = rate_dispatch(st, headers, "open") {
                return d;
            }
            let b: handlers::OpenBody = match serde_json::from_slice(body) {
                Ok(b) => b,
                Err(e) => {
                    return Dispatch::Json(400, json!({"ok": false, "error": format!("bad_json: {e}")}))
                }
            };
            match crate::idempotency::wrap_json(st, headers, "open", body, || {
                handlers::open_session(st, b, headers)
            }) {
                Ok((code, v)) => Dispatch::Json(code, v),
                Err(e) => map_err(e),
            }
        }
        ("POST", "/v1/biz/visit") => {
            let b: handlers::BizVisitBody = match serde_json::from_slice(body) {
                Ok(b) => b,
                Err(e) => {
                    return Dispatch::Json(400, json!({"ok": false, "error": format!("bad_json: {e}")}))
                }
            };
            match handlers::biz_visit(st, headers, b) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        ("POST", "/v1/assoc/observe") => {
            let b: handlers::AssocObserveBody = match serde_json::from_slice(body) {
                Ok(b) => b,
                Err(e) => {
                    return Dispatch::Json(
                        400,
                        json!({"ok": false, "error": format!("bad_json: {e}")}),
                    )
                }
            };
            match handlers::assoc_observe(st, headers, b) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        ("POST", "/v1/assoc/assess") => {
            let b: handlers::AssocAssessBody = match serde_json::from_slice(body) {
                Ok(b) => b,
                Err(e) => {
                    return Dispatch::Json(
                        400,
                        json!({"ok": false, "error": format!("bad_json: {e}")}),
                    )
                }
            };
            match handlers::assoc_assess(st, headers, b) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        ("POST", "/v1/assoc/label") => {
            let b: handlers::AssocLabelBody = match serde_json::from_slice(body) {
                Ok(b) => b,
                Err(e) => {
                    return Dispatch::Json(
                        400,
                        json!({"ok": false, "error": format!("bad_json: {e}")}),
                    )
                }
            };
            match handlers::assoc_label(st, headers, b) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        // iss/48–49 webhook outbox + analyze DLQ
        ("POST", "/v1/webhook/enqueue") => {
            let v: Value = match serde_json::from_slice(body) {
                Ok(v) => v,
                Err(e) => {
                    return Dispatch::Json(400, json!({"ok": false, "error": format!("bad_json: {e}")}))
                }
            };
            let tenant = v
                .get("tenant")
                .or_else(|| v.get("site_id"))
                .and_then(|x| x.as_str())
                .unwrap_or("default");
            let event = v.get("event").and_then(|x| x.as_str()).unwrap_or("analysis.complete");
            let url = v.get("url").and_then(|x| x.as_str()).unwrap_or("");
            if url.is_empty() {
                return Dispatch::Json(400, json!({"ok": false, "error": "url required"}));
            }
            let secret = v.get("secret").and_then(|x| x.as_str());
            let payload = v.get("payload").cloned().unwrap_or(json!({}));
            Dispatch::Json(
                200,
                crate::webhook_outbox::enqueue_webhook(tenant, event, url, payload, secret),
            )
        }
        ("GET", "/v1/webhook/outbox") => {
            Dispatch::Json(200, crate::webhook_outbox::outbox_status())
        }
        ("POST", "/v1/webhook/drain") => {
            let v: Value = serde_json::from_slice(body).unwrap_or(json!({}));
            let max_n = v
                .get("max_n")
                .and_then(|x| x.as_u64())
                .unwrap_or(16)
                .clamp(1, 64) as usize;
            let max_attempts = v
                .get("max_attempts")
                .and_then(|x| x.as_u64())
                .unwrap_or(3)
                .clamp(1, 10) as u32;
            Dispatch::Json(
                200,
                crate::webhook_outbox::drain_webhooks(max_n, max_attempts),
            )
        }
        ("GET", "/v1/ops/analyze_dlq") => {
            Dispatch::Json(200, crate::webhook_outbox::analyze_dlq_list(50))
        }
        ("GET", "/v1/ops/config_snapshot") => {
            Dispatch::Json(200, gr_probe_core::current_config_snapshot())
        }
        ("GET", "/v1/ops/nodes") => Dispatch::Json(200, handlers::ops_nodes(st)),
        ("GET", "/v1/ops/metrics") => Dispatch::Json(200, handlers::ops_metrics(st)),
        ("POST", "/v1/ops/replay") => {
            let v: Value = match serde_json::from_slice(body) {
                Ok(v) => v,
                Err(_) => json!({}),
            };
            Dispatch::Json(200, handlers::ops_replay(st, v))
        }
        ("POST", "/v1/ops/drain") => {
            let v: Value = match serde_json::from_slice(body) {
                Ok(v) => v,
                Err(_) => json!({}),
            };
            let drain = v.get("drain").and_then(|x| x.as_bool()).unwrap_or(false);
            Dispatch::Json(200, handlers::ops_drain_set(st, drain))
        }
        ("POST", "/v1/ops/config_push") => {
            let v: Value = match serde_json::from_slice(body) {
                Ok(v) => v,
                Err(e) => {
                    return Dispatch::Json(
                        400,
                        json!({"ok": false, "error": format!("bad_json: {e}")}),
                    )
                }
            };
            let path = v
                .get("path")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string());
            let overlay = v
                .get("overlay")
                .or_else(|| v.get("snapshot"))
                .cloned()
                .unwrap_or(v);
            Dispatch::Json(
                200,
                gr_probe_core::push_config_overlay(&overlay, path.as_deref()),
            )
        }
        // iss/54 F3: channel drift report / multi-family job
        ("POST", "/v1/ops/channel_drift") => {
            let v: Value = match serde_json::from_slice(body) {
                Ok(v) => v,
                Err(e) => {
                    return Dispatch::Json(
                        400,
                        json!({"ok": false, "error": format!("bad_json: {e}")}),
                    )
                }
            };
            if let Some(dir) = v.get("dir").and_then(|x| x.as_str()) {
                Dispatch::Json(
                    200,
                    gr_probe_core::channel_drift_job_from_dir(std::path::Path::new(dir)),
                )
            } else if v.get("families").is_some() {
                Dispatch::Json(200, gr_probe_core::channel_drift_job(&v))
            } else {
                Dispatch::Json(200, gr_probe_core::channel_drift_report(&v))
            }
        }
        // iss/54 F5: dump / fit / write / adopt fusion weights
        ("GET", "/v1/ops/fusion_weights") => {
            Dispatch::Json(200, gr_probe_core::fusion_weights_status())
        }
        ("POST", "/v1/ops/fusion_weights") => {
            let v: Value = match serde_json::from_slice(body) {
                Ok(v) => v,
                Err(e) => {
                    return Dispatch::Json(
                        400,
                        json!({"ok": false, "error": format!("bad_json: {e}")}),
                    )
                }
            };
            let mode = v
                .get("mode")
                .and_then(|x| x.as_str())
                .unwrap_or("fit");
            let pairs = v
                .get("pairs")
                .and_then(|x| x.as_array())
                .cloned()
                .or_else(|| v.as_array().cloned())
                .unwrap_or_default();
            let out = v.get("path").and_then(|x| x.as_str()).map(std::path::Path::new);
            Dispatch::Json(200, gr_probe_core::fusion_weights_ops(&pairs, mode, out))
        }
        // iss/54 F6: dump / calibrate / write / adopt engine norm
        ("GET", "/v1/ops/engine_norm") => {
            Dispatch::Json(200, gr_probe_core::engine_norm_status())
        }
        ("POST", "/v1/ops/engine_norm") => {
            let v: Value = match serde_json::from_slice(body) {
                Ok(v) => v,
                Err(e) => {
                    return Dispatch::Json(
                        400,
                        json!({"ok": false, "error": format!("bad_json: {e}")}),
                    )
                }
            };
            let mode = v
                .get("mode")
                .and_then(|x| x.as_str())
                .unwrap_or("calibrate");
            let batch = v
                .get("observations")
                .or_else(|| v.get("batch"))
                .cloned()
                .unwrap_or(v.clone());
            let out = v.get("path").and_then(|x| x.as_str()).map(std::path::Path::new);
            Dispatch::Json(200, gr_probe_core::engine_norm_ops(&batch, mode, out))
        }
        ("GET", "/v1/ops/association_backend") => {
            match st.admin.as_ref() {
                Some(a) => Dispatch::Json(200, a.association.backend_status()),
                None => Dispatch::Json(
                    503,
                    json!({
                        "ok": false,
                        "error": "admin_disabled",
                        "multi_region_ha": false,
                        "note": "Association store requires admin hub",
                    }),
                ),
            }
        }
        ("POST", "/v1/ops/ece_from_labels") => {
            let v: Value = serde_json::from_slice(body).unwrap_or(json!({}));
            let tenant = v
                .get("tenant")
                .or_else(|| v.get("site_id"))
                .and_then(|x| x.as_str())
                .unwrap_or("default");
            let limit = v.get("limit").and_then(|x| x.as_i64()).unwrap_or(500).clamp(1, 5000);
            match st.admin.as_ref() {
                Some(a) => match a.association.list_labels(tenant, limit) {
                    Ok(labels) => {
                        let ece = gr_probe_core::ece_from_customer_labels(&labels);
                        Dispatch::Json(
                            200,
                            json!({"ok": true, "tenant": tenant, "ece": ece, "n_labels": labels.len()}),
                        )
                    }
                    Err(e) => Dispatch::Json(500, json!({"ok": false, "error": e})),
                },
                None => Dispatch::Json(503, json!({"ok": false, "error": "admin_disabled"})),
            }
        }
        ("POST", "/v1/assoc/behavior_profile") => {
            let v: Value = match serde_json::from_slice(body) {
                Ok(v) => v,
                Err(e) => {
                    return Dispatch::Json(
                        400,
                        json!({"ok": false, "error": format!("bad_json: {e}")}),
                    )
                }
            };
            match handlers::assoc_behavior_profile_put(st, headers, v) {
                Ok(r) => Dispatch::Json(200, r),
                Err(e) => map_err(e),
            }
        }
        ("POST", "/v1/assoc/behavior_self_sim") => {
            let v: Value = match serde_json::from_slice(body) {
                Ok(v) => v,
                Err(e) => {
                    return Dispatch::Json(
                        400,
                        json!({"ok": false, "error": format!("bad_json: {e}")}),
                    )
                }
            };
            match handlers::assoc_behavior_self_sim(st, headers, v) {
                Ok(r) => Dispatch::Json(200, r),
                Err(e) => map_err(e),
            }
        }
        ("POST", "/v1/ops/curve_entropy_census") => {
            let v: Value = match serde_json::from_slice(body) {
                Ok(v) => v,
                Err(e) => {
                    return Dispatch::Json(400, json!({"ok": false, "error": format!("bad_json: {e}")}))
                }
            };
            let batch = v
                .get("observations")
                .or_else(|| v.get("batch"))
                .and_then(|x| x.as_array())
                .cloned()
                .unwrap_or_default();
            Dispatch::Json(
                200,
                gr_probe_core::census_from_curve_descriptor_batch(&batch),
            )
        }
        ("POST", "/v1/ops/cluster_lsh") => {
            let v: Value = match serde_json::from_slice(body) {
                Ok(v) => v,
                Err(e) => {
                    return Dispatch::Json(400, json!({"ok": false, "error": format!("bad_json: {e}")}))
                }
            };
            let items = v
                .get("items")
                .and_then(|x| x.as_array())
                .cloned()
                .unwrap_or_default();
            let max_d = v.get("max_distance").and_then(|x| x.as_u64()).unwrap_or(4) as u32;
            Dispatch::Json(200, gr_probe_core::cluster_by_curve_lsh(&items, max_d))
        }
        ("POST", "/v1/ingest") => {
            if let Some(d) = rate_dispatch(st, headers, "ingest") {
                return d;
            }
            match crate::idempotency::wrap_json(st, headers, "ingest", body, || {
                handlers::ingest(st, headers, body)
            }) {
            Ok((code, v)) => Dispatch::Json(code, v),
            Err(e) => {
                // Mirror real handler failures (not client disconnect noise).
                ingest_err_event(st, "/v1/ingest", &e);
                map_err(e)
            }
            }
        },
        ("POST", "/v1/ops/client_event") => {
            let b: Value = match serde_json::from_slice(body) {
                Ok(b) => b,
                Err(e) => {
                    return Dispatch::Json(
                        400,
                        json!({"ok": false, "error": format!("bad_json: {e}")}),
                    )
                }
            };
            match handlers::ops_client_event(st, headers, b) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        },
        ("GET", "/v1/ops/events") => match handlers::ops_events_list(st, query) {
            Ok(v) => Dispatch::Json(200, v),
            Err(e) => map_err(e),
        },
        ("GET", "/v1/ops/events/export") => match handlers::ops_events_export(st, query) {
            Ok(v) => Dispatch::Json(200, v),
            Err(e) => map_err(e),
        },
        ("GET", "/v1/ops/b10_health") => match handlers::ops_b10_health(st, query) {
            Ok(v) => Dispatch::Json(200, v),
            Err(e) => map_err(e),
        },
        ("POST", "/v1/ingest/sealed") => {
            if let Some(d) = rate_dispatch(st, headers, "ingest") {
                return d;
            }
            match crate::idempotency::wrap_json(st, headers, "ingest", body, || {
                handlers::ingest_sealed(st, headers, body)
            }) {
                Ok((code, v)) => Dispatch::Json(code, v),
                Err(e) => {
                    // P0-2: sealed arm previously emitted no ingest_http_4xx/5xx
                    // events (observability gap during the browser 401 outage).
                    ingest_err_event(st, "/v1/ingest/sealed", &e);
                    map_err(e)
                }
            }
        }
        ("POST", p) if p.starts_with("/v1/session/") && p.ends_with("/analyze") => {
            if let Some(d) = rate_dispatch(st, headers, "analyze") {
                return d;
            }
            let sid = extract_between(p, "/v1/session/", "/analyze").unwrap_or("");
            let b: handlers::AnalyzeBody = serde_json::from_slice(body).unwrap_or(handlers::AnalyzeBody {
                soft_v2_ready: None, // ignored; server AppState.soft_v2_ready only
                peer_session_ids: vec![],
                pagehide_flush: None,
                verbose: None,
            });
            match crate::idempotency::wrap_json(st, headers, "analyze", body, || {
                handlers::analyze_session(st, sid, b, headers)
            }) {
                Ok((code, v)) => Dispatch::Json(code, v),
                Err(e) => map_err(e),
            }
        }
        ("POST", p) if p.starts_with("/v1/session/") && p.ends_with("/complete") => {
            if let Some(d) = rate_dispatch(st, headers, "complete") {
                return d;
            }
            let sid = extract_between(p, "/v1/session/", "/complete").unwrap_or("");
            match crate::idempotency::wrap_json(st, headers, "complete", body, || {
                handlers::complete_session(st, sid, headers)
            }) {
                Ok((code, v)) => Dispatch::Json(code, v),
                Err(e) => map_err(e),
            }
        }
        // Multi-party state: FE posts local view → BE authoritative snapshot + corrections.
        ("POST", p) if p.starts_with("/v1/session/") && p.ends_with("/probe_status") => {
            let sid = extract_between(p, "/v1/session/", "/probe_status").unwrap_or("");
            let b: serde_json::Value = serde_json::from_slice(body).unwrap_or(serde_json::json!({}));
            match handlers::probe_status_reconcile(st, sid, &b, headers) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        ("GET", p) if p.starts_with("/v1/session/") && p.ends_with("/probe_status") => {
            let sid = extract_between(p, "/v1/session/", "/probe_status").unwrap_or("");
            match handlers::probe_status_reconcile(st, sid, &serde_json::json!({}), headers) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        ("GET", p) if p.starts_with("/v1/session/") && p.ends_with("/result") => {
            if let Some(d) = rate_dispatch(st, headers, "result") {
                return d;
            }
            let sid = extract_between(p, "/v1/session/", "/result").unwrap_or("");
            match handlers::get_result(st, sid, headers, query) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        ("GET", p) if p.starts_with("/v1/session/") && p.ends_with("/evidence") => {
            let sid = extract_between(p, "/v1/session/", "/evidence").unwrap_or("");
            match handlers::get_evidence(st, sid, headers) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        ("GET", p) if p.starts_with("/v1/session/") && p.ends_with("/analyses") => {
            let sid = extract_between(p, "/v1/session/", "/analyses").unwrap_or("");
            match handlers::list_analyses(st, sid, headers, query) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        // POST alias: same payload as GET — preferred under some CF/WAF policies after challenge
        ("POST", p) if p.starts_with("/v1/session/") && p.ends_with("/analyses") => {
            let sid = extract_between(p, "/v1/session/", "/analyses").unwrap_or("");
            match handlers::list_analyses(st, sid, headers, query) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        ("GET", p) if p.starts_with("/v1/session/") && p.ends_with("/batches") => {
            let sid = extract_between(p, "/v1/session/", "/batches").unwrap_or("");
            match handlers::list_batches(st, sid, headers) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        ("GET", p) if p.starts_with("/v1/session/") && p.ends_with("/window") => {
            let sid = extract_between(p, "/v1/session/", "/window").unwrap_or("");
            match handlers::session_window(st, sid, headers) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        ("GET", p) if p.starts_with("/v1/session/") && p.ends_with("/pages") => {
            let sid = extract_between(p, "/v1/session/", "/pages").unwrap_or("");
            match handlers::list_pages(st, sid, headers) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        ("GET", p) if p.starts_with("/v1/session/") && p.contains("/page/") => {
            // /v1/session/:id/page/:page_id
            let rest = p.strip_prefix("/v1/session/").unwrap_or("");
            let mut parts = rest.splitn(3, '/');
            let sid = parts.next().unwrap_or("");
            let _page = parts.next(); // "page"
            let page_id = parts.next().unwrap_or("");
            match handlers::get_page(st, sid, page_id, headers) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        ("GET", p) if p.starts_with("/v1/ops/session/") => {
            let sid = p.strip_prefix("/v1/ops/session/").unwrap_or("");
            match handlers::ops_session(st, sid) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        // R100 pseudo-static pack templates (like B plan-lazy: only when brain asks)
        ("GET", p) if p.starts_with("/v1/r100/pack/") && (p.ends_with(".js") || p.contains("spotcheck")) => {
            let rest = p.strip_prefix("/v1/r100/pack/").unwrap_or("");
            let pid = rest.trim_end_matches(".js");
            match handlers::r100_pack_js(st, pid) {
                Ok(js) => Dispatch::Bytes(
                    200,
                    "application/javascript; charset=utf-8",
                    js.into_bytes(),
                ),
                Err(e) => Dispatch::Json(e.status(), e.to_json()),
            }
        }
        ("GET", "/v1/r100/random.js") | ("GET", "/v1/r100/random") => {
            match handlers::r100_random_js(st, query) {
                Ok((pid, js)) => {
                    // Pack id is inside registerPack(); FE marks loaded via __GR_R_PACK_LOADED__.
                    let _ = pid;
                    Dispatch::Bytes(
                        200,
                        "application/javascript; charset=utf-8",
                        js.into_bytes(),
                    )
                }
                Err(e) => Dispatch::Json(e.status(), e.to_json()),
            }
        }
        ("GET", "/v1/ops/r100") => Dispatch::Json(200, handlers::ops_r100_status(st)),
        ("POST", "/v1/ops/r100/seed_redis") => {
            Dispatch::Json(200, handlers::ops_r100_seed_redis(st))
        }
        ("GET", "/v1/ops/overview") => Dispatch::Json(200, handlers::ops_overview(st)),
        ("GET", "/v1/ops/identity_governance") => {
            Dispatch::Json(200, handlers::ops_identity_governance(st))
        }
        ("GET", "/v1/ops/cross_query") => {
            match handlers::ops_cross_query(st, query) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        ("GET", "/v1/ops/vt_best_silicon") => {
            match handlers::ops_vt_best_silicon(st, query) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => Dispatch::Json(e.0, json!({"ok": false, "error": e.1})),
            }
        }
        ("GET", "/v1/ops/algo_groups") => match handlers::ops_algo_groups_registry(st) {
            Ok(v) => Dispatch::Json(200, v),
            Err(e) => Dispatch::Json(e.0, json!({"ok": false, "error": e.1})),
        },
        ("GET", "/v1/ops/probe_completeness") => {
            match handlers::ops_probe_completeness(st, query) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => Dispatch::Json(e.0, json!({"ok": false, "error": e.1})),
            }
        }
        ("GET", "/v1/ops/velocity") => match handlers::ops_velocity(st, query) {
            Ok(v) => Dispatch::Json(200, v),
            Err(e) => Dispatch::Json(e.0, json!({"ok": false, "error": e.1})),
        },
        ("GET", "/v1/ops/analysis_latest") => {
            match handlers::ops_analysis_latest(st, query) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        // Panel QA P3 (2026-09-08): admin Integrations "test query" — run the
        // IP-enrichment pipeline for one IP against the saved panel policy (or
        // an incoming ip_enrichment override) without waiting for a result
        // retrieval. Ops-gated like the other /v1/ops endpoints.
        ("POST", "/v1/ops/ip_enrichment/test") => {
            let v: Value = match serde_json::from_slice(body) {
                Ok(v) => v,
                Err(_) => json!({}),
            };
            match handlers::ops_ip_enrichment_test(st, &v) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        ("GET", "/v1/ops/binder_lookup") => {
            match handlers::ops_binder_lookup(st, query) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        ("GET", p) if p.starts_with("/v1/ops/device/") => {
            let did = p.strip_prefix("/v1/ops/device/").unwrap_or("");
            // Avoid colliding with /v1/ops/device_heat/
            if did.is_empty() || did.contains('/') {
                Dispatch::Json(404, json!({"ok": false, "error": "not found"}))
            } else {
                match handlers::ops_device(st, did, query) {
                    Ok(v) => Dispatch::Json(200, v),
                    Err(e) => map_err(e),
                }
            }
        }
        ("GET", p) if p.starts_with("/v1/ops/hot_probe/") => {
            let vt = p.strip_prefix("/v1/ops/hot_probe/").unwrap_or("");
            match handlers::ops_hot_probe(st, vt) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        ("POST", "/v1/ops/demote_idle") | ("GET", "/v1/ops/demote_idle") => {
            match handlers::ops_demote_idle(st, query) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        ("POST", "/v1/ops/harvest_arms") | ("GET", "/v1/ops/harvest_arms") => {
            match handlers::ops_harvest_arms(st, query) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        ("POST", "/v1/ops/purge_expired_cold") | ("GET", "/v1/ops/purge_expired_cold") => {
            match handlers::ops_purge_expired_cold(st, query) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        ("POST", "/v1/ops/retention_purge") | ("GET", "/v1/ops/retention_purge") => {
            match handlers::ops_retention_purge(st, query) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        // iss/opus5 05-S-5: DSAR erase (POST body) / export (GET query).
        ("POST", "/v1/ops/dsar/erase") => match handlers::ops_dsar_erase(st, body) {
            Ok(v) => Dispatch::Json(200, v),
            Err(e) => map_err(e),
        },
        ("GET", "/v1/ops/dsar/export") => match handlers::ops_dsar_export(st, query) {
            Ok(v) => Dispatch::Json(200, v),
            Err(e) => map_err(e),
        },
        ("GET", "/v1/ops/outcome_distribution") => {
            match handlers::ops_outcome_distribution(st, query) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        ("GET", "/v1/ops/probe_cold") => match handlers::ops_probe_cold(st, query) {
            Ok(v) => Dispatch::Json(200, v),
            Err(e) => map_err(e),
        },
        ("GET", "/v1/ops/probe_volume") => match handlers::ops_probe_volume(st, query) {
            Ok(v) => Dispatch::Json(200, v),
            Err(e) => map_err(e),
        },
        ("GET", "/v1/ops/timeouts") => match handlers::ops_timeouts(st) {
            Ok(v) => Dispatch::Json(200, v),
            Err(e) => map_err(e),
        },
        ("POST", "/v1/ops/promote_cold") | ("GET", "/v1/ops/promote_cold") => {
            match handlers::ops_promote_cold(st, query) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        },
        ("GET", "/v1/ops/side_channels") => Dispatch::Json(
            200,
            json!({
                "ok": true,
                "side": crate::listen::recent_side_summary(),
                "side_lab": crate::listen::side_lab_enabled(),
            }),
        ),
        ("POST", "/v1/ops/side_lab_inject") => {
            if !crate::listen::side_lab_enabled() {
                return Dispatch::Json(
                    403,
                    json!({"ok": false, "error": "set GR_SIDE_LAB=1 to enable lab inject"}),
                );
            }
            let v: Value = serde_json::from_slice(body).unwrap_or(json!({}));
            let kind = v
                .get("kind")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            match crate::listen::lab_inject(&kind, &v) {
                Ok(out) => Dispatch::Json(200, out),
                Err(e) => Dispatch::Json(400, json!({"ok": false, "error": e})),
            }
        },
        ("GET", "/v1/ops/device_index") => match handlers::ops_device_index(st, query) {
            Ok(v) => Dispatch::Json(200, v),
            Err(e) => Dispatch::Json(e.status(), e.to_json()),
        },
        ("GET", "/v1/ops/identity_sla") => match handlers::ops_identity_sla(st) {
            Ok(v) => Dispatch::Json(200, v),
            Err(e) => map_err(e),
        },
        ("POST", "/v1/ops/identity_sla") => {
            let b: handlers::SlaPostBody =
                serde_json::from_slice(body).unwrap_or(handlers::SlaPostBody { cells: None });
            Dispatch::Json(200, handlers::ops_identity_sla_post(st, b))
        }
        ("GET", p) if p.starts_with("/v1/ops/device_heat/") => {
            let did = p.strip_prefix("/v1/ops/device_heat/").unwrap_or("");
            match handlers::ops_device_heat(st, did) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        ("POST", "/v1/ops/device_heat") => {
            let b: handlers::HeatRecordBody = match serde_json::from_slice(body) {
                Ok(b) => b,
                Err(e) => {
                    return Dispatch::Json(400, json!({"ok": false, "error": e.to_string()}))
                }
            };
            match handlers::ops_device_heat_record(st, b) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        ("GET", "/v1/ops/soft_edges") => match handlers::ops_soft_edges_list(st) {
            Ok(v) => Dispatch::Json(200, v),
            Err(e) => map_err(e),
        },
        ("POST", "/v1/ops/soft_edges") => {
            let b: handlers::SoftEdgeBody = match serde_json::from_slice(body) {
                Ok(b) => b,
                Err(e) => {
                    return Dispatch::Json(400, json!({"ok": false, "error": e.to_string()}))
                }
            };
            match handlers::ops_soft_edge_put(st, b) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        ("POST", "/v1/ops/conf_calibrate") => {
            let b: handlers::ConfCalBody = serde_json::from_slice(body).unwrap_or(handlers::ConfCalBody {
                pairs: None,
                matrix_cells: None,
            });
            Dispatch::Json(200, handlers::ops_conf_calibrate(b))
        }
        ("GET", "/v1/ops/self_capability") => {
            Dispatch::Json(200, handlers::ops_self_capability())
        }
        ("POST", "/v1/ops/unknown_hub_aggregate") => {
            let b: handlers::UnknownHubBody =
                serde_json::from_slice(body).unwrap_or(handlers::UnknownHubBody {
                    sessions: None,
                    from_store: Some(true),
                    limit: Some(200),
                });
            Dispatch::Json(200, handlers::ops_unknown_hub_aggregate(st, b))
        }
        ("POST", "/v1/ops/probe_coverage_gaps") => {
            let b: handlers::UnknownHubBody =
                serde_json::from_slice(body).unwrap_or(handlers::UnknownHubBody {
                    sessions: None,
                    from_store: Some(true),
                    limit: Some(300),
                });
            Dispatch::Json(200, handlers::ops_probe_coverage_gaps(st, b))
        }
        ("POST", "/v1/ops/precision_matrix") => {
            let b: handlers::PrecisionMatrixBody =
                serde_json::from_slice(body).unwrap_or(handlers::PrecisionMatrixBody {
                    session_ids: None,
                    limit: Some(40),
                    peer_engine: Some("blink".into()),
                });
            Dispatch::Json(200, handlers::ops_precision_matrix(st, b))
        }
        ("GET", "/v1/ops/probe_health") => {
            Dispatch::Json(200, handlers::ops_probe_health(st, query))
        }
        ("POST", "/v1/ops/collision_kpi") => {
            let b: handlers::CollisionKpiBody =
                serde_json::from_slice(body).unwrap_or(handlers::CollisionKpiBody {
                    observations: None,
                });
            Dispatch::Json(200, handlers::ops_collision_kpi(b))
        }
        ("POST", "/v1/ops/challenge_rotate") => {
            let b: handlers::ChallengeRotateBody =
                serde_json::from_slice(body).unwrap_or(handlers::ChallengeRotateBody {
                    tenant: None,
                    epoch: None,
                    base_secret: None,
                });
            Dispatch::Json(200, handlers::ops_challenge_rotate(b))
        }
        ("POST", "/v1/evaluate") => {
            let b: handlers::EvaluateBody = match serde_json::from_slice(body) {
                Ok(b) => b,
                Err(e) => {
                    return Dispatch::Json(400, json!({"ok": false, "error": e.to_string()}))
                }
            };
            match handlers::evaluate_pure(b) {
                Ok(v) => Dispatch::Json(200, v),
                Err(e) => map_err(e),
            }
        }
        ("GET", "/v1/pixel.gif") => {
            let (stt, ct, bytes) = handlers::pixel_hit(st, headers, query);
            Dispatch::Bytes(stt, ct, bytes)
        }
        _ => Dispatch::Json(404, json!({"ok": false, "error": "not_found", "path": path})),
    }
}

fn extract_between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let rest = s.strip_prefix(start)?;
    let i = rest.find(end)?;
    Some(&rest[..i])
}

/// Read `name=value` from a request cookie header (case-insensitive cookie name).
fn cookie_value(
    headers: &std::collections::HashMap<String, String>,
    header_name: &str,
    cookie_name: &str,
) -> Option<String> {
    let raw = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(header_name))
        .map(|(_, v)| v.as_str())?;
    for part in raw.split(';') {
        let part = part.trim();
        if let Some((k, v)) = part.split_once('=') {
            if k.trim().eq_ignore_ascii_case(cookie_name) {
                let v = v.trim().trim_matches('"');
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// Visitor's real IP for LB decisions (internal-IP CIDR + ip_hash). Read-only
/// pass-through honor: CDN headers first, else the TCP peer. Never mutates the
/// request stream (pure passthrough requirement, docs/08).
fn lb_visitor_ip(
    headers: &std::collections::HashMap<String, String>,
    session: &Session,
) -> Option<IpAddr> {
    for key in ["cf-connecting-ip", "true-client-ip", "x-real-ip", "x-forwarded-for"] {
        let val = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str());
        if let Some(v) = val {
            let first = v.split(',').next().unwrap_or(v).trim();
            if !first.is_empty() && !first.eq_ignore_ascii_case("unknown") {
                if let Ok(ip) = first.parse::<IpAddr>() {
                    return Some(ip);
                }
            }
        }
    }
    session.client_addr().and_then(|a| a.as_inet()).map(|a| a.ip())
}

/// Strip optional version + asset_gen segments used only for cache-busting.
///
/// Supported layouts (physical files stay flat under static_dir):
/// - `v/<product_or_fe_version>/rest`
/// - `v/<fe_version>/g/<asset_gen>/rest`  ← FE-only OTA busts gen without runtime bump
fn strip_version_route(rel: &str) -> &str {
    let rel = rel.trim_start_matches('/');
    if let Some(rest) = rel.strip_prefix("v/") {
        if let Some(slash) = rest.find('/') {
            let after_ver = &rest[slash + 1..];
            // Optional second segment: g/<asset_gen>/…
            if let Some(after_g) = after_ver.strip_prefix("g/") {
                if let Some(slash2) = after_g.find('/') {
                    return &after_g[slash2 + 1..];
                }
                // bare g/<gen> — invalid file path
                return rel;
            }
            return after_ver;
        }
        // bare `v/<ver>` with no file — invalid
        return rel;
    }
    rel
}

#[cfg(test)]
mod strip_version_tests {
    use super::strip_version_route;

    #[test]
    fn strips_legacy_v_version() {
        assert_eq!(
            strip_version_route("v/6.0.12/gr.race.min.js"),
            "gr.race.min.js"
        );
        assert_eq!(
            strip_version_route("v/6.0.12/collectors/registry.static.hard.min.js"),
            "collectors/registry.static.hard.min.js"
        );
    }

    #[test]
    fn strips_fe_asset_gen_segment() {
        assert_eq!(
            strip_version_route("v/6.0.14/g/a1b2c3d4e5f6/gr.race.min.js"),
            "gr.race.min.js"
        );
        assert_eq!(
            strip_version_route("v/6.0.14/g/deadbeef0123/pack_loader.min.js"),
            "pack_loader.min.js"
        );
        assert_eq!(
            strip_version_route("v/6.0.14/g/deadbeef0123/collectors/registry.mid.min.js"),
            "collectors/registry.mid.min.js"
        );
    }

    #[test]
    fn leaves_flat_paths() {
        assert_eq!(strip_version_route("gr.race.min.js"), "gr.race.min.js");
        assert_eq!(
            strip_version_route("collectors/x.min.js"),
            "collectors/x.min.js"
        );
    }

    #[test]
    fn strips_version_then_content_hash() {
        use crate::handlers::strip_content_hash_filename;
        let after_ver = strip_version_route(
            "v/6.0.15/g/cef2e944ea2f/gr.race.a1b2c3d4e5f6.min.js",
        );
        assert_eq!(after_ver, "gr.race.a1b2c3d4e5f6.min.js");
        assert_eq!(
            strip_content_hash_filename(after_ver),
            "gr.race.min.js"
        );
        assert_eq!(
            strip_content_hash_filename(
                "collectors/registry.static.lite.deadbeef0123.min.js"
            ),
            "collectors/registry.static.lite.min.js"
        );
    }
}

fn map_boot_alias(name: &str) -> Option<&'static str> {
    match name {
        "mp.boot.min.js" | "mp.boot.js" | "gr.boot.js" => Some("gr.boot.js"),
        "gr.boot.min.js" => Some("gr.boot.min.js"),
        "gr.race.min.js" => Some("gr.race.min.js"),
        "gr.entry.min.js" => Some("gr.entry.min.js"),
        // Eternal pin: URL /dist/gr.js → min body (source stays fe/gr.js for debug)
        "gr.js" | "gr.pin.js" => Some("gr.min.js"),
        "gr.min.js" => Some("gr.min.js"),
        // Seal v2: hashed wire names (gr_seal_v2.<hash>.wasm) strip to the
        // underscore logical → alias to the dot-named physical file.
        "gr_seal_v2.wasm" => Some("gr.seal_v2.wasm"),
        "gr_seal_v2_loader.js" => Some("gr.seal_v2_loader.js"),
        _ => None,
    }
}

fn static_rel(path: &str, static_dir: &std::path::Path) -> Option<String> {
    // Boot aliases FIRST (must not be stripped as generic /dist/* files — P-A1)
    // Also accept versioned + content-hashed routes + opaque /dist/<hash>.ext
    let path_nv = {
        if let Some(r) = path.strip_prefix("/dist/") {
            strip_version_route(r).to_string()
        } else if let Some(r) = path.strip_prefix("/fe/") {
            strip_version_route(r).to_string()
        } else {
            path.to_string()
        }
    };
    // Opaque public basename: pure <12hex>.min.js|wasm|html (no product tokens).
    // Resolved via fe/OPAQUE_MAP.json written by bootstrap / rebuild_opaque_map.
    if let Some(file) = path_nv.rsplit('/').next() {
        let pure_opaque = {
            let base = file
                .strip_suffix(".min.js")
                .or_else(|| file.strip_suffix(".wasm"))
                .or_else(|| file.strip_suffix(".html"))
                .or_else(|| file.strip_suffix(".css"))
                .or_else(|| file.strip_suffix(".js"));
            match base {
                Some(b) => crate::handlers::is_content_hash_token(b),
                None => false,
            }
        };
        // Only treat as fully-opaque when basename has no extra labels (token.ext only).
        let bare_token = pure_opaque
            && !file.contains("gr")
            && !file.contains("registry")
            && !file.contains("pack_loader")
            && !file.contains("nest_frame")
            && file.matches('.').count() <= 2; // a1b2.min.js or a1b2.wasm
        if bare_token {
            // Opaque map values carry the wire logical name (e.g. gr_seal_v2.wasm);
            // alias to the dot-named physical file before resolving on disk.
            let map_logical = |l: String| match map_boot_alias(&l) {
                Some(a) => a.to_string(),
                None => l,
            };
            if let Some(logical) = crate::handlers::opaque_map_lookup(static_dir, file) {
                return Some(map_logical(logical));
            }
            // Map cold: rebuild once and retry
            crate::handlers::rebuild_opaque_map(static_dir);
            if let Some(logical) = crate::handlers::opaque_map_lookup(static_dir, file) {
                return Some(map_logical(logical));
            }
        }
    }
    // Strip Standard-B content hash from basename → logical on-disk name.
    let path_logical = crate::handlers::strip_content_hash_filename(&path_nv);
    // Full-path legacy aliases (pin only fixed names without hash)
    match path {
        "/dist/mp.boot.min.js" | "/dist/mp.boot.js" | "/dist/gr.boot.js" | "/fe/gr.boot.js" => {
            return Some("gr.boot.js".into());
        }
        "/dist/gr.boot.min.js" | "/fe/gr.boot.min.js" => {
            return Some("gr.boot.min.js".into());
        }
        "/dist/gr.race.min.js" | "/fe/gr.race.min.js" => {
            return Some("gr.race.min.js".into());
        }
        "/dist/gr.entry.min.js" | "/fe/gr.entry.min.js" => {
            return Some("gr.entry.min.js".into());
        }
        // Protocol 2 pin (inject /g5/gr.js → origin /dist/gr.js)
        "/dist/gr.js" | "/fe/gr.js" | "/gr.js" => {
            return Some("gr.min.js".into());
        }
        "/dist/gr.min.js" | "/fe/gr.min.js" => {
            return Some("gr.min.js".into());
        }
        // Seal v2 (wire logical names → greenpng/v8 dot-named physical files)
        "/dist/gr_seal_v2.wasm" | "/fe/gr_seal_v2.wasm" => {
            return Some("gr.seal_v2.wasm".into());
        }
        "/dist/gr_seal_v2_loader.js" | "/fe/gr_seal_v2_loader.js" => {
            return Some("gr.seal_v2_loader.js".into());
        }
        _ => {}
    }
    if path.starts_with("/dist/") || path.starts_with("/fe/") {
        if let Some(aliased) = map_boot_alias(&path_logical) {
            return Some(aliased.to_string());
        }
        return Some(path_logical);
    }
    None
}

fn resolve_static(root: &std::path::Path, rel: &str) -> PathBuf {
    // prevent path escape
    let clean = rel.trim_start_matches('/').replace("..", "");
    root.join(clean)
}
