//! Transparent reverse-proxy upstream config (P-V1).

use serde::Deserialize;
use std::net::ToSocketAddrs;

#[derive(Clone, Debug, Default)]
pub struct UpstreamConfig {
    /// host:port
    pub addr: String,
    pub tls: bool,
    pub sni: String,
}

impl UpstreamConfig {
    pub fn parse(addr: &str, tls: bool, sni: &str) -> Result<Self, String> {
        if addr.is_empty() {
            return Err("upstream empty".into());
        }
        // validate resolvable-ish
        let _ = addr
            .to_socket_addrs()
            .map_err(|e| format!("upstream resolve {addr}: {e}"))?;
        let sni = if sni.is_empty() {
            addr.split(':').next().unwrap_or(addr).to_string()
        } else {
            sni.to_string()
        };
        Ok(Self {
            addr: addr.to_string(),
            tls,
            sni,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProxyMode {
    /// All requests handled locally (default green-v5 API origin).
    Origin,
    /// Non-API traffic forwarded to upstream after edge capture.
    Transparent,
    /// Multi-node load balancer (docs/08, paid): non-API traffic routed by the
    /// shared `gr_lb::LbPool` — pure header passthrough, no mutations.
    Lb,
}

impl ProxyMode {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "transparent" | "proxy" | "forward" => Self::Transparent,
            "lb" | "load_balancer" | "load-balancer" => Self::Lb,
            _ => Self::Origin,
        }
    }
}

/// Paths that always stay on green-v5 origin even in transparent mode.
pub fn is_local_path(path: &str) -> bool {
    path == "/health"
        || path == "/healthz"
        || path == "/livez"
        || path == "/readyz"
        || path == "/s0"
        || path == "/ops"
        || path == "/ops.html"
        || path.starts_with("/v1/")
        || path.starts_with("/fe/")
        || path.starts_with("/dist/")
}

/// LB mode routes everything that is not local (pages, static assets outside
/// /fe, wildcard paths); health/API/FE stay on the node itself.
pub fn is_lb_path(path: &str) -> bool {
    !is_local_path(path)
}
