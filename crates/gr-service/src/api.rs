//! GR control-plane HTTP API + health checks toward in-tree probe plane.

use axum::body::Body;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use tower_http::services::ServeDir;
use gr_admin::metrics;
use gr_admin::sites::SiteRecord;
use gr_module_ingest;
use gr_runtime::Runtime;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

#[derive(Clone)]
pub struct AppState {
    pub rt: Arc<Runtime>,
    pub spa_dir: PathBuf,
    pub probe_base: String,
    /// FE static root (e.g. /opt/greenpng/fe) for panel OTA install-fe.
    pub static_dir: PathBuf,
    /// Install root (e.g. /opt/greenpng) for panel OTA install-runtime.
    pub install_root: PathBuf,
    /// Shared multi-node LB pool (docs/guides/08-LB-MODULE.md).
    pub lb_pool: Arc<gr_lb::LbPool>,
}

pub fn router(
    rt: Arc<Runtime>,
    spa_dir: PathBuf,
    probe_bind: String,
    static_dir: PathBuf,
    install_root: PathBuf,
    lb_pool: Arc<gr_lb::LbPool>,
) -> Router {
    let console = rt.admin.auth.console_path.clone();
    let probe_base = if probe_bind.starts_with("http") {
        probe_bind
    } else {
        // 0.0.0.0:28765 → http://127.0.0.1:28765 for local proxy
        let port = probe_bind.rsplit(':').next().unwrap_or("28765");
        format!("http://127.0.0.1:{port}")
    };
    let state = AppState {
        rt: rt.clone(),
        spa_dir: spa_dir.clone(),
        probe_base,
        static_dir,
        install_root,
        lb_pool,
    };
    let spa_assets = spa_dir.join("assets");
    let _ = SPA_DIR.set(spa_dir);

    // Explicit paths (avoid axum nest trailing-slash mismatch on /{console}/)
    let c = format!("/{console}");
    Router::new()
        .route("/v1/health", get(control_health))
        .route("/v1/session/open", post(gated_open_forward))
        .route("/v1/ingest", post(gated_ingest_forward))
        .route(&format!("{c}/api/login"), post(admin_login))
        .route(&format!("{c}/api/logout"), post(admin_logout))
        .route(&format!("{c}/api/me"), get(admin_me))
        // Official-site OAuth / cloud vault removed: panel is standalone.
        .route("/oauth/start", get(gone_official))
        .route("/oauth/callback", get(gone_official))
        .route(&format!("{c}/api/cloud/status"), get(gone_official))
        .route(&format!("{c}/api/cloud/license-status"), get(gone_official))
        .route(&format!("{c}/api/cloud/license-import"), post(gone_official))
        .route(&format!("{c}/api/cloud/sync-strategies"), post(gone_official))
        .route(&format!("{c}/api/cloud/sync-sites"), post(gone_official))
        .route(&format!("{c}/api/status"), get(admin_status))
        .route(&format!("{c}/api/security"), get(admin_security))
        .route(&format!("{c}/api/metrics"), get(admin_metrics))
        .route(&format!("{c}/api/sites"), get(list_sites).post(upsert_site))
        .route(&format!("{c}/api/sites/delete"), post(delete_site_body))
        .route(
            &format!("{c}/api/sites/:site_id/embed_token/rotate"),
            post(rotate_embed_token),
        )
        // Direct-bind edge: panel-managed domains + SSL for the Pingora TLS
        // listener (SNI per hostname, no reverse proxy). Same admin plane as
        // /api/sites — endpoints on the existing router, no second web service.
        .route(&format!("{c}/api/domains"), get(domains_list).post(domains_upsert))
        .route(
            &format!("{c}/api/domains/:domain_id/ssl/mint"),
            post(domain_ssl_mint),
        )
        .route(
            &format!("{c}/api/domains/:domain_id/ssl/pem"),
            post(domain_ssl_pem),
        )
        .route(
            &format!("{c}/api/domains/:domain_id/ssl/acme"),
            post(domain_ssl_acme),
        )
        .route(&format!("{c}/api/runtime/apply"), post(runtime_apply))
        .route(&format!("{c}/api/workers"), get(get_workers).post(set_workers))
        .route(&format!("{c}/api/modules"), get(list_modules))
        .route(&format!("{c}/api/modules/activate"), post(modules_activate))
        .route(&format!("{c}/api/modules/stage"), post(modules_stage))
        .route(&format!("{c}/api/ota/local"), get(ota_local))
        .route(&format!("{c}/api/ota/remote"), get(ota_remote))
        .route(&format!("{c}/api/ota/install"), post(ota_install))
        .route(&format!("{c}/api/ota/install-all"), post(ota_install_all))
        .route(&format!("{c}/api/ota/set-release-url"), post(ota_set_release_url))
        .route(&format!("{c}/api/ota/install-fe"), post(ota_install_fe))
        .route(&format!("{c}/api/ota/install-runtime"), post(ota_install_runtime))
        .route(&format!("{c}/api/ota/full-upgrade"), post(ota_full_upgrade))
        .route(&format!("{c}/api/ota/cluster-apply"), post(ota_cluster_apply_authed))
        .route(&format!("{c}/api/ota/cluster-desired"), get(ota_cluster_desired))
        .route("/v1/cluster/ota-apply", post(cluster_ota_apply_key))
        .route("/v1/cluster/heartbeat", post(cluster_heartbeat_key))
        .route("/v1/ota-mirror/:ver/:asset", get(ota_mirror_asset))
        .route(&format!("{c}/api/ota/activate"), post(modules_activate))
        .route(&format!("{c}/api/audit"), get(list_audit))
        .route(&format!("{c}/api/cluster"), get(cluster_nodes))
        .route(&format!("{c}/api/cluster/heartbeat"), post(cluster_heartbeat))
        // P0: SDK key management — single admin plane; keys live in public.sdk_keys
        .route(
            &format!("{c}/api/sdk/keys"),
            get(sdk_keys_list).post(sdk_keys_create),
        )
        .route(
            &format!("{c}/api/sdk/keys/:key_id/revoke"),
            post(sdk_key_revoke),
        )
        .route(
            &format!("{c}/api/sdk/keys/:key_id/rotate"),
            post(sdk_key_rotate),
        )
        .route(&format!("{c}/api/sdk/embed"), get(sdk_embed))
        // Multi-node load balancer (docs/guides/08-LB-MODULE.md)
        .route(
            &format!("{c}/api/lb/config"),
            get(lb_config_get).post(lb_config_put),
        )
        .route(&format!("{c}/api/lb/status"), get(lb_status))
        .route(&format!("{c}/api/dashboard"), get(dashboard))
        .route(&format!("{c}/api/probe/health"), get(probe_health))
        // Management: results summary (proxied ops, not algorithm internals)
        .route(&format!("{c}/api/results/summary"), get(results_summary))
        .route(&format!("{c}/api/results/sessions"), get(results_sessions))
        .route(&format!("{c}/api/results/actions"), get(results_actions))
        // Strategy presets + site/default policy config
        .route(
            &format!("{c}/api/strategies"),
            get(strategies_list).post(strategies_save),
        )
        // Third-party IP / enrichment provider config
        .route(
            &format!("{c}/api/integrations"),
            get(integrations_get).post(integrations_save),
        )
        .route(
            &format!("{c}/api/retention"),
            get(retention_get).post(retention_save),
        )
        .route(
            &format!("{c}/api/retention/purge"),
            post(retention_purge_now),
        )
        // iss/opus5 05-S-5: DSAR erase / export (admin session auth).
        .route(&format!("{c}/api/dsar/erase"), post(dsar_erase))
        .route(&format!("{c}/api/dsar/export"), get(dsar_export))
        .route(&c, get(serve_spa_index))
        .route(&format!("{c}/"), get(serve_spa_index))
        .route(&format!("{c}/index.html"), get(serve_spa_index))
        // legacy flat SPA files (if present)
        .route(&format!("{c}/app.js"), get(|| async { serve_spa_flat("app.js") }))
        .route(&format!("{c}/styles.css"), get(|| async { serve_spa_flat("styles.css") }))
        // Vite build assets
        .nest_service(
            &format!("{c}/assets"),
            ServeDir::new(spa_assets),
        )
        .fallback(fallback_404)
        .layer(axum::middleware::from_fn(csrf_middleware))
        .layer(axum::middleware::from_fn(security_headers_middleware))
        .with_state(state)
}

async fn csrf_middleware(req: Request<Body>, next: axum::middleware::Next) -> Response {
    let method = req.method();
    let path = req.uri().path();
    let cookie_authenticated = req
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(|v| cookie_value_from_header(v, SESSION_COOKIE).is_some())
        .unwrap_or(false);
    if cookie_authenticated
        && path.contains("/api/")
        && !matches!(*method, axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS)
    {
        let origin = req.headers().get(header::ORIGIN).and_then(|v| v.to_str().ok());
        let host = req.headers().get(header::HOST).and_then(|v| v.to_str().ok());
        let proto = req.headers().get("x-forwarded-proto").and_then(|v| v.to_str().ok()).unwrap_or("http");
        let expected = host.map(|h| format!("{proto}://{h}"));
        if origin.zip(expected.as_deref()).map(|(o, e)| o == e).unwrap_or(false) == false {
            return (StatusCode::FORBIDDEN, Json(json!({"ok":false,"error":"csrf_origin_required"}))).into_response();
        }
    }
    next.run(req).await
}

async fn security_headers_middleware(
    req: Request<Body>,
    next: axum::middleware::Next,
) -> Response {
    let path = req.uri().path().to_string();
    let mut res = next.run(req).await;
    let headers = res.headers_mut();
    headers.insert(
        header::HeaderName::from_static("x-content-type-options"),
        header::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::HeaderName::from_static("x-frame-options"),
        header::HeaderValue::from_static("DENY"),
    );
    headers.insert(
        header::HeaderName::from_static("referrer-policy"),
        header::HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::HeaderName::from_static("permissions-policy"),
        header::HeaderValue::from_static("geolocation=(), microphone=(), camera=()"),
    );
    if path.contains("/api/") {
        headers.insert(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("no-store"),
        );
    }
    // Production Vue messages are precompiled at build time, so CSP need not
    // permit dynamic code evaluation.
    if !path.contains("/api/") {
        headers.insert(
            header::HeaderName::from_static("content-security-policy"),
            header::HeaderValue::from_static(
                "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'",
            ),
        );
    }
    res
}

static SPA_DIR: OnceLock<PathBuf> = OnceLock::new();

fn spa_dir() -> PathBuf {
    SPA_DIR
        .get()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("admin-spa"))
}

async fn serve_spa_index(State(st): State<AppState>) -> Response {
    let index = spa_dir().join("index.html");
    if index.is_file() {
        match std::fs::read(&index) {
            Ok(bytes) => Response::builder()
                .status(200)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .body(Body::from(bytes))
                .unwrap(),
            Err(_) => fallback_console_html(State(st)).await,
        }
    } else {
        fallback_console_html(State(st)).await
    }
}

fn content_type_for(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".js") || lower.ends_with(".mjs") {
        "application/javascript; charset=utf-8"
    } else if lower.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if lower.ends_with(".svg") {
        "image/svg+xml"
    } else if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".woff2") {
        "font/woff2"
    } else if lower.ends_with(".map") {
        "application/json"
    } else if lower.ends_with(".html") {
        "text/html; charset=utf-8"
    } else {
        "application/octet-stream"
    }
}

async fn serve_spa_vite_asset(Path(file): Path<String>) -> Response {
    if file.contains("..") || file.contains('/') || file.contains('\\') {
        return (StatusCode::BAD_REQUEST, "bad path").into_response();
    }
    let full = spa_dir().join("assets").join(&file);
    match std::fs::read(&full) {
        Ok(bytes) => Response::builder()
            .status(200)
            .header(header::CONTENT_TYPE, content_type_for(&file))
            .body(Body::from(bytes))
            .unwrap(),
        Err(_) => (StatusCode::NOT_FOUND, "missing").into_response(),
    }
}

fn serve_spa_flat(name: &str) -> Response {
    let full = spa_dir().join(name);
    match std::fs::read(&full) {
        Ok(bytes) => Response::builder()
            .status(200)
            .header(header::CONTENT_TYPE, content_type_for(name))
            .body(Body::from(bytes))
            .unwrap(),
        Err(_) => (StatusCode::NOT_FOUND, "missing").into_response(),
    }
}

async fn control_health(State(st): State<AppState>) -> Json<Value> {
    // Control-plane public health: no module paths / mint internals for unauth callers.
    Json(json!({
        "ok": true,
        "product": "greenpng",
        "role": st.rt.cfg.role,
    }))
}

async fn probe_health(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    // Authenticated admin: prefer detail health (queue/backends), not public stub.
    match reqwest_get(&format!("{}/v1/health/detail", st.probe_base)).await {
        Ok((code, body)) => Json(json!({"ok": code == 200, "status": code, "body": body})).into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"ok": false, "error": e})),
        )
            .into_response(),
    }
}

const SETTING_STRATEGY: &str = "panel_strategy_config_v1";
const SETTING_INTEGRATIONS: &str = "panel_integrations_v1";

fn default_strategy_config() -> Value {
    json!({
        "default_strategy_id": "balanced",
        "site_strategies": {},
        "result_policy": {
            "rpa_collect": true,
            "primary_device_lane": "dv0",
            "response_profile": "standard",
            "include_signals": true,
            "site_overrides": {}
        },
        "note": "Merchant decision preset for product_public; does not rewrite mint digests.",
    })
}

