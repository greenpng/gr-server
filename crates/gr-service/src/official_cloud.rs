//! Official-site OAuth (PKCE) + encrypted algo bundle → in-memory vault.
//!
//! Preferred path: **algo-bundle v2** (per-pull X25519 ECDH → HKDF → AES-GCM).
//! Lab fallback: shared wrap-key + `/v1/runtime/strategy-bundle` when
//! `GR_OFFICIAL_EXPOSE_WRAP_KEY=1` on the official site.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use gr_probe_core::strategy_vault::{entry_from_payload, global_vault, SharedVault};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use x25519_dalek::{PublicKey, StaticSecret};

type HmacSha256 = Hmac<Sha256>;

fn b64_std() -> &'static base64::engine::GeneralPurpose {
    &base64::engine::general_purpose::STANDARD
}

#[derive(Clone)]
pub struct OfficialCloudCfg {
    pub base_url: String,
    pub client_id: String,
    pub redirect_uri: String,
}

impl OfficialCloudCfg {
    pub fn from_env() -> Self {
        Self {
            base_url: gr_abi::env::get("OFFICIAL_URL")
                .unwrap_or_else(|| "http://127.0.0.1:4101".into()),
            client_id: gr_abi::env::get("OAUTH_CLIENT_ID")
                .unwrap_or_else(|| "gr-admin-panel".into()),
            redirect_uri: gr_abi::env::get("OAUTH_REDIRECT_URI")
                .unwrap_or_else(|| "http://127.0.0.1:28680/oauth/callback".into()),
        }
    }
}

#[derive(Clone)]
struct PkceEntry {
    verifier: String,
    created_ms: i64,
}

static PKCE: OnceLock<Mutex<HashMap<String, PkceEntry>>> = OnceLock::new();
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct OAuthTokens {
    access_token: String,
    refresh_token: String,
    expires_at_ms: i64,
}
static ACCESS: OnceLock<Mutex<Option<OAuthTokens>>> = OnceLock::new();
static OFFICIAL_ED25519: OnceLock<Mutex<Option<[u8; 32]>>> = OnceLock::new();
static OFFICIAL_LICENSE_ED25519: OnceLock<Mutex<Option<[u8; 32]>>> = OnceLock::new();
static SYNC_STATE: OnceLock<Mutex<SyncState>> = OnceLock::new();
const PKCE_TTL_MS: i64 = 10 * 60 * 1000;

/// iss/opus5 06-P0-3: periodic entitlement refresh state. Before this, only
/// the OAuth callback and a manual button refreshed entitlements — after a
/// process restart a paid customer silently degraded to free until someone
/// logged in again.
#[derive(Clone, serde::Serialize)]
struct SyncState {
    last_attempt_ms: i64,
    last_ok_ms: i64,
    consecutive_failures: u32,
    last_error: String,
    /// Next scheduled check (panel display).
    next_check_ms: i64,
    /// Last plans snapshot hash — audit log fires only on CHANGE.
    plans_hash: String,
}

impl Default for SyncState {
    fn default() -> Self {
        Self {
            last_attempt_ms: 0,
            last_ok_ms: 0,
            consecutive_failures: 0,
            last_error: String::new(),
            next_check_ms: 0,
            plans_hash: String::new(),
        }
    }
}

fn sync_state() -> &'static Mutex<SyncState> {
    SYNC_STATE.get_or_init(|| Mutex::new(SyncState::default()))
}

/// iss/opus5 06-P0-3: public license/sync status for the admin panel — plan
/// provenance, offline-grace marker, and the next scheduled verification.
pub fn license_sync_status() -> Value {
    let st = sync_state().lock().map(|g| g.clone()).unwrap_or_default();
    let now = now_ms();
    let offline_grace = st.last_ok_ms > 0
        && st.consecutive_failures > 0
        && now < st.last_ok_ms + gr_probe_core::license_token::OFFLINE_GRACE_MS;
    json!({
        "ok": true,
        "logged_in": access_token().is_some(),
        "last_attempt_ms": st.last_attempt_ms,
        "last_ok_ms": st.last_ok_ms,
        "consecutive_failures": st.consecutive_failures,
        "last_error": st.last_error,
        "next_check_ms": st.next_check_ms,
        "offline_grace_active": offline_grace,
        "offline_grace_deadline_ms": if st.last_ok_ms > 0 {
            st.last_ok_ms + gr_probe_core::license_token::OFFLINE_GRACE_MS
        } else { 0 },
        "periodic_sync_interval_ms": license_sync_interval_ms(),
    })
}

fn license_sync_interval_ms() -> i64 {
    gr_abi::env::get("LICENSE_SYNC_INTERVAL_MS")
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(3600 * 1000) // 1h per doc 06-P0-3
        .clamp(60_000, 24 * 3600 * 1000)
}

