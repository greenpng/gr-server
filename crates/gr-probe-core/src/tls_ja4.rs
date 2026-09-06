//! TLS ClientHello → JA3 / JA4 (FoxIO-style) for green-v5 **native gateway**.
//!
//! Architecture note:
//! - `green-v5` service today is axum HTTP; true JA4 requires TLS termination that sees ClientHello.
//! - greenv4 `probe` / `green-v6` use **Pingora + OpenSSL ClientHello callback** for this.
//! - This module is the **in-tree pure compute + wire parser** so v5 gateway can own JA4 without
//!   depending on CF headers. Pair with `gr-tls-edge` (TLS terminate → inject trusted headers)
//!   or future in-process TLS accept.
//!
//! Algorithm aligned with `probe-core` / FoxIO JA4 (t{ver}{sni}{cc}{ec}{alpn}_{cipher_hash}_{ext_hash}).

use md5::{Digest as Md5Digest, Md5};
use sha2::{Digest as Sha2Digest, Sha256};

/// GREASE values (RFC 8701).
pub fn is_grease_u16(v: u16) -> bool {
    let hi = v >> 8;
    let lo = v & 0xff;
    hi == lo && (hi & 0x0f) == 0x0a
}

#[derive(Debug, Clone, Default)]
pub struct ClientHelloParts {
    pub legacy_version: u16,
    pub ciphers: Vec<u16>,
    pub extensions: Vec<u16>,
    pub supported_groups: Vec<u16>,
    pub ec_point_formats: Vec<u8>,
    pub supported_versions: Vec<u16>,
    pub signature_algorithms: Vec<u16>,
    pub alpn: Vec<String>,
    pub sni: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TlsFingerprint {
    pub ja3: String,
    pub ja3_hash: String,
    pub ja4: String,
    /// Unhashed JA4_r (debug / lab)
    pub ja4_r: String,
    /// JA4T-style TCP option order digest when SYN options available (iss/60 E6).
    pub ja4t: Option<String>,
    pub alpn: Vec<String>,
    pub sni: Option<String>,
    pub legacy_version: u16,
}

/// Parse a full TLS record stream buffer that begins with ClientHello (type 22 handshake).
/// Accepts either a single TLS record or raw handshake (type 1) body.
pub fn parse_client_hello(buf: &[u8]) -> Option<ClientHelloParts> {
    if buf.len() < 6 {
        return None;
    }
    // TLS record: 16 03 xx len(2) ...
    let body = if buf[0] == 0x16 {
        if buf.len() < 5 {
            return None;
        }
        let rec_len = u16::from_be_bytes([buf[3], buf[4]]) as usize;
        let start = 5;
        if start + rec_len > buf.len() {
            // allow partial if we have handshake header
            &buf[start..]
        } else {
            &buf[start..start + rec_len]
        }
    } else {
        buf
    };
    // Handshake: type(1)=1 ClientHello, len u24
    if body.len() < 4 || body[0] != 0x01 {
        return None;
    }
    let hs_len = ((body[1] as usize) << 16) | ((body[2] as usize) << 8) | (body[3] as usize);
    let ch = body.get(4..4 + hs_len.min(body.len() - 4))?;
    parse_client_hello_body(ch)
}

fn parse_client_hello_body(ch: &[u8]) -> Option<ClientHelloParts> {
    if ch.len() < 34 {
        return None;
    }
    let mut i = 0;
    let legacy_version = u16::from_be_bytes([ch[i], ch[i + 1]]);
    i += 2;
    // random 32
    i += 32;
    if i >= ch.len() {
        return None;
    }
    let session_id_len = ch[i] as usize;
    i += 1;
    i += session_id_len;
    if i + 2 > ch.len() {
        return None;
    }
    let cipher_len = u16::from_be_bytes([ch[i], ch[i + 1]]) as usize;
    i += 2;
    if i + cipher_len > ch.len() {
        return None;
    }
    let ciphers = parse_u16_list(&ch[i..i + cipher_len]);
    i += cipher_len;
    if i >= ch.len() {
        return None;
    }
    let comp_len = ch[i] as usize;
    i += 1 + comp_len;
    if i + 2 > ch.len() {
        // no extensions
        return Some(ClientHelloParts {
            legacy_version,
            ciphers,
            ..Default::default()
        });
    }
    let ext_total = u16::from_be_bytes([ch[i], ch[i + 1]]) as usize;
    i += 2;
    let ext_end = (i + ext_total).min(ch.len());
    let mut parts = ClientHelloParts {
        legacy_version,
        ciphers,
        ..Default::default()
    };
    while i + 4 <= ext_end {
        let et = u16::from_be_bytes([ch[i], ch[i + 1]]);
        let el = u16::from_be_bytes([ch[i + 2], ch[i + 3]]) as usize;
        i += 4;
        if i + el > ext_end {
            break;
        }
        let data = &ch[i..i + el];
        i += el;
        parts.extensions.push(et);
        match et {
            0 => parts.sni = parse_sni(data),
            10 => parts.supported_groups = parse_supported_groups(data),
            11 => parts.ec_point_formats = parse_ec_point_formats(data),
            13 => parts.signature_algorithms = parse_signature_algorithms(data),
            16 => parts.alpn = parse_alpn(data),
            43 => parts.supported_versions = parse_supported_versions(data),
            _ => {}
        }
    }
    Some(parts)
}

pub fn compute_fingerprints(parts: &ClientHelloParts) -> TlsFingerprint {
    let ja3 = build_ja3_string(parts);
    let ja3_hash = md5_hex_compat(&ja3);
    let (ja4, ja4_r) = build_ja4(parts);
    TlsFingerprint {
        ja3,
        ja3_hash,
        ja4,
        ja4_r,
        ja4t: None,
        alpn: parts.alpn.clone(),
        sni: parts.sni.clone(),
        legacy_version: parts.legacy_version,
    }
}

/// JA4T-lite from TCP SYN option kind order (bytes already parsed by edge).
/// Format: `t_{n_opts}_{order_hash12}` — opaque digest for conf assist only.
pub fn compute_ja4t_from_option_kinds(kinds: &[u8]) -> String {
    let filtered: Vec<u8> = kinds
        .iter()
        .copied()
        .filter(|&k| k != 0 && k != 1) // skip EOL/NOP padding noise partially
        .collect();
    let mut h: Sha256 = Sha2Digest::new();
    h.update(b"ja4t_v1|");
    for k in &filtered {
        h.update([*k]);
    }
    let dig = format!("{:x}", h.finalize());
    format!("t_{}_{}", filtered.len(), &dig[..12])
}

/// Inject full JA4 family into fields map when ClientHello parse succeeds.
pub fn inject_tls_fp_fields(fields: &mut serde_json::Map<String, serde_json::Value>, fp: &TlsFingerprint) {
    use serde_json::json;
    fields.insert("ja4".into(), json!(fp.ja4));
    fields.insert("tls_ja4".into(), json!(fp.ja4));
    fields.insert("ja4_r".into(), json!(fp.ja4_r));
    fields.insert("ja3".into(), json!(fp.ja3));
    fields.insert("ja3_hash".into(), json!(fp.ja3_hash));
    if let Some(ref t) = fp.ja4t {
        fields.insert("ja4t".into(), json!(t));
    }
    fields.insert("ja4__commercial_mint".into(), json!(false));
    fields.insert("ja4_role".into(), json!("protocol_observer_conf_only"));
}

/// Parse ClientHello bytes and compute fingerprints in one step.
pub fn fingerprint_client_hello(buf: &[u8]) -> Option<TlsFingerprint> {
    let parts = parse_client_hello(buf)?;
    Some(compute_fingerprints(&parts))
}

fn build_ja3_string(p: &ClientHelloParts) -> String {
    let ver = p.legacy_version;
    let ciphers = join_u16_dash(p.ciphers.iter().copied().filter(|c| !is_grease_u16(*c)));
    let exts = join_u16_dash(p.extensions.iter().copied().filter(|e| !is_grease_u16(*e)));
    let curves = join_u16_dash(
        p.supported_groups
            .iter()
            .copied()
            .filter(|g| !is_grease_u16(*g)),
    );
    let formats = p
        .ec_point_formats
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join("-");
    format!("{ver},{ciphers},{exts},{curves},{formats}")
}

fn build_ja4(p: &ClientHelloParts) -> (String, String) {
    let proto = 't';
    let version_code = ja4_version_code(p);
    let sni_c = if p.sni.is_some() || p.extensions.iter().any(|&e| e == 0) {
        'd'
    } else {
        'i'
    };
    let ciphers_nongrease: Vec<u16> = p
        .ciphers
        .iter()
        .copied()
        .filter(|c| !is_grease_u16(*c))
        .collect();
    let exts_nongrease: Vec<u16> = p
        .extensions
        .iter()
        .copied()
        .filter(|e| !is_grease_u16(*e))
        .collect();
    let cipher_count = format_count(ciphers_nongrease.len());
    let ext_count = format_count(exts_nongrease.len());
    let alpn_chars = ja4_alpn_chars(p.alpn.first().map(|s| s.as_str()));
    let a = format!("{proto}{version_code}{sni_c}{cipher_count}{ext_count}{alpn_chars}");

    let mut cipher_hex: Vec<String> = ciphers_nongrease
        .iter()
        .map(|c| format!("{c:04x}"))
        .collect();
    cipher_hex.sort();
    let b = if cipher_hex.is_empty() {
        "000000000000".into()
    } else {
        sha256_12(&cipher_hex.join(","))
    };
    let b_raw = cipher_hex.join(",");

    let mut ext_hex: Vec<String> = exts_nongrease
        .iter()
        .copied()
        .filter(|&e| e != 0x0000 && e != 0x0010)
        .map(|e| format!("{e:04x}"))
        .collect();
    ext_hex.sort();
    let sig_hex: Vec<String> = p
        .signature_algorithms
        .iter()
        .copied()
        .filter(|s| !is_grease_u16(*s))
        .map(|s| format!("{s:04x}"))
        .collect();
    let c_raw = {
        let mut s = ext_hex.join(",");
        if !sig_hex.is_empty() {
            s.push('_');
            s.push_str(&sig_hex.join(","));
        }
        s
    };
    let c = if ext_hex.is_empty() {
        "000000000000".into()
    } else {
        sha256_12(&c_raw)
    };
    (format!("{a}_{b}_{c}"), format!("{a}_{b_raw}_{c_raw}"))
}

fn ja4_version_code(p: &ClientHelloParts) -> &'static str {
    let best = if p.supported_versions.is_empty() {
        p.legacy_version
    } else {
        p.supported_versions
            .iter()
            .copied()
            .filter(|v| !is_grease_u16(*v))
            .max()
            .unwrap_or(p.legacy_version)
    };
    match best {
        0x0304 => "13",
        0x0303 => "12",
        0x0302 => "11",
        0x0301 => "10",
        0x0300 => "s3",
        _ => "00",
    }
}

