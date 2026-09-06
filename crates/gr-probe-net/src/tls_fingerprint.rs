//! JA3 / JA4 computation from parsed ClientHello fields.
//!
//! JA3: Salesforce classic (MD5 of version,ciphers,exts,curves,formats)
//! JA4: FoxIO JA4_a_b_c format (see FoxIO-LLC/ja4 technical details)

use crate::tls_ext::{parse_client_hello_ext_detail, ClientHelloExtDetail};
use md5::{Digest as Md5Digest, Md5};
use serde::{Deserialize, Serialize};
use sha2::{Digest as Sha2Digest, Sha256};

/// GREASE values (RFC 8701) — ignored in fingerprints.
pub fn is_grease_u16(v: u16) -> bool {
    let hi = v >> 8;
    let lo = v & 0xff;
    hi == lo && (hi & 0x0f) == 0x0a
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClientHelloFingerprint {
    pub ja3: String,
    pub ja3_hash: String,
    pub ja4: String,
    pub ja4_r: Option<String>,
    pub legacy_version: u16,
    pub ciphers: Vec<u16>,
    pub extensions: Vec<u16>,
    pub supported_groups: Vec<u16>,
    pub ec_point_formats: Vec<u8>,
    pub supported_versions: Vec<u16>,
    pub signature_algorithms: Vec<u16>,
    pub alpn: Vec<String>,
    pub sni: Option<String>,
    pub has_sni: bool,
    /// Wire cipher suite bytes as received
    pub ciphers_wire: Vec<u8>,
    /// Extension payloads (type, raw bytes)
    pub extension_payloads: Vec<(u16, Vec<u8>)>,
    pub raw_dump_sha256: Option<String>,
    pub raw_dump_b64: Option<String>,
    /// Structured high-signal extension fields (listen depth)
    pub ext_detail: ClientHelloExtDetail,
}

#[derive(Debug, Clone, Default)]
pub struct ClientHelloParts {
    /// legacy_version from ClientHello (e.g. 0x0303)
    pub legacy_version: u16,
    pub ciphers: Vec<u16>,
    /// extension types in order of appearance
    pub extensions: Vec<u16>,
    /// extension 10 / supported_groups (elliptic curves)
    pub supported_groups: Vec<u16>,
    /// extension 11 ec_point_formats
    pub ec_point_formats: Vec<u8>,
    /// extension 43 supported_versions
    pub supported_versions: Vec<u16>,
    /// extension 13 signature_algorithms
    pub signature_algorithms: Vec<u16>,
    /// first ALPN protocol strings
    pub alpn: Vec<String>,
    pub sni: Option<String>,
}

pub fn compute_fingerprints(parts: &ClientHelloParts) -> ClientHelloFingerprint {
    compute_fingerprints_with_raw(parts, &[], &[], false)
}

/// Build fingerprints and optional raw dump from wire pieces.
pub fn compute_fingerprints_with_raw(
    parts: &ClientHelloParts,
    ciphers_wire: &[u8],
    extension_payloads: &[(u16, Vec<u8>)],
    store_raw_b64: bool,
) -> ClientHelloFingerprint {
    let ja3 = build_ja3_string(parts);
    let ja3_hash = md5_hex(&ja3);
    let (ja4, ja4_r) = build_ja4(parts);

    // Assembled dump for stable hashing / optional storage (not full wire ClientHello
    // but all captured field bytes in a deterministic layout).
    let mut dump = Vec::new();
    dump.extend_from_slice(&parts.legacy_version.to_be_bytes());
    dump.extend_from_slice(ciphers_wire);
    for (t, p) in extension_payloads {
        dump.extend_from_slice(&t.to_be_bytes());
        dump.extend_from_slice(&(p.len() as u16).to_be_bytes());
        dump.extend_from_slice(p);
    }
    let mut h = Sha256::new();
    h.update(&dump);
    let raw_sha = hex::encode(h.finalize());

    let ext_detail = parse_client_hello_ext_detail(extension_payloads);

    ClientHelloFingerprint {
        ja3,
        ja3_hash,
        ja4,
        ja4_r: Some(ja4_r),
        legacy_version: parts.legacy_version,
        ciphers: parts.ciphers.clone(),
        extensions: parts.extensions.clone(),
        supported_groups: parts.supported_groups.clone(),
        ec_point_formats: parts.ec_point_formats.clone(),
        supported_versions: parts.supported_versions.clone(),
        signature_algorithms: parts.signature_algorithms.clone(),
        alpn: parts.alpn.clone(),
        sni: parts.sni.clone(),
        has_sni: parts.sni.is_some(),
        ciphers_wire: ciphers_wire.to_vec(),
        extension_payloads: extension_payloads.to_vec(),
        raw_dump_sha256: Some(raw_sha),
        raw_dump_b64: if store_raw_b64 {
            Some(simple_b64(&dump))
        } else {
            None
        },
        ext_detail,
    }
}

fn simple_b64(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[((n >> 18) & 63) as usize]);
        out.push(T[((n >> 12) & 63) as usize]);
        out.push(if chunk.len() > 1 {
            T[((n >> 6) & 63) as usize]
        } else {
            b'='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize]
        } else {
            b'='
        });
    }
    String::from_utf8(out).unwrap_or_default()
}

