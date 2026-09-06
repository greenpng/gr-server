//! Full HTTP/3 application server (Quinn + h3).
//!
//! Serves a subset of green-v5 API over real H3 so browsers/clients complete
//! the H3 handshake and SETTINGS/request exchange. Side-channel samples feed
//! gateway → brain/product (h3_app_* fields).

use crate::handlers::{self, AppState, GatewayEarlyBody, OpenBody};
use crate::listen::side_store::{
    inject_h3_app_headers_from_info, push_h3_app, H3AppInfo,
};
use bytes::Bytes;
use http::{Method, Request, Response, StatusCode};
use log::{info, warn};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// Spawn HTTP/3 server on `addr` with TLS cert/key (same PEM as Pingora TLS).
pub fn spawn_h3_server(
    addr: SocketAddr,
    cert: impl AsRef<Path>,
    key: impl AsRef<Path>,
    state: Arc<AppState>,
) -> Result<tokio::task::JoinHandle<()>, String> {
    let certs = load_certs(cert.as_ref())?;
    let key = load_private_key(key.as_ref())?;

    let mut tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("rustls cert: {e}"))?;
    tls.alpn_protocols = vec![b"h3".to_vec()];
    // iss/46 G0: disable 0-RTT early data — state-changing APIs (open/ingest)
    // must not be replayable via early data. Reject/ignore early data path.
    tls.max_early_data_size = 0;

    let quic_cfg = quinn::crypto::rustls::QuicServerConfig::try_from(tls)
        .map_err(|e| format!("QuicServerConfig: {e}"))?;
    let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(quic_cfg));
    let mut transport = quinn::TransportConfig::default();
    transport.max_idle_timeout(Some(Duration::from_secs(30).try_into().unwrap()));
    transport.keep_alive_interval(Some(Duration::from_secs(10)));
    server_config.transport = Arc::new(transport);

    let endpoint = quinn::Endpoint::server(server_config, addr)
        .map_err(|e| format!("h3 endpoint bind {addr}: {e}"))?;
    info!("HTTP/3 application server on quic://{addr} (ALPN h3)");

    let handle = tokio::spawn(async move {
        loop {
            let incoming = match endpoint.accept().await {
                Some(i) => i,
                None => break,
            };
            let st = state.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_connection(incoming, st).await {
                    warn!("h3 connection: {e}");
                }
            });
        }
    });
    Ok(handle)
}

async fn handle_connection(
    incoming: quinn::Incoming,
    state: Arc<AppState>,
) -> Result<(), String> {
    let conn = incoming
        .await
        .map_err(|e| format!("accept handshake: {e}"))?;
    let peer = conn.remote_address();
    let rtt_ms = conn.rtt().as_secs_f64() * 1000.0;
    let stats = conn.stats();
    let h3_conn = h3_quinn::Connection::new(conn);
    let mut h3 = h3::server::builder()
        .build(h3_conn)
        .await
        .map_err(|e| format!("h3 build: {e}"))?;

    // First request samples app-layer fingerprint (this peer's sample only).
    let mut this_conn_h3: Option<H3AppInfo> = None;
    loop {
        match h3.accept().await {
            Ok(Some(resolver)) => {
                let (req, mut stream) = resolver
                    .resolve_request()
                    .await
                    .map_err(|e| format!("resolve: {e}"))?;
                if this_conn_h3.is_none() {
                    this_conn_h3 = Some(record_h3_app(&req, peer, rtt_ms, &stats));
                }
                let (status, body) =
                    dispatch_h3(&state, &req, peer, this_conn_h3.as_ref()).await;
                let resp = Response::builder()
                    .status(status)
                    .header("content-type", "application/json; charset=utf-8")
                    .header("cache-control", "no-store")
                    .header("alt-svc", format!("h3=\":{}\"; ma=86400", peer.port())) // best-effort
                    .body(())
                    .unwrap();
                if let Err(e) = stream.send_response(resp).await {
                    warn!("h3 send_response: {e}");
                    break;
                }
                if let Err(e) = stream.send_data(Bytes::from(body)).await {
                    warn!("h3 send_data: {e}");
                    break;
                }
                let _ = stream.finish().await;
            }
            Ok(None) => break,
            Err(e) => {
                warn!("h3 accept: {e}");
                break;
            }
        }
    }
    Ok(())
}

