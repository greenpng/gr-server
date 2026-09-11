//! Control-plane admin store — PostgreSQL only (`control` schema).
//!
//! Shares `GR_ADMIN_DATABASE_URL` (legacy `GR_`/`GR_`) with the probe-plane
//! admin DB (`public` schema). SQLite is not used.

use postgres::{Client, Config, NoTls};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::mpsc::{self, SyncSender};
use std::time::{SystemTime, UNIX_EPOCH};

type JobFn = Box<dyn FnOnce(&mut Client) + Send>;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn require_admin_dsn() -> Result<String, String> {
    if let Some(v) = gr_abi::env::get("ADMIN_DATABASE_URL") {
        let t = v.trim();
        if !t.is_empty() {
            return Ok(t.to_string());
        }
    }
    Err(
        "GR_ADMIN_DATABASE_URL is required (legacy GR_/GR_ accepted). SQLite control-plane store was removed."
            .into(),
    )
}

fn connect(dsn: &str) -> Result<Client, String> {
    let mut cfg: Config = dsn
        .parse()
        .map_err(|e| format!("control admin pg dsn parse: {e}"))?;
    cfg.application_name("gr-control-admin");
    let mut c = cfg
        .connect(NoTls)
        .map_err(|e| format!("control admin pg connect: {e}"))?;
    // P1-3 (iss/grok4.6/05): keep runtime NOTICE spam (schema migrations) off
    // the admin connection; session-level SET per admin/db.rs connect_admin_pg.
    let _ = c.batch_execute("SET client_min_messages = warning");
    Ok(c)
}

fn redact_dsn(dsn: &str) -> String {
    if let Some(at) = dsn.find('@') {
        format!("postgres://***@{}", &dsn[at + 1..])
    } else {
        "postgres://***".into()
    }
}

