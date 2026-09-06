//! TCP SYN passive fingerprint (p0f-style), parsed from raw IPv4/IPv6+TCP headers.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TcpSynInfo {
    pub parse_ok: bool,
    pub ip_version: Option<u8>,
    pub src_ip: Option<String>,
    pub dst_ip: Option<String>,
    pub src_port: Option<u16>,
    pub dst_port: Option<u16>,
    pub ip_ttl: Option<u8>,
    pub ip_tos: Option<u8>,
    pub ip_total_len: Option<u16>,
    pub ip_id: Option<u16>,
    pub ip_df: Option<bool>,
    pub ip_mf: Option<bool>,
    pub tcp_seq: Option<u32>,
    pub tcp_ack: Option<u32>,
    pub tcp_data_off: Option<u8>,
    pub tcp_flags: Option<u8>,
    pub tcp_flags_str: Option<String>,
    pub tcp_window: Option<u16>,
    /// Scaled initial window estimate: window << wscale (when wscale present)
    pub tcp_window_scaled: Option<u32>,
    pub tcp_urgent: Option<u16>,
    pub tcp_mss: Option<u16>,
    pub tcp_wscale: Option<u8>,
    pub tcp_sack_ok: bool,
    pub tcp_timestamp: bool,
    pub tcp_ts_val: Option<u32>,
    pub tcp_ts_ecr: Option<u32>,
    pub tcp_nop_count: u8,
    pub tcp_eol: bool,
    /// ECE / CWR from SYN flags (ECN negotiation)
    pub tcp_ece: bool,
    pub tcp_cwr: bool,
    /// IP ECN bits from TOS (IPv4) / Traffic Class (IPv6)
    pub ip_ecn: Option<u8>,
    /// Raw TCP options bytes hex (capped) for layout forensic
    pub option_raw_hex: Option<String>,
    /// Options in order as type ids (fingerprint critical)
    pub option_order: Vec<u8>,
    pub option_order_str: Option<String>,
    /// p0f-like signature string
    pub p0f_sig: Option<String>,
    pub p0f_hash: Option<String>,
    /// Quirks string for deeper OS stack listen (cross-browser)
    pub quirks: Option<String>,
    /// Normalized TTL bucket (64/128/255)
    pub ttl_bucket: Option<u8>,
    /// Coarse OS family hint from TTL/options
    pub os_family_hint_zh: Option<String>,
    /// Option types raw list as "2-4-8-3" compact
    pub option_layout_compact: Option<String>,
    pub notes: Vec<String>,
    pub raw_len: usize,
}

/// Parse an Ethernet or IP-level frame containing a TCP SYN.
/// Accepts: raw IP packet, Ethernet + IP, or bare TCP header (TCP_SAVED_SYN).
pub fn parse_tcp_syn_packet(frame: &[u8]) -> Option<TcpSynInfo> {
    if frame.len() < 20 {
        return None;
    }
    // Skip Ethernet if needed
    let ip = if frame[0] == 0x45 || (frame[0] >> 4) == 4 || (frame[0] >> 4) == 6 {
        frame
    } else if frame.len() > 14 + 40 {
        // EtherType at 12
        let ethertype = u16::from_be_bytes([frame[12], frame[13]]);
        if ethertype == 0x0800 || ethertype == 0x86dd {
            &frame[14..]
        } else if ethertype == 0x8100 && frame.len() > 18 {
            // VLAN
            &frame[18..]
        } else {
            frame
        }
    } else {
        frame
    };
    let ver = ip[0] >> 4;
    if ver == 4 {
        parse_ipv4_tcp_syn(ip)
    } else if ver == 6 {
        parse_ipv6_tcp_syn(ip)
    } else {
        // Bare TCP header (some kernels return TCP-only for TCP_SAVED_SYN)
        parse_bare_tcp_syn(frame)
    }
}