fn default_integrations() -> Value {
    json!({
        "ip_enrichment": {
            "enabled": true,
            "provider": "dbip_lite",
            "providers": {
                "dbip_lite": {
                    "label": "DB-IP Lite (bundled)",
                    "asn_mmdb_path": "",
                    "country_mmdb_path": "",
                    "env_asn": "GR_GEOIP_ASN_MMDB",
                    "env_country": "GR_GEOIP_COUNTRY_MMDB"
                },
                "maxmind_geolite2": {
                    "label": "MaxMind GeoLite2 / GeoIP2",
                    "account_id": "",
                    "license_key": "",
                    "asn_mmdb_path": "",
                    "country_mmdb_path": "",
                    "note": "Store license on server only; panel never logs secrets in audit detail."
                },
                "ipinfo": {
                    "label": "IPinfo",
                    "token": "",
                    "base_url": "https://ipinfo.io",
                    "note": "Optional remote API fallback when MMDB empty."
                },
                "custom_http": {
                    "label": "Custom HTTP enricher",
                    "url": "",
                    "header_auth": "",
                    "note": "POST {ip} → {asn,country,org,datacenter}"
                }
            },
            "vpn_proxy_source": "none",
            "vpn_proxy_note": "Optional commercial VPN/proxy feeds (IP2Proxy, Spur, etc.) — config only."
        },
        "webhooks": {
            "enabled": false,
            "result_url": "",
            "secret": ""
        }
    })
}

/// Probe completeness / site rollup for ops (no algorithm internals).
#[derive(Deserialize)]
struct ResultsQuery {
    hours: Option<u32>,
    site_id: Option<String>,
    limit: Option<u32>,
}

async fn results_summary(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ResultsQuery>,
) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    let hours = q.hours.unwrap_or(24).clamp(1, 168);
    let url = format!(
        "{}/v1/ops/probe_completeness?hours={hours}",
        st.probe_base
    );
    match reqwest_get(&url).await {
        Ok((code, mut body)) => {
            if let Some(sid) = q.site_id.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                if let Some(arr) = body.get_mut("by_site_version").and_then(|v| v.as_array_mut()) {
                    arr.retain(|row| {
                        row.get("site_id")
                            .and_then(|x| x.as_str())
                            .map(|s| s == sid)
                            .unwrap_or(false)
                    });
                }
                if let Some(obj) = body.as_object_mut() {
                    obj.insert("site_id_filter".into(), json!(sid));
                }
            }
            if let Some(obj) = body.as_object_mut() {
                obj.insert("ok".into(), json!(code == 200));
                obj.insert("hours".into(), json!(hours));
                obj.insert(
                    "panel_note".into(),
                    json!("Aggregated probe completeness by site × product_version — not algorithm internals."),
                );
            }
            Json(body).into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"ok": false, "error": e})),
        )
            .into_response(),
    }
}

/// Outcome / action-proxy distribution for Overview charts.
async fn results_actions(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ResultsQuery>,
) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    let hours = q.hours.unwrap_or(24).clamp(1, 168);
    let url = format!(
        "{}/v1/ops/outcome_distribution?hours={hours}",
        st.probe_base
    );
    match reqwest_get(&url).await {
        Ok((code, mut body)) => {
            if let Some(obj) = body.as_object_mut() {
                obj.insert("ok".into(), json!(code == 200));
                obj.insert("hours".into(), json!(hours));
            }
            Json(body).into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"ok": false, "error": e})),
        )
            .into_response(),
    }
}

/// Recent analysis session rows for a site (merchant-facing fields only).
async fn results_sessions(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ResultsQuery>,
) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    let limit = q.limit.unwrap_or(40).clamp(1, 200);
    let mut url = format!(
        "{}/v1/ops/analysis_latest?limit={limit}",
        st.probe_base
    );
    if let Some(sid) = q.site_id.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        url.push_str(&format!("&site_id={}", urlencoding_simple(sid)));
    }
    match reqwest_get(&url).await {
        Ok((code, body)) => {
            // Project to panel-safe columns when array present
            let mut rows = body
                .get("rows")
                .or_else(|| body.get("items"))
                .or_else(|| body.get("sessions"))
                .cloned()
                .unwrap_or_else(|| {
                    if body.is_array() {
                        body.clone()
                    } else {
                        json!([])
                    }
                });
            if let Some(sid) = q.site_id.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                if let Some(arr) = rows.as_array_mut() {
                    arr.retain(|row| {
                        row.get("site_id")
                            .and_then(|x| x.as_str())
                            .map(|s| s == sid)
                            .unwrap_or(false)
                    });
                }
            }
            // Strip heavy fields if present
            if let Some(arr) = rows.as_array_mut() {
                for row in arr.iter_mut() {
                    if let Some(obj) = row.as_object_mut() {
                        obj.remove("result_json");
                        obj.remove("materials");
                        obj.remove("fields");
                    }
                }
            }
            Json(json!({
                "ok": code == 200,
                "status": code,
                "limit": limit,
                "site_id": q.site_id,
                "sessions": rows,
                "panel_note": "Session-level product outcomes for operators; digests/mint internals omitted.",
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"ok": false, "error": e, "hint": "probe plane analysis_latest may be unavailable"})),
        )
            .into_response(),
    }
}

fn urlencoding_simple(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => format!("%{:02X}", c as u8),
        })
        .collect()
}

async fn strategies_list(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    let presets = gr_probe_core::list_strategy_presets();
    let cfg_raw = st
        .rt
        .admin
        .db
        .get_setting(SETTING_STRATEGY)
        .ok()
        .flatten();
    let mut config: Value = cfg_raw
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(default_strategy_config);
    // The policy file is the probe-plane source of truth. Backfill this
    // section for settings created before result-policy support.
    if let Some(obj) = config.as_object_mut() {
        if !obj.contains_key("result_policy") {
            let policy = gr_probe_core::load_panel_policy(Some(control_data_dir(&st).as_path()));
            obj.insert(
                "result_policy".into(),
                serde_json::to_value(policy.result_policy).unwrap_or_else(|_| json!({})),
            );
        }
    }
    let sites = st.rt.admin.db.list_sites().unwrap_or_default();
    let site_rows: Vec<Value> = sites
        .iter()
        .map(|s| {
            json!({
                "site_id": s.get("site_id").and_then(|v| v.as_str()).unwrap_or(""),
                "name": s.get("name").and_then(|v| v.as_str()).unwrap_or(""),
            })
        })
        .collect();
    Json(json!({
        "ok": true,
        "presets": presets,
        "config": config,
        "sites": site_rows,
        "cloud": crate::official_cloud::vault_snapshot(),
        "note": "Strategy bodies are owned by the official site; panel config is auto-synced read-only when OAuth-linked.",
    }))
    .into_response()
}

fn control_data_dir(st: &AppState) -> std::path::PathBuf {
    // Runtime data_dir is install root for sites; panel_policy lives next to probe store.
    st.rt.cfg.data_dir.clone()
}

fn write_panel_policy_file(
    st: &AppState,
    patch_strategy: Option<(&str, &Value)>,
    patch_result: Option<&Value>,
    patch_retention: Option<&Value>,
    patch_integrations: Option<&Value>,
) -> Result<std::path::PathBuf, String> {
    let dir = control_data_dir(st);
    let mut pol = gr_probe_core::load_panel_policy(Some(dir.as_path()));
    if let Some((def, sites)) = patch_strategy {
        pol = gr_probe_core::merge_strategy(pol, def, sites);
    }
    if let Some(result) = patch_result {
        pol = gr_probe_core::merge_result_policy(pol, result);
    }
    if let Some(r) = patch_retention {
        pol = gr_probe_core::merge_retention(pol, r);
    }
    if let Some(i) = patch_integrations {
        pol.integrations = i.clone();
    }
    gr_probe_core::save_panel_policy(Some(dir.as_path()), pol)
}

async fn strategies_save(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let actor = match authed(&st, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    if let Ok(Some(raw)) = st.rt.admin.db.get_setting(SETTING_STRATEGY) {
        if let Ok(existing) = serde_json::from_str::<Value>(&raw) {
            if existing.get("read_only") == Some(&json!(true))
                || existing.get("source").and_then(|v| v.as_str()) == Some("official_site")
            {
                return (
                    StatusCode::FORBIDDEN,
                    Json(json!({
                        "ok": false,
                        "error": "strategies_managed_by_official_site",
                        "hint": "Change strategy version on the official site, then POST /api/cloud/sync-strategies",
                    })),
                )
                    .into_response();
            }
        }
    }
    let mut cfg = st
        .rt
        .admin
        .db
        .get_setting(SETTING_STRATEGY)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .unwrap_or_else(default_strategy_config);
    if let Some(obj) = cfg.as_object_mut() {
        if !obj.contains_key("result_policy") {
            obj.insert(
                "result_policy".into(),
                default_strategy_config()["result_policy"].clone(),
            );
        }
        if let Some(d) = body.get("default_strategy_id").and_then(|v| v.as_str()) {
            if !d.is_empty() {
                obj.insert("default_strategy_id".into(), json!(d));
            }
        }
        if let Some(ss) = body.get("site_strategies").cloned() {
            obj.insert("site_strategies".into(), ss);
        }
        if let Some(rp) = body.get("result_policy").cloned() {
            obj.insert("result_policy".into(), rp);
        }
    }
    let s = serde_json::to_string(&cfg).unwrap_or_else(|_| "{}".into());
    if let Err(e) = st.rt.admin.db.set_setting(SETTING_STRATEGY, &s) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok": false, "error": e})),
        )
            .into_response();
    }
    let def = cfg
        .get("default_strategy_id")
        .and_then(|v| v.as_str())
        .unwrap_or("balanced");
    let sites = cfg
        .get("site_strategies")
        .cloned()
        .unwrap_or(json!({}));
    let result_patch = cfg.get("result_policy");
    let policy_path = write_panel_policy_file(&st, Some((def, &sites)), result_patch, None, None);
    let _ = st.rt.admin.db.audit(
        &actor,
        "strategy_config_save",
        "",
        json!({
            "default_strategy_id": cfg.get("default_strategy_id"),
            "site_strategy_keys": cfg.get("site_strategies").and_then(|v| v.as_object()).map(|m| m.keys().cloned().collect::<Vec<_>>()),
            "result_policy": cfg.get("result_policy"),
            "policy_path": policy_path.as_ref().ok().map(|p| p.display().to_string()),
        }),
    );
    Json(json!({
        "ok": true,
        "config": cfg,
        "effective": true,
        "policy_path": policy_path.as_ref().ok().map(|p| p.display().to_string()),
        "note": "Probe plane get_result applies this strategy when strategy_id query is omitted.",
    }))
    .into_response()
}

async fn retention_get(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    let pol = gr_probe_core::load_panel_policy(Some(control_data_dir(&st).as_path()));
    Json(json!({
        "ok": true,
        "retention": pol.retention,
        "policy": gr_probe_core::policy_public_view(&pol),
        "defaults": {
            "analysis_retention_days": 30,
            "session_retention_days": 30,
            "cold_ttl_days": 7,
            "batch_delete_limit": 200,
            "purge_interval_sec": 300,
            "velocity_retention_days": 7,
        },
    }))
    .into_response()
}

async fn retention_save(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let actor = match authed(&st, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let patch = body.get("retention").cloned().unwrap_or(body);
    let path = match write_panel_policy_file(&st, None, None, Some(&patch), None) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok": false, "error": e})),
            )
                .into_response();
        }
    };
    let pol = gr_probe_core::load_panel_policy(Some(control_data_dir(&st).as_path()));
    let _ = st.rt.admin.db.audit(
        &actor,
        "retention_config_save",
        "",
        json!({
            "analysis_retention_days": pol.retention.analysis_retention_days,
            "batch_delete_limit": pol.retention.batch_delete_limit,
            "enabled": pol.retention.enabled,
            "policy_path": path.display().to_string(),
        }),
    );
    // Push cold TTL env-compatible note (runtime purge reads panel_policy)
    Json(json!({
        "ok": true,
        "retention": pol.retention,
        "policy_path": path.display().to_string(),
        "effective": true,
        "note": "Background purge runs in small batches on the probe plane; no full-table delete storms.",
    }))
    .into_response()
}

