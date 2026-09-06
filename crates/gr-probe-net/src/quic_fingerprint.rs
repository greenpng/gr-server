//! QUIC Initial packet listen / fingerprint (HTTP/3 path).
//!
//! Parses the unencrypted long header, then attempts RFC 9001 Initial AEAD
//! decrypt to recover the real TLS ClientHello (JA3/JA4 over HTTP/3).

use crate::tls_fingerprint::{
    parse_ec_point_formats, parse_signature_algorithms, parse_sni, parse_supported_groups,
    parse_supported_versions,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// QUIC version 1
pub const QUIC_V1: u32 = 0x00000001;
/// Draft/google variants we still label
pub const QUIC_V2: u32 = 0x6b3343cf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QuicInitialInfo {
    pub parse_ok: bool,
    pub version: Option<u32>,
    pub version_hex: Option<String>,
    pub version_label: Option<String>,
    pub long_header: bool,
    pub packet_type: Option<String>,
    pub fixed_bit: Option<bool>,
    pub spin_bit: Option<bool>, // for short header future
    pub dcid_len: Option<u8>,
    pub dcid_hex: Option<String>,
    pub scid_len: Option<u8>,
    pub scid_hex: Option<String>,
    pub token_len: Option<usize>,
    pub token_present: bool,
    /// Token bytes hex (capped) when present
    pub token_hex: Option<String>,
    pub length: Option<u64>,
    pub packet_number_len: Option<u8>,
    /// Packet number after header protection removal (AEAD path)
    pub packet_number: Option<u64>,
    /// Whether Initial AEAD decrypt succeeded
    pub aead_ok: bool,
    /// Reassembled CRYPTO stream length after AEAD
    pub crypto_len: Option<usize>,
    pub datagram_len: usize,
    pub src_ip: Option<String>,
    pub src_port: Option<u16>,
    pub dst_ip: Option<String>,
    pub dst_port: Option<u16>,
    /// Fingerprint string for correlation (header-only + optional TLS)
    pub quic_fp: Option<String>,
    pub quic_fp_hash: Option<String>,
    /// Decrypted TLS ClientHello JA3/JA4 when Initial AEAD succeeds
    pub tls_ja3: Option<String>,
    pub tls_ja3_hash: Option<String>,
    pub tls_ja4: Option<String>,
    pub sni: Option<String>,
    pub alpn: Vec<String>,
    /// key_share groups from AEAD ClientHello
    pub key_share_groups: Vec<u16>,
    pub key_share_groups_hex: Vec<String>,
    /// QUIC transport parameters from CH ext 57
    pub transport_params: Vec<crate::tls_ext::QuicTransportParam>,
    pub transport_params_summary: Option<String>,
    /// name → int/hex summary for join
    pub transport_params_map: std::collections::BTreeMap<String, String>,
    pub psk_modes_names: Vec<String>,
    pub has_ech: bool,
    pub is_version_negotiation: bool,
    pub is_retry: bool,
    pub supported_versions_offered: Vec<String>,
    pub notes: Vec<String>,
    pub raw_header_hex: Option<String>,
}

/// Parse a UDP datagram that may contain a QUIC long-header Initial packet.
pub fn parse_quic_datagram(data: &[u8]) -> QuicInitialInfo {
    let mut info = QuicInitialInfo {
        datagram_len: data.len(),
        ..Default::default()
    };
    if data.is_empty() {
        info.notes.push("empty datagram".into());
        return info;
    }
    let first = data[0];
    let long = (first & 0x80) != 0;
    info.long_header = long;
    if !long {
        info.notes.push("short header (1-RTT) — not Initial".into());
        info.packet_type = Some("short".into());
        return info;
    }
    // Long header: 1 type byte + 4 version + DCIL + DCID + SCIL + SCID + ...
    if data.len() < 6 {
        info.notes.push("long header too short".into());
        return info;
    }
    let version = u32::from_be_bytes([data[1], data[2], data[3], data[4]]);
    info.version = Some(version);
    info.version_hex = Some(format!("{version:08x}"));
    info.version_label = Some(version_label(version).into());
    info.fixed_bit = Some((first & 0x40) != 0);
    // Version Negotiation: version field is 0
    if version == 0 {
        info.is_version_negotiation = true;
        info.packet_type = Some("VersionNegotiation".into());
        info.parse_ok = true;
        // Remaining after DCIDs are supported version list (u32s)
        // parsed after cid section below
    }
    let ptype = (first & 0x30) >> 4;
    if !info.is_version_negotiation {
        info.packet_type = Some(match ptype {
            0 => "Initial".into(),
            1 => "0-RTT".into(),
            2 => "Handshake".into(),
            3 => {
                info.is_retry = true;
                "Retry".into()
            }
            _ => format!("type_{ptype}"),
        });
    }    let pn_len_bits = (first & 0x03) + 1;
    info.packet_number_len = Some(pn_len_bits);

    let mut i = 5;
    if i >= data.len() {
        return info;
    }
    let dcid_len = data[i] as usize;
    i += 1;
    info.dcid_len = Some(dcid_len as u8);
    if i + dcid_len > data.len() {
        info.notes.push("truncated DCID".into());
        return info;
    }
    info.dcid_hex = Some(hex::encode(&data[i..i + dcid_len]));
    i += dcid_len;
    if i >= data.len() {
        return info;
    }
    let scid_len = data[i] as usize;
    i += 1;
    info.scid_len = Some(scid_len as u8);
    if i + scid_len > data.len() {
        info.notes.push("truncated SCID".into());
        return info;
    }
    info.scid_hex = Some(hex::encode(&data[i..i + scid_len]));
    i += scid_len;

    if info.is_version_negotiation {
        let mut vers = Vec::new();
        while i + 4 <= data.len() && vers.len() < 16 {
            let v = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
            vers.push(format!("{v:08x}"));
            i += 4;
        }
        info.supported_versions_offered = vers;
        info.notes.push("QUIC Version Negotiation observed".into());
        return info;
    }
    if info.is_retry {
        info.parse_ok = true;
        info.notes.push("QUIC Retry observed (no AEAD ClientHello)".into());
        // Retry has token after SCIDs; leave rest unparsed
        return info;
    }

    // Initial-specific: token (varint length) + length (varint) + packet number + payload
    // Also treat QUIC v2 Initial (ptype==1) the same for token/length layout.
    let is_initial_layout = ptype == 0 || (version == QUIC_V2 && ptype == 1);
    if is_initial_layout {
        match read_varint(data, i) {
            Some((token_len, ni)) => {
                info.token_len = Some(token_len as usize);
                info.token_present = token_len > 0;
                i = ni;
                if i + token_len as usize > data.len() {
                    info.notes.push("truncated token".into());
                    return info;
                }
                if token_len > 0 {
                    let take = (token_len as usize).min(64);
                    info.token_hex = Some(hex::encode(&data[i..i + take]));
                    if token_len as usize > 64 {
                        info.notes.push(format!(
                            "token truncated in hex store ({token_len}B → 64B)"
                        ));
                    }
                }
                i += token_len as usize;
            }
            None => {
                info.notes.push("bad token varint".into());
                return info;
            }
        }
        match read_varint(data, i) {
            Some((length, ni)) => {
                info.length = Some(length);
                i = ni;
            }
            None => {
                info.notes.push("bad length varint".into());
                return info;
            }
        }
    }

    info.raw_header_hex = Some(hex::encode(&data[..i.min(data.len()).min(128)]));
    info.parse_ok = true;

    // Fingerprint from unencrypted header fields (always available)
    let fp = format!(
        "q{:08x}_d{}_s{}_t{}_pn{}_sz{}",
        version,
        info.dcid_len.unwrap_or(0),
        info.scid_len.unwrap_or(0),
        if info.token_present { 1 } else { 0 },
        info.packet_number_len.unwrap_or(0),
        data.len()
    );
    let mut h = Sha256::new();
    h.update(fp.as_bytes());
    info.quic_fp = Some(fp);
    info.quic_fp_hash = Some(hex::encode(h.finalize())[..16].to_string());

    // Initial AEAD → real ClientHello (QUIC v1 / v2)
    if ptype == 0 || (version == QUIC_V2 && ptype == 1) {
        crate::quic_decrypt::enrich_quic_with_aead(data, &mut info);
    }
    info
}

fn version_label(v: u32) -> &'static str {
    match v {
        QUIC_V1 => "QUIC v1",
        QUIC_V2 => "QUIC v2",
        0x51303334 => "Q034(gquic)",
        0x51303530 => "Q050(gquic)",
        _ => "unknown/draft",
    }
}

