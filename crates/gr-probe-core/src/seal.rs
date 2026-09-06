//! Sealed probe upload: compress + HMAC sign + encrypt (AES-256-GCM).
//!
//! Wire envelope (JSON):
//! ```json
//! {
//!   "v": 1,
//!   "alg": "aes256gcm+hmacsha256+deflate",
//!   "session_id": "...",
//!   "visitor_terminal_id": "...",
//!   "batch_id": "...",
//!   "nonce_b64": "...",
//!   "ciphertext_b64": "...",
//!   "sig_b64": "...",
//!   "key_mode": "session|master",
//!   "seal_exp_ms": 0
//! }
//! ```
//! Signature covers: `v1|session_id|vtid|batch_id|nonce|ciphertext` (HMAC-SHA256).
//!
//! **Browser path**: open issues a short-lived **session seal grant** (ephemeral key
//! derived from master secret). FE encrypts with grant.key; server re-derives from
//! master+session_id+exp. Master secret never ships in FE.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::Compression;
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;
use std::io::{Read, Write};
use thiserror::Error;

type HmacSha256 = Hmac<Sha256>;

/// Default browser session seal TTL (6h) — covers long probe cycles without re-open.
pub const SESSION_SEAL_TTL_MS: u64 = 6 * 60 * 60 * 1000;

