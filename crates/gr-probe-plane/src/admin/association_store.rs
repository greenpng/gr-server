//! Tenant association store — separate from probe evidence DB (iss/48).
//!
//! Backend: PostgreSQL via `GR_ASSOCIATION_DATABASE_URL` / `GR_ASSOCIATION_DATABASE_URL`.

use postgres::{Client, Config, NoTls};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, SyncSender};
use std::sync::Mutex;
use std::thread;

fn connect_assoc_pg(dsn: &str) -> Result<Client, String> {
    let mut cfg: Config = dsn
        .parse()
        .map_err(|e| format!("gr_assoc dsn parse: {e}"))?;
    cfg.application_name("gr-assoc");
    cfg.connect(NoTls)
        .map_err(|e| format!("gr_assoc connect: {e}"))
}

const SQLITE_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS assoc_events (
  tenant_id TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  event_id TEXT NOT NULL,
  event_type TEXT NOT NULL,
  subject_ref TEXT NOT NULL,
  session_id TEXT,
  visitor_terminal_id TEXT,
  device_id TEXT,
  identity_state TEXT,
  payload_json TEXT NOT NULL,
  occurred_at_ms INTEGER NOT NULL,
  received_at_ms INTEGER NOT NULL,
  PRIMARY KEY (tenant_id, idempotency_key)
);
CREATE INDEX IF NOT EXISTS idx_assoc_subj ON assoc_events(tenant_id, subject_ref, received_at_ms DESC);
CREATE TABLE IF NOT EXISTS assoc_labels (
  tenant_id TEXT NOT NULL,
  label_id TEXT NOT NULL,
  subject_ref TEXT NOT NULL DEFAULT '',
  assessment_id TEXT,
  event_id TEXT,
  outcome TEXT NOT NULL,
  label_source TEXT,
  effective_at_ms INTEGER NOT NULL,
  detail_json TEXT,
  PRIMARY KEY (tenant_id, label_id)
);
CREATE INDEX IF NOT EXISTS idx_assoc_lab_subj ON assoc_labels(tenant_id, subject_ref, effective_at_ms DESC);
CREATE TABLE IF NOT EXISTS assoc_behavior_profiles (
  tenant_id TEXT NOT NULL,
  subject_ref TEXT NOT NULL,
  session_id TEXT NOT NULL DEFAULT '',
  profile_digest TEXT NOT NULL,
  vec32_json TEXT NOT NULL,
  created_ms INTEGER NOT NULL,
  PRIMARY KEY (tenant_id, subject_ref, session_id)
);
CREATE INDEX IF NOT EXISTS idx_assoc_beh_subj ON assoc_behavior_profiles(tenant_id, subject_ref, created_ms DESC);
"#;

const PG_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS assoc_events (
  tenant_id TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  event_id TEXT NOT NULL,
  event_type TEXT NOT NULL,
  subject_ref TEXT NOT NULL,
  session_id TEXT,
  visitor_terminal_id TEXT,
  device_id TEXT,
  identity_state TEXT,
  payload_json TEXT NOT NULL,
  occurred_at_ms BIGINT NOT NULL,
  received_at_ms BIGINT NOT NULL,
  PRIMARY KEY (tenant_id, idempotency_key)
);
CREATE INDEX IF NOT EXISTS idx_assoc_events_subject
  ON assoc_events (tenant_id, subject_ref, occurred_at_ms DESC);
CREATE TABLE IF NOT EXISTS assoc_labels (
  tenant_id TEXT NOT NULL,
  label_id TEXT NOT NULL,
  subject_ref TEXT NOT NULL DEFAULT '',
  assessment_id TEXT,
  event_id TEXT,
  outcome TEXT NOT NULL,
  label_source TEXT,
  effective_at_ms BIGINT NOT NULL,
  detail_json TEXT,
  PRIMARY KEY (tenant_id, label_id)
);
CREATE INDEX IF NOT EXISTS idx_assoc_labels_subject
  ON assoc_labels (tenant_id, subject_ref, effective_at_ms DESC);
CREATE TABLE IF NOT EXISTS assoc_behavior_profiles (
  tenant_id TEXT NOT NULL,
  subject_ref TEXT NOT NULL,
  session_id TEXT NOT NULL DEFAULT '',
  profile_digest TEXT NOT NULL,
  vec32_json TEXT NOT NULL,
  created_ms BIGINT NOT NULL,
  PRIMARY KEY (tenant_id, subject_ref, session_id)
);
CREATE INDEX IF NOT EXISTS idx_assoc_beh_subj
  ON assoc_behavior_profiles (tenant_id, subject_ref, created_ms DESC);