/// iss/opus5 06-P0-3: hourly entitlement refresh with exponential backoff on
/// failure (5min → 6h cap). While the node is unreachable the last verified
/// entitlements keep working up to the 72h offline grace, and the panel shows
/// 离线宽限中 via `license_sync_status()`.
pub fn spawn_license_sync_loop() {
    tokio::spawn(async move {
        // Stagger the first tick so a fleet restart doesn't hammer the issuer.
        tokio::time::sleep(std::time::Duration::from_secs(45)).await;
        loop {
            let base_sleep = license_sync_interval_ms();
            let mut sleep_ms = base_sleep;
            if access_token().is_some() {
                let cfg = OfficialCloudCfg::from_env();
                let attempt = now_ms();
                match sync_all_site_bundles(&cfg).await {
                    Ok(v) => {
                        if let Ok(mut g) = sync_state().lock() {
                            let changed = {
                                let h = format!(
                                    "{:x}",
                                    Sha256::digest(
                                        v.get("loaded").cloned().unwrap_or(json!([])).to_string()
                                            .as_bytes()
                                    )
                                );
                                let ch = !g.plans_hash.is_empty() && g.plans_hash != h;
                                g.plans_hash = h;
                                ch
                            };
                            g.last_attempt_ms = attempt;
                            g.last_ok_ms = now_ms();
                            g.consecutive_failures = 0;
                            g.last_error.clear();
                            g.next_check_ms = now_ms() + base_sleep;
                            if changed {
                                // Entitlement change audit (06-P0-3).
                                tracing::warn!(
                                    target: "gr_license_audit",
                                    "license entitlements changed via periodic sync"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        let fails = {
                            let mut g = match sync_state().lock() {
                                Ok(g) => g,
                                Err(_) => continue,
                            };
                            g.last_attempt_ms = attempt;
                            g.consecutive_failures = g.consecutive_failures.saturating_add(1);
                            g.last_error = e.clone();
                            g.consecutive_failures
                        };
                        // Exponential backoff: 5min * 2^n, capped at 6h.
                        let backoff = (300_000i64)
                            .saturating_mul(1i64 << fails.min(5))
                            .min(6 * 3600 * 1000);
                        sleep_ms = backoff;
                        if let Ok(mut g) = sync_state().lock() {
                            g.next_check_ms = now_ms() + sleep_ms;
                        }
                        tracing::warn!(
                            "license periodic sync failed (attempt {fails}): {e}; next in {}min",
                            sleep_ms / 60_000
                        );
                    }
                }
            } else if let Ok(mut g) = sync_state().lock() {
                g.next_check_ms = now_ms() + base_sleep;
            }
            tokio::time::sleep(std::time::Duration::from_millis(sleep_ms.max(1000) as u64)).await;
        }
    });
}

fn pkce() -> &'static Mutex<HashMap<String, PkceEntry>> {
    PKCE.get_or_init(|| Mutex::new(pkce_load_file()))
}
fn access() -> &'static Mutex<Option<OAuthTokens>> {
    ACCESS.get_or_init(|| Mutex::new(load_oauth_tokens()))
}
fn official_ed25519_cache() -> &'static Mutex<Option<[u8; 32]>> {
    OFFICIAL_ED25519.get_or_init(|| Mutex::new(None))
}
fn official_license_ed25519_cache() -> &'static Mutex<Option<[u8; 32]>> {
    OFFICIAL_LICENSE_ED25519.get_or_init(|| Mutex::new(None))
}

fn pkce_file() -> PathBuf {
    data_dir().join("oauth_pkce.json")
}

fn pkce_load_file() -> HashMap<String, PkceEntry> {
    let Ok(raw) = std::fs::read_to_string(pkce_file()) else {
        return HashMap::new();
    };
    let Ok(v) = serde_json::from_str::<Value>(&raw) else {
        return HashMap::new();
    };
    let mut out = HashMap::new();
    if let Some(obj) = v.as_object() {
        for (k, e) in obj {
            let verifier = e.get("verifier").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let created_ms = e.get("created_ms").and_then(|x| x.as_i64()).unwrap_or(0);
            if !verifier.is_empty() {
                out.insert(k.clone(), PkceEntry { verifier, created_ms });
            }
        }
    }
    out
}

