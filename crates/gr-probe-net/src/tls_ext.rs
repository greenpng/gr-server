//! Structured TLS ClientHello extension parsers (listen depth).
//!
//! Covers high-signal extensions beyond basic JA3 groups:
//! - key_share (51)
//! - psk_key_exchange_modes (45)
//! - quic_transport_parameters (57)
//! - compress_certificate (27)
//! - signature_algorithms_cert (50)
//! - application_settings / ALPS (17513) presence
//! - encrypted_client_hello (65037) presence

use crate::quic_fingerprint::read_varint;
use serde::{Deserialize, Serialize};

/// One QUIC transport parameter (RFC 9000 §18).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuicTransportParam {
    pub id: u64,
    pub id_hex: String,
    pub name: String,
    pub value_len: usize,
    /// Integer decode when length is 0/1/2/4/8 (common for limits)
    pub value_int: Option<u64>,
    /// Hex of value, capped
    pub value_hex: Option<String>,
}

/// Structured listen fields derived from ClientHello extensions.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClientHelloExtDetail {
    /// Named groups offered in key_share (order preserved, includes GREASE)
    pub key_share_groups: Vec<u16>,
    pub key_share_groups_hex: Vec<String>,
    /// Per-entry key exchange lengths (group, kx_len)
    pub key_share_entry_lens: Vec<(u16, usize)>,
    /// PSK key exchange modes (0=psk_ke, 1=psk_dhe_ke)
    pub psk_modes: Vec<u8>,
    pub psk_modes_names: Vec<String>,
    /// Certificate compression algorithms (RFC 8879)
    pub cert_compression_algs: Vec<u16>,
    pub cert_compression_algs_hex: Vec<String>,
    /// signature_algorithms_cert
    pub signature_algorithms_cert: Vec<u16>,
    /// QUIC transport parameters (ext 57) — empty on pure TCP TLS unless h3-capable CH
    pub quic_transport_params: Vec<QuicTransportParam>,
    /// Compact summary string for fingerprints / reports
    pub quic_tp_summary: Option<String>,
    pub has_session_ticket: bool,
    pub has_pre_shared_key: bool,
    pub has_early_data: bool,
    pub has_cookie: bool,
    pub has_post_handshake_auth: bool,
    pub has_encrypted_client_hello: bool,
    pub has_application_settings: bool,
    /// ALPS protocol names when present (e.g. h2)
    pub alps_protocols: Vec<String>,
    pub has_record_size_limit: bool,
    pub record_size_limit: Option<u16>,
    pub has_status_request: bool,
    pub has_signed_cert_timestamp: bool,
    pub has_server_name: bool,
    /// Extension type order as comma-joined hex (browser-stack signal)
    pub extension_order_hex: String,
    pub extension_presence: Vec<String>,
}