"#;

enum Backend {
    Sqlite { path: PathBuf, conn: Mutex<Connection> },
    Postgres { jobs: SyncSender<JobFn>, label: String },
}

type JobFn = Box<dyn FnOnce(&mut Client) + Send>;

pub struct AssociationStore {
    backend: Backend,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn redact_dsn(dsn: &str) -> String {
    if let Some(at) = dsn.find('@') {
        format!("postgres://***@{}", &dsn[at + 1..])
    } else {
        "postgres://***".into()
    }
}

impl AssociationStore {
    /// Open: PostgreSQL via `GR_ASSOCIATION_DATABASE_URL` / `GR_ASSOCIATION_DATABASE_URL`.
    pub fn open(_dir: &Path) -> Result<Self, String> {
        let dsn = gr_abi::env::get("ASSOCIATION_DATABASE_URL")
             .or_else(|| gr_abi::env::get("ASSOCIATION_DATABASE_URL"))
            .unwrap_or_default();
        let dsn = dsn.trim().to_string();
        if dsn.is_empty() {
            return Err(
                "GR_ASSOCIATION_DATABASE_URL is required; SQLite association store was removed"
                    .into(),
            );
        }
        Self::open_postgres(&dsn)
    }

    pub fn open_sqlite(dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let path = dir.join("gr_association.sqlite");
        let conn = Connection::open(&path).map_err(|e| e.to_string())?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA busy_timeout=5000;",
        )
        .map_err(|e| e.to_string())?;
        conn.execute_batch(SQLITE_DDL).map_err(|e| e.to_string())?;
        let _ = conn.execute(
            "ALTER TABLE assoc_labels ADD COLUMN subject_ref TEXT NOT NULL DEFAULT ''",
            [],
        );
        Ok(Self {
            backend: Backend::Sqlite {
                path,
                conn: Mutex::new(conn),
            },
        })
    }

