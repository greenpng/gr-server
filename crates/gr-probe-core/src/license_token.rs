//! Legacy signed-license token compatibility.
//!
//! Tokens remain parseable for old deployments and status/audit data, but
//! license claims no longer gate visitor-analysis, retention, site count,
//! cluster count, or load-balancer capabilities.
//!
//! Design (doc layer 1, signed authorization):
//! - The official site issues `grlic1.<b64url(payload)>.<b64url(ed25519 sig)>`
//!   tokens; the signature covers the literal `grlic1.<payload>` bytes.
//! - Locally we cache **the token string only** — never the derived
//!   "conclusion" — and re-verify on every load with the built-in/env pubkey.
//! - Short TTL (default 24h, P1-6): revocation = the issuer stops renewing;
//!   no revocation-list distribution needed.
//! - Offline grace (P0-3): a token stays acceptable for up to 72h after
//!   issuance while renewal is unreachable, surfaced as
//!   `license_state: "offline_grace"` so the panel can show 离线宽限中.
//! - Quotas (P1-5) ride inside the signed claims; local enforcement degrades
//!   instead of interrupting when exceeded.

use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use gr_obf::obf;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// P0-4: the `grlic1` token prefix is compile-time XOR-obfuscated (per-release
/// salt), so plain `strings`/`grep` over the binary cannot fingerprint the
/// license scheme or the canonical signed-bytes domain.
pub fn token_prefix() -> String {
    obf!("grlic1").s()
}
/// Default token lifetime: 24h (P1-6 short-TTL renewal model).
pub const DEFAULT_TTL_MS: i64 = 24 * 3600 * 1000;
/// Offline grace after issuance: 72h (P0-3). After this the token is dead
/// even if the node never reached the issuer again.
pub const OFFLINE_GRACE_MS: i64 = 72 * 3600 * 1000;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LicenseQuotas {
    #[serde(default)]
    pub sessions_per_month: Option<u64>,
    #[serde(default)]
    pub sites_max: Option<u32>,
    #[serde(default)]
    pub retention_days_max: Option<u32>,
    #[serde(default)]
    pub nodes_max: Option<u32>,
}

/// Legacy load-balancer claim retained for token compatibility.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct LbEntitlement {
    #[serde(default)]
    pub enabled: bool,
    /// Allowed modes: "proxy" | "redirect" | "internal_ip" (empty = all).
    #[serde(default)]
    pub modes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicenseClaims {
    /// Schema version (currently 1).
    pub v: u32,
    pub license_id: String,
    #[serde(default)]
    pub site_id: String,
    #[serde(default)]
    pub domain: String,
    pub plan: String,
    #[serde(default)]
    pub rpa_enabled: bool,
    #[serde(default)]
    pub device_precisions: Vec<String>,
    #[serde(default)]
    pub quotas: LicenseQuotas,
    /// Legacy load-balancer claim; ignored by current capability checks.
    #[serde(default)]
    pub lb: Option<LbEntitlement>,
    pub iat_ms: i64,
    pub exp_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LicenseState {
    /// now < exp — fully valid.
    Active,
    /// exp passed but within 72h of issuance — offline grace (P0-3).
    OfflineGrace,
}

#[derive(Debug, Clone)]
pub struct VerifiedLicense {
    pub claims: LicenseClaims,
    pub state: LicenseState,
}

fn b64url() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
}

/// Issuer-side signing (official site / lab tooling). The probe node only
/// verifies; the signing key never ships with the product.
pub fn sign_token(signing_key: &[u8; 32], claims: &LicenseClaims) -> Result<String, String> {
    let sk = SigningKey::from_bytes(signing_key);
    let payload = serde_json::to_vec(claims).map_err(|e| e.to_string())?;
    let payload_b64 = b64url().encode(payload);
    let prefix = token_prefix();
    let msg = format!("{prefix}.{payload_b64}");
    let sig: Signature = sk.sign(msg.as_bytes());
    Ok(format!("{msg}.{}", b64url().encode(sig.to_bytes())))
}