async fn retention_purge_now(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let actor = match authed(&st, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let batches = body
        .get("batches")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .clamp(1, 20) as u32;
    // Proxy to probe plane so purge hits the real probe store backend.
    let url = format!(
        "{}/v1/ops/retention_purge?batches={batches}",
        st.probe_base
    );
    match reqwest_get(&url).await {
        Ok((code, body)) => {
            let _ = st.rt.admin.db.audit(
                &actor,
                "retention_purge_now",
                "",
                json!({"batches": batches, "status": code}),
            );
            Json(json!({
                "ok": code == 200,
                "status": code,
                "result": body,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"ok": false, "error": e})),
        )
            .into_response(),
    }
}

/// iss/opus5 05-S-5: DSAR erase (admin plane). Proxies to the probe-plane
/// store cascade; for kind=site_id additionally tears down control-plane site
/// rows (sites/tenants/licenses) — that data only lives here.
async fn dsar_erase(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let actor = match authed(&st, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let kind = body
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let value = body
        .get("value")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if !matches!(
        kind.as_str(),
        "visitor_terminal_id" | "vt" | "device_id" | "client_ip" | "site_id"
    ) || value.is_empty()
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok": false, "error": "kind (visitor_terminal_id|device_id|client_ip|site_id) + value required"})),
        )
            .into_response();
    }
    // Never log the raw selector — fingerprint only.
    let fp = gr_probe_core::sha256_hex(value.as_bytes());
    let fp: String = fp.chars().take(16).collect();

    let url = format!("{}/v1/ops/dsar/erase", st.probe_base);
    let payload = json!({"kind": kind, "value": value});
    match reqwest_post_json(&url, &payload).await {
        Ok((code, res)) => {
            let mut control = Value::Null;
            if kind == "site_id" {
                // Control-plane teardown (best-effort; S-5).
                control = match st.rt.admin.db.delete_site(&value) {
                    Ok(()) => {
                        let _ = st.rt.refresh_live_config();
                        json!({"ok": true, "site_deleted": true})
                    }
                    Err(e) => json!({"ok": false, "error": e}),
                };
            }
            let _ = st.rt.admin.db.audit(
                &actor,
                "dsar_erase",
                &format!("{kind}:{fp}"),
                json!({"status": code, "control_plane": control}),
            );
            Json(json!({
                "ok": code == 200,
                "status": code,
                "selector_sha256_16": fp,
                "probe_plane": res,
                "control_plane": control,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"ok": false, "error": e})),
        )
            .into_response(),
    }
}

/// iss/opus5 05-S-5: DSAR export (admin plane) — proxies to probe plane.
async fn dsar_export(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let actor = match authed(&st, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let kind = q.get("kind").map(|s| s.trim().to_string()).unwrap_or_default();
    let value = q
        .get("value")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if !matches!(
        kind.as_str(),
        "visitor_terminal_id" | "vt" | "device_id" | "client_ip" | "site_id"
    ) || value.is_empty()
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok": false, "error": "kind + value required"})),
        )
            .into_response();
    }
    let fp = gr_probe_core::sha256_hex(value.as_bytes());
    let fp: String = fp.chars().take(16).collect();
    let url = format!(
        "{}/v1/ops/dsar/export?kind={}&value={}",
        st.probe_base,
        urlenc(&kind),
        urlenc(&value)
    );
    match reqwest_get(&url).await {
        Ok((code, res)) => {
            let _ = st.rt.admin.db.audit(
                &actor,
                "dsar_export",
                &format!("{kind}:{fp}"),
                json!({"status": code}),
            );
            Json(json!({"ok": code == 200, "status": code, "result": res})).into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"ok": false, "error": e})),
        )
            .into_response(),
    }
}

fn urlenc(s: &str) -> String {
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

/// POST JSON helper (blocking reqwest in spawn_blocking, ops-token aware).
async fn reqwest_post_json(url: &str, body: &Value) -> Result<(u16, Value), String> {
    let url = url.to_string();
    let body = body.clone();
    tokio::task::spawn_blocking(move || {
        let token = gr_abi::env::get("OPS_TOKEN").unwrap_or_default();
        let token = token.trim().to_string();
        let mut req = reqwest::blocking::Client::new().post(&url).json(&body);
        if !token.is_empty() {
            req = req.header("x-gr-ops-token", token);
        }
        let r = req.send().map_err(|e| e.to_string())?;
        let code = r.status().as_u16();
        let text = r.text().unwrap_or_default();
        let parsed = serde_json::from_str(&text).unwrap_or(json!({"raw": text}));
        Ok((code, parsed))
    })
    .await
    .map_err(|e| e.to_string())?
}

async fn integrations_get(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    let cfg_raw = st
        .rt
        .admin
        .db
        .get_setting(SETTING_INTEGRATIONS)
        .ok()
        .flatten();
    let mut config: Value = cfg_raw
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(default_integrations);
    // Mask secrets for UI
    if let Some(ip) = config
        .pointer_mut("/ip_enrichment/providers/maxmind_geolite2/license_key")
    {
        if let Some(s) = ip.as_str().filter(|s| !s.is_empty()) {
            *ip = json!(mask_secret(s));
        }
    }
    if let Some(ip) = config.pointer_mut("/ip_enrichment/providers/ipinfo/token") {
        if let Some(s) = ip.as_str().filter(|s| !s.is_empty()) {
            *ip = json!(mask_secret(s));
        }
    }
    if let Some(ip) = config.pointer_mut("/ip_enrichment/providers/custom_http/header_auth") {
        if let Some(s) = ip.as_str().filter(|s| !s.is_empty()) {
            *ip = json!(mask_secret(s));
        }
    }
    if let Some(ip) = config.pointer_mut("/webhooks/secret") {
        if let Some(s) = ip.as_str().filter(|s| !s.is_empty()) {
            *ip = json!(mask_secret(s));
        }
    }
    // Live env path hint (no secrets)
    let env_asn = gr_abi::env::get("GEOIP_ASN_MMDB").unwrap_or_default();
    let env_cc = gr_abi::env::get("GEOIP_COUNTRY_MMDB").unwrap_or_default();
    Json(json!({
        "ok": true,
        "config": config,
        "runtime_env": {
            "GR_GEOIP_ASN_MMDB": if env_asn.is_empty() { Value::Null } else { json!(env_asn) },
            "GR_GEOIP_COUNTRY_MMDB": if env_cc.is_empty() { Value::Null } else { json!(env_cc) },
        },
        "catalog": [
            {"id": "dbip_lite"},
            {"id": "maxmind_geolite2"},
            {"id": "ipinfo"},
            {"id": "custom_http"},
        ],
    }))
    .into_response()
}

fn mask_secret(s: &str) -> String {
    if s.len() <= 4 {
        "****".into()
    } else {
        format!("****{}", &s[s.len().saturating_sub(4)..])
    }
}

async fn integrations_save(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let actor = match authed(&st, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    // Merge onto existing so masked **** fields don't wipe secrets
    let prev_raw = st
        .rt
        .admin
        .db
        .get_setting(SETTING_INTEGRATIONS)
        .ok()
        .flatten();
    let mut config: Value = prev_raw
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(default_integrations);
    if let Some(incoming) = body.get("config").or(Some(&body)) {
        merge_integrations(&mut config, incoming);
    }
    let s = serde_json::to_string(&config).unwrap_or_else(|_| "{}".into());
    if let Err(e) = st.rt.admin.db.set_setting(SETTING_INTEGRATIONS, &s) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok": false, "error": e})),
        )
            .into_response();
    }
    let provider = config
        .pointer("/ip_enrichment/provider")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let _ = write_panel_policy_file(&st, None, None, None, Some(&config));
    let _ = st.rt.admin.db.audit(
        &actor,
        "integrations_save",
        provider,
        json!({"provider": provider, "enabled": config.pointer("/ip_enrichment/enabled")}),
    );
    Json(json!({"ok": true, "effective": true})).into_response()
}

/// Sync control-plane site into probe-plane admin Postgres (`public.sites` / domains).
/// Control-plane rows live in schema `control`; probe-plane reads `public`.
fn sync_site_to_probe_admin(_data_dir: &std::path::Path, site: &SiteRecord) {
    let site_pg = site.clone();
    let _ = std::thread::Builder::new()
        .name("gr-site-pg-sync".into())
        .spawn(move || {
            if let Err(e) = sync_site_to_probe_admin_pg(&site_pg) {
                log::warn!("probe admin PG site sync failed site_id={}: {e}", site_pg.site_id);
            }
        });
}

fn sync_site_to_probe_admin_pg(site: &SiteRecord) -> Result<(), String> {
    let url = gr_abi::env::get("ADMIN_DATABASE_URL").unwrap_or_default();
    let url = url.trim().to_string();
    if url.is_empty() {
        return Err("GR_ADMIN_DATABASE_URL is required for probe admin site sync (legacy GR_/GR_ accepted)".into());
    }
    let mut client =
        postgres::Client::connect(&url, postgres::NoTls).map_err(|e| e.to_string())?;
    // P1-3 (iss/grok4.6/05): one-shot admin connection — suppress NOTICE noise.
    let _ = client.batch_execute("SET client_min_messages = warning");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let en: i32 = if site.collect_enabled { 1 } else { 0 };
    client
        .execute(
            r#"
            CREATE TABLE IF NOT EXISTS sites (
              site_id TEXT PRIMARY KEY,
              name TEXT NOT NULL,
              collect_enabled INTEGER NOT NULL DEFAULT 1,
              notes TEXT NOT NULL DEFAULT '',
              edge_mode TEXT NOT NULL DEFAULT 'first_party',
              pv_base TEXT NOT NULL DEFAULT '',
              gv_base TEXT NOT NULL DEFAULT '',
              fe_load TEXT NOT NULL DEFAULT 'pv',
              upload_ingest TEXT NOT NULL DEFAULT 'gv',
              poll_method TEXT NOT NULL DEFAULT 'both',
              cookie_fields TEXT NOT NULL DEFAULT '[]',
              embed_token TEXT NOT NULL DEFAULT '',
              created_ms BIGINT NOT NULL,
              updated_ms BIGINT NOT NULL
            )
            "#,
            &[],
        )
        .map_err(|e| e.to_string())?;
    let cookie_fields =
        serde_json::to_string(&site.cookie_fields).unwrap_or_else(|_| "[]".to_string());
    let _ = client.batch_execute(
        "ALTER TABLE sites ADD COLUMN IF NOT EXISTS embed_token TEXT NOT NULL DEFAULT ''",
    );
    client
        .execute(
            r#"
            INSERT INTO sites(
              site_id,name,collect_enabled,notes,edge_mode,pv_base,gv_base,
              fe_load,upload_ingest,poll_method,cookie_fields,embed_token,created_ms,updated_ms
            ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$13)
            ON CONFLICT(site_id) DO UPDATE SET
              name=EXCLUDED.name,
              collect_enabled=EXCLUDED.collect_enabled,
              notes=EXCLUDED.notes,
              edge_mode=EXCLUDED.edge_mode,
              pv_base=EXCLUDED.pv_base,
              gv_base=EXCLUDED.gv_base,
              fe_load=EXCLUDED.fe_load,
              upload_ingest=EXCLUDED.upload_ingest,
              poll_method=EXCLUDED.poll_method,
              cookie_fields=EXCLUDED.cookie_fields,
              embed_token=CASE WHEN EXCLUDED.embed_token = '' THEN sites.embed_token ELSE EXCLUDED.embed_token END,
              updated_ms=EXCLUDED.updated_ms
            "#,
            &[
                &site.site_id,
                &site.name,
                &en,
                &site.notes,
                &site.edge_mode,
                &site.pv_base,
                &site.gv_base,
                &site.fe_load,
                &site.upload_ingest,
                &site.poll_method,
                &cookie_fields,
                &site.embed_token,
                &now,
            ],
        )
        .map_err(|e| e.to_string())?;
    client
        .execute(
            r#"
            CREATE TABLE IF NOT EXISTS domains (
              domain_id TEXT PRIMARY KEY,
              site_id TEXT NOT NULL,
              hostname TEXT NOT NULL UNIQUE,
              display_name TEXT NOT NULL DEFAULT '',
              collect_enabled INTEGER NOT NULL DEFAULT 1,
              created_ms BIGINT NOT NULL,
              updated_ms BIGINT NOT NULL
            )
            "#,
            &[],
        )
        .ok();
    for root in &site.root_domains {
        let host = root.trim().to_ascii_lowercase();
        if host.is_empty() {
            continue;
        }
        let did = format!("d_{:x}", md5_simple(&host));
        let did = did.chars().take(14).collect::<String>();
        let _ = client.execute(
            r#"
            INSERT INTO domains(domain_id,site_id,hostname,display_name,collect_enabled,created_ms,updated_ms)
            VALUES($1,$2,$3,$4,$5,$6,$6)
            ON CONFLICT(hostname) DO UPDATE SET
              site_id=EXCLUDED.site_id,
              collect_enabled=EXCLUDED.collect_enabled,
              updated_ms=EXCLUDED.updated_ms
            "#,
            &[&did, &site.site_id, &host, &site.name, &en, &now],
        );
    }
    Ok(())
}

fn md5_simple(s: &str) -> u128 {
    // lightweight non-crypto hash for domain_id (not security-sensitive)
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish() as u128
}

fn merge_integrations(dst: &mut Value, src: &Value) {
    let Some(src_obj) = src.as_object() else {
        return;
    };
    let Some(dst_obj) = dst.as_object_mut() else {
        return;
    };
    for (k, v) in src_obj {
        if k == "providers" {
            if !dst_obj.contains_key("providers") {
                dst_obj.insert("providers".into(), json!({}));
            }
            if let (Some(sp), Some(dp)) = (
                v.as_object(),
                dst_obj.get_mut("providers").and_then(|x| x.as_object_mut()),
            ) {
                for (pk, pv) in sp {
                    if let Some(existing) = dp.get_mut(pk) {
                        merge_provider_obj(existing, pv);
                    } else {
                        dp.insert(pk.clone(), pv.clone());
                    }
                }
            }
            continue;
        }
        if let Some(cur) = dst_obj.get_mut(k) {
            if cur.is_object() && v.is_object() {
                merge_integrations(cur, v);
            } else if !is_masked_secret(v) {
                *cur = v.clone();
            }
        } else if !is_masked_secret(v) {
            dst_obj.insert(k.clone(), v.clone());
        }
    }
}

