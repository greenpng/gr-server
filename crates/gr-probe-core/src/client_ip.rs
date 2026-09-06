//! Client IP resolution with CDN/gateway priority.
//!
//! Priority (high → low):
//! 1. `CF-Connecting-IP` (Cloudflare)
//! 2. `True-Client-IP` (enterprise CDN)
//! 3. `X-Real-IP` / first `X-Forwarded-For` when peer is trusted **or loopback**
//!    (local nginx → Pingora is typically 127.0.0.1)
//! 4. TCP peer when not loopback
//! 5. Peer as last resort (may be 127.0.0.1)

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{LazyLock, RwLock};

/// Cluster peers that act as LB gateways (docs/guides/08-LB-MODULE.md): the control plane feeds
/// every node's `advertise`/`internal_addr` here so an upstream node treats a
/// gateway peer like a trusted reverse proxy and honors XFF from it.
static CLUSTER_TRUSTED_PEERS: LazyLock<RwLock<Vec<String>>> =
    LazyLock::new(|| RwLock::new(Vec::new()));

/// Replace the cluster-trusted peer list (call from the LB control loop).
pub fn set_cluster_trusted_ips(ips: Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    let mut clean: Vec<String> = Vec::new();
    for ip in ips {
        let t = ip.trim();
        if t.is_empty() {
            continue;
        }
        // Accept host:port entries; keep the host part.
        let host = if let Ok(a) = t.parse::<SocketAddr>() {
            a.ip().to_string()
        } else {
            t.to_string()
        };
        if seen.insert(host.clone()) {
            clean.push(host);
        }
    }
    if let Ok(mut g) = CLUSTER_TRUSTED_PEERS.write() {
        *g = clean;
    }
}

fn cluster_trusted_contains(ip: &str) -> bool {
    CLUSTER_TRUSTED_PEERS
        .read()
        .map(|g| g.iter().any(|p| p.eq_ignore_ascii_case(ip)))
        .unwrap_or(false)
}

/// Whether `peer` is a trusted reverse proxy (may supply XFF).
/// `GR_TRUSTED_PROXIES`: comma-separated IPs/CIDRs; `*` / `any` = lab trust-all.
/// Empty → only loopback peers + cluster LB gateway peers are treated as
/// proxy for header honor.
pub fn peer_is_trusted_proxy(peer_ip: Option<&str>) -> bool {
    if peer_ip.map(is_loopback_str).unwrap_or(false) {
        // Local reverse proxy (nginx/caddy → 127.0.0.1) must honor CF/XFF.
        return true;
    }
    if let Some(ip) = peer_ip {
        if cluster_trusted_contains(ip) {
            return true;
        }
    }
    let raw = gr_abi::env::get("TRUSTED_PROXIES").unwrap_or_default();
    let raw = raw.trim();
    if raw.is_empty() {
        return false;
    }
    if raw == "*" || raw.eq_ignore_ascii_case("any") {
        return true;
    }
    let Some(ip_s) = peer_ip else {
        return false;
    };
    let Ok(ip) = ip_s.parse::<IpAddr>() else {
        return false;
    };
    for part in raw.split(',') {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        if let Ok(exact) = p.parse::<IpAddr>() {
            if exact == ip {
                return true;
            }
            continue;
        }
        if let Some((net, bits)) = p.split_once('/') {
            if let (Ok(base), Ok(prefix)) = (net.parse::<IpAddr>(), bits.parse::<u8>()) {
                if ip_in_cidr(ip, base, prefix) {
                    return true;
                }
            }
        }
    }
    false
}

fn is_loopback_str(s: &str) -> bool {
    s.parse::<IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

fn ip_in_cidr(ip: IpAddr, base: IpAddr, prefix: u8) -> bool {
    match (ip, base) {
        (IpAddr::V4(a), IpAddr::V4(b)) => {
            if prefix > 32 {
                return false;
            }
            let mask = if prefix == 0 {
                0u32
            } else {
                u32::MAX << (32 - prefix)
            };
            (u32::from(a) & mask) == (u32::from(b) & mask)
        }
        (IpAddr::V6(a), IpAddr::V6(b)) => {
            if prefix > 128 {
                return false;
            }
            let a = u128::from(a);
            let b = u128::from(b);
            let mask = if prefix == 0 {
                0u128
            } else {
                u128::MAX << (128 - prefix)
            };
            (a & mask) == (b & mask)
        }
        _ => false,
    }
}

fn header_get<'a>(headers: &'a HashMap<String, String>, name: &str) -> Option<&'a str> {
    // Case-insensitive key match (HTTP headers).
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn first_ip_token(v: &str) -> Option<String> {
    let first = v.split(',').next().unwrap_or(v).trim();
    if first.is_empty() || first.eq_ignore_ascii_case("unknown") {
        return None;
    }
    // Basic validation: parse as IP when possible; still accept host-like tokens.
    if first.parse::<IpAddr>().is_ok() || first.contains('.') || first.contains(':') {
        return Some(first.to_string());
    }
    None
}