#[derive(Debug, Error)]
pub enum SealError {
    #[error("seal crypto: {0}")]
    Crypto(String),
    #[error("seal io: {0}")]
    Io(String),
    #[error("seal json: {0}")]
    Json(String),
    #[error("invalid signature")]
    BadSignature,
    #[error("invalid envelope: {0}")]
    Envelope(String),
    #[error("seal expired")]
    Expired,
    #[error("decompressed payload exceeds limit")]
    TooLarge,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedEnvelope {
    pub v: u32,
    pub alg: String,
    pub session_id: String,
    pub visitor_terminal_id: String,
    pub batch_id: String,
    pub nonce_b64: String,
    pub ciphertext_b64: String,
    pub sig_b64: String,
    /// `session` = FE ephemeral; omit/`master` = long-term SEAL_SECRET (edge/CLI).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_mode: Option<String>,
    /// Required when key_mode=session so server can re-derive the ephemeral secret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seal_exp_ms: Option<i64>,
    /// Seal suite id (v2 crypto matrix entry), e.g. `s2-aesgcm-hkdf-v1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suite_id: Option<String>,
    /// Client FE product epoch — must match server allowlist (v2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fe_epoch: Option<String>,
    /// WASM crypto module id used to seal (v2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wasm_module_id: Option<String>,
    /// Challenge binding digest (v2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge_bind: Option<String>,
    /// Hash of pack implementation set that produced this batch (v2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pack_set_hash: Option<String>,
}

fn derive_keys(secret: &[u8]) -> ([u8; 32], [u8; 32]) {
    use sha2::{Digest, Sha256};
    // P0-4: key-derivation domain labels are compile-time obfuscated (decoded
    // value is byte-identical, so server/client derivation stays compatible).
    let mut h1 = Sha256::new();
    h1.update(gr_obf::obf!("gr-seal-enc-v1").s().as_bytes());
    h1.update(secret);
    let mut enc = [0u8; 32];
    enc.copy_from_slice(&h1.finalize());

    let mut h2 = Sha256::new();
    h2.update(gr_obf::obf!("gr-seal-mac-v1").s().as_bytes());
    h2.update(secret);
    let mut mac = [0u8; 32];
    mac.copy_from_slice(&h2.finalize());
    (enc, mac)
}

fn compress(plain: &[u8]) -> Result<Vec<u8>, SealError> {
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
    enc.write_all(plain)
        .map_err(|e| SealError::Io(e.to_string()))?;
    enc.finish().map_err(|e| SealError::Io(e.to_string()))
}

/// Hard cap on inflated sealed/cold payloads (GR-INGEST-006).
pub const MAX_DECOMPRESSED_BYTES: u64 = 2 * 1024 * 1024;

pub fn decompress_limited(data: &[u8], max: u64) -> Result<Vec<u8>, SealError> {
    let mut dec = DeflateDecoder::new(data).take(max.saturating_add(1));
    let mut out = Vec::new();
    dec.read_to_end(&mut out)
        .map_err(|e| SealError::Io(e.to_string()))?;
    if out.len() as u64 > max {
        return Err(SealError::TooLarge);
    }
    Ok(out)
}

fn decompress(data: &[u8]) -> Result<Vec<u8>, SealError> {
    decompress_limited(data, MAX_DECOMPRESSED_BYTES)
}

/// true = raw deflate payload; false = plaintext before AES (legacy/no CompressionStream browsers).
fn envelope_uses_deflate(alg: &str) -> bool {
    let a = alg.to_ascii_lowercase();
    a.contains("deflate") && !a.contains("identity") && !a.contains("+rawplain")
}

fn sign(mac_key: &[u8; 32], envelope: &SealedEnvelope) -> Result<String, SealError> {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(mac_key)
        .map_err(|e| SealError::Crypto(e.to_string()))?;
    mac.update(b"v1|");
    mac.update(envelope.session_id.as_bytes());
    mac.update(b"|");
    mac.update(envelope.visitor_terminal_id.as_bytes());
    mac.update(b"|");
    mac.update(envelope.batch_id.as_bytes());
    mac.update(b"|");
    mac.update(envelope.nonce_b64.as_bytes());
    mac.update(b"|");
    mac.update(envelope.ciphertext_b64.as_bytes());
    Ok(B64.encode(mac.finalize().into_bytes()))
}

fn hmac_new(key: &[u8]) -> HmacSha256 {
    // Fail-closed (iss/opus5 S-1): an empty seal master key must never silently
    // degrade to a well-known all-zero key — anyone could then forge valid
    // signatures while the system appears healthy. Startup config validation
    // makes this unreachable; the assert is defense-in-depth.
    assert!(
        !key.is_empty(),
        "{} — set GR_SEAL_SECRET (refusing fail-open zero key)",
        gr_obf::obf!("seal master key is empty").s()
    );
    <HmacSha256 as Mac>::new_from_slice(key).expect("hmac accepts keys of any length")
}

/// Derive per-session seal material from master secret (never sent as master to FE).
pub fn derive_session_seal_secret(master: &[u8], session_id: &str, exp_ms: i64) -> Vec<u8> {
    let mut mac = hmac_new(master);
    mac.update(b"gr-session-seal-v1|");
    mac.update(session_id.as_bytes());
    mac.update(b"|");
    mac.update(exp_ms.to_string().as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// Issue FE-facing grant: short-lived key for browser seal (master stays server-side).
pub fn issue_session_seal_grant(
    master: &[u8],
    session_id: &str,
    now_ms: u64,
    ttl_ms: u64,
) -> Value {
    let ttl = if ttl_ms == 0 { SESSION_SEAL_TTL_MS } else { ttl_ms };
    let exp_ms = (now_ms.saturating_add(ttl)) as i64;
    let key = derive_session_seal_secret(master, session_id, exp_ms);
    let key_b64 = B64.encode(&key);
    // Grant integrity: FE may ignore; ops can audit.
    let mut mac = hmac_new(master);
    mac.update(b"gr-session-seal-grant-v1|");
    mac.update(session_id.as_bytes());
    mac.update(b"|");
    mac.update(exp_ms.to_string().as_bytes());
    mac.update(b"|");
    mac.update(key_b64.as_bytes());
    let grant_sig = B64.encode(mac.finalize().into_bytes());
    json!({
        "algo": "gr_session_seal_v1",
        "key_mode": "session",
        "session_id": session_id,
        "exp_ms": exp_ms,
        "ttl_ms": ttl,
        "key_b64": key_b64,
        "grant_sig_b64": grant_sig,
        "note": "Ephemeral; master SEAL_SECRET never embedded in FE",
    })
}

/// Seal a probe payload JSON object for upload.
pub fn seal_probe_payload(
    secret: &[u8],
    session_id: &str,
    visitor_terminal_id: &str,
    batch_id: &str,
    payload: &Value,
) -> Result<SealedEnvelope, SealError> {
    seal_probe_payload_ex(
        secret,
        session_id,
        visitor_terminal_id,
        batch_id,
        payload,
        None,
        None,
    )
}

/// Seal with optional key_mode / seal_exp_ms (browser session path).
pub fn seal_probe_payload_ex(
    secret: &[u8],
    session_id: &str,
    visitor_terminal_id: &str,
    batch_id: &str,
    payload: &Value,
    key_mode: Option<&str>,
    seal_exp_ms: Option<i64>,
) -> Result<SealedEnvelope, SealError> {
    seal_probe_payload_ex2(
        secret,
        session_id,
        visitor_terminal_id,
        batch_id,
        payload,
        key_mode,
        seal_exp_ms,
        true,
    )
}

/// `use_deflate=false` → alg `aes256gcm+hmacsha256+identity` (no CompressionStream browsers).
pub fn seal_probe_payload_ex2(
    secret: &[u8],
    session_id: &str,
    visitor_terminal_id: &str,
    batch_id: &str,
    payload: &Value,
    key_mode: Option<&str>,
    seal_exp_ms: Option<i64>,
    use_deflate: bool,
) -> Result<SealedEnvelope, SealError> {
    let plain = serde_json::to_vec(payload).map_err(|e| SealError::Json(e.to_string()))?;
    let pre_aes = if use_deflate {
        compress(&plain)?
    } else {
        plain
    };
    let (enc_key, mac_key) = derive_keys(secret);
    let cipher = Aes256Gcm::new_from_slice(&enc_key)
        .map_err(|e| SealError::Crypto(e.to_string()))?;
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, pre_aes.as_ref())
        .map_err(|e| SealError::Crypto(format!("encrypt: {e}")))?;
    let alg = if use_deflate {
        "aes256gcm+hmacsha256+deflate"
    } else {
        "aes256gcm+hmacsha256+identity"
    };
    let mut env = SealedEnvelope {
        v: 1,
        alg: alg.into(),
        session_id: session_id.to_string(),
        visitor_terminal_id: visitor_terminal_id.to_string(),
        batch_id: batch_id.to_string(),
        nonce_b64: B64.encode(nonce_bytes),
        ciphertext_b64: B64.encode(ct),
        sig_b64: String::new(),
        key_mode: key_mode.map(|s| s.to_string()),
        seal_exp_ms,
        suite_id: None,
        fe_epoch: None,
        wasm_module_id: None,
        challenge_bind: None,
        pack_set_hash: None,
    };
    env.sig_b64 = sign(&mac_key, &env)?;
    Ok(env)
}

/// Verify signature + decrypt + decompress → payload JSON (legacy v1 envelope only).
pub fn unseal_probe_payload(secret: &[u8], envelope: &SealedEnvelope) -> Result<Value, SealError> {
    if envelope.v != 1 {
        return Err(SealError::Envelope(format!(
            "unseal_probe_payload is v1-only; got v={} (use unseal_probe_payload_auto)",
            envelope.v
        )));
    }
    let (_enc_key, mac_key) = derive_keys(secret);
    let expect = sign(&mac_key, envelope)?;
    if expect != envelope.sig_b64 {
        return Err(SealError::BadSignature);
    }
    let (enc_key, _) = derive_keys(secret);
    let cipher = Aes256Gcm::new_from_slice(&enc_key)
        .map_err(|e| SealError::Crypto(e.to_string()))?;
    let nonce_raw = B64
        .decode(&envelope.nonce_b64)
        .map_err(|e| SealError::Envelope(e.to_string()))?;
    if nonce_raw.len() != 12 {
        return Err(SealError::Envelope("nonce must be 12 bytes".into()));
    }
    let nonce = Nonce::from_slice(&nonce_raw);
    let ct = B64
        .decode(&envelope.ciphertext_b64)
        .map_err(|e| SealError::Envelope(e.to_string()))?;
    let pre = cipher
        .decrypt(nonce, ct.as_ref())
        .map_err(|_| SealError::Crypto("decrypt failed".into()))?;
    let plain = if envelope_uses_deflate(&envelope.alg) {
        decompress(&pre)?
    } else {
        pre
    };
    serde_json::from_slice(&plain).map_err(|e| SealError::Json(e.to_string()))
}

/// Unseal trying current secret, then optional previous secret (key rotation window).
pub fn unseal_probe_payload_rotated(
    secret: &[u8],
    secret_prev: Option<&[u8]>,
    envelope: &SealedEnvelope,
) -> Result<Value, SealError> {
    match unseal_probe_payload(secret, envelope) {
        Ok(v) => Ok(v),
        Err(SealError::BadSignature) => {
            if let Some(prev) = secret_prev {
                if !prev.is_empty() && prev != secret {
                    return unseal_probe_payload(prev, envelope);
                }
            }
            Err(SealError::BadSignature)
        }
        Err(e) => Err(e),
    }
}

/// Production unseal entry — prefers seal v2 policy when required.
pub fn unseal_probe_payload_auto(
    master: &[u8],
    master_prev: Option<&[u8]>,
    envelope: &SealedEnvelope,
    now_ms: i64,
) -> Result<Value, SealError> {
    crate::seal_v2::unseal_probe_payload_auto_v2(master, master_prev, envelope, now_ms)
}

/// Legacy v1 unseal path (lab / edge master, or when v2 not required).
pub fn unseal_probe_payload_auto_legacy(
    master: &[u8],
    master_prev: Option<&[u8]>,
    envelope: &SealedEnvelope,
    now_ms: i64,
) -> Result<Value, SealError> {
    // Prefer session path when labeled (FE browser grant).
    let mode = envelope
        .key_mode
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    if mode == "session" || envelope.seal_exp_ms.is_some() {
        let exp = envelope.seal_exp_ms.unwrap_or(0);
        if exp > 0 && now_ms > exp {
            return Err(SealError::Expired);
        }
        if exp > 0 && !envelope.session_id.is_empty() {
            let sess = derive_session_seal_secret(master, &envelope.session_id, exp);
            match unseal_probe_payload(&sess, envelope) {
                Ok(v) => return Ok(v),
                Err(SealError::BadSignature) => {
                    if let Some(prev) = master_prev {
                        if !prev.is_empty() {
                            let sess_prev =
                                derive_session_seal_secret(prev, &envelope.session_id, exp);
                            if let Ok(v) = unseal_probe_payload(&sess_prev, envelope) {
                                return Ok(v);
                            }
                        }
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }
    // Master / edge / CLI seals
    unseal_probe_payload_rotated(master, master_prev, envelope)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn roundtrip_seal_unseal() {
        let secret = b"lab-seal-secret-for-tests-only!!";
        let payload = json!({
            "fields": {"user_agent": "Mozilla/5.0", "screen_width": 1920},
            "source": "main"
        });
        let env = seal_probe_payload(secret, "cycle_abc", "vt_xyz", "B0_bootstrap", &payload)
            .expect("seal");
        assert!(!env.sig_b64.is_empty());
        let out = unseal_probe_payload(secret, &env).expect("unseal");
        assert_eq!(out["fields"]["screen_width"], 1920);
        assert_eq!(out["fields"]["user_agent"], "Mozilla/5.0");
    }

    #[test]
    fn bad_sig_rejected() {
        let secret = b"lab-seal-secret-for-tests-only!!";
        let payload = json!({"fields": {}});
        let mut env = seal_probe_payload(secret, "s1", "v1", "B1", &payload).unwrap();
        env.sig_b64 = B64.encode([0u8; 32]);
        assert!(matches!(
            unseal_probe_payload(secret, &env),
            Err(SealError::BadSignature)
        ));
    }

    #[test]
    fn wrong_secret_rejected() {
        let payload = json!({"x": 1});
        let env = seal_probe_payload(b"secret-aaaaaaaaaaaaaaaaaaaa", "s", "v", "B", &payload)
            .unwrap();
        assert!(unseal_probe_payload(b"secret-bbbbbbbbbbbbbbbbbbbb", &env).is_err());
    }

    #[test]
    fn session_grant_roundtrip() {
        // Legacy v1 grant/crypto path (lab). Use auto_legacy so parallel seal_v2 tests
        // that set GR_SEAL_REQUIRE_V2=1 cannot flake this unit via process env.
        let master = b"lab-seal-secret-for-tests-only!!";
        let sid = "cycle_sess_1";
        let now = 1_700_000_000_000u64;
        let grant = issue_session_seal_grant(master, sid, now, 60_000);
        let exp = grant["exp_ms"].as_i64().unwrap();
        let key_b64 = grant["key_b64"].as_str().unwrap();
        let key = B64.decode(key_b64).unwrap();
        let payload = json!({"fields": {"a": 1}, "source": "main"});
        let env = seal_probe_payload_ex(
            &key,
            sid,
            "vt_1",
            "B0_bootstrap",
            &payload,
            Some("session"),
            Some(exp),
        )
        .unwrap();
        let out = unseal_probe_payload_auto_legacy(master, None, &env, now as i64 + 1000).unwrap();
        assert_eq!(out["fields"]["a"], 1);
        // expired
        assert!(matches!(
            unseal_probe_payload_auto_legacy(master, None, &env, exp + 1),
            Err(SealError::Expired)
        ));
    }

    #[test]
    fn identity_no_deflate_roundtrip() {
        let secret = b"lab-seal-secret-for-tests-only!!";
        let payload = json!({"fields": {"x": 9}});
        let env = seal_probe_payload_ex2(
            secret,
            "s",
            "v",
            "B0",
            &payload,
            Some("session"),
            Some(9_999_999_999_999),
            false,
        )
        .unwrap();
        assert!(env.alg.contains("identity"));
        let out = unseal_probe_payload(secret, &env).unwrap();
        assert_eq!(out["fields"]["x"], 9);
    }

    #[test]
    fn decompress_limited_rejects_oversize() {
        let huge = vec![b'A'; (MAX_DECOMPRESSED_BYTES as usize) + 64];
        let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&huge).unwrap();
        let c = enc.finish().unwrap();
        assert!(matches!(
            decompress_limited(&c, MAX_DECOMPRESSED_BYTES),
            Err(SealError::TooLarge)
        ));
        let small = b"ok-payload";
        let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
        enc.write_all(small).unwrap();
        let c = enc.finish().unwrap();
        assert_eq!(decompress_limited(&c, MAX_DECOMPRESSED_BYTES).unwrap(), small);
    }
}