fn record_h3_app(
    req: &Request<()>,
    peer: SocketAddr,
    rtt_ms: f64,
    stats: &quinn::ConnectionStats,
) -> H3AppInfo {
    let mut pseudo: Vec<String> = Vec::new();
    // http::Request always has method/uri; order as observed in header map first
    for (name, _) in req.headers().iter() {
        let n = name.as_str();
        if n.starts_with(':') {
            pseudo.push(n.to_string());
        }
    }
    // Track whether we padded missing pseudo headers (diagnostic-only when padded).
    let mut pseudo_padded = false;
    if !pseudo.iter().any(|p| p == ":method") {
        pseudo.insert(0, ":method".into());
        pseudo_padded = true;
    }
    if !pseudo.iter().any(|p| p == ":path") {
        pseudo.push(":path".into());
        pseudo_padded = true;
    }
    let path = req.uri().path().to_string();
    let method = req.method().as_str().to_string();
    let order = pseudo.join(",");
    let mut h = Sha256::new();
    h.update(order.as_bytes());
    h.update(method.as_bytes());
    let settings_fp = format!("{:x}", h.finalize());
    let settings_fp = settings_fp[..16.min(settings_fp.len())].to_string();

    let info = H3AppInfo {
        present: true,
        src_ip: peer.ip().to_string(),
        src_port: peer.port(),
        alpn: "h3".into(),
        method: method.clone(),
        path: path.clone(),
        pseudo_order: order,
        settings_fp,
        rtt_ms: Some(rtt_ms),
        frames_rx: Some(stats.frame_rx.acks + stats.frame_rx.stream + stats.frame_rx.max_data),
        udp_rx: Some(stats.udp_rx.datagrams),
        note: if pseudo_padded {
            "HTTP/3 app request; pseudo_order partially padded (diagnostic_only)".into()
        } else {
            "HTTP/3 application server request".into()
        },
    };
    push_h3_app(info.clone());
    info
}

