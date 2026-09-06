//! QUIC Initial packet AEAD decrypt (RFC 9000 / 9001).
//!
//! Client Initial packets use well-known salts so passive listeners can decrypt
//! CRYPTO frames that carry the TLS ClientHello — real JA3/JA4 over HTTP/3.
//!
//! Production browsers often:
//! - coalesce multiple QUIC packets in one UDP datagram
//! - split CRYPTO across several Initial packets (same DCID)
//! - send later Initials that are PADDING/ACK only (pn>0, crypto=0)
//!
//! We decrypt all coalesced Initials, reassemble CRYPTO by DCID, and fall back to
//! scanning plaintext for a TLS ClientHello if frame parse desyncs.

use crate::quic_fingerprint::{read_varint, try_parse_tls_client_hello_full, QuicInitialInfo};
use crate::tls_fingerprint::ClientHelloParts;
use aes::cipher::{BlockEncrypt, KeyInit as AesKeyInit};
use aes::Aes128;
use aes_gcm::aead::{Aead, Payload};
use aes_gcm::Aes128Gcm;
use aes_gcm::aead::generic_array::GenericArray;
use hkdf::Hkdf;
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// RFC 9001 §5.2 — QUIC v1 initial salt
const INITIAL_SALT_V1: [u8; 20] = [
    0x38, 0x76, 0x2c, 0xf7, 0xf5, 0x59, 0x34, 0xb3, 0x4d, 0x17, 0x9a, 0xe6, 0xa4, 0xc8, 0x0c,
    0xad, 0xcc, 0xbb, 0x7f, 0x0a,
];

/// RFC 9369 — QUIC v2 initial salt
const INITIAL_SALT_V2: [u8; 20] = [
    0x0d, 0xed, 0xe3, 0xde, 0xf7, 0x00, 0xa6, 0xdb, 0x81, 0x93, 0x81, 0xbe, 0x6e, 0x26, 0x9d,
    0xcb, 0xf9, 0xbd, 0x2e, 0xd9,
];

const QUIC_V1: u32 = 0x0000_0001;
const QUIC_V2: u32 = 0x6b33_43cf;

fn key_labels(version: u32) -> (&'static [u8], &'static [u8], &'static [u8]) {
    if version == QUIC_V2 {
        (b"quicv2 key", b"quicv2 iv", b"quicv2 hp")
    } else {
        (b"quic key", b"quic iv", b"quic hp")
    }
}

fn initial_salt(version: u32) -> Option<&'static [u8; 20]> {
    match version {
        QUIC_V1 => Some(&INITIAL_SALT_V1),
        QUIC_V2 => Some(&INITIAL_SALT_V2),
        _ => None,
    }
}

/// TLS 1.3 HKDF-Expand-Label (RFC 8446 §7.1) used by QUIC (RFC 9001 §5.1).
fn hkdf_expand_label(secret: &[u8], label: &[u8], context: &[u8], length: usize) -> Option<Vec<u8>> {
    let mut full_label = Vec::with_capacity(6 + label.len());
    full_label.extend_from_slice(b"tls13 ");
    full_label.extend_from_slice(label);

    let mut info = Vec::with_capacity(2 + 1 + full_label.len() + 1 + context.len());
    info.extend_from_slice(&(length as u16).to_be_bytes());
    info.push(full_label.len() as u8);
    info.extend_from_slice(&full_label);
    info.push(context.len() as u8);
    info.extend_from_slice(context);

    let hk = Hkdf::<Sha256>::from_prk(secret).ok()?;
    let mut out = vec![0u8; length];
    hk.expand(&info, &mut out).ok()?;
    Some(out)
}

fn derive_client_initial_secrets(version: u32, dcid: &[u8]) -> Option<([u8; 16], [u8; 12], [u8; 16])> {
    let salt = initial_salt(version)?;
    // initial_secret = HKDF-Extract(salt, dcid); client_in = Expand-Label(..., "client in")
    let hk = Hkdf::<Sha256>::new(Some(salt.as_slice()), dcid);
    let mut client_in = [0u8; 32];
    {
        let mut full_label = Vec::from(&b"tls13 "[..]);
        full_label.extend_from_slice(b"client in");
        let mut info = Vec::new();
        info.extend_from_slice(&32u16.to_be_bytes());
        info.push(full_label.len() as u8);
        info.extend_from_slice(&full_label);
        info.push(0); // empty context
        hk.expand(&info, &mut client_in).ok()?;
    }

    let (kl, il, hl) = key_labels(version);
    let key = hkdf_expand_label(&client_in, kl, &[], 16)?;
    let iv = hkdf_expand_label(&client_in, il, &[], 12)?;
    let hp = hkdf_expand_label(&client_in, hl, &[], 16)?;
    let mut k = [0u8; 16];
    let mut i = [0u8; 12];
    let mut h = [0u8; 16];
    k.copy_from_slice(&key);
    i.copy_from_slice(&iv);
    h.copy_from_slice(&hp);
    Some((k, i, h))
}