/// Resolve client IP. Prefer CDN headers over gateway peer.
pub fn client_ip_from(
    headers: &HashMap<String, String>,
    peer: Option<SocketAddr>,
) -> Option<String> {
    let peer_ip = peer.map(|a| a.ip().to_string());

    // 1–2: CDN headers win **only when non-empty** (lab has no CF — skip empty).
    for key in ["cf-connecting-ip", "true-client-ip"] {
        if let Some(v) = header_get(headers, key) {
            if v.trim().is_empty() {
                continue;
            }
            if let Some(ip) = first_ip_token(v) {
                return Some(ip);
            }
        }
    }

    // 3: proxy headers when peer trusted or loopback
    if peer_is_trusted_proxy(peer_ip.as_deref()) {
        for key in ["x-real-ip", "x-forwarded-for"] {
            if let Some(v) = header_get(headers, key) {
                if let Some(ip) = first_ip_token(v) {
                    // Prefer non-loopback from XFF over later peer loopback.
                    if !is_loopback_str(&ip) {
                        return Some(ip);
                    }
                    // Keep loopback XFF only if no better later — continue
                }
            }
        }
        // If XFF was only loopback, fall through to peer
        for key in ["x-real-ip", "x-forwarded-for"] {
            if let Some(v) = header_get(headers, key) {
                if let Some(ip) = first_ip_token(v) {
                    return Some(ip);
                }
            }
        }
    }

    // 4–5: peer
    peer_ip
}

/// Source tag for ops / persistence.
pub fn client_ip_source(
    headers: &HashMap<String, String>,
    peer: Option<SocketAddr>,
) -> &'static str {
    let peer_ip = peer.map(|a| a.ip().to_string());
    for key in ["cf-connecting-ip", "true-client-ip"] {
        if header_get(headers, key)
            .and_then(first_ip_token)
            .is_some()
        {
            return if key.starts_with("cf") {
                "cf"
            } else {
                "true_client_ip"
            };
        }
    }
    if peer_is_trusted_proxy(peer_ip.as_deref()) {
        if header_get(headers, "x-real-ip")
            .and_then(first_ip_token)
            .is_some()
        {
            return "x_real_ip";
        }
        if header_get(headers, "x-forwarded-for")
            .and_then(first_ip_token)
            .is_some()
        {
            return "x_forwarded_for";
        }
    }
    if peer_ip.as_deref().map(is_loopback_str).unwrap_or(false) {
        return "peer_loopback";
    }
    "peer"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    fn headers(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn peer(ip: &str) -> SocketAddr {
        SocketAddr::new(ip.parse().unwrap(), 443)
    }

    #[test]
    fn cf_beats_loopback_peer() {
        let h = headers(&[
            ("cf-connecting-ip", "203.0.113.50"),
            ("x-forwarded-for", "198.51.100.1"),
        ]);
        let ip = client_ip_from(&h, Some(peer("127.0.0.1")));
        assert_eq!(ip.as_deref(), Some("203.0.113.50"));
        assert_eq!(client_ip_source(&h, Some(peer("127.0.0.1"))), "cf");
    }

    #[test]
    fn xff_honored_when_peer_loopback() {
        let h = headers(&[("x-forwarded-for", "198.51.100.7, 10.0.0.1")]);
        let ip = client_ip_from(&h, Some(peer("127.0.0.1")));
        assert_eq!(ip.as_deref(), Some("198.51.100.7"));
    }

    #[test]
    fn peer_only_when_no_headers() {
        let h = headers(&[]);
        let ip = client_ip_from(&h, Some(peer("203.0.113.9")));
        assert_eq!(ip.as_deref(), Some("203.0.113.9"));
        assert_eq!(client_ip_source(&h, Some(peer("203.0.113.9"))), "peer");
    }

    #[test]
    fn untrusted_remote_peer_ignores_spoofed_xff() {
        // Clear env
        std::env::remove_var("GR_TRUSTED_PROXIES");
        let h = headers(&[("x-forwarded-for", "1.2.3.4")]);
        let ip = client_ip_from(&h, Some(peer("198.51.100.20")));
        // Not loopback, not trusted → peer wins
        assert_eq!(ip.as_deref(), Some("198.51.100.20"));
    }

    #[test]
    fn true_client_ip_second() {
        let h = headers(&[("true-client-ip", "203.0.113.88")]);
        assert_eq!(
            client_ip_from(&h, Some(peer("127.0.0.1"))).as_deref(),
            Some("203.0.113.88")
        );
    }

    #[test]
    fn loopback_const() {
        assert!(IpAddr::V4(Ipv4Addr::LOCALHOST).is_loopback());
    }
}