const PG_SCHEMA: &str = r#"
CREATE SCHEMA IF NOT EXISTS control;
CREATE TABLE IF NOT EXISTS control.admin_users (
  username TEXT PRIMARY KEY,
  password_hash TEXT NOT NULL,
  created_ms BIGINT NOT NULL
);
CREATE TABLE IF NOT EXISTS control.admin_sessions (
  token TEXT PRIMARY KEY,
  username TEXT NOT NULL,
  exp_ms BIGINT NOT NULL,
  created_ms BIGINT NOT NULL
);
CREATE TABLE IF NOT EXISTS control.admin_login_failures (
  peer TEXT NOT NULL,
  ms BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_control_admin_login_failures_peer_ms
  ON control.admin_login_failures(peer, ms);
CREATE TABLE IF NOT EXISTS control.sites (
  site_id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  collect_enabled INTEGER NOT NULL DEFAULT 1,
  notes TEXT NOT NULL DEFAULT '',
  edge_mode TEXT NOT NULL DEFAULT 'first_party',
  pv_base TEXT NOT NULL DEFAULT '',
  gv_base TEXT NOT NULL DEFAULT '',
  fe_load TEXT NOT NULL DEFAULT 'pv',
  upload_ingest TEXT NOT NULL DEFAULT 'gv',
  poll_method TEXT NOT NULL DEFAULT 'both',
  entry_js TEXT NOT NULL DEFAULT 'gr.js',
  crypto_json TEXT NOT NULL DEFAULT '{}',
  cookie_fields TEXT NOT NULL DEFAULT '[]',
  embed_token TEXT NOT NULL DEFAULT '',
  consent_confirmed_at BIGINT NOT NULL DEFAULT 0,
  consent_notice_version TEXT NOT NULL DEFAULT '',
  created_ms BIGINT NOT NULL,
  updated_ms BIGINT NOT NULL
);
CREATE TABLE IF NOT EXISTS control.root_domains (
  domain_id TEXT PRIMARY KEY,
  site_id TEXT NOT NULL,
  root_domain TEXT NOT NULL,
  collect_enabled INTEGER NOT NULL DEFAULT 1,
  created_ms BIGINT NOT NULL,
  UNIQUE(root_domain)
);
CREATE INDEX IF NOT EXISTS idx_control_root_domains_site ON control.root_domains(site_id);
CREATE TABLE IF NOT EXISTS control.admin_settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS control.audit_log (
  id BIGSERIAL PRIMARY KEY,
  actor TEXT NOT NULL,
  action TEXT NOT NULL,
  target TEXT NOT NULL DEFAULT '',
  detail_json TEXT NOT NULL DEFAULT '{}',
  ms BIGINT NOT NULL
);
CREATE TABLE IF NOT EXISTS control.module_versions (
  name TEXT NOT NULL,
  version TEXT NOT NULL,
  path TEXT NOT NULL,
  active INTEGER NOT NULL DEFAULT 0,
  meta_json TEXT NOT NULL DEFAULT '{}',
  PRIMARY KEY(name, version)
);
CREATE TABLE IF NOT EXISTS control.worker_settings (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  analyze_workers INTEGER NOT NULL DEFAULT 1,
  ingest_workers INTEGER NOT NULL DEFAULT 1,
  gateway_workers INTEGER NOT NULL DEFAULT 1,
  updated_ms BIGINT NOT NULL
);
"#;

pub struct AdminDb {
    jobs: SyncSender<JobFn>,
    pub backend_label: String,
}

/// Schema bootstrap for the control store — run on initial connect AND on
/// every STO-02 self-heal reconnect (all statements are idempotent).
fn bootstrap(c: &mut Client) -> Result<(), String> {
    c.batch_execute(PG_SCHEMA)
        .map_err(|e| format!("control admin schema: {e}"))?;
    // iss/opus5 02-§5 consent gate: additive migration for existing installs.
    c.batch_execute(
        "ALTER TABLE control.sites ADD COLUMN IF NOT EXISTS consent_confirmed_at BIGINT NOT NULL DEFAULT 0;
         ALTER TABLE control.sites ADD COLUMN IF NOT EXISTS consent_notice_version TEXT NOT NULL DEFAULT '';
         ALTER TABLE control.sites ADD COLUMN IF NOT EXISTS cookie_fields TEXT NOT NULL DEFAULT '[]';
         ALTER TABLE control.sites ADD COLUMN IF NOT EXISTS embed_token TEXT NOT NULL DEFAULT '';",
    )
    .map_err(|e| format!("control admin consent migration: {e}"))?;
    let ts = now_ms();
    c.execute(
        "INSERT INTO control.worker_settings(id, analyze_workers, ingest_workers, gateway_workers, updated_ms)
         VALUES (1,1,1,1,$1) ON CONFLICT (id) DO NOTHING",
        &[&ts],
    )
    .map(|_| ())
    .map_err(|e| format!("control worker_settings: {e}"))
}

impl AdminDb {
    pub fn open(_legacy_sqlite_path: &std::path::Path) -> Result<Self, String> {
        Self::open_postgres()
    }

    pub fn open_postgres() -> Result<Self, String> {
        let dsn = require_admin_dsn()?;
        let label = redact_dsn(&dsn);
        let (jobs_tx, jobs_rx) = mpsc::sync_channel::<JobFn>(64);
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let dsn_owned = dsn.clone();
        let label_t = label.clone();
        std::thread::Builder::new()
            .name("gr-control-admin-pg".into())
            .spawn(move || {
                let mut client = match connect(&dsn_owned) {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                if let Err(e) = bootstrap(&mut client) {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
                let _ = ready_tx.send(Ok(()));
                // iss/audit STO-02 (fifth store — found by the v1.0.13 lab
                // bounce test): this control-plane job worker (login/sessions/
                // audit/worker_settings) shared the same single-connection
                // pattern as the probe/biz/assoc/plane-admin stores. Self-heal
                // a broken connection (PG restart / failover) instead of
                // erroring every job on a dead socket until process restart.
                while let Ok(job) = jobs_rx.recv() {
                    if client.is_closed() {
                        let mut delay_ms: u64 = 500;
                        for attempt in 1..=6u32 {
                            match connect(&dsn_owned).and_then(|mut c| bootstrap(&mut c).map(|_| c)) {
                                Ok(c) => {
                                    client = c;
                                    tracing::info!(
                                        "control admin pg reconnected after connection loss (attempt {attempt})"
                                    );
                                    break;
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "control admin pg reconnect failed attempt {attempt}/6: {e}"
                                    );
                                }
                            }
                            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                            delay_ms = (delay_ms * 2).min(8_000);
                        }
                    }
                    job(&mut client);
                }
            })
            .map_err(|e| format!("control admin pg worker: {e}"))?;
        ready_rx
            .recv()
            .map_err(|e| format!("control admin pg ready: {e}"))??;
        tracing::info!(dsn = %label_t, "control admin store backend=postgres schema=control");
        Ok(Self {
            jobs: jobs_tx,
            backend_label: label,
        })
    }

    fn with_pg<T, F>(&self, f: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(&mut Client) -> Result<T, String> + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        self.jobs
            .send(Box::new(move |client| {
                let _ = tx.send(f(client));
            }))
            .map_err(|e| format!("control admin job send: {e}"))?;
        rx.recv()
            .map_err(|e| format!("control admin job recv: {e}"))?
    }

    pub fn ensure_user(&self, username: &str, password_hash: &str) -> Result<bool, String> {
        let username = username.to_string();
        let password_hash = password_hash.to_string();
        self.with_pg(move |c| {
            let exists: Option<String> = c
                .query_opt(
                    "SELECT username FROM control.admin_users WHERE username=$1",
                    &[&username],
                )
                .map_err(|e| e.to_string())?
                .map(|r| r.get(0));
            if exists.is_some() {
                return Ok(false);
            }
            c.execute(
                "INSERT INTO control.admin_users(username, password_hash, created_ms) VALUES ($1,$2,$3)",
                &[&username, &password_hash, &now_ms()],
            )
            .map_err(|e| e.to_string())?;
            Ok(true)
        })
    }

    /// Recover path for a wiped data dir: regenerate (or create) the admin
    /// user's password hash from operator-provided env credentials.
    pub fn update_user_password(&self, username: &str, password_hash: &str) -> Result<(), String> {
        let username = username.to_string();
        let password_hash = password_hash.to_string();
        self.with_pg(move |c| {
            c.execute(
                "INSERT INTO control.admin_users(username, password_hash, created_ms)
                 VALUES ($1,$2,$3)
                 ON CONFLICT (username) DO UPDATE SET password_hash=EXCLUDED.password_hash",
                &[&username, &password_hash, &now_ms()],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
    }

    pub fn list_users(&self) -> Result<Vec<String>, String> {
        self.with_pg(|c| {
            let rows = c
                .query("SELECT username FROM control.admin_users", &[])
                .map_err(|e| e.to_string())?;
            Ok(rows.into_iter().map(|r| r.get(0)).collect())
        })
    }

    pub fn user_hash(&self, username: &str) -> Result<Option<String>, String> {
        let username = username.to_string();
        self.with_pg(move |c| {
            Ok(c.query_opt(
                "SELECT password_hash FROM control.admin_users WHERE username=$1",
                &[&username],
            )
            .map_err(|e| e.to_string())?
            .map(|r| r.get(0)))
        })
    }

    /// Shared PostgreSQL login throttle. A process restart or another panel
    /// instance cannot reset the failure counter.
    pub fn login_rate_ok(&self, peer: &str, window_ms: i64, max_fail: i64) -> Result<bool, String> {
        let peer = peer.to_string();
        self.with_pg(move |c| {
            let now = now_ms();
            // P-.1: the count must filter by window expiry itself. A data
            // modifying CTE (DELETE) runs against the statement snapshot, so a
            // row that was already expired when the statement started is
            // deleted and not counted — but never rely on that: count only
            // `ms >= cutoff` so expired rows can never inflate the window
            // regardless of snapshot/visibility semantics.
            let count: i64 = c.query_one(
                "WITH purged AS (
                   DELETE FROM control.admin_login_failures WHERE ms < $1
                 )
                 SELECT COUNT(*) FROM control.admin_login_failures
                 WHERE peer=$2 AND ms >= $1",
                &[&(now - window_ms), &peer],
            ).map_err(|e| e.to_string())?.get(0);
            Ok(count < max_fail)
        })
    }

    pub fn record_login_failure(&self, peer: &str) -> Result<(), String> {
        let peer = peer.to_string();
        self.with_pg(move |c| {
            c.execute(
                "INSERT INTO control.admin_login_failures(peer, ms) VALUES ($1,$2)",
                &[&peer, &now_ms()],
            ).map_err(|e| e.to_string())?;
            Ok(())
        })
    }

    /// iss/opus5 S-10: admin session tokens are stored as SHA-256 digests,
    /// never plaintext (DB read privilege must not equal admin takeover).
    fn hash_session_token(token: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(token.as_bytes());
        hex::encode(h.finalize())
    }

    pub fn put_session(&self, token: &str, username: &str, exp_ms: i64) -> Result<(), String> {
        let token = Self::hash_session_token(token);
        let username = username.to_string();
        self.with_pg(move |c| {
            c.execute(
                "INSERT INTO control.admin_sessions(token, username, exp_ms, created_ms)
                 VALUES ($1,$2,$3,$4)
                 ON CONFLICT (token) DO UPDATE SET username=EXCLUDED.username, exp_ms=EXCLUDED.exp_ms",
                &[&token, &username, &exp_ms, &now_ms()],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
    }

    pub fn delete_session(&self, token: &str) -> Result<(), String> {
        let token = Self::hash_session_token(token);
        self.with_pg(move |c| {
            c.execute(
                "DELETE FROM control.admin_sessions WHERE token=$1",
                &[&token],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
    }

    pub fn get_session(&self, token: &str, now: i64) -> Result<Option<String>, String> {
        let token = Self::hash_session_token(token);
        self.with_pg(move |c| {
            let row = c
                .query_opt(
                    "SELECT username, exp_ms FROM control.admin_sessions WHERE token=$1",
                    &[&token],
                )
                .map_err(|e| e.to_string())?;
            match row {
                Some(r) => {
                    let u: String = r.get(0);
                    let exp: i64 = r.get(1);
                    if exp >= now {
                        Ok(Some(u))
                    } else {
                        Ok(None)
                    }
                }
                None => Ok(None),
            }
        })
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, String> {
        let key = key.to_string();
        self.with_pg(move |c| {
            Ok(c.query_opt(
                "SELECT value FROM control.admin_settings WHERE key=$1",
                &[&key],
            )
            .map_err(|e| e.to_string())?
            .map(|r| r.get(0)))
        })
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), String> {
        let key = key.to_string();
        let value = value.to_string();
        self.with_pg(move |c| {
            c.execute(
                "INSERT INTO control.admin_settings(key, value) VALUES ($1,$2)
                 ON CONFLICT (key) DO UPDATE SET value=EXCLUDED.value",
                &[&key, &value],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
    }

    pub fn audit(&self, actor: &str, action: &str, target: &str, detail: Value) -> Result<(), String> {
        let actor = actor.to_string();
        let action = action.to_string();
        let target = target.to_string();
        let detail_s = detail.to_string();
        self.with_pg(move |c| {
            c.execute(
                "INSERT INTO control.audit_log(actor, action, target, detail_json, ms) VALUES ($1,$2,$3,$4,$5)",
                &[&actor, &action, &target, &detail_s, &now_ms()],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
    }

    // --- SDK key management (P0: panel signs keys; probe validates) ---
    // Keys live in `public.sdk_keys` — the same table the probe plane's
    // `check_backend_key` reads at ingest time. The control plane is the
    // write-side authority; the probe plane is read-only at runtime.

    pub fn list_sdk_keys(&self, site_id: Option<&str>) -> Result<Vec<Value>, String> {
        let site_id_owned = site_id.map(|s| s.to_string());
        self.with_pg(move |c| {
            let rows = if let Some(ref s) = site_id_owned {
                c.query(
                    "SELECT key_id, site_id, kind, secret_prefix, status, allowed_origins_json, created_ms, revoked_ms
                     FROM public.sdk_keys WHERE site_id=$1 ORDER BY created_ms DESC",
                    &[&s],
                )
            } else {
                c.query(
                    "SELECT key_id, site_id, kind, secret_prefix, status, allowed_origins_json, created_ms, revoked_ms
                     FROM public.sdk_keys ORDER BY created_ms DESC",
                    &[],
                )
            }
            .map_err(|e| e.to_string())?;
            Ok(rows
                .iter()
                .map(|r| {
                    json!({
                        "key_id": r.get::<_, String>(0),
                        "site_id": r.get::<_, String>(1),
                        "kind": r.get::<_, String>(2),
                        "secret_prefix": r.get::<_, String>(3),
                        "status": r.get::<_, String>(4),
                        "allowed_origins": serde_json::from_str::<Value>(&r.get::<_, String>(5)).unwrap_or(json!([])),
                        "created_ms": r.get::<_, i64>(6),
                        "revoked_ms": r.get::<_, Option<i64>>(7),
                    })
                })
                .collect())
        })
    }

    /// Insert a new SDK key. Returns the full row including the raw secret
    /// (shown once by the caller). `secret_hash` is SHA-256 of the raw secret.
    pub fn insert_sdk_key(
        &self,
        site_id: &str,
        kind: &str,
        secret_hash: &str,
        secret_prefix: &str,
        allowed_origins: &[String],
    ) -> Result<Value, String> {
        let key_id = format!("sdk_{}", &hex_id());
        let origins = serde_json::to_string(allowed_origins).unwrap_or_else(|_| "[]".into());
        let site_id = site_id.to_string();
        let kind = kind.to_string();
        let secret_hash = secret_hash.to_string();
        let secret_prefix = secret_prefix.to_string();
        self.with_pg(move |c| {
            // Ensure the probe-side table exists (probe plane normally creates
            // it at boot; this guard keeps panel-first flows working).
            c.batch_execute(
                "CREATE TABLE IF NOT EXISTS public.sdk_keys (
                    key_id TEXT PRIMARY KEY,
                    site_id TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    secret_hash TEXT NOT NULL,
                    secret_prefix TEXT NOT NULL DEFAULT '',
                    status TEXT NOT NULL DEFAULT 'active',
                    allowed_origins_json TEXT NOT NULL DEFAULT '[]',
                    created_ms BIGINT NOT NULL,
                    revoked_ms BIGINT
                 );
                 CREATE INDEX IF NOT EXISTS idx_admin_sdk_site ON public.sdk_keys(site_id);",
            )
            .map_err(|e| e.to_string())?;
            c.execute(
                "INSERT INTO public.sdk_keys(key_id, site_id, kind, secret_hash, secret_prefix, status, allowed_origins_json, created_ms)
                 VALUES ($1,$2,$3,$4,$5,'active',$6,$7)",
                &[&key_id, &site_id, &kind, &secret_hash, &secret_prefix, &origins, &now_ms()],
            )
            .map_err(|e| e.to_string())?;
            Ok(json!({
                "key_id": key_id,
                "site_id": site_id,
                "kind": kind,
                "secret_prefix": secret_prefix,
                "status": "active",
                "allowed_origins": origins,
            }))
        })
    }

    pub fn revoke_sdk_key(&self, key_id: &str) -> Result<(), String> {
        let key_id = key_id.to_string();
        self.with_pg(move |c| {
            let n = c
                .execute(
                    "UPDATE public.sdk_keys SET status='revoked', revoked_ms=$1 WHERE key_id=$2",
                    &[&now_ms(), &key_id],
                )
                .map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("key_not_found".into());
            }
            Ok(())
        })
    }

    pub fn upsert_site(&self, site: &crate::sites::SiteRecord) -> Result<(), String> {
        let site = site.clone();
        let exists = self.site_exists(&site.site_id)?;
        // iss/opus5 02-§5 consent gate: creation requires the owner's explicit
        // confirmation; updates keep whatever confirmation was recorded earlier.
        if !exists && !site.consent_confirmed {
            return Err("consent_required: probing behavior must be disclosed to visitors and confirmed by the site owner before creating a site (consent_confirmed=true)".into());
        }
        self.with_pg(move |c| {
            let now = now_ms();
            let en: i32 = if site.collect_enabled { 1 } else { 0 };
            let crypto = serde_json::to_string(&site.crypto).unwrap_or_else(|_| "{}".into());
            let cookie_fields = serde_json::to_string(&site.cookie_fields).unwrap_or_else(|_| "[]".into());
            let consent_at: i64 = if exists { -1 } else { now };
            let embed_token = site.embed_token.clone();
            c.execute(
                "INSERT INTO control.sites(site_id, name, collect_enabled, notes, edge_mode, pv_base, gv_base, fe_load, upload_ingest, poll_method, entry_js, crypto_json, cookie_fields, embed_token, consent_confirmed_at, consent_notice_version, created_ms, updated_ms)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$17)
                 ON CONFLICT(site_id) DO UPDATE SET
                   name=EXCLUDED.name, collect_enabled=EXCLUDED.collect_enabled, notes=EXCLUDED.notes,
                   edge_mode=EXCLUDED.edge_mode, pv_base=EXCLUDED.pv_base, gv_base=EXCLUDED.gv_base,
                   fe_load=EXCLUDED.fe_load, upload_ingest=EXCLUDED.upload_ingest, poll_method=EXCLUDED.poll_method,
                   entry_js=EXCLUDED.entry_js, crypto_json=EXCLUDED.crypto_json, cookie_fields=EXCLUDED.cookie_fields,
                   embed_token=CASE WHEN EXCLUDED.embed_token = '' THEN control.sites.embed_token ELSE EXCLUDED.embed_token END,
                   updated_ms=EXCLUDED.updated_ms",
                &[
                    &site.site_id,
                    &site.name,
                    &en,
                    &site.notes,
                    &site.edge_mode,
                    &site.pv_base,
                    &site.gv_base,
                    &site.fe_load,
                    &site.upload_ingest,
                    &site.poll_method,
                    &site.entry_js,
                    &crypto,
                    &cookie_fields,
                    &embed_token,
                    &consent_at,
                    &site.consent_notice_version,
                    &now,
                ],
            )
            .map_err(|e| e.to_string())?;
            c.execute(
                "DELETE FROM control.root_domains WHERE site_id=$1",
                &[&site.site_id],
            )
            .map_err(|e| e.to_string())?;
            for root in &site.root_domains {
                let id = format!("d_{}", hex_id());
                let host = crate::domain::normalize_hostname(root);
                c.execute(
                    "INSERT INTO control.root_domains(domain_id, site_id, root_domain, collect_enabled, created_ms)
                     VALUES ($1,$2,$3,1,$4)
                     ON CONFLICT(root_domain) DO UPDATE SET site_id=EXCLUDED.site_id, collect_enabled=1",
                    &[&id, &site.site_id, &host, &now],
                )
                .map_err(|e| e.to_string())?;
            }
            Ok(())
        })
    }

    pub fn site_exists(&self, site_id: &str) -> Result<bool, String> {
        let site_id = site_id.to_string();
        self.with_pg(move |c| {
            let n: i64 = c
                .query_one(
                    "SELECT count(*) FROM control.sites WHERE site_id=$1",
                    &[&site_id],
                )
                .map_err(|e| e.to_string())?
                .get(0);
            Ok(n > 0)
        })
    }

    pub fn count_sites(&self) -> Result<i64, String> {
        self.with_pg(|c| {
            let n: i64 = c
                .query_one("SELECT count(*) FROM control.sites", &[])
                .map_err(|e| e.to_string())?
                .get(0);
            Ok(n)
        })
    }

    pub fn list_sites(&self) -> Result<Vec<Value>, String> {
        self.with_pg(|c| {
            let rows = c
                .query(
                    "SELECT site_id, name, collect_enabled, notes, edge_mode, pv_base, gv_base, fe_load, upload_ingest, poll_method, entry_js, crypto_json, cookie_fields, COALESCE(embed_token,''), consent_confirmed_at, consent_notice_version, created_ms, updated_ms
                     FROM control.sites ORDER BY updated_ms DESC",
                    &[],
                )
                .map_err(|e| e.to_string())?;
            let root_rows = c
                .query(
                    "SELECT site_id, root_domain FROM control.root_domains ORDER BY root_domain",
                    &[],
                )
                .map_err(|e| e.to_string())?;
            let mut roots: HashMap<String, Vec<String>> = HashMap::new();
            for r in root_rows {
                let sid: String = r.get(0);
                let host: String = r.get(1);
                roots.entry(sid).or_default().push(host);
            }
            let mut out = Vec::new();
            for r in rows {
                let sid: String = r.get(0);
                let name: String = r.get(1);
                let en: i32 = r.get(2);
                let notes: String = r.get(3);
                let edge: String = r.get(4);
                let pv: String = r.get(5);
                let gv: String = r.get(6);
                let fe: String = r.get(7);
                let up: String = r.get(8);
                let poll: String = r.get(9);
                let entry: String = r.get(10);
                let crypto: String = r.get(11);
                let cookie_fields: String = r.get(12);
                let embed_token: String = r.get(13);
                let consent_at: i64 = r.get(14);
                let consent_ver: String = r.get(15);
                let created: i64 = r.get(16);
                let updated: i64 = r.get(17);
                let crypto_v: Value = serde_json::from_str(&crypto).unwrap_or(json!({}));
                let cookie_v: Value = serde_json::from_str(&cookie_fields).unwrap_or(json!([]));
                out.push(json!({
                    "site_id": sid,
                    "name": name,
                    "collect_enabled": en != 0,
                    "notes": notes,
                    "edge_mode": edge,
                    "pv_base": pv,
                    "gv_base": gv,
                    "fe_load": fe,
                    "upload_ingest": up,
                    "poll_method": poll,
                    "entry_js": entry,
                    "crypto": crypto_v,
                    "cookie_fields": cookie_v,
                    "embed_token": embed_token,
                    "root_domains": roots.get(&sid).cloned().unwrap_or_default(),
                    "consent_confirmed": consent_at > 0,
                    "consent_confirmed_at": consent_at,
                    "consent_notice_version": consent_ver,
                    "created_ms": created,
                    "updated_ms": updated,
                }));
            }
            Ok(out)
        })
    }

    pub fn all_root_domains(&self) -> Result<Vec<(String, String, bool)>, String> {
        self.with_pg(|c| {
            let rows = c
                .query(
                    "SELECT root_domain, site_id, collect_enabled FROM control.root_domains ORDER BY root_domain",
                    &[],
                )
                .map_err(|e| e.to_string())?;
            Ok(rows
                .into_iter()
                .map(|r| {
                    let host: String = r.get(0);
                    let sid: String = r.get(1);
                    let en: i32 = r.get(2);
                    (host, sid, en != 0)
                })
                .collect())
        })
    }

    pub fn get_workers(&self) -> Result<(u32, u32, u32), String> {
        self.with_pg(|c| {
            let row = c
                .query_one(
                    "SELECT analyze_workers, ingest_workers, gateway_workers FROM control.worker_settings WHERE id=1",
                    &[],
                )
                .map_err(|e| e.to_string())?;
            let a: i32 = row.get(0);
            let i: i32 = row.get(1);
            let g: i32 = row.get(2);
            Ok((a.max(1) as u32, i.max(1) as u32, g.max(1) as u32))
        })
    }

    pub fn set_workers(&self, analyze: u32, ingest: u32, gateway: u32) -> Result<(), String> {
        let a = analyze.max(1) as i32;
        let i = ingest.max(1) as i32;
        let g = gateway.max(1) as i32;
        self.with_pg(move |c| {
            c.execute(
                "UPDATE control.worker_settings SET analyze_workers=$1, ingest_workers=$2, gateway_workers=$3, updated_ms=$4 WHERE id=1",
                &[&a, &i, &g, &now_ms()],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
    }

    pub fn record_module(
        &self,
        name: &str,
        version: &str,
        path: &str,
        active: bool,
        meta: &Value,
    ) -> Result<(), String> {
        let name = name.to_string();
        let version = version.to_string();
        let path = path.to_string();
        let active_i: i32 = if active { 1 } else { 0 };
        let meta_s = meta.to_string();
        self.with_pg(move |c| {
            if active_i == 1 {
                c.execute(
                    "UPDATE control.module_versions SET active=0 WHERE name=$1",
                    &[&name],
                )
                .map_err(|e| e.to_string())?;
            }
            c.execute(
                "INSERT INTO control.module_versions(name, version, path, active, meta_json) VALUES ($1,$2,$3,$4,$5)
                 ON CONFLICT(name, version) DO UPDATE SET path=EXCLUDED.path, active=EXCLUDED.active, meta_json=EXCLUDED.meta_json",
                &[&name, &version, &path, &active_i, &meta_s],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
    }

    pub fn list_modules(&self) -> Result<Vec<Value>, String> {
        self.with_pg(|c| {
            let rows = c
                .query(
                    "SELECT name, version, path, active, meta_json FROM control.module_versions ORDER BY name, version",
                    &[],
                )
                .map_err(|e| e.to_string())?;
            let mut out = Vec::new();
            for r in rows {
                let n: String = r.get(0);
                let v: String = r.get(1);
                let p: String = r.get(2);
                let a: i32 = r.get(3);
                let m: String = r.get(4);
                let meta: Value = serde_json::from_str(&m).unwrap_or(json!({}));
                out.push(json!({"name": n, "version": v, "path": p, "active": a != 0, "meta": meta}));
            }
            Ok(out)
        })
    }

    /// Rotate the site embed token. Returns the new token.
    pub fn rotate_embed_token(&self, site_id: &str) -> Result<String, String> {
        let site_id = site_id.to_string();
        let token = crate::sites::mint_embed_token();
        let token2 = token.clone();
        self.with_pg(move |c| {
            let n = c
                .execute(
                    "UPDATE control.sites SET embed_token=$1, updated_ms=$2 WHERE site_id=$3",
                    &[&token2, &now_ms(), &site_id],
                )
                .map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("site_not_found".into());
            }
            Ok(())
        })?;
        Ok(token)
    }

    pub fn get_embed_token(&self, site_id: &str) -> Result<Option<String>, String> {
        let site_id = site_id.to_string();
        self.with_pg(move |c| {
            let row = c
                .query_opt(
                    "SELECT COALESCE(embed_token,'') FROM control.sites WHERE site_id=$1",
                    &[&site_id],
                )
                .map_err(|e| e.to_string())?;
            Ok(row.map(|r| r.get::<_, String>(0)))
        })
    }

    pub fn delete_site(&self, site_id: &str) -> Result<(), String> {
        let site_id = site_id.to_string();
        self.with_pg(move |c| {
            c.execute(
                "DELETE FROM control.root_domains WHERE site_id=$1",
                &[&site_id],
            )
            .map_err(|e| e.to_string())?;
            let n = c
                .execute("DELETE FROM control.sites WHERE site_id=$1", &[&site_id])
                .map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("site_not_found".into());
            }
            Ok(())
        })
    }

    pub fn list_audit(&self, limit: i64) -> Result<Vec<Value>, String> {
        let lim = limit.clamp(1, 500);
        self.with_pg(move |c| {
            let rows = c
                .query(
                    "SELECT id, actor, action, target, detail_json, ms FROM control.audit_log ORDER BY id DESC LIMIT $1",
                    &[&lim],
                )
                .map_err(|e| e.to_string())?;
            let mut out = Vec::new();
            for r in rows {
                let id: i64 = r.get(0);
                let actor: String = r.get(1);
                let action: String = r.get(2);
                let target: String = r.get(3);
                let detail: String = r.get(4);
                let ms: i64 = r.get(5);
                let detail_v: Value = serde_json::from_str(&detail).unwrap_or(json!({}));
                out.push(json!({
                    "id": id,
                    "actor": actor,
                    "action": action,
                    "target": target,
                    "detail": detail_v,
                    "ms": ms,
                }));
            }
            Ok(out)
        })
    }

    pub fn set_module_active(&self, name: &str, version: &str) -> Result<(), String> {
        let name = name.to_string();
        let version = version.to_string();
        self.with_pg(move |c| {
            c.execute(
                "UPDATE control.module_versions SET active=0 WHERE name=$1",
                &[&name],
            )
            .map_err(|e| e.to_string())?;
            let n = c
                .execute(
                    "UPDATE control.module_versions SET active=1 WHERE name=$1 AND version=$2",
                    &[&name, &version],
                )
                .map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("module_version_not_found".into());
            }
            Ok(())
        })
    }
}

fn hex_id() -> String {
    use rand::RngCore;
    let mut b = [0u8; 6];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}