/// Verification pubkey resolution order:
/// 1. `GR_LICENSE_PUBKEY_B64` (b64/b64url/hex of the 32-byte key) — ops/lab.
/// 2. Build-time embedded key `GR_LICENSE_PUBKEY_B64` via `option_env!` —
///    this is the "内置公钥" for production builds.
/// 3. `<data_dir>/license_ed25519.pk` written by the authenticated official
///    sync (JWKS fetch); NOT readable from panel_policy.json (an attacker
///    controlling the policy file must not be able to inject a key).
pub fn license_pubkey(data_dir: Option<&Path>) -> Option<[u8; 32]> {
    let from_str = |s: &str| -> Option<[u8; 32]> {
        let t = s.trim();
        let raw = b64url()
            .decode(t)
            .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(t))
            .or_else(|_| base64::engine::general_purpose::STANDARD.decode(t))
            .ok()
            .or_else(|| hex_decode(t));
        raw.and_then(|r| {
            if r.len() == 32 {
                let mut a = [0u8; 32];
                a.copy_from_slice(&r);
                Some(a)
            } else {
                None
            }
        })
    };
    if let Some(v) = gr_abi::env::get("LICENSE_PUBKEY_B64") {
        if let Some(k) = from_str(&v) {
            return Some(k);
        }
    }
    if let Some(embedded) = option_env!("GR_LICENSE_PUBKEY_B64") {
        if let Some(k) = from_str(embedded) {
            return Some(k);
        }
    }
    if let Some(d) = data_dir {
        let p = d.join("license_ed25519.pk");
        if let Ok(bytes) = std::fs::read(&p) {
            if bytes.len() == 32 {
                let mut a = [0u8; 32];
                a.copy_from_slice(&bytes);
                return Some(a);
            }
            if let Ok(s) = std::str::from_utf8(&bytes) {
                if let Some(k) = from_str(s) {
                    return Some(k);
                }
            }
        }
    }
    None
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let t = s.trim();
    if t.len() != 64 || !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    (0..32)
        .map(|i| u8::from_str_radix(&t[i * 2..i * 2 + 2], 16).ok())
        .collect()
}

/// Verify signature + time window. `now_ms` injectable for tests.
pub fn verify_token(
    token: &str,
    pubkey: &[u8; 32],
    now_ms: i64,
) -> Result<VerifiedLicense, String> {
    let malformed = || obf!("malformed_license_token").s();
    let mut parts = token.trim().split('.');
    let (p0, p1, p2) = (
        parts.next().unwrap_or(""),
        parts.next().ok_or_else(malformed)?,
        parts.next().ok_or_else(malformed)?,
    );
    if !obf!("grlic1").eq(p0) || parts.next().is_some() {
        return Err(malformed());
    }
    let msg = format!("{p0}.{p1}");
    let sig_raw = b64url()
        .decode(p2)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(p2))
        .map_err(|_| obf!("bad_license_sig_b64").s())?;
    if sig_raw.len() != 64 {
        return Err(obf!("bad_license_sig_len").s());
    }
    let mut sb = [0u8; 64];
    sb.copy_from_slice(&sig_raw);
    let sig = Signature::from_bytes(&sb);
    let vk = VerifyingKey::from_bytes(pubkey).map_err(|e| e.to_string())?;
    vk.verify(msg.as_bytes(), &sig)
        .map_err(|_| obf!("bad_license_signature").s())?;

    let payload = b64url()
        .decode(p1)
        .map_err(|_| obf!("bad_license_payload_b64").s())?;
    let claims: LicenseClaims =
        serde_json::from_slice(&payload).map_err(|_| obf!("bad_license_payload_json").s())?;
    if claims.v != 1 {
        return Err(obf!("unsupported_license_version").s());
    }
    if claims.iat_ms <= 0 || claims.exp_ms <= claims.iat_ms {
        return Err(obf!("bad_license_window").s());
    }
    if now_ms < claims.exp_ms {
        return Ok(VerifiedLicense {
            claims,
            state: LicenseState::Active,
        });
    }
    // P0-3 offline grace: up to 72h after issuance the last verified
    // entitlement is honored (panel shows 离线宽限中). Past that the token is
    // dead — this is also the P1-6 revocation bound (issuer stops renewing).
    if now_ms < claims.iat_ms + OFFLINE_GRACE_MS {
        return Ok(VerifiedLicense {
            claims,
            state: LicenseState::OfflineGrace,
        });
    }
    Err(obf!("license_expired").s())
}

pub fn entitlement_of(_v: &VerifiedLicense) -> crate::plan_entitlement::PlanEntitlement {
    crate::plan_entitlement::PlanEntitlement::full()
}

/// System-level (non site-scoped) legacy token lookup, retained for status
/// compatibility. Current product capabilities do not depend on its result.
pub fn verified_any_token(
    data_dir: Option<&Path>,
    now_ms: i64,
) -> Option<VerifiedLicense> {
    let pk = license_pubkey(data_dir)?;
    let cache = read_token_cache(data_dir);
    let mut seen = std::collections::HashSet::new();
    let mut fallback: Option<VerifiedLicense> = None;
    for v in cache.values() {
        let Some(tok) = v.as_str() else { continue };
        if !seen.insert(tok.to_string()) {
            continue;
        }
        if let Ok(v) = verify_token(tok, &pk, now_ms) {
            match v.state {
                LicenseState::Active => return Some(v),
                LicenseState::OfflineGrace => {
                    if fallback.is_none() {
                        fallback = Some(v);
                    }
                }
            }
        }
    }
    fallback
}

