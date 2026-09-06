//! STUN (RFC 5389) parse + Binding Success response builder (WebRTC R4 listen).

use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

pub const STUN_MAGIC: u32 = 0x2112_A442;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StunInfo {
    pub parse_ok: bool,
    pub message_type: Option<u16>,
    pub message_type_name: Option<String>,
    pub is_binding_request: bool,
    pub is_binding_success: bool,
    pub is_binding_error: bool,
    pub transaction_id_hex: Option<String>,
    pub attribute_types: Vec<u16>,
    pub attribute_names: Vec<String>,
    pub has_fingerprint: bool,
    pub has_message_integrity: bool,
    pub has_username: bool,
    pub has_ice_controlling: bool,
    pub has_ice_controlled: bool,
    pub has_priority: bool,
    pub has_use_candidate: bool,
    pub mapped_addr: Option<String>,
    pub xor_mapped_addr: Option<String>,
    pub software: Option<String>,
    pub raw_len: usize,
    pub notes: Vec<String>,
}

/// Returns true if buffer looks like STUN (magic cookie at offset 4).
pub fn looks_like_stun(data: &[u8]) -> bool {
    if data.len() < 20 {
        return false;
    }
    // Top 2 bits of message type must be 0 for STUN
    if data[0] & 0xC0 != 0 {
        return false;
    }
    let magic = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    magic == STUN_MAGIC
}

pub fn parse_stun(data: &[u8]) -> StunInfo {
    let mut info = StunInfo {
        raw_len: data.len(),
        ..Default::default()
    };
    if !looks_like_stun(data) {
        info.notes.push("not stun".into());
        return info;
    }
    let msg_type = u16::from_be_bytes([data[0], data[1]]);
    let len = u16::from_be_bytes([data[2], data[3]]) as usize;
    if data.len() < 20 + len {
        info.notes.push("truncated stun".into());
        return info;
    }
    info.parse_ok = true;
    info.message_type = Some(msg_type);
    info.message_type_name = Some(stun_type_name(msg_type).into());
    info.is_binding_request = msg_type == 0x0001;
    info.is_binding_success = msg_type == 0x0101;
    info.is_binding_error = msg_type == 0x0111;
    info.transaction_id_hex = Some(hex::encode(&data[8..20]));

    let mut off = 20;
    let end = 20 + len;
    while off + 4 <= end && off + 4 <= data.len() {
        let atype = u16::from_be_bytes([data[off], data[off + 1]]);
        let alen = u16::from_be_bytes([data[off + 2], data[off + 3]]) as usize;
        let val_start = off + 4;
        let val_end = val_start + alen;
        if val_end > data.len() || val_end > end {
            break;
        }
        info.attribute_types.push(atype);
        info.attribute_names.push(stun_attr_name(atype).into());
        match atype {
            0x0001 => {
                // MAPPED-ADDRESS
                if let Some(a) = parse_mapped_addr(&data[val_start..val_end], false, &data[8..20]) {
                    info.mapped_addr = Some(a);
                }
            }
            0x0020 => {
                // XOR-MAPPED-ADDRESS
                if let Some(a) = parse_mapped_addr(&data[val_start..val_end], true, &data[8..20]) {
                    info.xor_mapped_addr = Some(a);
                }
            }
            0x0006 => info.has_username = true,
            0x0008 => info.has_message_integrity = true,
            0x8028 => info.has_fingerprint = true,
            0x0024 => info.has_priority = true,
            0x0025 => info.has_use_candidate = true,
            0x8029 => info.has_ice_controlled = true,
            0x802A => info.has_ice_controlling = true,
            0x8022 => {
                if let Ok(s) = std::str::from_utf8(&data[val_start..val_end]) {
                    info.software = Some(s.trim_end_matches('\0').to_string());
                }
            }
            _ => {}
        }
        // attributes are padded to 4 bytes
        let padded = (alen + 3) & !3;
        off = val_start + padded;
    }
    info
}

fn stun_type_name(t: u16) -> &'static str {
    match t {
        0x0001 => "BindingRequest",
        0x0101 => "BindingSuccess",
        0x0111 => "BindingError",
        0x0003 => "AllocateRequest",
        _ => "Other",
    }
}