    pub fn open_postgres(dsn: &str) -> Result<Self, String> {
        let dsn_owned = dsn.to_string();
        let label = redact_dsn(dsn);
        let (tx, rx) = mpsc::sync_channel::<JobFn>(128);
        let label_t = label.clone();
        thread::Builder::new()
            .name("gr-assoc-pg".into())
            .spawn(move || {
                // iss/audit STO-02 (scenario B): bootstrap retry (connect + DDL)
                // so a fresh-PG entrypoint restart window no longer kills this
                // thread on the first attempt (hub open retries on final
                // failure — unchanged semantics, quiet exit, zero panics).
                let mut client = None;
                for attempt in 1..=3u32 {
                    match connect_assoc_pg(&dsn_owned) {
                        Ok(mut c) => match c.batch_execute(PG_DDL) {
                            Ok(()) => {
                                client = Some(c);
                                break;
                            }
                            Err(e) => {
                                log::warn!(
                                    "gr_assoc pg schema failed ({label_t}) attempt {attempt}/3: {e}"
                                );
                            }
                        },
                        Err(e) => {
                            log::warn!(
                                "gr_assoc pg connect failed ({label_t}) attempt {attempt}/3: {e}"
                            );
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
                let mut client = match client {
                    Some(c) => c,
                    None => {
                        log::warn!(
                            "gr_assoc pg bootstrap failed after 3 attempts ({label_t}); hub open will retry"
                        );
                        return;
                    }
                };
                while let Ok(job) = rx.recv() {
                    // iss/audit STO-02 (scenario A): runtime self-heal — a PG
                    // restart/failover used to leave every assoc job erroring
                    // on a dead socket forever.
                    if client.is_closed() {
                        let mut delay_ms: u64 = 500;
                        for attempt in 1..=6u32 {
                            match connect_assoc_pg(&dsn_owned) {
                                Ok(mut c) => match c.batch_execute(PG_DDL) {
                                    Ok(()) => {
                                        client = c;
                                        log::info!(
                                            "gr_assoc pg reconnected after connection loss (attempt {attempt})"
                                        );
                                        break;
                                    }
                                    Err(e) => {
                                        log::warn!(
                                            "gr_assoc pg reconnect schema failed ({label_t}) attempt {attempt}/6: {e}"
                                        );
                                    }
                                },
                                Err(e) => {
                                    log::warn!(
                                        "gr_assoc pg reconnect failed ({label_t}) attempt {attempt}/6: {e}"
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
        let (done_tx, done_rx) = mpsc::sync_channel(1);
        tx.send(Box::new(move |_c| {
            let _ = done_tx.send(());
        }))
        .map_err(|e| e.to_string())?;
        done_rx
            .recv_timeout(std::time::Duration::from_secs(15))
            .map_err(|_| "assoc pg schema timeout".to_string())?;
        Ok(Self {
            backend: Backend::Postgres {
                jobs: tx,
                label: label.clone(),
            },
        })
    }

    pub fn path_label(&self) -> String {
        match &self.backend {
            Backend::Sqlite { path, .. } => format!("{} (sqlite_wal)", path.display()),
            Backend::Postgres { label, .. } => format!("{label} (postgres)"),
        }
    }

    pub fn backend_status(&self) -> Value {
        match &self.backend {
            Backend::Sqlite { path, .. } => json!({
                "ok": true,
                "backend": "sqlite_wal",
                "path": path.display().to_string(),
                "journal": "wal",
                "multi_process_same_host": true,
                "multi_node_ha": false,
                "multi_region_ha": false,
                "note": "SQLite WAL same-host multi-worker; set GR_ASSOCIATION_DATABASE_URL for PG multi-node",
            }),
            Backend::Postgres { label, .. } => json!({
                "ok": true,
                "backend": "postgres",
                "path": label,
                "journal": "pg_wal",
                "multi_process_same_host": true,
                "multi_node_ha": true,
                "multi_region_ha": false,
                "note": "Postgres association store active (lab/prod multi-node shared DB); multi-region still ops topology",
            }),
        }
    }

    fn with_sqlite<R>(&self, f: impl FnOnce(&Connection) -> Result<R, String>) -> Result<R, String> {
        match &self.backend {
            Backend::Sqlite { conn, .. } => {
                let g = conn.lock().map_err(|e| e.to_string())?;
                f(&g)
            }
            Backend::Postgres { .. } => Err("not_sqlite".into()),
        }
    }

    fn with_pg<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Client) -> Result<R, String> + Send + 'static,
    ) -> Result<R, String> {
        match &self.backend {
            Backend::Postgres { jobs, .. } => {
                let (tx, rx) = mpsc::sync_channel(1);
                jobs.send(Box::new(move |c| {
                    let r = f(c);
                    let _ = tx.send(r);
                }))
                .map_err(|e| e.to_string())?;
                rx.recv_timeout(std::time::Duration::from_secs(30))
                    .map_err(|_| "assoc pg job timeout".to_string())?
            }
            Backend::Sqlite { .. } => Err("not_pg".into()),
        }
    }

    pub fn observe(
        &self,
        tenant_id: &str,
        idempotency_key: &str,
        event_id: &str,
        event_type: &str,
        subject_ref: &str,
        session_id: Option<&str>,
        visitor_terminal_id: Option<&str>,
        device_id: Option<&str>,
        identity_state: Option<&str>,
        payload: &Value,
        occurred_at_ms: i64,
    ) -> Result<Value, String> {
        let now = now_ms();
        let sid = session_id.unwrap_or("").to_string();
        let vt = visitor_terminal_id.unwrap_or("").to_string();
        let did = device_id.unwrap_or("").to_string();
        let idst = identity_state.unwrap_or("").to_string();
        let payload_s = payload.to_string();
        let tenant = tenant_id.to_string();
        let idem = idempotency_key.to_string();
        let eid = event_id.to_string();
        let et = event_type.to_string();
        let subj = subject_ref.to_string();

        match &self.backend {
            Backend::Sqlite { .. } => self.with_sqlite(|g| {
                let existing: Option<String> = g
                    .query_row(
                        "SELECT event_id FROM assoc_events WHERE tenant_id=?1 AND idempotency_key=?2",
                        params![tenant, idem],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(|e| e.to_string())?;
                if let Some(ex) = existing {
                    return Ok(json!({
                        "ok": true, "deduped": true, "event_id": ex, "tenant_id": tenant,
                    }));
                }
                g.execute(
                    "INSERT INTO assoc_events(
                        tenant_id, idempotency_key, event_id, event_type, subject_ref,
                        session_id, visitor_terminal_id, device_id, identity_state,
                        payload_json, occurred_at_ms, received_at_ms
                     ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                    params![
                        tenant, idem, eid, et, subj, sid, vt, did, idst, payload_s, occurred_at_ms, now
                    ],
                )
                .map_err(|e| e.to_string())?;
                Ok(json!({
                    "ok": true, "deduped": false, "event_id": event_id,
                    "tenant_id": tenant_id, "received_at_ms": now,
                }))
            }),
            Backend::Postgres { .. } => self.with_pg(move |c| {
                let row = c
                    .query_opt(
                        "SELECT event_id FROM assoc_events WHERE tenant_id=$1 AND idempotency_key=$2",
                        &[&tenant, &idem],
                    )
                    .map_err(|e| e.to_string())?;
                if let Some(r) = row {
                    let ex: String = r.get(0);
                    return Ok(json!({
                        "ok": true, "deduped": true, "event_id": ex, "tenant_id": tenant,
                    }));
                }
                c.execute(
                    "INSERT INTO assoc_events(
                        tenant_id, idempotency_key, event_id, event_type, subject_ref,
                        session_id, visitor_terminal_id, device_id, identity_state,
                        payload_json, occurred_at_ms, received_at_ms
                     ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
                    &[
                        &tenant,
                        &idem,
                        &eid,
                        &et,
                        &subj,
                        &sid,
                        &vt,
                        &did,
                        &idst,
                        &payload_s,
                        &occurred_at_ms,
                        &now,
                    ],
                )
                .map_err(|e| e.to_string())?;
                Ok(json!({
                    "ok": true, "deduped": false, "event_id": eid,
                    "tenant_id": tenant, "received_at_ms": now,
                }))
            }),
        }
    }

    pub fn list_events_for_subject(
        &self,
        tenant_id: &str,
        subject_ref: &str,
        limit: i64,
    ) -> Result<Vec<Value>, String> {
        let tenant = tenant_id.to_string();
        let subj = subject_ref.to_string();
        match &self.backend {
            Backend::Sqlite { .. } => self.with_sqlite(|g| {
                let mut stmt = g
                    .prepare(
                        "SELECT event_id, event_type, session_id, visitor_terminal_id, device_id,
                                identity_state, payload_json, occurred_at_ms, received_at_ms
                         FROM assoc_events
                         WHERE tenant_id=?1 AND subject_ref=?2
                         ORDER BY received_at_ms DESC LIMIT ?3",
                    )
                    .map_err(|e| e.to_string())?;
                let rows = stmt
                    .query_map(params![tenant, subj, limit], |r| {
                        let payload: String = r.get(6)?;
                        let payload_v: Value =
                            serde_json::from_str(&payload).unwrap_or(json!({}));
                        Ok(json!({
                            "event_id": r.get::<_, String>(0)?,
                            "event_type": r.get::<_, String>(1)?,
                            "session_id": r.get::<_, String>(2)?,
                            "visitor_terminal_id": r.get::<_, String>(3)?,
                            "device_id": r.get::<_, String>(4)?,
                            "identity_state": r.get::<_, String>(5)?,
                            "payload": payload_v,
                            "probe_context": {
                                "session_id": r.get::<_, String>(2)?,
                                "visitor_terminal_id": r.get::<_, String>(3)?,
                                "device_id": r.get::<_, String>(4)?,
                                "identity_state": r.get::<_, String>(5)?,
                            },
                            "occurred_at_ms": r.get::<_, i64>(7)?,
                            "received_at_ms": r.get::<_, i64>(8)?,
                        }))
                    })
                    .map_err(|e| e.to_string())?;
                let mut out = Vec::new();
                for row in rows {
                    out.push(row.map_err(|e| e.to_string())?);
                }
                Ok(out)
            }),
            Backend::Postgres { .. } => self.with_pg(move |c| {
                let rows = c
                    .query(
                        "SELECT event_id, event_type, session_id, visitor_terminal_id, device_id,
                                identity_state, payload_json, occurred_at_ms, received_at_ms
                         FROM assoc_events
                         WHERE tenant_id=$1 AND subject_ref=$2
                         ORDER BY received_at_ms DESC LIMIT $3",
                        &[&tenant, &subj, &limit],
                    )
                    .map_err(|e| e.to_string())?;
                let mut out = Vec::new();
                for r in rows {
                    let payload: String = r.get(6);
                    let payload_v: Value =
                        serde_json::from_str(&payload).unwrap_or(json!({}));
                    let sid: String = r.get(2);
                    let vt: String = r.get(3);
                    let did: String = r.get(4);
                    let idst: String = r.get(5);
                    out.push(json!({
                        "event_id": r.get::<_, String>(0),
                        "event_type": r.get::<_, String>(1),
                        "session_id": sid.clone(),
                        "visitor_terminal_id": vt.clone(),
                        "device_id": did.clone(),
                        "identity_state": idst.clone(),
                        "payload": payload_v,
                        "probe_context": {
                            "session_id": sid,
                            "visitor_terminal_id": vt,
                            "device_id": did,
                            "identity_state": idst,
                        },
                        "occurred_at_ms": r.get::<_, i64>(7),
                        "received_at_ms": r.get::<_, i64>(8),
                    }));
                }
                Ok(out)
            }),
        }
    }

    pub fn label(
        &self,
        tenant_id: &str,
        label_id: &str,
        subject_ref: &str,
        outcome: &str,
        assessment_id: Option<&str>,
        event_id: Option<&str>,
        label_source: Option<&str>,
        detail: &Value,
    ) -> Result<Value, String> {
        let now = now_ms();
        let tenant = tenant_id.to_string();
        let lid = label_id.to_string();
        let subj = subject_ref.to_string();
        let outc = outcome.to_string();
        let aid = assessment_id.unwrap_or("").to_string();
        let eid = event_id.unwrap_or("").to_string();
        let src = label_source.unwrap_or("customer_rule").to_string();
        let detail_s = detail.to_string();

        match &self.backend {
            Backend::Sqlite { .. } => self.with_sqlite(|g| {
                g.execute(
                    "INSERT OR REPLACE INTO assoc_labels(
                        tenant_id, label_id, subject_ref, assessment_id, event_id, outcome, label_source,
                        effective_at_ms, detail_json
                     ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                    params![tenant, lid, subj, aid, eid, outc, src, now, detail_s],
                )
                .map_err(|e| e.to_string())?;
                Ok(json!({
                    "ok": true, "label_id": label_id, "tenant_id": tenant_id,
                    "subject_ref": subject_ref, "outcome": outcome, "effective_at_ms": now,
                }))
            }),
            Backend::Postgres { .. } => self.with_pg(move |c| {
                c.execute(
                    "INSERT INTO assoc_labels(
                        tenant_id, label_id, subject_ref, assessment_id, event_id, outcome, label_source,
                        effective_at_ms, detail_json
                     ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
                     ON CONFLICT (tenant_id, label_id) DO UPDATE SET
                        subject_ref=EXCLUDED.subject_ref,
                        assessment_id=EXCLUDED.assessment_id,
                        event_id=EXCLUDED.event_id,
                        outcome=EXCLUDED.outcome,
                        label_source=EXCLUDED.label_source,
                        effective_at_ms=EXCLUDED.effective_at_ms,
                        detail_json=EXCLUDED.detail_json",
                    &[&tenant, &lid, &subj, &aid, &eid, &outc, &src, &now, &detail_s],
                )
                .map_err(|e| e.to_string())?;
                Ok(json!({
                    "ok": true, "label_id": lid, "tenant_id": tenant,
                    "subject_ref": subj, "outcome": outc, "effective_at_ms": now,
                }))
            }),
        }
    }

    pub fn list_labels_for_subject(
        &self,
        tenant_id: &str,
        subject_ref: &str,
        limit: i64,
    ) -> Result<Vec<Value>, String> {
        let tenant = tenant_id.to_string();
        let subj = subject_ref.to_string();
        match &self.backend {
            Backend::Sqlite { .. } => self.with_sqlite(|g| {
                let mut stmt = g
                    .prepare(
                        "SELECT label_id, subject_ref, assessment_id, event_id, outcome, label_source,
                                effective_at_ms, detail_json
                         FROM assoc_labels
                         WHERE tenant_id=?1 AND subject_ref=?2
                         ORDER BY effective_at_ms DESC LIMIT ?3",
                    )
                    .map_err(|e| e.to_string())?;
                let rows = stmt
                    .query_map(params![tenant, subj, limit], |r| {
                        let detail: String = r.get(7)?;
                        Ok(json!({
                            "label_id": r.get::<_, String>(0)?,
                            "subject_ref": r.get::<_, String>(1)?,
                            "assessment_id": r.get::<_, String>(2)?,
                            "event_id": r.get::<_, String>(3)?,
                            "outcome": r.get::<_, String>(4)?,
                            "label_source": r.get::<_, String>(5)?,
                            "effective_at_ms": r.get::<_, i64>(6)?,
                            "detail": serde_json::from_str::<Value>(&detail).unwrap_or(json!({})),
                        }))
                    })
                    .map_err(|e| e.to_string())?;
                let mut out = Vec::new();
                for row in rows {
                    out.push(row.map_err(|e| e.to_string())?);
                }
                Ok(out)
            }),
            Backend::Postgres { .. } => self.with_pg(move |c| {
                let rows = c
                    .query(
                        "SELECT label_id, subject_ref, assessment_id, event_id, outcome, label_source,
                                effective_at_ms, COALESCE(detail_json, '{}')
                         FROM assoc_labels
                         WHERE tenant_id=$1 AND subject_ref=$2
                         ORDER BY effective_at_ms DESC LIMIT $3",
                        &[&tenant, &subj, &limit],
                    )
                    .map_err(|e| e.to_string())?;
                let mut out = Vec::new();
                for r in rows {
                    let detail: String = r.get(7);
                    out.push(json!({
                        "label_id": r.get::<_, String>(0),
                        "subject_ref": r.get::<_, String>(1),
                        "assessment_id": r.get::<_, String>(2),
                        "event_id": r.get::<_, String>(3),
                        "outcome": r.get::<_, String>(4),
                        "label_source": r.get::<_, String>(5),
                        "effective_at_ms": r.get::<_, i64>(6),
                        "detail": serde_json::from_str::<Value>(&detail).unwrap_or(json!({})),
                    }));
                }
                Ok(out)
            }),
        }
    }

    /// Tenant-wide labels for ECE (lab / ops calibration).
    pub fn list_labels(&self, tenant_id: &str, limit: i64) -> Result<Vec<Value>, String> {
        let tenant = tenant_id.to_string();
        match &self.backend {
            Backend::Sqlite { .. } => self.with_sqlite(|g| {
                let mut stmt = g
                    .prepare(
                        "SELECT label_id, subject_ref, assessment_id, event_id, outcome, label_source,
                                effective_at_ms, detail_json
                         FROM assoc_labels WHERE tenant_id=?1 ORDER BY effective_at_ms DESC LIMIT ?2",
                    )
                    .map_err(|e| e.to_string())?;
                let rows = stmt
                    .query_map(params![tenant, limit], |r| {
                        let detail: String = r.get(7)?;
                        Ok(json!({
                            "label_id": r.get::<_, String>(0)?,
                            "subject_ref": r.get::<_, String>(1)?,
                            "assessment_id": r.get::<_, String>(2)?,
                            "event_id": r.get::<_, String>(3)?,
                            "outcome": r.get::<_, String>(4)?,
                            "label_source": r.get::<_, String>(5)?,
                            "effective_at_ms": r.get::<_, i64>(6)?,
                            "detail": serde_json::from_str::<Value>(&detail).unwrap_or(json!({})),
                        }))
                    })
                    .map_err(|e| e.to_string())?;
                let mut out = Vec::new();
                for row in rows {
                    out.push(row.map_err(|e| e.to_string())?);
                }
                Ok(out)
            }),
            Backend::Postgres { .. } => self.with_pg(move |c| {
                let rows = c
                    .query(
                        "SELECT label_id, subject_ref, assessment_id, event_id, outcome, label_source,
                                effective_at_ms, COALESCE(detail_json, '{}')
                         FROM assoc_labels WHERE tenant_id=$1 ORDER BY effective_at_ms DESC LIMIT $2",
                        &[&tenant, &limit],
                    )
                    .map_err(|e| e.to_string())?;
                let mut out = Vec::new();
                for r in rows {
                    let detail: String = r.get(7);
                    out.push(json!({
                        "label_id": r.get::<_, String>(0),
                        "subject_ref": r.get::<_, String>(1),
                        "assessment_id": r.get::<_, String>(2),
                        "event_id": r.get::<_, String>(3),
                        "outcome": r.get::<_, String>(4),
                        "label_source": r.get::<_, String>(5),
                        "effective_at_ms": r.get::<_, i64>(6),
                        "detail": serde_json::from_str::<Value>(&detail).unwrap_or(json!({})),
                    }));
                }
                Ok(out)
            }),
        }
    }

    /// Persist behavior_self_sim vec32 for subject (iss/50 R7 storage).
    pub fn put_behavior_profile(
        &self,
        tenant_id: &str,
        subject_ref: &str,
        session_id: &str,
        profile: &Value,
    ) -> Result<Value, String> {
        let now = now_ms();
        let digest = profile
            .get("profile_digest")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let vec32 = profile
            .get("vec32")
            .cloned()
            .unwrap_or(json!([]))
            .to_string();
        let tenant = tenant_id.to_string();
        let subj = subject_ref.to_string();
        let sid = session_id.to_string();

        match &self.backend {
            Backend::Sqlite { .. } => self.with_sqlite(|g| {
                g.execute(
                    "INSERT OR REPLACE INTO assoc_behavior_profiles(
                        tenant_id, subject_ref, session_id, profile_digest, vec32_json, created_ms
                     ) VALUES (?1,?2,?3,?4,?5,?6)",
                    params![tenant, subj, sid, digest, vec32, now],
                )
                .map_err(|e| e.to_string())?;
                Ok(json!({"ok": true, "stored": true, "profile_digest": profile.get("profile_digest")}))
            }),
            Backend::Postgres { .. } => self.with_pg(move |c| {
                c.execute(
                    "INSERT INTO assoc_behavior_profiles(
                        tenant_id, subject_ref, session_id, profile_digest, vec32_json, created_ms
                     ) VALUES ($1,$2,$3,$4,$5,$6)
                     ON CONFLICT (tenant_id, subject_ref, session_id) DO UPDATE SET
                        profile_digest=EXCLUDED.profile_digest,
                        vec32_json=EXCLUDED.vec32_json,
                        created_ms=EXCLUDED.created_ms",
                    &[&tenant, &subj, &sid, &digest, &vec32, &now],
                )
                .map_err(|e| e.to_string())?;
                Ok(json!({"ok": true, "stored": true, "profile_digest": digest}))
            }),
        }
    }

    pub fn list_behavior_profiles(
        &self,
        tenant_id: &str,
        subject_ref: &str,
        limit: i64,
    ) -> Result<Vec<Value>, String> {
        let tenant = tenant_id.to_string();
        let subj = subject_ref.to_string();
        match &self.backend {
            Backend::Sqlite { .. } => self.with_sqlite(|g| {
                let mut stmt = g
                    .prepare(
                        "SELECT session_id, profile_digest, vec32_json, created_ms
                         FROM assoc_behavior_profiles
                         WHERE tenant_id=?1 AND subject_ref=?2
                         ORDER BY created_ms DESC LIMIT ?3",
                    )
                    .map_err(|e| e.to_string())?;
                let rows = stmt
                    .query_map(params![tenant, subj, limit], |r| {
                        let v: String = r.get(2)?;
                        Ok(json!({
                            "session_id": r.get::<_, String>(0)?,
                            "profile_digest": r.get::<_, String>(1)?,
                            "vec32": serde_json::from_str::<Value>(&v).unwrap_or(json!([])),
                            "created_ms": r.get::<_, i64>(3)?,
                        }))
                    })
                    .map_err(|e| e.to_string())?;
                let mut out = Vec::new();
                for row in rows {
                    out.push(row.map_err(|e| e.to_string())?);
                }
                Ok(out)
            }),
            Backend::Postgres { .. } => self.with_pg(move |c| {
                let rows = c
                    .query(
                        "SELECT session_id, profile_digest, vec32_json, created_ms
                         FROM assoc_behavior_profiles
                         WHERE tenant_id=$1 AND subject_ref=$2
                         ORDER BY created_ms DESC LIMIT $3",
                        &[&tenant, &subj, &limit],
                    )
                    .map_err(|e| e.to_string())?;
                let mut out = Vec::new();
                for r in rows {
                    let v: String = r.get(2);
                    out.push(json!({
                        "session_id": r.get::<_, String>(0),
                        "profile_digest": r.get::<_, String>(1),
                        "vec32": serde_json::from_str::<Value>(&v).unwrap_or(json!([])),
                        "created_ms": r.get::<_, i64>(3),
                    }));
                }
                Ok(out)
            }),
        }
    }

    /// Compare latest profiles for a subject (self-similarity cosine).
    pub fn compare_behavior_profiles(
        &self,
        tenant_id: &str,
        subject_ref: &str,
    ) -> Result<Value, String> {
        let profiles = self.list_behavior_profiles(tenant_id, subject_ref, 20)?;
        if profiles.len() < 2 {
            return Ok(json!({
                "ok": true,
                "n_profiles": profiles.len(),
                "self_sim": null,
                "note": "need >=2 sessions for self-sim",
            }));
        }
        let mut cosines = Vec::new();
        for i in 0..profiles.len() {
            for j in (i + 1)..profiles.len() {
                let a = profiles[i]
                    .get("vec32")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_f64())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let b = profiles[j]
                    .get("vec32")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_f64())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                cosines.push(gr_probe_core::cosine_vec32(&a, &b));
            }
        }
        cosines.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = cosines.len() as f64;
        let mean = if n > 0.0 {
            cosines.iter().sum::<f64>() / n
        } else {
            0.0
        };
        let p50 = cosines[(cosines.len() / 2).min(cosines.len().saturating_sub(1))];
        let p95 = cosines[((cosines.len() as f64 * 0.95) as usize).min(cosines.len().saturating_sub(1))];
        Ok(json!({
            "ok": true,
            "algo": "behavior_self_sim_window_v1",
            "n_profiles": profiles.len(),
            "n_pairs": cosines.len(),
            "behavior_self_sim_mean": (mean * 10000.0).round() / 10000.0,
            "behavior_self_sim_p50": (p50 * 10000.0).round() / 10000.0,
            "behavior_self_sim_p95": (p95 * 10000.0).round() / 10000.0,
            "promote_to_commercial_id": false,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observe_idempotent_and_assess_tenant_local() {
        let dir = std::env::temp_dir().join(format!(
            "gr_assoc_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = AssociationStore::open_sqlite(&dir).unwrap();
        let body = json!({"event_type": "auth.login_succeeded"});
        let r1 = store
            .observe(
                "site_a",
                "idem-1",
                "ev1",
                "auth.login_succeeded",
                "sub_v1_abc",
                Some("cycle_1"),
                Some("vt_1"),
                Some("dv0-aaaaaaaaaa-bbbbbbbbbb-cccccccccc-dddddddddd-0-0-0-0-0-0"),
                Some("linkable"),
                &body,
                1,
            )
            .unwrap();
        assert_eq!(r1["deduped"], false);
        let r2 = store
            .observe(
                "site_a",
                "idem-1",
                "ev1-dup",
                "auth.login_succeeded",
                "sub_v1_abc",
                Some("cycle_1"),
                Some("vt_1"),
                Some("dv0-aaaaaaaaaa-bbbbbbbbbb-cccccccccc-dddddddddd-0-0-0-0-0-0"),
                Some("linkable"),
                &body,
                1,
            )
            .unwrap();
        assert_eq!(r2["deduped"], true);
        assert_eq!(r2["event_id"], "ev1");
        let ev = store.list_events_for_subject("site_a", "sub_v1_abc", 10).unwrap();
        assert_eq!(ev.len(), 1);
        let other = store.list_events_for_subject("site_b", "sub_v1_abc", 10).unwrap();
        assert!(other.is_empty());
    }

    #[test]
    fn labels_isolated_by_subject_ref() {
        let dir = std::env::temp_dir().join(format!(
            "gr_assoc_lab_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = AssociationStore::open_sqlite(&dir).unwrap();
        store
            .label(
                "t1",
                "l1",
                "sub_a",
                "fraud",
                None,
                None,
                Some("lab"),
                &json!({"score": 0.9, "same_host": false}),
            )
            .unwrap();
        store
            .label(
                "t1",
                "l2",
                "sub_b",
                "legit",
                None,
                None,
                Some("lab"),
                &json!({"score": 0.8, "same_host": true}),
            )
            .unwrap();
        let a = store.list_labels_for_subject("t1", "sub_a", 10).unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0]["outcome"], "fraud");
        let all = store.list_labels("t1", 10).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn behavior_profile_self_sim() {
        let dir = std::env::temp_dir().join(format!(
            "gr_assoc_beh_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = AssociationStore::open_sqlite(&dir).unwrap();
        let p1 = gr_probe_core::behavior_profile_vec32(&json!({
            "rpa_features_v2": {"segment":{"sample_quality":"adequate"},
            "features":{"input_mouse_entropy":0.5,"n_events":40,"type_diversity_n":5}}
        }));
        let p2 = gr_probe_core::behavior_profile_vec32(&json!({
            "rpa_features_v2": {"segment":{"sample_quality":"adequate"},
            "features":{"input_mouse_entropy":0.52,"n_events":42,"type_diversity_n":5}}
        }));
        store
            .put_behavior_profile("t", "sub_v1_x", "s1", &p1)
            .unwrap();
        store
            .put_behavior_profile("t", "sub_v1_x", "s2", &p2)
            .unwrap();
        let c = store.compare_behavior_profiles("t", "sub_v1_x").unwrap();
        assert_eq!(c["ok"], true);
        assert!(c["behavior_self_sim_p50"].as_f64().unwrap() > 0.5);
    }
}