fn merge_provider_obj(dst: &mut Value, src: &Value) {
    let Some(src_obj) = src.as_object() else {
        return;
    };
    let Some(dst_obj) = dst.as_object_mut() else {
        return;
    };
    for (k, v) in src_obj {
        if is_masked_secret(v) {
            continue;
        }
        dst_obj.insert(k.clone(), v.clone());
    }
}

fn is_masked_secret(v: &Value) -> bool {
    v.as_str()
        .map(|s| s.starts_with("****"))
        .unwrap_or(false)
}

fn host_from(headers: &HeaderMap, body: &Value) -> String {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(',')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_string();
    if !host.is_empty() {
        return host;
    }
    let trust = matches!(
        gr_abi::env::get("TRUST_FORWARDED")
            .as_deref()
            .map(|s| s.trim()),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    );
    if trust {
        let fwd = headers
            .get("x-forwarded-host")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .split(',')
            .next()
            .unwrap_or("")
            .split(':')
            .next()
            .unwrap_or("")
            .to_string();
        if !fwd.is_empty() {
            return fwd;
        }
    }
    let prod = matches!(
        gr_abi::env::get("DEPLOY_ENV")
             .or_else(|| gr_abi::env::get("DEPLOY_ENV"))
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "prod" | "production" | "live"
    );
    if prod {
        return String::new();
    }
    body.get("page_host")
        .or_else(|| body.get("host"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}

/// Domain gate (v6) then forward to full v5 open handler on probe plane.
async fn gated_open_forward(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let host = host_from(&headers, &body);
    if let Err(_) = gr_module_ingest::gate_host(&host) {
        // only enforce when sites configured (gate_host returns lab when empty)
        // re-check: forbidden only
    }
    match gr_module_ingest::gate_host(&host) {
        Ok(_) => forward_json(&st.probe_base, "/v1/session/open", &body).await,
        Err(_) => (
            StatusCode::FORBIDDEN,
            Json(gr_module_ingest::forbidden_body()),
        )
            .into_response(),
    }
}

async fn gated_ingest_forward(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let host = host_from(&headers, &body);
    match gr_module_ingest::gate_host(&host) {
        Ok(_) => forward_json(&st.probe_base, "/v1/ingest", &body).await,
        Err(_) => (
            StatusCode::FORBIDDEN,
            Json(gr_module_ingest::forbidden_body()),
        )
            .into_response(),
    }
}

async fn forward_json(base: &str, path: &str, body: &Value) -> Response {
    let url = format!("{base}{path}");
    let body = body.clone();
    let res = tokio::task::spawn_blocking(move || {
        reqwest::blocking::Client::new()
            .post(&url)
            .json(&body)
            .send()
            .map_err(|e| e.to_string())
            .and_then(|r| {
                let status = r.status().as_u16();
                let text = r.text().unwrap_or_default();
                Ok((status, text))
            })
    })
    .await;
    match res {
        Ok(Ok((code, text))) => {
            let status = StatusCode::from_u16(code).unwrap_or(StatusCode::BAD_GATEWAY);
            let val: Value = serde_json::from_str(&text).unwrap_or(json!({"raw": text}));
            (status, Json(val)).into_response()
        }
        Ok(Err(e)) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"ok": false, "error": format!("probe_plane: {e}")})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"ok": false, "error": format!("probe_plane: {e}")})),
        )
            .into_response(),
    }
}

async fn reqwest_get(url: &str) -> Result<(u16, Value), String> {
    // use blocking in spawn_blocking to not freeze runtime
    let url = url.to_string();
    tokio::task::spawn_blocking(move || {
        // Attach ops token when the probe plane enforces ops auth
        // (GR_REQUIRE_OPS_AUTH=1 / prod). No-op when unset (open lab).
        let token = gr_abi::env::get("OPS_TOKEN").unwrap_or_default();
        let token = token.trim().to_string();
        let mut req = reqwest::blocking::Client::new().get(&url);
        if !token.is_empty() {
            req = req.header("x-gr-ops-token", token);
        }
        let r = req.send().map_err(|e| e.to_string())?;
        let code = r.status().as_u16();
        let text = r.text().unwrap_or_default();
        let body = serde_json::from_str(&text).unwrap_or(json!({"raw": text}));
        Ok((code, body))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Deserialize)]
struct LoginBody {
    username: String,
    password: String,
}

const SESSION_COOKIE: &str = "gr_session";
#[allow(dead_code)]
const OAUTH_STATE_COOKIE: &str = "gr_oauth_state";

/// iss/opus5 S-8: `Secure` is now default-on (local http dev must opt out via
/// `GR_DEV_INSECURE_COOKIE=1`; deployment env aliases may still set the same key);
/// `GR_COOKIE_SECURE` remains an explicit override.
fn cookie_secure() -> bool {
    if let Some(v) = gr_abi::env::get("COOKIE_SECURE") {
        return v == "1" || v.eq_ignore_ascii_case("true");
    }
    !gr_abi::env::get("DEV_INSECURE_COOKIE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn cookie_value_from_header(raw: &str, name: &str) -> Option<String> {
    for part in raw.split(';') {
        let Some((k, v)) = part.trim().split_once('=') else {
            continue;
        };
        if k.trim().eq_ignore_ascii_case(name) {
            let v = v.trim().trim_matches('"');
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn cookie_header(name: &str, path: &str, token: &str, max_age: i64) -> String {
    let mut parts = vec![
        format!("{}={}", name, token),
        format!("Path={path}"),
        "HttpOnly".into(),
        "SameSite=Lax".into(),
        format!("Max-Age={max_age}"),
    ];
    if cookie_secure() {
        parts.push("Secure".into());
    }
    parts.join("; ")
}

fn session_cookie_header(name: &str, console: &str, token: &str, clear: bool) -> String {
    let path = format!("/{}/", console.trim_matches('/'));
    let max_age = if clear {
        0
    } else {
        gr_admin::AdminAuth::session_ttl_secs()
    };
    cookie_header(name, &path, if clear { "" } else { token }, max_age)
}

fn append_set_cookie(headers: &mut HeaderMap, cookie: String) {
    if let Ok(value) = header::HeaderValue::from_str(&cookie) {
        headers.append(header::SET_COOKIE, value);
    }
}

fn append_session_cookie_headers(headers: &mut HeaderMap, console: &str, token: &str, clear: bool) {
    append_set_cookie(headers, session_cookie_header(SESSION_COOKIE, console, token, clear));
}

#[allow(dead_code)]
fn oauth_state_cookie_header(name: &str, state: &str, clear: bool) -> String {
    cookie_header(
        name,
        "/oauth/callback",
        if clear { "" } else { state },
        if clear { 0 } else { 600 },
    )
}

#[allow(dead_code)]
fn append_oauth_state_cookie_headers(headers: &mut HeaderMap, state: &str, clear: bool) {
    append_set_cookie(headers, oauth_state_cookie_header(OAUTH_STATE_COOKIE, state, clear));
}

fn token_from_headers(headers: &HeaderMap) -> String {
    if let Some(auth) = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
    {
        if !auth.is_empty() {
            return auth.to_string();
        }
    }
    if let Some(t) = headers.get("x-gr-token").and_then(|v| v.to_str().ok()) {
        if !t.is_empty() {
            return t.to_string();
        }
    }
    // HttpOnly cookie (SPA session).
    if let Some(raw) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) {
        if let Some(v) = cookie_value_from_header(raw, SESSION_COOKIE) {
            return v;
        }
    }
    String::new()
}

async fn admin_login(
    State(st): State<AppState>,
    _headers: HeaderMap,
    ConnectInfo(peer_addr): ConnectInfo<std::net::SocketAddr>,
    Json(body): Json<LoginBody>,
) -> Response {
    let deploy = gr_abi::env::get("DEPLOY_ENV").unwrap_or_default().to_ascii_lowercase();
    let prod = matches!(deploy.as_str(), "prod" | "production" | "live");
    // 2026-09 product decision: local admin password login is the primary
    // path in every environment — the official-site OAuth requirement has
    // been removed. The break-glass audit branch below is kept for tracing.
    let peer = peer_addr.ip().to_string();
    match st
        .rt
        .admin
        .auth
        .login(&st.rt.admin.db, &peer, &body.username, &body.password)
    {
        Ok(token) => {
            if prod {
                let _ = st.rt.admin.db.audit(
                    &body.username,
                    "break_glass_login",
                    &peer_addr.to_string(),
                    json!({"source":"local_password"}),
                );
            }
            let mut res = Json(json!({ "ok": true, "token": token })).into_response();
            append_session_cookie_headers(res.headers_mut(), &st.rt.admin.auth.console_path, &token, false);
            res
        }
        Err(e) => {
            let status = if e == "rate_limited" {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::UNAUTHORIZED
            };
            (
                status,
                Json(json!({
                    "ok": false,
                    "error": e,
                    "hint": "Use the one-time bootstrap password from <data-dir>/admin/admin_bootstrap_once.txt, then rotate it via environment configuration.",
                })),
            )
                .into_response()
        }
    }
}

fn constant_time_equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn gone_official() -> Response {
    (
        StatusCode::GONE,
        Json(json!({
            "ok": false,
            "error": "gone",
            "hint": "Official-site OAuth and cloud vault are removed. Sign in with the local admin account."
        })),
    )
        .into_response()
}

#[allow(dead_code)]
async fn oauth_start() -> Response {
    let cfg = crate::official_cloud::OfficialCloudCfg::from_env();
    match crate::official_cloud::oauth_authorize_url(&cfg) {
        Ok((url, state)) => {
            // iss/opus5 S-7: bind state to the browser (HttpOnly, SameSite=Lax,
            // path-scoped to the callback) to close login CSRF / session fixation.
            let mut resp = axum::response::Redirect::temporary(&url).into_response();
            append_oauth_state_cookie_headers(resp.headers_mut(), &state, false);
            resp
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok": false, "error": e})),
        )
            .into_response(),
    }
}

#[allow(dead_code)]
fn cookie_state(headers: &HeaderMap) -> String {
    let raw = headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    cookie_value_from_header(raw, OAUTH_STATE_COOKIE).unwrap_or_default()
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct OAuthCb {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

#[allow(dead_code)]
async fn oauth_callback(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<OAuthCb>,
) -> Response {
    if let Some(err) = q.error {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok": false, "error": err})),
        )
            .into_response();
    }
    let code = match q.code.as_deref() {
        Some(c) if !c.is_empty() => c,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"ok": false, "error": "missing_code"})),
            )
                .into_response();
        }
    };
    let state = q.state.unwrap_or_default();
    // S-7: the callback must arrive in the browser that started the flow.
    if state.is_empty() || cookie_state(&headers) != state {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok": false, "error": "state_cookie_mismatch"})),
        )
            .into_response();
    }
    let cfg = crate::official_cloud::OfficialCloudCfg::from_env();
    match crate::official_cloud::oauth_exchange_code(&cfg, code, &state).await {
        Ok(sess) => {
            let prod = matches!(
                gr_abi::env::get("DEPLOY_ENV")
                     .or_else(|| gr_abi::env::get("DEPLOY_ENV"))
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .as_str(),
                "prod" | "production" | "live"
            );
            let lab_email = gr_abi::env::get("OFFICIAL_LAB_EMAIL").unwrap_or_default() == "1";
            if prod && !lab_email && !sess.email_verified {
                return (
                    StatusCode::FORBIDDEN,
                    Json(json!({"ok": false, "error": "email_unverified"})),
                )
                    .into_response();
            }
            let allow: Vec<String> = gr_abi::env::get("OAUTH_ADMIN_EMAILS")
                .unwrap_or_default()
                .split(',')
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .collect();
            let email_lc = sess.email.trim().to_ascii_lowercase();
            if allow.is_empty() {
                if prod && !lab_email {
                    return (
                        StatusCode::FORBIDDEN,
                        Json(json!({"ok": false, "error": "oauth_allowlist_required"})),
                    )
                        .into_response();
                }
            } else if !allow.iter().any(|e| e == &email_lc) {
                return (
                    StatusCode::FORBIDDEN,
                    Json(json!({"ok": false, "error": "oauth_email_not_allowlisted"})),
                )
                    .into_response();
            }
            let uname = match crate::official_cloud::resolve_bound_local_user(
                &st.rt.admin.db,
                &sess,
                &cfg,
                prod && !lab_email,
            ) {
                Ok(u) => u,
                Err(e) => {
                    return (
                        StatusCode::FORBIDDEN,
                        Json(json!({"ok": false, "error": e})),
                    )
                        .into_response();
                }
            };
            let token = match st.rt.admin.auth.create_session(&st.rt.admin.db, &uname) {
                Ok(t) => t,
                Err(e) => {
                    return (
                        StatusCode::FORBIDDEN,
                        Json(json!({"ok": false, "error": e})),
                    )
                        .into_response();
                }
            };
            let _ = st.rt.admin.db.set_setting(
                "official_oauth_linked",
                &json!({"at_ms": js_now_ms(), "via": "oauth"}).to_string(),
            );
            let cfg2 = cfg.clone();
            tokio::spawn(async move {
                let _ = crate::official_cloud::sync_all_site_bundles(&cfg2).await;
            });
            let dest = format!("/{}/", st.rt.admin.auth.console_path);
            let mut res = axum::response::Redirect::temporary(&dest).into_response();
            append_session_cookie_headers(res.headers_mut(), &st.rt.admin.auth.console_path, &token, false);
            append_oauth_state_cookie_headers(res.headers_mut(), "", true);
            res
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok": false, "error": e})),
        )
            .into_response(),
    }
}