/// Parse TCP header only (no IP) — used for TCP_SAVED_SYN fallback.
fn parse_bare_tcp_syn(tcp: &[u8]) -> Option<TcpSynInfo> {
    if tcp.len() < 20 {
        return None;
    }
    // data offset sanity: bits 4-7 of byte 12
    let doff = ((tcp[12] >> 4) as usize) * 4;
    if doff < 20 || doff > tcp.len() {
        return None;
    }
    let mut info = base_tcp_syn(tcp)?;
    if !is_syn(&info) {
        return None;
    }
    info.notes.push("parsed bare TCP (no IP header)".into());
    finalize_p0f(&mut info);
    Some(info)
}

fn parse_ipv4_tcp_syn(ip: &[u8]) -> Option<TcpSynInfo> {
    if ip.len() < 20 {
        return None;
    }
    let ihl = (ip[0] & 0x0f) as usize * 4;
    if ip.len() < ihl + 20 {
        return None;
    }
    let proto = ip[9];
    if proto != 6 {
        return None;
    }
    let flags_frag = u16::from_be_bytes([ip[6], ip[7]]);
    let tcp = &ip[ihl..];
    let mut info = base_tcp_syn(tcp)?;
    if !is_syn(&info) {
        return None;
    }
    info.ip_version = Some(4);
    info.ip_ttl = Some(ip[8]);
    info.ip_tos = Some(ip[1]);
    info.ip_ecn = Some(ip[1] & 0x03);
    info.ip_total_len = Some(u16::from_be_bytes([ip[2], ip[3]]));
    info.ip_id = Some(u16::from_be_bytes([ip[4], ip[5]]));
    info.ip_df = Some((flags_frag & 0x4000) != 0);
    info.ip_mf = Some((flags_frag & 0x2000) != 0);
    info.src_ip = Some(format!("{}.{}.{}.{}", ip[12], ip[13], ip[14], ip[15]));
    info.dst_ip = Some(format!("{}.{}.{}.{}", ip[16], ip[17], ip[18], ip[19]));
    finalize_p0f(&mut info);
    Some(info)
}

fn parse_ipv6_tcp_syn(ip: &[u8]) -> Option<TcpSynInfo> {
    if ip.len() < 40 + 20 {
        return None;
    }
    // simplified: assume no extension headers, next header at 6
    if ip[6] != 6 {
        return None;
    }
    let tcp = &ip[40..];
    let mut info = base_tcp_syn(tcp)?;
    if !is_syn(&info) {
        return None;
    }
    info.ip_version = Some(6);
    info.ip_ttl = Some(ip[7]); // hop limit
    // Traffic Class straddles bytes 0-1
    let tc = ((ip[0] & 0x0f) << 4) | ((ip[1] & 0xf0) >> 4);
    info.ip_tos = Some(tc);
    info.ip_ecn = Some(tc & 0x03);
    let src = &ip[8..24];
    let dst = &ip[24..40];
    info.src_ip = Some(format_ipv6(src));
    info.dst_ip = Some(format_ipv6(dst));
    finalize_p0f(&mut info);
    Some(info)
}

fn format_ipv6(b: &[u8]) -> String {
    let mut parts = Vec::new();
    for c in b.chunks(2) {
        parts.push(format!("{:x}", u16::from_be_bytes([c[0], c[1]])));
    }
    parts.join(":")
}

fn is_syn(info: &TcpSynInfo) -> bool {
    let f = info.tcp_flags.unwrap_or(0);
    // SYN set, ACK not set
    (f & 0x02) != 0 && (f & 0x10) == 0
}

fn base_tcp_syn(tcp: &[u8]) -> Option<TcpSynInfo> {
    if tcp.len() < 20 {
        return None;
    }
    let data_off = (tcp[12] >> 4) as usize * 4;
    if tcp.len() < data_off {
        return None;
    }
    let flags = tcp[13];
    let mut info = TcpSynInfo {
        parse_ok: true,
        src_port: Some(u16::from_be_bytes([tcp[0], tcp[1]])),
        dst_port: Some(u16::from_be_bytes([tcp[2], tcp[3]])),
        tcp_seq: Some(u32::from_be_bytes([tcp[4], tcp[5], tcp[6], tcp[7]])),
        tcp_ack: Some(u32::from_be_bytes([tcp[8], tcp[9], tcp[10], tcp[11]])),
        tcp_data_off: Some((tcp[12] >> 4) as u8),
        tcp_flags: Some(flags),
        tcp_flags_str: Some(flags_str(flags)),
        tcp_window: Some(u16::from_be_bytes([tcp[14], tcp[15]])),
        tcp_urgent: Some(u16::from_be_bytes([tcp[18], tcp[19]])),
        tcp_ece: (flags & 0x40) != 0,
        tcp_cwr: (flags & 0x80) != 0,
        raw_len: tcp.len(),
        ..Default::default()
    };
    // options
    if data_off > 20 {
        let opts = &tcp[20..data_off];
        info.option_raw_hex = Some(hex::encode(&opts[..opts.len().min(40)]));
        parse_tcp_options(opts, &mut info);
    }
    Some(info)
}