/// AES-ECB encrypt a single 16-byte block (header protection sample).
fn aes_ecb_encrypt(key: &[u8; 16], block: &[u8; 16]) -> [u8; 16] {
    use aes::cipher::generic_array::GenericArray;
    let cipher = Aes128::new(GenericArray::from_slice(key));
    let mut b = GenericArray::clone_from_slice(block);
    cipher.encrypt_block(&mut b);
    let mut out = [0u8; 16];
    out.copy_from_slice(&b);
    out
}

/// Result of successful Initial AEAD decrypt.
#[derive(Debug, Clone)]
pub struct DecryptedInitial {
    pub packet_number: u64,
    pub pn_len: u8,
    pub plaintext: Vec<u8>,
    /// Contiguous CRYPTO stream from this packet (offset-0 prefix).
    pub crypto_data: Vec<u8>,
    /// Raw CRYPTO pieces (offset, bytes) for multi-packet reassembly.
    pub crypto_pieces: Vec<(u64, Vec<u8>)>,
    pub dcid_hex: String,
    /// Byte length of this QUIC packet in the UDP datagram (for coalescing).
    pub packet_len: usize,
    pub version: u32,
}

/// Cross-datagram CRYPTO reassembly keyed by DCID (client Initial path).
struct CryptoReasmEntry {
    pieces: Vec<(u64, Vec<u8>)>,
    last_seen: Instant,
    best_stream: Vec<u8>,
}

fn crypto_reasm() -> &'static Mutex<HashMap<String, CryptoReasmEntry>> {
    static M: OnceLock<Mutex<HashMap<String, CryptoReasmEntry>>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(HashMap::new()))
}

const REASM_TTL: Duration = Duration::from_secs(30);
const REASM_MAX_KEYS: usize = 256;

/// Merge CRYPTO pieces for `dcid_hex` and return best contiguous stream from offset 0.
pub fn reasm_push_crypto(dcid_hex: &str, pieces: &[(u64, Vec<u8>)]) -> Vec<u8> {
    if dcid_hex.is_empty() || pieces.is_empty() {
        return assemble_crypto(pieces);
    }
    let Ok(mut g) = crypto_reasm().lock() else {
        return assemble_crypto(pieces);
    };
    // GC
    let now = Instant::now();
    g.retain(|_, e| now.duration_since(e.last_seen) < REASM_TTL);
    while g.len() >= REASM_MAX_KEYS {
        if let Some(k) = g
            .iter()
            .min_by_key(|(_, e)| e.last_seen)
            .map(|(k, _)| k.clone())
        {
            g.remove(&k);
        } else {
            break;
        }
    }
    let ent = g.entry(dcid_hex.to_string()).or_insert_with(|| CryptoReasmEntry {
        pieces: Vec::new(),
        last_seen: now,
        best_stream: Vec::new(),
    });
    ent.last_seen = now;
    for p in pieces {
        ent.pieces.push((p.0, p.1.clone()));
    }
    // Cap piece count
    if ent.pieces.len() > 64 {
        ent.pieces.drain(0..ent.pieces.len() - 64);
    }
    let stream = assemble_crypto(&ent.pieces);
    if stream.len() > ent.best_stream.len() {
        ent.best_stream = stream.clone();
    }
    ent.best_stream.clone()
}

