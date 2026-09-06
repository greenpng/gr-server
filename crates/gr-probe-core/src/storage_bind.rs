//! Signed storage bind for VT / cool / product_version (anti-tamper).
//!
//! FE stores the opaque `bind` blob; open validates HMAC so clients cannot
//! forge cool_until or VT association without the server secret.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const BIND_PREFIX: &str = "g5b1.";

fn hmac_sha256_hex(secret: &[u8], msg: &[u8]) -> String {
    // HMAC-SHA256 without extra crate: ipad/opad construction
    const BLK: usize = 64;
    let mut key = [0u8; BLK];
    if secret.len() > BLK {
        let mut h = Sha256::new();
        h.update(secret);
        let d = h.finalize();
        key[..32].copy_from_slice(&d);
    } else {
        key[..secret.len()].copy_from_slice(secret);
    }
    let mut ipad = [0x36u8; BLK];
    let mut opad = [0x5cu8; BLK];
    for i in 0..BLK {
        ipad[i] ^= key[i];
        opad[i] ^= key[i];
    }
    let mut ih = Sha256::new();
    ih.update(ipad);
    ih.update(msg);
    let inner = ih.finalize();
    let mut oh = Sha256::new();
    oh.update(opad);
    oh.update(inner);
    format!("{:x}", oh.finalize())
}

/// Issue signed bind for FE dual-write storage.
pub fn issue_storage_bind(
    secret: &str,
    vt_id: &str,
    cool_until_ms: i64,
    product_version: &str,
    issued_ms: i64,
    exp_ms: i64,
) -> Value {
    let payload = format!(
        "v1|{vt_id}|{cool_until_ms}|{product_version}|{issued_ms}|{exp_ms}"
    );
    let sig = hmac_sha256_hex(secret.as_bytes(), payload.as_bytes());
    let bind = format!("{BIND_PREFIX}{payload}|{sig}");
    json!({
        "algo": "storage_bind_v1",
        "bind": bind,
        "vt_id": vt_id,
        "cool_until_ms": cool_until_ms,
        "product_version": product_version,
        "issued_ms": issued_ms,
        "exp_ms": exp_ms,
    })
}

/// Validate bind; returns Ok(fields) or Err reason.
pub fn validate_storage_bind(
    secret: &str,
    bind: &str,
    now_ms: i64,
    force_identity: bool,
) -> Result<Value, &'static str> {
    if force_identity {
        return Err("force_identity");
    }
    let raw = bind.trim();
    let body = raw.strip_prefix(BIND_PREFIX).ok_or("bad_prefix")?;
    let (payload, sig) = body.rsplit_once('|').ok_or("bad_shape")?;
    if sig.len() < 32 {
        return Err("bad_sig");
    }
    let expect = hmac_sha256_hex(secret.as_bytes(), payload.as_bytes());
    // constant-time-ish compare
    if expect.len() != sig.len() || expect.as_bytes().iter().zip(sig.as_bytes()).fold(0u8, |a, (x, y)| a | (x ^ y)) != 0 {
        return Err("sig_mismatch");
    }
    let parts: Vec<&str> = payload.split('|').collect();
    if parts.len() != 6 || parts[0] != "v1" {
        return Err("bad_payload");
    }
    let vt_id = parts[1];
    let cool_until_ms: i64 = parts[2].parse().map_err(|_| "bad_cool")?;
    let product_version = parts[3];
    let issued_ms: i64 = parts[4].parse().map_err(|_| "bad_issued")?;
    let exp_ms: i64 = parts[5].parse().map_err(|_| "bad_exp")?;
    if exp_ms > 0 && now_ms > exp_ms {
        return Err("expired");
    }
    Ok(json!({
        "ok": true,
        "vt_id": vt_id,
        "cool_until_ms": cool_until_ms,
        "product_version": product_version,
        "issued_ms": issued_ms,
        "exp_ms": exp_ms,
    }))
}

/// Simple request body signature for first-party relay anti-replay (short window).
pub fn sign_relay_body(secret: &str, session_id: &str, ts_ms: i64, nonce: &str, body_hash: &str) -> String {
    let msg = format!("relay_v1|{session_id}|{ts_ms}|{nonce}|{body_hash}");
    hmac_sha256_hex(secret.as_bytes(), msg.as_bytes())
}

pub fn verify_relay_sig(
    secret: &str,
    session_id: &str,
    ts_ms: i64,
    nonce: &str,
    body_hash: &str,
    sig: &str,
    now_ms: i64,
    max_skew_ms: i64,
) -> bool {
    if (now_ms - ts_ms).abs() > max_skew_ms {
        return false;
    }
    if nonce.is_empty() || sig.len() < 32 {
        return false;
    }
    let expect = sign_relay_body(secret, session_id, ts_ms, nonce, body_hash);
    expect.len() == sig.len()
        && expect
            .as_bytes()
            .iter()
            .zip(sig.as_bytes())
            .fold(0u8, |a, (x, y)| a | (x ^ y))
            == 0
}

pub fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    format!("{:x}", h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_roundtrip() {
        let secret = "lab-secret";
        let b = issue_storage_bind(secret, "vt_abc", 2_000_000, "v5.8.48", 1_000_000, 3_000_000);
        let bind = b.get("bind").and_then(|v| v.as_str()).unwrap();
        let v = validate_storage_bind(secret, bind, 1_500_000, false).unwrap();
        assert_eq!(v["vt_id"], "vt_abc");
        assert!(validate_storage_bind(secret, bind, 1_500_000, true).is_err());
        assert!(validate_storage_bind(secret, bind, 4_000_000, false).is_err());
    }

    #[test]
    fn relay_sig() {
        let secret = "s";
        let sig = sign_relay_body(secret, "sid", 1000, "n1", "deadbeef");
        assert!(verify_relay_sig(secret, "sid", 1000, "n1", "deadbeef", &sig, 1500, 5000));
        assert!(!verify_relay_sig(secret, "sid", 1000, "n1", "deadbeef", &sig, 20_000, 5000));
    }
}