fn pkce_save_file(map: &HashMap<String, PkceEntry>) {
    let mut obj = serde_json::Map::new();
    for (k, e) in map {
        obj.insert(
            k.clone(),
            json!({"verifier": e.verifier, "created_ms": e.created_ms}),
        );
    }
    let _ = std::fs::create_dir_all(data_dir());
    let path = pkce_file();
    let _ = std::fs::write(&path, serde_json::to_vec(&Value::Object(obj)).unwrap_or_default());
    // iss/opus5 S-9: verifier file is credential traffic — restrict to owner.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
}
pub fn vault() -> SharedVault {
    global_vault()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn b64url(data: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

fn random_b64url(n: usize) -> String {
    let mut b = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut b);
    b64url(&b)
}

/// iss/opus5 S-7: returns `(authorize_url, state)` so the caller can bind the
/// state to the browser via an HttpOnly cookie (login CSRF / session fixation).
pub fn oauth_authorize_url(cfg: &OfficialCloudCfg) -> Result<(String, String), String> {
    let verifier = random_b64url(32);
    let challenge = {
        let mut h = Sha256::new();
        h.update(verifier.as_bytes());
        b64url(&h.finalize())
    };
    let state = random_b64url(16);
    {
        let mut g = pkce().lock().map_err(|e| e.to_string())?;
        let now = now_ms();
        g.retain(|_, e| now.saturating_sub(e.created_ms) < PKCE_TTL_MS);
        // Merge disk (other instances) then insert.
        for (k, v) in pkce_load_file() {
            if now.saturating_sub(v.created_ms) < PKCE_TTL_MS {
                g.entry(k).or_insert(v);
            }
        }
        g.insert(
            state.clone(),
            PkceEntry {
                verifier,
                created_ms: now,
            },
        );
        pkce_save_file(&g);
    }
    Ok((
        format!(
            "{}/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&code_challenge={}&code_challenge_method=S256&state={}",
            cfg.base_url.trim_end_matches('/'),
            urlencoding_simple(&cfg.client_id),
            urlencoding_simple(&cfg.redirect_uri),
            urlencoding_simple(&challenge),
            urlencoding_simple(&state),
        ),
        state,
    ))
}

fn urlencoding_simple(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => format!("%{:02X}", c as u8),
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct OAuthSession {
    pub access_token: String,
    pub email: String,
    pub email_verified: bool,
    pub sub: String,
    pub iss: String,
    pub aud: String,
}

pub async fn oauth_exchange_code(
    cfg: &OfficialCloudCfg,
    code: &str,
    state: &str,
) -> Result<OAuthSession, String> {
    let verifier = {
        let mut g = pkce().lock().map_err(|e| e.to_string())?;
        let now = now_ms();
        g.retain(|_, e| now.saturating_sub(e.created_ms) < PKCE_TTL_MS);
        for (k, v) in pkce_load_file() {
            if now.saturating_sub(v.created_ms) < PKCE_TTL_MS {
                g.entry(k).or_insert(v);
            }
        }
        let v = g
            .remove(state)
            .map(|e| e.verifier)
            .ok_or_else(|| "state_mismatch".to_string())?;
        pkce_save_file(&g);
        v
    };
    let body = json!({
        "grant_type": "authorization_code",
        "code": code,
        "redirect_uri": cfg.redirect_uri,
        "client_id": cfg.client_id,
        "code_verifier": verifier,
    });
    let client = reqwest::Client::new();
    let base = cfg.base_url.trim_end_matches('/');
    let res = client
        .post(format!("{base}/oauth/token"))
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = res.status();
    let v: Value = res.json().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("token_http_{status}: {v}"));
    }
    let tok = v
        .get("access_token")
        .and_then(|x| x.as_str())
        .ok_or("missing_access_token")?
        .to_string();
    let refresh = v
        .get("refresh_token")
        .and_then(|x| x.as_str())
        .ok_or("missing_refresh_token")?
        .to_string();
    let expires_at_ms = now_ms()
        + v.get("expires_in")
            .and_then(|x| x.as_i64())
            .unwrap_or(3600)
            .saturating_mul(1000);
    save_oauth_tokens(&OAuthTokens {
        access_token: tok.clone(),
        refresh_token: refresh,
        expires_at_ms,
    })?;
    let me = client
        .get(format!("{base}/v1/me"))
        .bearer_auth(&tok)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let me_v: Value = me.json().await.unwrap_or(json!({}));
    let user = me_v.get("user").cloned().unwrap_or(me_v);
    let email = user
        .get("email")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let email_verified = user
        .get("email_verified")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    let sub = user
        .get("id")
        .or_else(|| user.get("sub"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if sub.is_empty() {
        return Err("oauth_sub_missing".into());
    }
    Ok(OAuthSession {
        access_token: tok,
        email,
        email_verified,
        sub,
        iss: base.to_string(),
        aud: cfg.client_id.clone(),
    })
}

/// Durable issuer+subject+audience → local panel user. Unbound identities are rejected.
pub fn resolve_bound_local_user(
    db: &gr_admin::db::AdminDb,
    sess: &OAuthSession,
    cfg: &OfficialCloudCfg,
    prod: bool,
) -> Result<String, String> {
    let iss = sess.iss.trim().trim_end_matches('/');
    let cfg_iss = cfg.base_url.trim().trim_end_matches('/');
    if !iss.eq_ignore_ascii_case(cfg_iss) {
        return Err("oauth_issuer_mismatch".into());
    }
    if sess.aud.trim() != cfg.client_id.trim() {
        return Err("oauth_audience_mismatch".into());
    }
    if sess.sub.trim().is_empty() {
        return Err("oauth_sub_missing".into());
    }
    let key = format!("{}|{}|{}", iss.to_ascii_lowercase(), sess.sub, sess.aud);
    let mut bindings: Value = db
        .get_setting("oauth_identity_bindings")
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));
    if let Some(uname) = bindings
        .get(&key)
        .and_then(|v| v.get("local_user").or_else(|| v.get("username")))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
    {
        let users = db.list_users()?;
        if !users.iter().any(|u| u == &uname) {
            return Err("oauth_bound_user_missing".into());
        }
        return Ok(uname);
    }
    let allow: Vec<String> = gr_abi::env::get("OAUTH_ADMIN_EMAILS")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    let email_lc = sess.email.trim().to_ascii_lowercase();
    let allowlisted = !email_lc.is_empty() && allow.iter().any(|e| e == &email_lc);
    if prod && !allowlisted {
        return Err("oauth_identity_unbound".into());
    }
    let uname = if prod {
        gr_abi::env::get("OAUTH_LOCAL_USER")
            .ok_or_else(|| "oauth_local_user_required".to_string())?
    } else {
        gr_abi::env::get("OAUTH_LOCAL_USER").unwrap_or_else(|| "admin".into())
    };
    let uname = uname.trim().to_string();
    if uname.is_empty() {
        return Err("oauth_local_user_required".into());
    }
    let users = db.list_users()?;
    if !users.iter().any(|u| u == &uname) {
        return Err("oauth_local_user_missing".into());
    }
    // Production: one local panel user may be bootstrap-bound once. A second
    // allowlisted identity cannot share that account (GR-OAUTH-004).
    if prod {
        let occupied = bindings.as_object().map(|o| {
            o.values().any(|v| {
                v.get("local_user")
                    .or_else(|| v.get("username"))
                    .and_then(|x| x.as_str())
                    == Some(uname.as_str())
            })
        }).unwrap_or(false);
        if occupied {
            return Err("oauth_identity_unbound".into());
        }
    }
    if let Some(obj) = bindings.as_object_mut() {
        obj.insert(
            key,
            json!({
                "local_user": uname,
                "email": sess.email,
                "sub": sess.sub,
                "iss": sess.iss,
                "aud": sess.aud,
                "bound_ms": now_ms(),
            }),
        );
    }
    db.set_setting("oauth_identity_bindings", &bindings.to_string())?;
    Ok(uname)
}