/// Decrypt a client→server QUIC Initial packet starting at `data[0]` (best-effort).
pub fn decrypt_client_initial(data: &[u8]) -> Result<DecryptedInitial, String> {
    if data.len() < 20 {
        return Err("datagram too short".into());
    }
    let first = data[0];
    if first & 0x80 == 0 {
        return Err("not long header".into());
    }
    let version = u32::from_be_bytes([data[1], data[2], data[3], data[4]]);
    let ptype = (first & 0x30) >> 4;
    // v1: Initial=0b00; v2: Initial=0b01 (RFC 9369)
    let is_initial = if version == QUIC_V2 {
        ptype == 1
    } else {
        ptype == 0
    };
    if !is_initial {
        return Err(format!(
            "not Initial packet (ptype={ptype} ver={version:08x})"
        ));
    }

    let mut i = 5;
    if i >= data.len() {
        return Err("truncated after version".into());
    }
    let dcid_len = data[i] as usize;
    i += 1;
    if i + dcid_len > data.len() {
        return Err("truncated DCID".into());
    }
    let dcid = &data[i..i + dcid_len];
    let dcid_hex = hex::encode(dcid);
    i += dcid_len;
    if i >= data.len() {
        return Err("truncated SCID len".into());
    }
    let scid_len = data[i] as usize;
    i += 1;
    if i + scid_len > data.len() {
        return Err("truncated SCID".into());
    }
    i += scid_len;

    let (token_len, ni) = read_varint(data, i).ok_or("bad token varint")?;
    i = ni;
    if i + token_len as usize > data.len() {
        return Err("truncated token".into());
    }
    i += token_len as usize;

    let (length, ni) = read_varint(data, i).ok_or("bad length varint")?;
    i = ni;
    let pn_offset = i;
    let packet_end = (pn_offset + length as usize).min(data.len());
    if packet_end < pn_offset + 4 + 16 {
        return Err("Initial length too small for HP sample".into());
    }

    let sample_off = pn_offset + 4;
    if sample_off + 16 > packet_end {
        return Err("not enough bytes for HP sample".into());
    }
    let mut sample = [0u8; 16];
    sample.copy_from_slice(&data[sample_off..sample_off + 16]);

    let (key, iv, hp_key) = derive_client_initial_secrets(version, dcid)
        .ok_or_else(|| format!("unsupported version {version:08x}"))?;

    let mask_block = aes_ecb_encrypt(&hp_key, &sample);
    let mut unprotected = data[..packet_end].to_vec();
    unprotected[0] ^= mask_block[0] & 0x0f;
    let pn_len = ((unprotected[0] & 0x03) + 1) as usize;
    if pn_offset + pn_len > packet_end {
        return Err("PN extends past packet".into());
    }
    for j in 0..pn_len {
        unprotected[pn_offset + j] ^= mask_block[1 + j];
    }
    let mut pn: u64 = 0;
    for j in 0..pn_len {
        pn = (pn << 8) | unprotected[pn_offset + j] as u64;
    }

    let aad = &unprotected[..pn_offset + pn_len];
    let ct = &unprotected[pn_offset + pn_len..packet_end];
    if ct.len() < 16 {
        return Err("ciphertext shorter than GCM tag".into());
    }

    // Nonce = iv XOR left-padded packet number (62-bit PN in 12-byte IV)
    let mut nonce_bytes = iv;
    for j in 0..8 {
        nonce_bytes[4 + j] ^= ((pn >> (8 * (7 - j))) & 0xff) as u8;
    }

    use aes_gcm::KeyInit;
    let cipher = Aes128Gcm::new(GenericArray::from_slice(&key));
    let nonce = GenericArray::from_slice(&nonce_bytes);
    let plaintext = cipher
        .decrypt(
            nonce,
            Payload {
                msg: ct,
                aad,
            },
        )
        .map_err(|_| "AES-GCM decrypt failed (tag mismatch or corrupt packet)".to_string())?;

    let pieces = extract_crypto_pieces(&plaintext);
    let mut crypto_data = assemble_crypto(&pieces);
    // Fallback: frame desync — scan plaintext for handshake ClientHello
    if crypto_data.is_empty() {
        if let Some(ch) = find_client_hello_in_bytes(&plaintext) {
            crypto_data = ch;
        }
    }
    Ok(DecryptedInitial {
        packet_number: pn,
        pn_len: pn_len as u8,
        plaintext,
        crypto_data,
        crypto_pieces: pieces,
        dcid_hex,
        packet_len: packet_end,
        version,
    })
}

/// Decrypt every coalesced client Initial in a UDP datagram (in order).
pub fn decrypt_all_client_initials(data: &[u8]) -> Vec<DecryptedInitial> {
    let mut out = Vec::new();
    let mut off = 0usize;
    let mut guard = 0;
    while off + 20 <= data.len() && guard < 8 {
        guard += 1;
        if data[off] & 0x80 == 0 {
            break; // short header remainder
        }
        match decrypt_client_initial(&data[off..]) {
            Ok(d) => {
                let step = d.packet_len.max(1);
                out.push(d);
                off += step;
            }
            Err(_) => {
                // Not an Initial at this offset — stop coalescing walk
                break;
            }
        }
    }
    out
}