/// Read QUIC variable-length integer (RFC 9000 §16). Returns (value, new_offset).
pub fn read_varint(data: &[u8], offset: usize) -> Option<(u64, usize)> {
    if offset >= data.len() {
        return None;
    }
    let b = data[offset];
    let prefix = b >> 6;
    let len = 1usize << prefix;
    if offset + len > data.len() {
        return None;
    }
    let mut v = (b & 0x3f) as u64;
    for j in 1..len {
        v = (v << 8) | data[offset + j] as u64;
    }
    Some((v, offset + len))
}

/// Parse TLS ClientHello from plaintext bytes (after CRYPTO frame extract).
/// Lite variant kept for callers that only need ciphers/sni/alpn.
pub fn try_parse_tls_client_hello(crypto: &[u8]) -> Option<TlsHelloLite> {
    try_parse_tls_client_hello_full(crypto).map(|f| TlsHelloLite {
        legacy_version: f.legacy_version,
        ciphers: f.ciphers,
        extensions: f.extensions,
        sni: f.sni,
        alpn: f.alpn,
    })
}

/// Full ClientHello parse for JA3/JA4 (groups, formats, versions, sigalgs).
pub fn try_parse_tls_client_hello_full(crypto: &[u8]) -> Option<TlsHelloFull> {
    // TLS handshake: type(1)=1 ClientHello, len(3), legacy_version(2), random(32), ...
    if crypto.len() < 4 {
        return None;
    }
    // May be nested in TLS record: type 22 handshake
    let mut p = crypto;
    if p[0] == 0x16 && p.len() > 5 {
        p = &p[5..];
    }
    if p.is_empty() || p[0] != 0x01 {
        // QUIC CRYPTO carries raw handshake messages (no record layer) — also
        // tolerate leading noise by scanning for handshake type 0x01 with plausible len.
        if let Some(idx) = find_client_hello(p) {
            p = &p[idx..];
        } else {
            return None;
        }
    }
    if p.len() < 38 {
        return None;
    }
    let body = &p[4..];
    if body.len() < 34 {
        return None;
    }
    let legacy_version = u16::from_be_bytes([body[0], body[1]]);
    let mut i = 2 + 32; // version + random
    if i >= body.len() {
        return None;
    }
    let sid_len = body[i] as usize;
    i += 1 + sid_len;
    if i + 2 > body.len() {
        return None;
    }
    let cs_len = u16::from_be_bytes([body[i], body[i + 1]]) as usize;
    i += 2;
    if i + cs_len > body.len() {
        return None;
    }
    let ciphers: Vec<u16> = body[i..i + cs_len]
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    i += cs_len;
    if i >= body.len() {
        return None;
    }
    let comp_len = body[i] as usize;
    i += 1 + comp_len;
    if i + 2 > body.len() {
        return Some(TlsHelloFull {
            legacy_version,
            ciphers,
            extensions: vec![],
            supported_groups: vec![],
            ec_point_formats: vec![],
            supported_versions: vec![],
            signature_algorithms: vec![],
            sni: None,
            alpn: vec![],
        });
    }
    let ext_len = u16::from_be_bytes([body[i], body[i + 1]]) as usize;
    i += 2;
    let ext_end = (i + ext_len).min(body.len());
    let mut extensions = Vec::new();
    let mut sni = None;
    let mut alpn = Vec::new();
    let mut supported_groups = Vec::new();
    let mut ec_point_formats = Vec::new();
    let mut supported_versions = Vec::new();
    let mut signature_algorithms = Vec::new();
    while i + 4 <= ext_end {
        let et = u16::from_be_bytes([body[i], body[i + 1]]);
        let el = u16::from_be_bytes([body[i + 2], body[i + 3]]) as usize;
        i += 4;
        if i + el > ext_end {
            break;
        }
        let ed = &body[i..i + el];
        extensions.push(et);
        match et {
            0 => sni = parse_sni(ed),
            10 => supported_groups = parse_supported_groups(ed),
            11 => ec_point_formats = parse_ec_point_formats(ed),
            13 => signature_algorithms = parse_signature_algorithms(ed),
            16 => {
                // ALPN
                let mut j = 2;
                while j < ed.len() {
                    let l = ed[j] as usize;
                    j += 1;
                    if j + l > ed.len() {
                        break;
                    }
                    if let Ok(s) = std::str::from_utf8(&ed[j..j + l]) {
                        alpn.push(s.to_string());
                    }
                    j += l;
                }
            }
            43 => supported_versions = parse_supported_versions(ed),
            _ => {}
        }
        i += el;
    }
    Some(TlsHelloFull {
        legacy_version,
        ciphers,
        extensions,
        supported_groups,
        ec_point_formats,
        supported_versions,
        signature_algorithms,
        sni,
        alpn,
    })
}