#[allow(dead_code)]
fn js_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// iss/opus5 05 low: `/cloud/status` previously skipped `authed()` while
/// neighboring sync endpoints enforced it. Vault snapshot contains strategy
/// bodies (policy) — require the admin credential before disclosure.
#[allow(dead_code)]
async fn cloud_status(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    Json(crate::official_cloud::vault_snapshot()).into_response()
}

/// iss/opus5 06-P0-3: panel visibility into the entitlement lifecycle —
/// current plan source, offline-grace marker, next scheduled verification.
#[allow(dead_code)]
async fn cloud_license_status(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    // License files live next to panel_policy.json (control_data_dir is the
    // SSOT for the panel policy location in this process).
    let data_dir = control_data_dir(&st);
    let mut out = crate::official_cloud::license_sync_status();
    // Per-site entitlement provenance (signed license / vault / unsigned).
    if let Ok(sites) = st.rt.admin.db.list_sites() {
        let mut rows = Vec::new();
        for s in sites.iter().take(200) {
            let sid = s.get("site_id").and_then(|v| v.as_str()).map(|s| s.to_string());
            let (ent, prov) = gr_probe_core::plan_entitlement::resolve_site_entitlement_status(
                sid.as_deref(),
                None,
                Some(&data_dir),
            );
            rows.push(json!({
                "site_id": sid,
                "plan": ent.plan,
                "rpa_enabled": ent.rpa_enabled,
                "provenance": prov,
            }));
        }
        out.as_object_mut()
            .map(|o| o.insert("sites".into(), Value::Array(rows)));
    }
    Json(out).into_response()
}

/// iss/opus5 06-P0-1: manual import of an issuer-signed license token
/// (out-of-band delivery). Only the raw token is stored; verification with
/// the built-in/env pubkey happens on every entitlement resolution.
#[allow(dead_code)]
async fn cloud_license_import(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(v): Json<Value>,
) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    let token = v
        .get("license_token")
        .or_else(|| v.get("token"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if !token.starts_with(&gr_probe_core::license_token::token_prefix()) {
        // P0-4: response code literal obfuscated (compile-time, per-release salt).
        return (
            StatusCode::BAD_REQUEST,
            gr_obf::obf!("malformed_license_token").s(),
        )
            .into_response();
    }
    let data_dir = control_data_dir(&st);
    // Verify BEFORE caching so a bad signature/typo fails loudly at import.
    let Some(pk) = gr_probe_core::license_token::license_pubkey(Some(&data_dir)) else {
        return (
            StatusCode::PRECONDITION_FAILED,
            "no_license_pubkey: set GR_LICENSE_PUBKEY_B64 or sync with the official site first",
        )
            .into_response();
    };
    match gr_probe_core::license_token::verify_token(&token, &pk, js_now_ms()) {
        Ok(verified) => {
            if let Err(e) =
                gr_probe_core::license_token::cache_token(Some(&data_dir), &token)
            {
                return (StatusCode::INTERNAL_SERVER_ERROR, format!("cache: {e}")).into_response();
            }
            Json(json!({
                "ok": true,
                "license_id": verified.claims.license_id,
                "site_id": verified.claims.site_id,
                "domain": verified.claims.domain,
                "plan": verified.claims.plan,
                "exp_ms": verified.claims.exp_ms,
                "state": match verified.state {
                    gr_probe_core::license_token::LicenseState::Active => "active",
                    gr_probe_core::license_token::LicenseState::OfflineGrace => "offline_grace",
                },
            }))
            .into_response()
        }
        Err(e) => (StatusCode::FORBIDDEN, format!("license_verify_failed: {e}")).into_response(),
    }
}

#[allow(dead_code)]
async fn cloud_sync_strategies(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    let cfg = crate::official_cloud::OfficialCloudCfg::from_env();
    match crate::official_cloud::sync_all_site_bundles(&cfg).await {
        Ok(v) => {
            if let Some(arr) = v.get("loaded").and_then(|x| x.as_array()) {
                let mut site_map = serde_json::Map::new();
                let mut ent_map = serde_json::Map::new();
                let mut def = "observe_only".to_string();
                // Map official domains → local lab site_ids via root_domains.
                let domain_to_local: std::collections::HashMap<String, String> = st
                    .rt
                    .admin
                    .db
                    .all_root_domains()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(domain, site_id, _en)| (domain.to_ascii_lowercase(), site_id))
                    .collect();
                for row in arr {
                    if row.get("ok") != Some(&json!(true)) {
                        continue;
                    }
                    let domain = row
                        .get("domain")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    let plan = row.get("plan").and_then(|x| x.as_str()).unwrap_or("legacy");
                    let ent = json!({
                        "plan": plan,
                        "rpa_enabled": true,
                        "device_precisions": ["dv0", "dv4", "dv5", "dv6"],
                        "domain": domain,
                        "source": "official_site",
                        "expires_at_ms": row.get("expires_at_ms").and_then(|x| x.as_i64()).unwrap_or(0),
                        "revoked": row.get("revoked").and_then(|x| x.as_bool()).unwrap_or(false),
                    });
                    if let Some(local_sid) = domain_to_local.get(&domain) {
                        site_map.insert(local_sid.clone(), json!("observe_only"));
                        ent_map.insert(local_sid.clone(), ent.clone());
                    } else if !domain.is_empty() {
                        // Auto-import official domains without a plan gate.
                        let local_id = row
                            .get("site_id")
                            .and_then(|x| x.as_str())
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| format!("off_{}", &domain.replace('.', "_")));
                        let rec = gr_admin::sites::SiteRecord {
                            site_id: local_id.clone(),
                            name: domain.clone(),
                            collect_enabled: true,
                            notes: "imported from legacy official_site sync".into(),
                            edge_mode: "dual_domain".into(),
                            pv_base: String::new(),
                            gv_base: String::new(),
                            fe_load: "pv".into(),
                            upload_ingest: "gv".into(),
                            poll_method: "both".into(),
                            entry_js: "gr.js".into(),
                            cookie_fields: Vec::new(),
                            embed_token: gr_admin::mint_embed_token(),
                            root_domains: vec![domain.clone()],
                            crypto: gr_admin::sites::CryptoProfile::default(),
                            // Official-site import is itself an owner-confirmed
                            // pairing (site created on the official console).
                            consent_confirmed: true,
                            consent_notice_version: "v1".into(),
                        };
                        if st.rt.admin.db.upsert_site(&rec).is_ok() {
                            site_map.insert(local_id.clone(), json!("observe_only"));
                            ent_map.insert(local_id, ent.clone());
                            sync_site_to_probe_admin(&st.rt.cfg.data_dir, &rec);
                        }
                    }
                    if !domain.is_empty() {
                        ent_map.insert(domain.clone(), ent.clone());
                    }
                    if let Some(sid) = row.get("site_id").and_then(|x| x.as_str()) {
                        if !sid.is_empty() {
                            site_map.insert(sid.to_string(), json!("observe_only"));
                            ent_map.insert(sid.to_string(), ent);
                        }
                    }
                    let _ = &def;
                }
                let sites_v = Value::Object(site_map);
                let _ = write_panel_policy_file(&st, Some((&def, &sites_v)), None, None, None);
                // Persist legacy metadata; current capabilities are ungated.
                {
                    let data_dir = st.rt.cfg.data_dir.clone();
                    let pol = gr_probe_core::load_panel_policy(Some(std::path::Path::new(&data_dir)));
                    let merged = gr_probe_core::merge_entitlements(pol, &Value::Object(ent_map));
                    let _ = gr_probe_core::save_panel_policy(Some(std::path::Path::new(&data_dir)), merged);
                }
                let cfg_store = json!({
                    "default_strategy_id": def,
                    "site_strategies": sites_v,
                    "source": "official_site",
                    "read_only": true,
                    "note": "User strategy catalog removed; legacy entitlement metadata is non-authoritative",
                });
                let _ = st
                    .rt
                    .admin
                    .db
                    .set_setting(SETTING_STRATEGY, &cfg_store.to_string());
            }
            Json(v).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok": false, "error": e})),
        )
            .into_response(),
    }
}

async fn admin_logout(State(st): State<AppState>, headers: HeaderMap) -> Response {
    let token = token_from_headers(&headers);
    let _ = st.rt.admin.auth.logout(&st.rt.admin.db, &token);
    let mut res = Json(json!({"ok": true})).into_response();
    append_session_cookie_headers(res.headers_mut(), &st.rt.admin.auth.console_path, "", true);
    res
}

fn authed(st: &AppState, headers: &HeaderMap) -> Result<String, Response> {
    let token = token_from_headers(headers);
    if token.is_empty() {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok": false, "error": "unauthorized"})),
        )
            .into_response());
    }
    match st.rt.admin.auth.check_session(&st.rt.admin.db, &token) {
        Ok(Some(u)) => Ok(u),
        _ => Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok": false, "error": "unauthorized"})),
        )
            .into_response()),
    }
}

async fn admin_status(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    let mut status = st.rt.status_json();
    if let Value::Object(ref mut m) = status {
        m.insert("probe_base".into(), json!(st.probe_base));
        m.insert("data_plane".into(), json!("in_tree_gr_probe_plane"));
    }
    Json(status).into_response()
}

async fn admin_metrics(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    Json(json!(metrics::sample())).into_response()
}

async fn list_sites(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    match st.rt.admin.db.list_sites() {
        Ok(v) => Json(json!({"ok": true, "sites": v})).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"ok":false,"error":e}))).into_response(),
    }
}

// --- Direct-bind edge: panel-managed domains + SSL (Pingora TLS SNI) ---

/// Probe-plane AdminDb — the domains/SSL tables live in the shared PG control
/// store the probe plane serves (same bridge pattern as `sdk_embed`).
fn probe_admin_db(st: &AppState) -> Result<gr_probe_plane::admin::db::AdminDb, Response> {
    let url = gr_abi::env::get("ADMIN_DATABASE_URL").unwrap_or_default();
    gr_probe_plane::admin::db::AdminDb::open_postgres(&url, st.rt.cfg.data_dir.clone())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"ok":false,"error":e}))).into_response())
}

async fn domains_list(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    let db = match probe_admin_db(&st) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let site_id = params.get("site_id").map(|s| s.trim()).filter(|s| !s.is_empty());
    let q = params.get("q").map(|s| s.trim()).filter(|s| !s.is_empty());
    match db.list_domains(site_id, q) {
        Ok(v) => Json(json!({"ok": true, "domains": v})).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"ok":false,"error":e}))).into_response(),
    }
}

#[derive(Deserialize)]
struct DomainUpsertBody {
    site_id: String,
    hostname: String,
    display_name: Option<String>,
    collect_enabled: Option<bool>,
    domain_id: Option<String>,
}

async fn domains_upsert(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<DomainUpsertBody>,
) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    let host = body.hostname.trim().to_ascii_lowercase();
    if host.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({"ok":false,"error":"hostname_required"}))).into_response();
    }
    let site_id = body.site_id.trim();
    if site_id.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({"ok":false,"error":"site_id_required"}))).into_response();
    }
    let display = body
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&host)
        .to_string();
    let db = match probe_admin_db(&st) {
        Ok(d) => d,
        Err(r) => return r,
    };
    match db.upsert_domain(
        body.domain_id.as_deref(),
        site_id,
        &host,
        &display,
        body.collect_enabled.unwrap_or(true),
    ) {
        Ok(row) => {
            let _ = db.audit("panel", "domain_upsert", &host, json!({"site_id": site_id}));
            Json(json!({"ok": true, "domain": row})).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"ok":false,"error":e}))).into_response(),
    }
}

async fn domain_ssl_mint(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(domain_id): Path<String>,
) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    let db = match probe_admin_db(&st) {
        Ok(d) => d,
        Err(r) => return r,
    };
    match gr_probe_plane::admin::ssl::mint_self_signed(&db, &domain_id) {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"ok":false,"error":e}))).into_response(),
    }
}

#[derive(Deserialize)]
struct DomainSslPemBody {
    cert_pem: String,
    key_pem: String,
    set_active: Option<bool>,
}

async fn domain_ssl_pem(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(domain_id): Path<String>,
    Json(body): Json<DomainSslPemBody>,
) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    if body.cert_pem.trim().is_empty() || body.key_pem.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({"ok":false,"error":"cert_and_key_required"}))).into_response();
    }
    let db = match probe_admin_db(&st) {
        Ok(d) => d,
        Err(r) => return r,
    };
    match gr_probe_plane::admin::ssl::save_pem(
        &db,
        &domain_id,
        &body.cert_pem,
        &body.key_pem,
        body.set_active.unwrap_or(true),
    ) {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"ok":false,"error":e}))).into_response(),
    }
}

#[derive(Deserialize)]
struct DomainSslAcmeBody {
    email: Option<String>,
    production: Option<bool>,
}

/// Request an ACME HTTP-01 issuance or renewal. The probe plane serves the
/// challenge from its public listener, then hot-reloads the resulting SNI
/// binding. Production issuance requires an explicit `production: true`.
async fn domain_ssl_acme(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(domain_id): Path<String>,
    Json(body): Json<DomainSslAcmeBody>,
) -> Response {
    let actor = match authed(&st, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let db = match probe_admin_db(&st) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let email = body.email.unwrap_or_default();
    let production = body.production.unwrap_or(false);
    match gr_probe_plane::admin::ssl::issue_letsencrypt(&db, &domain_id, &email, production).await {
        Ok(v) => {
            let _ = db.audit(
                &actor,
                "domain_ssl_acme",
                &domain_id,
                json!({"production": production}),
            );
            Json(v).into_response()
        }
        Err(e) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"ok": false, "error": e, "production": production})),
        )
            .into_response(),
    }
}