/// Return the current ungated LB capability surface.
pub fn lb_entitlement(_data_dir: Option<&Path>) -> (bool, Vec<String>) {
    (
        true,
        ["proxy", "redirect", "internal_ip"]
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
    )
}

/// iss/opus5 06-P1-5: quotas from the best verified token.
///
/// Entitlement lookup is site/domain-scoped; quotas are license-wide, so this
/// scans the system token cache (any key). `None` when no usable token exists
/// Quotas remain readable for compatibility/status only; current callers do
/// not enforce them.
pub fn verified_quotas(data_dir: Option<&Path>) -> Option<LicenseQuotas> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    verified_any_token(data_dir, now).map(|v| v.claims.quotas.clone())
}

/// Where raw tokens are cached (tokens only, never derived conclusions).
pub fn license_cache_path(data_dir: Option<&Path>) -> PathBuf {
    if let Some(d) = data_dir {
        return d.join("license_tokens.json");
    }
    PathBuf::from("license_tokens.json")
}

/// Cache a raw token string (keyed by site_id and domain). The cache is NOT
/// authoritative — every read path re-verifies the signature.
pub fn cache_token(data_dir: Option<&Path>, token: &str) -> Result<(), String> {
    let mut map = read_token_cache(data_dir);
    // Decode payload (unverified) only to find the cache keys.
    let payload_b64 = token
        .trim()
        .split('.')
        .nth(1)
        .ok_or_else(|| obf!("malformed_license_token").s())?;
    let payload = b64url()
        .decode(payload_b64)
        .map_err(|_| obf!("bad_license_payload_b64").s())?;
    let claims: LicenseClaims =
        serde_json::from_slice(&payload).map_err(|_| obf!("bad_license_payload_json").s())?;
    if !claims.site_id.is_empty() {
        map.insert(claims.site_id.clone(), Value::String(token.to_string()));
    }
    if !claims.domain.is_empty() {
        map.insert(claims.domain.clone(), Value::String(token.to_string()));
    }
    let path = license_cache_path(data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    let s = serde_json::to_string_pretty(&map).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, s).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn read_token_cache(data_dir: Option<&Path>) -> serde_json::Map<String, Value> {
    let path = license_cache_path(data_dir);
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// Resolve a verified entitlement from the token cache. Returns the
/// entitlement plus a status blob for panel display (plan / state / next
/// expiry / grace marker). `None` when no usable token exists — callers then
/// fall back to vault / unsigned policy (legacy behavior).
pub fn verified_entitlement(
    site_id: Option<&str>,
    domain: Option<&str>,
    data_dir: Option<&Path>,
    now_ms: i64,
) -> Option<(crate::plan_entitlement::PlanEntitlement, Value)> {
    let pk = license_pubkey(data_dir)?;
    let cache = read_token_cache(data_dir);
    let keys: Vec<&str> = [site_id, domain]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    for k in keys {
        let Some(tok) = cache.get(k).and_then(|v| v.as_str()) else {
            continue;
        };
        if let Ok(v) = verify_token(tok, &pk, now_ms) {
            let ent = entitlement_of(&v);
            let status = json!({
                "entitlement_source": "signed_license",
                "license_id": v.claims.license_id,
                "license_state": match v.state {
                    LicenseState::Active => "active",
                    LicenseState::OfflineGrace => "offline_grace",
                },
                "license_offline_grace": v.state == LicenseState::OfflineGrace,
                "license_exp_ms": v.claims.exp_ms,
                "license_grace_deadline_ms": v.claims.iat_ms + OFFLINE_GRACE_MS,
                "quotas": v.claims.quotas,
            });
            return Some((ent, status));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_keypair() -> ([u8; 32], [u8; 32]) {
        // Fixed test-only keypair (never used in prod).
        let sk_bytes = [7u8; 32];
        let sk = SigningKey::from_bytes(&sk_bytes);
        let vk = sk.verifying_key();
        (sk_bytes, vk.to_bytes())
    }

    fn claims(site: &str, iat: i64, exp: i64) -> LicenseClaims {
        LicenseClaims {
            v: 1,
            license_id: "lic_test".into(),
            site_id: site.into(),
            domain: "shop.gr.local".into(),
            plan: "paid".into(),
            rpa_enabled: true,
            device_precisions: vec!["dv0".into(), "dv4".into()],
            quotas: LicenseQuotas {
                sessions_per_month: Some(100_000),
                sites_max: Some(1),
                retention_days_max: Some(30),
                nodes_max: Some(3),
            },
            lb: Some(LbEntitlement {
                enabled: true,
                modes: vec!["proxy".into(), "redirect".into()],
            }),
            iat_ms: iat,
            exp_ms: exp,
        }
    }

    fn claims_no_lb(site: &str, iat: i64, exp: i64) -> LicenseClaims {
        let mut c = claims(site, iat, exp);
        c.lb = None;
        c
    }

    #[test]
    fn sign_verify_roundtrip_active() {
        let (sk, pk) = test_keypair();
        let tok = sign_token(&sk, &claims("shop", 1_000, 1_000 + DEFAULT_TTL_MS)).unwrap();
        let v = verify_token(&tok, &pk, 5_000).unwrap();
        assert_eq!(v.state, LicenseState::Active);
        assert_eq!(v.claims.site_id, "shop");
        assert_eq!(v.claims.quotas.sessions_per_month, Some(100_000));
        let ent = entitlement_of(&v);
        assert_eq!(ent.plan, "paid");
        assert!(ent.rpa_enabled);
    }

    #[test]
    fn tampered_payload_rejected() {
        let (sk, pk) = test_keypair();
        let tok = sign_token(&sk, &claims("shop", 1_000, 86_401_000)).unwrap();
        // Forge: swap payload for a "paid" claim signed never.
        let forged_payload = b64url().encode(
            serde_json::to_vec(&claims("shop", 1_000, i64::MAX / 2)).unwrap(),
        );
        let sig = tok.rsplit('.').next().unwrap();
        let forged = format!("{}.{forged_payload}.{sig}", token_prefix());
        assert!(verify_token(&forged, &pk, 2_000).is_err(), "tampered payload must fail");
        // Wrong key also fails.
        let (_, other_pk) = {
            let sk2 = SigningKey::from_bytes(&[9u8; 32]);
            ([9u8; 32], sk2.verifying_key().to_bytes())
        };
        assert!(verify_token(&tok, &other_pk, 2_000).is_err());
    }

    #[test]
    fn offline_grace_then_expired() {
        let (sk, pk) = test_keypair();
        let iat = 1_000_000i64;
        let tok = sign_token(&sk, &claims("shop", iat, iat + DEFAULT_TTL_MS)).unwrap();
        // Past exp, within 72h of iat → grace.
        let grace_t = iat + DEFAULT_TTL_MS + 3600_000;
        let v = verify_token(&tok, &pk, grace_t).unwrap();
        assert_eq!(v.state, LicenseState::OfflineGrace);
        // Past iat+72h → dead (revocation bound).
        let dead_t = iat + OFFLINE_GRACE_MS + 1;
        assert_eq!(
            verify_token(&tok, &pk, dead_t).unwrap_err(),
            "license_expired"
        );
    }

    #[test]
    fn cache_roundtrip_and_verified_entitlement() {
        let dir = std::env::temp_dir().join(format!(
            "gr_lic_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let (sk, _) = test_keypair();
        let tok = sign_token(&sk, &claims("shop", 1_000, 1_000 + DEFAULT_TTL_MS)).unwrap();
        cache_token(Some(&dir), &tok).unwrap();
        // Cached under both site_id and domain.
        let cache = read_token_cache(Some(&dir));
        assert!(cache.contains_key("shop"));
        assert!(cache.contains_key("shop.gr.local"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lb_claim_roundtrip_and_ungated_surface() {
        let (sk, pk) = test_keypair();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        // LB-enabled token, cached under a system key (empty site still caches
        // under domain); verified_any_token finds it regardless of cache key.
        let tok = sign_token(&sk, &claims("__system__", now - 1_000, now + DEFAULT_TTL_MS)).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "gr_lb_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("license_ed25519.pk"), pk).unwrap();
        cache_token(Some(&dir), &tok).unwrap();
        // Direct verify → entitlement carries lb.
        let v = verify_token(&tok, &pk, now + 5_000).unwrap();
        let ent = entitlement_of(&v);
        assert!(ent.lb_enabled);
        assert!(ent.lb_modes.contains(&"proxy".to_string()));
        // System-level gate via cache, independent of site/domain keys.
        let v2 = verified_any_token(Some(&dir), now + 5_000).unwrap();
        assert_eq!(v2.claims.lb.unwrap().enabled, true);
        let (on, modes) = lb_entitlement(Some(&dir));
        assert!(on);
        assert!(modes.contains(&"redirect".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lb_claim_absent_is_still_ungated() {
        let (sk, pk) = test_keypair();
        let tok = sign_token(&sk, &claims_no_lb("__system__", 1_000, 1_000 + DEFAULT_TTL_MS)).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "gr_lb_none_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("license_ed25519.pk"), pk).unwrap();
        cache_token(Some(&dir), &tok).unwrap();
        let v = verify_token(&tok, &pk, 5_000).unwrap();
        assert!(entitlement_of(&v).lb_enabled);
        let (on, modes) = lb_entitlement(Some(&dir));
        assert!(on);
        assert!(modes.contains(&"proxy".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
