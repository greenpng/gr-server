//! Admin configuration store — PG preferred (`GR_ADMIN_DATABASE_URL`), SQLite lab fallback.
//!
//! Postgres ops always run on a dedicated OS thread (sync client must not run on Pingora/Tokio).
//!
//! **Connection policy (multi-worker / multi-site):** one long-lived `Client` per process via a
//! job channel — never connect-per-call. Heartbeat / CORS reload used to open a new TCP session
//! every tick and exhausted `max_connections` under `GR_INGEST_N + GW + ANALYZE` scale.

use postgres::{Client, Config, NoTls};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

type JobFn = Box<dyn FnOnce(&mut Client) + Send>;

fn connect_admin_pg(dsn: &str) -> Result<Client, String> {
    let mut cfg: Config = dsn
        .parse()
        .map_err(|e| format!("admin pg dsn parse: {e}"))?;
    cfg.application_name("gr-admin");
    let mut c = cfg
        .connect(NoTls)
        .map_err(|e| format!("admin pg connect: {e}"))?;
    // P1-3 (iss/grok4.6/05): periodic schema ensure emits NOTICE per cycle.
    // Session-level SET (not startup `options`, which deadlocked AdminHub
    // boot in a silent retry loop) keeps runtime NOTICE noise off node logs.
    let _ = c.batch_execute("SET client_min_messages = warning");
    Ok(c)
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS sites (
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
  cookie_fields TEXT NOT NULL DEFAULT '[]',
  embed_token TEXT NOT NULL DEFAULT '',
  created_ms INTEGER NOT NULL,
  updated_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS domains (
  domain_id TEXT PRIMARY KEY,
  site_id TEXT NOT NULL,
  hostname TEXT NOT NULL UNIQUE,
  display_name TEXT NOT NULL DEFAULT '',
  collect_enabled INTEGER NOT NULL DEFAULT 1,
  ssl_status TEXT NOT NULL DEFAULT 'none',
  ssl_expires_ms INTEGER,
  cert_path TEXT,
  key_path TEXT,
  created_ms INTEGER NOT NULL,
  updated_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_domains_site ON domains(site_id);
CREATE TABLE IF NOT EXISTS acme_challenges (
  token TEXT PRIMARY KEY,
  content TEXT NOT NULL,
  domain_id TEXT NOT NULL,
  exp_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS sdk_keys (
  key_id TEXT PRIMARY KEY,
  site_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  secret_hash TEXT NOT NULL,
  secret_prefix TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'active',
  allowed_origins_json TEXT NOT NULL DEFAULT '[]',
  created_ms INTEGER NOT NULL,
  revoked_ms INTEGER
);
CREATE INDEX IF NOT EXISTS idx_sdk_site ON sdk_keys(site_id);
CREATE TABLE IF NOT EXISTS runtime_desired (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  config_json TEXT NOT NULL,
  updated_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS admin_settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS audit_log (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  actor TEXT NOT NULL,
  action TEXT NOT NULL,
  target TEXT NOT NULL DEFAULT '',
  detail_json TEXT NOT NULL DEFAULT '{}',
  ms INTEGER NOT NULL
);
"#;

const PG_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS sites (
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
  cookie_fields TEXT NOT NULL DEFAULT '[]',
  embed_token TEXT NOT NULL DEFAULT '',
  created_ms BIGINT NOT NULL,
  updated_ms BIGINT NOT NULL
);
CREATE TABLE IF NOT EXISTS domains (
  domain_id TEXT PRIMARY KEY,
  site_id TEXT NOT NULL,
  hostname TEXT NOT NULL UNIQUE,
  display_name TEXT NOT NULL DEFAULT '',
  collect_enabled INTEGER NOT NULL DEFAULT 1,
  ssl_status TEXT NOT NULL DEFAULT 'none',
  ssl_expires_ms BIGINT,
  cert_path TEXT,
  key_path TEXT,
  created_ms BIGINT NOT NULL,
  updated_ms BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_admin_domains_site ON domains(site_id);
CREATE TABLE IF NOT EXISTS acme_challenges (
  token TEXT PRIMARY KEY,
  content TEXT NOT NULL,
  domain_id TEXT NOT NULL,
  exp_ms BIGINT NOT NULL
);
CREATE TABLE IF NOT EXISTS sdk_keys (
  key_id TEXT PRIMARY KEY,
  site_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  secret_hash TEXT NOT NULL,
  secret_prefix TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'active',
  allowed_origins_json TEXT NOT NULL DEFAULT '[]',
  created_ms BIGINT NOT NULL,
  revoked_ms BIGINT
);
CREATE INDEX IF NOT EXISTS idx_admin_sdk_site ON sdk_keys(site_id);
CREATE TABLE IF NOT EXISTS runtime_desired (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  config_json TEXT NOT NULL,
  updated_ms BIGINT NOT NULL
);
CREATE TABLE IF NOT EXISTS admin_settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS audit_log (
  id BIGSERIAL PRIMARY KEY,
  actor TEXT NOT NULL,
  action TEXT NOT NULL,
  target TEXT NOT NULL DEFAULT '',
  detail_json TEXT NOT NULL DEFAULT '{}',
  ms BIGINT NOT NULL
);
"#;


enum Backend {
    Sqlite(Mutex<Connection>),
    /// Long-lived Client on a dedicated OS thread (Pingora-safe; 1 conn / process).
    Postgres { jobs: SyncSender<JobFn> },
}

pub struct AdminDb {
    backend: Backend,
    pub data_dir: PathBuf,
    backend_label: String,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn new_id(prefix: &str) -> String {
    use rand::RngCore;
    let mut b = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut b);
    format!("{prefix}_{}", hex::encode(b))
}

/// Normalize edge topology mode stored on `sites` (legacy + derived).
pub fn normalize_edge_mode(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "hybrid" | "hybrid_gv" | "first_party_gv" | "fp_gv" => "hybrid".into(),
        "dual_domain" | "dual" | "pv_gv" | "pv+gv" | "cross_origin" | "cdn" => "dual_domain".into(),
        "first_party" | "first-party" | "fp" | "g5" | "same_origin" | "same-origin" | "" => {
            "first_party".into()
        }
        other if !other.is_empty() => other.to_string(),
        _ => "first_party".into(),
    }
}

/// FE script load channel (runtime vocabulary).
///
/// Panel may send product labels; map them to wire channels:
/// - `first_party` / `hybrid` → same-origin `/g5` pin (hybrid is same load, different edge note)
/// - `pv` / `dual_domain` / `third_party` / `cdn` → CDN-style script origin (`pv_base` when set)
/// - `gv` → scripts from absolute `gv_base`
///
/// Industry note: pure third-party script origins shorten cookie life (Safari);
/// Fingerprint-class products prefer first-party custom subdomain proxies.
pub fn normalize_fe_load(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "pv" | "probe" | "cdn" | "cross_origin" | "dual" | "dual_domain"
        | "third_party" | "third-party" | "3p" => "pv".into(),
        "gv" | "unified" => "gv".into(),
        "hybrid" | "first_party" | "first-party" | "fp" | "same_origin" | "nginx" | "" => {
            "first_party".into()
        }
        _ => "first_party".into(),
    }
}

/// Ingest/open/upload channel for probe materials.
/// Default product path: **`gv`** (Pingora TLS direct on gv host; open/ingest + B8).
/// `first_party` = legacy upload via www `/g5` nginx proxy.
pub fn normalize_upload_ingest(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "pv" | "probe" | "cdn" | "third_party" | "third-party" => "pv".into(),
        "first_party" | "first-party" | "fp" | "g5" | "same_origin" | "hybrid" => {
            "first_party".into()
        }
        "dual_domain" | "dual" | "gv" | "unified" | "unified_gv" | "gateway" | "" => "gv".into(),
        _ => "gv".into(),
    }
}

/// Poll analyses/health method preference.
pub fn normalize_poll_method(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "get" => "get".into(),
        "post" => "post".into(),
        _ => "both".into(),
    }
}

/// True if base is absolute https URL suitable for Pingora gv TLS.
pub fn is_absolute_https(base: &str) -> bool {
    let b = base.trim();
    b.starts_with("https://") && hostname_from_base(b).is_some()
}

fn trim_base(s: &str) -> String {
    s.trim().trim_end_matches('/').to_string()
}

/// Extract bare hostname from absolute URL base (`https://pv.x.com` → `pv.x.com`).
/// Relative paths (`/g5`) return None.
pub fn hostname_from_base(base: &str) -> Option<String> {
    let b = base.trim();
    if b.is_empty() || b.starts_with('/') {
        return None;
    }
    let rest = b
        .strip_prefix("https://")
        .or_else(|| b.strip_prefix("http://"))
        .unwrap_or(b);
    let host = rest.split('/').next().unwrap_or("").split(':').next().unwrap_or("");
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() || host == "localhost" {
        return None;
    }
    Some(host)
}

fn site_json(
    site_id: String,
    name: String,
    collect_enabled: bool,
    notes: String,
    edge_mode: String,
    pv_base: String,
    gv_base: String,
    fe_load: String,
    upload_ingest: String,
    poll_method: String,
    cookie_fields: String,
    embed_token: String,
    created_ms: i64,
    updated_ms: i64,
) -> Value {
    let fe = normalize_fe_load(&fe_load);
    let up = normalize_upload_ingest(&upload_ingest);
    let poll = normalize_poll_method(&poll_method);
    let cookie_v: Vec<String> =
        serde_json::from_str(&cookie_fields).unwrap_or_else(|_| Vec::new());
    // Keep edge_mode in sync with fe_load/upload for older embed clients.
    let mode = if fe == "pv" || up == "pv" {
        if fe == "first_party" && up == "first_party" {
            "first_party".into()
        } else if fe == "first_party" {
            "hybrid".into()
        } else {
            "dual_domain".into()
        }
    } else {
        let m = normalize_edge_mode(&edge_mode);
        if m == "dual_domain" || m == "hybrid" {
            m
        } else {
            "first_party".into()
        }
    };
    json!({
        "site_id": site_id,
        "name": name,
        "collect_enabled": collect_enabled,
        "notes": notes,
        "edge_mode": mode,
        "pv_base": trim_base(&pv_base),
        "gv_base": trim_base(&gv_base),
        "fe_load": fe,
        "upload_ingest": up,
        "poll_method": poll,
        "cookie_fields": cookie_v,
        "embed_token": embed_token,
        "gv_required": true,
        "gv_ssl_required": true,
        "created_ms": created_ms,
        "updated_ms": updated_ms,
    })
}

/// Ensure edge topology columns exist on older admin DBs (SQLite + PG).
fn migrate_site_edge_sql_sqlite(c: &Connection) {
    let _ = c.execute(
        "ALTER TABLE sites ADD COLUMN edge_mode TEXT NOT NULL DEFAULT 'first_party'",
        [],
    );
    let _ = c.execute(
        "ALTER TABLE sites ADD COLUMN pv_base TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = c.execute(
        "ALTER TABLE sites ADD COLUMN gv_base TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = c.execute(
        "ALTER TABLE sites ADD COLUMN fe_load TEXT NOT NULL DEFAULT 'first_party'",
        [],
    );
    let _ = c.execute(
        "ALTER TABLE sites ADD COLUMN upload_ingest TEXT NOT NULL DEFAULT 'first_party'",
        [],
    );
    let _ = c.execute(
        "ALTER TABLE sites ADD COLUMN poll_method TEXT NOT NULL DEFAULT 'both'",
        [],
    );
    let _ = c.execute(
        "ALTER TABLE sites ADD COLUMN cookie_fields TEXT NOT NULL DEFAULT '[]'",
        [],
    );
    let _ = c.execute(
        "ALTER TABLE sites ADD COLUMN embed_token TEXT NOT NULL DEFAULT ''",
        [],
    );
}

fn migrate_site_edge_sql_pg(c: &mut Client) {
    let _ = c.batch_execute(
        r#"
        ALTER TABLE sites ADD COLUMN IF NOT EXISTS edge_mode TEXT NOT NULL DEFAULT 'first_party';
        ALTER TABLE sites ADD COLUMN IF NOT EXISTS pv_base TEXT NOT NULL DEFAULT '';
        ALTER TABLE sites ADD COLUMN IF NOT EXISTS gv_base TEXT NOT NULL DEFAULT '';
        ALTER TABLE sites ADD COLUMN IF NOT EXISTS fe_load TEXT NOT NULL DEFAULT 'first_party';
        ALTER TABLE sites ADD COLUMN IF NOT EXISTS upload_ingest TEXT NOT NULL DEFAULT 'first_party';
        ALTER TABLE sites ADD COLUMN IF NOT EXISTS poll_method TEXT NOT NULL DEFAULT 'both';
        ALTER TABLE sites ADD COLUMN IF NOT EXISTS cookie_fields TEXT NOT NULL DEFAULT '[]';
        ALTER TABLE sites ADD COLUMN IF NOT EXISTS embed_token TEXT NOT NULL DEFAULT '';
        "#,
    );
}

impl AdminDb {
    pub fn open(path: &Path) -> Result<Self, String> {
        let dsn = gr_abi::env::get("ADMIN_DATABASE_URL")
             .or_else(|| gr_abi::env::get("ADMIN_DATABASE_URL"))
            .unwrap_or_default();
        let dsn = dsn.trim().to_string();
        let data_dir = path
            .parent()
            .unwrap_or(Path::new("run/admin"))
            .to_path_buf();
        std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
        if !dsn.is_empty() {
            let db = Self::open_postgres(&dsn, data_dir)?;
            if let Err(e) = db.maybe_migrate_from_sqlite(path) {
                log::warn!("admin sqlite→pg migrate: {e}");
            }
            Ok(db)
        } else {
            Err(
                "GR_ADMIN_DATABASE_URL (or GR_ADMIN_DATABASE_URL) is required; SQLite admin store was removed"
                    .into(),
            )
        }
    }

    pub fn open_sqlite(path: &Path, data_dir: PathBuf) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        let _ = conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=8000;");
        conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        migrate_site_edge_sql_sqlite(&conn);
        Ok(Self {
            backend: Backend::Sqlite(Mutex::new(conn)),
            data_dir,
            backend_label: path.display().to_string(),
        })
    }

    pub fn open_postgres(dsn: &str, data_dir: PathBuf) -> Result<Self, String> {
        let dsn_owned = dsn.to_string();
        let label = {
            if let Some(at) = dsn.find('@') {
                if let Some(se) = dsn.find("://") {
                    let head = &dsn[..se + 3];
                    let rest = &dsn[se + 3..at];
                    let user = rest.split(':').next().unwrap_or("?");
                    format!("{head}{user}:***{}", &dsn[at..])
                } else {
                    dsn.to_string()
                }
            } else {
                dsn.to_string()
            }
        };
        let (jobs_tx, jobs_rx) = mpsc::sync_channel::<JobFn>(64);
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let label_t = label.clone();
        std::thread::Builder::new()
            .name("gr-admin-pg".into())
            .spawn(move || {
                let mut client = match connect_admin_pg(&dsn_owned) {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                if let Err(e) = client.batch_execute(PG_SCHEMA) {
                    let _ = ready_tx.send(Err(format!("admin pg schema: {e}")));
                    return;
                }
                migrate_site_edge_sql_pg(&mut client);
                let _ = ready_tx.send(Ok(()));
                // iss/audit STO-02 (extended to the admin store — it shares the
                // pooled worker model with the probe/biz/assoc stores): self-heal
                // a broken connection (PG restart / failover) instead of letting
                // every admin job error on a dead socket forever.
                while let Ok(job) = jobs_rx.recv() {
                    if client.is_closed() {
                        let mut delay_ms: u64 = 500;
                        for attempt in 1..=6u32 {
                            match connect_admin_pg(&dsn_owned) {
                                Ok(mut c) => match c.batch_execute(PG_SCHEMA) {
                                    Ok(()) => {
                                        migrate_site_edge_sql_pg(&mut c);
                                        client = c;
                                        log::info!(
                                            "admin pg reconnected after connection loss (attempt {attempt})"
                                        );
                                        break;
                                    }
                                    Err(e) => {
                                        log::warn!(
                                            "admin pg reconnect schema failed attempt {attempt}/6: {e}"
                                        );
                                    }
                                },
                                Err(e) => {
                                    log::warn!("admin pg reconnect failed attempt {attempt}/6: {e}");
                                }
                            }
                            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                            delay_ms = (delay_ms * 2).min(8_000);
                        }
                    }
                    job(&mut client);
                }
            })
            .map_err(|e| format!("admin pg worker thread: {e}"))?;
        ready_rx
            .recv()
            .map_err(|e| format!("admin pg ready: {e}"))?
            .map_err(|e| e)?;
        log::info!("admin config store backend=postgres dsn={label_t} pool=1 long-lived");
        Ok(Self {
            backend: Backend::Postgres { jobs: jobs_tx },
            data_dir,
            backend_label: label,
        })
    }

    pub fn backend_name(&self) -> &'static str {
        match &self.backend {
            Backend::Sqlite(_) => "sqlite",
            Backend::Postgres { .. } => "postgres",
        }
    }

    pub fn backend_label(&self) -> &str {
        &self.backend_label
    }

    fn with_pg<T, F>(&self, f: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(&mut Client) -> Result<T, String> + Send + 'static,
    {
        match &self.backend {
            Backend::Postgres { jobs } => {
                let (tx, rx) = mpsc::channel();
                jobs
                    .send(Box::new(move |client| {
                        let _ = tx.send(f(client));
                    }))
                    .map_err(|e| format!("admin pg job send: {e}"))?;
                rx.recv()
                    .map_err(|e| format!("admin pg job recv: {e}"))?
            }
            Backend::Sqlite(_) => Err("not_postgres".into()),
        }
    }

    fn is_pg(&self) -> bool {
        matches!(self.backend, Backend::Postgres { .. })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, String> {
        match &self.backend {
            Backend::Sqlite(m) => m.lock().map_err(|e| e.to_string()),
            Backend::Postgres { .. } => Err("not_sqlite".into()),
        }
    }

    fn maybe_migrate_from_sqlite(&self, sqlite_path: &Path) -> Result<(), String> {
        if !self.is_pg() || !sqlite_path.is_file() {
            return Ok(());
        }
        if !self.list_sites(None)?.is_empty() {
            return Ok(());
        }
        let src = Connection::open(sqlite_path).map_err(|e| e.to_string())?;
        let n: i64 = src
            .query_row("SELECT count(*) FROM sites", [], |r| r.get(0))
            .unwrap_or(0);
        if n <= 0 {
            return Ok(());
        }
        log::warn!("admin migrate: {} sites from sqlite → pg", n);
        // sites
        let mut st = src
            .prepare("SELECT site_id, name, collect_enabled, notes, created_ms, updated_ms FROM sites")
            .map_err(|e| e.to_string())?;
        let sites: Vec<(String, String, i64, String, i64, i64)> = st
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        for (id, name, en, notes, cr, up) in sites {
            let _ = self.upsert_site(Some(&id), &name, en != 0, &notes);
            let _ = (cr, up);
        }
        let mut st = src
            .prepare(
                "SELECT domain_id, site_id, hostname, display_name, collect_enabled FROM domains",
            )
            .map_err(|e| e.to_string())?;
        let doms: Vec<(String, String, String, String, i64)> = st
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        for (id, sid, host, disp, en) in doms {
            let _ = self.upsert_domain(Some(&id), &sid, &host, &disp, en != 0);
        }
        // users
        let mut st = src
            .prepare("SELECT username, password_hash FROM admin_users")
            .map_err(|e| e.to_string())?;
        let users: Vec<(String, String)> = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        for (u, h) in users {
            let _ = self.ensure_user(&u, &h);
        }
        // settings
        let mut st = src
            .prepare("SELECT key, value FROM admin_settings")
            .map_err(|e| e.to_string())?;
        let sets: Vec<(String, String)> = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        for (k, v) in sets {
            let _ = self.set_setting(&k, &v);
        }
        log::warn!("admin migrate done sites={}", self.list_sites(None)?.len());
        Ok(())
    }

    pub fn audit(&self, actor: &str, action: &str, target: &str, detail: Value) {
        if self.is_pg() {
            let actor = actor.to_string();
            let action = action.to_string();
            let target = target.to_string();
            let detail_s = detail.to_string();
            let ms = now_ms();
            let _ = self.with_pg(move |c| {
                let _ = c.execute(
                    "INSERT INTO audit_log(actor, action, target, detail_json, ms) VALUES ($1,$2,$3,$4,$5)",
                    &[&actor, &action, &target, &detail_s, &ms],
                );
                Ok(())
            });
            return;
        }
        let Ok(c) = self.lock() else { return };
        let _ = c.execute(
            "INSERT INTO audit_log(actor, action, target, detail_json, ms) VALUES (?1,?2,?3,?4,?5)",
            params![actor, action, target, detail.to_string(), now_ms()],
        );
    }

    pub fn list_audit(&self, limit: usize) -> Result<Vec<Value>, String> {
        if self.is_pg() {
            let lim = limit as i64;
            return self.with_pg(move |c| {
                let rows = c.query("SELECT id, actor, action, target, detail_json, ms FROM audit_log ORDER BY id DESC LIMIT $1", &[&lim]).map_err(|e| e.to_string())?;
                Ok(rows.iter().map(|r| {
                    let d: String = r.get(4);
                    json!({"id": r.get::<_, i64>(0), "actor": r.get::<_, String>(1), "action": r.get::<_, String>(2), "target": r.get::<_, String>(3),
                        "detail": serde_json::from_str::<Value>(&d).unwrap_or(json!({})), "ms": r.get::<_, i64>(5)})
                }).collect())
            });
        }
        let c = self.lock()?;
        let mut stmt = c
            .prepare(
                "SELECT id, actor, action, target, detail_json, ms FROM audit_log ORDER BY id DESC LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![limit as i64], |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?,
                    "actor": r.get::<_, String>(1)?,
                    "action": r.get::<_, String>(2)?,
                    "target": r.get::<_, String>(3)?,
                    "detail": serde_json::from_str::<Value>(&r.get::<_, String>(4)?).unwrap_or(json!({})),
                    "ms": r.get::<_, i64>(5)?,
                }))
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, String> {
        if self.is_pg() {
            let key = key.to_string();
            return self.with_pg(move |c| {
                let rows = c.query("SELECT value FROM admin_settings WHERE key=$1", &[&key]).map_err(|e| e.to_string())?;
                Ok(rows.first().map(|r| r.get(0)))
            });
        }
        let c = self.lock()?;
        c.query_row(
            "SELECT value FROM admin_settings WHERE key=?1",
            params![key],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), String> {
        if self.is_pg() {
            let key = key.to_string();
            let value = value.to_string();
            return self.with_pg(move |c| {
                c.execute(
                    "INSERT INTO admin_settings(key, value) VALUES ($1,$2) ON CONFLICT(key) DO UPDATE SET value=EXCLUDED.value",
                    &[&key, &value],
                ).map_err(|e| e.to_string())?;
                Ok(())
            });
        }
        let c = self.lock()?;
        c.execute(
            "INSERT INTO admin_settings(key, value) VALUES (?1,?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn ensure_user(&self, username: &str, password_hash: &str) -> Result<bool, String> {
        if self.is_pg() {
            let username = username.to_string();
            let password_hash = password_hash.to_string();
            let now = now_ms();
            return self.with_pg(move |c| {
                let n = c.execute(
                    "INSERT INTO admin_users(username, password_hash, created_ms) VALUES ($1,$2,$3) ON CONFLICT(username) DO NOTHING",
                    &[&username, &password_hash, &now],
                ).map_err(|e| e.to_string())?;
                Ok(n > 0)
            });
        }
        let c = self.lock()?;
        // INSERT OR IGNORE: multi-worker cold start can race on the same admin SQLite.
        let n = c
            .execute(
                "INSERT OR IGNORE INTO admin_users(username, password_hash, created_ms) VALUES (?1,?2,?3)",
                params![username, password_hash, now_ms()],
            )
            .map_err(|e| e.to_string())?;
        Ok(n > 0)
    }

    pub fn get_user_hash(&self, username: &str) -> Result<Option<String>, String> {
        if self.is_pg() {
            let username = username.to_string();
            return self.with_pg(move |c| {
                let rows = c.query("SELECT password_hash FROM admin_users WHERE username=$1", &[&username]).map_err(|e| e.to_string())?;
                Ok(rows.first().map(|r| r.get(0)))
            });
        }
        let c = self.lock()?;
        c.query_row(
            "SELECT password_hash FROM admin_users WHERE username=?1",
            params![username],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())
    }

    pub fn create_session(&self, username: &str, ttl_ms: i64) -> Result<String, String> {
        if self.is_pg() {
            use rand::RngCore;
            let mut b = [0u8; 24];
            rand::thread_rng().fill_bytes(&mut b);
            let token = format!("gra_{}", hex::encode(b));
            let now = now_ms();
            let exp = now + ttl_ms;
            let username = username.to_string();
            let token2 = token.clone();
            self.with_pg(move |c| {
                c.execute(
                    "INSERT INTO admin_sessions(token, username, exp_ms, created_ms) VALUES ($1,$2,$3,$4)",
                    &[&token2, &username, &exp, &now],
                ).map_err(|e| e.to_string())?;
                Ok(())
            })?;
            return Ok(token);
        }
        use rand::RngCore;
        let mut b = [0u8; 24];
        rand::thread_rng().fill_bytes(&mut b);
        let token = format!("gra_{}", hex::encode(b));
        let now = now_ms();
        let c = self.lock()?;
        c.execute(
            "INSERT INTO admin_sessions(token, username, exp_ms, created_ms) VALUES (?1,?2,?3,?4)",
            params![token, username, now + ttl_ms, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(token)
    }

    pub fn session_user(&self, token: &str) -> Result<Option<String>, String> {
        if self.is_pg() {
            let token = token.to_string();
            let now = now_ms();
            return self.with_pg(move |c| {
                let rows = c.query("SELECT username, exp_ms FROM admin_sessions WHERE token=$1", &[&token]).map_err(|e| e.to_string())?;
                if let Some(r) = rows.first() {
                    let u: String = r.get(0);
                    let exp: i64 = r.get(1);
                    if exp >= now {
                        Ok(Some(u))
                    } else {
                        let _ = c.execute("DELETE FROM admin_sessions WHERE token=$1", &[&token]);
                        Ok(None)
                    }
                } else {
                    Ok(None)
                }
            });
        }
        let now = now_ms();
        let c = self.lock()?;
        let row: Option<(String, i64)> = c
            .query_row(
                "SELECT username, exp_ms FROM admin_sessions WHERE token=?1",
                params![token],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        match row {
            Some((u, exp)) if exp >= now => Ok(Some(u)),
            Some(_) => {
                let _ = c.execute("DELETE FROM admin_sessions WHERE token=?1", params![token]);
                Ok(None)
            }
            None => Ok(None),
        }
    }

    pub fn delete_session(&self, token: &str) -> Result<(), String> {
        if self.is_pg() {
            let token = token.to_string();
            return self.with_pg(move |c| {
                c.execute("DELETE FROM admin_sessions WHERE token=$1", &[&token]).map_err(|e| e.to_string())?;
                Ok(())
            });
        }
        let c = self.lock()?;
        c.execute("DELETE FROM admin_sessions WHERE token=?1", params![token])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn upsert_site(
        &self,
        site_id: Option<&str>,
        name: &str,
        collect_enabled: bool,
        notes: &str,
    ) -> Result<Value, String> {
        self.upsert_site_full(site_id, name, collect_enabled, notes, None, None, None, None)
    }

    /// Upsert site including optional edge topology (`edge_mode` / `pv_base` / `gv_base`)
    /// and optional cookie allowlist (`cookie_fields`).
    /// When a field is `None`, the existing value is preserved on update (defaults on insert).
    pub fn upsert_site_full(
        &self,
        site_id: Option<&str>,
        name: &str,
        collect_enabled: bool,
        notes: &str,
        edge_mode: Option<&str>,
        pv_base: Option<&str>,
        gv_base: Option<&str>,
        cookie_fields: Option<&[String]>,
    ) -> Result<Value, String> {
        let now = now_ms();
        let id = site_id
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| new_id("site"));
        let mode = edge_mode
            .map(normalize_edge_mode)
            .unwrap_or_else(|| "first_party".into());
        let pv = pv_base.map(trim_base).unwrap_or_default();
        let gv = gv_base.map(trim_base).unwrap_or_default();
        let touch_edge = edge_mode.is_some() || pv_base.is_some() || gv_base.is_some();
        let cookies = cookie_fields.map(|c| serde_json::to_string(c).unwrap_or_else(|_| "[]".to_string()));

        if self.is_pg() {
            let name = name.to_string();
            let notes = notes.to_string();
            let en = collect_enabled as i32;
            let id2 = id.clone();
            let mode2 = mode.clone();
            let pv2 = pv.clone();
            let gv2 = gv.clone();
            self.with_pg(move |c| {
                match (touch_edge, cookies.as_deref()) {
                    (true, Some(cs)) => {
                        c.execute(
                            "INSERT INTO sites(site_id, name, collect_enabled, notes, edge_mode, pv_base, gv_base, cookie_fields, created_ms, updated_ms)
                             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$9)
                             ON CONFLICT(site_id) DO UPDATE SET
                               name=EXCLUDED.name,
                               collect_enabled=EXCLUDED.collect_enabled,
                               notes=EXCLUDED.notes,
                               edge_mode=EXCLUDED.edge_mode,
                               pv_base=EXCLUDED.pv_base,
                               gv_base=EXCLUDED.gv_base,
                               cookie_fields=EXCLUDED.cookie_fields,
                               updated_ms=EXCLUDED.updated_ms",
                            &[&id2, &name, &en, &notes, &mode2, &pv2, &gv2, &cs, &now],
                        )
                        .map_err(|e| e.to_string())?;
                    }
                    (true, None) => {
                        c.execute(
                            "INSERT INTO sites(site_id, name, collect_enabled, notes, edge_mode, pv_base, gv_base, created_ms, updated_ms)
                             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$8)
                             ON CONFLICT(site_id) DO UPDATE SET
                               name=EXCLUDED.name,
                               collect_enabled=EXCLUDED.collect_enabled,
                               notes=EXCLUDED.notes,
                               edge_mode=EXCLUDED.edge_mode,
                               pv_base=EXCLUDED.pv_base,
                               gv_base=EXCLUDED.gv_base,
                               updated_ms=EXCLUDED.updated_ms",
                            &[&id2, &name, &en, &notes, &mode2, &pv2, &gv2, &now],
                        )
                        .map_err(|e| e.to_string())?;
                    }
                    (false, Some(cs)) => {
                        c.execute(
                            "INSERT INTO sites(site_id, name, collect_enabled, notes, edge_mode, pv_base, gv_base, cookie_fields, created_ms, updated_ms)
                             VALUES ($1,$2,$3,$4,'first_party','','',$5,$6,$6)
                             ON CONFLICT(site_id) DO UPDATE SET
                               name=EXCLUDED.name,
                               collect_enabled=EXCLUDED.collect_enabled,
                               notes=EXCLUDED.notes,
                               cookie_fields=EXCLUDED.cookie_fields,
                               updated_ms=EXCLUDED.updated_ms",
                            &[&id2, &name, &en, &notes, &cs, &now],
                        )
                        .map_err(|e| e.to_string())?;
                    }
                    (false, None) => {
                        c.execute(
                            "INSERT INTO sites(site_id, name, collect_enabled, notes, edge_mode, pv_base, gv_base, created_ms, updated_ms)
                             VALUES ($1,$2,$3,$4,'first_party','','',$5,$5)
                             ON CONFLICT(site_id) DO UPDATE SET
                               name=EXCLUDED.name,
                               collect_enabled=EXCLUDED.collect_enabled,
                               notes=EXCLUDED.notes,
                               updated_ms=EXCLUDED.updated_ms",
                            &[&id2, &name, &en, &notes, &now],
                        )
                        .map_err(|e| e.to_string())?;
                    }
                }
                Ok(())
            })?;
            return self
                .get_site(&id)?
                .ok_or_else(|| "site_missing_after_upsert".to_string());
        }

        let c = self.lock()?;
        if let Some(cs) = cookies.as_deref() {
            c.execute(
                "INSERT INTO sites(site_id, name, collect_enabled, notes, edge_mode, pv_base, gv_base, cookie_fields, created_ms, updated_ms)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?9)
                 ON CONFLICT(site_id) DO UPDATE SET
                   name=excluded.name,
                   collect_enabled=excluded.collect_enabled,
                   notes=excluded.notes,
                   edge_mode=excluded.edge_mode,
                   pv_base=excluded.pv_base,
                   gv_base=excluded.gv_base,
                   cookie_fields=excluded.cookie_fields,
                   updated_ms=excluded.updated_ms",
                params![
                    id,
                    name,
                    collect_enabled as i64,
                    notes,
                    mode,
                    pv,
                    gv,
                    cs,
                    now
                ],
            )
            .map_err(|e| e.to_string())?;
        } else if touch_edge {
            c.execute(
                "INSERT INTO sites(site_id, name, collect_enabled, notes, edge_mode, pv_base, gv_base, created_ms, updated_ms)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?8)
                 ON CONFLICT(site_id) DO UPDATE SET
                   name=excluded.name,
                   collect_enabled=excluded.collect_enabled,
                   notes=excluded.notes,
                   edge_mode=excluded.edge_mode,
                   pv_base=excluded.pv_base,
                   gv_base=excluded.gv_base,
                   updated_ms=excluded.updated_ms",
                params![id, name, collect_enabled as i64, notes, mode, pv, gv, now],
            )
            .map_err(|e| e.to_string())?;
        } else {
            c.execute(
                "INSERT INTO sites(site_id, name, collect_enabled, notes, edge_mode, pv_base, gv_base, created_ms, updated_ms)
                 VALUES (?1,?2,?3,?4,'first_party','','',?5,?5)
                 ON CONFLICT(site_id) DO UPDATE SET
                   name=excluded.name,
                   collect_enabled=excluded.collect_enabled,
                   notes=excluded.notes,
                   updated_ms=excluded.updated_ms",
                params![id, name, collect_enabled as i64, notes, now],
            )
            .map_err(|e| e.to_string())?;
        }
        drop(c);
        self.get_site(&id)?
            .ok_or_else(|| "site_missing_after_upsert".to_string())
    }

    /// Update deploy topology for a site.
    ///
    /// Product rules:
    /// - **gv_base** always required as absolute `https://gv…` (Pingora TLS B8 depth)
    /// - **fe_load**: `first_party` | `pv` (how statistics JS is loaded)
    /// - **upload_ingest**: `first_party` | `pv` (where open/ingest goes; B8 always also uses gv)
    /// - **poll_method**: `get` | `post` | `both` for /analyses|/health style polls
    pub fn set_site_edge(
        &self,
        site_id: &str,
        edge_mode: &str,
        pv_base: &str,
        gv_base: &str,
    ) -> Result<Value, String> {
        self.set_site_deploy(
            site_id,
            None,
            None,
            None,
            Some(edge_mode),
            Some(pv_base),
            Some(gv_base),
        )
    }

    pub fn set_site_deploy(
        &self,
        site_id: &str,
        fe_load: Option<&str>,
        upload_ingest: Option<&str>,
        poll_method: Option<&str>,
        edge_mode: Option<&str>,
        pv_base: Option<&str>,
        gv_base: Option<&str>,
    ) -> Result<Value, String> {
        let cur = self
            .get_site(site_id)?
            .ok_or_else(|| "site_not_found".to_string())?;
        let fe = normalize_fe_load(
            fe_load.unwrap_or_else(|| cur.get("fe_load").and_then(|v| v.as_str()).unwrap_or("first_party")),
        );
        let up = normalize_upload_ingest(
            upload_ingest.unwrap_or_else(|| {
                cur.get("upload_ingest")
                    .and_then(|v| v.as_str())
                    .unwrap_or("first_party")
            }),
        );
        let poll = normalize_poll_method(
            poll_method.unwrap_or_else(|| {
                cur.get("poll_method")
                    .and_then(|v| v.as_str())
                    .unwrap_or("both")
            }),
        );
        let pv = trim_base(
            pv_base.unwrap_or_else(|| cur.get("pv_base").and_then(|v| v.as_str()).unwrap_or("")),
        );
        let gv = trim_base(
            gv_base.unwrap_or_else(|| cur.get("gv_base").and_then(|v| v.as_str()).unwrap_or("")),
        );

        if up == "first_party" && !gr_abi::env::flag("ALLOW_FIRST_PARTY_UPLOAD") {
            return Err("upload_ingest_first_party_forbidden".into());
        }
        // gv is ALWAYS required: Pingora gateway TLS collects B8 side-channel material.
        if !is_absolute_https(&gv) {
            return Err(
                "gv_base required as absolute HTTPS (https://gv.example.com) for ALL upload+B8 via Pingora TLS direct (no /g5-gw backup)"
                    .into(),
            );
        }
        if fe == "pv" || up == "pv" {
            if pv.is_empty()
                || !(pv.starts_with("https://") || pv.starts_with("http://"))
            {
                return Err(
                    "pv_base absolute URL required when fe_load=pv or upload_ingest=pv".into(),
                );
            }
        }
        // Default: fe first_party load + upload/b8 on gv → dual_domain style edge
        let mode = if let Some(em) = edge_mode.filter(|s| !s.trim().is_empty()) {
            normalize_edge_mode(em)
        } else if up == "gv" || fe == "gv" {
            "dual_domain".into()
        } else if fe == "pv" || up == "pv" {
            "dual_domain".into()
        } else {
            // first_party load+upload both on /g5 (legacy)
            "first_party".into()
        };

        let now = now_ms();
        if self.is_pg() {
            let site_id = site_id.to_string();
            let mode2 = mode.clone();
            let pv2 = pv.clone();
            let gv2 = gv.clone();
            let fe2 = fe.clone();
            let up2 = up.clone();
            let poll2 = poll.clone();
            let n = self.with_pg(move |c| {
                c.execute(
                    "UPDATE sites SET edge_mode=$1, pv_base=$2, gv_base=$3, fe_load=$4, upload_ingest=$5, poll_method=$6, updated_ms=$7 WHERE site_id=$8",
                    &[&mode2, &pv2, &gv2, &fe2, &up2, &poll2, &now, &site_id],
                )
                .map_err(|e| e.to_string())
            })?;
            if n == 0 {
                return Err("site_not_found".into());
            }
        } else {
            let c = self.lock()?;
            let n = c
                .execute(
                    "UPDATE sites SET edge_mode=?1, pv_base=?2, gv_base=?3, fe_load=?4, upload_ingest=?5, poll_method=?6, updated_ms=?7 WHERE site_id=?8",
                    params![mode, pv, gv, fe, up, poll, now, site_id],
                )
                .map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("site_not_found".into());
            }
        }
        // Auto-register pv/gv hostnames so Host→site_id works.
        if let Some(h) = hostname_from_base(&pv) {
            let _ = self.upsert_domain(None, site_id, &h, "pv", true);
        }
        if let Some(h) = hostname_from_base(&gv) {
            let _ = self.upsert_domain(None, site_id, &h, "gv", true);
        }
        self.get_site(site_id)?
            .ok_or_else(|| "site_missing_after_edge".to_string())
    }

    pub fn list_sites(&self, q: Option<&str>) -> Result<Vec<Value>, String> {
        const COLS: &str = "site_id, name, collect_enabled, notes, COALESCE(edge_mode,'first_party'), COALESCE(pv_base,''), COALESCE(gv_base,''), COALESCE(fe_load,'pv'), COALESCE(upload_ingest,'gv'), COALESCE(poll_method,'both'), COALESCE(cookie_fields,'[]'), COALESCE(embed_token,''), created_ms, updated_ms";
        if self.is_pg() {
            let q = q.map(|s| s.to_string());
            return self.with_pg(move |c| {
                let rows = if let Some(ref q) = q {
                    let like = format!("%{q}%");
                    c.query(
                        &format!("SELECT {COLS} FROM sites WHERE name ILIKE $1 OR site_id ILIKE $1 ORDER BY updated_ms DESC"),
                        &[&like],
                    )
                } else {
                    c.query(
                        &format!("SELECT {COLS} FROM sites ORDER BY updated_ms DESC"),
                        &[],
                    )
                }
                .map_err(|e| e.to_string())?;
                Ok(rows
                    .iter()
                    .map(|r| {
                        site_json(
                            r.get::<_, String>(0),
                            r.get::<_, String>(1),
                            r.get::<_, i32>(2) != 0,
                            r.get::<_, String>(3),
                            r.get::<_, String>(4),
                            r.get::<_, String>(5),
                            r.get::<_, String>(6),
                            r.get::<_, String>(7),
                            r.get::<_, String>(8),
                            r.get::<_, String>(9),
                            r.get::<_, String>(10),
                            r.get::<_, String>(11),
                            r.get::<_, i64>(12),
                            r.get::<_, i64>(13),
                        )
                    })
                    .collect())
            });
        }
        let c = self.lock()?;
        let mut out = Vec::new();
        if let Some(q) = q.filter(|s| !s.is_empty()) {
            let like = format!("%{q}%");
            let mut stmt = c
                .prepare(&format!(
                    "SELECT {COLS} FROM sites WHERE name LIKE ?1 OR site_id LIKE ?1 ORDER BY updated_ms DESC"
                ))
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params![like], |r| {
                    Ok(site_json(
                        r.get(0)?,
                        r.get(1)?,
                        r.get::<_, i64>(2)? != 0,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                        r.get(9)?,
                        r.get(10)?,
                        r.get(11)?,
                        r.get(12)?,
                        r.get(13)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            for row in rows {
                out.push(row.map_err(|e| e.to_string())?);
            }
        } else {
            let mut stmt = c
                .prepare(&format!("SELECT {COLS} FROM sites ORDER BY updated_ms DESC"))
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(site_json(
                        r.get(0)?,
                        r.get(1)?,
                        r.get::<_, i64>(2)? != 0,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                        r.get(9)?,
                        r.get(10)?,
                        r.get(11)?,
                        r.get(12)?,
                        r.get(13)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            for row in rows {
                out.push(row.map_err(|e| e.to_string())?);
            }
        }
        Ok(out)
    }

    pub fn get_site(&self, site_id: &str) -> Result<Option<Value>, String> {
        const COLS: &str = "site_id, name, collect_enabled, notes, COALESCE(edge_mode,'first_party'), COALESCE(pv_base,''), COALESCE(gv_base,''), COALESCE(fe_load,'pv'), COALESCE(upload_ingest,'gv'), COALESCE(poll_method,'both'), COALESCE(cookie_fields,'[]'), COALESCE(embed_token,''), created_ms, updated_ms";
        if self.is_pg() {
            let site_id = site_id.to_string();
            return self.with_pg(move |c| {
                let rows = c
                    .query(
                        &format!("SELECT {COLS} FROM sites WHERE site_id=$1"),
                        &[&site_id],
                    )
                    .map_err(|e| e.to_string())?;
                Ok(rows.first().map(|r| {
                    site_json(
                        r.get::<_, String>(0),
                        r.get::<_, String>(1),
                        r.get::<_, i32>(2) != 0,
                        r.get::<_, String>(3),
                        r.get::<_, String>(4),
                        r.get::<_, String>(5),
                        r.get::<_, String>(6),
                        r.get::<_, String>(7),
                        r.get::<_, String>(8),
                        r.get::<_, String>(9),
                        r.get::<_, String>(10),
                        r.get::<_, String>(11),
                        r.get::<_, i64>(12),
                        r.get::<_, i64>(13),
                    )
                }))
            });
        }
        let c = self.lock()?;
        c.query_row(
            &format!("SELECT {COLS} FROM sites WHERE site_id=?1"),
            params![site_id],
            |r| {
                Ok(site_json(
                    r.get(0)?,
                    r.get(1)?,
                    r.get::<_, i64>(2)? != 0,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                    r.get(11)?,
                    r.get(12)?,
                    r.get(13)?,
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())
    }

    /// Token-first lookup for `/gr.js?grt=`. Empty token never matches.
    pub fn find_site_by_embed_token(&self, token: &str) -> Result<Option<Value>, String> {
        let token = token.trim().to_string();
        if token.is_empty() {
            return Ok(None);
        }
        const COLS: &str = "site_id, name, collect_enabled, notes, COALESCE(edge_mode,'first_party'), COALESCE(pv_base,''), COALESCE(gv_base,''), COALESCE(fe_load,'pv'), COALESCE(upload_ingest,'gv'), COALESCE(poll_method,'both'), COALESCE(cookie_fields,'[]'), COALESCE(embed_token,''), created_ms, updated_ms";
        if self.is_pg() {
            let t = token.clone();
            return self.with_pg(move |c| {
                let rows = c
                    .query(
                        &format!("SELECT {COLS} FROM sites WHERE embed_token=$1 LIMIT 1"),
                        &[&t],
                    )
                    .map_err(|e| e.to_string())?;
                Ok(rows.first().map(|r| {
                    site_json(
                        r.get::<_, String>(0),
                        r.get::<_, String>(1),
                        r.get::<_, i32>(2) != 0,
                        r.get::<_, String>(3),
                        r.get::<_, String>(4),
                        r.get::<_, String>(5),
                        r.get::<_, String>(6),
                        r.get::<_, String>(7),
                        r.get::<_, String>(8),
                        r.get::<_, String>(9),
                        r.get::<_, String>(10),
                        r.get::<_, String>(11),
                        r.get::<_, i64>(12),
                        r.get::<_, i64>(13),
                    )
                }))
            });
        }
        let c = self.lock()?;
        c.query_row(
            &format!("SELECT {COLS} FROM sites WHERE embed_token=?1 LIMIT 1"),
            params![token],
            |r| {
                Ok(site_json(
                    r.get(0)?,
                    r.get(1)?,
                    r.get::<_, i64>(2)? != 0,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                    r.get(11)?,
                    r.get(12)?,
                    r.get(13)?,
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())
    }

    pub fn set_embed_token(&self, site_id: &str, token: &str) -> Result<(), String> {
        let site_id = site_id.to_string();
        let token = token.to_string();
        let now = now_ms();
        if self.is_pg() {
            let n = self.with_pg(move |c| {
                c.execute(
                    "UPDATE sites SET embed_token=$1, updated_ms=$2 WHERE site_id=$3",
                    &[&token, &now, &site_id],
                )
                .map_err(|e| e.to_string())
            })?;
            if n == 0 {
                return Err("site_not_found".into());
            }
            return Ok(());
        }
        let c = self.lock()?;
        let n = c
            .execute(
                "UPDATE sites SET embed_token=?1, updated_ms=?2 WHERE site_id=?3",
                params![token, now, site_id],
            )
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("site_not_found".into());
        }
        Ok(())
    }

    pub fn set_site_collect(&self, site_id: &str, enabled: bool) -> Result<(), String> {
        if self.is_pg() {
            let site_id = site_id.to_string();
            let now = now_ms();
            let en = enabled as i32;
            return self.with_pg(move |c| {
                let n = c.execute("UPDATE sites SET collect_enabled=$1, updated_ms=$2 WHERE site_id=$3", &[&en, &now, &site_id]).map_err(|e| e.to_string())?;
                if n == 0 { Err("site_not_found".into()) } else { Ok(()) }
            });
        }
        let c = self.lock()?;
        let n = c
            .execute(
                "UPDATE sites SET collect_enabled=?1, updated_ms=?2 WHERE site_id=?3",
                params![enabled as i64, now_ms(), site_id],
            )
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("site_not_found".into());
        }
        Ok(())
    }

    pub fn upsert_domain(
        &self,
        domain_id: Option<&str>,
        site_id: &str,
        hostname: &str,
        display_name: &str,
        collect_enabled: bool,
    ) -> Result<Value, String> {
        if self.is_pg() {
            let host = hostname.trim().to_ascii_lowercase();
            if host.is_empty() { return Err("hostname_required".into()); }
            if site_id.trim().is_empty() { return Err("site_id_required".into()); }
            let now = now_ms();
            let id = domain_id.filter(|s| !s.is_empty()).map(|s| s.to_string()).unwrap_or_else(|| new_id("dom"));
            let site_id = site_id.to_string();
            let display_name = display_name.to_string();
            let en = collect_enabled as i32;
            let id2 = id.clone();
            let site2 = site_id.clone();
            let host2 = host.clone();
            let disp2 = display_name.clone();
            let row = self.with_pg(move |c| {
                let owned = c.query("SELECT domain_id, site_id FROM domains WHERE hostname=$1", &[&host2]).map_err(|e| e.to_string())?;
                if let Some(r) = owned.first() {
                    let exist_id: String = r.get(0);
                    let exist_site: String = r.get(1);
                    if exist_site != site2 && exist_id != id2 {
                        return Err(format!("hostname_owned_by_other_site:{exist_site} (multi-tenant isolation)"));
                    }
                    if exist_id != id2 && exist_site == site2 {
                        c.execute("UPDATE domains SET site_id=$1, display_name=$2, collect_enabled=$3, updated_ms=$4 WHERE domain_id=$5",
                            &[&site2, &disp2, &en, &now, &exist_id]).map_err(|e| e.to_string())?;
                        let rows = c.query("SELECT domain_id, site_id, hostname, display_name, collect_enabled, ssl_status, ssl_expires_ms, cert_path, key_path, created_ms, updated_ms FROM domains WHERE domain_id=$1", &[&exist_id]).map_err(|e| e.to_string())?;
                        let r = rows.first().ok_or_else(|| "domain_missing".to_string())?;
                        return Ok(json!({
                            "domain_id": r.get::<_, String>(0), "site_id": r.get::<_, String>(1), "hostname": r.get::<_, String>(2),
                            "display_name": r.get::<_, String>(3), "collect_enabled": r.get::<_, i32>(4) != 0,
                            "ssl_status": r.get::<_, String>(5), "ssl_expires_ms": r.get::<_, Option<i64>>(6),
                            "cert_path": r.get::<_, Option<String>>(7), "key_path": r.get::<_, Option<String>>(8),
                            "created_ms": r.get::<_, i64>(9), "updated_ms": r.get::<_, i64>(10),
                        }));
                    }
                }
                c.execute(
                    "INSERT INTO domains(domain_id, site_id, hostname, display_name, collect_enabled, created_ms, updated_ms)
                     VALUES ($1,$2,$3,$4,$5,$6,$6)
                     ON CONFLICT(domain_id) DO UPDATE SET site_id=EXCLUDED.site_id, hostname=EXCLUDED.hostname, display_name=EXCLUDED.display_name, collect_enabled=EXCLUDED.collect_enabled, updated_ms=EXCLUDED.updated_ms",
                    &[&id2, &site2, &host2, &disp2, &en, &now],
                ).map_err(|e| {
                    let s = e.to_string();
                    if s.contains("unique") || s.contains("UNIQUE") { format!("hostname_conflict:{host2} (must be unique across sites)") } else { s }
                })?;
                let rows = c.query("SELECT domain_id, site_id, hostname, display_name, collect_enabled, ssl_status, ssl_expires_ms, cert_path, key_path, created_ms, updated_ms FROM domains WHERE domain_id=$1", &[&id2]).map_err(|e| e.to_string())?;
                let r = rows.first().ok_or_else(|| "domain_missing".to_string())?;
                Ok(json!({
                    "domain_id": r.get::<_, String>(0), "site_id": r.get::<_, String>(1), "hostname": r.get::<_, String>(2),
                    "display_name": r.get::<_, String>(3), "collect_enabled": r.get::<_, i32>(4) != 0,
                    "ssl_status": r.get::<_, String>(5), "ssl_expires_ms": r.get::<_, Option<i64>>(6),
                    "cert_path": r.get::<_, Option<String>>(7), "key_path": r.get::<_, Option<String>>(8),
                    "created_ms": r.get::<_, i64>(9), "updated_ms": r.get::<_, i64>(10),
                }))
            })?;
            return Ok(row);
        }
        let host = hostname.trim().to_ascii_lowercase();
        if host.is_empty() {
            return Err("hostname_required".into());
        }
        if site_id.trim().is_empty() {
            return Err("site_id_required".into());
        }
        let now = now_ms();
        let id = domain_id
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| new_id("dom"));
        let c = self.lock()?;
        // Multi-tenant isolation: hostname is globally unique → one domain maps to one site.
        // Check under same lock (Mutex is not re-entrant).
        let owned: Option<(String, String)> = c
            .query_row(
                "SELECT domain_id, site_id FROM domains WHERE hostname=?1",
                params![host],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some((exist_id, exist_site)) = owned {
            if exist_site != site_id && exist_id != id {
                return Err(format!(
                    "hostname_owned_by_other_site:{exist_site} (multi-tenant isolation)"
                ));
            }
            // Same site + same hostname under different domain_id: reuse existing row.
            if exist_id != id && exist_site == site_id {
                // Fall through with exist_id so we update instead of UNIQUE fail.
                let id = exist_id;
                c.execute(
                    "UPDATE domains SET site_id=?1, display_name=?2, collect_enabled=?3, updated_ms=?4
                     WHERE domain_id=?5",
                    params![site_id, display_name, collect_enabled as i64, now, id],
                )
                .map_err(|e| e.to_string())?;
                return self
                    .domain_row_unlocked(&c, &id)?
                    .ok_or_else(|| "domain_missing_after_upsert".to_string());
            }
        }
        c.execute(
            "INSERT INTO domains(domain_id, site_id, hostname, display_name, collect_enabled, created_ms, updated_ms)
             VALUES (?1,?2,?3,?4,?5,?6,?6)
             ON CONFLICT(domain_id) DO UPDATE SET
               site_id=excluded.site_id,
               hostname=excluded.hostname,
               display_name=excluded.display_name,
               collect_enabled=excluded.collect_enabled,
               updated_ms=excluded.updated_ms",
            params![id, site_id, host, display_name, collect_enabled as i64, now],
        )
        .map_err(|e| {
            let s = e.to_string();
            if s.contains("UNIQUE") || s.contains("unique") {
                format!("hostname_conflict:{host} (must be unique across sites)")
            } else {
                s
            }
        })?;
        Ok(self
            .domain_row_unlocked(&c, &id)?
            .ok_or_else(|| "domain_missing_after_upsert".to_string())?)
    }

    fn domain_row_unlocked(
        &self,
        c: &Connection,
        domain_id: &str,
    ) -> Result<Option<Value>, String> {
        c.query_row(
            "SELECT domain_id, site_id, hostname, display_name, collect_enabled,
                    ssl_status, ssl_expires_ms, cert_path, key_path, created_ms, updated_ms
             FROM domains WHERE domain_id=?1",
            params![domain_id],
            |r| {
                Ok(json!({
                    "domain_id": r.get::<_, String>(0)?,
                    "site_id": r.get::<_, String>(1)?,
                    "hostname": r.get::<_, String>(2)?,
                    "display_name": r.get::<_, String>(3)?,
                    "collect_enabled": r.get::<_, i64>(4)? != 0,
                    "ssl_status": r.get::<_, String>(5)?,
                    "ssl_expires_ms": r.get::<_, Option<i64>>(6)?,
                    "cert_path": r.get::<_, Option<String>>(7)?,
                    "key_path": r.get::<_, Option<String>>(8)?,
                    "created_ms": r.get::<_, i64>(9)?,
                    "updated_ms": r.get::<_, i64>(10)?,
                }))
            },
        )
        .optional()
        .map_err(|e| e.to_string())
    }

    pub fn get_domain(&self, domain_id: &str) -> Result<Option<Value>, String> {
        if self.is_pg() {
            let domain_id = domain_id.to_string();
            return self.with_pg(move |c| {
                let rows = c.query(
                    "SELECT domain_id, site_id, hostname, display_name, collect_enabled, ssl_status, ssl_expires_ms, cert_path, key_path, created_ms, updated_ms FROM domains WHERE domain_id=$1",
                    &[&domain_id],
                ).map_err(|e| e.to_string())?;
                Ok(rows.first().map(|r| json!({
                    "domain_id": r.get::<_, String>(0),
                    "site_id": r.get::<_, String>(1),
                    "hostname": r.get::<_, String>(2),
                    "display_name": r.get::<_, String>(3),
                    "collect_enabled": r.get::<_, i32>(4) != 0,
                    "ssl_status": r.get::<_, String>(5),
                    "ssl_expires_ms": r.get::<_, Option<i64>>(6),
                    "cert_path": r.get::<_, Option<String>>(7),
                    "key_path": r.get::<_, Option<String>>(8),
                    "created_ms": r.get::<_, i64>(9),
                    "updated_ms": r.get::<_, i64>(10),
                })))
            });
        }
        let c = self.lock()?;
        self.domain_row_unlocked(&c, domain_id)
    }

    /// Lookup domain by hostname (site binding / SDK host checks).
    ///
    /// Order: exact `domains.hostname` → exact `root_domains.root_domain` →
    /// longest suffix match on either table (e.g. `www.shop.example` → `shop.example`).
    pub fn get_domain_by_hostname(&self, hostname: &str) -> Result<Option<Value>, String> {
        let host = hostname.trim().to_ascii_lowercase();
        if host.is_empty() {
            return Ok(None);
        }
        if self.is_pg() {
            return self.with_pg(move |c| {
                let rows = c.query(
                    "SELECT domain_id, site_id, hostname, display_name, collect_enabled, ssl_status, ssl_expires_ms, cert_path, key_path, created_ms, updated_ms FROM domains WHERE hostname=$1",
                    &[&host],
                ).map_err(|e| e.to_string())?;
                if let Some(r) = rows.first() {
                    return Ok(Some(json!({
                        "domain_id": r.get::<_, String>(0),
                        "site_id": r.get::<_, String>(1),
                        "hostname": r.get::<_, String>(2),
                        "display_name": r.get::<_, String>(3),
                        "collect_enabled": r.get::<_, i32>(4) != 0,
                        "ssl_status": r.get::<_, String>(5),
                        "ssl_expires_ms": r.get::<_, Option<i64>>(6),
                        "cert_path": r.get::<_, Option<String>>(7),
                        "key_path": r.get::<_, Option<String>>(8),
                        "created_ms": r.get::<_, i64>(9),
                        "updated_ms": r.get::<_, i64>(10),
                    })));
                }
                // root_domains exact
                if let Ok(rows) = c.query(
                    "SELECT domain_id, site_id, root_domain, collect_enabled, created_ms FROM root_domains WHERE lower(root_domain)=$1",
                    &[&host],
                ) {
                    if let Some(r) = rows.first() {
                        let root: String = r.get(2);
                        return Ok(Some(json!({
                            "domain_id": r.get::<_, String>(0),
                            "site_id": r.get::<_, String>(1),
                            "hostname": root,
                            "display_name": "root_domain",
                            "collect_enabled": r.get::<_, i32>(3) != 0,
                            "ssl_status": "none",
                            "ssl_expires_ms": Value::Null,
                            "cert_path": Value::Null,
                            "key_path": Value::Null,
                            "created_ms": r.get::<_, i64>(4),
                            "updated_ms": r.get::<_, i64>(4),
                            "source": "root_domains",
                        })));
                    }
                }
                // suffix match on domains
                if let Ok(rows) = c.query(
                    "SELECT domain_id, site_id, hostname, display_name, collect_enabled, ssl_status, ssl_expires_ms, cert_path, key_path, created_ms, updated_ms FROM domains ORDER BY length(hostname) DESC",
                    &[],
                ) {
                    for r in &rows {
                        let hn: String = r.get(2);
                        if host == hn || host.ends_with(&format!(".{hn}")) {
                            return Ok(Some(json!({
                                "domain_id": r.get::<_, String>(0),
                                "site_id": r.get::<_, String>(1),
                                "hostname": hn,
                                "display_name": r.get::<_, String>(3),
                                "collect_enabled": r.get::<_, i32>(4) != 0,
                                "ssl_status": r.get::<_, String>(5),
                                "ssl_expires_ms": r.get::<_, Option<i64>>(6),
                                "cert_path": r.get::<_, Option<String>>(7),
                                "key_path": r.get::<_, Option<String>>(8),
                                "created_ms": r.get::<_, i64>(9),
                                "updated_ms": r.get::<_, i64>(10),
                                "source": "domains_suffix",
                            })));
                        }
                    }
                }
                // suffix match on root_domains
                if let Ok(rows) = c.query(
                    "SELECT domain_id, site_id, root_domain, collect_enabled, created_ms FROM root_domains ORDER BY length(root_domain) DESC",
                    &[],
                ) {
                    for r in &rows {
                        let root: String = r.get(2);
                        let root_l = root.to_ascii_lowercase();
                        if host == root_l || host.ends_with(&format!(".{root_l}")) {
                            return Ok(Some(json!({
                                "domain_id": r.get::<_, String>(0),
                                "site_id": r.get::<_, String>(1),
                                "hostname": root_l,
                                "display_name": "root_domain",
                                "collect_enabled": r.get::<_, i32>(3) != 0,
                                "ssl_status": "none",
                                "ssl_expires_ms": Value::Null,
                                "cert_path": Value::Null,
                                "key_path": Value::Null,
                                "created_ms": r.get::<_, i64>(4),
                                "updated_ms": r.get::<_, i64>(4),
                                "source": "root_domains_suffix",
                            })));
                        }
                    }
                }
                Ok(None)
            });
        }
        let c = self.lock()?;
        let id: Option<String> = c
            .query_row(
                "SELECT domain_id FROM domains WHERE hostname=?1",
                params![host],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some(id) = id {
            return self.domain_row_unlocked(&c, &id);
        }
        // root_domains exact (control-plane sites API writes here; probe domains may be empty)
        let root_hit: Option<(String, String, String, i64, i64)> = c
            .query_row(
                "SELECT domain_id, site_id, root_domain, collect_enabled, created_ms FROM root_domains WHERE lower(root_domain)=?1",
                params![host],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()
            .unwrap_or(None);
        if let Some((did, sid, root, en, ms)) = root_hit {
            return Ok(Some(json!({
                "domain_id": did,
                "site_id": sid,
                "hostname": root,
                "display_name": "root_domain",
                "collect_enabled": en != 0,
                "ssl_status": "none",
                "ssl_expires_ms": Value::Null,
                "cert_path": Value::Null,
                "key_path": Value::Null,
                "created_ms": ms,
                "updated_ms": ms,
                "source": "root_domains",
            })));
        }
        // suffix match domains (www.shop… → shop…)
        {
            let mut stmt = c
                .prepare(
                    "SELECT domain_id, site_id, hostname, display_name, collect_enabled, ssl_status, ssl_expires_ms, cert_path, key_path, created_ms, updated_ms FROM domains",
                )
                .map_err(|e| e.to_string())?;
            let mut best: Option<(usize, Value)> = None;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, Option<i64>>(6)?,
                        r.get::<_, Option<String>>(7)?,
                        r.get::<_, Option<String>>(8)?,
                        r.get::<_, i64>(9)?,
                        r.get::<_, i64>(10)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            for row in rows {
                let (did, sid, hn, disp, en, ssl, exp, cert, key, cms, ums) =
                    row.map_err(|e| e.to_string())?;
                let hn_l = hn.to_ascii_lowercase();
                if host == hn_l || host.ends_with(&format!(".{hn_l}")) {
                    let score = hn_l.len();
                    if best.as_ref().map(|(s, _)| score > *s).unwrap_or(true) {
                        best = Some((
                            score,
                            json!({
                                "domain_id": did,
                                "site_id": sid,
                                "hostname": hn_l,
                                "display_name": disp,
                                "collect_enabled": en != 0,
                                "ssl_status": ssl,
                                "ssl_expires_ms": exp,
                                "cert_path": cert,
                                "key_path": key,
                                "created_ms": cms,
                                "updated_ms": ums,
                                "source": "domains_suffix",
                            }),
                        ));
                    }
                }
            }
            if let Some((_, v)) = best {
                return Ok(Some(v));
            }
        }
        // suffix match root_domains
        {
            let mut stmt = match c.prepare(
                "SELECT domain_id, site_id, root_domain, collect_enabled, created_ms FROM root_domains",
            ) {
                Ok(s) => s,
                Err(_) => return Ok(None), // table may not exist on pure probe_admin
            };
            let mut best: Option<(usize, Value)> = None;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            for row in rows {
                let (did, sid, root, en, ms) = row.map_err(|e| e.to_string())?;
                let root_l = root.to_ascii_lowercase();
                if host == root_l || host.ends_with(&format!(".{root_l}")) {
                    let score = root_l.len();
                    if best.as_ref().map(|(s, _)| score > *s).unwrap_or(true) {
                        best = Some((
                            score,
                            json!({
                                "domain_id": did,
                                "site_id": sid,
                                "hostname": root_l,
                                "display_name": "root_domain",
                                "collect_enabled": en != 0,
                                "ssl_status": "none",
                                "ssl_expires_ms": Value::Null,
                                "cert_path": Value::Null,
                                "key_path": Value::Null,
                                "created_ms": ms,
                                "updated_ms": ms,
                                "source": "root_domains_suffix",
                            }),
                        ));
                    }
                }
            }
            if let Some((_, v)) = best {
                return Ok(Some(v));
            }
        }
        Ok(None)
    }

    pub fn list_domains(
        &self,
        site_id: Option<&str>,
        q: Option<&str>,
    ) -> Result<Vec<Value>, String> {
        if self.is_pg() {
            let site_id = site_id.map(|s| s.to_string());
            let q = q.map(|s| s.to_string());
            return self.with_pg(move |c| {
                let rows = match (site_id.as_deref(), q.as_deref()) {
                    (Some(s), Some(qq)) => {
                        let like = format!("%{qq}%");
                        c.query("SELECT domain_id, site_id, hostname, display_name, collect_enabled, ssl_status, ssl_expires_ms, cert_path, key_path, created_ms, updated_ms FROM domains WHERE site_id=$1 AND (hostname ILIKE $2 OR display_name ILIKE $2 OR domain_id ILIKE $2) ORDER BY updated_ms DESC", &[&s, &like])
                    }
                    (Some(s), None) => c.query("SELECT domain_id, site_id, hostname, display_name, collect_enabled, ssl_status, ssl_expires_ms, cert_path, key_path, created_ms, updated_ms FROM domains WHERE site_id=$1 ORDER BY updated_ms DESC", &[&s]),
                    (None, Some(qq)) => {
                        let like = format!("%{qq}%");
                        c.query("SELECT domain_id, site_id, hostname, display_name, collect_enabled, ssl_status, ssl_expires_ms, cert_path, key_path, created_ms, updated_ms FROM domains WHERE hostname ILIKE $1 OR display_name ILIKE $1 OR domain_id ILIKE $1 ORDER BY updated_ms DESC", &[&like])
                    }
                    (None, None) => c.query("SELECT domain_id, site_id, hostname, display_name, collect_enabled, ssl_status, ssl_expires_ms, cert_path, key_path, created_ms, updated_ms FROM domains ORDER BY updated_ms DESC", &[]),
                }.map_err(|e| e.to_string())?;
                Ok(rows.iter().map(|r| json!({
                    "domain_id": r.get::<_, String>(0),
                    "site_id": r.get::<_, String>(1),
                    "hostname": r.get::<_, String>(2),
                    "display_name": r.get::<_, String>(3),
                    "collect_enabled": r.get::<_, i32>(4) != 0,
                    "ssl_status": r.get::<_, String>(5),
                    "ssl_expires_ms": r.get::<_, Option<i64>>(6),
                    "cert_path": r.get::<_, Option<String>>(7),
                    "key_path": r.get::<_, Option<String>>(8),
                    "created_ms": r.get::<_, i64>(9),
                    "updated_ms": r.get::<_, i64>(10),
                })).collect())
            });
        }
        let c = self.lock()?;
        let mut sql = String::from(
            "SELECT domain_id, site_id, hostname, display_name, collect_enabled,
                    ssl_status, ssl_expires_ms, cert_path, key_path, created_ms, updated_ms
             FROM domains WHERE 1=1",
        );
        let mut args: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        if let Some(s) = site_id.filter(|s| !s.is_empty()) {
            sql.push_str(" AND site_id=?");
            args.push(Box::new(s.to_string()));
        }
        if let Some(q) = q.filter(|s| !s.is_empty()) {
            sql.push_str(" AND (hostname LIKE ? OR display_name LIKE ? OR domain_id LIKE ?)");
            let like = format!("%{q}%");
            args.push(Box::new(like.clone()));
            args.push(Box::new(like.clone()));
            args.push(Box::new(like));
        }
        sql.push_str(" ORDER BY updated_ms DESC");
        let mut stmt = c.prepare(&sql).map_err(|e| e.to_string())?;
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            args.iter().map(|a| a.as_ref()).collect();
        let mut out: Vec<Value> = stmt
            .query_map(params_ref.as_slice(), |r| {
                Ok(json!({
                    "domain_id": r.get::<_, String>(0)?,
                    "site_id": r.get::<_, String>(1)?,
                    "hostname": r.get::<_, String>(2)?,
                    "display_name": r.get::<_, String>(3)?,
                    "collect_enabled": r.get::<_, i64>(4)? != 0,
                    "ssl_status": r.get::<_, String>(5)?,
                    "ssl_expires_ms": r.get::<_, Option<i64>>(6)?,
                    "cert_path": r.get::<_, Option<String>>(7)?,
                    "key_path": r.get::<_, Option<String>>(8)?,
                    "created_ms": r.get::<_, i64>(9)?,
                    "updated_ms": r.get::<_, i64>(10)?,
                }))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        // Union root_domains so CORS / host boards see control-plane site bindings
        // even when `domains` was never populated (lab dual-admin DB split).
        if let Ok(mut stmt) = c.prepare(
            "SELECT domain_id, site_id, root_domain, collect_enabled, created_ms FROM root_domains",
        ) {
            let roots = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            });
            if let Ok(roots) = roots {
                let mut seen: std::collections::HashSet<String> = out
                    .iter()
                    .filter_map(|v| v.get("hostname").and_then(|h| h.as_str()).map(|s| s.to_ascii_lowercase()))
                    .collect();
                for row in roots.flatten() {
                    let (did, sid, root, en, ms) = row;
                    if let Some(filter_site) = site_id.filter(|s| !s.is_empty()) {
                        if sid != filter_site {
                            continue;
                        }
                    }
                    let root_l = root.to_ascii_lowercase();
                    if let Some(qq) = q.filter(|s| !s.is_empty()) {
                        let ql = qq.to_ascii_lowercase();
                        if !root_l.contains(&ql) && !sid.to_ascii_lowercase().contains(&ql) {
                            continue;
                        }
                    }
                    if seen.insert(root_l.clone()) {
                        out.push(json!({
                            "domain_id": did,
                            "site_id": sid,
                            "hostname": root_l,
                            "display_name": "root_domain",
                            "collect_enabled": en != 0,
                            "ssl_status": "none",
                            "ssl_expires_ms": Value::Null,
                            "cert_path": Value::Null,
                            "key_path": Value::Null,
                            "created_ms": ms,
                            "updated_ms": ms,
                            "source": "root_domains",
                        }));
                    }
                }
            }
        }
        Ok(out)
    }

    pub fn set_domain_collect(&self, domain_id: &str, enabled: bool) -> Result<(), String> {
        if self.is_pg() {
            let domain_id = domain_id.to_string();
            let now = now_ms();
            let en = enabled as i32;
            return self.with_pg(move |c| {
                let n = c.execute("UPDATE domains SET collect_enabled=$1, updated_ms=$2 WHERE domain_id=$3", &[&en, &now, &domain_id]).map_err(|e| e.to_string())?;
                if n == 0 { Err("domain_not_found".into()) } else { Ok(()) }
            });
        }
        let c = self.lock()?;
        let n = c
            .execute(
                "UPDATE domains SET collect_enabled=?1, updated_ms=?2 WHERE domain_id=?3",
                params![enabled as i64, now_ms(), domain_id],
            )
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("domain_not_found".into());
        }
        Ok(())
    }

    pub fn update_domain_ssl(
        &self,
        domain_id: &str,
        status: &str,
        expires_ms: Option<i64>,
        cert_path: &str,
        key_path: &str,
    ) -> Result<Value, String> {
        if self.is_pg() {
            let domain_id = domain_id.to_string();
            let status = status.to_string();
            let cert_path = cert_path.to_string();
            let key_path = key_path.to_string();
            let now = now_ms();
            return self.with_pg(move |c| {
                c.execute(
                    "UPDATE domains SET ssl_status=$1, ssl_expires_ms=$2, cert_path=$3, key_path=$4, updated_ms=$5 WHERE domain_id=$6",
                    &[&status, &expires_ms, &cert_path, &key_path, &now, &domain_id],
                ).map_err(|e| e.to_string())?;
                let rows = c.query(
                    "SELECT domain_id, site_id, hostname, display_name, collect_enabled, ssl_status, ssl_expires_ms, cert_path, key_path, created_ms, updated_ms FROM domains WHERE domain_id=$1",
                    &[&domain_id],
                ).map_err(|e| e.to_string())?;
                let r = rows.first().ok_or_else(|| "domain_not_found".to_string())?;
                Ok(json!({
                    "domain_id": r.get::<_, String>(0), "site_id": r.get::<_, String>(1), "hostname": r.get::<_, String>(2),
                    "display_name": r.get::<_, String>(3), "collect_enabled": r.get::<_, i32>(4) != 0,
                    "ssl_status": r.get::<_, String>(5), "ssl_expires_ms": r.get::<_, Option<i64>>(6),
                    "cert_path": r.get::<_, Option<String>>(7), "key_path": r.get::<_, Option<String>>(8),
                    "created_ms": r.get::<_, i64>(9), "updated_ms": r.get::<_, i64>(10),
                }))
            });
        }
        let c = self.lock()?;
        c.execute(
            "UPDATE domains SET ssl_status=?1, ssl_expires_ms=?2, cert_path=?3, key_path=?4, updated_ms=?5
             WHERE domain_id=?6",
            params![status, expires_ms, cert_path, key_path, now_ms(), domain_id],
        )
        .map_err(|e| e.to_string())?;
        self.domain_row_unlocked(&c, domain_id)?
            .ok_or_else(|| "domain_not_found".into())
    }

    pub fn put_acme_challenge(
        &self,
        token: &str,
        content: &str,
        domain_id: &str,
        ttl_ms: i64,
    ) -> Result<(), String> {
        if self.is_pg() {
            let token = token.to_string();
            let content = content.to_string();
            let domain_id = domain_id.to_string();
            let exp = now_ms() + ttl_ms;
            return self.with_pg(move |c| {
                c.execute(
                    "INSERT INTO acme_challenges(token, content, domain_id, exp_ms) VALUES ($1,$2,$3,$4) ON CONFLICT(token) DO UPDATE SET content=EXCLUDED.content, domain_id=EXCLUDED.domain_id, exp_ms=EXCLUDED.exp_ms",
                    &[&token, &content, &domain_id, &exp],
                ).map_err(|e| e.to_string())?;
                Ok(())
            });
        }
        let c = self.lock()?;
        c.execute(
            "INSERT INTO acme_challenges(token, content, domain_id, exp_ms) VALUES (?1,?2,?3,?4)
             ON CONFLICT(token) DO UPDATE SET content=excluded.content, domain_id=excluded.domain_id, exp_ms=excluded.exp_ms",
            params![token, content, domain_id, now_ms() + ttl_ms],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn get_acme_challenge(&self, token: &str) -> Result<Option<String>, String> {
        if self.is_pg() {
            let token = token.to_string();
            let now = now_ms();
            return self.with_pg(move |c| {
                let rows = c.query("SELECT content, exp_ms FROM acme_challenges WHERE token=$1", &[&token]).map_err(|e| e.to_string())?;
                if let Some(r) = rows.first() {
                    let content: String = r.get(0);
                    let exp: i64 = r.get(1);
                    if exp >= now { Ok(Some(content)) } else { Ok(None) }
                } else { Ok(None) }
            });
        }
        let now = now_ms();
        let c = self.lock()?;
        let row: Option<(String, i64)> = c
            .query_row(
                "SELECT content, exp_ms FROM acme_challenges WHERE token=?1",
                params![token],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        match row {
            Some((content, exp)) if exp >= now => Ok(Some(content)),
            _ => Ok(None),
        }
    }

    /// Panel Workers setting (gr-admin writes `control.worker_settings`).
    /// Returns (analyze, ingest, gateway); Err when the table is absent
    /// (legacy sqlite/lab) so callers keep the boot-time target.
    pub fn get_worker_settings(&self) -> Result<(i32, i32, i32), String> {
        if self.is_pg() {
            return self.with_pg(|c| {
                let rows = c.query(
                    "SELECT analyze_workers, ingest_workers, gateway_workers FROM control.worker_settings WHERE id=1",
                    &[],
                )
                .map_err(|e| e.to_string())?;
                let r = rows.first().ok_or_else(|| "no worker_settings row".to_string())?;
                // int4 columns — read as i32 (Row::get panics on mismatch and
                // would take down the shared admin pg worker thread).
                let a: i32 = r.get(0);
                let i: i32 = r.get(1);
                let g: i32 = r.get(2);
                Ok((a.max(1), i.max(1), g.max(1)))
            });
        }
        Err("worker_settings requires PG control plane".to_string())
    }

    pub fn get_runtime_desired(&self) -> Result<Value, String> {
        if self.is_pg() {
            return self.with_pg(move |c| {
                let rows = c.query("SELECT config_json FROM runtime_desired WHERE id=1", &[]).map_err(|e| e.to_string())?;
                Ok(rows.first().and_then(|r| {
                    let s: String = r.get(0);
                    serde_json::from_str(&s).ok()
                }).unwrap_or_else(|| json!({})))
            });
        }
        let c = self.lock()?;
        let row: Option<String> = c
            .query_row(
                "SELECT config_json FROM runtime_desired WHERE id=1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(row
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| json!({})))
    }

    pub fn set_runtime_desired(&self, cfg: &Value) -> Result<(), String> {
        if self.is_pg() {
            let s = cfg.to_string();
            let now = now_ms();
            return self.with_pg(move |c| {
                c.execute(
                    "INSERT INTO runtime_desired(id, config_json, updated_ms) VALUES (1,$1,$2) ON CONFLICT(id) DO UPDATE SET config_json=EXCLUDED.config_json, updated_ms=EXCLUDED.updated_ms",
                    &[&s, &now],
                ).map_err(|e| e.to_string())?;
                Ok(())
            });
        }
        let c = self.lock()?;
        c.execute(
            "INSERT INTO runtime_desired(id, config_json, updated_ms) VALUES (1,?1,?2)
             ON CONFLICT(id) DO UPDATE SET config_json=excluded.config_json, updated_ms=excluded.updated_ms",
            params![cfg.to_string(), now_ms()],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn insert_sdk_key(
        &self,
        site_id: &str,
        kind: &str,
        secret_hash: &str,
        secret_prefix: &str,
        allowed_origins: &[String],
    ) -> Result<Value, String> {
        if self.is_pg() {
            let id = new_id("sdk");
            let now = now_ms();
            let origins = serde_json::to_string(allowed_origins).unwrap_or_else(|_| "[]".into());
            let site_id = site_id.to_string();
            let kind = kind.to_string();
            let secret_hash = secret_hash.to_string();
            let secret_prefix = secret_prefix.to_string();
            let id2 = id.clone();
            let ao = allowed_origins.to_vec();
            let site_r = site_id.clone();
            let kind_r = kind.clone();
            let pref_r = secret_prefix.clone();
            self.with_pg(move |c| {
                c.execute(
                    "INSERT INTO sdk_keys(key_id, site_id, kind, secret_hash, secret_prefix, status, allowed_origins_json, created_ms) VALUES ($1,$2,$3,$4,$5,'active',$6,$7)",
                    &[&id2, &site_id, &kind, &secret_hash, &secret_prefix, &origins, &now],
                ).map_err(|e| e.to_string())?;
                Ok(())
            })?;
            return Ok(json!({"key_id": id, "site_id": site_r, "kind": kind_r, "secret_prefix": pref_r, "status": "active", "allowed_origins": ao, "created_ms": now}));
        }
        let id = new_id("sdk");
        let now = now_ms();
        let origins = serde_json::to_string(allowed_origins).unwrap_or_else(|_| "[]".into());
        let c = self.lock()?;
        c.execute(
            "INSERT INTO sdk_keys(key_id, site_id, kind, secret_hash, secret_prefix, status, allowed_origins_json, created_ms)
             VALUES (?1,?2,?3,?4,?5,'active',?6,?7)",
            params![id, site_id, kind, secret_hash, secret_prefix, origins, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(json!({
            "key_id": id,
            "site_id": site_id,
            "kind": kind,
            "secret_prefix": secret_prefix,
            "status": "active",
            "allowed_origins": allowed_origins,
            "created_ms": now,
        }))
    }

    pub fn list_sdk_keys(&self, site_id: Option<&str>) -> Result<Vec<Value>, String> {
        if self.is_pg() {
            let site_id = site_id.map(|s| s.to_string());
            return self.with_pg(move |c| {
                let rows = if let Some(ref s) = site_id {
                    c.query("SELECT key_id, site_id, kind, secret_prefix, status, allowed_origins_json, created_ms, revoked_ms FROM sdk_keys WHERE site_id=$1 ORDER BY created_ms DESC", &[s])
                } else {
                    c.query("SELECT key_id, site_id, kind, secret_prefix, status, allowed_origins_json, created_ms, revoked_ms FROM sdk_keys ORDER BY created_ms DESC", &[])
                }.map_err(|e| e.to_string())?;
                Ok(rows.iter().map(|r| {
                    let origins: String = r.get(5);
                    json!({
                        "key_id": r.get::<_, String>(0), "site_id": r.get::<_, String>(1), "kind": r.get::<_, String>(2),
                        "secret_prefix": r.get::<_, String>(3), "status": r.get::<_, String>(4),
                        "allowed_origins": serde_json::from_str::<Value>(&origins).unwrap_or(json!([])),
                        "created_ms": r.get::<_, i64>(6), "revoked_ms": r.get::<_, Option<i64>>(7),
                    })
                }).collect())
            });
        }
        let c = self.lock()?;
        let mut out = Vec::new();
        if let Some(s) = site_id.filter(|s| !s.is_empty()) {
            let mut stmt = c
                .prepare(
                    "SELECT key_id, site_id, kind, secret_prefix, status, allowed_origins_json, created_ms, revoked_ms
                     FROM sdk_keys WHERE site_id=?1 ORDER BY created_ms DESC",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params![s], |r| {
                    let origins: String = r.get(5)?;
                    Ok(json!({
                        "key_id": r.get::<_, String>(0)?,
                        "site_id": r.get::<_, String>(1)?,
                        "kind": r.get::<_, String>(2)?,
                        "secret_prefix": r.get::<_, String>(3)?,
                        "status": r.get::<_, String>(4)?,
                        "allowed_origins": serde_json::from_str::<Value>(&origins).unwrap_or(json!([])),
                        "created_ms": r.get::<_, i64>(6)?,
                        "revoked_ms": r.get::<_, Option<i64>>(7)?,
                    }))
                })
                .map_err(|e| e.to_string())?;
            for row in rows {
                out.push(row.map_err(|e| e.to_string())?);
            }
        } else {
            let mut stmt = c
                .prepare(
                    "SELECT key_id, site_id, kind, secret_prefix, status, allowed_origins_json, created_ms, revoked_ms
                     FROM sdk_keys ORDER BY created_ms DESC",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |r| {
                    let origins: String = r.get(5)?;
                    Ok(json!({
                        "key_id": r.get::<_, String>(0)?,
                        "site_id": r.get::<_, String>(1)?,
                        "kind": r.get::<_, String>(2)?,
                        "secret_prefix": r.get::<_, String>(3)?,
                        "status": r.get::<_, String>(4)?,
                        "allowed_origins": serde_json::from_str::<Value>(&origins).unwrap_or(json!([])),
                        "created_ms": r.get::<_, i64>(6)?,
                        "revoked_ms": r.get::<_, Option<i64>>(7)?,
                    }))
                })
                .map_err(|e| e.to_string())?;
            for row in rows {
                out.push(row.map_err(|e| e.to_string())?);
            }
        }
        Ok(out)
    }

    pub fn revoke_sdk_key(&self, key_id: &str) -> Result<(), String> {
        if self.is_pg() {
            let key_id = key_id.to_string();
            let now = now_ms();
            return self.with_pg(move |c| {
                let n = c.execute("UPDATE sdk_keys SET status='revoked', revoked_ms=$1 WHERE key_id=$2", &[&now, &key_id]).map_err(|e| e.to_string())?;
                if n == 0 { Err("key_not_found".into()) } else { Ok(()) }
            });
        }
        let c = self.lock()?;
        let n = c
            .execute(
                "UPDATE sdk_keys SET status='revoked', revoked_ms=?1 WHERE key_id=?2",
                params![now_ms(), key_id],
            )
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("key_not_found".into());
        }
        Ok(())
    }

    /// Lookup active backend key by raw secret hash.
    pub fn find_active_backend_key(
        &self,
        secret_hash: &str,
    ) -> Result<Option<(String, String, Vec<String>)>, String> {
        self.find_active_sdk_key(secret_hash, Some("backend"))
    }

    /// Lookup active SDK key (`fe_embed` or `backend`) by secret hash.
    /// When `kind` is None, either kind matches.
    pub fn find_active_sdk_key(
        &self,
        secret_hash: &str,
        kind: Option<&str>,
    ) -> Result<Option<(String, String, Vec<String>)>, String> {
        if self.is_pg() {
            let secret_hash = secret_hash.to_string();
            let kind = kind.map(|s| s.to_string());
            return self.with_pg(move |c| {
                let rows = match kind.as_deref() {
                    Some(k) => c.query("SELECT key_id, site_id, allowed_origins_json FROM sdk_keys WHERE secret_hash=$1 AND kind=$2 AND status='active'", &[&secret_hash, &k]),
                    None => c.query("SELECT key_id, site_id, allowed_origins_json FROM sdk_keys WHERE secret_hash=$1 AND kind IN ('fe_embed','backend') AND status='active'", &[&secret_hash]),
                }.map_err(|e| e.to_string())?;
                Ok(rows.first().map(|r| {
                    let origins: String = r.get(2);
                    let o: Vec<String> = serde_json::from_str(&origins).unwrap_or_default();
                    (r.get::<_, String>(0), r.get::<_, String>(1), o)
                }))
            });
        }
        let c = self.lock()?;
        let row: Option<(String, String, String)> = match kind {
            Some(k) => c
                .query_row(
                    "SELECT key_id, site_id, allowed_origins_json FROM sdk_keys
                     WHERE secret_hash=?1 AND kind=?2 AND status='active'",
                    params![secret_hash, k],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .map_err(|e| e.to_string())?,
            None => c
                .query_row(
                    "SELECT key_id, site_id, allowed_origins_json FROM sdk_keys
                     WHERE secret_hash=?1 AND kind IN ('fe_embed','backend') AND status='active'",
                    params![secret_hash],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .map_err(|e| e.to_string())?,
        };
        Ok(row.map(|(kid, sid, origins)| {
            let o: Vec<String> = serde_json::from_str(&origins).unwrap_or_default();
            (kid, sid, o)
        }))
    }

    pub fn site_hostnames(&self, site_id: &str) -> Result<Vec<String>, String> {
        if self.is_pg() {
            let site_id = site_id.to_string();
            return self.with_pg(move |c| {
                let rows = c.query("SELECT hostname FROM domains WHERE site_id=$1 AND collect_enabled=1", &[&site_id]).map_err(|e| e.to_string())?;
                Ok(rows.iter().map(|r| r.get(0)).collect())
            });
        }
        let c = self.lock()?;
        let mut stmt = c
            .prepare("SELECT hostname FROM domains WHERE site_id=?1 AND collect_enabled=1")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![site_id], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    pub fn sdk_enforce_enabled(&self) -> bool {
        let deploy = gr_abi::env::get("DEPLOY_ENV")
             .or_else(|| gr_abi::env::get("DEPLOY_ENV"))
            .unwrap_or_default();
        let prod = matches!(
            deploy.trim().to_ascii_lowercase().as_str(),
            "prod" | "production" | "live"
        );
        if prod {
            return true;
        }
        if let Some(v) = self.get_setting("sdk_enforce").ok().flatten() {
            return v == "1" || v.eq_ignore_ascii_case("true");
        }
        false
    }
}
