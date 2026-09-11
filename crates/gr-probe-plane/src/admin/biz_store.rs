//! Independent business dashboard store (gr_biz) — separate from probe/analyze DB.
//!
//! DSN: `GR_BIZ_DATABASE_URL` / `GR_BIZ_DATABASE_URL` (PostgreSQL only).

use postgres::{Client, Config, NoTls};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, SyncSender};
use std::sync::Mutex;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

fn connect_biz_pg(dsn: &str) -> Result<Client, String> {
    let mut cfg: Config = dsn
        .parse()
        .map_err(|e| format!("gr_biz dsn parse: {e}"))?;
    cfg.application_name("gr-biz");
    let mut c = cfg
        .connect(NoTls)
        .map_err(|e| format!("gr_biz connect: {e}"))?;
    // P1-3 (iss/grok4.6/05): session-level NOTICE suppression (see
    // admin/db.rs connect_admin_pg — startup `options` deadlocked boot).
    let _ = c.batch_execute("SET client_min_messages = warning");
    Ok(c)
}

const SQLITE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS biz_visits (
  visit_id TEXT PRIMARY KEY,
  site_id TEXT NOT NULL,
  visitor_terminal_id TEXT NOT NULL,
  visitor_facet TEXT NOT NULL DEFAULT 'unknown',
  session_id TEXT NOT NULL DEFAULT '',
  page_host TEXT NOT NULL DEFAULT '',
  ua_hash TEXT NOT NULL DEFAULT '',
  created_ms INTEGER NOT NULL,
  updated_ms INTEGER NOT NULL,
  summary_json TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_biz_visits_site_ms ON biz_visits(site_id, updated_ms DESC);
CREATE INDEX IF NOT EXISTS idx_biz_visits_facet ON biz_visits(visitor_facet, updated_ms DESC);
CREATE INDEX IF NOT EXISTS idx_biz_visits_vt ON biz_visits(visitor_terminal_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_biz_visits_site_session
  ON biz_visits(site_id, session_id) WHERE session_id != '';
CREATE TABLE IF NOT EXISTS biz_visit_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  visit_id TEXT NOT NULL,
  site_id TEXT NOT NULL,
  visitor_terminal_id TEXT NOT NULL,
  event TEXT NOT NULL,
  detail_json TEXT NOT NULL DEFAULT '{}',
  ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_biz_events_visit ON biz_visit_events(visit_id, ms DESC);
"#;

const PG_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS biz_visits (
  visit_id TEXT PRIMARY KEY,
  site_id TEXT NOT NULL,
  visitor_terminal_id TEXT NOT NULL,
  visitor_facet TEXT NOT NULL DEFAULT 'unknown',
  session_id TEXT NOT NULL DEFAULT '',
  page_host TEXT NOT NULL DEFAULT '',
  ua_hash TEXT NOT NULL DEFAULT '',
  created_ms BIGINT NOT NULL,
  updated_ms BIGINT NOT NULL,
  summary_json TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_biz_visits_site_ms ON biz_visits(site_id, updated_ms DESC);
CREATE INDEX IF NOT EXISTS idx_biz_visits_facet ON biz_visits(visitor_facet, updated_ms DESC);
CREATE INDEX IF NOT EXISTS idx_biz_visits_vt ON biz_visits(visitor_terminal_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_biz_visits_site_session
  ON biz_visits(site_id, session_id) WHERE session_id <> '';
CREATE TABLE IF NOT EXISTS biz_visit_events (
  id BIGSERIAL PRIMARY KEY,
  visit_id TEXT NOT NULL,
  site_id TEXT NOT NULL,
  visitor_terminal_id TEXT NOT NULL,
  event TEXT NOT NULL,
  detail_json TEXT NOT NULL DEFAULT '{}',
  ms BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_biz_events_visit ON biz_visit_events(visit_id, ms DESC);
"#;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn new_visit_id() -> String {
    use rand::RngCore;
    let mut b = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut b);
    format!("bv_{}", hex::encode(b))
}

pub fn ua_hash(ua: &str) -> String {
    if ua.is_empty() {
        return String::new();
    }
    let mut h = Sha256::new();
    h.update(ua.as_bytes());
    hex::encode(h.finalize())[..16].to_string()
}

#[derive(Clone, Debug)]
pub struct VisitUpsert {
    pub site_id: String,
    pub visitor_terminal_id: String,
    pub visitor_facet: String,
    pub session_id: String,
    pub page_host: String,
    pub ua_hash: String,
    pub summary: Value,
    pub event: Option<String>,
    pub event_detail: Value,
}

enum Backend {
    Sqlite(Mutex<Connection>),
    Postgres {
        jobs: SyncSender<JobFn>,
        // backend label kept for ops log attribution
        #[allow(dead_code)]
        label: String,
    },
}

type JobFn = Box<dyn FnOnce(&mut Client) + Send>;

pub struct BizStore {
    backend: Backend,
    pub path_or_label: String,
}

impl BizStore {
    /// Open from `GR_BIZ_DATABASE_URL` / `GR_BIZ_DATABASE_URL` (PostgreSQL only).
    pub fn open_auto(_data_dir: &Path) -> Result<Self, String> {
        let dsn = gr_abi::env::get("BIZ_DATABASE_URL")
             .or_else(|| gr_abi::env::get("BIZ_DATABASE_URL"))
            .unwrap_or_default();
        let dsn = dsn.trim().to_string();
        if dsn.is_empty() {
            return Err(
                "GR_BIZ_DATABASE_URL is required; SQLite biz store was removed".into(),
            );
        }
        Self::open_postgres(&dsn)
    }

    pub fn open_sqlite(path: &Path) -> Result<Self, String> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        conn.execute_batch(SQLITE_SCHEMA).map_err(|e| e.to_string())?;
        Ok(Self {
            backend: Backend::Sqlite(Mutex::new(conn)),
            path_or_label: path.display().to_string(),
        })
    }

    pub fn open_postgres(dsn: &str) -> Result<Self, String> {
        let dsn_owned = dsn.to_string();
        let label = redact_dsn(dsn);
        let (tx, rx) = mpsc::sync_channel::<JobFn>(64);
        let label_t = label.clone();
        thread::Builder::new()
            .name("gr-biz-pg".into())
            .spawn(move || {
                // iss/audit STO-02 (scenario B): a fresh PG entrypoint restart
                // window used to kill this thread on the FIRST connect failure
                // — every later biz op then errored with "channel closed"
                // forever. Retry the bootstrap (connect + schema) a few times
                // before giving up; on final failure keep the documented
                // quiet-exit error path (fulltest asserts zero log panics).
                let mut client = None;
                for attempt in 1..=3u32 {
                    match connect_biz_pg(&dsn_owned) {
                        Ok(mut c) => {
                            match c.batch_execute(PG_SCHEMA) {
                                Ok(()) => {
                                    client = Some(c);
                                    break;
                                }
                                Err(e) => {
                                    log::warn!(
                                        "gr_biz pg schema failed ({label_t}) attempt {attempt}/3: {e}"
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            log::warn!(
                                "gr_biz pg connect failed ({label_t}) attempt {attempt}/3: {e}"
                            );
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
                let mut client = match client {
                    Some(c) => c,
                    None => {
                        log::warn!(
                            "gr_biz pg bootstrap failed after 3 attempts ({label_t}); jobs will error"
                        );
                        return;
                    }
                };
                while let Ok(job) = rx.recv() {
                    // iss/audit STO-02 (scenario A): heal a broken runtime
                    // connection (PG restart / failover) instead of erroring
                    // every job forever on a dead socket.
                    if client.is_closed() {
                        let mut delay_ms: u64 = 500;
                        for attempt in 1..=6u32 {
                            match connect_biz_pg(&dsn_owned) {
                                Ok(mut c) => match c.batch_execute(PG_SCHEMA) {
                                    Ok(()) => {
                                        client = c;
                                        log::info!(
                                            "gr_biz pg reconnected after connection loss (attempt {attempt})"
                                        );
                                        break;
                                    }
                                    Err(e) => {
                                        log::warn!(
                                            "gr_biz pg reconnect schema failed ({label_t}) attempt {attempt}/6: {e}"
                                        );
                                    }
                                },
                                Err(e) => {
                                    log::warn!(
                                        "gr_biz pg reconnect failed ({label_t}) attempt {attempt}/6: {e}"
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
            .map_err(|e| e.to_string())?;
        // Wait for schema via a ping job
        let (done_tx, done_rx) = mpsc::sync_channel(1);
        tx.send(Box::new(move |_c| {
            let _ = done_tx.send(());
        }))
        .map_err(|e| e.to_string())?;
        let _ = done_rx.recv_timeout(std::time::Duration::from_secs(10));
        Ok(Self {
            backend: Backend::Postgres {
                jobs: tx,
                label: label.clone(),
            },
            path_or_label: label,
        })
    }

    pub fn backend_name(&self) -> &'static str {
        match &self.backend {
            Backend::Sqlite(_) => "sqlite",
            Backend::Postgres { .. } => "postgres",
        }
    }

    pub fn upsert_visit(&self, u: &VisitUpsert) -> Result<Value, String> {
        let facet = normalize_facet(&u.visitor_facet);
        let site_id = u.site_id.trim();
        let vt = u.visitor_terminal_id.trim();
        if site_id.is_empty() || vt.is_empty() {
            return Err("site_id_and_visitor_terminal_id_required".into());
        }
        let now = now_ms();
        let summary = u.summary.to_string();
        let session_id = u.session_id.trim();

        match &self.backend {
            Backend::Sqlite(m) => {
                let c = m.lock().map_err(|e| e.to_string())?;
                let existing: Option<(String, i64, String)> = if !session_id.is_empty() {
                    c.query_row(
                        "SELECT visit_id, created_ms, visitor_facet FROM biz_visits WHERE site_id=?1 AND session_id=?2",
                        params![site_id, session_id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()
                    .map_err(|e| e.to_string())?
                } else {
                    None
                };
                let (visit_id, created, facet_out) = if let Some((id, cr, prev_f)) = existing {
                    let facet_w = sticky_write_facet(Some(&prev_f), facet);
                    c.execute(
                        "UPDATE biz_visits SET visitor_terminal_id=?1, visitor_facet=?2, page_host=?3,
                         ua_hash=?4, updated_ms=?5, summary_json=?6 WHERE visit_id=?7",
                        params![vt, facet_w, u.page_host, u.ua_hash, now, summary, id],
                    )
                    .map_err(|e| e.to_string())?;
                    (id, cr, facet_w)
                } else {
                    let id = new_visit_id();
                    let facet_w = facet.to_string();
                    c.execute(
                        "INSERT INTO biz_visits(visit_id, site_id, visitor_terminal_id, visitor_facet,
                         session_id, page_host, ua_hash, created_ms, updated_ms, summary_json)
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?8,?9)",
                        params![
                            id,
                            site_id,
                            vt,
                            facet_w,
                            session_id,
                            u.page_host,
                            u.ua_hash,
                            now,
                            summary
                        ],
                    )
                    .map_err(|e| e.to_string())?;
                    (id, now, facet_w)
                };
                if let Some(ev) = u.event.as_ref().filter(|s| !s.is_empty()) {
                    c.execute(
                        "INSERT INTO biz_visit_events(visit_id, site_id, visitor_terminal_id, event, detail_json, ms)
                         VALUES (?1,?2,?3,?4,?5,?6)",
                        params![
                            visit_id,
                            site_id,
                            vt,
                            ev,
                            u.event_detail.to_string(),
                            now
                        ],
                    )
                    .map_err(|e| e.to_string())?;
                }
                Ok(json!({
                    "ok": true,
                    "visit_id": visit_id,
                    "site_id": site_id,
                    "visitor_terminal_id": vt,
                    "visitor_facet": facet_out,
                    "session_id": session_id,
                    "created_ms": created,
                    "updated_ms": now,
                }))
            }
            Backend::Postgres { jobs, .. } => {
                let u = u.clone();
                let site_id = site_id.to_string();
                let vt = vt.to_string();
                let facet = facet.to_string();
                let session_id = session_id.to_string();
                let (tx, rx) = mpsc::sync_channel(1);
                jobs.send(Box::new(move |client| {
                    let res = pg_upsert(client, &u, &site_id, &vt, &facet, &session_id, now, &summary);
                    let _ = tx.send(res);
                }))
                .map_err(|e| e.to_string())?;
                rx.recv().map_err(|e| e.to_string())?
            }
        }
    }

    pub fn facet_stats(&self, site_id: Option<&str>, limit_scan: usize) -> Result<Value, String> {
        let limit = limit_scan.clamp(10, 50_000) as i64;
        match &self.backend {
            Backend::Sqlite(m) => {
                let c = m.lock().map_err(|e| e.to_string())?;
                // Binary board (new writes only): browser | robots
                let mut browser = 0i64;
                let mut robots = 0i64;
                // Legacy rows (not migrated): js / nojs — shown separately, not mixed into binary
                let mut legacy_js = 0i64;
                let mut legacy_nojs = 0i64;
                let mut unknown = 0i64;
                let sql = if site_id.map(|s| !s.is_empty()).unwrap_or(false) {
                    "SELECT visitor_facet, COUNT(*) FROM (
                       SELECT visitor_facet FROM biz_visits WHERE site_id=?1
                       ORDER BY updated_ms DESC LIMIT ?2
                     ) GROUP BY visitor_facet"
                } else {
                    "SELECT visitor_facet, COUNT(*) FROM (
                       SELECT visitor_facet FROM biz_visits
                       ORDER BY updated_ms DESC LIMIT ?1
                     ) GROUP BY visitor_facet"
                };
                if let Some(sid) = site_id.filter(|s| !s.is_empty()) {
                    let mut stmt = c.prepare(sql).map_err(|e| e.to_string())?;
                    let rows = stmt
                        .query_map(params![sid, limit], |r| {
                            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                        })
                        .map_err(|e| e.to_string())?;
                    for row in rows {
                        let (f, n) = row.map_err(|e| e.to_string())?;
                        match f.as_str() {
                            "browser" => browser += n,
                            "robots" => robots += n,
                            "js" => legacy_js += n,
                            "nojs" => legacy_nojs += n,
                            _ => unknown += n,
                        }
                    }
                } else {
                    let mut stmt = c.prepare(sql).map_err(|e| e.to_string())?;
                    let rows = stmt
                        .query_map(params![limit], |r| {
                            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                        })
                        .map_err(|e| e.to_string())?;
                    for row in rows {
                        let (f, n) = row.map_err(|e| e.to_string())?;
                        match f.as_str() {
                            "browser" => browser += n,
                            "robots" => robots += n,
                            "js" => legacy_js += n,
                            "nojs" => legacy_nojs += n,
                            _ => unknown += n,
                        }
                    }
                }
                let binary_total = browser + robots;
                let scanned = binary_total + legacy_js + legacy_nojs + unknown;
                let robots_by_name = sqlite_robots_by_name(&c, site_id, limit)?;
                Ok(json!({
                    "ok": true,
                    "source": "gr_biz",
                    "backend": "sqlite",
                    "taxonomy": "browser_robots_v2",
                    "note": "binary board counts only new visitor_facet=browser|robots; historical js/nojs not remapped",
                    "site_id": site_id,
                    "facets": {
                        "browser": browser,
                        "robots": robots,
                        "total": binary_total,
                    },
                    "legacy_facets": {
                        "js": legacy_js,
                        "nojs": legacy_nojs,
                        "unknown": unknown,
                    },
                    "robots_by_name": robots_by_name,
                    "scanned": scanned,
                }))
            }
            Backend::Postgres { jobs, .. } => {
                let sid = site_id.map(|s| s.to_string());
                let (tx, rx) = mpsc::sync_channel(1);
                jobs.send(Box::new(move |client| {
                    let res = pg_facet_stats(client, sid.as_deref(), limit);
                    let _ = tx.send(res);
                }))
                .map_err(|e| e.to_string())?;
                rx.recv().map_err(|e| e.to_string())?
            }
        }
    }

    pub fn list_visits(
        &self,
        site_id: Option<&str>,
        facet: Option<&str>,
        q: Option<&str>,
        limit: usize,
    ) -> Result<Value, String> {
        let limit = limit.clamp(1, 200) as i64;
        match &self.backend {
            Backend::Sqlite(m) => {
                let c = m.lock().map_err(|e| e.to_string())?;
                let mut sql = String::from(
                    "SELECT visit_id, site_id, visitor_terminal_id, visitor_facet, session_id,
                            page_host, ua_hash, created_ms, updated_ms, summary_json
                     FROM biz_visits WHERE 1=1",
                );
                let mut args: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
                if let Some(s) = site_id.filter(|s| !s.is_empty()) {
                    sql.push_str(" AND site_id=?");
                    args.push(Box::new(s.to_string()));
                }
                if let Some(f) = facet.filter(|s| !s.is_empty()) {
                    sql.push_str(" AND visitor_facet=?");
                    args.push(Box::new(f.to_string()));
                }
                if let Some(q) = q.filter(|s| !s.is_empty()) {
                    sql.push_str(" AND (visitor_terminal_id LIKE ? OR session_id LIKE ? OR page_host LIKE ? OR visit_id LIKE ?)");
                    let like = format!("%{q}%");
                    args.push(Box::new(like.clone()));
                    args.push(Box::new(like.clone()));
                    args.push(Box::new(like.clone()));
                    args.push(Box::new(like));
                }
                sql.push_str(" ORDER BY updated_ms DESC LIMIT ?");
                args.push(Box::new(limit));
                let mut stmt = c.prepare(&sql).map_err(|e| e.to_string())?;
                let params_ref: Vec<&dyn rusqlite::types::ToSql> =
                    args.iter().map(|a| a.as_ref()).collect();
                let rows = stmt
                    .query_map(params_ref.as_slice(), |r| {
                        let summary: String = r.get(9)?;
                        Ok(json!({
                            "visit_id": r.get::<_, String>(0)?,
                            "site_id": r.get::<_, String>(1)?,
                            "visitor_terminal_id": r.get::<_, String>(2)?,
                            "visitor_facet": r.get::<_, String>(3)?,
                            "session_id": r.get::<_, String>(4)?,
                            "page_host": r.get::<_, String>(5)?,
                            "ua_hash": r.get::<_, String>(6)?,
                            "created_ms": r.get::<_, i64>(7)?,
                            "updated_ms": r.get::<_, i64>(8)?,
                            "summary": serde_json::from_str::<Value>(&summary).unwrap_or(json!({})),
                        }))
                    })
                    .map_err(|e| e.to_string())?;
                let visits: Vec<Value> = rows
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?;
                let n = visits.len();
                let arr = Value::Array(visits);
                Ok(json!({
                    "ok": true,
                    "source": "gr_biz",
                    "visits": arr.clone(),
                    "sessions": arr,
                    "count": n,
                }))
            }
            Backend::Postgres { jobs, .. } => {
                let sid = site_id.map(|s| s.to_string());
                let fac = facet.map(|s| s.to_string());
                let qq = q.map(|s| s.to_string());
                let (tx, rx) = mpsc::sync_channel(1);
                jobs.send(Box::new(move |client| {
                    let res = pg_list_visits(client, sid.as_deref(), fac.as_deref(), qq.as_deref(), limit);
                    let _ = tx.send(res);
                }))
                .map_err(|e| e.to_string())?;
                rx.recv().map_err(|e| e.to_string())?
            }
        }
    }
}

/// Normalize **writes** to binary taxonomy only (browser | robots).
/// Legacy js/nojs are mapped to browser so new upserts never store nojs/js.
fn normalize_facet(f: &str) -> &str {
    match f {
        "robots" => "robots",
        "browser" | "js" | "nojs" => "browser",
        _ => "browser",
    }
}

/// Sticky when updating an existing visit: robots never demotes.
fn sticky_write_facet(existing: Option<&str>, incoming: &str) -> String {
    crate::admin::facet::sticky_facet(existing, incoming)
}

fn redact_dsn(dsn: &str) -> String {
    if let Some(at) = dsn.find('@') {
        format!("postgres://***@{}", &dsn[at + 1..])
    } else {
        "postgres://***".into()
    }
}

fn pg_upsert(
    client: &mut Client,
    u: &VisitUpsert,
    site_id: &str,
    vt: &str,
    facet: &str,
    session_id: &str,
    now: i64,
    summary: &str,
) -> Result<Value, String> {
    let existing = if !session_id.is_empty() {
        client
            .query_opt(
                "SELECT visit_id, created_ms, visitor_facet FROM biz_visits WHERE site_id=$1 AND session_id=$2",
                &[&site_id, &session_id],
            )
            .map_err(|e| e.to_string())?
    } else {
        None
    };
    let (visit_id, created, facet_out) = if let Some(row) = existing {
        let id: String = row.get(0);
        let cr: i64 = row.get(1);
        let prev_f: String = row.get(2);
        let facet_w = sticky_write_facet(Some(&prev_f), facet);
        client
            .execute(
                "UPDATE biz_visits SET visitor_terminal_id=$1, visitor_facet=$2, page_host=$3,
                 ua_hash=$4, updated_ms=$5, summary_json=$6 WHERE visit_id=$7",
                &[
                    &vt,
                    &facet_w,
                    &u.page_host,
                    &u.ua_hash,
                    &now,
                    &summary,
                    &id,
                ],
            )
            .map_err(|e| e.to_string())?;
        (id, cr, facet_w)
    } else {
        let id = new_visit_id();
        let facet_w = facet.to_string();
        client
            .execute(
                "INSERT INTO biz_visits(visit_id, site_id, visitor_terminal_id, visitor_facet,
                 session_id, page_host, ua_hash, created_ms, updated_ms, summary_json)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$8,$9)",
                &[
                    &id,
                    &site_id,
                    &vt,
                    &facet_w,
                    &session_id,
                    &u.page_host,
                    &u.ua_hash,
                    &now,
                    &summary,
                ],
            )
            .map_err(|e| e.to_string())?;
        (id, now, facet_w)
    };
    if let Some(ev) = u.event.as_ref().filter(|s| !s.is_empty()) {
        let detail = u.event_detail.to_string();
        client
            .execute(
                "INSERT INTO biz_visit_events(visit_id, site_id, visitor_terminal_id, event, detail_json, ms)
                 VALUES ($1,$2,$3,$4,$5,$6)",
                &[&visit_id, &site_id, &vt, ev, &detail, &now],
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(json!({
        "ok": true,
        "visit_id": visit_id,
        "site_id": site_id,
        "visitor_terminal_id": vt,
        "visitor_facet": facet_out,
        "session_id": session_id,
        "created_ms": created,
        "updated_ms": now,
    }))
}

fn robot_name_from_summary(summary: &str) -> String {
    if let Ok(v) = serde_json::from_str::<Value>(summary) {
        if let Some(n) = v.get("robot_name").and_then(|x| x.as_str()) {
            if !n.is_empty() {
                return n.to_string();
            }
        }
    }
    "crawler".into()
}

fn sqlite_robots_by_name(
    c: &rusqlite::Connection,
    site_id: Option<&str>,
    limit: i64,
) -> Result<Value, String> {
    use std::collections::BTreeMap;
    let mut map: BTreeMap<String, i64> = BTreeMap::new();
    let sql = if site_id.map(|s| !s.is_empty()).unwrap_or(false) {
        "SELECT summary_json FROM biz_visits WHERE site_id=?1 AND visitor_facet='robots' ORDER BY updated_ms DESC LIMIT ?2"
    } else {
        "SELECT summary_json FROM biz_visits WHERE visitor_facet='robots' ORDER BY updated_ms DESC LIMIT ?1"
    };
    if let Some(sid) = site_id.filter(|s| !s.is_empty()) {
        let mut stmt = c.prepare(sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![sid, limit], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        for row in rows {
            let s = row.map_err(|e| e.to_string())?;
            *map.entry(robot_name_from_summary(&s)).or_default() += 1;
        }
    } else {
        let mut stmt = c.prepare(sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![limit], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        for row in rows {
            let s = row.map_err(|e| e.to_string())?;
            *map.entry(robot_name_from_summary(&s)).or_default() += 1;
        }
    }
    Ok(json!(map))
}

fn pg_facet_stats(client: &mut Client, site_id: Option<&str>, limit: i64) -> Result<Value, String> {
    let rows = if let Some(sid) = site_id.filter(|s| !s.is_empty()) {
        client
            .query(
                "SELECT visitor_facet, COUNT(*) FROM (
                   SELECT visitor_facet FROM biz_visits WHERE site_id=$1
                   ORDER BY updated_ms DESC LIMIT $2
                 ) t GROUP BY visitor_facet",
                &[&sid, &limit],
            )
            .map_err(|e| e.to_string())?
    } else {
        client
            .query(
                "SELECT visitor_facet, COUNT(*) FROM (
                   SELECT visitor_facet FROM biz_visits
                   ORDER BY updated_ms DESC LIMIT $1
                 ) t GROUP BY visitor_facet",
                &[&limit],
            )
            .map_err(|e| e.to_string())?
    };
    let mut browser = 0i64;
    let mut robots = 0i64;
    let mut legacy_js = 0i64;
    let mut legacy_nojs = 0i64;
    let mut unknown = 0i64;
    for r in rows {
        let f: String = r.get(0);
        let n: i64 = r.get(1);
        match f.as_str() {
            "browser" => browser += n,
            "robots" => robots += n,
            "js" => legacy_js += n,
            "nojs" => legacy_nojs += n,
            _ => unknown += n,
        }
    }
    // robots_by_name from summary_json.robot_name
    let name_rows = if let Some(sid) = site_id.filter(|s| !s.is_empty()) {
        client
            .query(
                "SELECT COALESCE(summary_json::json->>'robot_name', 'crawler') AS n, COUNT(*)
                 FROM (
                   SELECT summary_json FROM biz_visits
                   WHERE site_id=$1 AND visitor_facet='robots'
                   ORDER BY updated_ms DESC LIMIT $2
                 ) t GROUP BY 1",
                &[&sid, &limit],
            )
            .map_err(|e| e.to_string())?
    } else {
        client
            .query(
                "SELECT COALESCE(summary_json::json->>'robot_name', 'crawler') AS n, COUNT(*)
                 FROM (
                   SELECT summary_json FROM biz_visits
                   WHERE visitor_facet='robots'
                   ORDER BY updated_ms DESC LIMIT $1
                 ) t GROUP BY 1",
                &[&limit],
            )
            .map_err(|e| e.to_string())?
    };
    let mut robots_by_name = serde_json::Map::new();
    for r in name_rows {
        let n: String = r.get(0);
        let c: i64 = r.get(1);
        robots_by_name.insert(n, json!(c));
    }
    let binary_total = browser + robots;
    let scanned = binary_total + legacy_js + legacy_nojs + unknown;
    Ok(json!({
        "ok": true,
        "source": "gr_biz",
        "backend": "postgres",
        "taxonomy": "browser_robots_v2",
        "note": "binary board counts only new visitor_facet=browser|robots; historical js/nojs not remapped",
        "site_id": site_id,
        "facets": {
            "browser": browser,
            "robots": robots,
            "total": binary_total,
        },
        "legacy_facets": {
            "js": legacy_js,
            "nojs": legacy_nojs,
            "unknown": unknown,
        },
        "robots_by_name": robots_by_name,
        "scanned": scanned,
    }))
}

fn pg_list_visits(
    client: &mut Client,
    site_id: Option<&str>,
    facet: Option<&str>,
    q: Option<&str>,
    limit: i64,
) -> Result<Value, String> {
    // Build simple filtered query
    let mut sql = String::from(
        "SELECT visit_id, site_id, visitor_terminal_id, visitor_facet, session_id,
                page_host, ua_hash, created_ms, updated_ms, summary_json
         FROM biz_visits WHERE 1=1",
    );
    let mut idx = 1;
    let mut binds: Vec<Box<dyn postgres::types::ToSql + Sync + Send>> = Vec::new();
    if let Some(s) = site_id.filter(|s| !s.is_empty()) {
        sql.push_str(&format!(" AND site_id=${idx}"));
        binds.push(Box::new(s.to_string()));
        idx += 1;
    }
    if let Some(f) = facet.filter(|s| !s.is_empty()) {
        sql.push_str(&format!(" AND visitor_facet=${idx}"));
        binds.push(Box::new(f.to_string()));
        idx += 1;
    }
    if let Some(qq) = q.filter(|s| !s.is_empty()) {
        let like = format!("%{qq}%");
        sql.push_str(&format!(
            " AND (visitor_terminal_id LIKE ${a} OR session_id LIKE ${b} OR page_host LIKE ${c} OR visit_id LIKE ${d})",
            a = idx,
            b = idx + 1,
            c = idx + 2,
            d = idx + 3,
        ));
        binds.push(Box::new(like.clone()));
        binds.push(Box::new(like.clone()));
        binds.push(Box::new(like.clone()));
        binds.push(Box::new(like));
        idx += 4;
    }
    sql.push_str(&format!(" ORDER BY updated_ms DESC LIMIT ${idx}"));
    binds.push(Box::new(limit));

    let params: Vec<&(dyn postgres::types::ToSql + Sync)> =
        binds.iter().map(|b| b.as_ref() as _).collect();
    let rows = client.query(&sql, &params).map_err(|e| e.to_string())?;
    let mut visits = Vec::new();
    for r in rows {
        let summary: String = r.get(9);
        visits.push(json!({
            "visit_id": r.get::<_, String>(0),
            "site_id": r.get::<_, String>(1),
            "visitor_terminal_id": r.get::<_, String>(2),
            "visitor_facet": r.get::<_, String>(3),
            "session_id": r.get::<_, String>(4),
            "page_host": r.get::<_, String>(5),
            "ua_hash": r.get::<_, String>(6),
            "created_ms": r.get::<_, i64>(7),
            "updated_ms": r.get::<_, i64>(8),
            "summary": serde_json::from_str::<Value>(&summary).unwrap_or(json!({})),
        }));
    }
    let n = visits.len();
    Ok(json!({
        "ok": true,
        "source": "gr_biz",
        "visits": visits.clone(),
        "sessions": visits,
        "count": n,
    }))
}

/// Helper used by handlers: best-effort biz upsert (never fails probe path).
pub fn try_record_visit(store: &Option<std::sync::Arc<BizStore>>, u: VisitUpsert) {
    let Some(s) = store else { return };
    if let Err(e) = s.upsert_visit(&u) {
        log::debug!("biz_visit upsert skipped: {e}");
    }
}

#[allow(dead_code)]
pub fn default_biz_path(admin_data_dir: &Path) -> PathBuf {
    admin_data_dir.join("gr_biz.sqlite")
}