fn stun_attr_name(t: u16) -> &'static str {
    match t {
        0x0001 => "MAPPED-ADDRESS",
        0x0006 => "USERNAME",
        0x0008 => "MESSAGE-INTEGRITY",
        0x0009 => "ERROR-CODE",
        0x000A => "UNKNOWN-ATTRIBUTES",
        0x0014 => "REALM",
        0x0015 => "NONCE",
        0x0020 => "XOR-MAPPED-ADDRESS",
        0x0024 => "PRIORITY",
        0x0025 => "USE-CANDIDATE",
        0x8022 => "SOFTWARE",
        0x8028 => "FINGERPRINT",
        0x8029 => "ICE-CONTROLLED",
        0x802A => "ICE-CONTROLLING",
        _ => "ATTR",
    }
}

fn parse_mapped_addr(val: &[u8], xor: bool, txid: &[u8]) -> Option<String> {
    if val.len() < 4 {
        return None;
    }
    let family = val[1];
    let mut port = u16::from_be_bytes([val[2], val[3]]);
    if xor {
        port ^= (STUN_MAGIC >> 16) as u16;
    }
    match family {
        0x01 if val.len() >= 8 => {
            let mut ip = [val[4], val[5], val[6], val[7]];
            if xor {
                let m = STUN_MAGIC.to_be_bytes();
                for i in 0..4 {
                    ip[i] ^= m[i];
                }
            }
            Some(format!("{}:{}", Ipv4Addr::from(ip), port))
        }
        0x02 if val.len() >= 20 && txid.len() >= 12 => {
            let mut ip = [0u8; 16];
            ip.copy_from_slice(&val[4..20]);
            if xor {
                let m = STUN_MAGIC.to_be_bytes();
                for i in 0..4 {
                    ip[i] ^= m[i];
                }
                for i in 0..12 {
                    ip[4 + i] ^= txid[i];
                }
            }
            Some(format!("{}:{}", Ipv6Addr::from(ip), port))
        }
        _ => None,
    }
}

/// Build Binding Success with XOR-MAPPED-ADDRESS = `mapped`.
pub fn build_binding_success(request: &[u8], mapped: SocketAddr) -> Option<Vec<u8>> {
    if request.len() < 20 || !looks_like_stun(request) {
        return None;
    }
    let txid = &request[8..20];
    let mut xor_attr = Vec::new();
    xor_attr.push(0u8); // reserved
    match mapped.ip() {
        IpAddr::V4(v4) => {
            xor_attr.push(0x01);
            let port = mapped.port() ^ ((STUN_MAGIC >> 16) as u16);
            xor_attr.extend_from_slice(&port.to_be_bytes());
            let mut ip = v4.octets();
            let m = STUN_MAGIC.to_be_bytes();
            for i in 0..4 {
                ip[i] ^= m[i];
            }
            xor_attr.extend_from_slice(&ip);
        }
        IpAddr::V6(v6) => {
            xor_attr.push(0x02);
            let port = mapped.port() ^ ((STUN_MAGIC >> 16) as u16);
            xor_attr.extend_from_slice(&port.to_be_bytes());
            let mut ip = v6.octets();
            let m = STUN_MAGIC.to_be_bytes();
            for i in 0..4 {
                ip[i] ^= m[i];
            }
            for i in 0..12 {
                ip[4 + i] ^= txid[i];
            }
            xor_attr.extend_from_slice(&ip);
        }
    }
    let attr_len = xor_attr.len() as u16;
    let msg_len = 4 + attr_len; // one attribute header + value (already 4-aligned for v4)
    let pad = (4 - (xor_attr.len() % 4)) % 4;
    let msg_len = msg_len + pad as u16;

    let mut out = Vec::with_capacity(20 + msg_len as usize);
    out.extend_from_slice(&0x0101u16.to_be_bytes()); // Binding Success
    out.extend_from_slice(&msg_len.to_be_bytes());
    out.extend_from_slice(&STUN_MAGIC.to_be_bytes());
    out.extend_from_slice(txid);
    out.extend_from_slice(&0x0020u16.to_be_bytes()); // XOR-MAPPED-ADDRESS
    out.extend_from_slice(&attr_len.to_be_bytes());
    out.extend_from_slice(&xor_attr);
    for _ in 0..pad {
        out.push(0);
    }
    // fix length field
    let len = (out.len() - 20) as u16;
    out[2] = (len >> 8) as u8;
    out[3] = (len & 0xff) as u8;
    Some(out)
}