fn flags_str(f: u8) -> String {
    let mut s = String::new();
    if f & 0x01 != 0 {
        s.push('F');
    }
    if f & 0x02 != 0 {
        s.push('S');
    }
    if f & 0x04 != 0 {
        s.push('R');
    }
    if f & 0x08 != 0 {
        s.push('P');
    }
    if f & 0x10 != 0 {
        s.push('A');
    }
    if f & 0x20 != 0 {
        s.push('U');
    }
    if f & 0x40 != 0 {
        s.push('E'); // ECE
    }
    if f & 0x80 != 0 {
        s.push('C'); // CWR
    }
    if s.is_empty() {
        s.push('-');
    }
    s
}

fn parse_tcp_options(opts: &[u8], info: &mut TcpSynInfo) {
    let mut i = 0;
    while i < opts.len() {
        let kind = opts[i];
        info.option_order.push(kind);
        match kind {
            0 => {
                // EOL
                info.tcp_eol = true;
                break;
            }
            1 => {
                // NOP
                info.tcp_nop_count += 1;
                i += 1;
            }
            _ => {
                if i + 1 >= opts.len() {
                    break;
                }
                let len = opts[i + 1] as usize;
                if len < 2 || i + len > opts.len() {
                    break;
                }
                let data = &opts[i + 2..i + len];
                match kind {
                    2 if data.len() >= 2 => {
                        info.tcp_mss = Some(u16::from_be_bytes([data[0], data[1]]));
                    }
                    3 if !data.is_empty() => {
                        info.tcp_wscale = Some(data[0]);
                    }
                    4 => info.tcp_sack_ok = true,
                    8 if data.len() >= 8 => {
                        info.tcp_timestamp = true;
                        info.tcp_ts_val =
                            Some(u32::from_be_bytes([data[0], data[1], data[2], data[3]]));
                        info.tcp_ts_ecr =
                            Some(u32::from_be_bytes([data[4], data[5], data[6], data[7]]));
                    }
                    _ => {}
                }
                i += len;
            }
        }
    }
    info.option_order_str = Some(
        info.option_order
            .iter()
            .map(|k| match k {
                0 => "eol".into(),
                1 => "nop".into(),
                2 => "mss".into(),
                3 => "ws".into(),
                4 => "sok".into(),
                8 => "ts".into(),
                n => format!("{n}"),
            })
            .collect::<Vec<_>>()
            .join(","),
    );
}