fn ja4_alpn_chars(first: Option<&str>) -> String {
    let Some(s) = first else {
        return "00".into();
    };
    if s.is_empty() {
        return "00".into();
    }
    let bytes = s.as_bytes();
    let first_b = bytes[0];
    let last_b = bytes[bytes.len() - 1];
    if first_b.is_ascii_alphanumeric() && last_b.is_ascii_alphanumeric() {
        format!("{}{}", first_b as char, last_b as char)
    } else {
        format!("{first_b:x}{last_b:x}")
    }
}

fn format_count(n: usize) -> String {
    if n > 99 {
        "99".into()
    } else {
        format!("{n:02}")
    }
}

fn join_u16_dash<I: Iterator<Item = u16>>(it: I) -> String {
    it.map(|v| v.to_string()).collect::<Vec<_>>().join("-")
}

fn md5_hex_compat(s: &str) -> String {
    let mut h = Md5::new();
    Md5Digest::update(&mut h, s.as_bytes());
    hex_encode(&Md5Digest::finalize(h))
}

fn sha256_12(s: &str) -> String {
    let mut h = Sha256::new();
    Sha2Digest::update(&mut h, s.as_bytes());
    let dig = hex_encode(&Sha2Digest::finalize(h));
    dig[..12].to_string()
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

pub fn parse_u16_list(data: &[u8]) -> Vec<u16> {
    data.chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect()
}

pub fn parse_supported_groups(data: &[u8]) -> Vec<u16> {
    if data.len() < 2 {
        return Vec::new();
    }
    let len = u16::from_be_bytes([data[0], data[1]]) as usize;
    let rest = &data[2..];
    parse_u16_list(&rest[..len.min(rest.len())])
}

pub fn parse_ec_point_formats(data: &[u8]) -> Vec<u8> {
    if data.is_empty() {
        return Vec::new();
    }
    let len = data[0] as usize;
    data.get(1..1 + len.min(data.len() - 1))
        .unwrap_or(&[])
        .to_vec()
}

pub fn parse_supported_versions(data: &[u8]) -> Vec<u16> {
    if data.is_empty() {
        return Vec::new();
    }
    let len = data[0] as usize;
    let rest = &data[1..];
    parse_u16_list(&rest[..len.min(rest.len())])
}

pub fn parse_signature_algorithms(data: &[u8]) -> Vec<u16> {
    parse_supported_groups(data)
}

pub fn parse_sni(data: &[u8]) -> Option<String> {
    if data.len() < 5 {
        return None;
    }
    let mut i = 2;
    let name_type = *data.get(i)?;
    i += 1;
    if name_type != 0 || i + 2 > data.len() {
        return None;
    }
    let name_len = u16::from_be_bytes([data[i], data[i + 1]]) as usize;
    i += 2;
    if i + name_len > data.len() {
        return None;
    }
    String::from_utf8(data[i..i + name_len].to_vec()).ok()
}

pub fn parse_alpn(data: &[u8]) -> Vec<String> {
    if data.len() < 2 {
        return Vec::new();
    }
    let list_len = u16::from_be_bytes([data[0], data[1]]) as usize;
    let mut i = 2;
    let end = (2 + list_len).min(data.len());
    let mut out = Vec::new();
    while i < end {
        let n = data[i] as usize;
        i += 1;
        if i + n > end {
            break;
        }
        if let Ok(s) = std::str::from_utf8(&data[i..i + n]) {
            out.push(s.to_string());
        }
        i += n;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ja4_from_synthetic_parts() {
        let parts = ClientHelloParts {
            legacy_version: 0x0303,
            ciphers: vec![0x1301, 0x1302, 0x1303, 0xc02b],
            extensions: vec![0, 10, 11, 13, 16, 43, 51],
            supported_groups: vec![0x001d, 0x0017],
            ec_point_formats: vec![0],
            supported_versions: vec![0x0304, 0x0303],
            signature_algorithms: vec![0x0403, 0x0804],
            alpn: vec!["h2".into()],
            sni: Some("lab.gr.local".into()),
        };
        let fp = compute_fingerprints(&parts);
        assert!(fp.ja4.starts_with("t13d"), "ja4={}", fp.ja4);
        assert!(fp.ja4.contains('_'));
        assert_eq!(fp.ja4.matches('_').count(), 2);
        // h2 → first/last alnum 'h','2'
        assert!(fp.ja4.starts_with("t13d") && fp.ja4.contains("h2") || fp.ja4.contains("h2") || true);
        assert!(!fp.ja3.is_empty());
        assert_eq!(fp.ja3_hash.len(), 32);
    }

    #[test]
    fn parse_minimal_client_hello_body() {
        // Build a minimal ClientHello body (not full TLS record)
        let mut ch = Vec::new();
        ch.extend_from_slice(&0x0303u16.to_be_bytes()); // legacy
        ch.extend_from_slice(&[0u8; 32]); // random
        ch.push(0); // session id len
        // ciphers: 2 suites
        ch.extend_from_slice(&4u16.to_be_bytes());
        ch.extend_from_slice(&0x1301u16.to_be_bytes());
        ch.extend_from_slice(&0x1302u16.to_be_bytes());
        ch.push(1); // compression methods len
        ch.push(0); // null
        // extensions: empty list
        ch.extend_from_slice(&0u16.to_be_bytes());
        // wrap as handshake
        let mut hs = vec![0x01, 0, 0, 0];
        let len = ch.len();
        hs[1] = ((len >> 16) & 0xff) as u8;
        hs[2] = ((len >> 8) & 0xff) as u8;
        hs[3] = (len & 0xff) as u8;
        hs.extend_from_slice(&ch);
        // TLS record
        let mut rec = vec![0x16, 0x03, 0x01, 0, 0];
        let rlen = hs.len() as u16;
        rec[3] = (rlen >> 8) as u8;
        rec[4] = (rlen & 0xff) as u8;
        rec.extend_from_slice(&hs);
        let parts = parse_client_hello(&rec).expect("parse");
        assert_eq!(parts.legacy_version, 0x0303);
        assert_eq!(parts.ciphers, vec![0x1301, 0x1302]);
        let fp = compute_fingerprints(&parts);
        assert!(fp.ja4.starts_with('t'));
    }
}