/// Detect DTLS ClientHello (content_type=22, version 0xfe**, handshake_type=1).
pub fn looks_like_dtls_client_hello(data: &[u8]) -> bool {
    // DTLS record: type(1) ver(2) epoch(2) seq(6) len(2) = 13, then handshake type
    data.len() >= 14 && data[0] == 22 && data[1] == 0xfe && data[13] == 1
}

/// Best-effort DTLS ClientHello cipher fingerprint (JA3-like short hash).
/// Returns (fingerprint_string, sha256_16hex).
pub fn fingerprint_dtls_client_hello(data: &[u8]) -> Option<(String, String)> {
    if !looks_like_dtls_client_hello(data) {
        return None;
    }
    // Skip DTLS record header (13) + handshake header (12) = 25
    // handshake: type(1) len(3) msg_seq(2) frag_off(3) frag_len(3)
    if data.len() < 25 + 2 + 32 + 1 {
        return None;
    }
    let mut i = 25;
    let client_ver = u16::from_be_bytes([data[i], data[i + 1]]);
    i += 2;
    i += 32; // random
    if i >= data.len() {
        return None;
    }
    let sid_len = data[i] as usize;
    i += 1 + sid_len;
    if i >= data.len() {
        return None;
    }
    // DTLS cookie
    let cookie_len = data[i] as usize;
    i += 1 + cookie_len;
    if i + 2 > data.len() {
        return None;
    }
    let cs_len = u16::from_be_bytes([data[i], data[i + 1]]) as usize;
    i += 2;
    if i + cs_len > data.len() || cs_len % 2 != 0 {
        return None;
    }
    let mut ciphers = Vec::new();
    for j in (0..cs_len).step_by(2) {
        let c = u16::from_be_bytes([data[i + j], data[i + j + 1]]);
        // skip GREASE
        if (c & 0x0f0f) != 0x0a0a {
            ciphers.push(format!("{c:04x}"));
        }
    }
    i += cs_len;
    if i >= data.len() {
        return None;
    }
    let comp_len = data[i] as usize;
    i += 1 + comp_len;
    let mut exts = Vec::new();
    if i + 2 <= data.len() {
        let elen = u16::from_be_bytes([data[i], data[i + 1]]) as usize;
        i += 2;
        let end = (i + elen).min(data.len());
        while i + 4 <= end {
            let et = u16::from_be_bytes([data[i], data[i + 1]]);
            let l = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
            i += 4;
            if (et & 0x0f0f) != 0x0a0a {
                exts.push(format!("{et:04x}"));
            }
            i += l;
        }
    }
    let fp = format!(
        "dtls_{:04x}_{}_{}",
        client_ver,
        ciphers.join("-"),
        exts.join("-")
    );
    let mut h = sha2::Sha256::new();
    use sha2::Digest;
    h.update(fp.as_bytes());
    let hash = hex::encode(h.finalize());
    Some((fp, hash[..16.min(hash.len())].to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddrV4;

    #[test]
    fn parse_binding_request_minimal() {
        let mut pkt = vec![0u8; 20];
        pkt[0] = 0x00;
        pkt[1] = 0x01; // Binding Request
        pkt[2] = 0x00;
        pkt[3] = 0x00; // length 0
        pkt[4..8].copy_from_slice(&STUN_MAGIC.to_be_bytes());
        for i in 0..12 {
            pkt[8 + i] = i as u8;
        }
        assert!(looks_like_stun(&pkt));
        let info = parse_stun(&pkt);
        assert!(info.parse_ok);
        assert!(info.is_binding_request);
    }

    #[test]
    fn binding_success_roundtrip_shape() {
        let mut req = vec![0u8; 20];
        req[1] = 0x01;
        req[4..8].copy_from_slice(&STUN_MAGIC.to_be_bytes());
        let mapped = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(1, 2, 3, 4), 5555));
        let resp = build_binding_success(&req, mapped).unwrap();
        assert!(looks_like_stun(&resp));
        let info = parse_stun(&resp);
        assert!(info.is_binding_success);
        assert!(info.xor_mapped_addr.as_deref() == Some("1.2.3.4:5555"));
    }
}