/// Walk QUIC frames and collect CRYPTO pieces (offset-based).
fn extract_crypto_pieces(frames: &[u8]) -> Vec<(u64, Vec<u8>)> {
    let mut pieces: Vec<(u64, Vec<u8>)> = Vec::new();
    let mut i = 0usize;
    while i < frames.len() {
        // Run of zeros is pure PADDING
        if frames[i] == 0x00 {
            while i < frames.len() && frames[i] == 0x00 {
                i += 1;
            }
            continue;
        }
        let (ftype, ni) = match read_varint(frames, i) {
            Some(v) => v,
            None => break,
        };
        i = ni;
        match ftype {
            0x00 => {
                // single PADDING already handled; keep for safety
            }
            0x01 => {
                // PING
            }
            0x02 | 0x03 => {
                let skip = |i: &mut usize| -> Option<()> {
                    let (_, n) = read_varint(frames, *i)?;
                    *i = n;
                    Some(())
                };
                if skip(&mut i).is_none() {
                    break;
                }
                if skip(&mut i).is_none() {
                    break;
                }
                let (range_count, n) = match read_varint(frames, i) {
                    Some(v) => v,
                    None => break,
                };
                i = n;
                if skip(&mut i).is_none() {
                    break;
                }
                for _ in 0..range_count {
                    if skip(&mut i).is_none() {
                        break;
                    }
                    if skip(&mut i).is_none() {
                        break;
                    }
                }
                if ftype == 0x03 {
                    for _ in 0..3 {
                        if skip(&mut i).is_none() {
                            break;
                        }
                    }
                }
            }
            0x04 => {
                // RESET_STREAM: stream_id, error, final_size
                for _ in 0..3 {
                    match read_varint(frames, i) {
                        Some((_, n)) => i = n,
                        None => return pieces,
                    }
                }
            }
            0x05 => {
                // STOP_SENDING
                for _ in 0..2 {
                    match read_varint(frames, i) {
                        Some((_, n)) => i = n,
                        None => return pieces,
                    }
                }
            }
            0x06 => {
                // CRYPTO
                let (offset, n) = match read_varint(frames, i) {
                    Some(v) => v,
                    None => break,
                };
                i = n;
                let (len, n) = match read_varint(frames, i) {
                    Some(v) => v,
                    None => break,
                };
                i = n;
                let len = len as usize;
                if i + len > frames.len() {
                    pieces.push((offset, frames[i..].to_vec()));
                    break;
                }
                pieces.push((offset, frames[i..i + len].to_vec()));
                i += len;
            }
            0x07 => {
                // NEW_TOKEN
                let (len, n) = match read_varint(frames, i) {
                    Some(v) => v,
                    None => break,
                };
                i = n + len as usize;
            }
            0x08..=0x0f => {
                // STREAM frames — rare on Initial; skip by reading offset/len flags
                let has_off = (ftype & 0x04) != 0;
                let has_len = (ftype & 0x02) != 0;
                // stream_id
                match read_varint(frames, i) {
                    Some((_, n)) => i = n,
                    None => break,
                }
                if has_off {
                    match read_varint(frames, i) {
                        Some((_, n)) => i = n,
                        None => break,
                    }
                }
                if has_len {
                    let (len, n) = match read_varint(frames, i) {
                        Some(v) => v,
                        None => break,
                    };
                    i = n + len as usize;
                } else {
                    // rest of packet is stream data
                    break;
                }
            }
            0x1c | 0x1d => break, // CONNECTION_CLOSE
            _ => {
                // Unknown: if remainder looks like padding, stop cleanly; else stop
                if frames[i..].iter().all(|&b| b == 0) {
                    break;
                }
                // Try CH fallback region from here later; stop frame walk
                break;
            }
        }
    }
    pieces
}

#[allow(dead_code)]
fn extract_crypto_stream(frames: &[u8]) -> Vec<u8> {
    assemble_crypto(&extract_crypto_pieces(frames))
}