/// Apply desired runtime: hot-reload SNI bindings from the admin store and
/// flag bind/role changes that need a process restart (GR_ADMIN_RESTART_CMD).
/// Subset of probe-plane `admin::apply::apply_runtime` — the control plane
/// has no handle on the probe AppState (analyze workers keep live value).
async fn runtime_apply(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    let db = match probe_admin_db(&st) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let desired = match db.get_runtime_desired() {
        Ok(v) => v,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"ok":false,"error":e}))).into_response();
        }
    };
    let mut notes: Vec<String> = Vec::new();
    let mut needs_restart = false;

    // Hot: SNI map file + per-domain bindings (Pingora ClientHello callback)
    let sni_path = db.data_dir.join("sni-map.json");
    if sni_path.is_file() {
        match gr_probe_plane::sni_map::load_sni_map(&sni_path.to_string_lossy()) {
            Ok(n) => notes.push(format!("sni_reloaded n={n}")),
            Err(e) => notes.push(format!("sni_reload_err={e}")),
        }
    } else {
        notes.push("sni-map.json missing (mint/upload SSL for a domain first)".into());
    }
    if let Ok(domains) = db.list_domains(None, None) {
        for d in domains {
            let host = d.get("hostname").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let cert = d.get("cert_path").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let key = d.get("key_path").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if host.is_empty() || cert.is_empty() || key.is_empty() {
                continue;
            }
            match gr_probe_plane::sni_map::upsert_binding(
                &host,
                std::path::Path::new(&cert),
                std::path::Path::new(&key),
            ) {
                Ok(()) => notes.push(format!("sni_binding={host}")),
                Err(e) => notes.push(format!("sni_upsert_err {host}: {e}")),
            }
        }
    }

    // Binds / role / TLS material require process restart
    for k in [
        "bind",
        "bind_tls",
        "admin_bind",
        "role",
        "tls_cert",
        "tls_key",
        "quic_listen",
        "h3_listen",
    ] {
        if desired.get(k).is_some() {
            needs_restart = true;
            notes.push(format!("{k}_change_requires_restart"));
        }
    }

    let mut restart_ran = false;
    let mut restart_out = Value::Null;
    if needs_restart {
        let cmd = std::env::var("GR_ADMIN_RESTART_CMD")
            .or_else(|_| std::env::var("ADMIN_RESTART_CMD"))
            .unwrap_or_default();
        let cmd = cmd.trim();
        if cmd.is_empty() {
            notes.push("set GR_ADMIN_RESTART_CMD to auto-restart after bind/role changes".into());
        } else {
            match std::process::Command::new("sh").arg("-c").arg(cmd).output() {
                Ok(o) => {
                    restart_ran = true;
                    restart_out = json!({
                        "status": o.status.code(),
                        "stdout": String::from_utf8_lossy(&o.stdout),
                        "stderr": String::from_utf8_lossy(&o.stderr),
                    });
                    notes.push("restart_cmd_executed".into());
                }
                Err(e) => notes.push(format!("restart_cmd_err={e}")),
            }
        }
    }

    let _ = db.audit("panel", "runtime_apply", "", json!({"notes": notes.clone()}));
    Json(json!({
        "ok": true,
        "desired": desired,
        "notes": notes,
        "needs_restart": needs_restart,
        "restart_ran": restart_ran,
        "restart": restart_out,
        "sni_map_len": gr_probe_plane::sni_map::sni_map_len(),
    }))
    .into_response()
}

// --- P0: SDK key management — single admin plane; keys live in public.sdk_keys ---

#[derive(Deserialize)]
struct SdkKeyBody {
    site_id: String,
    kind: Option<String>,
    allowed_origins: Option<Vec<String>>,
}

fn validate_sdk_origins(kind: &str, origins: Option<Vec<String>>) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for raw in origins.unwrap_or_default() {
        let origin = raw.trim();
        if origin.is_empty() {
            continue;
        }
        if origin == "*" || !matches!(kind, "backend" | "fe_embed") {
            return Err("allowed_origin_invalid".into());
        }
        let parsed = reqwest::Url::parse(origin).map_err(|_| "allowed_origin_invalid")?;
        if !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none()
            || parsed.username() != ""
            || parsed.password().is_some()
            || parsed.path() != "/"
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err("allowed_origin_must_be_http_origin".into());
        }
        let normalized = format!(
            "{}://{}{}",
            parsed.scheme(),
            parsed.host_str().unwrap_or_default(),
            parsed
                .port()
                .map(|port| format!(":{port}"))
                .unwrap_or_default()
        );
        if !out.iter().any(|v| v.eq_ignore_ascii_case(&normalized)) {
            out.push(normalized);
        }
    }
    Ok(out)
}

/// List SDK keys (optionally filtered by site_id query param).
async fn sdk_keys_list(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    let site_id = params.get("site_id").map(|s| s.as_str());
    match st.rt.admin.db.list_sdk_keys(site_id) {
        Ok(keys) => Json(json!({"ok": true, "keys": keys})).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"ok":false,"error":e}))).into_response(),
    }
}

/// Mint a new SDK key for a site. Secret is returned exactly once.
async fn sdk_keys_create(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SdkKeyBody>,
) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    let kind = body.kind.unwrap_or_else(|| "backend".into());
    if !matches!(kind.as_str(), "backend" | "fe_embed") {
        return (StatusCode::BAD_REQUEST, Json(json!({"ok":false,"error":"kind must be backend|fe_embed"}))).into_response();
    }
    if body.site_id.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({"ok":false,"error":"site_id_required"}))).into_response();
    }
    let origins = match validate_sdk_origins(&kind, body.allowed_origins.clone()) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({"ok":false,"error":e}))).into_response(),
    };
    match st.rt.admin.db.site_exists(body.site_id.trim()) {
        Ok(true) => {}
        Ok(false) => return (StatusCode::BAD_REQUEST, Json(json!({"ok":false,"error":"site_not_found"}))).into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"ok":false,"error":e}))).into_response(),
    }
    let raw = gr_probe_plane::admin::sdk::mint_raw_secret();
    let secret_hash = gr_probe_plane::admin::sdk::hash_secret(&raw);
    let secret_prefix: String = raw.chars().take(12).collect();
    match st.rt.admin.db.insert_sdk_key(body.site_id.trim(), &kind, &secret_hash, &secret_prefix, &origins) {
        Ok(key) => Json(json!({
            "ok": true,
            "key": key,
            "secret": raw,
        })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"ok":false,"error":e}))).into_response(),
    }
}

/// Revoke a single SDK key by id.
async fn sdk_key_revoke(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    match st.rt.admin.db.revoke_sdk_key(&key_id) {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"ok":false,"error":e}))).into_response(),
    }
}

/// Rotate = revoke the old key then mint a fresh one for the same site.
async fn sdk_key_rotate(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
    Json(body): Json<SdkKeyBody>,
) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    let kind = body.kind.unwrap_or_else(|| "backend".into());
    if !matches!(kind.as_str(), "backend" | "fe_embed") {
        return (StatusCode::BAD_REQUEST, Json(json!({"ok":false,"error":"kind must be backend|fe_embed"}))).into_response();
    }
    if body.site_id.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({"ok":false,"error":"site_id_required"}))).into_response();
    }
    let origins = match validate_sdk_origins(&kind, body.allowed_origins.clone()) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({"ok":false,"error":e}))).into_response(),
    };
    match st.rt.admin.db.site_exists(body.site_id.trim()) {
        Ok(true) => {}
        Ok(false) => return (StatusCode::BAD_REQUEST, Json(json!({"ok":false,"error":"site_not_found"}))).into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"ok":false,"error":e}))).into_response(),
    }
    let _ = st.rt.admin.db.revoke_sdk_key(&key_id);
    let raw = gr_probe_plane::admin::sdk::mint_raw_secret();
    let secret_hash = gr_probe_plane::admin::sdk::hash_secret(&raw);
    let secret_prefix: String = raw.chars().take(12).collect();
    match st.rt.admin.db.insert_sdk_key(body.site_id.trim(), &kind, &secret_hash, &secret_prefix, &origins) {
        Ok(key) => Json(json!({
            "ok": true,
            "key": key,
            "secret": raw,
        })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"ok":false,"error":e}))).into_response(),
    }
}

/// Build FE embed snippet (proxied from the probe plane).
async fn sdk_embed(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    let site_id = params.get("site_id").cloned().unwrap_or_default();
    let public_base = params.get("public_base").cloned().unwrap_or_default();
    let gw_base = params.get("gw_base").cloned();
    let mode = params.get("mode").cloned();
    // Probe-plane AdminDb reads the same shared PG database (public schema:
    // sites/domains/sdk_keys) that sync_site_to_probe_admin_pg writes.
    let url = gr_abi::env::get("ADMIN_DATABASE_URL").unwrap_or_default();
    let pdb = match gr_probe_plane::admin::db::AdminDb::open_postgres(&url, st.rt.cfg.data_dir.clone()) {
        Ok(d) => d,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"ok":false,"error":e}))).into_response(),
    };
    match gr_probe_plane::admin::sdk::embed_snippet(&pdb, &site_id, &public_base, gw_base.as_deref(), mode.as_deref()) {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"ok":false,"error":e}))).into_response(),
    }
}


async fn upsert_site(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(mut site): Json<SiteRecord>,
) -> Response {
    let actor = match authed(&st, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let is_new = !st.rt.admin.db.site_exists(&site.site_id).unwrap_or(false);
    site.fe_load = gr_admin::normalize_fe_load(&site.fe_load);
    site.upload_ingest = gr_admin::normalize_upload_ingest(&site.upload_ingest);
    if site.fe_load == "pv" || site.upload_ingest == "gv" {
        if site.edge_mode.trim().is_empty() || site.edge_mode == "first_party" {
            site.edge_mode = "dual_domain".into();
        }
    }
    if let Some(code) = gr_admin::reject_proxied_upload(&site.upload_ingest) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "error": code,
                "hint": "upload_ingest=first_party proxies probe upload and breaks Pingora TLS evidence. Set upload_ingest=gv (or pv) for bound-domain direct, or GR_ALLOW_FIRST_PARTY_UPLOAD=1 to override."
            })),
        )
            .into_response();
    }
    if is_new {
        // iss/opus5 02-§5: creation requires the owner's disclosure confirmation.
        if !site.consent_confirmed {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"ok": false, "error": "consent_required",
                            "hint": "探测行为必须已告知访客；网站主确认后才可创建站点 (consent_confirmed=true)"})),
            )
                .into_response();
        }
        if site.embed_token.trim().is_empty() {
            site.embed_token = gr_admin::mint_embed_token();
        }
    } else if site.embed_token.trim().is_empty() {
        if let Ok(Some(existing)) = st.rt.admin.db.get_embed_token(&site.site_id) {
            site.embed_token = existing;
        }
    }
    if let Err(e) = st.rt.admin.db.upsert_site(&site) {
        return (StatusCode::BAD_REQUEST, Json(json!({"ok":false,"error":e}))).into_response();
    }
    // Keep probe-plane Host→site_id in sync (multi-tenant domain table).
    sync_site_to_probe_admin(&st.rt.cfg.data_dir, &site);
    let _ = st.rt.admin.db.audit(
        &actor,
        "site_upsert",
        &site.site_id,
        json!({"roots": site.root_domains, "probe_domain_sync": true}),
    );
    let _ = st.rt.refresh_live_config();
    Json(json!({
        "ok": true,
        "site_id": site.site_id,
        "embed_token": site.embed_token,
        "fe_load": site.fe_load,
        "upload_ingest": site.upload_ingest,
        "probe_domain_sync": true
    })).into_response()
}

async fn rotate_embed_token(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(site_id): Path<String>,
) -> Response {
    let actor = match authed(&st, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    match st.rt.admin.db.rotate_embed_token(&site_id) {
        Ok(token) => {
            if let Ok(Some(mut rec)) = st.rt.admin.db.list_sites().map(|rows| {
                rows.into_iter().find(|s| {
                    s.get("site_id").and_then(|v| v.as_str()) == Some(site_id.as_str())
                })
            }) {
                if let Some(obj) = rec.as_object_mut() {
                    obj.insert("embed_token".into(), json!(token.clone()));
                }
                if let Ok(site) = serde_json::from_value::<SiteRecord>(rec) {
                    sync_site_to_probe_admin(&st.rt.cfg.data_dir, &site);
                } else {
                    let tok = token.clone();
                    let sid = site_id.clone();
                    let _ = std::thread::Builder::new()
                        .name("gr-embed-sync".into())
                        .spawn(move || {
                            let url = gr_abi::env::get("ADMIN_DATABASE_URL").unwrap_or_default();
                            if url.trim().is_empty() {
                                return;
                            }
                            if let Ok(mut c) = postgres::Client::connect(&url, postgres::NoTls) {
                                let _ = c.execute(
                                    "UPDATE sites SET embed_token=$1 WHERE site_id=$2",
                                    &[&tok, &sid],
                                );
                            }
                        });
                }
            }
            let _ = st.rt.admin.db.audit(
                &actor,
                "embed_token_rotate",
                &site_id,
                json!({"rotated": true}),
            );
            Json(json!({"ok": true, "site_id": site_id, "embed_token": token})).into_response()
        }
        Err(e) if e == "site_not_found" => {
            (StatusCode::NOT_FOUND, Json(json!({"ok":false,"error":e}))).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"ok":false,"error":e}))).into_response(),
    }
}