pub fn access_token() -> Option<String> {
    access().lock().ok().and_then(|g| g.as_ref().map(|t| t.access_token.clone()))
}

fn data_dir() -> PathBuf {
    if let Some(d) = gr_abi::env::get("DATA_DIR") {
        return PathBuf::from(d);
    }
    PathBuf::from("data")
}

fn oauth_tokens_file() -> PathBuf {
    data_dir().join("oauth_tokens.json")
}

fn oauth_token_key() -> Result<[u8; 32], String> {
    let raw = gr_abi::env::get("OAUTH_TOKEN_ENCRYPTION_KEY")
        .and_then(|s| {
            hex::decode(s.trim()).ok().or_else(|| {
                base64::engine::general_purpose::STANDARD.decode(s.trim()).ok()
            })
        });
    if let Some(raw) = raw {
        if raw.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(&raw);
            return Ok(key);
        }
    }
    let key_path = data_dir().join("oauth_tokens.key");
    if let Ok(raw) = std::fs::read(&key_path) {
        if raw.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(&raw);
            return Ok(key);
        }
    }
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    std::fs::create_dir_all(data_dir()).map_err(|e| e.to_string())?;
    std::fs::write(&key_path, key).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    Ok(key)
}

fn load_oauth_tokens() -> Option<OAuthTokens> {
    let envelope: Value =
        serde_json::from_slice(&std::fs::read(oauth_tokens_file()).ok()?).ok()?;
    let nonce = b64_std()
        .decode(envelope.get("nonce")?.as_str()?)
        .ok()?;
    let ciphertext = b64_std()
        .decode(envelope.get("ciphertext")?.as_str()?)
        .ok()?;
    if nonce.len() != 12 {
        return None;
    }
    let key = oauth_token_key().ok()?;
    let plaintext = Aes256Gcm::new_from_slice(&key)
        .ok()?
        .decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
        .ok()?;
    let tokens = serde_json::from_slice::<OAuthTokens>(&plaintext).ok()?;
    if tokens.access_token.is_empty() || tokens.refresh_token.is_empty() {
        return None;
    }
    Some(tokens)
}