/// Parse listen-relevant extensions from (type, payload) list.
pub fn parse_client_hello_ext_detail(payloads: &[(u16, Vec<u8>)]) -> ClientHelloExtDetail {
    let mut d = ClientHelloExtDetail::default();
    d.extension_order_hex = payloads
        .iter()
        .map(|(t, _)| format!("{t:04x}"))
        .collect::<Vec<_>>()
        .join(",");
    for (et, data) in payloads {
        match *et {
            0 => {
                d.has_server_name = true;
                d.extension_presence.push("server_name".into());
            }
            5 => {
                d.has_status_request = true;
                d.extension_presence.push("status_request".into());
            }
            18 => {
                d.has_signed_cert_timestamp = true;
                d.extension_presence.push("signed_cert_timestamp".into());
            }
            27 => {
                d.cert_compression_algs = parse_cert_compression(data);
                d.cert_compression_algs_hex = d
                    .cert_compression_algs
                    .iter()
                    .map(|a| format!("{a:04x}"))
                    .collect();
                d.extension_presence.push("compress_certificate".into());
            }
            28 => {
                d.has_record_size_limit = true;
                if data.len() >= 2 {
                    d.record_size_limit =
                        Some(u16::from_be_bytes([data[0], data[1]]));
                }
                d.extension_presence.push("record_size_limit".into());
            }
            35 => {
                d.has_session_ticket = true;
                d.extension_presence.push("session_ticket".into());
            }
            41 => {
                d.has_pre_shared_key = true;
                d.extension_presence.push("pre_shared_key".into());
            }
            42 => {
                d.has_early_data = true;
                d.extension_presence.push("early_data".into());
            }
            44 => {
                d.has_cookie = true;
                d.extension_presence.push("cookie".into());
            }
            45 => {
                d.psk_modes = parse_psk_modes(data);
                d.psk_modes_names = d
                    .psk_modes
                    .iter()
                    .map(|m| match m {
                        0 => "psk_ke".into(),
                        1 => "psk_dhe_ke".into(),
                        _ => format!("mode_{m}"),
                    })
                    .collect();
                d.extension_presence.push("psk_key_exchange_modes".into());
            }
            49 => {
                d.has_post_handshake_auth = true;
                d.extension_presence.push("post_handshake_auth".into());
            }
            50 => {
                d.signature_algorithms_cert = parse_u16_list_len_prefixed(data);
                d.extension_presence
                    .push("signature_algorithms_cert".into());
            }
            51 => {
                let (groups, lens) = parse_key_share(data);
                d.key_share_groups = groups;
                d.key_share_groups_hex = d
                    .key_share_groups
                    .iter()
                    .map(|g| format!("{g:04x}"))
                    .collect();
                d.key_share_entry_lens = lens;
                d.extension_presence.push("key_share".into());
            }
            57 => {
                d.quic_transport_params = parse_quic_transport_params(data);
                d.quic_tp_summary = Some(summarize_quic_tp(&d.quic_transport_params));
                d.extension_presence
                    .push("quic_transport_parameters".into());
            }
            // ALPS (draft) 17513 = 0x4469
            17513 => {
                d.has_application_settings = true;
                d.alps_protocols = parse_alps_protocols(data);
                d.extension_presence.push("application_settings".into());
            }
            // ECH 0xfe0d = 65037
            65037 => {
                d.has_encrypted_client_hello = true;
                d.extension_presence
                    .push("encrypted_client_hello".into());
            }
            _ => {}
        }
    }
    d
}

/// ALPS extension: ProtocolNameList (same shape as ALPN).
fn parse_alps_protocols(data: &[u8]) -> Vec<String> {
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

fn parse_u16_list_len_prefixed(data: &[u8]) -> Vec<u16> {
    if data.len() < 2 {
        return Vec::new();
    }
    let len = u16::from_be_bytes([data[0], data[1]]) as usize;
    let rest = &data[2..];
    let take = len.min(rest.len());
    rest[..take]
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect()
}

/// Certificate compression algorithms: uint8 length + list of uint16.
fn parse_cert_compression(data: &[u8]) -> Vec<u16> {
    if data.is_empty() {
        return Vec::new();
    }
    let len = data[0] as usize;
    let rest = &data[1..];
    let take = len.min(rest.len());
    rest[..take]
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect()
}

fn parse_psk_modes(data: &[u8]) -> Vec<u8> {
    if data.is_empty() {
        return Vec::new();
    }
    let len = data[0] as usize;
    let rest = &data[1..];
    rest.get(..len.min(rest.len())).unwrap_or(&[]).to_vec()
}

/// KeyShareClientHello → (groups in order, (group, kx_len) entries).
pub fn parse_key_share(data: &[u8]) -> (Vec<u16>, Vec<(u16, usize)>) {
    if data.len() < 2 {
        return (Vec::new(), Vec::new());
    }
    let list_len = u16::from_be_bytes([data[0], data[1]]) as usize;
    let mut i = 2;
    let end = (2 + list_len).min(data.len());
    let mut groups = Vec::new();
    let mut lens = Vec::new();
    while i + 4 <= end {
        let group = u16::from_be_bytes([data[i], data[i + 1]]);
        let kx_len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        i += 4;
        if i + kx_len > end {
            break;
        }
        groups.push(group);
        lens.push((group, kx_len));
        i += kx_len;
    }
    (groups, lens)
}

/// Parse QUIC transport_parameters extension payload.
pub fn parse_quic_transport_params(data: &[u8]) -> Vec<QuicTransportParam> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < data.len() {
        let (id, ni) = match read_varint(data, i) {
            Some(v) => v,
            None => break,
        };
        i = ni;
        let (len, ni) = match read_varint(data, i) {
            Some(v) => v,
            None => break,
        };
        i = ni;
        let len = len as usize;
        if i + len > data.len() {
            break;
        }
        let val = &data[i..i + len];
        i += len;
        let value_int = decode_tp_int(val);
        let value_hex = if !val.is_empty() {
            Some(hex::encode(&val[..val.len().min(32)]))
        } else {
            None
        };
        out.push(QuicTransportParam {
            id,
            id_hex: format!("{id:x}"),
            name: quic_tp_name(id).into(),
            value_len: len,
            value_int,
            value_hex,
        });
        if out.len() >= 64 {
            break;
        }
    }
    out
}

