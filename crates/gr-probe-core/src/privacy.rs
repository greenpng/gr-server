//! iss/opus5 05-S-4: IP privacy controls.
//!
//! Default policy: client IPs are truncated to the network class at the
//! earliest ingestion point — `/24` for IPv4, `/48` for IPv6 — and the masked
//! form is what lands in payload `fields.server_client_ip` AND the indexed
//! columns (single write, one value, no raw/masked dual-track).
//! Full IPs are stored only in an explicit forensic mode
//! (`GR_IP_FORENSIC_MODE=1`) which operators must pair with a short
//! retention window.

/// Forensic mode: store full client IPs. Default OFF.
pub fn ip_forensic_mode() -> bool {
    gr_abi::env::get("IP_FORENSIC_MODE")
         .or_else(|| gr_abi::env::get("IP_FORENSIC_MODE"))
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "on" | "yes"))
        .unwrap_or(false)
}

/// Truncate an IP literal to its network class: v4 → `a.b.c.0/24`,
/// v6 → `first3Groups::/48`. Values that are already masked (contain '/')
/// pass through unchanged so double-masking is idempotent. Non-IP strings
/// (hashes, "unknown") pass through unchanged — they carry no extra PII.
pub fn mask_ip_subnet(ip: &str) -> String {
    let t = ip.trim();
    if t.is_empty() {
        return String::new();
    }
    if t.contains('/') {
        return t.to_string();
    }
    if let Ok(v4) = t.parse::<std::net::Ipv4Addr>() {
        let o = v4.octets();
        return format!("{}.{}.{}.0/24", o[0], o[1], o[2]);
    }
    if let Ok(v6) = t.parse::<std::net::Ipv6Addr>() {
        let s = v6.segments();
        return format!("{:x}:{:x}:{:x}::/48", s[0], s[1], s[2]);
    }
    t.to_string()
}

/// Apply the configured IP policy: forensic mode keeps the raw address,
/// default mode truncates to the network class.
pub fn apply_ip_policy(ip: &str) -> String {
    if ip_forensic_mode() {
        ip.trim().to_string()
    } else {
        mask_ip_subnet(ip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_v4_v24_and_v6_v48() {
        assert_eq!(mask_ip_subnet("192.168.1.87"), "192.168.1.0/24");
        assert_eq!(mask_ip_subnet("10.0.0.1"), "10.0.0.0/24");
        assert_eq!(mask_ip_subnet("2001:db8:abcd:1234::1"), "2001:db8:abcd::/48");
        // Idempotent on already-masked values.
        assert_eq!(mask_ip_subnet("192.168.1.0/24"), "192.168.1.0/24");
        // Non-IP tokens pass through (hashes / unknown markers).
        assert_eq!(mask_ip_subnet("unknown"), "unknown");
        assert_eq!(mask_ip_subnet(""), "");
    }

    #[test]
    fn forensic_mode_env_toggle() {
        // Default off.
        std::env::remove_var("GR_IP_FORENSIC_MODE");
        std::env::remove_var("GR_IP_FORENSIC_MODE");
        assert!(!ip_forensic_mode());
        assert_eq!(apply_ip_policy("203.0.113.77"), "203.0.113.0/24");
        std::env::set_var("GR_IP_FORENSIC_MODE", "1");
        assert!(ip_forensic_mode());
        assert_eq!(apply_ip_policy("203.0.113.77"), "203.0.113.77");
        std::env::remove_var("GR_IP_FORENSIC_MODE");
    }
}