fn save_oauth_tokens(tokens: &OAuthTokens) -> Result<(), String> {
    std::fs::create_dir_all(data_dir()).map_err(|e| e.to_string())?;
    let path = oauth_tokens_file();
    let tmp = path.with_extension("json.tmp");
    let key = oauth_token_key()?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);
    let plaintext = serde_json::to_vec(tokens).map_err(|e| e.to_string())?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext.as_ref())
        .map_err(|_| "oauth_token_encrypt_failed".to_string())?;
    let envelope = json!({
        "v": 1,
        "nonce": b64_std().encode(nonce),
        "ciphertext": b64_std().encode(ciphertext),
    });
    std::fs::write(&tmp, serde_json::to_vec(&envelope).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

async fn ensure_access_token(cfg: &OfficialCloudCfg) -> Result<String, String> {
    let current = access().lock().map_err(|e| e.to_string())?.clone();
    if let Some(tokens) = current {
        if tokens.expires_at_ms > now_ms() + 60_000 {
            return Ok(tokens.access_token);
        }
        let client = reqwest::Client::new();
        let res = client
            .post(format!("{}/oauth/token", cfg.base_url.trim_end_matches('/')))
            .json(&json!({
                "grant_type": "refresh_token",
                "refresh_token": tokens.refresh_token,
            }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = res.status();
        let v: Value = res.json().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            if let Ok(mut g) = access().lock() {
                *g = None;
            }
            return Err(format!("refresh_http_{status}: {v}"));
        }
        let next = OAuthTokens {
            access_token: v
                .get("access_token")
                .and_then(|x| x.as_str())
                .ok_or("refresh_missing_access_token")?
                .to_string(),
            refresh_token: v
                .get("refresh_token")
                .and_then(|x| x.as_str())
                .ok_or("refresh_missing_refresh_token")?
                .to_string(),
            expires_at_ms: now_ms()
                + v.get("expires_in")
                    .and_then(|x| x.as_i64())
                    .unwrap_or(3600)
                    .saturating_mul(1000),
        };
        save_oauth_tokens(&next)?;
        *access().lock().map_err(|e| e.to_string())? = Some(next.clone());
        return Ok(next.access_token);
    }
    Err("not_logged_in_official".into())
}

/// Stable panel instance id (persisted under data dir).
pub fn instance_id() -> String {
    let path = data_dir().join("official_instance_id");
    if let Ok(s) = std::fs::read_to_string(&path) {
        let t = s.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    let id = format!("gr-{}", random_b64url(12));
    let _ = std::fs::create_dir_all(data_dir());
    let _ = std::fs::write(&path, &id);
    id
}

/// X25519 SPKI DER prefix (OID 1.3.101.110) + 32-byte raw public key.
const X25519_SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x6e, 0x03, 0x21, 0x00,
];

fn x25519_pub_spki_b64(pk: &PublicKey) -> String {
    let mut der = Vec::with_capacity(44);
    der.extend_from_slice(&X25519_SPKI_PREFIX);
    der.extend_from_slice(pk.as_bytes());
    b64_std().encode(der)
}

fn x25519_pub_from_spki_b64(b64: &str) -> Result<PublicKey, String> {
    let der = b64_std().decode(b64).map_err(|e| e.to_string())?;
    let raw = if der.len() == 32 {
        der
    } else if der.len() >= 44 && der[..12] == X25519_SPKI_PREFIX {
        der[12..44].to_vec()
    } else if der.len() > 32 {
        // Tolerant: take last 32 bytes of SPKI-like blob.
        der[der.len() - 32..].to_vec()
    } else {
        return Err("bad_x25519_spki".into());
    };
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&raw);
    Ok(PublicKey::from(arr))
}

async fn fetch_official_ed25519_raw(
    client: &reqwest::Client,
    base: &str,
) -> Result<[u8; 32], String> {
    if let Ok(g) = official_ed25519_cache().lock() {
        if let Some(pk) = *g {
            return Ok(pk);
        }
    }
    let jwks: Value = client
        .get(format!("{}/v1/jwks", base.trim_end_matches('/')))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let x = jwks
        .pointer("/keys/0/x")
        .and_then(|v| v.as_str())
        .ok_or("jwks_missing_x")?;
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(x)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(x))
        .map_err(|e| e.to_string())?;
    if raw.len() != 32 {
        return Err(format!("jwks_x_len_{}", raw.len()));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&raw);
    if let Ok(mut g) = official_ed25519_cache().lock() {
        *g = Some(arr);
    }
    Ok(arr)
}

async fn fetch_official_license_ed25519_raw(
    client: &reqwest::Client,
    base: &str,
) -> Result<[u8; 32], String> {
    if let Ok(g) = official_license_ed25519_cache().lock() {
        if let Some(pk) = *g {
            return Ok(pk);
        }
    }
    let jwks: Value = client
        .get(format!("{}/v1/license-jwks", base.trim_end_matches('/')))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let x = jwks
        .pointer("/keys/0/x")
        .and_then(|v| v.as_str())
        .ok_or("license_jwks_missing_x")?;
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(x)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(x))
        .map_err(|e| e.to_string())?;
    if raw.len() != 32 {
        return Err(format!("license_jwks_x_len_{}", raw.len()));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&raw);
    if let Ok(mut g) = official_license_ed25519_cache().lock() {
        *g = Some(arr);
    }
    Ok(arr)
}

fn verify_ed25519(pubkey: &[u8; 32], msg: &[u8], sig_b64: &str) -> Result<(), String> {
    let vk = VerifyingKey::from_bytes(pubkey).map_err(|e| e.to_string())?;
    let sig_raw = b64_std().decode(sig_b64).map_err(|e| e.to_string())?;
    if sig_raw.len() != 64 {
        return Err("bad_ed25519_sig_len".into());
    }
    let mut sb = [0u8; 64];
    sb.copy_from_slice(&sig_raw);
    let sig = Signature::from_bytes(&sb);
    vk.verify(msg, &sig).map_err(|_| "bad_ed25519_sig".to_string())
}

fn ecdh_aes_key(our_secret: &StaticSecret, peer_pub: &PublicKey) -> Result<[u8; 32], String> {
    let shared = our_secret.diffie_hellman(peer_pub);
    // Match Node crypto.hkdfSync(..., salt=Buffer.alloc(0), info, 32)
    let hk = Hkdf::<Sha256>::new(Some(&[]), shared.as_bytes());
    let mut out = [0u8; 32];
    hk.expand(b"gr-algo-bundle-v2", &mut out)
        .map_err(|_| "hkdf_expand_failed".to_string())?;
    Ok(out)
}

fn generate_ephemeral() -> (StaticSecret, PublicKey) {
    let mut rng = rand::thread_rng();
    let secret = StaticSecret::random_from_rng(&mut rng);
    let public = PublicKey::from(&secret);
    (secret, public)
}

