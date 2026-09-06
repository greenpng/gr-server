//! HAProxy PROXY protocol v1 / v2 parser (listen depth).

use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProxyProtocolInfo {
    pub version: u8,
    pub command: String,
    pub family: String,
    pub transport: String,
    pub src_ip: Option<String>,
    pub src_port: Option<u16>,
    pub dst_ip: Option<String>,
    pub dst_port: Option<u16>,
    pub raw_header_hex: Option<String>,
    pub raw_header_len: usize,
    pub parse_ok: bool,
    pub notes: Vec<String>,
}

/// Try parse PROXY protocol from buffer. Returns (info, bytes_consumed).
/// If not PROXY, returns None (caller should rewind).
pub fn try_parse(buf: &[u8]) -> Option<(ProxyProtocolInfo, usize)> {
    if buf.starts_with(b"PROXY ") {
        return parse_v1(buf);
    }
    // v2 signature: \r\n\r\n\0\r\nQUIT\n
    const SIG: &[u8] = b"\r\n\r\n\0\r\nQUIT\n";
    if buf.len() >= 16 && buf.starts_with(SIG) {
        return parse_v2(buf);
    }
    None
}

fn parse_v1(buf: &[u8]) -> Option<(ProxyProtocolInfo, usize)> {
    let end = buf.windows(2).position(|w| w == b"\r\n")?;
    let line = std::str::from_utf8(&buf[..end]).ok()?;
    let consumed = end + 2;
    let mut info = ProxyProtocolInfo {
        version: 1,
        raw_header_len: consumed,
        raw_header_hex: Some(hex::encode(&buf[..consumed.min(256)])),
        parse_ok: false,
        ..Default::default()
    };
    // PROXY TCP4 1.2.3.4 5.6.7.8 12345 443\r\n
    // PROXY UNKNOWN\r\n
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 2 || parts[0] != "PROXY" {
        info.notes.push("invalid v1 line".into());
        return Some((info, consumed));
    }
    info.command = "PROXY".into();
    let fam = parts[1];
    info.family = fam.into();
    if fam == "UNKNOWN" {
        info.parse_ok = true;
        info.notes.push("v1 UNKNOWN".into());
        return Some((info, consumed));
    }
    if parts.len() < 6 {
        info.notes.push("v1 missing fields".into());
        return Some((info, consumed));
    }
    info.transport = if fam.starts_with("TCP") {
        "stream".into()
    } else {
        "unknown".into()
    };
    info.src_ip = Some(parts[2].into());
    info.dst_ip = Some(parts[3].into());
    info.src_port = parts[4].parse().ok();
    info.dst_port = parts[5].parse().ok();
    info.parse_ok = info.src_ip.is_some() && info.src_port.is_some();
    Some((info, consumed))
}

fn parse_v2(buf: &[u8]) -> Option<(ProxyProtocolInfo, usize)> {
    if buf.len() < 16 {
        return None;
    }
    let ver_cmd = buf[12];
    let version = ver_cmd >> 4;
    let cmd = ver_cmd & 0x0f;
    let fam_proto = buf[13];
    let family = fam_proto >> 4;
    let proto = fam_proto & 0x0f;
    let len = u16::from_be_bytes([buf[14], buf[15]]) as usize;
    let total = 16 + len;
    if buf.len() < total {
        return None; // incomplete
    }
    let mut info = ProxyProtocolInfo {
        version: version as u8,
        command: match cmd {
            0 => "LOCAL".into(),
            1 => "PROXY".into(),
            n => format!("cmd_{n}"),
        },
        family: match family {
            0 => "UNSPEC".into(),
            1 => "INET".into(),
            2 => "INET6".into(),
            3 => "UNIX".into(),
            n => format!("fam_{n}"),
        },
        transport: match proto {
            0 => "UNSPEC".into(),
            1 => "STREAM".into(),
            2 => "DGRAM".into(),
            n => format!("proto_{n}"),
        },
        raw_header_len: total,
        raw_header_hex: Some(hex::encode(&buf[..total.min(256)])),
        parse_ok: false,
        ..Default::default()
    };
    let addr = &buf[16..total];
    match (family, proto) {
        (1, _) if addr.len() >= 12 => {
            // src_addr(4) dst_addr(4) src_port(2) dst_port(2)
            let s = Ipv4Addr::new(addr[0], addr[1], addr[2], addr[3]);
            let d = Ipv4Addr::new(addr[4], addr[5], addr[6], addr[7]);
            let sp = u16::from_be_bytes([addr[8], addr[9]]);
            let dp = u16::from_be_bytes([addr[10], addr[11]]);
            info.src_ip = Some(IpAddr::V4(s).to_string());
            info.dst_ip = Some(IpAddr::V4(d).to_string());
            info.src_port = Some(sp);
            info.dst_port = Some(dp);
            info.parse_ok = true;
        }
        (2, _) if addr.len() >= 36 => {
            let mut sa = [0u8; 16];
            let mut da = [0u8; 16];
            sa.copy_from_slice(&addr[0..16]);
            da.copy_from_slice(&addr[16..32]);
            let sp = u16::from_be_bytes([addr[32], addr[33]]);
            let dp = u16::from_be_bytes([addr[34], addr[35]]);
            info.src_ip = Some(IpAddr::V6(Ipv6Addr::from(sa)).to_string());
            info.dst_ip = Some(IpAddr::V6(Ipv6Addr::from(da)).to_string());
            info.src_port = Some(sp);
            info.dst_port = Some(dp);
            info.parse_ok = true;
        }
        (0, _) => {
            info.parse_ok = true;
            info.notes.push("v2 LOCAL/UNSPEC".into());
        }
        _ => {
            info.notes.push(format!(
                "v2 unparsed family={} len={}",
                family,
                addr.len()
            ));
            info.parse_ok = cmd == 0; // LOCAL ok
        }
    }
    Some((info, total))
}

/// Convenience: parse SocketAddr pair if present
pub fn src_socket(info: &ProxyProtocolInfo) -> Option<SocketAddr> {
    let ip: IpAddr = info.src_ip.as_ref()?.parse().ok()?;
    let port = info.src_port?;
    Some(SocketAddr::new(ip, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_tcp4() {
        let s = b"PROXY TCP4 192.168.0.1 192.168.0.11 56324 443\r\nGET / HTTP/1.1\r\n";
        let (info, n) = try_parse(s).unwrap();
        assert!(info.parse_ok);
        assert_eq!(info.version, 1);
        assert_eq!(info.src_ip.as_deref(), Some("192.168.0.1"));
        assert_eq!(info.src_port, Some(56324));
        assert_eq!(n, s.iter().position(|&b| b == b'\n').unwrap() + 1);
    }

    #[test]
    fn v2_inet() {
        let mut buf = Vec::from(&b"\r\n\r\n\0\r\nQUIT\n"[..]);
        buf.push(0x21); // ver=2 cmd=PROXY
        buf.push(0x11); // INET STREAM
        buf.extend_from_slice(&12u16.to_be_bytes());
        buf.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 0x30, 0x39, 0x01, 0xbb]); // ports 12345, 443
        let (info, n) = try_parse(&buf).unwrap();
        assert!(info.parse_ok);
        assert_eq!(info.version, 2);
        assert_eq!(info.src_ip.as_deref(), Some("1.2.3.4"));
        assert_eq!(info.src_port, Some(12345));
        assert_eq!(info.dst_port, Some(443));
        assert_eq!(n, 28);
    }

    #[test]
    fn not_proxy() {
        assert!(try_parse(b"GET / HTTP/1.1\r\n").is_none());
    }
}