fn finalize_p0f(info: &mut TcpSynInfo) {
    // Simplified p0f v3-like signature:
    // ver:ittl:olen:mss:wsize,scale:olayout:quirks:pclass
    let ver = info.ip_version.unwrap_or(0);
    let ttl = info.ip_ttl.unwrap_or(0);
    let mss = info.tcp_mss.unwrap_or(0);
    let wsize = info.tcp_window.unwrap_or(0);
    let scale = info.tcp_wscale.map(|s| s.to_string()).unwrap_or_else(|| "*".into());
    // Sliding-window listen: scaled initial window when wscale present
    if let (Some(w), Some(ws)) = (info.tcp_window, info.tcp_wscale) {
        info.tcp_window_scaled = Some((w as u32) << (ws.min(14) as u32));
    } else if let Some(w) = info.tcp_window {
        info.tcp_window_scaled = Some(w as u32);
    }
    let olayout = info.option_order_str.clone().unwrap_or_default();
    info.option_layout_compact = Some(
        info.option_order
            .iter()
            .map(|k| k.to_string())
            .collect::<Vec<_>>()
            .join("-"),
    );

    // Quirks (cross-browser OS stack depth)
    let mut quirks = Vec::new();
    if info.ip_df == Some(true) {
        quirks.push("df");
    }
    if info.ip_df == Some(false) {
        quirks.push("!df");
    }
    if info.tcp_seq == Some(0) {
        quirks.push("seq0");
    }
    if info.tcp_ack != Some(0) && info.tcp_ack.is_some() {
        quirks.push("ack+"); // unusual on pure SYN
    }
    if info.tcp_urgent.unwrap_or(0) != 0 {
        quirks.push("urg");
    }
    if info.tcp_nop_count >= 3 {
        quirks.push("many_nop");
    }
    if info.tcp_eol {
        quirks.push("eol");
    }
    if !info.tcp_timestamp && info.tcp_sack_ok {
        quirks.push("sack_no_ts");
    }
    if info.tcp_wscale.is_none() && info.tcp_mss.is_some() {
        quirks.push("mss_no_ws");
    }
    if info.tcp_ece {
        quirks.push("ece");
    }
    if info.tcp_cwr {
        quirks.push("cwr");
    }
    if info.ip_ecn.unwrap_or(0) != 0 {
        quirks.push("ip_ecn");
    }
    let quirks_str = quirks.join(",");
    info.quirks = Some(quirks_str.clone());

    // TTL bucket / OS family coarse hint
    let (bucket, family) = if ttl == 0 {
        (0, "unknown")
    } else if ttl <= 64 {
        (64, "linux/mac/bsd-like")
    } else if ttl <= 128 {
        (128, "windows-like-or-multi-hop")
    } else {
        (255, "high-ttl/custom")
    };
    info.ttl_bucket = Some(bucket);
    info.os_family_hint_zh = Some(match bucket {
        64 => "偏 Linux/macOS/BSD 初始 TTL 族".into(),
        128 => "偏 Windows 初始 TTL 族或经跳数衰减".into(),
        255 => "高 TTL / 定制网络栈".into(),
        _ => "未知".into(),
    });
    let _ = family;

    let df = if info.ip_df == Some(true) { "df" } else { "" };
    // Forensic sig may include path-sensitive raw TTL/MSS/window (not used in os_stack_id core).
    let sig = format!(
        "{ver}:{ttl}:0:{mss}:{wsize},{scale}:{olayout}:{df}:{quirks_str}"
    );
    let mut h = Sha256::new();
    h.update(sig.as_bytes());
    info.p0f_sig = Some(sig);
    info.p0f_hash = Some(hex::encode(h.finalize())[..16].to_string());
    info.parse_ok = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_synthetic_syn() {
        // Minimal IPv4 + TCP SYN with MSS option
        let mut ip = vec![0x45, 0x00, 0x00, 0x2c, 0x00, 0x01, 0x40, 0x00, 0x40, 0x06, 0x00, 0x00];
        ip.extend_from_slice(&[192, 168, 1, 2, 10, 0, 0, 1]); // src dst
        // TCP
        let mut tcp = vec![
            0x04, 0xd2, // sport 1234
            0x00, 0x50, // dport 80
            0x00, 0x00, 0x00, 0x01, // seq
            0x00, 0x00, 0x00, 0x00, // ack
            0x60, 0x02, // dataoff=6 (24 bytes), SYN
            0xff, 0xff, // window
            0x00, 0x00, // checksum
            0x00, 0x00, // urgent
            0x02, 0x04, 0x05, 0xb4, // MSS 1460
        ];
        ip.append(&mut tcp);
        // fix total len
        let total = ip.len() as u16;
        ip[2] = (total >> 8) as u8;
        ip[3] = (total & 0xff) as u8;
        let info = parse_tcp_syn_packet(&ip).expect("syn");
        assert_eq!(info.src_port, Some(1234));
        assert_eq!(info.dst_port, Some(80));
        assert_eq!(info.tcp_mss, Some(1460));
        assert!(info.p0f_sig.is_some());
        assert!(info.option_order_str.as_ref().unwrap().contains("mss"));
    }
}
