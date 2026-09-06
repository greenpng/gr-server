//! Random console path + single-user scrypt auth (no fixed /admin, no default password).

use crate::db::AdminDb;
use password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use rand::RngCore;
use scrypt::{Params, Scrypt};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const SESSION_TTL_MS: i64 = 12 * 3600 * 1000;
const LOGIN_MAX_FAIL: usize = 8;
const LOGIN_WINDOW_MS: i64 = 900_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapSecrets {
    pub username: String,
    pub password: String,
    pub console_path: String,
}

pub struct AdminAuth {
    pub console_path: String,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn random_hex(n_bytes: usize) -> String {
    let mut b = vec![0u8; n_bytes];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

pub fn hash_password(password: &str) -> Result<String, String> {
    if password.len() < 10 {
        return Err("password must be at least 10 characters".into());
    }
    let salt = SaltString::generate(&mut rand::thread_rng());
    let params = Params::new(15, 8, 1, 32).map_err(|e| e.to_string())?;
    Scrypt
        .hash_password_customized(password.as_bytes(), None, None, params, &salt)
        .map(|h| h.to_string())
        .map_err(|e| e.to_string())
}

pub fn verify_password(password: &str, stored: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(stored) else {
        return false;
    };
    Scrypt
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

fn gen_console_path() -> String {
    // Obscure path segment — not /admin
    format!("c-{}", &random_hex(16)[..24])
}

/// Order of preference for a fresh bootstrap: env (`GR_ADMIN_USER` /
/// `GR_ADMIN_PASSWORD`, legacy `GR_`/`GR_`) → existing bootstrap file → random generation.
/// An env password that fails the min-length policy is ignored with a warning
/// instead of aborting the install.
fn env_credentials() -> (Option<String>, Option<String>) {
    let user = gr_abi::env::get("ADMIN_USER")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let pass = gr_abi::env::get("ADMIN_PASSWORD")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    (user, pass)
}

fn resolve_credentials(
    secrets: Option<&BootstrapSecrets>,
    env_user: Option<&str>,
    env_pass: Option<&str>,
) -> (String, String) {
    let env_user_ok = env_user.filter(|u| !u.is_empty());
    // An env password that fails the min-length policy is ignored with a
    // warning instead of aborting the install.
    let env_pass_ok = env_pass.filter(|p| p.len() >= 10);
    if let Some(p) = env_pass_ok {
        let u = env_user_ok
            .map(str::to_string)
            .or_else(|| secrets.map(|s| s.username.clone()))
            .unwrap_or_else(|| format!("u{}", &random_hex(4)[..6]));
        return (u, p.to_string());
    }
    let user = env_user_ok
        .map(str::to_string)
        .or_else(|| secrets.map(|s| s.username.clone()))
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| format!("u{}", &random_hex(4)[..6]));
    let pass = secrets
        .map(|s| s.password.clone())
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| format!("gr-{}", &random_hex(12)[..16]));
    (user, pass)
}

fn write_secrets_file(secrets_file: &Path, username: &str, password: &str, console_path: &str) -> Result<(), String> {
    if let Some(parent) = secrets_file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let body = format!(
        "username={username}\npassword={password}\nconsole_path=/{console_path}/\n# store securely; change password after first login\n"
    );
    std::fs::write(secrets_file, body).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(secrets_file, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

impl AdminAuth {
    pub fn bootstrap(db: &AdminDb, secrets_file: &Path) -> Result<Self, String> {
        let console_path = if let Some(p) = db.get_setting("console_path")? {
            p
        } else if secrets_file.is_file() {
            parse_secrets_file(secrets_file)
                .map(|s| s.console_path)
                .unwrap_or_else(|_| gen_console_path())
        } else {
            gen_console_path()
        };
        db.set_setting("console_path", &console_path)?;

        let existing = db.list_users()?;
        let (env_user, env_pass) = env_credentials();
        if existing.is_empty() {
            let secrets = if secrets_file.is_file() {
                parse_secrets_file(secrets_file).ok()
            } else {
                None
            };
            let (username, password) = resolve_credentials(
                secrets.as_ref(),
                env_user.as_deref(),
                env_pass.as_deref(),
            );
            if env_pass.is_some() && password != env_pass.as_deref().unwrap_or("") {
                tracing::warn!(
                    "GR_ADMIN_PASSWORD ignored: must be at least 10 characters (falling back); set a valid password then re-run gr install"
                );
            }
            let h = hash_password(&password)?;
            db.ensure_user(&username, &h)?;
            write_secrets_file(secrets_file, &username, &password, &console_path)?;
            tracing::warn!(
                path = %secrets_file.display(),
                "admin bootstrap secrets written (single user, random path)"
            );
        } else if !secrets_file.is_file() {
            // Re-install onto an existing control DB with a wiped data dir:
            // the one-time password file is gone, so recover it from env when
            // the operator provides one; otherwise fail loudly instead of
            // silently locking everyone out of the panel.
            if let (Some(u), Some(p)) = (env_user, env_pass) {
                let h = hash_password(&p)?;
                db.update_user_password(&u, &h)?;
                write_secrets_file(secrets_file, &u, &p, &console_path)?;
                tracing::warn!(
                    path = %secrets_file.display(),
                    "admin password recovered from GR_ADMIN_USER/GR_ADMIN_PASSWORD after data-dir wipe"
                );
            } else {
                tracing::warn!(
                    "admin users exist in the control DB but {} is missing; to recover, set GR_ADMIN_USER and GR_ADMIN_PASSWORD (>= 10 chars; legacy GR_/GR_ accepted) and re-run gr install",
                    secrets_file.display()
                );
            }
        }

        Ok(Self {
            console_path,
        })
    }

    pub fn login(
        &self,
        db: &AdminDb,
        peer: &str,
        username: &str,
        password: &str,
    ) -> Result<String, String> {
        if !db.login_rate_ok(peer, LOGIN_WINDOW_MS, LOGIN_MAX_FAIL as i64)? {
            return Err("rate_limited".into());
        }
        let Some(hash) = db.user_hash(username)? else {
            db.record_login_failure(peer)?;
            return Err("invalid_credentials".into());
        };
        if !verify_password(password, &hash) {
            db.record_login_failure(peer)?;
            return Err("invalid_credentials".into());
        }
        let token = random_hex(24);
        let exp = now_ms() + SESSION_TTL_MS;
        db.put_session(&token, username, exp)?;
        Ok(token)
    }

    /// Mint a panel session after official-site OAuth (no password).
    /// The local username must already exist — never fall back to the first user.
    pub fn create_session(&self, db: &AdminDb, username: &str) -> Result<String, String> {
        let users = db.list_users()?;
        if !users.iter().any(|u| u == username) {
            return Err("oauth_local_user_missing".into());
        }
        let token = random_hex(24);
        let exp = now_ms() + SESSION_TTL_MS;
        db.put_session(&token, username, exp)?;
        Ok(token)
    }

    pub fn check_session(&self, db: &AdminDb, token: &str) -> Result<Option<String>, String> {
        db.get_session(token, now_ms())
    }

    pub fn logout(&self, db: &AdminDb, token: &str) -> Result<(), String> {
        if token.is_empty() {
            return Ok(());
        }
        db.delete_session(token)
    }

    pub fn session_ttl_secs() -> i64 {
        SESSION_TTL_MS / 1000
    }
}

fn parse_secrets_file(path: &Path) -> Result<BootstrapSecrets, String> {
    let s = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut username = String::new();
    let mut password = String::new();
    let mut console_path = String::new();
    for line in s.lines() {
        if let Some(v) = line.strip_prefix("username=") {
            username = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("password=") {
            password = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("console_path=") {
            console_path = v.trim().trim_matches('/').to_string();
            if let Some(rest) = console_path.strip_prefix("c-") {
                console_path = format!("c-{rest}");
            }
            // normalize /c-xxx/ → c-xxx
            console_path = console_path.trim_matches('/').to_string();
        } else if let Some(v) = line.strip_prefix("console=") {
            console_path = v.trim().trim_matches('/').to_string();
        }
    }
    if username.is_empty() || password.is_empty() {
        return Err("incomplete secrets file".into());
    }
    if console_path.is_empty() {
        console_path = gen_console_path();
    }
    Ok(BootstrapSecrets {
        username,
        password,
        console_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secrets(user: &str, pass: &str) -> BootstrapSecrets {
        BootstrapSecrets {
            username: user.to_string(),
            password: pass.to_string(),
            console_path: "c-test".to_string(),
        }
    }

    #[test]
    fn resolve_prefers_valid_env_password() {
        let (u, p) = resolve_credentials(
            Some(&secrets("fileuser", "filepass-ok")),
            Some("envuser"),
            Some("envpassword-123"),
        );
        assert_eq!(u, "envuser");
        assert_eq!(p, "envpassword-123");
    }

    #[test]
    fn resolve_env_user_without_env_password_falls_back_to_file() {
        let (u, p) = resolve_credentials(
            Some(&secrets("fileuser", "filepass-ok")),
            Some("envuser"),
            None,
        );
        assert_eq!(u, "envuser");
        assert_eq!(p, "filepass-ok");
    }

    #[test]
    fn resolve_short_env_password_is_ignored() {
        let (u, p) = resolve_credentials(
            Some(&secrets("fileuser", "filepass-ok")),
            Some("envuser"),
            Some("short"),
        );
        assert_eq!(u, "envuser");
        assert_eq!(p, "filepass-ok");
    }

    #[test]
    fn resolve_generates_random_without_sources() {
        let (u, p) = resolve_credentials(None, None, None);
        assert!(u.starts_with('u'));
        assert!(p.starts_with("gr-"));
        assert!(p.len() >= 16);
    }

    #[test]
    fn resolve_env_without_file_uses_env_user_and_pass() {
        let (u, p) = resolve_credentials(None, Some("ops"), Some("long-enough-pass"));
        assert_eq!(u, "ops");
        assert_eq!(p, "long-enough-pass");
    }

    #[test]
    fn parse_secrets_file_accepts_legacy_console_field() {
        let dir = std::env::temp_dir().join(format!("gr-admin-test-{}", std::process::id()));
        let f = dir.join("secrets.txt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &f,
            "username=u1\npassword=pw-12345678\nconsole=/admin/mx-abc\n",
        )
        .unwrap();
        let s = parse_secrets_file(&f).expect("legacy console= format parses");
        assert_eq!(s.username, "u1");
        // legacy `console=` value is normalized like `console_path=` is.
        assert!(s.console_path == "admin/mx-abc" || s.console_path == "/admin/mx-abc");
        let _ = std::fs::remove_file(&f);
        let _ = std::fs::remove_dir(&dir);
    }
}