async fn get_workers(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    match st.rt.admin.db.get_workers() {
        Ok((a, i, g)) => Json(json!({"ok":true,"analyze":a,"ingest":i,"gateway":g})).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"ok":false,"error":e}))).into_response(),
    }
}

#[derive(Deserialize)]
struct WorkersBody {
    analyze: Option<u32>,
    ingest: Option<u32>,
    gateway: Option<u32>,
}

async fn set_workers(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<WorkersBody>,
) -> Response {
    let actor = match authed(&st, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let (a0, i0, g0) = st.rt.admin.db.get_workers().unwrap_or((1, 1, 1));
    let a = body.analyze.unwrap_or(a0);
    let i = body.ingest.unwrap_or(i0);
    let g = body.gateway.unwrap_or(g0);
    if let Err(e) = st.rt.set_workers(a, i, g) {
        return (StatusCode::BAD_REQUEST, Json(json!({"ok":false,"error":e}))).into_response();
    }
    let _ = st.rt.admin.db.audit(
        &actor,
        "workers_hot_set",
        "",
        json!({"analyze": a, "ingest": i, "gateway": g}),
    );
    // analyze resizes live (probe-plane supervisor reconciles the target);
    // ingest/gateway are cluster/orchestration hints, no local restart either.
    Json(json!({
        "ok":true,"analyze":a,"ingest":i,"gateway":g,"restart_required":false,
        "hot":{"analyze":true},
        "note":"analyze workers resized live; ingest/gateway propagate via cluster desired state"
    }))
    .into_response()
}

async fn list_modules(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    let db = st.rt.admin.db.list_modules().unwrap_or_default();
    let live = st.rt.registry.list_status();
    Json(json!({"ok":true,"db":db,"loaded":live})).into_response()
}

async fn cluster_nodes(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    Json(json!({"ok":true,"nodes": st.rt.cluster.snapshot()})).into_response()
}

#[derive(Deserialize)]
struct HbBody {
    key: String,
    info: gr_cluster::NodeInfo,
    /// Signed heartbeat timestamp from `ClusterHub::sign_heartbeat` (ms epoch).
    #[serde(default)]
    ts_ms: i64,
    /// Hex HMAC-SHA256 from `ClusterHub::sign_heartbeat` over `node_id|ts|payload`.
    #[serde(default)]
    sig: String,
}

async fn cluster_heartbeat(State(st): State<AppState>, Json(body): Json<HbBody>) -> Response {
    if st.rt.cluster.ingest_peer(&body.key, body.info, body.ts_ms, &body.sig) {
        Json(json!({"ok": true})).into_response()
    } else {
        (StatusCode::FORBIDDEN, Json(json!({"ok": false, "error": "auth"}))).into_response()
    }
}

// --- Multi-node load balancer (docs/guides/08-LB-MODULE.md) — ungated: ships with every build ---

async fn lb_status(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    // lb_config_store uses the blocking `postgres` client; never call it on an
    // async worker (sync connect builds a nested runtime → panic).
    let persisted = match tokio::task::spawn_blocking(crate::lb_config_store::read_lb_config).await {
        Ok(Ok(Some(cfg))) => cfg,
        _ => gr_lb::LbConfig::default(),
    };
    Json(json!({
        "ok": true,
        "entitled": true,
        "license_state": "ungated",
        "allowed_modes": ["proxy", "redirect", "internal_ip"],
        "config": persisted,
        "applied": st.lb_pool.status_view(),
        "mode_hint": "gateway node env GR_PROBE_MODE=lb (docs/guides/08-LB-MODULE.md)",
    }))
    .into_response()
}

async fn lb_config_get(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    // Blocking postgres client — see lb_status for why this must leave the async worker.
    let persisted = match tokio::task::spawn_blocking(crate::lb_config_store::read_lb_config).await {
        Ok(Ok(Some(cfg))) => cfg,
        _ => gr_lb::LbConfig::default(),
    };
    Json(json!({
        "ok": true,
        "entitled": true,
        "license_state": "ungated",
        "allowed_modes": ["proxy", "redirect", "internal_ip"],
        "config": persisted,
    }))
    .into_response()
}

async fn lb_config_put(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let user = match authed(&st, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let cfg: gr_lb::LbConfig = match serde_json::from_value(body.clone()) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"ok": false, "error": format!("bad_lb_config: {e}")})),
            )
                .into_response()
        }
    };
    if let Err(e) = cfg.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok": false, "error": format!("invalid_lb_config: {e}")})),
        )
            .into_response();
    }
    // Blocking postgres client — run it off the async worker (spawn_blocking).
    let write = tokio::task::spawn_blocking({
        let cfg = cfg.clone();
        move || crate::lb_config_store::write_lb_config(&cfg)
    })
    .await;
    let write_err = match write {
        Ok(Ok(())) => None,
        Ok(Err(e)) => Some(e),
        Err(e) => Some(e.to_string()),
    };
    if let Some(e) = write_err {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok": false, "error": format!("persist_lb_config: {e}")})),
        )
            .into_response();
    }
    // Apply immediately (control loop also re-applies from PG every 5s).
    st.lb_pool.set_config(cfg.clone());
    let _ = st.rt.admin.db.audit(
        &user,
        "lb.config.update",
        "lb",
        json!({
            "enabled": cfg.enabled,
            "mode": cfg.mode,
            "strategy": cfg.strategy,
            "sticky_cookie": cfg.sticky_cookie,
        }),
    );
    Json(json!({"ok": true, "config": cfg})).into_response()
}

#[derive(Deserialize)]
struct DashQuery {
    range: Option<String>,
}

async fn dashboard(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<DashQuery>,
) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    let range = q.range.unwrap_or_else(|| "1h".into());
    let nodes = st.rt.cluster.snapshot();
    Json(json!({
        "ok": true,
        "range": range,
        "node": {
            "metrics": metrics::sample(),
            "modules": st.rt.registry.list_status(),
            "probe_base": st.probe_base,
        },
        "cluster": { "nodes": nodes },
        "ranges_supported": ["1m","5m","1h","6h","1d","7d"],
    }))
    .into_response()
}

async fn admin_me(State(st): State<AppState>, headers: HeaderMap) -> Response {
    match authed(&st, &headers) {
        Ok(u) => Json(json!({
            "ok": true,
            "username": u,
            "console_path": format!("/{}/", st.rt.admin.auth.console_path),
            "product": "greenpng",
            "version": env!("CARGO_PKG_VERSION"),
        }))
        .into_response(),
        Err(r) => r,
    }
}

/// Security posture snapshot for the control plane (no secrets).
async fn admin_security(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    let sites = st.rt.admin.db.list_sites().unwrap_or_default();
    let deploy = gr_abi::env::get("DEPLOY_ENV")
         .or_else(|| gr_abi::env::get("DEPLOY_ENV"))
        .unwrap_or_else(|| "lab".into());
    let cors = gr_abi::env::get("CORS_ORIGINS")
         .or_else(|| gr_abi::env::get("CORS_ORIGINS"))
        .unwrap_or_default();
    Json(json!({
        "ok": true,
        "console_path_random": !st.rt.admin.auth.console_path.is_empty()
            && !st.rt.admin.auth.console_path.contains("admin"),
        "single_admin_user": true,
        "password_hash": "scrypt",
        "session_ttl_hours": 12,
        "login_rate_limit": {"max_fail": 8, "window_ms": 900_000},
        "ota_pubkey_configured": !st.rt.cfg.pubkey.is_empty(),
        "release_base_url_set": !st.rt.get_release_base_url().is_empty(),
        "release_base_url": st.rt.get_release_base_url(),
        "signed_module_install_required": !st.rt.cfg.pubkey.is_empty(),
        "panel_ota": {
            "set_release_url": true,
            "install_modules": true,
            "install_fe": true,
            "install_runtime": true,
            "full_upgrade": true,
            "hot": ["modules", "fe", "workers", "sites", "release_url"],
            "restart_required": ["runtime_binary"],
        },
        "sites_configured": sites.len(),
        "domain_gate_active": !sites.is_empty(),
        "deploy_env": deploy,
        "cors_open_star": cors.trim() == "*",
        "security_headers": ["X-Content-Type-Options","X-Frame-Options","Referrer-Policy","CSP(admin SPA)","Cache-Control:no-store(API)"],
        "recommendations": [
            if cors.trim() == "*" { "production: set GR_CORS_ORIGINS to business origins only (not *)" } else { "cors locked down" },
            if st.rt.cfg.pubkey.is_empty() { "configure --pubkey-path for signed OTA" } else { "ota pubkey configured" },
            "keep admin_bootstrap_once.txt mode 0600 and off-box after first login",
            "do not expose control-plane port publicly without reverse-proxy TLS + IP allowlist",
            "prefer Bearer tokens short-lived; rotate password after bootstrap",
        ],
    }))
    .into_response()
}

#[derive(Deserialize)]
struct DeleteSiteBody {
    site_id: String,
}

async fn delete_site_body(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<DeleteSiteBody>,
) -> Response {
    let actor = match authed(&st, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let site_id = body.site_id.trim().to_string();
    if site_id.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({"ok": false, "error": "site_id required"})))
            .into_response();
    }
    if let Err(e) = st.rt.admin.db.delete_site(&site_id) {
        return (StatusCode::BAD_REQUEST, Json(json!({"ok": false, "error": e}))).into_response();
    }
    let _ = st.rt.admin.db.audit(&actor, "site_delete", &site_id, json!({}));
    let _ = st.rt.refresh_live_config();
    Json(json!({"ok": true, "site_id": site_id})).into_response()
}

#[derive(Deserialize)]
struct ActivateBody {
    name: String,
    version: String,
}

async fn modules_activate(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ActivateBody>,
) -> Response {
    let actor = match authed(&st, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    match st.rt.ota_activate(&body.name, &body.version) {
        Ok(mut v) => {
            if body.name == "analyze" || body.name == "all" {
                crate::mint_wire::wire_analyze_mint(&st.rt);
                if let Some(obj) = v.as_object_mut() {
                    obj.insert(
                        "mint_wire".into(),
                        crate::mint_wire::mint_wire_status(&st.rt),
                    );
                }
            }
            let _ = st.rt.admin.db.audit(
                &actor,
                "module_activate",
                &body.name,
                json!({"version": body.version}),
            );
            Json(v).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"ok": false, "error": e}))).into_response(),
    }
}

#[derive(Deserialize)]
struct StageBody {
    name: String,
    version: String,
    #[serde(default)]
    domain: String,
    path: String,
}

async fn modules_stage(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<StageBody>,
) -> Response {
    let actor = match authed(&st, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let path = std::path::PathBuf::from(&body.path);
    match st
        .rt
        .ota_stage_local(&body.name, &body.version, &body.domain, &path)
    {
        Ok(dest) => {
            let _ = st.rt.admin.db.audit(
                &actor,
                "module_stage",
                &body.name,
                json!({"version": body.version, "dest": dest}),
            );
            Json(json!({
                "ok": true,
                "name": body.name,
                "version": body.version,
                "path": dest,
            }))
            .into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"ok": false, "error": e}))).into_response(),
    }
}

async fn ota_local(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    let local = st.rt.ota_list_local().unwrap_or_default();
    let db = st.rt.admin.db.list_modules().unwrap_or_default();
    let loaded = st.rt.registry.list_status();
    Json(json!({
        "ok": true,
        "local": local,
        "db": db,
        "loaded": loaded,
        "modules_dir": st.rt.cfg.modules_dir,
        "release_base_url": st.rt.get_release_base_url(),
        "pubkey_configured": !st.rt.cfg.pubkey.is_empty(),
        "static_dir": st.static_dir,
        "install_root": st.install_root,
        "control_version": gr_runtime::PRODUCT_VERSION,
        "capabilities": {
            "set_release_url": true,
            "install_modules": true,
            "install_fe": true,
            "install_runtime": true,
            "full_upgrade": true,
            "cluster_apply": true,
            "hot_no_restart": ["modules", "fe", "workers", "sites", "release_url", "cluster_ota"],
            "restart_required": ["runtime_binary"],
        },
    }))
    .into_response()
}