fn decode_tp_int(val: &[u8]) -> Option<u64> {
    match val.len() {
        0 => Some(0),
        1 => Some(val[0] as u64),
        2 => Some(u16::from_be_bytes([val[0], val[1]]) as u64),
        4 => Some(u32::from_be_bytes([val[0], val[1], val[2], val[3]]) as u64),
        8 => Some(u64::from_be_bytes([
            val[0], val[1], val[2], val[3], val[4], val[5], val[6], val[7],
        ])),
        // varint-encoded integers inside some params — try QUIC varint
        _ => read_varint(val, 0).map(|(v, _)| v),
    }
}

fn quic_tp_name(id: u64) -> &'static str {
    match id {
        0x00 => "original_destination_connection_id",
        0x01 => "max_idle_timeout",
        0x02 => "stateless_reset_token",
        0x03 => "max_udp_payload_size",
        0x04 => "initial_max_data",
        0x05 => "initial_max_stream_data_bidi_local",
        0x06 => "initial_max_stream_data_bidi_remote",
        0x07 => "initial_max_stream_data_uni",
        0x08 => "initial_max_streams_bidi",
        0x09 => "initial_max_streams_uni",
        0x0a => "ack_delay_exponent",
        0x0b => "max_ack_delay",
        0x0c => "disable_active_migration",
        0x0d => "preferred_address",
        0x0e => "active_connection_id_limit",
        0x0f => "initial_source_connection_id",
        0x10 => "retry_source_connection_id",
        0x11 => "version_information",
        0x20 => "max_datagram_frame_size",
        0x32 => "grease_quic_bit",
        _ => "unknown/private",
    }
}

fn summarize_quic_tp(params: &[QuicTransportParam]) -> String {
    if params.is_empty() {
        return "-".into();
    }
    params
        .iter()
        .filter(|p| {
            matches!(
                p.id,
                0x01 | 0x03 | 0x04 | 0x05 | 0x06 | 0x07 | 0x08 | 0x09 | 0x0e | 0x20
            )
        })
        .map(|p| {
            if let Some(v) = p.value_int {
                format!("{}={}", p.name, v)
            } else {
                format!("{}(len={})", p.name, p.value_len)
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_share_two_groups() {
        // list_len=4+32+4+32? simplified: one x25519 entry group=0x001d, kx_len=32, zeros
        let mut data = vec![0x00, 0x24]; // 36 bytes list
        data.extend_from_slice(&0x001du16.to_be_bytes());
        data.extend_from_slice(&32u16.to_be_bytes());
        data.extend(std::iter::repeat(0u8).take(32));
        let (groups, lens) = parse_key_share(&data);
        assert_eq!(groups, vec![0x001d]);
        assert_eq!(lens, vec![(0x001d, 32)]);
    }

    #[test]
    fn quic_tp_idle_and_max_data() {
        // max_idle_timeout=0x01, len=2, value=30000
        // initial_max_data=0x04, len=4, value=0x00100000
        let mut data = Vec::new();
        data.push(0x01); // id
        data.push(0x02); // len
        data.extend_from_slice(&30_000u16.to_be_bytes());
        data.push(0x04);
        data.push(0x04);
        data.extend_from_slice(&0x0010_0000u32.to_be_bytes());
        let params = parse_quic_transport_params(&data);
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "max_idle_timeout");
        assert_eq!(params[0].value_int, Some(30_000));
        assert_eq!(params[1].name, "initial_max_data");
        assert_eq!(params[1].value_int, Some(0x0010_0000));
    }

    #[test]
    fn psk_modes() {
        let d = parse_client_hello_ext_detail(&[(45, vec![0x01, 0x01])]);
        assert_eq!(d.psk_modes, vec![1]);
        assert_eq!(d.psk_modes_names, vec!["psk_dhe_ke".to_string()]);
    }
}