fn find_client_hello(p: &[u8]) -> Option<usize> {
    for idx in 0..p.len().saturating_sub(4) {
        if p[idx] != 0x01 {
            continue;
        }
        let hs_len = ((p[idx + 1] as usize) << 16)
            | ((p[idx + 2] as usize) << 8)
            | (p[idx + 3] as usize);
        // ClientHello is typically hundreds of bytes; reject tiny/huge noise hits
        if hs_len >= 38 && hs_len < 16 * 1024 && idx + 4 + hs_len.min(38) <= p.len() {
            // legacy_version often 0x0303
            if idx + 6 <= p.len() {
                let ver = u16::from_be_bytes([p[idx + 4], p[idx + 5]]);
                if ver == 0x0303 || ver == 0x0301 || ver == 0x0302 {
                    return Some(idx);
                }
            }
        }
    }
    None
}

#[derive(Debug, Clone)]
pub struct TlsHelloLite {
    pub legacy_version: u16,
    pub ciphers: Vec<u16>,
    pub extensions: Vec<u16>,
    pub sni: Option<String>,
    pub alpn: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TlsHelloFull {
    pub legacy_version: u16,
    pub ciphers: Vec<u16>,
    pub extensions: Vec<u16>,
    pub supported_groups: Vec<u16>,
    pub ec_point_formats: Vec<u8>,
    pub supported_versions: Vec<u16>,
    pub signature_algorithms: Vec<u16>,
    pub sni: Option<String>,
    pub alpn: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_encoding() {
        assert_eq!(read_varint(&[0x05], 0), Some((5, 1)));
        assert_eq!(read_varint(&[0x40, 0x64], 0), Some((100, 2)));
    }

    #[test]
    fn parse_minimal_long_header() {
        // Craft minimal long header Initial-like
        let mut p = vec![0xc0]; // long + fixed + initial + pnlen1
        p.extend_from_slice(&1u32.to_be_bytes()); // v1
        p.push(4); // dcid len
        p.extend_from_slice(&[1, 2, 3, 4]);
        p.push(0); // scid len
        p.push(0); // token len varint 0
        p.push(0x10); // length varint small
        let info = parse_quic_datagram(&p);
        assert!(info.long_header);
        assert_eq!(info.version, Some(1));
        assert_eq!(info.dcid_len, Some(4));
        assert!(info.quic_fp.is_some());
    }
}