/// JA3 string: SSLVersion,Cipher,SSLExtension,EllipticCurve,EllipticCurvePointFormat
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
    // Protocol: t = TLS over TCP (we only support this path)
    let proto = 't';

    // Version: highest from supported_versions (ext 43), else legacy
    let version_code = ja4_version_code(p);

    // SNI: d if present else i
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

    // b: sorted cipher hex list hash
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
    let b_raw = if cipher_hex.is_empty() {
        String::new()
    } else {
        cipher_hex.join(",")
    };

    // c: sorted extensions (minus SNI 0000 and ALPN 0010) + "_" + sigalgs in order
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

    let c = if ext_hex.is_empty() {
        "000000000000".into()
    } else {
        let mut s = ext_hex.join(",");
        if !sig_hex.is_empty() {
            s.push('_');
            s.push_str(&sig_hex.join(","));
        }
        sha256_12(&s)
    };
    let c_raw = {
        let mut s = ext_hex.join(",");
        if !sig_hex.is_empty() {
            s.push('_');
            s.push_str(&sig_hex.join(","));
        }
        s
    };

    let ja4 = format!("{a}_{b}_{c}");
    let ja4_r = format!("{a}_{b_raw}_{c_raw}");
    (ja4, ja4_r)
}

fn ja4_version_code(p: &ClientHelloParts) -> &'static str {
    let mut best = p
        .supported_versions
        .iter()
        .copied()
        .filter(|v| !is_grease_u16(*v))
        .max()
        .unwrap_or(p.legacy_version);
    if p.supported_versions.is_empty() {
        best = p.legacy_version;
    }
    match best {
        0x0304 => "13",
        0x0303 => "12",
        0x0302 => "11",
        0x0301 => "10",
        0x0300 => "s3",
        0x0002 => "s2",
        0xfeff => "d1",
        0xfefd => "d2",
        0xfefc => "d3",
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
    let alnum = |b: u8| b.is_ascii_alphanumeric();
    if alnum(first_b) && alnum(last_b) {
        format!("{}{}", first_b as char, last_b as char)
    } else {
        // hex of first and last bytes
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

fn md5_hex(s: &str) -> String {
    let mut h = Md5::new();
    Md5Digest::update(&mut h, s.as_bytes());
    hex::encode(Md5Digest::finalize(h))
}

fn sha256_12(s: &str) -> String {
    let mut h = Sha256::new();
    Sha2Digest::update(&mut h, s.as_bytes());
    let dig = hex::encode(Sha2Digest::finalize(h));
    dig[..12].to_string()
}

/// Parse uint16 list from big-endian wire bytes.
pub fn parse_u16_list(data: &[u8]) -> Vec<u16> {
    data.chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect()
}

/// Parse TLS extension 10 (supported_groups): uint16 length + list of uint16
pub fn parse_supported_groups(data: &[u8]) -> Vec<u16> {
    if data.len() < 2 {
        return Vec::new();
    }
    let len = u16::from_be_bytes([data[0], data[1]]) as usize;
    let rest = &data[2..];
    let take = len.min(rest.len());
    parse_u16_list(&rest[..take])
}

/// Parse extension 11 ec_point_formats: uint8 length + bytes
pub fn parse_ec_point_formats(data: &[u8]) -> Vec<u8> {
    if data.is_empty() {
        return Vec::new();
    }
    let len = data[0] as usize;
    let rest = &data[1..];
    rest.get(..len.min(rest.len())).unwrap_or(&[]).to_vec()
}

/// Parse extension 43 supported_versions
pub fn parse_supported_versions(data: &[u8]) -> Vec<u16> {
    if data.is_empty() {
        return Vec::new();
    }
    // ClientHello: uint8 length + versions
    let len = data[0] as usize;
    let rest = &data[1..];
    parse_u16_list(&rest[..len.min(rest.len())])
}

/// Parse extension 13 signature_algorithms: uint16 length + pairs
pub fn parse_signature_algorithms(data: &[u8]) -> Vec<u16> {
    parse_supported_groups(data) // same wire layout as supported_groups
}

/// Parse SNI extension (0)
pub fn parse_sni(data: &[u8]) -> Option<String> {
    if data.len() < 5 {
        return None;
    }
    // uint16 list_len, uint8 type=0, uint16 name_len, name
    let mut i = 2; // skip list length
    if i >= data.len() {
        return None;
    }
    let name_type = data[i];
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

/// Parse ALPN extension (16)
pub fn parse_alpn(data: &[u8]) -> Vec<String> {
    if data.len() < 2 {
        return Vec::new();
    }
    let list_len = u16::from_be_bytes([data[0], data[1]]) as usize;
    let mut i = 2;
    let end = (2 + list_len).min(data.len());
    let mut out = Vec::new();
    while i < end {
        let l = data[i] as usize;
        i += 1;
        if i + l > end {
            break;
        }
        if let Ok(s) = std::str::from_utf8(&data[i..i + l]) {
            out.push(s.to_string());
        }
        i += l;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grease_detection() {
        assert!(is_grease_u16(0x0a0a));
        assert!(is_grease_u16(0x1a1a));
        assert!(!is_grease_u16(0x1301));
        assert!(!is_grease_u16(0x0000));
    }

    #[test]
    fn ja3_basic_shape() {
        let parts = ClientHelloParts {
            legacy_version: 771,
            ciphers: vec![0x1301, 0x1302, 0x0a0a], // grease ignored
            extensions: vec![0, 10, 11, 0x1a1a],
            supported_groups: vec![23, 24],
            ec_point_formats: vec![0],
            ..Default::default()
        };
        let fp = compute_fingerprints(&parts);
        assert!(fp.ja3.starts_with("771,"));
        assert!(!fp.ja3.contains("2570")); // 0x0a0a grease not in ciphers
        assert_eq!(fp.ja3_hash.len(), 32);
        assert!(fp.ja4.starts_with('t'));
        assert_eq!(fp.ja4.matches('_').count(), 2);
    }

    #[test]
    fn ja4_alpn_h2() {
        assert_eq!(ja4_alpn_chars(Some("h2")), "h2");
        assert_eq!(ja4_alpn_chars(Some("http/1.1")), "h1");
        assert_eq!(ja4_alpn_chars(None), "00");
    }

    #[test]
    fn parse_sni_basic() {
        // list_len=0x0009, type=0, name_len=0x0006, "foo.com" wait 7 chars for example.com
        // example.com = 11 chars
        let mut data = vec![0x00, 0x0e, 0x00, 0x00, 0x0b];
        data.extend_from_slice(b"example.com");
        assert_eq!(parse_sni(&data).as_deref(), Some("example.com"));
    }
}