async fn ota_remote(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    let rt = st.rt.clone();
    let res = tokio::task::spawn_blocking(move || rt.ota_fetch_manifest()).await;
    match res {
        Ok(Ok(m)) => Json(json!({
            "ok": true,
            "manifest": m,
            "release_base_url": st.rt.get_release_base_url(),
        }))
        .into_response(),
        Ok(Err(e)) => Json(json!({
            "ok": false,
            "error": e,
            "release_base_url": st.rt.get_release_base_url(),
            "hint": "POST ota/set-release-url with GitHub release download base, e.g. https://github.com/org/repo/releases/download/v6.0.x",
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok": false, "error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct SetReleaseUrlBody {
    url: String,
}

async fn ota_set_release_url(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SetReleaseUrlBody>,
) -> Response {
    let actor = match authed(&st, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    match st.rt.set_release_base_url(&body.url) {
        Ok(()) => {
            let url = st.rt.get_release_base_url();
            let url2 = url.clone();
            let cluster = tokio::task::spawn_blocking(move || crate::cluster_ota_store::write_desired(&url2, true))
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
            let cluster = cluster
                .map(|d| json!({"version": d.version, "written_ms": d.written_ms}))
                .unwrap_or_else(|e| json!({"error": e}));
            let _ = st.rt.admin.db.audit(
                &actor,
                "ota_set_release_url",
                "",
                json!({"url": url, "cluster": cluster}),
            );
            Json(json!({
                "ok": true,
                "release_base_url": url,
                "cluster_ota": cluster,
                "restart_required": false,
                "note": "hot; persisted to data/ota_release_url.txt, shared admin PG cluster desired, and best-effort .env",
            }))
            .into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"ok": false, "error": e}))).into_response(),
    }
}

fn current_analyze_version(rt: &Runtime) -> Option<String> {
    rt.registry.get("analyze").map(|m| m.version().to_string())
}

fn apply_github_analyze(rt: &Arc<Runtime>, url: &str, activate: bool) -> Result<Value, String> {
    let desired = crate::cluster_ota_store::write_desired(url, activate)?;
    rt.set_release_base_url(&desired.release_url)?;
    let analyze = current_analyze_version(rt);
    if !gr_runtime::cluster_ota::should_apply_cluster_ota(
        &rt.get_release_base_url(),
        analyze.as_deref(),
        &desired,
    ) {
        return Ok(json!({
            "ok": true,
            "applied": false,
            "already": true,
            "desired": desired,
            "analyze": analyze,
        }));
    }
    let installed = rt.ota_install_remote("analyze", Some(&desired.version), activate)?;
    crate::mint_wire::wire_analyze_mint(rt);
    Ok(json!({
        "ok": true,
        "applied": true,
        "desired": desired,
        "install": installed,
        "mint_wire": crate::mint_wire::mint_wire_status(rt),
    }))
}

/// Serve GitHub-published assets already on this node so docker lab nodes
/// without egress can still run the same panel OTA against the GitHub tag.
async fn ota_mirror_asset(
    State(st): State<AppState>,
    Path((ver, asset)): Path<(String, String)>,
) -> Response {
    let ver = ver.trim().trim_start_matches('v');
    if ver.is_empty() || asset.contains("..") || asset.contains('/') || asset.contains('\\') {
        return (StatusCode::BAD_REQUEST, Json(json!({"ok": false, "error": "bad_asset"}))).into_response();
    }
    let mut candidates: Vec<PathBuf> = vec![
        st.rt.cfg.data_dir.join("ota_mirror").join(ver).join(&asset),
        st.install_root.join("dist").join(format!("release-{ver}")).join(&asset),
        PathBuf::from("dist").join(format!("release-{ver}")).join(&asset),
    ];
    // 整包安装过的节点: bundle 缓存树里直接取模块 (集群 OTA 无需手工预镜像 .so)
    if asset.ends_with(".so") {
        if let Ok(rd) = std::fs::read_dir(st.rt.cfg.data_dir.join("ota_staging").join("bundle")) {
            for entry in rd.flatten() {
                let cand = entry.path().join("modules").join(&asset);
                if cand.is_file() {
                    candidates.push(cand);
                    break;
                }
            }
        }
    }
    for p in candidates {
        if p.is_file() {
            match std::fs::read(&p) {
                Ok(bytes) => {
                    let ctype = if asset.ends_with(".json") {
                        "application/json"
                    } else if asset.ends_with(".so") {
                        "application/octet-stream"
                    } else {
                        "application/octet-stream"
                    };
                    return Response::builder()
                        .status(200)
                        .header(header::CONTENT_TYPE, ctype)
                        .header(header::CACHE_CONTROL, "public, max-age=60")
                        .body(Body::from(bytes))
                        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
                }
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({"ok": false, "error": e.to_string()})),
                    )
                        .into_response();
                }
            }
        }
    }
    (
        StatusCode::NOT_FOUND,
        Json(json!({"ok": false, "error": "not_on_this_node", "version": ver, "asset": asset})),
    )
        .into_response()
}

async fn ota_cluster_desired(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    let av = current_analyze_version(&st.rt);
    let cur = st.rt.get_release_base_url();
    let av2 = av.clone();
    let status = tokio::task::spawn_blocking(move || {
        crate::cluster_ota_store::desired_status_json(&cur, av2.as_deref())
    })
    .await
    .unwrap_or_else(|e| json!({"ok": false, "error": e.to_string()}));
    Json(status).into_response()
}

#[derive(Deserialize)]
struct ClusterApplyBody {
    #[serde(default)]
    release_url: Option<String>,
    #[serde(default = "default_true_bool")]
    activate: bool,
}

async fn ota_cluster_apply_authed(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ClusterApplyBody>,
) -> Response {
    let actor = match authed(&st, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let url = body
        .release_url
        .unwrap_or_else(|| st.rt.get_release_base_url());
    let rt = st.rt.clone();
    let activate = body.activate;
    let res = tokio::task::spawn_blocking(move || apply_github_analyze(&rt, &url, activate)).await;
    match res {
        Ok(Ok(v)) => {
            let _ = st.rt.admin.db.audit(&actor, "ota_cluster_apply", "", v.clone());
            Json(v).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(json!({"ok": false, "error": e}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok": false, "error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct ClusterApplyKeyBody {
    key: String,
    #[serde(default)]
    release_url: Option<String>,
    #[serde(default = "default_true_bool")]
    activate: bool,
}

/// Key-authed peer heartbeat ingestion (multi-node LB discovery, docs/08).
/// Peers POST `sign_heartbeat()` output to `/v1/cluster/heartbeat`; the
/// receiver verifies cluster key + HMAC + clock window (see ClusterHub).
async fn cluster_heartbeat_key(
    State(st): State<AppState>,
    Json(body): Json<HbBody>,
) -> Response {
    if st.rt.cluster.ingest_peer(&body.key, body.info, body.ts_ms, &body.sig) {
        Json(json!({"ok": true})).into_response()
    } else {
        (StatusCode::FORBIDDEN, Json(json!({"ok": false, "error": "auth"}))).into_response()
    }
}

async fn cluster_ota_apply_key(
    State(st): State<AppState>,
    Json(body): Json<ClusterApplyKeyBody>,
) -> Response {
    if !st.rt.cluster.auth_ok(&body.key) {
        return (StatusCode::FORBIDDEN, Json(json!({"ok": false, "error": "auth"}))).into_response();
    }
    let url = body
        .release_url
        .unwrap_or_else(|| st.rt.get_release_base_url());
    let rt = st.rt.clone();
    let activate = body.activate;
    let res = tokio::task::spawn_blocking(move || apply_github_analyze(&rt, &url, activate)).await;
    match res {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(json!({"ok": false, "error": e}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok": false, "error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn ota_install_fe(
    State(st): State<AppState>,
    headers: HeaderMap,
) -> Response {
    let actor = match authed(&st, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let rt = st.rt.clone();
    let static_dir = st.static_dir.clone();
    let res = tokio::task::spawn_blocking(move || rt.ota_install_fe(&static_dir)).await;
    match res {
        Ok(Ok(v)) => {
            let _ = st.rt.admin.db.audit(&actor, "ota_install_fe", "", json!(&v));
            Json(v).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(json!({"ok": false, "error": e}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok": false, "error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct OtaInstallRuntimeBody {
    /// When true, attempt `systemctl restart greenpng` after binary swap.
    #[serde(default = "default_true_bool")]
    restart: bool,
}

async fn ota_install_runtime(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<OtaInstallRuntimeBody>,
) -> Response {
    let actor = match authed(&st, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let rt = st.rt.clone();
    let install_root = st.install_root.clone();
    let restart = body.restart;
    let res =
        tokio::task::spawn_blocking(move || rt.ota_install_runtime(restart, &install_root)).await;
    match res {
        Ok(Ok(v)) => {
            let _ = st.rt.admin.db.audit(
                &actor,
                "ota_install_runtime",
                "",
                json!({"restart": restart, "result": &v}),
            );
            Json(v).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(json!({"ok": false, "error": e}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok": false, "error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct OtaFullUpgradeBody {
    /// Optional: hot-set release URL before install (e.g. …/download/v6.0.8).
    #[serde(default)]
    release_url: Option<String>,
    #[serde(default = "default_true_bool")]
    install_modules: bool,
    #[serde(default = "default_true_bool")]
    install_fe: bool,
    #[serde(default = "default_true_bool")]
    install_runtime: bool,
    /// Runtime binary restart after swap (default true).
    #[serde(default = "default_true_bool")]
    restart_runtime: bool,
    #[serde(default = "default_true_bool")]
    activate: bool,
}

async fn ota_full_upgrade(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<OtaFullUpgradeBody>,
) -> Response {
    let actor = match authed(&st, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    if let Some(ref url) = body.release_url {
        if let Err(e) = st.rt.set_release_base_url(url) {
            return (StatusCode::BAD_REQUEST, Json(json!({"ok": false, "error": e}))).into_response();
        }
        let url_pg = url.clone();
        let act = body.activate;
        let _ = tokio::task::spawn_blocking(move || crate::cluster_ota_store::write_desired(&url_pg, act)).await;
    }
    let rt = st.rt.clone();
    let static_dir = st.static_dir.clone();
    let install_root = st.install_root.clone();
    let install_modules = body.install_modules;
    let install_fe = body.install_fe;
    let install_runtime = body.install_runtime;
    let restart_runtime = body.restart_runtime;
    let activate = body.activate;
    let res = tokio::task::spawn_blocking(move || {
        rt.ota_full_upgrade(
            &static_dir,
            &install_root,
            install_modules,
            install_fe,
            install_runtime,
            restart_runtime,
            activate,
        )
    })
    .await;
    match res {
        Ok(Ok(mut v)) => {
            if body.activate && body.install_modules {
                crate::mint_wire::wire_analyze_mint(&st.rt);
                if let Some(obj) = v.as_object_mut() {
                    obj.insert(
                        "mint_wire".into(),
                        crate::mint_wire::mint_wire_status(&st.rt),
                    );
                }
            }
            let _ = st.rt.admin.db.audit(
                &actor,
                "ota_full_upgrade",
                "",
                json!({
                    "install_modules": body.install_modules,
                    "install_fe": body.install_fe,
                    "install_runtime": body.install_runtime,
                    "restart_runtime": body.restart_runtime,
                }),
            );
            Json(v).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(json!({"ok": false, "error": e}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok": false, "error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct OtaInstallBody {
    name: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default = "default_true_bool")]
    activate: bool,
}

fn default_true_bool() -> bool {
    true
}

async fn ota_install(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<OtaInstallBody>,
) -> Response {
    let actor = match authed(&st, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let rt = st.rt.clone();
    let name = body.name.clone();
    let version = body.version.clone();
    let activate = body.activate;
    let res = tokio::task::spawn_blocking(move || {
        rt.ota_install_remote(&name, version.as_deref(), activate)
    })
    .await;
    match res {
        Ok(Ok(mut v)) => {
            if body.activate && (body.name == "analyze" || body.name == "all") {
                crate::mint_wire::wire_analyze_mint(&st.rt);
                if let Some(obj) = v.as_object_mut() {
                    obj.insert(
                        "mint_wire".into(),
                        crate::mint_wire::mint_wire_status(&st.rt),
                    );
                }
            }
            let _ = st.rt.admin.db.audit(
                &actor,
                "ota_install",
                &body.name,
                json!({"version": body.version, "activate": body.activate}),
            );
            Json(v).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(json!({"ok": false, "error": e}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok": false, "error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct OtaInstallAllBody {
    #[serde(default = "default_true_bool")]
    activate: bool,
}

async fn ota_install_all(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<OtaInstallAllBody>,
) -> Response {
    let actor = match authed(&st, &headers) {
        Ok(u) => u,
        Err(r) => return r,
    };
    let rt = st.rt.clone();
    let activate = body.activate;
    let res = tokio::task::spawn_blocking(move || rt.ota_install_all_remote(activate)).await;
    match res {
        Ok(Ok(mut v)) => {
            if body.activate {
                crate::mint_wire::wire_analyze_mint(&st.rt);
                if let Some(obj) = v.as_object_mut() {
                    obj.insert(
                        "mint_wire".into(),
                        crate::mint_wire::mint_wire_status(&st.rt),
                    );
                }
            }
            let _ = st.rt.admin.db.audit(
                &actor,
                "ota_install_all",
                "",
                json!({"activate": body.activate}),
            );
            Json(v).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(json!({"ok": false, "error": e}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok": false, "error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn list_audit(State(st): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = authed(&st, &headers) {
        return r;
    }
    match st.rt.admin.db.list_audit(100) {
        Ok(v) => Json(json!({"ok": true, "items": v})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok": false, "error": e})),
        )
            .into_response(),
    }
}

async fn fallback_console_html(State(st): State<AppState>) -> Response {
    let path = &st.rt.admin.auth.console_path;
    let html = format!(
        r#"<!doctype html><html><head><meta charset=utf-8><title>gr</title></head>
<body style="font-family:system-ui;background:#0b1220;color:#e7eefc;padding:2rem">
<h1>greenpng</h1>
<p>Console <code>/{path}/</code> — build admin-spa for full UI.</p>
<p>Probe plane: <code>{}</code></p>
</body></html>"#,
        st.probe_base
    );
    Response::builder()
        .status(200)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(html))
        .unwrap()
}

async fn fallback_404(req: Request<Body>) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({"ok":false,"error":"not_found","path": req.uri().path()})),
    )
        .into_response()
}