/// Scan for TLS handshake ClientHello (type 0x01) when CRYPTO frame walk fails.
fn find_client_hello_in_bytes(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 42 {
        return None;
    }
    let n = data.len();
    for i in 0..n.saturating_sub(42) {
        if data[i] != 0x01 {
            continue;
        }
        if i + 4 > n {
            break;
        }
        let len = ((data[i + 1] as usize) << 16)
            | ((data[i + 2] as usize) << 8)
            | (data[i + 3] as usize);
        if len < 38 || len > 16 * 1024 {
            continue;
        }
        if i + 4 + len > n {
            continue;
        }
        let body = &data[i + 4..i + 4 + len];
        // legacy_version often 0x0303
        if body.len() >= 2 && body[0] == 0x03 && (body[1] == 0x03 || body[1] == 0x01) {
            // session_id length at offset 34
            if body.len() > 35 {
                let sid_len = body[34] as usize;
                if 35 + sid_len + 2 <= body.len() {
                    return Some(data[i..i + 4 + len].to_vec());
                }
            }
        }
    }
    None
}

fn assemble_crypto(pieces: &[(u64, Vec<u8>)]) -> Vec<u8> {
    if pieces.is_empty() {
        return Vec::new();
    }
    let mut max_end = 0u64;
    for (off, d) in pieces {
        max_end = max_end.max(*off + d.len() as u64);
    }
    if max_end > 64 * 1024 {
        max_end = 64 * 1024;
    }
    let mut out = vec![0u8; max_end as usize];
    let mut filled = vec![false; max_end as usize];
    for (off, d) in pieces {
        let start = *off as usize;
        for (j, b) in d.iter().enumerate() {
            let idx = start + j;
            if idx < out.len() {
                out[idx] = *b;
                filled[idx] = true;
            }
        }
    }
    let mut end = 0;
    while end < filled.len() && filled[end] {
        end += 1;
    }
    out.truncate(end);
    out
}

/// Pull (type, payload) pairs from CRYPTO stream ClientHello for structured ext parse.
fn extract_ext_payloads_from_crypto(crypto: &[u8]) -> Vec<(u16, Vec<u8>)> {
    let mut p = crypto;
    if p.first() == Some(&0x16) && p.len() > 5 {
        p = &p[5..];
    }
    // handshake ClientHello
    if p.is_empty() || p[0] != 0x01 || p.len() < 42 {
        return Vec::new();
    }
    let body = &p[4..];
    if body.len() < 38 {
        return Vec::new();
    }
    let mut i = 2 + 32;
    if i >= body.len() {
        return Vec::new();
    }
    let sid_len = body[i] as usize;
    i += 1 + sid_len;
    if i + 2 > body.len() {
        return Vec::new();
    }
    let cs_len = u16::from_be_bytes([body[i], body[i + 1]]) as usize;
    i += 2 + cs_len;
    if i >= body.len() {
        return Vec::new();
    }
    let comp_len = body[i] as usize;
    i += 1 + comp_len;
    if i + 2 > body.len() {
        return Vec::new();
    }
    let ext_len = u16::from_be_bytes([body[i], body[i + 1]]) as usize;
    i += 2;
    let ext_end = (i + ext_len).min(body.len());
    let mut out = Vec::new();
    while i + 4 <= ext_end {
        let et = u16::from_be_bytes([body[i], body[i + 1]]);
        let el = u16::from_be_bytes([body[i + 2], body[i + 3]]) as usize;
        i += 4;
        if i + el > ext_end {
            break;
        }
        out.push((et, body[i..i + el].to_vec()));
        i += el;
    }
    out
}

fn apply_client_hello_to_info(info: &mut QuicInitialInfo, crypto: &[u8], via: &str) -> bool {
    let Some(hello) = try_parse_tls_client_hello_full(crypto) else {
        return false;
    };
    let parts = ClientHelloParts {
        legacy_version: hello.legacy_version,
        ciphers: hello.ciphers.clone(),
        extensions: hello.extensions.clone(),
        supported_groups: hello.supported_groups.clone(),
        ec_point_formats: hello.ec_point_formats.clone(),
        supported_versions: hello.supported_versions.clone(),
        signature_algorithms: hello.signature_algorithms.clone(),
        sni: hello.sni.clone(),
        alpn: hello.alpn.clone(),
    };
    let payloads = extract_ext_payloads_from_crypto(crypto);
    let fp = crate::tls_fingerprint::compute_fingerprints_with_raw(
        &parts,
        &[],
        &payloads,
        false,
    );
    info.tls_ja3 = Some(fp.ja3);
    info.tls_ja3_hash = Some(fp.ja3_hash);
    info.tls_ja4 = Some(fp.ja4);
    info.sni = hello.sni;
    info.alpn = hello.alpn;
    info.key_share_groups = fp.ext_detail.key_share_groups.clone();
    info.key_share_groups_hex = fp.ext_detail.key_share_groups_hex.clone();
    info.transport_params = fp.ext_detail.quic_transport_params.clone();
    info.transport_params_summary = fp.ext_detail.quic_tp_summary.clone();
    info.transport_params_map = info
        .transport_params
        .iter()
        .map(|p| {
            let v = p
                .value_int
                .map(|i| i.to_string())
                .or_else(|| p.value_hex.clone())
                .unwrap_or_else(|| format!("len={}", p.value_len));
            (p.name.clone(), v)
        })
        .collect();
    info.psk_modes_names = fp.ext_detail.psk_modes_names.clone();
    info.has_ech = fp.ext_detail.has_encrypted_client_hello;
    info.crypto_len = Some(crypto.len());
    info.notes.push(format!(
        "TLS ClientHello from {via} → JA3/JA4 (crypto={}B)",
        crypto.len()
    ));
    if let Some(sum) = &info.transport_params_summary {
        if sum != "-" {
            info.notes.push(format!("QUIC TP: {sum}"));
        }
    }
    true
}