async fn enroll_node(cfg: &OfficialCloudCfg, tok: &str) -> Result<(), String> {
    // Long-lived node identity pubkey (separate from per-pull ephemeral).
    let path = data_dir().join("official_node_x25519.sk");
    let secret = load_or_create_node_secret(&path)?;
    let public = PublicKey::from(&secret);
    let client = reqwest::Client::new();
    let base = cfg.base_url.trim_end_matches('/');
    let res = client
        .post(format!("{base}/v1/nodes/enroll"))
        .bearer_auth(tok)
        .json(&json!({
            "instance_id": instance_id(),
            "x25519_pub_b64": x25519_pub_spki_b64(&public),
            "label": "admin-panel",
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        let v: Value = res.json().await.unwrap_or(json!({}));
        return Err(format!("enroll_failed: {v}"));
    }
    Ok(())
}

fn load_or_create_node_secret(path: &Path) -> Result<StaticSecret, String> {
    if let Ok(bytes) = std::fs::read(path) {
        if bytes.len() == 32 {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            return Ok(StaticSecret::from(arr));
        }
    }
    let mut rng = rand::thread_rng();
    let secret = StaticSecret::random_from_rng(&mut rng);
    let _ = std::fs::create_dir_all(path.parent().unwrap_or_else(|| Path::new(".")));
    let _ = std::fs::write(path, secret.to_bytes());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(secret)
}

pub async fn sync_all_site_bundles(cfg: &OfficialCloudCfg) -> Result<Value, String> {
    let tok = ensure_access_token(cfg).await?;
    let client = reqwest::Client::new();
    let base = cfg.base_url.trim_end_matches('/');
    let me = client
        .get(format!("{base}/v1/sites"))
        .bearer_auth(&tok)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let sites_v: Value = me.json().await.map_err(|e| e.to_string())?;
    let sites = sites_v
        .get("sites")
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default();

    // Best-effort enroll (required for audit / last_seen; ECDH uses per-pull ephemeral).
    let _ = enroll_node(cfg, &tok).await;

    let mut loaded = Vec::new();
    let mut entitlements = serde_json::Map::new();
    let mut used_protocol = "algo-bundle-v2";
    let mut wrap_key_fallback: Option<Vec<u8>> = None;

    for s in sites {
        let site_id = s.get("site_id").and_then(|v| v.as_str()).unwrap_or("");
        let domain = s.get("domain").and_then(|v| v.as_str()).unwrap_or("");
        if site_id.is_empty() || domain.is_empty() {
            continue;
        }

        match fetch_and_ingest_ecdh(&client, base, &tok, site_id, domain).await {
            Ok(entry) => {
                record_entitlement(&mut entitlements, &entry);
                vault().upsert(entry.clone());
                loaded.push(json!({
                    "site_id": entry.site_id,
                    "domain": entry.domain,
                    "plan": entry.plan,
                    "rpa_enabled": entry.rpa_enabled,
                    "device_precisions": entry.device_precisions,
                    "expires_at_ms": entry.expires_at_ms,
                    "ok": true,
                    "protocol": "algo-bundle-v2",
                }));
                continue;
            }
            Err(e) => {
                // Lab fallback: shared wrap-key path (official must expose wrap-key).
                if wrap_key_fallback.is_none() {
                    wrap_key_fallback = try_fetch_wrap_key(&client, base, &tok).await;
                }
                if let Some(ref wrap_key) = wrap_key_fallback {
                    used_protocol = "strategy-bundle-lab";
                    match fetch_and_ingest_lab(
                        &client, base, &tok, wrap_key, site_id, domain,
                    )
                    .await
                    {
                        Ok(entry) => {
                            record_entitlement(&mut entitlements, &entry);
                            vault().upsert(entry.clone());
                            loaded.push(json!({
                                "site_id": entry.site_id,
                                "domain": entry.domain,
                                "plan": entry.plan,
                                "rpa_enabled": entry.rpa_enabled,
                                "device_precisions": entry.device_precisions,
                                "expires_at_ms": entry.expires_at_ms,
                                "ok": true,
                                "protocol": "strategy-bundle-lab",
                                "ecdh_error": e,
                            }));
                            continue;
                        }
                        Err(e2) => {
                            loaded.push(json!({
                                "site_id": site_id,
                                "ok": false,
                                "ecdh_error": e,
                                "lab_error": e2,
                            }));
                            continue;
                        }
                    }
                }
                loaded.push(json!({"site_id": site_id, "ok": false, "error": e}));
            }
        }
    }

    persist_entitlements(&entitlements);
    // iss/opus5 06-P0-1: persist the issuer pubkey (fetched over the
    // authenticated channel) so license tokens verify after restart even on
    // builds without an embedded key. Raw tokens were cached in ingest_bundle.
    if let Ok(client2) = reqwest::Client::builder().build() {
        if let Ok(pk) = fetch_official_license_ed25519_raw(&client2, base).await {
            let _ = std::fs::create_dir_all(data_dir());
            let _ = std::fs::write(data_dir().join("license_ed25519.pk"), pk);
        }
        let _ = client2; // dropped
    }
    Ok(json!({
        "ok": true,
        "loaded": loaded,
        "protocol": used_protocol,
        "vault_note": "plan entitlements held in process memory; no user strategy config",
        "license_note": "signed license tokens cached (raw tokens only; verified per load)",
        "at_ms": now_ms(),
    }))
}

fn record_entitlement(
    entitlements: &mut serde_json::Map<String, Value>,
    entry: &gr_probe_core::strategy_vault::VaultEntry,
) {
    entitlements.insert(
        entry.site_id.clone(),
        json!({
            "plan": entry.plan,
            "rpa_enabled": entry.rpa_enabled,
            "device_precisions": entry.device_precisions,
            "domain": entry.domain,
            "expires_at_ms": entry.expires_at_ms,
        }),
    );
    entitlements.insert(
        entry.domain.clone(),
        json!({
            "plan": entry.plan,
            "rpa_enabled": entry.rpa_enabled,
            "device_precisions": entry.device_precisions,
            "site_id": entry.site_id,
            "expires_at_ms": entry.expires_at_ms,
        }),
    );
}

fn persist_entitlements(entitlements: &serde_json::Map<String, Value>) {
    let path = data_dir();
    let pol = if path.exists() {
        gr_probe_core::load_panel_policy(Some(&path))
    } else {
        gr_probe_core::load_panel_policy(None)
    };
    let merged = gr_probe_core::merge_entitlements(pol, &Value::Object(entitlements.clone()));
    if path.exists() {
        let _ = gr_probe_core::save_panel_policy(Some(&path), merged);
    } else {
        let _ = gr_probe_core::save_panel_policy(None, merged);
    }
}

async fn try_fetch_wrap_key(
    client: &reqwest::Client,
    base: &str,
    tok: &str,
) -> Option<Vec<u8>> {
    let wrap = client
        .get(format!("{base}/v1/crypto/wrap-key"))
        .bearer_auth(tok)
        .send()
        .await
        .ok()?;
    if !wrap.status().is_success() {
        return None;
    }
    let wrap_v: Value = wrap.json().await.ok()?;
    let wrap_b64 = wrap_v.get("wrap_key_b64")?.as_str()?;
    b64_std().decode(wrap_b64).ok()
}

async fn fetch_and_ingest_ecdh(
    client: &reqwest::Client,
    base: &str,
    tok: &str,
    site_id: &str,
    domain: &str,
) -> Result<gr_probe_core::strategy_vault::VaultEntry, String> {
    let (secret, public) = generate_ephemeral();
    let bundle_res = client
        .post(format!("{base}/v1/runtime/algo-bundle"))
        .bearer_auth(tok)
        .json(&json!({
            "site_id": site_id,
            "domain": domain,
            "instance_id": instance_id(),
            "node_ephemeral_pub_b64": x25519_pub_spki_b64(&public),
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = bundle_res.status();
    let bundle: Value = bundle_res.json().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("algo_bundle_http_{status}: {bundle}"));
    }
    let server_pub_b64 = bundle
        .pointer("/header/server_ephemeral_pub_b64")
        .and_then(|v| v.as_str())
        .ok_or("missing_server_ephemeral_pub")?;
    let server_pub = x25519_pub_from_spki_b64(server_pub_b64)?;
    let aes_key = ecdh_aes_key(&secret, &server_pub)?;
    let official_pk = fetch_official_ed25519_raw(client, base).await?;
    ingest_bundle(&aes_key, &bundle, Some(&official_pk))
}

async fn fetch_and_ingest_lab(
    client: &reqwest::Client,
    base: &str,
    tok: &str,
    wrap_key: &[u8],
    site_id: &str,
    domain: &str,
) -> Result<gr_probe_core::strategy_vault::VaultEntry, String> {
    let bundle_res = client
        .post(format!("{base}/v1/runtime/strategy-bundle"))
        .bearer_auth(tok)
        .json(&json!({ "site_id": site_id, "domain": domain }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = bundle_res.status();
    let bundle: Value = bundle_res.json().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("lab_bundle_http_{status}: {bundle}"));
    }
    // Lab path: HMAC over wrap-key; Ed25519 optional (legacy used HMAC as sig_b64).
    ingest_bundle(wrap_key, &bundle, None)
}

fn ingest_bundle(
    aes_key: &[u8],
    bundle: &Value,
    official_ed25519: Option<&[u8; 32]>,
) -> Result<gr_probe_core::strategy_vault::VaultEntry, String> {
    let mut header = bundle.get("header").cloned().ok_or("no_header")?;
    let header_bytes = if let Some(hb) = bundle.get("header_b64").and_then(|v| v.as_str()) {
        b64_std().decode(hb).map_err(|e| e.to_string())?
    } else {
        serde_json::to_vec(&header).map_err(|e| e.to_string())?
    };
    if bundle.get("header_b64").is_some() {
        if let Ok(h) = serde_json::from_slice::<Value>(&header_bytes) {
            header = h;
        }
    }
    let iv = b64_std()
        .decode(bundle.get("iv_b64").and_then(|v| v.as_str()).unwrap_or(""))
        .map_err(|e| e.to_string())?;
    let ct = b64_std()
        .decode(
            bundle
                .get("ciphertext_b64")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
        )
        .map_err(|e| e.to_string())?;

    let protocol = bundle
        .get("protocol")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let is_v2 = protocol == "algo-bundle-v2"
        || header.get("v").and_then(|v| v.as_i64()) == Some(2)
        || header.get("kex").and_then(|v| v.as_str()) == Some("X25519-HKDF-SHA256");

    // HMAC binding (always present for both lab + v2).
    if let Some(mac_b64) = bundle.get("mac_b64").and_then(|v| v.as_str()) {
        let mac = b64_std().decode(mac_b64).map_err(|e| e.to_string())?;
        if mac.len() == 32 {
            let mut mac_c =
                <HmacSha256 as Mac>::new_from_slice(aes_key).map_err(|e| e.to_string())?;
            mac_c.update(&header_bytes);
            mac_c.update(&iv);
            mac_c.update(&ct);
            mac_c
                .verify_slice(&mac)
                .map_err(|_| "bad_mac".to_string())?;
        }
    } else if !is_v2 {
        // Legacy lab: sig_b64 was HMAC.
        let mac = b64_std()
            .decode(bundle.get("sig_b64").and_then(|v| v.as_str()).unwrap_or(""))
            .map_err(|e| e.to_string())?;
        if mac.len() == 32 {
            let mut mac_c =
                <HmacSha256 as Mac>::new_from_slice(aes_key).map_err(|e| e.to_string())?;
            mac_c.update(&header_bytes);
            mac_c.update(&iv);
            mac_c.update(&ct);
            mac_c
                .verify_slice(&mac)
                .map_err(|_| "bad_mac".to_string())?;
        }
    }

    // v2 authenticity: Ed25519 over header||iv||ct against official JWKS.
    if is_v2 {
        let pk = official_ed25519.ok_or("official_ed25519_required_for_v2")?;
        let sig_b64 = bundle
            .get("sig_b64")
            .and_then(|v| v.as_str())
            .ok_or("missing_ed25519_sig")?;
        let mut msg = Vec::with_capacity(header_bytes.len() + iv.len() + ct.len());
        msg.extend_from_slice(&header_bytes);
        msg.extend_from_slice(&iv);
        msg.extend_from_slice(&ct);
        verify_ed25519(pk, &msg, sig_b64)?;
    }

    if aes_key.len() != 32 {
        return Err("aes_key must be 32 bytes".into());
    }
    let cipher = Aes256Gcm::new_from_slice(aes_key).map_err(|e| e.to_string())?;
    let nonce = Nonce::from_slice(&iv);
    let pt = cipher
        .decrypt(nonce, ct.as_ref())
        .map_err(|_| "decrypt_failed".to_string())?;
    let payload: Value = serde_json::from_slice(&pt).map_err(|e| e.to_string())?;
    let exp = header.get("exp").and_then(|v| v.as_i64()).unwrap_or(0);
    // iss/opus5 06-P0-1: the issuer may attach a signed license token inside
    // the (already authenticated + encrypted) bundle. Cache the raw token —
    // verification happens on every entitlement resolution.
    if let Some(tok) = payload
        .get("license_token")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| s.starts_with(&gr_probe_core::license_token::token_prefix()))
    {
        if let Err(e) = gr_probe_core::license_token::cache_token(Some(&data_dir()), tok) {
            tracing::warn!("license token cache failed: {e}");
        }
    }
    // Bind decrypted site/domain to header claims when present.
    if let (Some(hs), Some(ps)) = (
        header.get("site_id").and_then(|v| v.as_str()),
        payload.get("site_id").and_then(|v| v.as_str()),
    ) {
        if !hs.is_empty() && !ps.is_empty() && hs != ps {
            return Err("site_id_header_payload_mismatch".into());
        }
    }
    if let (Some(hd), Some(pd)) = (
        header.get("domain").and_then(|v| v.as_str()),
        payload.get("domain").and_then(|v| v.as_str()),
    ) {
        if !hd.is_empty()
            && !pd.is_empty()
            && hd.to_ascii_lowercase() != pd.to_ascii_lowercase()
        {
            return Err("domain_header_payload_mismatch".into());
        }
    }
    entry_from_payload(&payload, exp)
}

pub fn vault_snapshot() -> Value {
    let v = vault();
    json!({
        "ok": true,
        "access_token_present": access_token().is_some(),
        "instance_id": instance_id(),
        "note": "Use POST /api/cloud/sync-sites; plaintext never returned to SPA. Prefer algo-bundle-v2 ECDH.",
        "vault_domains": v.list_domains_meta(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spki_roundtrip() {
        let (secret, public) = generate_ephemeral();
        let b64 = x25519_pub_spki_b64(&public);
        let parsed = x25519_pub_from_spki_b64(&b64).expect("parse");
        assert_eq!(public.as_bytes(), parsed.as_bytes());
        let peer_secret = StaticSecret::random_from_rng(&mut rand::thread_rng());
        let peer_pub = PublicKey::from(&peer_secret);
        let k1 = ecdh_aes_key(&secret, &peer_pub).unwrap();
        let k2 = ecdh_aes_key(&peer_secret, &public).unwrap();
        assert_eq!(k1, k2);
    }
}
