//! H16 challenge seed issue / validate (HMAC-SHA256 style keyed digest + TTL).
//!
//! Not a bare FNV hash: server issues seed+exp+sig; FE binds residual materials to seed;
//! server validates presentation. Expired or tampered seeds fail.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Default algo id for challenge materials.
pub const CHALLENGE_ALGO: &str = "gr_challenge_hmac_v1";

/// Default TTL for session challenge seeds (2 minutes).
pub const DEFAULT_CHALLENGE_TTL_MS: u64 = 120_000;

/// Env or compile-time default secret for lab; production should inject via service config.
pub const DEFAULT_CHALLENGE_SECRET: &str = "gr-lab-challenge-secret-v1";

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Keyed digest: SHA256(secret || "|" || session_id || "|" || seed || "|" || exp_ms).
pub fn challenge_sig(secret: &str, session_id: &str, seed: &str, exp_ms: u64) -> String {
    let mut h = Sha256::new();
    h.update(secret.as_bytes());
    h.update(b"|");
    h.update(session_id.as_bytes());
    h.update(b"|");
    h.update(seed.as_bytes());
    h.update(b"|");
    h.update(exp_ms.to_string().as_bytes());
    hex_lower(&h.finalize())
}

fn make_seed(session_id: &str, now_ms: u64, salt: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"seed|");
    h.update(session_id.as_bytes());
    h.update(b"|");
    h.update(now_ms.to_string().as_bytes());
    h.update(b"|");
    h.update(salt.as_bytes());
    format!("cs{}", &hex_lower(&h.finalize())[..24])
}

/// Issue verifiable challenge material for a session.
pub fn issue_challenge_seed(
    session_id: &str,
    now_ms: u64,
    ttl_ms: u64,
    secret: &str,
) -> Value {
    let exp_ms = now_ms.saturating_add(ttl_ms);
    let seed = make_seed(session_id, now_ms, secret);
    let sig = challenge_sig(secret, session_id, &seed, exp_ms);
    json!({
        "algo": CHALLENGE_ALGO,
        "challenge_seed": seed,
        "challenge_seed_exp_ms": exp_ms,
        "challenge_seed_sig": sig,
        "challenge_seed_ttl_ms": ttl_ms,
        "session_id": session_id,
    })
}

/// Validate seed presentation. Returns Ok(()) if valid.
pub fn validate_challenge_seed(
    session_id: &str,
    seed: &str,
    exp_ms: u64,
    sig: &str,
    secret: &str,
    now_ms: u64,
) -> Result<(), String> {
    if seed.is_empty() || sig.is_empty() {
        return Err("challenge_seed_missing".into());
    }
    if now_ms > exp_ms {
        return Err("challenge_seed_expired".into());
    }
    let expect = challenge_sig(secret, session_id, seed, exp_ms);
    // Constant-time-ish compare length first
    if expect.len() != sig.len() {
        return Err("challenge_seed_sig_mismatch".into());
    }
    let mut diff = 0u8;
    for (a, b) in expect.bytes().zip(sig.bytes()) {
        diff |= a ^ b;
    }
    if diff != 0 {
        return Err("challenge_seed_sig_mismatch".into());
    }
    Ok(())
}

/// Evaluate challenge fields from evidence map: returns (ok, reasons).
pub fn evaluate_challenge_fields(
    session_id: &str,
    fields: &serde_json::Map<String, Value>,
    secret: &str,
    now_ms: u64,
) -> (bool, Vec<String>) {
    let mut reasons = Vec::new();
    let seed = fields
        .get("challenge_seed")
        .or_else(|| fields.get("challenge_seed_value"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let exp = fields
        .get("challenge_seed_exp_ms")
        .and_then(|v| v.as_u64().or_else(|| v.as_i64().map(|i| i as u64)));
    let sig = fields
        .get("challenge_seed_sig")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if seed.is_empty() {
        reasons.push("challenge_seed_absent".into());
        return (false, reasons);
    }
    // If FE only has digest without sig (legacy), note partial
    if sig.is_empty() || exp.is_none() {
        reasons.push("challenge_seed_unsigned_legacy".into());
        return (false, reasons);
    }
    match validate_challenge_seed(session_id, seed, exp.unwrap(), sig, secret, now_ms) {
        Ok(()) => {
            reasons.push("challenge_seed_valid".into());
            (true, reasons)
        }
        Err(e) => {
            reasons.push(e);
            (false, reasons)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_and_validate_roundtrip() {
        let sid = "sess_pow_1";
        let now = 1_700_000_000_000u64;
        let mat = issue_challenge_seed(sid, now, 60_000, DEFAULT_CHALLENGE_SECRET);
        let seed = mat["challenge_seed"].as_str().unwrap();
        let exp = mat["challenge_seed_exp_ms"].as_u64().unwrap();
        let sig = mat["challenge_seed_sig"].as_str().unwrap();
        assert!(validate_challenge_seed(sid, seed, exp, sig, DEFAULT_CHALLENGE_SECRET, now).is_ok());
        assert!(validate_challenge_seed(sid, seed, exp, sig, DEFAULT_CHALLENGE_SECRET, now + 30_000).is_ok());
    }

    #[test]
    fn expired_seed_rejected() {
        let sid = "sess_pow_2";
        let now = 1_000u64;
        let mat = issue_challenge_seed(sid, now, 1000, DEFAULT_CHALLENGE_SECRET);
        let seed = mat["challenge_seed"].as_str().unwrap();
        let exp = mat["challenge_seed_exp_ms"].as_u64().unwrap();
        let sig = mat["challenge_seed_sig"].as_str().unwrap();
        let err = validate_challenge_seed(sid, seed, exp, sig, DEFAULT_CHALLENGE_SECRET, now + 5000)
            .unwrap_err();
        assert!(err.contains("expired"), "{err}");
    }

    #[test]
    fn tampered_sig_rejected() {
        let sid = "sess_pow_3";
        let now = 50_000u64;
        let mat = issue_challenge_seed(sid, now, 60_000, DEFAULT_CHALLENGE_SECRET);
        let seed = mat["challenge_seed"].as_str().unwrap();
        let exp = mat["challenge_seed_exp_ms"].as_u64().unwrap();
        let bad = "00".repeat(32);
        let err = validate_challenge_seed(sid, seed, exp, &bad, DEFAULT_CHALLENGE_SECRET, now)
            .unwrap_err();
        assert!(err.contains("mismatch"), "{err}");
    }

    #[test]
    fn evaluate_fields_valid_and_bad() {
        let sid = "s4";
        let now = 9_000u64;
        let mat = issue_challenge_seed(sid, now, 10_000, DEFAULT_CHALLENGE_SECRET);
        let mut fo = serde_json::Map::new();
        fo.insert("challenge_seed".into(), mat["challenge_seed"].clone());
        fo.insert("challenge_seed_exp_ms".into(), mat["challenge_seed_exp_ms"].clone());
        fo.insert("challenge_seed_sig".into(), mat["challenge_seed_sig"].clone());
        let (ok, rs) = evaluate_challenge_fields(sid, &fo, DEFAULT_CHALLENGE_SECRET, now);
        assert!(ok, "{rs:?}");
        fo.insert("challenge_seed_sig".into(), json!("deadbeef"));
        let (ok2, rs2) = evaluate_challenge_fields(sid, &fo, DEFAULT_CHALLENGE_SECRET, now);
        assert!(!ok2);
        assert!(rs2.iter().any(|r| r.contains("mismatch")));
    }
}