/// Decrypt Initial(s) and fill TLS ClientHello fields on `info` when possible.
///
/// Handles: UDP coalescing, multi-packet CRYPTO reassembly by DCID, CH scan fallback.
pub fn enrich_quic_with_aead(data: &[u8], info: &mut QuicInitialInfo) {
    let packets = decrypt_all_client_initials(data);
    if packets.is_empty() {
        // Keep single-path error note for diagnostics
        match decrypt_client_initial(data) {
            Ok(_) => {}
            Err(e) => {
                info.aead_ok = false;
                info.notes.push(format!("Initial AEAD: {e}"));
            }
        }
        return;
    }

    info.aead_ok = true;
    let mut best_crypto: Vec<u8> = Vec::new();
    let mut best_pn: Option<u64> = None;
    let mut total_crypto_piece = 0usize;

    for (idx, dec) in packets.iter().enumerate() {
        info.notes.push(format!(
            "Initial[{idx}] AEAD ok: pn={} crypto={}B pieces={} plaintext={}B dcid={}",
            dec.packet_number,
            dec.crypto_data.len(),
            dec.crypto_pieces.len(),
            dec.plaintext.len(),
            &dec.dcid_hex[..dec.dcid_hex.len().min(16)]
        ));
        if best_pn.is_none() || dec.crypto_data.len() > best_crypto.len() {
            if !dec.crypto_data.is_empty() {
                best_crypto = dec.crypto_data.clone();
            }
            best_pn = Some(dec.packet_number);
            info.packet_number = Some(dec.packet_number);
            info.packet_number_len = Some(dec.pn_len);
        }
        total_crypto_piece += dec.crypto_pieces.iter().map(|(_, d)| d.len()).sum::<usize>();

        // Cross-packet reassembly by DCID
        if !dec.crypto_pieces.is_empty() {
            let reassembled = reasm_push_crypto(&dec.dcid_hex, &dec.crypto_pieces);
            if reassembled.len() > best_crypto.len() {
                best_crypto = reassembled;
                info.notes.push(format!(
                    "CRYPTO reasm dcid={} stream={}B",
                    &dec.dcid_hex[..dec.dcid_hex.len().min(16)],
                    best_crypto.len()
                ));
            }
        } else if !dec.crypto_data.is_empty() {
            // pieces empty but crypto_data from CH fallback scan
            let reassembled =
                reasm_push_crypto(&dec.dcid_hex, &[(0, dec.crypto_data.clone())]);
            if reassembled.len() > best_crypto.len() {
                best_crypto = reassembled;
            }
        }
    }

    info.crypto_len = Some(best_crypto.len());
    if packets.len() > 1 {
        info.notes.push(format!(
            "coalesced_initials={} total_crypto_piece_bytes={}",
            packets.len(),
            total_crypto_piece
        ));
    }

    if best_crypto.is_empty() {
        info.notes
            .push("AEAD ok but no CRYPTO/ClientHello yet (reasm waiting)".into());
        return;
    }

    if apply_client_hello_to_info(info, &best_crypto, "CRYPTO reasm/AEAD") {
        return;
    }
    info.notes.push(format!(
        "CRYPTO present ({}B) but ClientHello parse failed; head_hex={}",
        best_crypto.len(),
        hex::encode(&best_crypto[..best_crypto.len().min(32)])
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 9001 Appendix A.1 — client Initial keys
    #[test]
    fn rfc9001_a1_client_keys() {
        let dcid = hex::decode("8394c8f03e515708").unwrap();
        let (key, iv, hp) = derive_client_initial_secrets(QUIC_V1, &dcid).unwrap();
        assert_eq!(hex::encode(key), "1f369613dd76d5467730efcbe3b1a22d");
        assert_eq!(hex::encode(iv), "fa044b2f42a3fd3b46fb255c");
        assert_eq!(hex::encode(hp), "9f50449e04a0e810283a1e9933adedd2");
    }

    /// RFC 9001 Appendix A.2 — header protection mask
    #[test]
    fn rfc9001_a2_header_protection_mask() {
        let hp = hex::decode("9f50449e04a0e810283a1e9933adedd2").unwrap();
        let mut hp_key = [0u8; 16];
        hp_key.copy_from_slice(&hp);
        let sample = hex::decode("d1b1c98dd7689fb8ec11d242b123dc9b").unwrap();
        let mut s = [0u8; 16];
        s.copy_from_slice(&sample);
        let mask = aes_ecb_encrypt(&hp_key, &s);
        assert_eq!(&mask[..5], &hex::decode("437b9aec36").unwrap()[..]);
    }

    /// RFC 9001 Appendix A.2 — full client Initial decrypt → CRYPTO ClientHello
    #[test]
    fn rfc9001_a2_decrypt_client_initial() {
        let pkt = hex::decode(concat!(
            "c000000001088394c8f03e5157080000",
            "449e7b9aec34d1b1c98dd7689fb8ec11",
            "d242b123dc9bd8bab936b47d92ec356c",
            "0bab7df5976d27cd449f63300099f399",
            "1c260ec4c60d17b31f8429157bb35a12",
            "82a643a8d2262cad67500cadb8e7378c",
            "8eb7539ec4d4905fed1bee1fc8aafba1",
            "7c750e2c7ace01e6005f80fcb7df6212",
            "30c83711b39343fa028cea7f7fb5ff89",
            "eac2308249a02252155e2347b63d58c5",
            "457afd84d05dfffdb20392844ae81215",
            "4682e9cf012f9021a6f0be17ddd0c208",
            "4dce25ff9b06cde535d0f920a2db1bf3",
            "62c23e596d11a4f5a6cf3948838a3aec",
            "4e15daf8500a6ef69ec4e3feb6b1d98e",
            "610ac8b7ec3faf6ad760b7bad1db4ba3",
            "485e8a94dc250ae3fdb41ed15fb6a8e5",
            "eba0fc3dd60bc8e30c5c4287e53805db",
            "059ae0648db2f64264ed5e39be2e20d8",
            "2df566da8dd5998ccabdae053060ae6c",
            "7b4378e846d29f37ed7b4ea9ec5d82e7",
            "961b7f25a9323851f681d582363aa5f8",
            "9937f5a67258bf63ad6f1a0b1d96dbd4",
            "faddfcefc5266ba6611722395c906556",
            "be52afe3f565636ad1b17d508b73d874",
            "3eeb524be22b3dcbc2c7468d54119c74",
            "68449a13d8e3b95811a198f3491de3e7",
            "fe942b330407abf82a4ed7c1b311663a",
            "c69890f4157015853d91e923037c227a",
            "33cdd5ec281ca3f79c44546b9d90ca00",
            "f064c99e3dd97911d39fe9c5d0b23a22",
            "9a234cb36186c4819e8b9c5927726632",
            "291d6a418211cc2962e20fe47feb3edf",
            "330f2c603a9d48c0fcb5699dbfe58964",
            "25c5bac4aee82e57a85aaf4e2513e4f0",
            "5796b07ba2ee47d80506f8d2c25e50fd",
            "14de71e6c418559302f939b0e1abd576",
            "f279c4b2e0feb85c1f28ff18f58891ff",
            "ef132eef2fa09346aee33c28eb130ff2",
            "8f5b766953334113211996d20011a198",
            "e3fc433f9f2541010ae17c1bf202580f",
            "6047472fb36857fe843b19f5984009dd",
            "c324044e847a4f4a0ab34f719595de37",
            "252d6235365e9b84392b061085349d73",
            "203a4a13e96f5432ec0fd4a1ee65accd",
            "d5e3904df54c1da510b0ff20dcc0c77f",
            "cb2c0e0eb605cb0504db87632cf3d8b4",
            "dae6e705769d1de354270123cb11450e",
            "fc60ac47683d7b8d0f811365565fd98c",
            "4c8eb936bcab8d069fc33bd801b03ade",
            "a2e1fbc5aa463d08ca19896d2bf59a07",
            "1b851e6c239052172f296bfb5e724047",
            "90a2181014f3b94a4e97d117b4381303",
            "68cc39dbb2d198065ae3986547926cd2",
            "162f40a29f0c3c8745c0f50fba3852e5",
            "66d44575c29d39a03f0cda721984b6f4",
            "40591f355e12d439ff150aab7613499d",
            "bd49adabc8676eef023b15b65bfc5ca0",
            "6948109f23f350db82123535eb8a7433",
            "bdabcb909271a6ecbcb58b936a88cd4e",
            "8f2e6ff5800175f113253d8fa9ca8885",
            "c2f552e657dc603f252e1a8e308f76f0",
            "be79e2fb8f5d5fbbe2e30ecadd220723",
            "c8c0aea8078cdfcb3868263ff8f09400",
            "54da48781893a7e49ad5aff4af300cd8",
            "04a6b6279ab3ff3afb64491c85194aab",
            "760d58a606654f9f4400e8b38591356f",
            "bf6425aca26dc85244259ff2b19c41b9",
            "f96f3ca9ec1dde434da7d2d392b905dd",
            "f3d1f9af93d1af5950bd493f5aa731b4",
            "056df31bd267b6b90a079831aaf579be",
            "0a39013137aac6d404f518cfd4684064",
            "7e78bfe706ca4cf5e9c5453e9f7cfd2b",
            "8b4c8d169a44e55c88d4a9a7f9474241",
            "e221af44860018ab0856972e194cd934",
        ))
        .unwrap();

        let dec = decrypt_client_initial(&pkt).expect("decrypt RFC sample");
        assert_eq!(dec.packet_number, 2);
        assert_eq!(dec.pn_len, 4);
        assert!(!dec.crypto_data.is_empty());
        // CRYPTO starts with handshake ClientHello (type 0x01)
        assert_eq!(dec.crypto_data[0], 0x01);
        let hello = try_parse_tls_client_hello_full(&dec.crypto_data).expect("parse CH");
        assert_eq!(hello.sni.as_deref(), Some("example.com"));
        assert!(hello.alpn.iter().any(|a| a == "alpn"));

        // Full parse path: AEAD + structured extensions (key_share / TP)
        let info = crate::quic_fingerprint::parse_quic_datagram(&pkt);
        assert!(info.aead_ok);
        assert_eq!(info.sni.as_deref(), Some("example.com"));
        assert!(info.tls_ja4.is_some());
        // Sample CH includes key_share (x25519) and transport params
        assert!(
            !info.key_share_groups.is_empty() || info.transport_params_summary.is_some(),
            "expected key_share and/or transport params on sample CH"
        );
    }

    #[test]
    fn reject_garbage() {
        assert!(decrypt_client_initial(&[0x40, 1, 2, 3]).is_err());
        assert!(decrypt_client_initial(&[]).is_err());
    }

    #[test]
    fn crypto_reasm_merges_pieces_by_dcid() {
        // Simulate two CRYPTO frames: offset 0 len 3 "abc", offset 3 len 3 "def"
        let stream = reasm_push_crypto("deadbeef", &[(0, b"abc".to_vec())]);
        assert_eq!(stream, b"abc");
        let stream2 = reasm_push_crypto("deadbeef", &[(3, b"def".to_vec())]);
        assert_eq!(stream2, b"abcdef");
    }

    #[test]
    fn find_client_hello_fallback_scan() {
        let mut buf = vec![0u8; 20]; // padding noise
        // Minimal fake CH: type=1, len=40, legacy 0x0303, random 32, sid_len=0, cs_len=0, comp_len=1,0, ext_len=0
        // body must be >= 38: ver2 + random32 + sid1 + cs2 + comp1 = 38
        let mut ch = vec![0x01, 0x00, 0x00, 40];
        ch.extend_from_slice(&[0x03, 0x03]); // legacy
        ch.extend_from_slice(&[0u8; 32]); // random
        ch.push(0); // sid
        ch.extend_from_slice(&[0x00, 0x00]); // cipher suites len
        ch.push(1); // compression len
        ch.push(0); // null compression
        ch.extend_from_slice(&[0x00, 0x00]); // extensions len
        // pad body to 40
        while ch.len() < 4 + 40 {
            ch.push(0);
        }
        buf.extend_from_slice(&ch);
        let found = find_client_hello_in_bytes(&buf).expect("find ch");
        assert_eq!(found[0], 0x01);
    }
}