async fn dispatch_h3(
    state: &AppState,
    req: &Request<()>,
    peer: SocketAddr,
    this_conn_h3: Option<&H3AppInfo>,
) -> (StatusCode, Vec<u8>) {
    let path = req.uri().path();
    let method = req.method();
    let mut headers = HashMap::new();
    for (k, v) in req.headers().iter() {
        if let Ok(s) = v.to_str() {
            headers.insert(k.as_str().to_ascii_lowercase(), s.to_string());
        }
    }
    headers.insert("x-gr-peer-addr".into(), peer.to_string());
    headers.insert("x-gr-peer-ip".into(), peer.ip().to_string());
    headers.insert("x-gr-peer-port".into(), peer.port().to_string());
    headers.insert("x-real-ip".into(), peer.ip().to_string());
    headers.insert("x-gr-h3-app".into(), "1".into());
    headers.insert("x-tls-alpn".into(), "h3".into());

    // G1: inject **this connection's** H3 sample (never IP-only latest-wins across NAT).
    if let Some(info) = this_conn_h3 {
        inject_h3_app_headers_from_info(&mut headers, info);
    } else {
        // Fallback: port-scoped lookup (still stronger than bare IP).
        crate::listen::side_store::inject_h3_app_headers_scoped(
            &mut headers,
            &crate::listen::JoinScope {
                client_ip: Some(peer.ip().to_string()),
                src_port: Some(peer.port()),
                ..Default::default()
            },
        );
    }

    let json_body = |v: Value| serde_json::to_vec(&v).unwrap_or_else(|_| b"{}".to_vec());

    match (method, path) {
        (&Method::GET, "/v1/health") | (&Method::GET, "/health") | (&Method::GET, "/healthz") => {
            (StatusCode::OK, json_body(handlers::health(state)))
        }
        (&Method::POST, "/v1/session/open") => match handlers::open_session(
            state,
            OpenBody {
                session_id: None,
                visitor_terminal_id: None,
                meta: None,
                inject_path: Some("h3".into()),
                session_ticket: None,
                site_id: None,
                force_identity: None,
                storage_bind: None,
                relay_ts_ms: None,
                relay_nonce: None,
                relay_sig: None,
                cookie_fields: None,
                embed_token: None,
            },
            &headers,
        ) {
            Ok(v) => (StatusCode::OK, json_body(v)),
            Err(e) => (
                StatusCode::from_u16(e.status()).unwrap_or(StatusCode::BAD_REQUEST),
                json_body(e.to_json()),
            ),
        },
        (&Method::POST, "/v1/gateway/early") | (&Method::POST, "/v1/gateway/s0") => {
            let body = GatewayEarlyBody {
                session_id: None,
                visitor_terminal_id: None,
                inject_path: Some("h3".into()),
                session_ticket: None,
                fields: json!({
                    "user_agent": headers.get("user-agent").cloned().unwrap_or_default(),
                    "h3_app_client": true,
                }),
                analyze: false,
                site_id: None,
            };
            match dispatch_gateway(state, &headers, body) {
                Ok(v) => (StatusCode::OK, json_body(v)),
                Err(e) => (
                    StatusCode::from_u16(e.status()).unwrap_or(StatusCode::BAD_REQUEST),
                    json_body(e.to_json()),
                ),
            }
        }
        (&Method::GET, "/v1/ops/side_channels") => {
            if let Err(e) = crate::handlers::require_ops_auth(state, &headers) {
                (
                    StatusCode::from_u16(e.status()).unwrap_or(StatusCode::UNAUTHORIZED),
                    json_body(e.to_json()),
                )
            } else {
                (
                    StatusCode::OK,
                    json_body(json!({
                        "ok": true,
                        "side": crate::listen::recent_side_summary(),
                        "h3_app": true,
                    })),
                )
            }
        }
        _ => (
            StatusCode::NOT_FOUND,
            json_body(json!({
                "ok": false,
                "error": "h3 route not found",
                "path": path,
                "hint": "supported: GET /v1/health, POST /v1/session/open, POST /v1/gateway/early"
            })),
        ),
    }
}

fn dispatch_gateway(
    state: &AppState,
    headers: &HashMap<String, String>,
    body: GatewayEarlyBody,
) -> Result<Value, handlers::ApiError> {
    handlers::gateway_early(state, headers, body)
}

fn load_certs(path: &Path) -> Result<Vec<CertificateDer<'static>>, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("open cert {path:?}: {e}"))?;
    let mut reader = std::io::BufReader::new(f);
    let certs: Vec<_> = rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("parse certs: {e}"))?;
    if certs.is_empty() {
        return Err("no certificates in PEM".into());
    }
    Ok(certs)
}

fn load_private_key(path: &Path) -> Result<PrivateKeyDer<'static>, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("open key {path:?}: {e}"))?;
    let mut reader = std::io::BufReader::new(f);
    let mut keys = rustls_pemfile::pkcs8_private_keys(&mut reader);
    if let Some(k) = keys.next() {
        let k = k.map_err(|e| format!("pkcs8 key: {e}"))?;
        return Ok(PrivateKeyDer::Pkcs8(k));
    }
    // rewind-like: re-open for RSA
    let f = std::fs::File::open(path).map_err(|e| format!("reopen key: {e}"))?;
    let mut reader = std::io::BufReader::new(f);
    let mut keys = rustls_pemfile::rsa_private_keys(&mut reader);
    if let Some(k) = keys.next() {
        let k = k.map_err(|e| format!("rsa key: {e}"))?;
        return Ok(PrivateKeyDer::Pkcs1(k));
    }
    Err("no private key found in PEM".into())
}
