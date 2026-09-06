//! PostgreSQL backend for multi-worker green-v5 (W1+).
//!
//! The sync `postgres` crate owns an internal Tokio runtime. Calling it from
//! inside `#[tokio::main]` panics ("Cannot start a runtime from within a runtime").
//! All Client I/O therefore runs on a dedicated OS thread via a job channel.

use crate::{
    cycle_status_from_meta, hex_now, now_ms, AnalyzeDueMerge, StoreError, ANALYZE_DEBOUNCE_MS,
    ANALYZE_IDLE_IMMINENT_MS, ANALYZE_LOCK_MS, cycle_cool_ms, cycle_cool_ms_for_site,
    cycle_incomplete_ms,
    SESSION_HARD_MAX_MS, SESSION_INACTIVITY_MS,
};
use postgres::{Client, Config, NoTls};
use serde_json::{json, Map, Value};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;

type JobFn = Box<dyn FnOnce(&mut Client) + Send>;

fn connect_store_pg(dsn: &str) -> Result<Client, String> {
    let mut cfg: Config = dsn
        .parse()
        .map_err(|e| format!("postgres dsn parse: {e}"))?;
    // Visible in pg_stat_activity.application_name for multi-worker budgeting.
    let app = gr_abi::env::get("PG_APP_NAME").unwrap_or_else(|| "gr-store".into());
    cfg.application_name(&app);
    let mut c = cfg
        .connect(NoTls)
        .map_err(|e| format!("postgres connect: {e}"))?;
    // P1-3 (iss/grok4.6/05): runtime migration NOTICEs flooded node logs
    // (~3.4k lines). Session-level SET (startup `options` interacts badly
    // with some AdminHub boot paths) keeps NOTICE noise off node logs.
    let _ = c.batch_execute("SET client_min_messages = warning");
    Ok(c)
}

/// iss/opus5 04-P0-1: connection pool size. The previous design ran the whole
/// process on ONE postgres connection (single `gr-pg` thread), serializing
/// every HTTP handler, analyze worker and retention task behind one socket.
/// Jobs are still closures over `&mut Client`; each job executes start-to-finish
/// on one pooled connection, so session-scoped advisory locks and multi-
/// statement transactions keep their semantics. Per-caller ordering is
/// preserved because `run()` blocks until the job completes.
fn pg_pool_size() -> usize {
    let from_env = gr_abi::env::get("PG_POOL_SIZE")
        .and_then(|s| s.trim().parse::<usize>().ok());
    match from_env {
        Some(n) => n.clamp(1, 64),
        None => {
            // Default 8 (doc: 8–16 starting point); never exceed 2×CPU and
            // stay well under PG's default max_connections=100 for multi-node.
            let cpu = thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4);
            (cpu.min(8)).clamp(2, 16)
        }
    }
}

pub struct PgStore {
    jobs: Vec<SyncSender<JobFn>>,
    next: std::sync::atomic::AtomicUsize,
    dsn_label: String,
}

/// iss/opus5 04-P0-2: `probe_cold` (fields_json + payload_z) is the single
/// source of truth; `probe_batches.payload_json` is an uncompressed debug
/// copy of the same envelope and is now OFF by default. Set
/// `GR_PROBE_BATCHES_PAYLOAD=1` to keep the raw copy for forensics.
/// The probe_batches ROW (PK + material columns + client_ip) is always
/// written — ingest ACK semantics (was_insert/same_capture/stale-gen) key
/// off the material columns, not the payload text.
fn probe_batches_payload_enabled() -> bool {
    gr_abi::env::get("PROBE_BATCHES_PAYLOAD")
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "on" | "yes"))
        .unwrap_or(false)
}

/// iss/opus5 04-P0-2: observation_events keeps the pointer columns plus a
/// slim summary (validated + lineage); the full claimed/validated envelope
/// copy is debug-only (`GR_OBSERVATION_ENVELOPE_FULL=1`). The only reader
/// (ops replay) uses the row count, not the envelope body. Shared with the
/// SQLite backend (crate root).
pub(crate) fn observation_envelope_full() -> bool {
    gr_abi::env::get("OBSERVATION_ENVELOPE_FULL")
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "on" | "yes"))
        .unwrap_or(false)
}

/// Reduce an observation envelope to its durable summary: validated identity
/// + lineage hash. Drops the unvalidated client `claimed` block and fields
/// already carried by table columns (session/batch/source/ids).
pub(crate) fn slim_observation_envelope(env: &Value) -> Value {
    if observation_envelope_full() {
        return env.clone();
    }
    json!({
        "schema": env.get("schema").cloned().unwrap_or(Value::Null),
        "observation_id": env.get("observation_id").cloned().unwrap_or(Value::Null),
        "validated": env.get("validated").cloned().unwrap_or(json!({})),
        "lineage": env.get("lineage").cloned().unwrap_or(json!({})),
        "stamped_at_ms": env.get("stamped_at_ms").cloned().unwrap_or(Value::Null),
    })
}

const PG_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS sessions (
  session_id TEXT PRIMARY KEY,
  visitor_terminal_id TEXT,
  created_ms BIGINT NOT NULL,
  updated_ms BIGINT NOT NULL,
  meta_json TEXT NOT NULL DEFAULT '{}'
);
CREATE TABLE IF NOT EXISTS probe_batches (
  session_id TEXT NOT NULL REFERENCES sessions(session_id),
  batch_id TEXT NOT NULL,
  source TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  created_ms BIGINT NOT NULL,
  client_ip TEXT,
  -- iss/72 material identity
  material_generation BIGINT NOT NULL DEFAULT 1,
  material_hash TEXT,
  capture_id TEXT,
  PRIMARY KEY (session_id, batch_id, source)
);
CREATE TABLE IF NOT EXISTS observation_events (
  observation_id TEXT PRIMARY KEY,
  tenant_id TEXT NOT NULL DEFAULT '',
  session_id TEXT NOT NULL,
  batch_id TEXT NOT NULL,
  source TEXT NOT NULL DEFAULT '',
  source_kind TEXT NOT NULL DEFAULT '',
  realm_kind TEXT NOT NULL DEFAULT '',
  probe_method_id TEXT NOT NULL DEFAULT '',
  capture_id TEXT,
  attempt_id TEXT,
  envelope_json TEXT NOT NULL,
  created_ms BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_obs_session ON observation_events(session_id, created_ms);
CREATE TABLE IF NOT EXISTS api_idempotency (
  tenant_id TEXT NOT NULL DEFAULT '',
  route TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  body_hash TEXT NOT NULL,
  status INTEGER NOT NULL,
  response_json TEXT NOT NULL,
  created_ms BIGINT NOT NULL,
  PRIMARY KEY (tenant_id, route, idempotency_key)
);
-- Shared multi-node rate-limit quota: one row per (site:route, minute window).
-- Atomic UPSERT bumps count; window change resets count to 1. Old windows purged lazily.
CREATE TABLE IF NOT EXISTS probe_rate_limit_windows (
  k TEXT PRIMARY KEY,
  window_ms BIGINT NOT NULL,
  count BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_rl_windows ON probe_rate_limit_windows(window_ms);
CREATE TABLE IF NOT EXISTS analysis_results (
  session_id TEXT NOT NULL REFERENCES sessions(session_id),
  rev BIGINT NOT NULL,
  result_json TEXT NOT NULL,
  created_ms BIGINT NOT NULL,
  -- Scalar projections for ops reports (avoid scanning huge result_json TOAST)
  real_band TEXT,
  device_id TEXT,
  bot_verdict TEXT,
  device_confidence DOUBLE PRECISION,
  client_ip TEXT,
  device_tier TEXT,
  collision_risk BOOLEAN,
  product_version TEXT,
  digest_path TEXT,
  residual_entropy_ok BOOLEAN,
  PRIMARY KEY (session_id, rev)
);
-- Compact queryable cold probe store (deflated full payload BYTEA + jsonb fields)
CREATE TABLE IF NOT EXISTS probe_cold (
  id BIGSERIAL PRIMARY KEY,
  session_id TEXT NOT NULL,
  visitor_terminal_id TEXT NOT NULL DEFAULT '',
  batch_id TEXT NOT NULL,
  source TEXT NOT NULL DEFAULT '',
  client_ip TEXT,
  fields_json JSONB NOT NULL DEFAULT '{}'::jsonb,
  payload_z BYTEA NOT NULL DEFAULT '\x'::bytea,
  created_ms BIGINT NOT NULL,
  UNIQUE (session_id, batch_id, source)
);
CREATE INDEX IF NOT EXISTS idx_probe_cold_session ON probe_cold(session_id);
CREATE INDEX IF NOT EXISTS idx_probe_cold_vt ON probe_cold(visitor_terminal_id);
CREATE TABLE IF NOT EXISTS analyze_jobs (
  session_id TEXT PRIMARY KEY REFERENCES sessions(session_id),
  due_ms BIGINT NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending',
  locked_until BIGINT NOT NULL DEFAULT 0,
  locked_by TEXT NOT NULL DEFAULT '',
  updated_ms BIGINT NOT NULL
);
CREATE TABLE IF NOT EXISTS page_results (
  session_id TEXT NOT NULL REFERENCES sessions(session_id),
  page_id TEXT NOT NULL,
  page_rev BIGINT NOT NULL,
  result_json TEXT NOT NULL,
  created_ms BIGINT NOT NULL,
  PRIMARY KEY (session_id, page_id, page_rev)
);
CREATE INDEX IF NOT EXISTS idx_batches_session ON probe_batches(session_id);
CREATE INDEX IF NOT EXISTS idx_analysis_session ON analysis_results(session_id);
CREATE INDEX IF NOT EXISTS idx_analyze_jobs_due ON analyze_jobs(status, due_ms, locked_until);
CREATE INDEX IF NOT EXISTS idx_page_results_session ON page_results(session_id, page_id);
CREATE INDEX IF NOT EXISTS idx_sessions_vt ON sessions(visitor_terminal_id);
CREATE INDEX IF NOT EXISTS idx_sessions_created_ms ON sessions(created_ms);
CREATE INDEX IF NOT EXISTS idx_batches_created_ms ON probe_batches(created_ms);
CREATE INDEX IF NOT EXISTS idx_analysis_created_ms ON analysis_results(created_ms);
CREATE INDEX IF NOT EXISTS idx_analysis_device_id ON analysis_results(device_id)
  WHERE device_id IS NOT NULL AND device_id <> '';
CREATE TABLE IF NOT EXISTS visitor_terminals (
  vt_id TEXT PRIMARY KEY,
  last_complete_ms BIGINT NOT NULL DEFAULT 0,
  cool_until_ms BIGINT NOT NULL DEFAULT 0,
  active_cycle_id TEXT NOT NULL DEFAULT '',
  updated_ms BIGINT NOT NULL,
  meta_json TEXT NOT NULL DEFAULT '{}',
  product_version_last TEXT NOT NULL DEFAULT ''
);
-- iss/39–40 local: DeviceIndex tables (same shape as sqlite skeleton; not multi-region prod SLA)
CREATE TABLE IF NOT EXISTS device_index_devices (
  tenant_id TEXT NOT NULL,
  device_id TEXT NOT NULL,
  binder_obs_json TEXT NOT NULL,
  updated_ms BIGINT NOT NULL,
  PRIMARY KEY (tenant_id, device_id)
);
CREATE TABLE IF NOT EXISTS device_index_keys (
  tenant_id TEXT NOT NULL,
  binder_key TEXT NOT NULL,
  device_id TEXT NOT NULL,
  PRIMARY KEY (tenant_id, binder_key, device_id)
);
CREATE INDEX IF NOT EXISTS idx_device_index_keys_dev ON device_index_keys(tenant_id, device_id);
CREATE INDEX IF NOT EXISTS idx_device_index_keys_binder ON device_index_keys(tenant_id, binder_key);
-- Soft edge store (multi-worker PG)
CREATE TABLE IF NOT EXISTS soft_edges (
  tenant_id TEXT NOT NULL,
  a_session TEXT NOT NULL,
  b_session TEXT NOT NULL,
  priority TEXT NOT NULL DEFAULT 'p1',
  confidence DOUBLE PRECISION NOT NULL DEFAULT 0,
  reason TEXT NOT NULL DEFAULT '',
  created_ms BIGINT NOT NULL,
  PRIMARY KEY (tenant_id, a_session, b_session)
);
CREATE TABLE IF NOT EXISTS soft_heat (
  tenant_id TEXT NOT NULL,
  device_id TEXT NOT NULL,
  session_id TEXT NOT NULL,
  first_ms BIGINT NOT NULL,
  last_ms BIGINT NOT NULL,
  PRIMARY KEY (tenant_id, device_id, session_id)
);
CREATE INDEX IF NOT EXISTS idx_soft_heat_device ON soft_heat(tenant_id, device_id);
-- P0: one row per session — primary filter path (no result_json TOAST)
CREATE TABLE IF NOT EXISTS analysis_latest (
  session_id TEXT PRIMARY KEY REFERENCES sessions(session_id) ON DELETE CASCADE,
  rev BIGINT NOT NULL,
  created_ms BIGINT NOT NULL,
  device_id TEXT,
  device_tier TEXT,
  device_prefix TEXT,
  collision_risk BOOLEAN,
  residual_entropy_ok BOOLEAN,
  real_band TEXT,
  bot_verdict TEXT,
  device_confidence DOUBLE PRECISION,
  client_ip TEXT,
  digest_path TEXT,
  product_version TEXT,
  visitor_terminal_id TEXT,
  site_id TEXT,
  inject_path TEXT,
  association_level TEXT,
  authenticity_band TEXT,
  form_class TEXT,
  os_family TEXT,
  platform TEXT,
  os_score DOUBLE PRECISION,
  br_score DOUBLE PRECISION,
  rpa_score DOUBLE PRECISION,
  os_status TEXT,
  br_status TEXT,
  rpa_status TEXT,
  country TEXT,
  asn TEXT,
  residual_algo TEXT,
  has_webrtc_host BOOLEAN,
  mint_residual_ok BOOLEAN,
  mint_host_ok BOOLEAN,
  mint_silicon_ok BOOLEAN,
  mint_conflict_pressure DOUBLE PRECISION,
  mint_single_source_pressure DOUBLE PRECISION,
  mint_ok_keys_n INTEGER,
  mint_conf_only_keys_n INTEGER,
  mint_conflict_keys_n INTEGER,
  mint_gate_summary TEXT
);
CREATE INDEX IF NOT EXISTS idx_al_device ON analysis_latest(device_id)
  WHERE device_id IS NOT NULL AND device_id <> '';
CREATE INDEX IF NOT EXISTS idx_al_tier_created ON analysis_latest(device_tier, created_ms DESC);
CREATE INDEX IF NOT EXISTS idx_al_ip_created ON analysis_latest(client_ip, created_ms DESC)
  WHERE client_ip IS NOT NULL AND client_ip <> '';
CREATE INDEX IF NOT EXISTS idx_al_vt_created ON analysis_latest(visitor_terminal_id, created_ms DESC)
  WHERE visitor_terminal_id IS NOT NULL AND visitor_terminal_id <> '';
CREATE INDEX IF NOT EXISTS idx_al_created ON analysis_latest(created_ms DESC);
CREATE INDEX IF NOT EXISTS idx_al_prefix_created ON analysis_latest(device_prefix, created_ms DESC)
  WHERE device_prefix IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_al_collision ON analysis_latest(collision_risk, device_tier)
  WHERE collision_risk IS TRUE;
CREATE INDEX IF NOT EXISTS idx_al_site ON analysis_latest(site_id, created_ms DESC)
  WHERE site_id IS NOT NULL AND site_id <> '';
-- P1: commercial device master + session membership
CREATE TABLE IF NOT EXISTS devices (
  device_id TEXT PRIMARY KEY,
  first_seen_ms BIGINT NOT NULL,
  last_seen_ms BIGINT NOT NULL,
  tier_last TEXT,
  collision_risk_last BOOLEAN,
  residual_entropy_ok_last BOOLEAN,
  session_count BIGINT NOT NULL DEFAULT 0,
  distinct_ip_count BIGINT NOT NULL DEFAULT 0,
  distinct_vt_count BIGINT NOT NULL DEFAULT 0,
  last_client_ip TEXT,
  last_session_id TEXT,
  product_version_last TEXT,
  digest_path_last TEXT,
  meta_json JSONB NOT NULL DEFAULT '{}'::jsonb
);
CREATE INDEX IF NOT EXISTS idx_devices_last_seen ON devices(last_seen_ms DESC);
CREATE INDEX IF NOT EXISTS idx_devices_tier ON devices(tier_last, last_seen_ms DESC);
CREATE TABLE IF NOT EXISTS device_sessions (
  device_id TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
  session_id TEXT NOT NULL,
  created_ms BIGINT NOT NULL,
  client_ip TEXT,
  visitor_terminal_id TEXT,
  device_tier TEXT,
  PRIMARY KEY (device_id, session_id)
);
CREATE INDEX IF NOT EXISTS idx_device_sessions_sid ON device_sessions(session_id);
CREATE INDEX IF NOT EXISTS idx_device_sessions_created ON device_sessions(created_ms DESC);
"#;

impl PgStore {
    pub fn open(dsn: &str) -> Result<Self, StoreError> {
        let dsn_label = redact_dsn(dsn);
        let pool = pg_pool_size();
        let mut senders: Vec<SyncSender<JobFn>> = Vec::with_capacity(pool);

        for idx in 0..pool {
            let dsn_owned = dsn.to_string();
            let (jobs_tx, jobs_rx): (SyncSender<JobFn>, Receiver<JobFn>) = mpsc::sync_channel(64);
            let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();

            thread::Builder::new()
                .name(format!("gr-pg-{idx}"))
                .spawn(move || {
                    let mut client = match connect_store_pg(&dsn_owned) {
                        Ok(c) => c,
                        Err(e) => {
                            let _ = ready_tx.send(Err(e));
                            return;
                        }
                    };
                    // Serialize multi-process schema init (CREATE IF NOT EXISTS
                    // races under load). Worker 0 owns schema init; the rest
                    // connect after it completes (open() waits for worker 0
                    // before spawning them, see below).
                    if idx == 0 {
                        if let Err(e) = ensure_schema(&mut client) {
                            let _ = ready_tx.send(Err(e));
                            return;
                        }
                    }
                    let _ = ready_tx.send(Ok(()));
                    while let Ok(job) = jobs_rx.recv() {
                        job(&mut client);
                    }
                })
                .map_err(|e| StoreError::Msg(format!("pg worker thread: {e}")))?;

            ready_rx
                .recv()
                .map_err(|e| StoreError::Msg(format!("pg worker ready: {e}")))?
                .map_err(StoreError::Msg)?;
            senders.push(jobs_tx);
        }

        Ok(Self {
            jobs: senders,
            next: std::sync::atomic::AtomicUsize::new(0),
            dsn_label,
        })
    }

    pub fn label(&self) -> &str {
        &self.dsn_label
    }

    fn run<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Client) -> Result<R, StoreError> + Send + 'static,
    ) -> Result<R, StoreError> {
        let (tx, rx) = mpsc::channel();
        // Round-robin dispatch across pooled connections; the blocking send
        // preserves backpressure (per-worker queue depth 64).
        let idx = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % self.jobs.len();
        self.jobs[idx]
            .send(Box::new(move |client| {
                let _ = tx.send(f(client));
            }))
            .map_err(|e| StoreError::Msg(format!("pg job send: {e}")))?;
        rx.recv()
            .map_err(|e| StoreError::Msg(format!("pg job recv: {e}")))?
    }

    pub fn open_session(
        &self,
        session_id: Option<String>,
        visitor_terminal_id: Option<String>,
        meta: Option<Value>,
    ) -> Result<Value, StoreError> {
        self.run(move |c| open_session_inner(c, session_id, visitor_terminal_id, meta))
    }

    pub fn open_cycle(
        &self,
        cycle_id_hint: Option<String>,
        visitor_terminal_id: Option<String>,
        meta: Option<Value>,
    ) -> Result<Value, StoreError> {
        self.run(move |c| open_cycle_inner(c, cycle_id_hint, visitor_terminal_id, meta))
    }

    pub fn complete_cycle(&self, cycle_id: &str) -> Result<Value, StoreError> {
        let cycle_id = cycle_id.to_string();
        self.run(move |c| complete_cycle_inner(c, &cycle_id))
    }


    pub fn insert_ops_client_event(&self, row: Value) -> Result<Value, StoreError> {
        self.run(move |c| insert_ops_client_event_inner(c, row))
    }

    pub fn insert_ops_server_event(&self, row: Value) -> Result<Value, StoreError> {
        self.run(move |c| insert_ops_server_event_inner(c, row))
    }

    pub fn insert_observation_event(&self, row: Value) -> Result<Value, StoreError> {
        self.run(move |c| insert_observation_event_inner(c, row))
    }

    pub fn lookup_api_idempotency(
        &self,
        tenant_id: &str,
        route: &str,
        idempotency_key: &str,
    ) -> Result<Option<Value>, StoreError> {
        let tenant_id = tenant_id.to_string();
        let route = route.to_string();
        let idempotency_key = idempotency_key.to_string();
        self.run(move |c| lookup_api_idempotency_inner(c, &tenant_id, &route, &idempotency_key))
    }

    pub fn put_api_idempotency(
        &self,
        tenant_id: &str,
        route: &str,
        idempotency_key: &str,
        body_hash: &str,
        status: i64,
        response: &Value,
    ) -> Result<Value, StoreError> {
        let tenant_id = tenant_id.to_string();
        let route = route.to_string();
        let idempotency_key = idempotency_key.to_string();
        let body_hash = body_hash.to_string();
        let response = response.clone();
        self.run(move |c| {
            put_api_idempotency_inner(
                c,
                &tenant_id,
                &route,
                &idempotency_key,
                &body_hash,
                status,
                &response,
            )
        })
    }

    /// Shared per-minute rate-limit counter across all nodes. Returns the new
    /// count for `(key, window_ms)` after an atomic UPSERT; window change resets
    /// to 1. Old windows are purged lazily with every bump.
    pub fn bump_rate_limit_window(
        &self,
        key: &str,
        window_ms: i64,
    ) -> Result<i64, StoreError> {
        let key = key.to_string();
        self.run(move |c| bump_rate_limit_window_inner(c, &key, window_ms))
    }

    pub fn session_meta(&self, session_id: &str) -> Result<Option<Value>, StoreError> {
        let session_id = session_id.to_string();
        self.run(move |c| session_meta_inner(c, &session_id))
    }

    pub fn list_observation_events(
        &self,
        session_id: &str,
        limit: i64,
    ) -> Result<Vec<Value>, StoreError> {
        let session_id = session_id.to_string();
        self.run(move |c| list_observation_events_inner(c, &session_id, limit))
    }

    pub fn list_ops_events(
        &self,
        source: &str,
        limit: i64,
        code: Option<String>,
        site_id: Option<String>,
        since_ms: Option<i64>,
        product_version: Option<String>,
    ) -> Result<Value, StoreError> {
        let source = source.to_string();
        self.run(move |c| {
            list_ops_events_inner(c, &source, limit, code, site_id, since_ms, product_version)
        })
    }

    pub fn ops_b10_health(&self, since_ms: i64, limit_sites: i64) -> Result<Value, StoreError> {
        self.run(move |c| ops_b10_health_inner(c, since_ms, limit_sites))
    }

    pub fn ops_probe_completeness(&self, since_ms: i64) -> Result<Value, StoreError> {
        self.run(move |c| ops_probe_completeness_inner(c, since_ms))
    }

    pub fn ops_outcome_distribution(&self, since_ms: i64) -> Result<Value, StoreError> {
        self.run(move |c| ops_outcome_distribution_inner(c, since_ms))
    }

    pub fn velocity_record(
        &self,
        session_id: &str,
        device_id: Option<&str>,
        client_ip: Option<&str>,
        hit_ms: i64,
    ) -> Result<(), StoreError> {
        let session_id = session_id.to_string();
        let device_id = device_id.map(|s| s.to_string());
        let client_ip = client_ip.map(|s| s.to_string());
        self.run(move |c| {
            velocity_record_inner(
                c,
                &session_id,
                device_id.as_deref(),
                client_ip.as_deref(),
                hit_ms,
            )
        })
    }

    pub fn velocity_summary(
        &self,
        device_id: Option<&str>,
        client_ip: Option<&str>,
        now_ms: i64,
    ) -> Result<Value, StoreError> {
        let device_id = device_id.map(|s| s.to_string());
        let client_ip = client_ip.map(|s| s.to_string());
        self.run(move |c| {
            velocity_summary_inner(c, device_id.as_deref(), client_ip.as_deref(), now_ms)
        })
    }

    pub fn purge_cycle_evidence(&self, cycle_id: &str) -> Result<Value, StoreError> {
        let cycle_id = cycle_id.to_string();
        self.run(move |c| purge_cycle_evidence_inner(c, &cycle_id))
    }

    pub fn session_window(&self, session_id: &str) -> Result<Value, StoreError> {
        let session_id = session_id.to_string();
        self.run(move |c| session_window_inner(c, &session_id))
    }

    pub fn require_active_session(&self, session_id: &str) -> Result<Value, StoreError> {
        let w = self.session_window(session_id)?;
        if !w.get("active").and_then(|v| v.as_bool()).unwrap_or(false) {
            let reason = w
                .get("expired_reason")
                .and_then(|v| v.as_str())
                .unwrap_or("expired");
            return Err(StoreError::SessionExpired(format!(
                "{session_id} ({reason})"
            )));
        }
        Ok(w)
    }

    pub fn upsert_batch(
        &self,
        session_id: &str,
        batch_id: &str,
        source: &str,
        payload: &Value,
    ) -> Result<Value, StoreError> {
        self.upsert_batch_with_ip(session_id, batch_id, source, payload, None)
    }

    pub fn upsert_batch_with_ip(
        &self,
        session_id: &str,
        batch_id: &str,
        source: &str,
        payload: &Value,
        client_ip: Option<&str>,
    ) -> Result<Value, StoreError> {
        self.require_active_session(session_id)?;
        let sid = session_id.to_string();
        let batch_id = batch_id.to_string();
        let source = source.to_string();
        let payload = payload.clone();
        let cip = client_ip.map(|s| s.to_string());
        let sid2 = sid.clone();
        let out = self.run(move |c| {
            upsert_batch_inner(c, &sid2, &batch_id, &source, &payload, cip.as_deref())
        })?;
        // Brain-owned analyze arm applied by service after ingest (not per-batch).
        Ok(out)
    }

    /// Cross-filter analysis without scanning full result_json TOAST.
    pub fn cross_query_analysis(
        &self,
        device_id: Option<&str>,
        client_ip: Option<&str>,
        bot_verdict: Option<&str>,
        real_band: Option<&str>,
        field_key: Option<&str>,
        field_value: Option<&str>,
        limit: i64,
    ) -> Result<Value, StoreError> {
        let device_id = device_id.map(|s| s.to_string());
        let client_ip = client_ip.map(|s| s.to_string());
        let bot_verdict = bot_verdict.map(|s| s.to_string());
        let real_band = real_band.map(|s| s.to_string());
        let field_key = field_key.map(|s| s.to_string());
        let field_value = field_value.map(|s| s.to_string());
        self.run(move |c| {
            cross_query_analysis_inner(
                c,
                device_id.as_deref(),
                client_ip.as_deref(),
                bot_verdict.as_deref(),
                real_band.as_deref(),
                field_key.as_deref(),
                field_value.as_deref(),
                limit,
            )
        })
    }

    /// Fetch cold probe row: compact fields + decompressed full payload (lab/ops).
    pub fn get_probe_cold(
        &self,
        session_id: &str,
        batch_id: &str,
        source: Option<&str>,
    ) -> Result<Value, StoreError> {
        let session_id = session_id.to_string();
        let batch_id = batch_id.to_string();
        let source = source.map(|s| s.to_string());
        self.run(move |c| get_probe_cold_inner(c, &session_id, &batch_id, source.as_deref()))
    }

    /// Storage volume snapshot for a session (batch plaintext vs cold compact/z).
    pub fn probe_volume_stats(&self, session_id: &str) -> Result<Value, StoreError> {
        let session_id = session_id.to_string();
        self.run(move |c| probe_volume_stats_inner(c, &session_id))
    }

    pub fn list_cold_for_vt(
        &self,
        visitor_terminal_id: &str,
        since_ms: i64,
        limit: i64,
    ) -> Result<Value, StoreError> {
        let vt = visitor_terminal_id.to_string();
        self.run(move |c| list_cold_for_vt_inner(c, &vt, since_ms, limit))
    }

    pub fn purge_expired_cold(&self, older_than_ms: i64) -> Result<i64, StoreError> {
        self.run(move |c| purge_expired_cold_inner(c, older_than_ms))
    }

    pub fn retention_purge_batch(
        &self,
        older_than_ms: i64,
        limit: i64,
        velocity_older_than_ms: Option<i64>,
        ops_older_than_ms: Option<i64>,
        master_older_than_ms: Option<i64>,
    ) -> Result<Value, StoreError> {
        self.run(move |c| {
            retention_purge_batch_inner(
                c,
                older_than_ms,
                limit,
                velocity_older_than_ms,
                ops_older_than_ms,
                master_older_than_ms,
            )
        })
    }

    /// iss/opus5 05-S-5: DSAR erase — cascade-delete all data for a subject
    /// (visitor_terminal_id | device_id | client_ip | site_id).
    pub fn subject_erase(&self, kind: &str, value: &str) -> Result<Value, StoreError> {
        let kind = kind.to_string();
        let value = value.to_string();
        self.run(move |c| dsar_erase_inner(c, &kind, &value))
    }

    /// iss/opus5 05-S-5: DSAR export — everything held for a subject as JSON.
    pub fn subject_export(&self, kind: &str, value: &str) -> Result<Value, StoreError> {
        let kind = kind.to_string();
        let value = value.to_string();
        self.run(move |c| dsar_export_inner(c, &kind, &value))
    }

    /// iss/opus5 06-P1-5: monthly per-site session counter (billing signal).
    pub fn bump_monthly_sessions(&self, site_id: &str) -> Result<(String, u64), StoreError> {
        let site_id = site_id.to_string();
        self.run(move |c| bump_monthly_sessions_inner(c, &site_id))
    }

    /// iss/opus5 05 low (seal replay ledger).
    pub fn mark_seal_consumed(
        &self,
        session_id: &str,
        batch_id: &str,
        nonce: &str,
    ) -> Result<bool, StoreError> {
        let session_id = session_id.to_string();
        let batch_id = batch_id.to_string();
        let nonce = nonce.to_string();
        self.run(move |c| mark_seal_consumed_inner(c, &session_id, &batch_id, &nonce))
    }

    pub fn schedule_analyze(&self, session_id: &str, debounce_ms: i64) -> Result<(), StoreError> {
        self.schedule_analyze_merge(session_id, debounce_ms, AnalyzeDueMerge::PullEarlier)
    }

    pub fn schedule_analyze_merge(
        &self,
        session_id: &str,
        debounce_ms: i64,
        merge: AnalyzeDueMerge,
    ) -> Result<(), StoreError> {
        let session_id = session_id.to_string();
        self.run(move |c| schedule_analyze_inner(c, &session_id, debounce_ms, merge))
    }

    pub fn claim_due_analyze_jobs(
        &self,
        worker_id: &str,
        limit: usize,
        lock_ms: i64,
    ) -> Result<Vec<String>, StoreError> {
        let worker_id = worker_id.to_string();
        self.run(move |c| claim_due_analyze_jobs_inner(c, &worker_id, limit, lock_ms))
    }

    pub fn complete_analyze_job(&self, session_id: &str, worker_id: &str) -> Result<bool, StoreError> {
        let session_id = session_id.to_string();
        let worker_id = worker_id.to_string();
        self.run(move |c| complete_analyze_job_inner(c, &session_id, &worker_id))
    }

    pub fn pending_analyze_job_count(&self) -> Result<i64, StoreError> {
        self.run(pending_analyze_job_count_inner)
    }

    pub fn analyze_queue_stats(&self) -> Result<Value, StoreError> {
        self.run(analyze_queue_stats_inner)
    }

    pub fn build_evidence(&self, session_id: &str) -> Result<Value, StoreError> {
        let session_id = session_id.to_string();
        self.run(move |c| build_evidence_inner(c, &session_id))
    }

    pub fn save_analysis(&self, session_id: &str, result: &Value) -> Result<i64, StoreError> {
        self.require_active_session(session_id)?;
        let session_id = session_id.to_string();
        let result = result.clone();
        self.run(move |c| save_analysis_inner(c, &session_id, &result))
    }

    pub fn force_session_times(
        &self,
        session_id: &str,
        created_ms: i64,
        updated_ms: i64,
    ) -> Result<(), StoreError> {
        let session_id = session_id.to_string();
        self.run(move |c| force_session_times_inner(c, &session_id, created_ms, updated_ms))
    }

    pub fn latest_analysis(&self, session_id: &str) -> Result<Option<Value>, StoreError> {
        let session_id = session_id.to_string();
        self.run(move |c| latest_analysis_inner(c, &session_id))
    }

    pub fn list_analyses(&self, session_id: &str) -> Result<Vec<Value>, StoreError> {
        let session_id = session_id.to_string();
        self.run(move |c| list_analyses_inner(c, &session_id))
    }

    pub fn save_page_result(
        &self,
        session_id: &str,
        page_id: &str,
        page_rev: i64,
        result: &Value,
    ) -> Result<(), StoreError> {
        let session_id = session_id.to_string();
        let page_id = page_id.to_string();
        let result = result.clone();
        self.run(move |c| {
            let s = serde_json::to_string(&result).map_err(|e| StoreError::Msg(e.to_string()))?;
            let ts = now_ms();
            c.execute(
                "INSERT INTO page_results(session_id, page_id, page_rev, result_json, created_ms)
                 VALUES($1,$2,$3,$4,$5)
                 ON CONFLICT(session_id, page_id, page_rev) DO UPDATE SET
                   result_json=EXCLUDED.result_json,
                   created_ms=EXCLUDED.created_ms",
                &[&session_id, &page_id, &page_rev, &s, &ts],
            )
            .map_err(|e| StoreError::Msg(e.to_string()))?;
            Ok(())
        })
    }

    pub fn latest_page_result(
        &self,
        session_id: &str,
        page_id: &str,
    ) -> Result<Option<Value>, StoreError> {
        let session_id = session_id.to_string();
        let page_id = page_id.to_string();
        self.run(move |c| {
            let row = c.query_opt(
                "SELECT page_rev, result_json, created_ms FROM page_results
                 WHERE session_id=$1 AND page_id=$2 ORDER BY page_rev DESC LIMIT 1",
                &[&session_id, &page_id],
            )
            .map_err(|e| StoreError::Msg(e.to_string()))?;
            match row {
                Some(r) => {
                    let rev: i64 = r.get(0);
                    let js: String = r.get(1);
                    let created_ms: i64 = r.get(2);
                    let mut result: Value =
                        serde_json::from_str(&js).map_err(|e| StoreError::Msg(e.to_string()))?;
                    if let Some(obj) = result.as_object_mut() {
                        obj.insert("page_rev".into(), json!(rev));
                        obj.insert("analyzed_ms".into(), json!(created_ms));
                    }
                    Ok(Some(result))
                }
                None => Ok(None),
            }
        })
    }

    pub fn list_page_results(&self, session_id: &str) -> Result<Vec<Value>, StoreError> {
        let session_id = session_id.to_string();
        self.run(move |c| {
            let rows = c
                .query(
                    "SELECT page_id, page_rev, result_json, created_ms FROM page_results
                     WHERE session_id=$1 ORDER BY created_ms ASC, page_rev ASC",
                    &[&session_id],
                )
                .map_err(|e| StoreError::Msg(e.to_string()))?;
            let mut out = Vec::new();
            for r in rows {
                let page_id: String = r.get(0);
                let rev: i64 = r.get(1);
                let js: String = r.get(2);
                let created_ms: i64 = r.get(3);
                let result: Value =
                    serde_json::from_str(&js).map_err(|e| StoreError::Msg(e.to_string()))?;
                out.push(json!({
                    "page_id": page_id,
                    "page_rev": rev,
                    "created_ms": created_ms,
                    "result": result,
                }));
            }
            Ok(out)
        })
    }

    pub fn merge_session_meta(&self, session_id: &str, patch: &Value) -> Result<Value, StoreError> {
        let session_id = session_id.to_string();
        let patch = patch.clone();
        self.run(move |c| {
            let row = c
                .query_opt(
                    "SELECT meta_json FROM sessions WHERE session_id=$1",
                    &[&session_id],
                )
                .map_err(|e| StoreError::Msg(e.to_string()))?;
            let Some(r) = row else {
                return Err(StoreError::NotFound(format!("session {session_id}")));
            };
            let prev_s: String = r.get(0);
            let mut meta: Value = serde_json::from_str(&prev_s).unwrap_or(json!({}));
            if let (Some(obj), Some(p)) = (meta.as_object_mut(), patch.as_object()) {
                for (k, v) in p {
                    obj.insert(k.clone(), v.clone());
                }
            }
            let ts = now_ms();
            let meta_s = serde_json::to_string(&meta).map_err(|e| StoreError::Msg(e.to_string()))?;
            c.execute(
                "UPDATE sessions SET meta_json=$1, updated_ms=$2 WHERE session_id=$3",
                &[&meta_s, &ts, &session_id],
            )
            .map_err(|e| StoreError::Msg(e.to_string()))?;
            Ok(meta)
        })
    }

    pub fn list_received_batches(&self, session_id: &str) -> Result<Vec<Value>, StoreError> {
        let session_id = session_id.to_string();
        self.run(move |c| list_received_batches_inner(c, &session_id))
    }

    pub fn list_peer_session_ids(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<String>, StoreError> {
        let session_id = session_id.to_string();
        self.run(move |c| list_peer_session_ids_inner(c, &session_id, limit))
    }

    pub fn list_recent_session_ids(&self, limit: usize) -> Result<Vec<String>, StoreError> {
        let lim = limit.max(1).min(5000) as i64;
        self.run(move |c| {
            let rows = c
                .query(
                    "SELECT session_id FROM sessions ORDER BY updated_ms DESC LIMIT $1",
                    &[&lim],
                )
                .map_err(|e| StoreError::Msg(e.to_string()))?;
            Ok(rows.iter().map(|r| r.get::<_, String>(0)).collect())
        })
    }

    pub fn list_recent_latest_analyses(&self, limit: usize) -> Result<Vec<Value>, StoreError> {
        let ids = self.list_recent_session_ids(limit)?;
        let mut out = Vec::new();
        for sid in ids {
            if let Some(mut a) = self.latest_analysis(&sid)? {
                if let Some(obj) = a.as_object_mut() {
                    obj.insert("session_id".into(), json!(sid));
                }
                out.push(a);
            }
        }
        Ok(out)
    }

    /// P1: commercial device master (scalar row, no TOAST).
    pub fn get_device(&self, device_id: &str) -> Result<Value, StoreError> {
        let device_id = device_id.to_string();
        self.run(move |c| get_device_inner(c, &device_id))
    }

    /// P1: sessions that share a commercial device_id.
    pub fn list_device_sessions(&self, device_id: &str, limit: i64) -> Result<Value, StoreError> {
        let device_id = device_id.to_string();
        self.run(move |c| list_device_sessions_inner(c, &device_id, limit))
    }

    /// P2/P3: reverse index by binder key (wg:/au:/lan:/id:).
    pub fn lookup_devices_by_binder(
        &self,
        tenant_id: &str,
        binder_key: &str,
        limit: i64,
    ) -> Result<Value, StoreError> {
        let tenant_id = tenant_id.to_string();
        let binder_key = binder_key.to_string();
        self.run(move |c| lookup_devices_by_binder_inner(c, &tenant_id, &binder_key, limit))
    }

    /// P0: list analysis_latest scalar rows (ops filters; no result_json).
    pub fn list_analysis_latest(
        &self,
        limit: i64,
        device_tier: Option<&str>,
        since_ms: Option<i64>,
    ) -> Result<Value, StoreError> {
        let device_tier = device_tier.map(|s| s.to_string());
        self.run(move |c| {
            list_analysis_latest_inner(c, limit, device_tier.as_deref(), since_ms)
        })
    }

    pub fn has_batch(
        &self,
        session_id: &str,
        batch_id: &str,
        source: &str,
    ) -> Result<bool, StoreError> {
        let session_id = session_id.to_string();
        let batch_id = batch_id.to_string();
        let source = source.to_string();
        self.run(move |c| has_batch_inner(c, &session_id, &batch_id, &source))
    }

    /// DeviceIndex upsert (lab/single-region PG). Same contract as sqlite skeleton.
    pub fn device_index_upsert(
        &self,
        tenant_id: &str,
        device_id: &str,
        binder_obs: &Value,
        binder_keys: &[String],
    ) -> Result<Value, StoreError> {
        let tenant_id = tenant_id.to_string();
        let device_id = device_id.to_string();
        let binder_obs = binder_obs.clone();
        let binder_keys = binder_keys.to_vec();
        self.run(move |c| {
            device_index_upsert_inner(c, &tenant_id, &device_id, &binder_obs, &binder_keys)
        })
    }

    /// Export DeviceIndex as FileDeviceIndex v1 JSON shape.
    pub fn device_index_export(&self, tenant_id: &str) -> Result<Value, StoreError> {
        let tenant_id = tenant_id.to_string();
        self.run(move |c| device_index_export_inner(c, &tenant_id))
    }

    pub fn soft_edge_put(
        &self,
        tenant_id: &str,
        a_session: &str,
        b_session: &str,
        priority: &str,
        confidence: f64,
        reason: &str,
    ) -> Result<Value, StoreError> {
        let tenant_id = tenant_id.to_string();
        let a_session = a_session.to_string();
        let b_session = b_session.to_string();
        let priority = priority.to_string();
        let reason = reason.to_string();
        self.run(move |c| {
            soft_edge_put_inner(c, &tenant_id, &a_session, &b_session, &priority, confidence, &reason)
        })
    }

    pub fn soft_edge_list(&self, tenant_id: &str) -> Result<Vec<Value>, StoreError> {
        let tenant_id = tenant_id.to_string();
        self.run(move |c| soft_edge_list_inner(c, &tenant_id))
    }

    pub fn soft_heat_record(
        &self,
        tenant_id: &str,
        device_id: &str,
        session_id: &str,
    ) -> Result<i64, StoreError> {
        let tenant_id = tenant_id.to_string();
        let device_id = device_id.to_string();
        let session_id = session_id.to_string();
        self.run(move |c| soft_heat_record_inner(c, &tenant_id, &device_id, &session_id))
    }

    pub fn soft_heat_get(&self, tenant_id: &str, device_id: &str) -> Result<Value, StoreError> {
        let tenant_id = tenant_id.to_string();
        let device_id = device_id.to_string();
        self.run(move |c| soft_heat_get_inner(c, &tenant_id, &device_id))
    }
}

fn soft_edge_put_inner(
    c: &mut Client,
    tenant_id: &str,
    a_session: &str,
    b_session: &str,
    priority: &str,
    confidence: f64,
    reason: &str,
) -> Result<Value, StoreError> {
    let ts = now_ms();
    c.execute(
        "INSERT INTO soft_edges(tenant_id, a_session, b_session, priority, confidence, reason, created_ms)
         VALUES($1,$2,$3,$4,$5,$6,$7)
         ON CONFLICT (tenant_id, a_session, b_session) DO UPDATE SET
           priority=EXCLUDED.priority, confidence=EXCLUDED.confidence,
           reason=EXCLUDED.reason, created_ms=EXCLUDED.created_ms",
        &[&tenant_id, &a_session, &b_session, &priority, &confidence, &reason, &ts],
    )
    .map_err(|e| StoreError::Msg(e.to_string()))?;
    Ok(json!({"ok": true, "tenant_id": tenant_id, "created_ms": ts}))
}

fn soft_edge_list_inner(c: &mut Client, tenant_id: &str) -> Result<Vec<Value>, StoreError> {
    let rows = c
        .query(
            "SELECT a_session, b_session, priority, confidence, reason, created_ms
             FROM soft_edges WHERE tenant_id=$1 ORDER BY created_ms DESC LIMIT 5000",
            &[&tenant_id],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    Ok(rows
        .iter()
        .map(|r| {
            json!({
                "a_session": r.get::<_, String>(0),
                "b_session": r.get::<_, String>(1),
                "priority": r.get::<_, String>(2),
                "confidence": r.get::<_, f64>(3),
                "reason": r.get::<_, String>(4),
                "created_ms": r.get::<_, i64>(5),
                "promote_to_commercial_id": false,
            })
        })
        .collect())
}

fn soft_heat_record_inner(
    c: &mut Client,
    tenant_id: &str,
    device_id: &str,
    session_id: &str,
) -> Result<i64, StoreError> {
    let ts = now_ms();
    c.execute(
        "INSERT INTO soft_heat(tenant_id, device_id, session_id, first_ms, last_ms)
         VALUES($1,$2,$3,$4,$4)
         ON CONFLICT (tenant_id, device_id, session_id) DO UPDATE SET last_ms=EXCLUDED.last_ms",
        &[&tenant_id, &device_id, &session_id, &ts],
    )
    .map_err(|e| StoreError::Msg(e.to_string()))?;
    let n: i64 = c
        .query_one(
            "SELECT COUNT(*) FROM soft_heat WHERE tenant_id=$1 AND device_id=$2",
            &[&tenant_id, &device_id],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?
        .get(0);
    Ok(n)
}

fn soft_heat_get_inner(
    c: &mut Client,
    tenant_id: &str,
    device_id: &str,
) -> Result<Value, StoreError> {
    let rows = c
        .query(
            "SELECT session_id FROM soft_heat WHERE tenant_id=$1 AND device_id=$2
             ORDER BY last_ms DESC LIMIT 200",
            &[&tenant_id, &device_id],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    let sessions: Vec<String> = rows.iter().map(|r| r.get(0)).collect();
    let n = sessions.len() as i64;
    Ok(json!({
        "device_id": device_id,
        "session_count": n,
        "sessions": sessions,
        "collision_style": n >= 2,
        "soft_promote": false,
    }))
}

fn ensure_schema(client: &mut Client) -> Result<(), String> {
    // Session-scoped advisory lock: multi-worker cold start safe.
    client
        .execute("SELECT pg_advisory_lock(872365001)", &[])
        .map_err(|e| format!("postgres advisory_lock: {e}"))?;
    let result = (|| {
        client
            .batch_execute(PG_SCHEMA)
            .map_err(|e| format!("postgres schema: {e}"))?;
        // Migrations for existing DBs (CREATE TABLE IF NOT EXISTS will not add columns).
        client
            .batch_execute(
                r#"
ALTER TABLE analysis_results ADD COLUMN IF NOT EXISTS real_band TEXT;
ALTER TABLE analysis_results ADD COLUMN IF NOT EXISTS device_id TEXT;
ALTER TABLE analysis_results ADD COLUMN IF NOT EXISTS bot_verdict TEXT;
ALTER TABLE analysis_results ADD COLUMN IF NOT EXISTS device_confidence DOUBLE PRECISION;
ALTER TABLE analysis_results ADD COLUMN IF NOT EXISTS client_ip TEXT;
ALTER TABLE analysis_results ADD COLUMN IF NOT EXISTS device_tier TEXT;
ALTER TABLE analysis_results ADD COLUMN IF NOT EXISTS collision_risk BOOLEAN;
ALTER TABLE analysis_results ADD COLUMN IF NOT EXISTS product_version TEXT;
ALTER TABLE visitor_terminals ADD COLUMN IF NOT EXISTS product_version_last TEXT NOT NULL DEFAULT '';
ALTER TABLE analysis_results ADD COLUMN IF NOT EXISTS digest_path TEXT;
ALTER TABLE analysis_results ADD COLUMN IF NOT EXISTS residual_entropy_ok BOOLEAN;
ALTER TABLE analysis_results ADD COLUMN IF NOT EXISTS site_id TEXT;
ALTER TABLE analysis_results ADD COLUMN IF NOT EXISTS product_action TEXT;
ALTER TABLE analysis_latest ADD COLUMN IF NOT EXISTS product_action TEXT;
CREATE INDEX IF NOT EXISTS idx_al_product_action ON analysis_latest(product_action)
  WHERE product_action IS NOT NULL AND product_action <> '';
ALTER TABLE analysis_latest ADD COLUMN IF NOT EXISTS mint_residual_ok BOOLEAN;
ALTER TABLE analysis_latest ADD COLUMN IF NOT EXISTS mint_host_ok BOOLEAN;
ALTER TABLE analysis_latest ADD COLUMN IF NOT EXISTS mint_silicon_ok BOOLEAN;
ALTER TABLE analysis_latest ADD COLUMN IF NOT EXISTS mint_conflict_pressure DOUBLE PRECISION;
ALTER TABLE analysis_latest ADD COLUMN IF NOT EXISTS mint_single_source_pressure DOUBLE PRECISION;
ALTER TABLE analysis_latest ADD COLUMN IF NOT EXISTS mint_ok_keys_n INTEGER;
ALTER TABLE analysis_latest ADD COLUMN IF NOT EXISTS mint_conf_only_keys_n INTEGER;
ALTER TABLE analysis_latest ADD COLUMN IF NOT EXISTS mint_conflict_keys_n INTEGER;
ALTER TABLE analysis_latest ADD COLUMN IF NOT EXISTS mint_gate_summary TEXT;
CREATE INDEX IF NOT EXISTS idx_al_mint_residual ON analysis_latest(mint_residual_ok)
  WHERE mint_residual_ok IS NOT NULL;
CREATE TABLE IF NOT EXISTS ops_client_events (
  id BIGSERIAL PRIMARY KEY,
  event_id TEXT NOT NULL DEFAULT '',
  ts_ms BIGINT NOT NULL,
  server_recv_ms BIGINT NOT NULL,
  site_id TEXT NOT NULL DEFAULT '',
  visitor_terminal_id TEXT NOT NULL DEFAULT '',
  session_id TEXT NOT NULL DEFAULT '',
  product_version TEXT NOT NULL DEFAULT '',
  inject_path TEXT NOT NULL DEFAULT '',
  engine_family TEXT NOT NULL DEFAULT '',
  ua_hash TEXT NOT NULL DEFAULT '',
  stage TEXT NOT NULL DEFAULT '',
  code TEXT NOT NULL,
  severity TEXT NOT NULL DEFAULT 'error',
  detail_json TEXT NOT NULL DEFAULT '{}',
  sample_rate DOUBLE PRECISION NOT NULL DEFAULT 1.0,
  client_ip TEXT
);
CREATE INDEX IF NOT EXISTS idx_ops_ce_ts ON ops_client_events(server_recv_ms DESC);
CREATE INDEX IF NOT EXISTS idx_ops_ce_code ON ops_client_events(code, server_recv_ms DESC);
CREATE INDEX IF NOT EXISTS idx_ops_ce_product_version ON ops_client_events(product_version, server_recv_ms DESC);
CREATE TABLE IF NOT EXISTS ops_server_events (
  id BIGSERIAL PRIMARY KEY,
  event_id TEXT NOT NULL DEFAULT '',
  ts_ms BIGINT NOT NULL,
  site_id TEXT NOT NULL DEFAULT '',
  visitor_terminal_id TEXT NOT NULL DEFAULT '',
  session_id TEXT NOT NULL DEFAULT '',
  product_version TEXT NOT NULL DEFAULT '',
  engine_family TEXT NOT NULL DEFAULT '',
  stage TEXT NOT NULL DEFAULT '',
  code TEXT NOT NULL,
  severity TEXT NOT NULL DEFAULT 'error',
  detail_json TEXT NOT NULL DEFAULT '{}',
  client_ip TEXT
);
CREATE INDEX IF NOT EXISTS idx_ops_se_ts ON ops_server_events(ts_ms DESC);
CREATE INDEX IF NOT EXISTS idx_ops_se_code ON ops_server_events(code, ts_ms DESC);
ALTER TABLE probe_batches ADD COLUMN IF NOT EXISTS client_ip TEXT;
ALTER TABLE probe_batches ADD COLUMN IF NOT EXISTS material_generation BIGINT NOT NULL DEFAULT 1;
ALTER TABLE probe_batches ADD COLUMN IF NOT EXISTS material_hash TEXT;
ALTER TABLE probe_batches ADD COLUMN IF NOT EXISTS capture_id TEXT;
CREATE INDEX IF NOT EXISTS idx_probe_batches_capture ON probe_batches(capture_id)
  WHERE capture_id IS NOT NULL AND capture_id <> '';
CREATE INDEX IF NOT EXISTS idx_probe_batches_mat_gen ON probe_batches(session_id, batch_id, material_generation);
CREATE TABLE IF NOT EXISTS observation_events (
  observation_id TEXT PRIMARY KEY,
  tenant_id TEXT NOT NULL DEFAULT '',
  session_id TEXT NOT NULL,
  batch_id TEXT NOT NULL,
  source TEXT NOT NULL DEFAULT '',
  source_kind TEXT NOT NULL DEFAULT '',
  realm_kind TEXT NOT NULL DEFAULT '',
  probe_method_id TEXT NOT NULL DEFAULT '',
  capture_id TEXT,
  attempt_id TEXT,
  envelope_json TEXT NOT NULL,
  created_ms BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_obs_session ON observation_events(session_id, created_ms);
CREATE TABLE IF NOT EXISTS api_idempotency (
  tenant_id TEXT NOT NULL DEFAULT '',
  route TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  body_hash TEXT NOT NULL,
  status INTEGER NOT NULL,
  response_json TEXT NOT NULL,
  created_ms BIGINT NOT NULL,
  PRIMARY KEY (tenant_id, route, idempotency_key)
);
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS client_ip TEXT;
CREATE INDEX IF NOT EXISTS idx_analysis_device_tier ON analysis_results(device_tier)
  WHERE device_tier IS NOT NULL AND device_tier <> '';
CREATE INDEX IF NOT EXISTS idx_analysis_collision_risk ON analysis_results(collision_risk)
  WHERE collision_risk IS TRUE;
CREATE INDEX IF NOT EXISTS idx_analysis_product_version ON analysis_results(product_version)
  WHERE product_version IS NOT NULL AND product_version <> '';
CREATE INDEX IF NOT EXISTS idx_analysis_digest_path ON analysis_results(digest_path)
  WHERE digest_path IS NOT NULL AND digest_path <> '';
CREATE INDEX IF NOT EXISTS idx_analysis_residual_entropy ON analysis_results(residual_entropy_ok)
  WHERE residual_entropy_ok IS FALSE;
CREATE TABLE IF NOT EXISTS probe_cold (
  id BIGSERIAL PRIMARY KEY,
  session_id TEXT NOT NULL,
  visitor_terminal_id TEXT NOT NULL DEFAULT '',
  batch_id TEXT NOT NULL,
  source TEXT NOT NULL DEFAULT '',
  client_ip TEXT,
  fields_json JSONB NOT NULL DEFAULT '{}'::jsonb,
  payload_z BYTEA NOT NULL DEFAULT '\x'::bytea,
  created_ms BIGINT NOT NULL,
  UNIQUE (session_id, batch_id, source)
);
CREATE INDEX IF NOT EXISTS idx_probe_cold_session ON probe_cold(session_id);
CREATE INDEX IF NOT EXISTS idx_probe_cold_vt ON probe_cold(visitor_terminal_id);
CREATE INDEX IF NOT EXISTS idx_probe_cold_ip ON probe_cold(client_ip)
  WHERE client_ip IS NOT NULL AND client_ip <> '';
CREATE INDEX IF NOT EXISTS idx_probe_batches_ip ON probe_batches(client_ip)
  WHERE client_ip IS NOT NULL AND client_ip <> '';
CREATE INDEX IF NOT EXISTS idx_analysis_client_ip ON analysis_results(client_ip)
  WHERE client_ip IS NOT NULL AND client_ip <> '';
CREATE INDEX IF NOT EXISTS idx_analysis_bot_verdict ON analysis_results(bot_verdict)
  WHERE bot_verdict IS NOT NULL AND bot_verdict <> '';
-- Upgrade legacy TEXT cold columns → jsonb + bytea (hex text → binary).
DO $$
DECLARE
  fj_type text;
  pz_type text;
BEGIN
  SELECT data_type INTO fj_type FROM information_schema.columns
    WHERE table_schema='public' AND table_name='probe_cold' AND column_name='fields_json';
  SELECT data_type INTO pz_type FROM information_schema.columns
    WHERE table_schema='public' AND table_name='probe_cold' AND column_name='payload_z';
  IF fj_type IS NOT NULL AND fj_type <> 'jsonb' THEN
    ALTER TABLE probe_cold ALTER COLUMN fields_json DROP DEFAULT;
    ALTER TABLE probe_cold
      ALTER COLUMN fields_json TYPE JSONB
      USING CASE
        WHEN fields_json IS NULL OR btrim(fields_json::text) = '' THEN '{}'::jsonb
        ELSE fields_json::text::jsonb
      END;
    ALTER TABLE probe_cold ALTER COLUMN fields_json SET DEFAULT '{}'::jsonb;
  END IF;
  IF pz_type = 'text' OR pz_type = 'character varying' THEN
    ALTER TABLE probe_cold ADD COLUMN IF NOT EXISTS payload_z_bin BYTEA;
    UPDATE probe_cold SET payload_z_bin = CASE
      WHEN payload_z IS NULL OR payload_z = '' THEN '\x'::bytea
      WHEN payload_z ~ '^[0-9a-fA-F]+$' AND (length(payload_z) % 2) = 0
        THEN decode(payload_z, 'hex')
      ELSE convert_to(payload_z, 'UTF8')
    END
    WHERE payload_z_bin IS NULL;
    ALTER TABLE probe_cold DROP COLUMN payload_z;
    ALTER TABLE probe_cold RENAME COLUMN payload_z_bin TO payload_z;
    ALTER TABLE probe_cold ALTER COLUMN payload_z SET DEFAULT '\x'::bytea;
    ALTER TABLE probe_cold ALTER COLUMN payload_z SET NOT NULL;
  ELSIF pz_type IS NULL THEN
    -- missing column (should not happen after CREATE)
    NULL;
  END IF;
END $$;
-- Drop obsolete demote-only duplicate rows if any remain from earlier builds.
DELETE FROM probe_cold WHERE source = 'hot_demote';
-- Query helpers: GIN on fields + expression indexes for common filters.
CREATE INDEX IF NOT EXISTS idx_probe_cold_fields_gin
  ON probe_cold USING GIN (fields_json jsonb_path_ops);
CREATE INDEX IF NOT EXISTS idx_probe_cold_field_ip
  ON probe_cold ((fields_json #>> '{fields,server_client_ip}'))
  WHERE (fields_json #>> '{fields,server_client_ip}') IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_probe_cold_field_sw
  ON probe_cold ((fields_json #>> '{fields,screen_width}'))
  WHERE (fields_json #>> '{fields,screen_width}') IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_probe_cold_field_robot
  ON probe_cold ((fields_json #>> '{fields,robot_name}'))
  WHERE (fields_json #>> '{fields,robot_name}') IS NOT NULL;
-- P0 composite indexes on analysis_results history
CREATE INDEX IF NOT EXISTS idx_ar_created_tier ON analysis_results (created_ms DESC, device_tier)
  WHERE device_id IS NOT NULL AND device_id <> '';
CREATE INDEX IF NOT EXISTS idx_ar_device_created ON analysis_results (device_id, created_ms DESC)
  WHERE device_id IS NOT NULL AND device_id <> '';
CREATE INDEX IF NOT EXISTS idx_ar_device_prefix ON analysis_results ((left(device_id, 2)), created_ms DESC)
  WHERE device_id IS NOT NULL AND device_id <> '';
-- P0/P1 analysis_latest + devices (CREATE IF NOT EXISTS already in PG_SCHEMA; columns for upgrades)
CREATE TABLE IF NOT EXISTS analysis_latest (
  session_id TEXT PRIMARY KEY REFERENCES sessions(session_id) ON DELETE CASCADE,
  rev BIGINT NOT NULL,
  created_ms BIGINT NOT NULL,
  device_id TEXT,
  device_tier TEXT,
  device_prefix TEXT,
  collision_risk BOOLEAN,
  residual_entropy_ok BOOLEAN,
  real_band TEXT,
  bot_verdict TEXT,
  device_confidence DOUBLE PRECISION,
  client_ip TEXT,
  digest_path TEXT,
  product_version TEXT,
  visitor_terminal_id TEXT,
  site_id TEXT,
  inject_path TEXT,
  association_level TEXT,
  authenticity_band TEXT,
  form_class TEXT,
  os_family TEXT,
  platform TEXT,
  os_score DOUBLE PRECISION,
  br_score DOUBLE PRECISION,
  rpa_score DOUBLE PRECISION,
  os_status TEXT,
  br_status TEXT,
  rpa_status TEXT,
  country TEXT,
  asn TEXT,
  residual_algo TEXT,
  has_webrtc_host BOOLEAN,
  mint_residual_ok BOOLEAN,
  mint_host_ok BOOLEAN,
  mint_silicon_ok BOOLEAN,
  mint_conflict_pressure DOUBLE PRECISION,
  mint_single_source_pressure DOUBLE PRECISION,
  mint_ok_keys_n INTEGER,
  mint_conf_only_keys_n INTEGER,
  mint_conflict_keys_n INTEGER,
  mint_gate_summary TEXT
);
CREATE INDEX IF NOT EXISTS idx_al_device ON analysis_latest(device_id)
  WHERE device_id IS NOT NULL AND device_id <> '';
CREATE INDEX IF NOT EXISTS idx_al_tier_created ON analysis_latest(device_tier, created_ms DESC);
CREATE INDEX IF NOT EXISTS idx_al_ip_created ON analysis_latest(client_ip, created_ms DESC)
  WHERE client_ip IS NOT NULL AND client_ip <> '';
CREATE INDEX IF NOT EXISTS idx_al_vt_created ON analysis_latest(visitor_terminal_id, created_ms DESC)
  WHERE visitor_terminal_id IS NOT NULL AND visitor_terminal_id <> '';
CREATE INDEX IF NOT EXISTS idx_al_created ON analysis_latest(created_ms DESC);
CREATE INDEX IF NOT EXISTS idx_al_prefix_created ON analysis_latest(device_prefix, created_ms DESC)
  WHERE device_prefix IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_al_collision ON analysis_latest(collision_risk, device_tier)
  WHERE collision_risk IS TRUE;
CREATE INDEX IF NOT EXISTS idx_al_site ON analysis_latest(site_id, created_ms DESC)
  WHERE site_id IS NOT NULL AND site_id <> '';
CREATE INDEX IF NOT EXISTS idx_al_os_family ON analysis_latest(os_family, created_ms DESC)
  WHERE os_family IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_al_form ON analysis_latest(form_class, created_ms DESC)
  WHERE form_class IS NOT NULL;
CREATE TABLE IF NOT EXISTS devices (
  device_id TEXT PRIMARY KEY,
  first_seen_ms BIGINT NOT NULL,
  last_seen_ms BIGINT NOT NULL,
  tier_last TEXT,
  collision_risk_last BOOLEAN,
  residual_entropy_ok_last BOOLEAN,
  session_count BIGINT NOT NULL DEFAULT 0,
  distinct_ip_count BIGINT NOT NULL DEFAULT 0,
  distinct_vt_count BIGINT NOT NULL DEFAULT 0,
  last_client_ip TEXT,
  last_session_id TEXT,
  product_version_last TEXT,
  digest_path_last TEXT,
  meta_json JSONB NOT NULL DEFAULT '{}'::jsonb
);
CREATE INDEX IF NOT EXISTS idx_devices_last_seen ON devices(last_seen_ms DESC);
CREATE INDEX IF NOT EXISTS idx_devices_tier ON devices(tier_last, last_seen_ms DESC);
CREATE TABLE IF NOT EXISTS device_sessions (
  device_id TEXT NOT NULL,
  session_id TEXT NOT NULL,
  created_ms BIGINT NOT NULL,
  client_ip TEXT,
  visitor_terminal_id TEXT,
  device_tier TEXT,
  PRIMARY KEY (device_id, session_id)
);
CREATE INDEX IF NOT EXISTS idx_device_sessions_sid ON device_sessions(session_id);
CREATE INDEX IF NOT EXISTS idx_device_sessions_created ON device_sessions(created_ms DESC);
-- P2: probe_cold TOP-field expression indexes (fast field filters)
CREATE INDEX IF NOT EXISTS idx_probe_cold_field_os
  ON probe_cold ((fields_json #>> '{fields,os_family}'))
  WHERE (fields_json #>> '{fields,os_family}') IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_probe_cold_field_form
  ON probe_cold ((fields_json #>> '{fields,form_class}'))
  WHERE (fields_json #>> '{fields,form_class}') IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_probe_cold_field_platform
  ON probe_cold ((fields_json #>> '{fields,platform}'))
  WHERE (fields_json #>> '{fields,platform}') IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_probe_cold_field_residual_algo
  ON probe_cold ((fields_json #>> '{fields,residual_algo}'))
  WHERE (fields_json #>> '{fields,residual_algo}') IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_probe_cold_field_webrtc
  ON probe_cold ((fields_json #>> '{fields,webrtc_host_ip_hash}'))
  WHERE (fields_json #>> '{fields,webrtc_host_ip_hash}') IS NOT NULL;
-- P3: binder reverse lookup + device_sessions VT/IP + BRIN on large time columns
CREATE INDEX IF NOT EXISTS idx_device_index_keys_binder
  ON device_index_keys(tenant_id, binder_key);
CREATE INDEX IF NOT EXISTS idx_device_sessions_vt
  ON device_sessions(visitor_terminal_id, created_ms DESC)
  WHERE visitor_terminal_id IS NOT NULL AND visitor_terminal_id <> '';
CREATE INDEX IF NOT EXISTS idx_device_sessions_ip
  ON device_sessions(client_ip, created_ms DESC)
  WHERE client_ip IS NOT NULL AND client_ip <> '';
CREATE INDEX IF NOT EXISTS idx_al_digest ON analysis_latest(digest_path)
  WHERE digest_path IS NOT NULL AND digest_path <> '';
CREATE INDEX IF NOT EXISTS idx_devices_collision
  ON devices(collision_risk_last, last_seen_ms DESC)
  WHERE collision_risk_last IS TRUE;
-- BRIN helps very large append-only history scans (cheap maintenance)
CREATE INDEX IF NOT EXISTS idx_ar_created_brin ON analysis_results USING BRIN (created_ms);
CREATE INDEX IF NOT EXISTS idx_al_created_brin ON analysis_latest USING BRIN (created_ms);
CREATE INDEX IF NOT EXISTS idx_probe_cold_created_brin ON probe_cold USING BRIN (created_ms);
"#,
            )
            .map_err(|e| format!("postgres migrate analysis scalars: {e}"))?;
        Ok(())
    })();
    let _ = client.execute("SELECT pg_advisory_unlock(872365001)", &[]);
    result
}



fn open_session_inner(
    c: &mut Client,
    session_id: Option<String>,
    visitor_terminal_id: Option<String>,
    meta: Option<Value>,
) -> Result<Value, StoreError> {
    let mut sid = session_id.unwrap_or_else(|| format!("sess_{}", hex_now()));
    let mut meta_v = meta.unwrap_or_else(|| json!({}));
    if !meta_v.is_object() {
        meta_v = json!({});
    }
    let ts = now_ms();
    // Race auto-open / gateway early without product_version: if VT already has an
    // active incomplete cycle, join that bag instead of minting parallel B8-only cycles.
    let race_auto = meta_v
        .get("ingest_auto_open")
        .or_else(|| meta_v.get("fe_race"))
        .or_else(|| meta_v.get("early_kick"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || meta_v
            .get("fe")
            .and_then(|v| v.as_str())
            .map(|s| s.contains("micro") || s.contains("early"))
            .unwrap_or(false);
    if race_auto {
        if let Some(vt0) = visitor_terminal_id.as_ref().filter(|s| !s.is_empty()) {
            if let Ok(Some((_lc, _cu, active, _pvl))) = pg_load_vt(c, vt0) {
                if !active.is_empty() && active != sid {
                    if let Ok(Some((_, created_ms, _, row_meta))) = pg_cycle_row(c, &active) {
                        let status = cycle_status_from_meta(&row_meta);
                        let age = ts - created_ms;
                        if status != "complete" && age < cycle_incomplete_ms() {
                            sid = active;
                            if let Some(obj) = meta_v.as_object_mut() {
                                obj.insert("converged_to_active".into(), json!(true));
                                obj.insert("race_join_active".into(), json!(true));
                            }
                        }
                    }
                }
            }
        }
    }
    let prev = c
        .query_opt(
            "SELECT visitor_terminal_id, meta_json FROM sessions WHERE session_id=$1",
            &[&sid],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    let vt = if let Some(row) = prev {
        let prev_vt: Option<String> = row.get(0);
        let prev_s: String = row.get(1);
        if let Ok(Value::Object(prev_m)) = serde_json::from_str::<Value>(&prev_s) {
            if let Some(obj) = meta_v.as_object_mut() {
                // Prefer sticky primary inject_path (nginx/cf_worker) over app.
                let prev_ip = prev_m
                    .get("inject_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let new_ip = obj
                    .get("inject_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let primary = |s: &str| s == "nginx" || s == "cf_worker";
                let keep_ip = if primary(prev_ip) {
                    prev_ip.to_string()
                } else if primary(new_ip) {
                    new_ip.to_string()
                } else if !new_ip.is_empty() {
                    new_ip.to_string()
                } else {
                    prev_ip.to_string()
                };
                for (k, v) in prev_m {
                    obj.entry(k).or_insert(v);
                }
                if !keep_ip.is_empty() {
                    obj.insert("inject_path".into(), json!(keep_ip));
                }
            }
        }
        prev_vt
            .filter(|s| !s.is_empty())
            .or_else(|| visitor_terminal_id.filter(|s| !s.is_empty()))
            .unwrap_or_else(|| format!("vt_{}", hex_now()))
    } else {
        visitor_terminal_id.unwrap_or_else(|| format!("vt_{}", hex_now()))
    };
    let inject_path = meta_v
        .get("inject_path")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let meta_s = serde_json::to_string(&meta_v)?;
    c.execute(
        "INSERT INTO sessions(session_id, visitor_terminal_id, created_ms, updated_ms, meta_json)
         VALUES($1,$2,$3,$3,$4)
         ON CONFLICT(session_id) DO UPDATE SET
           updated_ms=EXCLUDED.updated_ms,
           visitor_terminal_id=COALESCE(NULLIF(sessions.visitor_terminal_id, ''), EXCLUDED.visitor_terminal_id),
           meta_json=EXCLUDED.meta_json",
        &[&sid, &vt, &ts, &meta_s],
    )
    .map_err(|e| StoreError::Msg(e.to_string()))?;
    Ok(json!({
        "session_id": sid,
        "cycle_id": sid,
        "visitor_terminal_id": vt,
        "created_ms": ts,
        "inject_path": inject_path,
        "meta": meta_v,
        "phase": "active",
        "skip_identity_probe": false,
    }))
}

/// (last_complete_ms, cool_until_ms, active_cycle_id, product_version_last)
fn pg_load_vt(
    c: &mut Client,
    vt: &str,
) -> Result<Option<(i64, i64, String, String)>, StoreError> {
    let row = c
        .query_opt(
            "SELECT last_complete_ms, cool_until_ms, active_cycle_id,
                    COALESCE(product_version_last, '')
             FROM visitor_terminals WHERE vt_id=$1",
            &[&vt],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    Ok(row.map(|r| (r.get(0), r.get(1), r.get(2), r.get(3))))
}

fn pg_upsert_vt(
    c: &mut Client,
    vt: &str,
    last_complete_ms: i64,
    cool_until_ms: i64,
    active_cycle_id: &str,
    product_version_last: &str,
) -> Result<(), StoreError> {
    let ts = now_ms();
    c.execute(
        "INSERT INTO visitor_terminals(
           vt_id, last_complete_ms, cool_until_ms, active_cycle_id, updated_ms,
           meta_json, product_version_last)
         VALUES($1,$2,$3,$4,$5,'{}',$6)
         ON CONFLICT(vt_id) DO UPDATE SET
           last_complete_ms=EXCLUDED.last_complete_ms,
           cool_until_ms=EXCLUDED.cool_until_ms,
           active_cycle_id=EXCLUDED.active_cycle_id,
           updated_ms=EXCLUDED.updated_ms,
           product_version_last=EXCLUDED.product_version_last",
        &[
            &vt,
            &last_complete_ms,
            &cool_until_ms,
            &active_cycle_id,
            &ts,
            &product_version_last,
        ],
    )
    .map_err(|e| StoreError::Msg(e.to_string()))?;
    Ok(())
}

fn pg_cycle_row(
    c: &mut Client,
    cycle_id: &str,
) -> Result<Option<(String, i64, i64, Value)>, StoreError> {
    let row = c
        .query_opt(
            "SELECT visitor_terminal_id, created_ms, updated_ms, meta_json
             FROM sessions WHERE session_id=$1",
            &[&cycle_id],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    Ok(row.map(|r| {
        let meta_s: String = r.get(3);
        let meta: Value = serde_json::from_str(&meta_s).unwrap_or(json!({}));
        (r.get(0), r.get(1), r.get(2), meta)
    }))
}


fn session_has_b10_batch(c: &mut Client, session_id: &str) -> bool {
    c.query_opt(
        "SELECT 1 FROM probe_batches WHERE session_id=$1 AND batch_id='B10_hw_curves' LIMIT 1",
        &[&session_id],
    )
    .ok()
    .flatten()
    .is_some()
}

/// Last analysis JSON has residual or hw_curve silicon materials.
fn last_identity_has_silicon(last: &Option<Value>) -> bool {
    let Some(v) = last else { return false };
    let r = v.get("result").cloned().unwrap_or_else(|| v.clone());
    if crate::result_has_silicon(&r) || crate::result_commercial_identity_final(&r) {
        return true;
    }
    // analysis may be wrapped {result: ...} or flat product/device
    let fields = v
        .pointer("/result/fields")
        .or_else(|| v.get("fields"))
        .or_else(|| v.pointer("/device/materials"))
        .cloned()
        .unwrap_or(Value::Null);
    let fo = fields.as_object();
    let residual = fo
        .map(|o| {
            o.get("residual_std").and_then(|x| x.as_f64()).is_some()
                || o.get("residual_mean").and_then(|x| x.as_f64()).is_some()
                || o.get("residual_ok").and_then(|x| x.as_bool()) == Some(true)
        })
        .unwrap_or(false);
    let curves = fo
        .map(|o| {
            o.get("hw_curve_webgl").map(|x| !x.is_null()).unwrap_or(false)
                || o.get("hw_curve_audio").map(|x| !x.is_null()).unwrap_or(false)
        })
        .unwrap_or(false);
    let digest = v
        .pointer("/result/device/digest_path")
        .or_else(|| v.pointer("/device/digest_path"))
        .or_else(|| v.get("digest_path"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let tier = v
        .pointer("/result/device/device_tier")
        .or_else(|| v.pointer("/device/device_tier"))
        .or_else(|| v.get("device_tier"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    (residual || curves)
        && (digest.contains("real_curves") || tier == "dh" || tier == "dv")
}

fn open_cycle_inner(
    c: &mut Client,
    cycle_id_hint: Option<String>,
    visitor_terminal_id: Option<String>,
    meta: Option<Value>,
) -> Result<Value, StoreError> {
    let ts = now_ms();
    let mut meta_v = meta.unwrap_or_else(|| json!({}));
    if !meta_v.is_object() {
        meta_v = json!({});
    }
    let vt = visitor_terminal_id
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("vt_{}", hex_now()));
    // Always stamp product_version on open (gateway early / race path often omit it).
    // Empty PV cycles must NOT steal VT.active_cycle_id (orphan B8 storm).
    let mut current_product_version = crate::product_version_from_meta(&meta_v);
    if current_product_version.is_empty() {
        current_product_version = gr_abi::env::get("PRODUCT_VERSION").unwrap_or_default();
    }
    if let Some(obj) = meta_v.as_object_mut() {
        obj.entry("identity_class").or_insert_with(|| json!("js"));
        obj.entry("cycle_status").or_insert_with(|| json!("active"));
        if !current_product_version.is_empty() {
            obj.insert(
                "product_version".into(),
                json!(current_product_version.clone()),
            );
        }
    }
    let (last_complete_ms, cool_until_ms, active_cycle_id, product_version_last) =
        pg_load_vt(c, &vt)?.unwrap_or((0, 0, String::new(), String::new()));

    let mut cool_ok = crate::cool_valid_for_product_version(
        cool_until_ms,
        ts,
        &product_version_last,
        &current_product_version,
    );
    let force_identity = meta_v
        .get("force_identity")
        .or_else(|| meta_v.get("force_reprobe"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if force_identity {
        cool_ok = false;
    }
    let mut cool_until_ms = cool_until_ms;
    let mut reprobe_reason: Option<crate::ReprobeReason> = None;
    if cool_until_ms > ts && !cool_ok {
        reprobe_reason = crate::cool_invalid_reason(
            cool_until_ms,
            ts,
            &product_version_last,
            &current_product_version,
        )
        .or(Some(if product_version_last.is_empty() {
            crate::ReprobeReason::MissingProductVersionLast
        } else {
            crate::ReprobeReason::ProductVersionMismatch
        }));
        let _ = pg_upsert_vt(
            c,
            &vt,
            last_complete_ms,
            0,
            &active_cycle_id,
            &product_version_last,
        );
        cool_until_ms = 0;
        if let Some(obj) = meta_v.as_object_mut() {
            obj.insert(
                "cool_invalidated_reason".into(),
                json!(reprobe_reason
                    .map(|r| r.as_str())
                    .unwrap_or("product_version_mismatch")),
            );
            obj.insert(
                "product_version_last".into(),
                json!(product_version_last.clone()),
            );
        }
    } else if cool_ok {
        let cid = if !active_cycle_id.is_empty() {
            active_cycle_id.clone()
        } else {
            cycle_id_hint
                .clone()
                .unwrap_or_else(|| format!("cycle_{}", hex_now()))
        };
        if pg_cycle_row(c, &cid)?.is_none() {
            let meta_s = serde_json::to_string(&meta_v)?;
            let _ = c.execute(
                "INSERT INTO sessions(session_id, visitor_terminal_id, created_ms, updated_ms, meta_json)
                 VALUES($1,$2,$3,$3,$4) ON CONFLICT DO NOTHING",
                &[&cid, &vt, &ts, &meta_s],
            );
        }
        let last = latest_analysis_inner(c, &cid).ok().flatten();
        // Production: cool without silicon is invalid — never skip identity after thin complete.
        // Users must not need to clear browser cache to re-run B10.
        let has_b10 = session_has_b10_batch(c, &cid);
        let silicon_ok = last_identity_has_silicon(&last) || has_b10;
        if !silicon_ok {
            cool_until_ms = 0;
            if let Some(obj) = meta_v.as_object_mut() {
                obj.insert(
                    "cool_invalidated_reason".into(),
                    json!("cool_without_silicon"),
                );
                obj.insert("force_identity".into(), json!(true));
            }
            let _ = pg_upsert_vt(
                c,
                &vt,
                last_complete_ms,
                0,
                &active_cycle_id,
                &product_version_last,
            );
            // fall through to full open (do not return cool)
        } else {
            return Ok(json!({
                "session_id": cid,
                "cycle_id": cid,
                "visitor_terminal_id": vt,
                "created_ms": ts,
                "phase": "cool",
                "skip_identity_probe": true,
                "skip_session_probe": true,
                "cool_until_ms": cool_until_ms,
                "last_complete_ms": last_complete_ms,
                "last_identity_result": last,
                "page_probe_required": true,
                "product_version": current_product_version,
                "product_version_last": product_version_last,
                "meta": meta_v,
                "policy": {
                    "cycle_cool_ms": cycle_cool_ms(),
                    "cycle_incomplete_ms": cycle_incomplete_ms(),
                    "cool_scoped_by_product_version": true,
                    "cool_requires_silicon": true,
                }
            }));
        }
    }

    // v57 race: FE mints cycle_id; resume only if incomplete + product_version matches.
    // Completed / wrong-version sticky hints are **superseded** (server mints new id;
    // FE adopts via openApplied — no user cookie clear).
    if let Some(hint) = cycle_id_hint.as_ref().filter(|h| !h.is_empty()) {
        if let Some((row_vt, created_ms, _, row_meta)) = pg_cycle_row(c, hint)? {
            let status = cycle_status_from_meta(&row_meta);
            let age = ts - created_ms;
            let ver_ok =
                crate::cycle_compatible_with_product_version(&row_meta, &current_product_version);
            if status == "complete" {
                reprobe_reason = Some(crate::ReprobeReason::CycleCompleteSuperseded);
            } else if !ver_ok {
                reprobe_reason = Some(crate::ReprobeReason::CycleVersionMismatch);
            } else if age >= cycle_incomplete_ms() {
                reprobe_reason = Some(crate::ReprobeReason::CycleIncompleteExpired);
            } else if (row_vt == vt || row_vt.is_empty()) && age < cycle_incomplete_ms() {
                // Always resume incomplete sticky same-version cycle.
                // Do NOT supersede after 8s without B10 — that minted a remint storm
                // (micro B8-only cycles). FE re-kicks hard anchors via need_hard_anchor.
                // Heal: schedule-final already on sticky incomplete → complete + cool open.
                if let Ok(Some(last)) = latest_analysis_inner(c, hint) {
                    let r = last.get("result").cloned().unwrap_or_else(|| last.clone());
                    if crate::analysis_completes_cycle(&r) {
                        let _ = complete_cycle_inner(c, hint);
                        return open_cycle_inner(c, None, Some(vt), Some(meta_v));
                    }
                }
                if let Some(obj) = meta_v.as_object_mut() {
                    if let Some(rm) = row_meta.as_object() {
                        for (k, v) in rm {
                            obj.entry(k.clone()).or_insert_with(|| v.clone());
                        }
                    }
                    obj.insert("cycle_status".into(), json!("active"));
                    if !current_product_version.is_empty() {
                        obj.insert(
                            "product_version".into(),
                            json!(current_product_version.clone()),
                        );
                    }
                }
                let meta_s = serde_json::to_string(&meta_v)?;
                c.execute(
                    "UPDATE sessions SET updated_ms=$1, visitor_terminal_id=$2, meta_json=$3
                     WHERE session_id=$4",
                    &[&ts, &vt, &meta_s, hint],
                )
                .map_err(|e| StoreError::Msg(e.to_string()))?;
                pg_upsert_vt(c, &vt, last_complete_ms, cool_until_ms, hint, &product_version_last)?;
                let bonded = attach_orphan_b8_for_vt(c, &vt, hint, 30_000).unwrap_or(false);
                return Ok(json!({
                    "session_id": hint,
                    "cycle_id": hint,
                    "visitor_terminal_id": vt,
                    "created_ms": created_ms,
                    "phase": "active",
                    "skip_identity_probe": false,
                    "force_identity_probe": true,
                    "resumed": true,
                    "need_hard_anchor": !session_has_b10_batch(c, hint),
                    "b8_bonded_from_orphan": bonded,
                    "cycle_expires_ms": created_ms + cycle_incomplete_ms(),
                    "product_version": current_product_version,
                    "product_version_last": product_version_last,
                    "meta": meta_v,
                    "policy": {
                        "cycle_cool_ms": cycle_cool_ms(),
                        "cycle_incomplete_ms": cycle_incomplete_ms(),
                        "cool_scoped_by_product_version": true,
                    }
                }));
            }
            // Unresumable sticky hint → mint fresh server id (FE adopts).
            let cid = format!("cycle_{}", hex_now());
            if let Some(obj) = meta_v.as_object_mut() {
                obj.insert("cycle_status".into(), json!("active"));
                obj.insert("cycle_id".into(), json!(cid.clone()));
                if let Some(r) = reprobe_reason {
                    obj.insert("client_hint_superseded_reason".into(), json!(r.as_str()));
                    obj.insert("superseded_cycle_id".into(), json!(hint.clone()));
                }
            }
            let meta_s = serde_json::to_string(&meta_v)?;
            c.execute(
                "INSERT INTO sessions(session_id, visitor_terminal_id, created_ms, updated_ms, meta_json)
                 VALUES($1,$2,$3,$3,$4)",
                &[&cid, &vt, &ts, &meta_s],
            )
            .map_err(|e| StoreError::Msg(e.to_string()))?;
            pg_upsert_vt(c, &vt, last_complete_ms, 0, &cid, &product_version_last)?;
            let bonded = attach_orphan_b8_for_vt(c, &vt, &cid, 30_000).unwrap_or(false);
            return Ok(json!({
                "session_id": cid,
                "cycle_id": cid,
                "visitor_terminal_id": vt,
                "created_ms": ts,
                "phase": "new",
                "skip_identity_probe": false,
                "skip_session_probe": false,
                "force_identity_probe": true,
                "cycle_expires_ms": ts + cycle_incomplete_ms(),
                "resumed": false,
                "client_hint_honored": false,
                "client_hint_superseded": true,
                "superseded_cycle_id": hint,
                "reprobe_reason": reprobe_reason.map(|r| r.as_str()),
                "b8_bonded_from_orphan": bonded,
                "product_version": current_product_version,
                "product_version_last": product_version_last,
                "meta": meta_v,
                "policy": {
                    "cycle_cool_ms": cycle_cool_ms(),
                    "cycle_incomplete_ms": cycle_incomplete_ms(),
                    "cool_scoped_by_product_version": true,
                }
            }));
        } else {
            // Client-minted id not in DB yet. Prefer VT active incomplete (one bag per VT).
            if !active_cycle_id.is_empty() && active_cycle_id != *hint {
                if let Some((row_vt, created_ms, _, row_meta)) = pg_cycle_row(c, &active_cycle_id)? {
                    let status = cycle_status_from_meta(&row_meta);
                    let age = ts - created_ms;
                    let ver_ok = crate::cycle_compatible_with_product_version(
                        &row_meta,
                        &current_product_version,
                    );
                    if status != "complete"
                        && ver_ok
                        && age < cycle_incomplete_ms()
                        && (row_vt == vt || row_vt.is_empty())
                    {
                        // Re-enter with active id so FE adopts server bag (one VT one active).
                        let mut out = open_cycle_inner(
                            c,
                            Some(active_cycle_id.clone()),
                            Some(vt),
                            Some(meta_v),
                        )?;
                        if let Some(obj) = out.as_object_mut() {
                            obj.insert("converged_to_active".into(), json!(true));
                            obj.insert("client_hint_honored".into(), json!(false));
                            obj.insert("client_hint_superseded".into(), json!(true));
                            obj.insert(
                                "reprobe_reason".into(),
                                json!("one_vt_one_active_cycle"),
                            );
                            obj.insert("superseded_cycle_id".into(), json!(hint.clone()));
                        }
                        return Ok(out);
                    }
                }
            }
            // Create with client-minted id (race path / first open for VT).
            // If VT already has another active incomplete, never promote a parallel bag.
            if !active_cycle_id.is_empty() && active_cycle_id != *hint {
                // fall through handled above; if we still got here active was unusable
            }
            let cid = hint.clone();
            if let Some(obj) = meta_v.as_object_mut() {
                obj.insert("cycle_status".into(), json!("active"));
                obj.insert("cycle_id".into(), json!(cid.clone()));
            }
            let meta_s = serde_json::to_string(&meta_v)?;
            c.execute(
                "INSERT INTO sessions(session_id, visitor_terminal_id, created_ms, updated_ms, meta_json)
                 VALUES($1,$2,$3,$3,$4) ON CONFLICT DO NOTHING",
                &[&cid, &vt, &ts, &meta_s],
            )
            .map_err(|e| StoreError::Msg(e.to_string()))?;
            let _ = c.execute(
                "UPDATE sessions SET visitor_terminal_id=$1, updated_ms=$2 WHERE session_id=$3
                 AND (visitor_terminal_id IS NULL OR visitor_terminal_id='' OR visitor_terminal_id=$1)",
                &[&vt, &ts, &cid],
            );
            // Only bind as active when product_version is known (prevents orphan B8 steal).
            if !current_product_version.is_empty() {
                pg_upsert_vt(c, &vt, last_complete_ms, 0, &cid, &product_version_last)?;
            }
            let bonded = attach_orphan_b8_for_vt(c, &vt, &cid, 30_000).unwrap_or(false);
            return Ok(json!({
                "session_id": cid,
                "cycle_id": cid,
                "visitor_terminal_id": vt,
                "created_ms": ts,
                "phase": "new",
                "skip_identity_probe": false,
                "skip_session_probe": false,
                "force_identity_probe": true,
                "cycle_expires_ms": ts + cycle_incomplete_ms(),
                "resumed": false,
                "client_hint_honored": true,
                "active_promoted": !current_product_version.is_empty(),
                "reprobe_reason": reprobe_reason.map(|r| r.as_str()),
                "b8_bonded_from_orphan": bonded,
                "product_version": current_product_version,
                "product_version_last": product_version_last,
                "meta": meta_v,
                "policy": {
                    "cycle_cool_ms": cycle_cool_ms(),
                    "cycle_incomplete_ms": cycle_incomplete_ms(),
                    "cool_scoped_by_product_version": true,
                    "one_vt_one_active_cycle": true,
                }
            }));
        }
    }

    // One-VT-one-active-cycle: prefer VT.active_cycle_id when incomplete + version-ok,
    // even if client minted a different hint (prevents parallel active bags).
    if !active_cycle_id.is_empty() {
        if let Some((row_vt, created_ms, _upd, row_meta)) = pg_cycle_row(c, &active_cycle_id)? {
            let status = cycle_status_from_meta(&row_meta).to_string();
            let age = ts - created_ms;
            let ver_ok =
                crate::cycle_compatible_with_product_version(&row_meta, &current_product_version);
            if status != "complete" && age >= cycle_incomplete_ms() {
                let _ = purge_cycle_evidence_inner(c, &active_cycle_id);
                // Stamp current product_version on VT when clearing expired incomplete.
                let pv_stamp = if current_product_version.is_empty() {
                    product_version_last.clone()
                } else {
                    current_product_version.clone()
                };
                pg_upsert_vt(c, &vt, last_complete_ms, 0, "", &pv_stamp)?;
                return open_cycle_inner(c, None, Some(vt), Some(meta_v));
            }
            if status != "complete" && !ver_ok {
                let pv_stamp = if current_product_version.is_empty() {
                    product_version_last.clone()
                } else {
                    current_product_version.clone()
                };
                pg_upsert_vt(c, &vt, last_complete_ms, 0, "", &pv_stamp)?;
                // fall through to new cycle under current product_version
            } else if (status == "active" || status.is_empty())
                && (row_vt == vt || row_vt.is_empty())
                && ver_ok
                && age < cycle_incomplete_ms()
            {
                // Heal sticky incomplete with schedule-final already present.
                if let Ok(Some(last)) = latest_analysis_inner(c, &active_cycle_id) {
                    let r = last.get("result").cloned().unwrap_or_else(|| last.clone());
                    if crate::analysis_completes_cycle(&r) {
                        let _ = complete_cycle_inner(c, &active_cycle_id);
                        return open_cycle_inner(c, None, Some(vt), Some(meta_v));
                    }
                }
                if let Some(obj) = meta_v.as_object_mut() {
                    if let Some(rm) = row_meta.as_object() {
                        for (k, v) in rm {
                            obj.entry(k.clone()).or_insert_with(|| v.clone());
                        }
                    }
                    obj.insert("cycle_status".into(), json!("active"));
                    if !current_product_version.is_empty() {
                        obj.insert(
                            "product_version".into(),
                            json!(current_product_version.clone()),
                        );
                    }
                }
                let meta_s = serde_json::to_string(&meta_v)?;
                c.execute(
                    "UPDATE sessions SET updated_ms=$1, visitor_terminal_id=$2, meta_json=$3
                     WHERE session_id=$4",
                    &[&ts, &vt, &meta_s, &active_cycle_id],
                )
                .map_err(|e| StoreError::Msg(e.to_string()))?;
                // Keep product_version_last as last *complete* stamp; surface current in response.
                pg_upsert_vt(
                    c,
                    &vt,
                    last_complete_ms,
                    cool_until_ms,
                    &active_cycle_id,
                    &product_version_last,
                )?;
                let bonded = attach_orphan_b8_for_vt(c, &vt, &active_cycle_id, 30_000).unwrap_or(false);
                let client_hint_diff = cycle_id_hint
                    .as_ref()
                    .map(|h| h != &active_cycle_id)
                    .unwrap_or(false);
                return Ok(json!({
                    "session_id": active_cycle_id,
                    "cycle_id": active_cycle_id,
                    "visitor_terminal_id": vt,
                    "created_ms": created_ms,
                    "phase": "active",
                    "skip_identity_probe": false,
                    "skip_session_probe": false,
                    "force_identity_probe": true,
                    "cycle_expires_ms": created_ms + cycle_incomplete_ms(),
                    "resumed": true,
                    "converged_to_active": client_hint_diff,
                    "client_hint_honored": !client_hint_diff,
                    "need_hard_anchor": !session_has_b10_batch(c, &active_cycle_id),
                    "b8_bonded_from_orphan": bonded,
                    "product_version": current_product_version,
                    "product_version_last": product_version_last,
                    "meta": meta_v,
                    "policy": {
                        "cycle_cool_ms": cycle_cool_ms(),
                        "cycle_incomplete_ms": cycle_incomplete_ms(),
                        "cool_scoped_by_product_version": true,
                        "one_vt_one_active_cycle": true,
                    }
                }));
            }
        }
    }

    let cid = format!("cycle_{}", hex_now());
    if let Some(obj) = meta_v.as_object_mut() {
        obj.insert("cycle_status".into(), json!("active"));
        obj.insert("cycle_id".into(), json!(cid.clone()));
        if !current_product_version.is_empty() {
            obj.insert(
                "product_version".into(),
                json!(current_product_version.clone()),
            );
        }
    }
    let meta_s = serde_json::to_string(&meta_v)?;
    c.execute(
        "INSERT INTO sessions(session_id, visitor_terminal_id, created_ms, updated_ms, meta_json)
         VALUES($1,$2,$3,$3,$4)",
        &[&cid, &vt, &ts, &meta_s],
    )
    .map_err(|e| StoreError::Msg(e.to_string()))?;
    // Mint path always has server PV after stamp above; still guard active promote.
    if !current_product_version.is_empty() {
        pg_upsert_vt(c, &vt, last_complete_ms, 0, &cid, &product_version_last)?;
    }
    Ok(json!({
        "session_id": cid,
        "cycle_id": cid,
        "visitor_terminal_id": vt,
        "created_ms": ts,
        "phase": "new",
        "skip_identity_probe": false,
        "skip_session_probe": false,
        "force_identity_probe": true,
        "cycle_expires_ms": ts + cycle_incomplete_ms(),
        "resumed": false,
        "reprobe_reason": reprobe_reason.map(|r| r.as_str()),
        "product_version": current_product_version,
        "product_version_last": product_version_last,
        "meta": meta_v,
        "policy": {
            "cycle_cool_ms": cycle_cool_ms(),
            "cycle_incomplete_ms": cycle_incomplete_ms(),
            "cool_scoped_by_product_version": true,
            "one_vt_one_active_cycle": true,
        }
    }))
}

fn complete_cycle_inner(c: &mut Client, cycle_id: &str) -> Result<Value, StoreError> {
    let ts = now_ms();
    let row = pg_cycle_row(c, cycle_id)?
        .ok_or_else(|| StoreError::NotFound(format!("cycle {cycle_id}")))?;
    let (vt, created_ms, _upd, mut meta) = row;
    let mut product_version = c
        .query_opt(
            "SELECT product_version FROM analysis_results
             WHERE session_id=$1 AND product_version IS NOT NULL AND product_version <> ''
             ORDER BY created_ms DESC LIMIT 1",
            &[&cycle_id],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?
        .map(|r| r.get::<_, String>(0))
        .unwrap_or_default();
    if product_version.is_empty() {
        product_version = crate::product_version_from_meta(&meta);
    }
    if product_version.is_empty() {
        product_version = gr_abi::env::get("PRODUCT_VERSION").unwrap_or_default();
    }
    // Only grant cool when silicon materials landed — thin complete must not skip next open.
    let has_b10 = session_has_b10_batch(c, cycle_id);
    let last = latest_analysis_inner(c, cycle_id).ok().flatten();
    let silicon_ok = has_b10 || last_identity_has_silicon(&last);
    let site_for_cool = meta
        .get("site_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let cool_until = if silicon_ok {
        ts + cycle_cool_ms_for_site(&site_for_cool)
    } else {
        0
    };
    if let Some(obj) = meta.as_object_mut() {
        obj.insert("cycle_status".into(), json!("complete"));
        obj.insert("completed_ms".into(), json!(ts));
        obj.insert("skip_session_probe".into(), json!(silicon_ok));
        obj.insert("cool_silicon_ok".into(), json!(silicon_ok));
        if !silicon_ok {
            obj.insert("cool_denied_reason".into(), json!("complete_without_silicon"));
        }
        if !product_version.is_empty() {
            obj.insert("product_version".into(), json!(product_version.clone()));
        }
    }
    let meta_s = serde_json::to_string(&meta)?;
    c.execute(
        "UPDATE sessions SET updated_ms=$1, meta_json=$2 WHERE session_id=$3",
        &[&ts, &meta_s, &cycle_id],
    )
    .map_err(|e| StoreError::Msg(e.to_string()))?;
    let _ = c.execute("DELETE FROM analyze_jobs WHERE session_id=$1", &[&cycle_id]);
    pg_upsert_vt(c, &vt, ts, cool_until, cycle_id, &product_version)?;
    Ok(json!({
        "ok": true,
        "cycle_id": cycle_id,
        "session_id": cycle_id,
        "visitor_terminal_id": vt,
        "phase": "complete",
        "completed_ms": ts,
        "cool_until_ms": cool_until,
        "cool_silicon_ok": silicon_ok,
        "created_ms": created_ms,
        "cycle_cool_ms": cycle_cool_ms(),
        "product_version": product_version,
    }))
}

fn purge_cycle_evidence_inner(c: &mut Client, cycle_id: &str) -> Result<Value, StoreError> {
    let ts = now_ms();
    let row = pg_cycle_row(c, cycle_id)?;
    let vt = row.as_ref().map(|r| r.0.clone()).unwrap_or_default();
    let n_batches = c
        .execute("DELETE FROM probe_batches WHERE session_id=$1", &[&cycle_id])
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    let n_jobs = c
        .execute("DELETE FROM analyze_jobs WHERE session_id=$1", &[&cycle_id])
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    let n_analysis = c
        .execute(
            "DELETE FROM analysis_results WHERE session_id=$1",
            &[&cycle_id],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    if let Some((_, _, _, mut meta)) = row {
        if let Some(obj) = meta.as_object_mut() {
            obj.insert("cycle_status".into(), json!("purged"));
            obj.insert("purged_ms".into(), json!(ts));
        }
        let meta_s = serde_json::to_string(&meta)?;
        let _ = c.execute(
            "UPDATE sessions SET updated_ms=$1, meta_json=$2 WHERE session_id=$3",
            &[&ts, &meta_s, &cycle_id],
        );
    }
    if !vt.is_empty() {
        if let Some((lc, cu, active, pvl)) = pg_load_vt(c, &vt)? {
            if active == cycle_id {
                pg_upsert_vt(c, &vt, lc, cu, "", &pvl)?;
            }
        }
    }
    Ok(json!({
        "ok": true,
        "cycle_id": cycle_id,
        "purged_batches": n_batches,
        "purged_jobs": n_jobs,
        "purged_analyses": n_analysis,
        "purged_ms": ts,
    }))
}

fn session_window_inner(c: &mut Client, session_id: &str) -> Result<Value, StoreError> {
    let row = c
        .query_opt(
            "SELECT created_ms, updated_ms, meta_json FROM sessions WHERE session_id=$1",
            &[&session_id],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    let Some(row) = row else {
        return Err(StoreError::NotFound(format!("session {session_id}")));
    };
    let created_ms: i64 = row.get(0);
    let updated_ms: i64 = row.get(1);
    let meta_s: String = row.get(2);
    let meta: Value = serde_json::from_str(&meta_s).unwrap_or(json!({}));
    let status = cycle_status_from_meta(&meta).to_string();
    let now = now_ms();
    let idle = now - updated_ms;
    let age = now - created_ms;
    let (active, expired_reason) = if status == "complete" {
        (false, "cycle_complete")
    } else if status == "purged" {
        (false, "cycle_purged")
    } else if age > cycle_incomplete_ms() {
        (false, "incomplete_ttl")
    } else {
        (true, "")
    };
    Ok(json!({
        "session_id": session_id,
        "cycle_id": session_id,
        "created_ms": created_ms,
        "updated_ms": updated_ms,
        "idle_ms": idle,
        "age_ms": age,
        "cycle_status": status,
        "inactivity_window_ms": SESSION_INACTIVITY_MS,
        "hard_max_session_ms": SESSION_HARD_MAX_MS,
        "cycle_incomplete_ms": cycle_incomplete_ms(),
        "cycle_cool_ms": cycle_cool_ms(),
        "active": active,
        "expired_reason": expired_reason,
        "meta": meta,
        "unknown_bucket": meta.get("unknown_bucket").cloned().unwrap_or(Value::Null),
    }))
}

/// Prefer true-edge TCP/TLS fields over CDN-proxy weak fields; keep both tags.
fn merge_b8_payload(existing: &Value, incoming: &Value) -> Value {
    let mut out = existing.clone();
    let mut fields = existing
        .get("fields")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    if let Some(inc) = incoming.get("fields").and_then(|v| v.as_object()) {
        let inc_is_edge = inc
            .get("b8_path_kind")
            .and_then(|v| v.as_str())
            .map(|s| s == "edge")
            .unwrap_or(false)
            || !inc.contains_key("b8_via_cdn");
        let inc_is_cdn = inc.contains_key("b8_via_cdn")
            || inc
                .get("b8_path_kind")
                .and_then(|v| v.as_str())
                .map(|s| s == "cdn")
                .unwrap_or(false);
        for (k, v) in inc {
            let is_empty = v.is_null() || v.as_str().map(|s| s.is_empty()).unwrap_or(false);
            if is_empty {
                continue;
            }
            let depth_key = k.starts_with("tcp_")
                || k.starts_with("tls_")
                || k == "ja4"
                || k == "h2_fingerprint"
                || k == "client_tcp_rtt"
                || k == "client_tcp_rtt_us"
                || k == "protocol_fp_source"
                || k == "tcp_info_available"
                || k == "tcp_saved_syn";
            if depth_key {
                // Never let CDN overwrite existing edge depth; edge always wins depth keys.
                if inc_is_cdn && fields.contains_key(k) && !inc_is_edge {
                    continue;
                }
                fields.insert(k.clone(), v.clone());
            } else if k == "b8_via_cdn" || k == "b8_path_kind" || k == "b8_dual" {
                if let Some(prev) = fields.get(k).cloned() {
                    fields.insert(format!("{k}_prev"), prev);
                }
                fields.insert(k.clone(), v.clone());
            } else if !fields.contains_key(k) {
                fields.insert(k.clone(), v.clone());
            } else {
                fields.insert(k.clone(), v.clone());
            }
        }
        if (fields.contains_key("b8_via_cdn") || fields.contains_key("b8_via_cdn_prev"))
            && inc_is_edge
        {
            fields.insert("b8_edge_enriched".into(), json!(true));
        }
        if inc_is_cdn {
            fields.insert("b8_cdn_joined".into(), json!(true));
        }
    }
    if let Some(obj) = out.as_object_mut() {
        obj.insert("fields".into(), Value::Object(fields));
        if let Some(ip) = incoming.get("inject_path") {
            obj.entry("inject_path").or_insert_with(|| ip.clone());
        }
        obj.insert("early".into(), json!(true));
        obj.insert("merged".into(), json!(true));
    } else {
        out = incoming.clone();
    }
    out
}

fn upsert_batch_inner(
    c: &mut Client,
    session_id: &str,
    batch_id: &str,
    source: &str,
    payload: &Value,
    client_ip: Option<&str>,
) -> Result<Value, StoreError> {
    let ts = now_ms();
    let mut final_payload = payload.clone();
    // Dual-fire B8: merge fields so CDN join + edge TCP depth coexist.
    // iss/opus5 04-P0-2: read the previous payload from probe_cold (SSOT);
    // probe_batches.payload_json is only a legacy/debug copy (may be '').
    if batch_id == "B8_gateway" && source == "gateway" {
        let prev: Option<Value> = c
            .query_opt(
                "SELECT payload_z FROM probe_cold
                 WHERE session_id=$1 AND batch_id=$2 AND source=$3",
                &[&session_id, &batch_id, &source],
            )
            .ok()
            .flatten()
            .and_then(|r| {
                let z: Vec<u8> = r.get(0);
                crate::decompress_json_payload(&z).ok()
            })
            .or_else(|| {
                c.query_opt(
                    "SELECT payload_json FROM probe_batches
                     WHERE session_id=$1 AND batch_id=$2 AND source=$3 AND payload_json <> ''",
                    &[&session_id, &batch_id, &source],
                )
                .ok()
                .flatten()
                .and_then(|r| serde_json::from_str::<Value>(&r.get::<_, String>(0)).ok())
            });
        if let Some(prev) = prev {
            final_payload = merge_b8_payload(&prev, payload);
        }
    }
    let payload_s = serde_json::to_string(&final_payload)?;
    let exists = c
        .query_opt(
            "SELECT session_id FROM sessions WHERE session_id=$1",
            &[&session_id],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    if exists.is_none() {
        return Err(StoreError::NotFound(format!("session {session_id}")));
    }
    // Prefer explicit client_ip, else payload fields.server_client_ip
    let ip_owned: Option<String> = client_ip
        .map(|s| s.to_string())
        .or_else(|| {
            final_payload
                .pointer("/fields/server_client_ip")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        });
    let ip_bind_batch: Option<String> = ip_owned.clone();
    // iss/72: pre-read for ACK was_insert / same_capture / material columns.
    // iss/opus5 04-P0-2: same_capture keys off the material columns
    // (material_hash = client_payload_hash, capture_id); payload_json is only
    // consulted for legacy rows written before the material columns existed.
    let prev_row: Option<(i64, String, String, String)> = c
        .query_opt(
            "SELECT COALESCE(material_generation, 1), COALESCE(material_hash, ''),
                    COALESCE(capture_id, ''), payload_json
             FROM probe_batches
             WHERE session_id=$1 AND batch_id=$2 AND source=$3",
            &[&session_id, &batch_id, &source],
        )
        .ok()
        .flatten()
        .map(|r| {
            (
                r.get::<_, i64>(0),
                r.get::<_, String>(1),
                r.get::<_, String>(2),
                r.get::<_, String>(3),
            )
        });
    let was_insert = prev_row.is_none();
    let prev_gen = prev_row.as_ref().map(|p| p.0).unwrap_or(0);
    let prev_cols = prev_row
        .as_ref()
        .map(|p| (p.1.clone(), p.2.clone()));
    let prev_payload = prev_row
        .as_ref()
        .map(|p| p.3.clone())
        .filter(|s| !s.is_empty());
    let client_cap = final_payload
        .pointer("/fields/capture_id")
        .or_else(|| final_payload.get("capture_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let client_ph = final_payload
        .pointer("/fields/client_payload_hash")
        .or_else(|| final_payload.get("payload_hash"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let client_gen: i64 = final_payload
        .pointer("/fields/material_generation")
        .or_else(|| final_payload.get("material_generation"))
        .and_then(|v| v.as_i64())
        .unwrap_or(1)
        .max(1);
    let mut same_capture = false;
    if let Some((ref prev_hash, ref prev_cap_col)) = prev_cols {
        // Fast path: material columns (populated by all iss/72+ writers).
        if !client_cap.is_empty() && prev_cap_col == client_cap {
            same_capture = true;
        }
        if !same_capture && !client_ph.is_empty() && prev_hash == client_ph {
            same_capture = true;
        }
        // Legacy fallback: row predates material columns but still carries the
        // raw payload copy → parse capture_id / client_payload_hash from it.
        if !same_capture && prev_cap_col.is_empty() && prev_hash.is_empty() {
            if let Some(ref prev_s) = prev_payload {
                if let Ok(prev_v) = serde_json::from_str::<Value>(prev_s) {
                    let prev_cap = prev_v
                        .pointer("/fields/capture_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let prev_ph = prev_v
                        .pointer("/fields/client_payload_hash")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if !client_cap.is_empty() && prev_cap == client_cap {
                        same_capture = true;
                    }
                    if !same_capture && !client_ph.is_empty() && prev_ph == client_ph {
                        same_capture = true;
                    }
                }
            }
        }
    }
    if !was_insert && !same_capture && prev_gen > 0 && client_gen < prev_gen {
        return Ok(json!({
            "ok": false,
            "conflict": true,
            "error": "stale_material_generation",
            "session_id": session_id,
            "batch_id": batch_id,
            "source": source,
            "client_ip": ip_owned,
            "created_ms": ts,
            "was_insert": false,
            "unchanged": false,
            "same_capture": false,
            "material_generation": prev_gen,
            "client_material_generation": client_gen,
            "cold_written": false,
            "cold_error": null,
            "durability_state": "rejected",
        }));
    }
    if same_capture && !was_insert {
        // Transport retry of same capture: do not rewrite material.
        let touch_iv = crate::session_touch_min_interval_ms();
        let _ = c.execute(
            "UPDATE sessions SET updated_ms=$1
             WHERE session_id=$2
               AND (updated_ms IS NULL OR ($1 - updated_ms) >= $3)",
            &[&ts, &session_id, &touch_iv],
        );
        return Ok(json!({
            "ok": true,
            "session_id": session_id,
            "batch_id": batch_id,
            "source": source,
            "client_ip": ip_owned,
            "created_ms": ts,
            "analyze_scheduled": true,
            "analyze_debounce_ms": ANALYZE_DEBOUNCE_MS,
            "merged": final_payload.get("merged").cloned().unwrap_or(Value::Bool(false)),
            "was_insert": false,
            "unchanged": true,
            "same_capture": true,
            "material_generation": prev_gen.max(client_gen),
            "material_hash": client_ph,
            "capture_id": client_cap,
            "cold_skipped_unchanged": true,
            "cold_written": false,
            "cold_error": null,
            "durability_state": "duplicate_durable",
        }));
    }
    let mat_hash: Option<&str> = if client_ph.is_empty() {
        None
    } else {
        Some(client_ph)
    };
    let cap_opt: Option<&str> = if client_cap.is_empty() {
        None
    } else {
        Some(client_cap)
    };
    // Durable ingest: primary batch + cold + analyze job in one transaction.
    // iss/opus5 04-P0-2: payload_json stores '' unless the debug copy is
    // enabled; probe_cold.payload_z below is the single source of truth.
    let payload_bind: &str = if probe_batches_payload_enabled() {
        payload_s.as_str()
    } else {
        ""
    };
    c.batch_execute("BEGIN")
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    if let Err(e) = c.execute(
        "INSERT INTO probe_batches(session_id, batch_id, source, payload_json, created_ms, client_ip,
            material_generation, material_hash, capture_id)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)
         ON CONFLICT(session_id, batch_id, source) DO UPDATE SET
           payload_json=CASE WHEN EXCLUDED.payload_json = '' THEN probe_batches.payload_json
                             ELSE EXCLUDED.payload_json END,
           created_ms=LEAST(probe_batches.created_ms, EXCLUDED.created_ms),
           client_ip=COALESCE(EXCLUDED.client_ip, probe_batches.client_ip),
           material_generation=EXCLUDED.material_generation,
           material_hash=EXCLUDED.material_hash,
           capture_id=EXCLUDED.capture_id
         WHERE probe_batches.payload_json IS DISTINCT FROM EXCLUDED.payload_json
            OR probe_batches.client_ip IS DISTINCT FROM EXCLUDED.client_ip
            OR COALESCE(probe_batches.material_generation,1) < EXCLUDED.material_generation",
        &[
            &session_id,
            &batch_id,
            &source,
            &payload_bind,
            &ts,
            &ip_bind_batch,
            &client_gen,
            &mat_hash,
            &cap_opt,
        ],
    ) {
        let _ = c.batch_execute("ROLLBACK");
        return Err(StoreError::Msg(e.to_string()));
    }
    // Throttle session touch: inactivity clock does not need per-batch resolution.
    let touch_iv = crate::session_touch_min_interval_ms();
    if let Some(ref ip) = ip_owned {
        let _ = c.execute(
            "UPDATE sessions SET updated_ms=$1, client_ip=COALESCE($2, client_ip)
             WHERE session_id=$3
               AND (
                 client_ip IS DISTINCT FROM $2
                 OR updated_ms IS NULL
                 OR ($1 - updated_ms) >= $4
               )",
            &[&ts, ip, &session_id, &touch_iv],
        );
    } else {
        let _ = c.execute(
            "UPDATE sessions SET updated_ms=$1
             WHERE session_id=$2
               AND (updated_ms IS NULL OR ($1 - updated_ms) >= $3)",
            &[&ts, &session_id, &touch_iv],
        );
    }
    // Cold compact JSON store (always write for query path)
    let vt: String = c
        .query_opt(
            "SELECT COALESCE(visitor_terminal_id,'') FROM sessions WHERE session_id=$1",
            &[&session_id],
        )
        .ok()
        .flatten()
        .map(|r| r.get::<_, String>(0))
        .unwrap_or_default();
    let fields_compact = crate::compact_probe_payload(&final_payload);
    // Bind as TEXT then cast to jsonb in SQL (portable; avoids Json wrapper deps).
    let fields_s = serde_json::to_string(&fields_compact).unwrap_or_else(|_| "{}".into());
    let payload_z_bytes = crate::compress_json_payload(&final_payload).unwrap_or_else(|_| {
        serde_json::to_vec(&final_payload).unwrap_or_default()
    });
    // BYTEA bind: raw deflate bytes (≈½ hex TEXT).
    let ip_bind: Option<String> = ip_owned.clone();
    // Bind fields as text + cast in SQL (`$n::jsonb` alone confuses rust-postgres type inference).
    // Bind payload as raw BYTEA (Vec<u8>).
    // Dual-write cold, but no-op UPDATE when compact fields + payload_z unchanged
    // (prod: ~equal n_tup_upd to n_tup_ins from always-rewrite ON CONFLICT).
    let cold_n = c
        .execute(
            "INSERT INTO probe_cold(session_id, visitor_terminal_id, batch_id, source, client_ip, fields_json, payload_z, created_ms)
             VALUES($1,$2,$3,$4,$5, CAST($6 AS text)::jsonb, $7, $8)
             ON CONFLICT(session_id, batch_id, source) DO UPDATE SET
               client_ip=COALESCE(EXCLUDED.client_ip, probe_cold.client_ip),
               fields_json=EXCLUDED.fields_json,
               payload_z=EXCLUDED.payload_z,
               visitor_terminal_id=EXCLUDED.visitor_terminal_id,
               created_ms=LEAST(probe_cold.created_ms, EXCLUDED.created_ms)
             WHERE probe_cold.fields_json IS DISTINCT FROM EXCLUDED.fields_json
                OR probe_cold.payload_z IS DISTINCT FROM EXCLUDED.payload_z
                OR probe_cold.client_ip IS DISTINCT FROM EXCLUDED.client_ip
                OR probe_cold.visitor_terminal_id IS DISTINCT FROM EXCLUDED.visitor_terminal_id",
            &[
                &session_id,
                &vt,
                &batch_id,
                &source,
                &ip_bind,
                &fields_s,
                &payload_z_bytes,
                &ts,
            ],
        );
    let (cold_written, cold_err, cold_skipped): (bool, Option<String>, bool) = match cold_n {
        Ok(n) => (true, None, n == 0),
        Err(e) => {
            let _ = c.batch_execute("ROLLBACK");
            return Ok(json!({
                "ok": false,
                "error": "cold_write_failed",
                "session_id": session_id,
                "batch_id": batch_id,
                "source": source,
                "client_ip": ip_owned,
                "created_ms": ts,
                "analyze_scheduled": false,
                "was_insert": was_insert,
                "same_capture": false,
                "material_generation": client_gen,
                "material_hash": client_ph,
                "capture_id": client_cap,
                "cold_written": false,
                "cold_error": e.to_string(),
                "durability_state": "failed",
            }));
        }
    };
    if let Err(e) = schedule_analyze_inner(
        c,
        session_id,
        ANALYZE_DEBOUNCE_MS,
        AnalyzeDueMerge::PullEarlier,
    ) {
        let _ = c.batch_execute("ROLLBACK");
        return Ok(json!({
            "ok": false,
            "error": "analyze_schedule_failed",
            "detail": e.to_string(),
            "session_id": session_id,
            "batch_id": batch_id,
            "source": source,
            "cold_written": cold_written,
            "durability_state": "failed",
        }));
    }
    if let Err(e) = c.batch_execute("COMMIT") {
        let _ = c.batch_execute("ROLLBACK");
        return Err(StoreError::Msg(e.to_string()));
    }
    Ok(json!({
        "ok": true,
        "session_id": session_id,
        "batch_id": batch_id,
        "source": source,
        "client_ip": ip_owned,
        "created_ms": ts,
        "analyze_scheduled": true,
        "analyze_debounce_ms": ANALYZE_DEBOUNCE_MS,
        "merged": final_payload.get("merged").cloned().unwrap_or(Value::Bool(false)),
        "was_insert": was_insert,
        "same_capture": false,
        "material_generation": client_gen,
        "material_hash": client_ph,
        "capture_id": client_cap,
        "unchanged": cold_skipped && !was_insert,
        "cold_written": cold_written,
        "cold_skipped_unchanged": cold_skipped,
        "cold_error": cold_err,
        "cold_fields_bytes": fields_s.len(),
        "cold_payload_z_bytes": payload_z_bytes.len(),
        "session_touch_min_interval_ms": crate::session_touch_min_interval_ms(),
        "durability_state": "stored_durable",
    }))
}

fn get_probe_cold_inner(
    c: &mut Client,
    session_id: &str,
    batch_id: &str,
    source: Option<&str>,
) -> Result<Value, StoreError> {
    let row = if let Some(src) = source {
        c.query_opt(
            "SELECT source, client_ip, fields_json::text, payload_z, octet_length(payload_z), created_ms
             FROM probe_cold WHERE session_id=$1 AND batch_id=$2 AND source=$3",
            &[&session_id, &batch_id, &src],
        )
    } else {
        c.query_opt(
            "SELECT source, client_ip, fields_json::text, payload_z, octet_length(payload_z), created_ms
             FROM probe_cold WHERE session_id=$1 AND batch_id=$2
             ORDER BY created_ms DESC LIMIT 1",
            &[&session_id, &batch_id],
        )
    }
    .map_err(|e| StoreError::Msg(e.to_string()))?;
    let Some(r) = row else {
        return Ok(json!({"ok": true, "found": false, "session_id": session_id, "batch_id": batch_id}));
    };
    let src: String = r.get(0);
    let cip: Option<String> = r.get(1);
    let fields_s: String = r.get(2);
    let payload_z: Vec<u8> = r.get(3);
    let z_len: i32 = r.get(4);
    let created_ms: i64 = r.get(5);
    let fields: Value = serde_json::from_str(&fields_s).unwrap_or(json!({}));
    let full = crate::decompress_json_payload(&payload_z).ok();
    let full_ip = full
        .as_ref()
        .and_then(|v| v.pointer("/fields/server_client_ip"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Ok(json!({
        "ok": true,
        "found": true,
        "session_id": session_id,
        "batch_id": batch_id,
        "source": src,
        "client_ip": cip,
        "fields_json": fields,
        "payload_z_bytes": z_len,
        "payload_full": full,
        "payload_full_ip": full_ip,
        "created_ms": created_ms,
    }))
}

fn list_cold_for_vt_inner(
    c: &mut Client,
    visitor_terminal_id: &str,
    since_ms: i64,
    limit: i64,
) -> Result<Value, StoreError> {
    let lim = limit.clamp(1, 100);
    let rows = c
        .query(
            "SELECT session_id, batch_id, source, client_ip, fields_json::text, payload_z, created_ms
             FROM probe_cold
             WHERE visitor_terminal_id = $1 AND created_ms >= $2
             ORDER BY created_ms DESC
             LIMIT $3",
            &[&visitor_terminal_id, &since_ms, &lim],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    let mut out = Vec::new();
    for r in rows {
        let payload_z: Vec<u8> = r.get(5);
        let full = crate::decompress_json_payload(&payload_z).ok();
        let fields_s: String = r.get(4);
        let fields: Value = serde_json::from_str(&fields_s).unwrap_or(json!({}));
        out.push(json!({
            "session_id": r.get::<_, String>(0),
            "batch_id": r.get::<_, String>(1),
            "source": r.get::<_, String>(2),
            "client_ip": r.get::<_, Option<String>>(3),
            "fields_json": fields,
            "payload_full": full,
            "created_ms": r.get::<_, i64>(6),
        }));
    }
    Ok(json!({
        "ok": true,
        "visitor_terminal_id": visitor_terminal_id,
        "since_ms": since_ms,
        "count": out.len(),
        "rows": out,
    }))
}

fn purge_expired_cold_inner(c: &mut Client, older_than_ms: i64) -> Result<i64, StoreError> {
    let n = c
        .execute(
            "DELETE FROM probe_cold WHERE created_ms < $1",
            &[&older_than_ms],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    Ok(n as i64)
}

/// Small-batch retention: analysis rows then orphaned sessions, optional velocity_hits.
/// iss/opus5 04-P1-6: also bounds the previously unbounded tables —
/// `ops_older_than_ms` covers observation_events / ops_*_events /
/// api_idempotency; `master_older_than_ms` covers devices / device_sessions /
/// device_index_* / soft_edges / soft_heat. page_results follows
/// `older_than_ms` (same lifecycle as analysis_results).
fn retention_purge_batch_inner(
    c: &mut Client,
    older_than_ms: i64,
    limit: i64,
    velocity_older_than_ms: Option<i64>,
    ops_older_than_ms: Option<i64>,
    master_older_than_ms: Option<i64>,
) -> Result<Value, StoreError> {
    let lim = limit.clamp(10, 5000);
    // Serialize concurrent background + panel purge (each call still ≤ limit).
    let _ = c.execute("SELECT pg_advisory_lock(872365002)", &[]);
    let out = (|| -> Result<Value, StoreError> {
    // analysis_results by created_ms
    let n_analysis = c
        .execute(
            r#"
            DELETE FROM analysis_results
            WHERE ctid IN (
              SELECT ctid FROM analysis_results
              WHERE created_ms < $1
              ORDER BY created_ms ASC
              LIMIT $2
            )
            "#,
            &[&older_than_ms, &lim],
        )
        .map_err(|e| StoreError::Msg(format!("retention analysis: {e}")))?
        as i64;
    // analysis_latest if present
    let n_latest = c
        .execute(
            r#"
            DELETE FROM analysis_latest
            WHERE ctid IN (
              SELECT ctid FROM analysis_latest
              WHERE created_ms < $1
              ORDER BY created_ms ASC
              LIMIT $2
            )
            "#,
            &[&older_than_ms, &lim],
        )
        .unwrap_or(0) as i64;
    // sessions idle/old without recent analysis (best-effort)
    let n_sessions = c
        .execute(
            r#"
            DELETE FROM sessions
            WHERE session_id IN (
              SELECT s.session_id FROM sessions s
              WHERE s.updated_ms < $1
                AND NOT EXISTS (
                  SELECT 1 FROM analysis_results ar WHERE ar.session_id = s.session_id
                )
              ORDER BY s.updated_ms ASC
              LIMIT $2
            )
            "#,
            &[&older_than_ms, &lim],
        )
        .unwrap_or(0) as i64;
    // orphan batches for missing sessions
    let n_batches = c
        .execute(
            r#"
            DELETE FROM probe_batches
            WHERE ctid IN (
              SELECT pb.ctid FROM probe_batches pb
              WHERE NOT EXISTS (SELECT 1 FROM sessions s WHERE s.session_id = pb.session_id)
              LIMIT $1
            )
            "#,
            &[&lim],
        )
        .unwrap_or(0) as i64;
    let mut n_velocity = 0i64;
    if let Some(v_old) = velocity_older_than_ms {
        n_velocity = c
            .execute(
                r#"
                DELETE FROM velocity_hits
                WHERE ctid IN (
                  SELECT ctid FROM velocity_hits WHERE hit_ms < $1 ORDER BY hit_ms ASC LIMIT $2
                )
                "#,
                &[&v_old, &lim],
            )
            .unwrap_or(0) as i64;
    }
    // cold TTL aligned with older_than when colder
    let n_cold = purge_expired_cold_inner(c, older_than_ms).unwrap_or(0);
    // iss/opus5 04-P1-6: page_results share the analysis lifecycle window.
    let n_page = c
        .execute(
            r#"
            DELETE FROM page_results
            WHERE ctid IN (
              SELECT ctid FROM page_results
              WHERE created_ms < $1 ORDER BY created_ms ASC LIMIT $2
            )
            "#,
            &[&older_than_ms, &lim],
        )
        .unwrap_or(0) as i64;
    let mut n_obs = 0i64;
    let mut n_ops_client = 0i64;
    let mut n_ops_server = 0i64;
    let mut n_idem = 0i64;
    if let Some(ops_old) = ops_older_than_ms {
        n_obs = c
            .execute(
                r#"
                DELETE FROM observation_events
                WHERE ctid IN (
                  SELECT ctid FROM observation_events
                  WHERE created_ms < $1 ORDER BY created_ms ASC LIMIT $2
                )
                "#,
                &[&ops_old, &lim],
            )
            .unwrap_or(0) as i64;
        n_ops_client = c
            .execute(
                r#"
                DELETE FROM ops_client_events
                WHERE ctid IN (
                  SELECT ctid FROM ops_client_events
                  WHERE ts_ms < $1 ORDER BY ts_ms ASC LIMIT $2
                )
                "#,
                &[&ops_old, &lim],
            )
            .unwrap_or(0) as i64;
        n_ops_server = c
            .execute(
                r#"
                DELETE FROM ops_server_events
                WHERE ctid IN (
                  SELECT ctid FROM ops_server_events
                  WHERE ts_ms < $1 ORDER BY ts_ms ASC LIMIT $2
                )
                "#,
                &[&ops_old, &lim],
            )
            .unwrap_or(0) as i64;
        n_idem = c
            .execute(
                r#"
                DELETE FROM api_idempotency
                WHERE ctid IN (
                  SELECT ctid FROM api_idempotency
                  WHERE created_ms < $1 ORDER BY created_ms ASC LIMIT $2
                )
                "#,
                &[&ops_old, &lim],
            )
            .unwrap_or(0) as i64;
    }
    let mut n_devices = 0i64;
    let mut n_dev_sessions = 0i64;
    let mut n_soft_edges = 0i64;
    let mut n_soft_heat = 0i64;
    let mut n_di_devices = 0i64;
    let mut n_di_keys = 0i64;
    if let Some(master_old) = master_older_than_ms {
        // devices last_seen beyond the master window; device_sessions rows
        // cascade via FK, aged memberships deleted explicitly below.
        n_devices = c
            .execute(
                r#"
                DELETE FROM devices
                WHERE ctid IN (
                  SELECT ctid FROM devices
                  WHERE last_seen_ms < $1 ORDER BY last_seen_ms ASC LIMIT $2
                )
                "#,
                &[&master_old, &lim],
            )
            .unwrap_or(0) as i64;
        n_dev_sessions = c
            .execute(
                r#"
                DELETE FROM device_sessions
                WHERE ctid IN (
                  SELECT ctid FROM device_sessions
                  WHERE created_ms < $1 ORDER BY created_ms ASC LIMIT $2
                )
                "#,
                &[&master_old, &lim],
            )
            .unwrap_or(0) as i64;
        n_soft_edges = c
            .execute(
                r#"
                DELETE FROM soft_edges
                WHERE ctid IN (
                  SELECT ctid FROM soft_edges
                  WHERE created_ms < $1 ORDER BY created_ms ASC LIMIT $2
                )
                "#,
                &[&master_old, &lim],
            )
            .unwrap_or(0) as i64;
        n_soft_heat = c
            .execute(
                r#"
                DELETE FROM soft_heat
                WHERE ctid IN (
                  SELECT ctid FROM soft_heat
                  WHERE last_ms < $1 ORDER BY last_ms ASC LIMIT $2
                )
                "#,
                &[&master_old, &lim],
            )
            .unwrap_or(0) as i64;
        n_di_devices = c
            .execute(
                r#"
                DELETE FROM device_index_devices
                WHERE ctid IN (
                  SELECT ctid FROM device_index_devices
                  WHERE updated_ms < $1 ORDER BY updated_ms ASC LIMIT $2
                )
                "#,
                &[&master_old, &lim],
            )
            .unwrap_or(0) as i64;
        // device_index_keys has no timestamp: drop keys whose device left the index.
        n_di_keys = c
            .execute(
                r#"
                DELETE FROM device_index_keys
                WHERE ctid IN (
                  SELECT k.ctid FROM device_index_keys k
                  WHERE NOT EXISTS (
                    SELECT 1 FROM device_index_devices d
                    WHERE d.tenant_id = k.tenant_id AND d.device_id = k.device_id
                  )
                  LIMIT $1
                )
                "#,
                &[&lim],
            )
            .unwrap_or(0) as i64;
    }
    Ok(json!({
        "ok": true,
        "deleted_analysis": n_analysis,
        "deleted_analysis_latest": n_latest,
        "deleted_sessions": n_sessions,
        "deleted_batches": n_batches,
        "deleted_velocity": n_velocity,
        "deleted_cold": n_cold,
        "deleted_page_results": n_page,
        "deleted_observation_events": n_obs,
        "deleted_ops_client_events": n_ops_client,
        "deleted_ops_server_events": n_ops_server,
        "deleted_api_idempotency": n_idem,
        "deleted_devices": n_devices,
        "deleted_device_sessions": n_dev_sessions,
        "deleted_soft_edges": n_soft_edges,
        "deleted_soft_heat": n_soft_heat,
        "deleted_device_index_devices": n_di_devices,
        "deleted_device_index_keys": n_di_keys,
        "limit": lim,
        "older_than_ms": older_than_ms,
    }))
    })();
    let _ = c.execute("SELECT pg_advisory_unlock(872365002)", &[]);
    out
}

// ---------------------------------------------------------------------------
// iss/opus5 05-S-5: DSAR subject erase / export.
// ---------------------------------------------------------------------------

/// Local copy of the network-class mask (store crate has no core dep).
/// Must match gr-probe-core privacy::mask_ip_subnet so a DSAR selector given
/// in either raw or masked form matches both legacy (raw) and new (masked)
/// rows.
fn dsar_mask_ip(ip: &str) -> String {
    let t = ip.trim();
    if t.is_empty() || t.contains('/') {
        return t.to_string();
    }
    if let Ok(v4) = t.parse::<std::net::Ipv4Addr>() {
        let o = v4.octets();
        return format!("{}.{}.{}.0/24", o[0], o[1], o[2]);
    }
    if let Ok(v6) = t.parse::<std::net::Ipv6Addr>() {
        let s = v6.segments();
        return format!("{:x}:{:x}:{:x}::/48", s[0], s[1], s[2]);
    }
    t.to_string()
}

/// Max sessions a single DSAR call will touch — forces a narrower selector
/// instead of a table-wide cascade by accident.
const DSAR_MAX_SESSIONS: usize = 50_000;

/// Resolve the session set for a subject selector.
fn dsar_resolve_sessions(
    c: &mut Client,
    kind: &str,
    value: &str,
) -> Result<Vec<String>, StoreError> {
    let ip_masked = dsar_mask_ip(value);
    let rows = match kind {
        "visitor_terminal_id" | "vt" => c.query(
            "SELECT session_id FROM sessions WHERE visitor_terminal_id=$1",
            &[&value],
        ),
        "device_id" => c.query(
            "SELECT session_id FROM analysis_results WHERE device_id=$1
             UNION SELECT session_id FROM analysis_latest WHERE device_id=$1
             UNION SELECT session_id FROM device_sessions WHERE device_id=$1",
            &[&value],
        ),
        // Match raw AND masked stored forms (legacy vs post-S-4 rows).
        "client_ip" => c.query(
            "SELECT session_id FROM sessions WHERE client_ip=$1 OR client_ip=$2",
            &[&value, &ip_masked],
        ),
        "site_id" => c.query(
            "SELECT session_id FROM sessions
             WHERE meta_json->>'site_id'=$1 OR meta_json->>'siteId'=$1",
            &[&value],
        ),
        _ => {
            return Err(StoreError::Msg(format!(
                "unknown subject kind {kind} (want visitor_terminal_id|device_id|client_ip|site_id)"
            )))
        }
    }
    .map_err(|e| StoreError::Msg(format!("dsar resolve: {e}")))?;
    let out: Vec<String> = rows.iter().map(|r| r.get(0)).collect();
    if out.len() > DSAR_MAX_SESSIONS {
        return Err(StoreError::Msg(format!(
            "dsar selector too broad: {} sessions > {} cap",
            out.len(),
            DSAR_MAX_SESSIONS
        )));
    }
    Ok(out)
}

/// Cascade-delete everything held for one subject. Returns per-table counts.
fn dsar_erase_inner(c: &mut Client, kind: &str, value: &str) -> Result<Value, StoreError> {
    let sessions = dsar_resolve_sessions(c, kind, value)?;
    let ip_masked = dsar_mask_ip(value);
    // Serialize with retention purge (same advisory lock family).
    let _ = c.execute("SELECT pg_advisory_lock(872365002)", &[]);
    let out = (|| -> Result<Value, StoreError> {
        let mut counts = Map::new();
        let mut del_any = |c: &mut Client, name: &str, sql: &str, params: &[&(dyn postgres::types::ToSql + Sync)]| {
            let n = c.execute(sql, params).unwrap_or(0) as i64;
            if n > 0 {
                counts.insert(name.to_string(), json!(n));
            }
            n
        };
        // 1) Session-scoped cascade.
        if !sessions.is_empty() {
            let s = &sessions;
            for (name, table) in [
                ("analyze_jobs", "analyze_jobs"),
                ("page_results", "page_results"),
                ("observation_events", "observation_events"),
                ("analysis_results", "analysis_results"),
                ("analysis_latest", "analysis_latest"),
                ("probe_batches", "probe_batches"),
                ("probe_cold", "probe_cold"),
                ("ops_client_events", "ops_client_events"),
                ("ops_server_events", "ops_server_events"),
                ("device_sessions", "device_sessions"),
                ("soft_heat", "soft_heat"),
                ("velocity_hits", "velocity_hits"),
            ] {
                let sql = format!("DELETE FROM {table} WHERE session_id = ANY($1)");
                del_any(c, name, &sql, &[s]);
            }
            // soft_edges keys on a/b session columns.
            del_any(
                c,
                "soft_edges",
                "DELETE FROM soft_edges WHERE a_session = ANY($1) OR b_session = ANY($1)",
                &[s],
            );
            del_any(c, "sessions", "DELETE FROM sessions WHERE session_id = ANY($1)", &[s]);
        }
        // 2) Selector-direct deletes (rows not reachable via the session set).
        match kind {
            "visitor_terminal_id" | "vt" => {
                del_any(c, "visitor_terminals", "DELETE FROM visitor_terminals WHERE vt_id=$1", &[&value]);
                del_any(c, "probe_cold", "DELETE FROM probe_cold WHERE visitor_terminal_id=$1", &[&value]);
                del_any(c, "ops_client_events", "DELETE FROM ops_client_events WHERE visitor_terminal_id=$1", &[&value]);
                del_any(c, "ops_server_events", "DELETE FROM ops_server_events WHERE visitor_terminal_id=$1", &[&value]);
            }
            "device_id" => {
                // device_sessions cascades from devices via FK; delete explicitly
                // anyway for rows whose session set missed them.
                del_any(c, "device_sessions", "DELETE FROM device_sessions WHERE device_id=$1", &[&value]);
                del_any(c, "devices", "DELETE FROM devices WHERE device_id=$1", &[&value]);
                del_any(c, "device_index_devices", "DELETE FROM device_index_devices WHERE device_id=$1", &[&value]);
                del_any(c, "device_index_keys", "DELETE FROM device_index_keys WHERE device_id=$1", &[&value]);
                del_any(c, "soft_heat", "DELETE FROM soft_heat WHERE device_id=$1", &[&value]);
                del_any(c, "velocity_hits", "DELETE FROM velocity_hits WHERE key_value=$1", &[&value]);
                del_any(c, "analysis_results", "UPDATE analysis_results SET device_id=NULL WHERE device_id=$1", &[&value]);
                del_any(c, "analysis_latest", "UPDATE analysis_latest SET device_id=NULL WHERE device_id=$1", &[&value]);
            }
            "client_ip" => {
                for (name, table, col) in [
                    ("probe_batches", "probe_batches", "client_ip"),
                    ("probe_cold", "probe_cold", "client_ip"),
                    ("device_sessions", "device_sessions", "client_ip"),
                    ("ops_client_events", "ops_client_events", "client_ip"),
                    ("ops_server_events", "ops_server_events", "client_ip"),
                ] {
                    let sql = format!("DELETE FROM {table} WHERE {col}=$1 OR {col}=$2");
                    del_any(c, name, &sql, &[&value, &ip_masked]);
                }
                del_any(c, "devices", "UPDATE devices SET last_client_ip=NULL WHERE last_client_ip=$1 OR last_client_ip=$2", &[&value, &ip_masked]);
                del_any(c, "analysis_results", "UPDATE analysis_results SET client_ip=NULL WHERE client_ip=$1 OR client_ip=$2", &[&value, &ip_masked]);
                del_any(c, "analysis_latest", "UPDATE analysis_latest SET client_ip=NULL WHERE client_ip=$1 OR client_ip=$2", &[&value, &ip_masked]);
            }
            "site_id" => {
                // Site teardown cascade (05-S-5: delete_site previously removed
                // only control-plane rows). analysis tables have no site column
                // on the results side — their sessions were cascaded above;
                // analysis_latest carries site_id directly.
                del_any(c, "ops_client_events", "DELETE FROM ops_client_events WHERE site_id=$1", &[&value]);
                del_any(c, "ops_server_events", "DELETE FROM ops_server_events WHERE site_id=$1", &[&value]);
                del_any(c, "observation_events", "DELETE FROM observation_events WHERE tenant_id=$1", &[&value]);
                del_any(c, "analysis_latest", "DELETE FROM analysis_latest WHERE site_id=$1", &[&value]);
            }
            _ => {}
        }
        Ok(json!({
            "ok": true,
            "subject_kind": kind,
            "sessions_matched": sessions.len(),
            "deleted": counts,
        }))
    })();
    let _ = c.execute("SELECT pg_advisory_unlock(872365002)", &[]);
    out
}

/// Export everything held for one subject as a JSON bundle (DSAR access).
fn dsar_export_inner(c: &mut Client, kind: &str, value: &str) -> Result<Value, StoreError> {
    let sessions = dsar_resolve_sessions(c, kind, value)?;
    let ip_masked = dsar_mask_ip(value);
    let lim: i64 = 5000;
    let mut out = Map::new();
    out.insert("sessions_matched".into(), json!(sessions.len()));
    out.insert("session_ids".into(), json!(sessions));
    if !sessions.is_empty() {
        let s = &sessions;
        let rows = c
            .query(
                "SELECT session_id, visitor_terminal_id, created_ms, updated_ms, client_ip, meta_json
                 FROM sessions WHERE session_id = ANY($1) LIMIT $2",
                &[s, &lim],
            )
            .map_err(|e| StoreError::Msg(e.to_string()))?;
        let sess: Vec<Value> = rows
            .iter()
            .map(|r| {
                json!({
                    "session_id": r.get::<_, String>(0),
                    "visitor_terminal_id": r.get::<_, Option<String>>(1),
                    "created_ms": r.get::<_, i64>(2),
                    "updated_ms": r.get::<_, i64>(3),
                    "client_ip": r.get::<_, Option<String>>(4),
                    "meta": serde_json::from_str::<Value>(&r.get::<_, String>(5)).unwrap_or(json!({})),
                })
            })
            .collect();
        out.insert("sessions".into(), json!(sess));
        // Cold probe fields subset (fields_json) — full payloads stay internal.
        let rows = c
            .query(
                "SELECT session_id, batch_id, source, fields_json::text, created_ms
                 FROM probe_cold WHERE session_id = ANY($1) ORDER BY created_ms ASC LIMIT $2",
                &[s, &lim],
            )
            .map_err(|e| StoreError::Msg(e.to_string()))?;
        let cold: Vec<Value> = rows
            .iter()
            .map(|r| {
                json!({
                    "session_id": r.get::<_, String>(0),
                    "batch_id": r.get::<_, String>(1),
                    "source": r.get::<_, String>(2),
                    "fields": serde_json::from_str::<Value>(&r.get::<_, String>(3)).unwrap_or(json!({})),
                    "created_ms": r.get::<_, i64>(4),
                })
            })
            .collect();
        out.insert("probe_fields".into(), json!(cold));
        let rows = c
            .query(
                "SELECT session_id, rev, result_json, created_ms FROM analysis_results
                 WHERE session_id = ANY($1) ORDER BY created_ms ASC LIMIT $2",
                &[s, &lim],
            )
            .map_err(|e| StoreError::Msg(e.to_string()))?;
        let ana: Vec<Value> = rows
            .iter()
            .map(|r| {
                let raw: String = r.get(2);
                let decoded = crate::decode_analysis_result_json(&raw).unwrap_or(json!({}));
                json!({
                    "session_id": r.get::<_, String>(0),
                    "rev": r.get::<_, i64>(1),
                    "result": decoded,
                    "created_ms": r.get::<_, i64>(3),
                })
            })
            .collect();
        out.insert("analysis_results".into(), json!(ana));
    }
    // Selector-direct extras.
    match kind {
        "device_id" => {
            let rows = c
                .query(
                    "SELECT device_id, first_seen_ms, last_seen_ms, tier_last, session_count, distinct_ip_count, distinct_vt_count
                     FROM devices WHERE device_id=$1",
                    &[&value],
                )
                .map_err(|e| StoreError::Msg(e.to_string()))?;
            let devs: Vec<Value> = rows
                .iter()
                .map(|r| {
                    json!({
                        "device_id": r.get::<_, String>(0),
                        "first_seen_ms": r.get::<_, i64>(1),
                        "last_seen_ms": r.get::<_, i64>(2),
                        "tier_last": r.get::<_, Option<String>>(3),
                        "session_count": r.get::<_, i64>(4),
                        "distinct_ip_count": r.get::<_, i64>(5),
                        "distinct_vt_count": r.get::<_, i64>(6),
                    })
                })
                .collect();
            out.insert("devices".into(), json!(devs));
        }
        "client_ip" => {
            let rows = c
                .query(
                    "SELECT id, ts_ms, code, severity FROM ops_server_events
                     WHERE client_ip=$1 OR client_ip=$2 ORDER BY ts_ms ASC LIMIT $3",
                    &[&value, &ip_masked, &lim],
                )
                .unwrap_or_default();
            let ev: Vec<Value> = rows
                .iter()
                .map(|r| {
                    json!({
                        "id": r.get::<_, i64>(0),
                        "ts_ms": r.get::<_, i64>(1),
                        "code": r.get::<_, String>(2),
                        "severity": r.get::<_, String>(3),
                    })
                })
                .collect();
            out.insert("ops_server_events".into(), json!(ev));
        }
        _ => {}
    }
    Ok(json!({
        "ok": true,
        "subject_kind": kind,
        "export": Value::Object(out),
        "note": "dsar_export_v1 — probe payloads exported as fields subset; raw payloads are internal-only",
    }))
}

fn probe_volume_stats_inner(c: &mut Client, session_id: &str) -> Result<Value, StoreError> {
    let batch = c
        .query_one(
            "SELECT count(*)::bigint,
                    coalesce(sum(length(payload_json)),0)::bigint,
                    coalesce(sum(length(client_ip)),0)::bigint
             FROM probe_batches WHERE session_id=$1",
            &[&session_id],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    let cold = c
        .query_one(
            "SELECT count(*)::bigint,
                    coalesce(sum(octet_length(payload_z)),0)::bigint,
                    coalesce(sum(length(fields_json::text)),0)::bigint,
                    count(*) FILTER (WHERE source='hot_demote')::bigint
             FROM probe_cold WHERE session_id=$1",
            &[&session_id],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    let batch_n: i64 = batch.get(0);
    let batch_bytes: i64 = batch.get(1);
    let cold_n: i64 = cold.get(0);
    let cold_z: i64 = cold.get(1);
    let cold_fields: i64 = cold.get(2);
    let hot_demote_n: i64 = cold.get(3);
    Ok(json!({
        "ok": true,
        "session_id": session_id,
        "probe_batches": {"rows": batch_n, "payload_json_bytes": batch_bytes},
        "probe_cold": {
            "rows": cold_n,
            "payload_z_bytes": cold_z,
            "fields_json_bytes": cold_fields,
            "hot_demote_rows": hot_demote_n,
        },
        "ratio_z_over_batch": if batch_bytes > 0 {
            (cold_z as f64) / (batch_bytes as f64)
        } else {
            0.0
        },
    }))
}

fn cross_query_analysis_inner(
    c: &mut Client,
    device_id: Option<&str>,
    client_ip: Option<&str>,
    bot_verdict: Option<&str>,
    real_band: Option<&str>,
    field_key: Option<&str>,
    field_value: Option<&str>,
    limit: i64,
) -> Result<Value, StoreError> {
    let lim = limit.clamp(1, 500);
    // P0: prefer analysis_latest (one row / session, no TOAST / no DISTINCT ON history).
    // Fallback path still available if table empty (pre-backfill).
    let sql = r#"
SELECT
  al.session_id, al.rev, al.real_band, al.device_id, al.bot_verdict,
  al.device_confidence, al.client_ip, al.created_ms,
  COALESCE(al.visitor_terminal_id, s.visitor_terminal_id) AS visitor_terminal_id,
  al.device_tier, al.collision_risk, al.residual_entropy_ok,
  al.site_id, al.os_family, al.form_class, al.digest_path, al.product_version
FROM analysis_latest al
LEFT JOIN sessions s ON s.session_id = al.session_id
WHERE ($1::text IS NULL OR al.device_id = $1)
  AND ($2::text IS NULL OR al.client_ip = $2 OR EXISTS (
        SELECT 1 FROM probe_cold pc
        WHERE pc.session_id = al.session_id AND pc.client_ip = $2
      ))
  AND ($3::text IS NULL OR al.bot_verdict = $3)
  AND ($4::text IS NULL OR al.real_band = $4)
  AND ($5::text IS NULL OR $6::text IS NULL OR EXISTS (
        SELECT 1 FROM probe_cold pc
        WHERE pc.session_id = al.session_id
          AND (
            pc.fields_json #>> ARRAY['fields', $5] = $6
            OR pc.fields_json #>> ARRAY[$5] = $6
          )
      ))
ORDER BY al.created_ms DESC
LIMIT $7
"#;
    let rows = c
        .query(
            sql,
            &[
                &device_id,
                &client_ip,
                &bot_verdict,
                &real_band,
                &field_key,
                &field_value,
                &lim,
            ],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(json!({
            "session_id": r.get::<_, String>(0),
            "rev": r.get::<_, i64>(1),
            "real_band": r.get::<_, Option<String>>(2),
            "device_id": r.get::<_, Option<String>>(3),
            "bot_verdict": r.get::<_, Option<String>>(4),
            "device_confidence": r.get::<_, Option<f64>>(5),
            "client_ip": r.get::<_, Option<String>>(6),
            "created_ms": r.get::<_, i64>(7),
            "visitor_terminal_id": r.get::<_, Option<String>>(8),
            "device_tier": r.get::<_, Option<String>>(9),
            "collision_risk": r.get::<_, Option<bool>>(10),
            "residual_entropy_ok": r.get::<_, Option<bool>>(11),
            "site_id": r.get::<_, Option<String>>(12),
            "os_family": r.get::<_, Option<String>>(13),
            "form_class": r.get::<_, Option<String>>(14),
            "digest_path": r.get::<_, Option<String>>(15),
            "product_version": r.get::<_, Option<String>>(16),
        }));
    }
    // Pre-backfill empty: fall back to DISTINCT ON history once (still no result_json).
    if out.is_empty() {
        let fb = cross_query_analysis_history_fallback(
            c, device_id, client_ip, bot_verdict, real_band, field_key, field_value, lim,
        )?;
        if let Some(arr) = fb.get("rows").and_then(|v| v.as_array()) {
            if !arr.is_empty() {
                return Ok(fb);
            }
        }
    }
    Ok(json!({
        "ok": true,
        "count": out.len(),
        "rows": out,
        "source": "analysis_latest",
        "filters": {
            "device_id": device_id,
            "client_ip": client_ip,
            "bot_verdict": bot_verdict,
            "real_band": real_band,
            "field_key": field_key,
            "field_value": field_value,
        }
    }))
}

/// Legacy path when analysis_latest not yet backfilled.
fn cross_query_analysis_history_fallback(
    c: &mut Client,
    device_id: Option<&str>,
    client_ip: Option<&str>,
    bot_verdict: Option<&str>,
    real_band: Option<&str>,
    field_key: Option<&str>,
    field_value: Option<&str>,
    lim: i64,
) -> Result<Value, StoreError> {
    let sql = r#"
WITH latest AS (
  SELECT DISTINCT ON (ar.session_id)
    ar.session_id, ar.rev, ar.real_band, ar.device_id, ar.bot_verdict,
    ar.device_confidence, ar.client_ip, ar.created_ms,
    ar.device_tier, ar.collision_risk, ar.residual_entropy_ok,
    ar.digest_path, ar.product_version
  FROM analysis_results ar
  ORDER BY ar.session_id, ar.rev DESC
)
SELECT l.session_id, l.rev, l.real_band, l.device_id, l.bot_verdict,
       l.device_confidence, l.client_ip, l.created_ms,
       s.visitor_terminal_id,
       l.device_tier, l.collision_risk, l.residual_entropy_ok,
       l.digest_path, l.product_version
FROM latest l
JOIN sessions s ON s.session_id = l.session_id
WHERE ($1::text IS NULL OR l.device_id = $1)
  AND ($2::text IS NULL OR l.client_ip = $2 OR EXISTS (
        SELECT 1 FROM probe_cold pc
        WHERE pc.session_id = l.session_id AND pc.client_ip = $2
      ))
  AND ($3::text IS NULL OR l.bot_verdict = $3)
  AND ($4::text IS NULL OR l.real_band = $4)
  AND ($5::text IS NULL OR $6::text IS NULL OR EXISTS (
        SELECT 1 FROM probe_cold pc
        WHERE pc.session_id = l.session_id
          AND (
            pc.fields_json #>> ARRAY['fields', $5] = $6
            OR pc.fields_json #>> ARRAY[$5] = $6
          )
      ))
ORDER BY l.created_ms DESC
LIMIT $7
"#;
    let rows = c
        .query(
            sql,
            &[
                &device_id,
                &client_ip,
                &bot_verdict,
                &real_band,
                &field_key,
                &field_value,
                &lim,
            ],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(json!({
            "session_id": r.get::<_, String>(0),
            "rev": r.get::<_, i64>(1),
            "real_band": r.get::<_, Option<String>>(2),
            "device_id": r.get::<_, Option<String>>(3),
            "bot_verdict": r.get::<_, Option<String>>(4),
            "device_confidence": r.get::<_, Option<f64>>(5),
            "client_ip": r.get::<_, Option<String>>(6),
            "created_ms": r.get::<_, i64>(7),
            "visitor_terminal_id": r.get::<_, Option<String>>(8),
            "device_tier": r.get::<_, Option<String>>(9),
            "collision_risk": r.get::<_, Option<bool>>(10),
            "residual_entropy_ok": r.get::<_, Option<bool>>(11),
            "digest_path": r.get::<_, Option<String>>(12),
            "product_version": r.get::<_, Option<String>>(13),
        }));
    }
    Ok(json!({
        "ok": true,
        "count": out.len(),
        "rows": out,
        "source": "analysis_results_distinct_on",
        "filters": {
            "device_id": device_id,
            "client_ip": client_ip,
            "bot_verdict": bot_verdict,
            "real_band": real_band,
            "field_key": field_key,
            "field_value": field_value,
        }
    }))
}

/// P1: commercial device master row (no TOAST).
fn get_device_inner(c: &mut Client, device_id: &str) -> Result<Value, StoreError> {
    let row = c
        .query_opt(
            r#"
SELECT device_id, first_seen_ms, last_seen_ms, tier_last, collision_risk_last,
       residual_entropy_ok_last, session_count, distinct_ip_count, distinct_vt_count,
       last_client_ip, last_session_id, product_version_last, digest_path_last,
       COALESCE(meta_json::text, '{}')
FROM devices WHERE device_id = $1
"#,
            &[&device_id],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    let Some(r) = row else {
        return Ok(json!({"ok": true, "found": false, "device_id": device_id}));
    };
    let meta_s: String = r.get(13);
    let meta: Value = serde_json::from_str(&meta_s).unwrap_or(json!({}));
    Ok(json!({
        "ok": true,
        "found": true,
        "device": {
            "device_id": r.get::<_, String>(0),
            "first_seen_ms": r.get::<_, i64>(1),
            "last_seen_ms": r.get::<_, i64>(2),
            "tier_last": r.get::<_, Option<String>>(3),
            "collision_risk_last": r.get::<_, Option<bool>>(4),
            "residual_entropy_ok_last": r.get::<_, Option<bool>>(5),
            "session_count": r.get::<_, i64>(6),
            "distinct_ip_count": r.get::<_, i64>(7),
            "distinct_vt_count": r.get::<_, i64>(8),
            "last_client_ip": r.get::<_, Option<String>>(9),
            "last_session_id": r.get::<_, Option<String>>(10),
            "product_version_last": r.get::<_, Option<String>>(11),
            "digest_path_last": r.get::<_, Option<String>>(12),
            "meta": meta,
        }
    }))
}

/// P1: sessions that minted this commercial id (membership edge table).
fn list_device_sessions_inner(
    c: &mut Client,
    device_id: &str,
    limit: i64,
) -> Result<Value, StoreError> {
    let lim = limit.clamp(1, 2000);
    let rows = c
        .query(
            r#"
SELECT session_id, created_ms, client_ip, visitor_terminal_id, device_tier
FROM device_sessions
WHERE device_id = $1
ORDER BY created_ms DESC
LIMIT $2
"#,
            &[&device_id, &lim],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(json!({
            "session_id": r.get::<_, String>(0),
            "created_ms": r.get::<_, i64>(1),
            "client_ip": r.get::<_, Option<String>>(2),
            "visitor_terminal_id": r.get::<_, Option<String>>(3),
            "device_tier": r.get::<_, Option<String>>(4),
        }));
    }
    Ok(json!({
        "ok": true,
        "device_id": device_id,
        "count": out.len(),
        "rows": out,
    }))
}

/// P2/P3: reverse binder lookup (wg:/au:/lan:/id: keys).
fn lookup_devices_by_binder_inner(
    c: &mut Client,
    tenant_id: &str,
    binder_key: &str,
    limit: i64,
) -> Result<Value, StoreError> {
    let lim = limit.clamp(1, 500);
    let rows = c
        .query(
            r#"
SELECT dik.device_id, d.tier_last, d.last_seen_ms, d.session_count,
       d.collision_risk_last, d.digest_path_last
FROM device_index_keys dik
LEFT JOIN devices d ON d.device_id = dik.device_id
WHERE dik.tenant_id = $1 AND dik.binder_key = $2
ORDER BY COALESCE(d.last_seen_ms, 0) DESC
LIMIT $3
"#,
            &[&tenant_id, &binder_key, &lim],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(json!({
            "device_id": r.get::<_, String>(0),
            "tier_last": r.get::<_, Option<String>>(1),
            "last_seen_ms": r.get::<_, Option<i64>>(2),
            "session_count": r.get::<_, Option<i64>>(3),
            "collision_risk_last": r.get::<_, Option<bool>>(4),
            "digest_path_last": r.get::<_, Option<String>>(5),
        }));
    }
    Ok(json!({
        "ok": true,
        "tenant_id": tenant_id,
        "binder_key": binder_key,
        "count": out.len(),
        "rows": out,
        "source": "device_index_keys",
    }))
}

/// P0/P1: list recent analysis_latest scalars (no result_json TOAST).
fn list_analysis_latest_inner(
    c: &mut Client,
    limit: i64,
    device_tier: Option<&str>,
    since_ms: Option<i64>,
) -> Result<Value, StoreError> {
    let lim = limit.clamp(1, 2000);
    let rows = c
        .query(
            r#"
SELECT session_id, rev, created_ms, device_id, device_tier, device_prefix,
       collision_risk, residual_entropy_ok, real_band, bot_verdict, device_confidence,
       client_ip, digest_path, product_version, visitor_terminal_id, site_id,
       os_family, form_class, residual_algo, has_webrtc_host
FROM analysis_latest
WHERE ($1::text IS NULL OR device_tier = $1)
  AND ($2::bigint IS NULL OR created_ms >= $2)
ORDER BY created_ms DESC
LIMIT $3
"#,
            &[&device_tier, &since_ms, &lim],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(json!({
            "session_id": r.get::<_, String>(0),
            "rev": r.get::<_, i64>(1),
            "created_ms": r.get::<_, i64>(2),
            "device_id": r.get::<_, Option<String>>(3),
            "device_tier": r.get::<_, Option<String>>(4),
            "device_prefix": r.get::<_, Option<String>>(5),
            "collision_risk": r.get::<_, Option<bool>>(6),
            "residual_entropy_ok": r.get::<_, Option<bool>>(7),
            "real_band": r.get::<_, Option<String>>(8),
            "bot_verdict": r.get::<_, Option<String>>(9),
            "device_confidence": r.get::<_, Option<f64>>(10),
            "client_ip": r.get::<_, Option<String>>(11),
            "digest_path": r.get::<_, Option<String>>(12),
            "product_version": r.get::<_, Option<String>>(13),
            "visitor_terminal_id": r.get::<_, Option<String>>(14),
            "site_id": r.get::<_, Option<String>>(15),
            "os_family": r.get::<_, Option<String>>(16),
            "form_class": r.get::<_, Option<String>>(17),
            "residual_algo": r.get::<_, Option<String>>(18),
            "has_webrtc_host": r.get::<_, Option<bool>>(19),
        }));
    }
    Ok(json!({
        "ok": true,
        "count": out.len(),
        "rows": out,
        "source": "analysis_latest",
    }))
}

/// Copy recent orphan B8 (gateway-only, same VT, within window) onto FE cycle.
fn attach_orphan_b8_for_vt(
    c: &mut Client,
    vt: &str,
    target_sid: &str,
    window_ms: i64,
) -> Result<bool, StoreError> {
    if vt.is_empty() || target_sid.is_empty() {
        return Ok(false);
    }
    let ts = now_ms();
    let since = ts - window_ms;
    // Find newest other session for this VT that has B8 but no B0.
    // iss/opus5 04-P0-2: payload comes from probe_cold (SSOT); the
    // probe_batches.payload_json column is a legacy/debug copy that may be ''.
    let row = c
        .query_opt(
            "SELECT s.session_id, pb.payload_json, c.payload_z
             FROM sessions s
             JOIN probe_batches pb
               ON pb.session_id = s.session_id
              AND pb.batch_id IN ('B8_gateway','B8_gateway_early')
              AND pb.source = 'gateway'
             LEFT JOIN probe_cold c
               ON c.session_id = pb.session_id
              AND c.batch_id = pb.batch_id
              AND c.source = pb.source
             WHERE s.visitor_terminal_id = $1
               AND s.session_id <> $2
               AND s.created_ms >= $3
               AND NOT EXISTS (
                 SELECT 1 FROM probe_batches b0
                 WHERE b0.session_id = s.session_id
                   AND b0.batch_id IN ('B0_bootstrap','B0')
                   AND b0.source = 'main'
               )
             ORDER BY s.created_ms DESC
             LIMIT 1",
            &[&vt, &target_sid, &since],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    let Some(row) = row else {
        return Ok(false);
    };
    let src_sid: String = row.get(0);
    let payload_s: String = row.get(1);
    let payload_z: Option<Vec<u8>> = row.get(2);
    let mut payload: Value = payload_z
        .as_deref()
        .filter(|z| !z.is_empty())
        .and_then(|z| crate::decompress_json_payload(z).ok())
        .or_else(|| serde_json::from_str(&payload_s).ok())
        .unwrap_or_else(|| json!({}));
    if let Some(obj) = payload.as_object_mut() {
        if let Some(f) = obj.get_mut("fields").and_then(|v| v.as_object_mut()) {
            f.insert("b8_attached_from".into(), json!(src_sid));
            f.insert("b8_nojs_fe_bond".into(), json!(true));
        } else {
            obj.insert(
                "fields".into(),
                json!({
                    "b8_attached_from": src_sid,
                    "b8_nojs_fe_bond": true,
                }),
            );
        }
        obj.insert("early".into(), json!(true));
    }
    // Target must exist
    let _ = upsert_batch_inner(c, target_sid, "B8_gateway", "gateway", &payload, None)?;
    Ok(true)
}

fn bump_monthly_sessions_inner(
    c: &mut Client,
    site_id: &str,
) -> Result<(String, u64), StoreError> {
    let month = crate::month_key_utc();
    c.batch_execute(
        "CREATE TABLE IF NOT EXISTS usage_monthly (
           month TEXT NOT NULL,
           site_id TEXT NOT NULL,
           sessions BIGINT NOT NULL DEFAULT 0,
           PRIMARY KEY (month, site_id)
         )",
    )
    .map_err(|e| StoreError::Msg(e.to_string()))?;
    let n: i64 = c
        .query_one(
            "INSERT INTO usage_monthly(month, site_id, sessions) VALUES($1,$2,1)
             ON CONFLICT(month, site_id) DO UPDATE SET sessions = usage_monthly.sessions + 1
             RETURNING sessions",
            &[&month, &site_id],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?
        .get(0);
    Ok((month, n as u64))
}

fn mark_seal_consumed_inner(
    c: &mut Client,
    session_id: &str,
    batch_id: &str,
    nonce: &str,
) -> Result<bool, StoreError> {
    c.batch_execute(
        "CREATE TABLE IF NOT EXISTS seal_consumed (
           session_id TEXT NOT NULL,
           batch_id TEXT NOT NULL,
           nonce TEXT NOT NULL,
           consumed_ms BIGINT NOT NULL DEFAULT 0,
           PRIMARY KEY (session_id, batch_id, nonce)
         )",
    )
    .map_err(|e| StoreError::Msg(e.to_string()))?;
    let n: u64 = c
        .execute(
            "INSERT INTO seal_consumed(session_id, batch_id, nonce, consumed_ms)
             VALUES($1, $2, $3, $4) ON CONFLICT (session_id, batch_id, nonce) DO NOTHING",
            &[&session_id, &batch_id, &nonce, &crate::now_ms()],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    Ok(n > 0)
}

fn schedule_analyze_inner(
    c: &mut Client,
    session_id: &str,
    debounce_ms: i64,
    merge: AnalyzeDueMerge,
) -> Result<(), StoreError> {
    let now = now_ms();
    let new_due = now + debounce_ms.max(0);
    let existing: Option<i64> = c
        .query_opt(
            "SELECT due_ms FROM analyze_jobs WHERE session_id=$1 AND status='pending'",
            &[&session_id],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?
        .map(|r| r.get(0));
    let due = match (merge, existing) {
        (_, None) => new_due,
        (AnalyzeDueMerge::Replace, _) => new_due,
        (AnalyzeDueMerge::PullEarlier, Some(ex)) => ex.min(new_due),
        (AnalyzeDueMerge::IdleReset, Some(ex)) => {
            if ex <= now + ANALYZE_IDLE_IMMINENT_MS {
                ex
            } else {
                new_due
            }
        }
    };
    c.execute(
        "INSERT INTO analyze_jobs(session_id, due_ms, status, locked_until, locked_by, updated_ms)
         VALUES($1,$2,'pending',0,'',$3)
         ON CONFLICT(session_id) DO UPDATE SET
           due_ms=EXCLUDED.due_ms,
           status='pending',
           updated_ms=EXCLUDED.updated_ms
         WHERE analyze_jobs.status IS DISTINCT FROM 'pending'
            OR analyze_jobs.due_ms IS DISTINCT FROM EXCLUDED.due_ms",
        &[&session_id, &due, &now],
    )
    .map_err(|e| StoreError::Msg(e.to_string()))?;
    Ok(())
}

fn claim_due_analyze_jobs_inner(
    c: &mut Client,
    worker_id: &str,
    limit: usize,
    lock_ms: i64,
) -> Result<Vec<String>, StoreError> {
    let now = now_ms();
    let until = now + lock_ms.max(1000);
    let lim = limit.max(1).min(32) as i64;
    // Cheap read-only probe: empty queue must not UPDATE (WAL/fsync storm under
    // many analyze workers polling every few tens of ms).
    let any: bool = c
        .query_one(
            "SELECT EXISTS(
               SELECT 1 FROM analyze_jobs
               WHERE status='pending' AND due_ms<=$1 AND locked_until<$1
             )",
            &[&now],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?
        .get(0);
    if !any {
        return Ok(Vec::new());
    }
    // Single round-trip claim up to `lim` (was N sequential UPDATE LIMIT 1).
    let rows = c
        .query(
            "WITH cte AS (
               SELECT session_id FROM analyze_jobs
               WHERE status='pending' AND due_ms<=$1 AND locked_until<$1
               ORDER BY due_ms ASC
               FOR UPDATE SKIP LOCKED
               LIMIT $4
             )
             UPDATE analyze_jobs j SET locked_until=$2, locked_by=$3, updated_ms=$1
             FROM cte WHERE j.session_id=cte.session_id
             RETURNING j.session_id",
            &[&now, &until, &worker_id, &lim],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    Ok(rows.iter().map(|r| r.get::<_, String>(0)).collect())
}

fn complete_analyze_job_inner(
    c: &mut Client,
    session_id: &str,
    worker_id: &str,
) -> Result<bool, StoreError> {
    let now = now_ms();
    let row = c
        .query_opt(
            "SELECT due_ms, locked_by FROM analyze_jobs WHERE session_id=$1",
            &[&session_id],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    let Some(row) = row else {
        return Ok(false);
    };
    let due_ms: i64 = row.get(0);
    let locked_by: String = row.get(1);
    if locked_by != worker_id && !locked_by.is_empty() {
        return Ok(false);
    }
    if due_ms > now {
        c.execute(
            "UPDATE analyze_jobs SET locked_until=0, locked_by='', status='pending', updated_ms=$1
             WHERE session_id=$2",
            &[&now, &session_id],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
        return Ok(true);
    }
    c.execute("DELETE FROM analyze_jobs WHERE session_id=$1", &[&session_id])
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    Ok(false)
}

fn pending_analyze_job_count_inner(c: &mut Client) -> Result<i64, StoreError> {
    let row = c
        .query_one(
            "SELECT COUNT(1)::bigint FROM analyze_jobs WHERE status='pending'",
            &[],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    Ok(row.get(0))
}

fn analyze_queue_stats_inner(c: &mut Client) -> Result<Value, StoreError> {
    let now = now_ms();
    let row = c
        .query_one(
            "SELECT
               COUNT(1)::bigint AS pending,
               COUNT(1) FILTER (WHERE due_ms<=$1 AND locked_until<$1)::bigint AS due_now,
               COUNT(1) FILTER (WHERE locked_until>=$1)::bigint AS locked,
               COALESCE(MIN($1 - due_ms) FILTER (WHERE due_ms<=$1), 0)::bigint AS oldest_lag_ms
             FROM analyze_jobs WHERE status='pending'",
            &[&now],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    let (idle_min, idle_max) = crate::analyze_idle_poll_ms_range();
    Ok(json!({
        "pending": row.get::<_, i64>(0),
        "due_now": row.get::<_, i64>(1),
        "locked": row.get::<_, i64>(2),
        "oldest_lag_ms": row.get::<_, i64>(3).max(0),
        "debounce_ms": ANALYZE_DEBOUNCE_MS,
        "lock_ms": ANALYZE_LOCK_MS,
        "claim_batch": crate::ANALYZE_CLAIM_BATCH,
        "idle_poll_ms_min": idle_min,
        "idle_poll_ms_max": idle_max,
        "backend": "postgres",
    }))
}

fn payload_fields(payload: &Value) -> Map<String, Value> {
    let mut out = Map::new();
    if let Some(obj) = payload.as_object() {
        if let Some(f) = obj.get("fields").and_then(|v| v.as_object()) {
            for (k, v) in f {
                out.insert(k.clone(), v.clone());
            }
        } else {
            for (k, v) in obj {
                if k != "batch_id" && k != "source" && k != "session_id" && k != "inject_path" && k != "early"
                {
                    out.insert(k.clone(), v.clone());
                }
            }
        }
    }
    out
}

fn build_evidence_inner(c: &mut Client, session_id: &str) -> Result<Value, StoreError> {
    let meta = c
        .query_opt(
            "SELECT visitor_terminal_id, meta_json, updated_ms FROM sessions WHERE session_id=$1",
            &[&session_id],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    let Some(meta) = meta else {
        return Err(StoreError::NotFound(format!("session {session_id}")));
    };
    let vt: String = meta.get::<_, Option<String>>(0).unwrap_or_default();
    let meta_s: String = meta.get(1);
    let updated_ms: i64 = meta.get(2);
    // iss/opus5 04-P0-2: probe_cold.payload_z is the single source of truth;
    // probe_batches.payload_json is a legacy/debug copy that may be ''.
    let rows = c
        .query(
            "SELECT b.batch_id, b.source, b.payload_json, c.payload_z
             FROM probe_batches b
             LEFT JOIN probe_cold c
               ON c.session_id = b.session_id
              AND c.batch_id = b.batch_id
              AND c.source = b.source
             WHERE b.session_id=$1 ORDER BY b.created_ms ASC",
            &[&session_id],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;

    let mut batches = Vec::new();
    let mut sources = Vec::new();
    let mut fields = Map::new();
    let mut fields_by_source = Map::new();
    let mut gateway_fields = Map::new();
    let mut cf_fields = Map::new();
    fields.insert("session_id".into(), json!(session_id));
    fields.insert("visitor_terminal_id".into(), json!(vt));

    for row in rows {
        let batch_id: String = row.get(0);
        let source: String = row.get(1);
        let payload_s: String = row.get(2);
        let payload_z: Option<Vec<u8>> = row.get(3);
        let base_src = source.split(':').next().unwrap_or(source.as_str()).to_string();
        if !sources.iter().any(|s: &String| s == &source || s == &base_src) {
            sources.push(if source.contains(':') {
                base_src.clone()
            } else {
                source.clone()
            });
        }
        batches.push(json!({"batch_id": batch_id, "source": source}));
        // Prefer the cold compressed copy; fall back to the legacy raw copy.
        let payload: Value = payload_z
            .as_deref()
            .filter(|z| !z.is_empty())
            .and_then(|z| crate::decompress_json_payload(z).ok())
            .or_else(|| serde_json::from_str(&payload_s).ok())
            .unwrap_or_else(|| json!({}));
        let pf = payload_fields(&payload);
        crate::evidence_merge::merge_batch_with_id(
            &base_src,
            Some(batch_id.as_str()),
            pf,
            &mut fields,
            &mut fields_by_source,
            &mut gateway_fields,
            &mut cf_fields,
        );
    }
    let source_conflicts = crate::evidence_merge::detect_source_conflicts(&fields_by_source);
    let multi_source_consistency =
        crate::evidence_merge::assess_multi_source_consistency(&fields_by_source);
    if let Some(obj) = multi_source_consistency.as_object() {
        for key in [
            "sandbox_blocked",
            "sandbox_ok",
            "js_ok_sandbox_dead",
            "sandbox_all_empty",
            "sandbox_under_two_kinds",
            "sandbox_thin_vs_main",
            "sandbox_partial",
            "sandbox_capability_score",
            "sandbox_capability_band",
            "sandbox_payload_source_n",
            "sandbox_sources_received_n",
        ] {
            if let Some(v) = obj.get(key).cloned() {
                fields.entry(key.to_string()).or_insert(v);
            }
        }
        if let Some(r) = obj.get("match_ratio").cloned() {
            fields
                .entry("multi_source_match_ratio".to_string())
                .or_insert(r);
        }
    }
    let source_auth_view =
        crate::evidence_merge::build_source_auth_view(&fields_by_source, &source_conflicts);
    let realm_conflict_graph = crate::evidence_merge::structured_realm_diff(&fields_by_source);
    let authentic_fields_for_mint = crate::evidence_merge::authentic_fields_for_mint(
        &fields,
        &fields_by_source,
        &source_conflicts,
    );
    let meta_v: Value = serde_json::from_str(&meta_s).unwrap_or(json!({}));
    let has_gateway = !gateway_fields.is_empty()
        || sources.iter().any(|s| s == "gateway" || s.starts_with("gateway"));
    let has_cloudflare = ["bot_score", "country", "cf_ray", "colo", "asn", "cf_connecting_ip", "cf_edge_present"]
        .iter()
        .any(|k| cf_fields.contains_key(*k))
        || sources.iter().any(|s| s == "cloudflare" || s == "cf")
        || fields
            .get("cf_edge_present")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let evidence_rev = batches.len() as i64;
    let mut meta_out = meta_v.clone();
    if let Some(obj) = meta_out.as_object_mut() {
        obj.insert("evidence_rev".into(), json!(evidence_rev));
    }
    Ok(json!({
        "session_id": session_id,
        "visitor_terminal_id": vt,
        "sources": sources,
        "batches": batches,
        "fields": fields,
        "fields_by_source": fields_by_source,
        "source_conflicts": source_conflicts,
        "multi_source_consistency": multi_source_consistency,
        "source_auth_view": source_auth_view,
        "realm_conflict_graph": realm_conflict_graph,
        "authentic_fields_for_mint": authentic_fields_for_mint,
        "meta": meta_out,
        "session_meta": meta_out,
        "prior_belief": meta_v.get("belief").cloned().unwrap_or(Value::Null),
        "evidence_rev": evidence_rev,
        "has_gateway": has_gateway,
        "has_cloudflare": has_cloudflare,
        "gateway_fields": gateway_fields,
        "cf_fields": cf_fields,
        // Last probe upload wall time — used by SDK return gate (vt idle ≥1m).
        "last_upload_ms": updated_ms,
        "updated_ms": updated_ms,
    }))
}

fn save_analysis_inner(
    c: &mut Client,
    session_id: &str,
    result: &Value,
) -> Result<i64, StoreError> {
    // Multi-process race: analyze workers + ingest debounce both call save_analysis.
    // Plain MAX(rev)+1 then INSERT collided on analysis_results_pkey → client saw
    // `db error` / HTTP 500 (lab v5.8.81). Serialize per-session via transaction +
    // FOR UPDATE on sessions, with unique-violation retry as belt-and-suspenders.
    let slim = crate::slim_analysis_result_for_storage(result);
    let s = crate::encode_analysis_result_json(&slim).map_err(StoreError::Msg)?;
    let mut sc = crate::analysis_report_scalars(result);
    let ts = now_ms();

    let mut last_err = String::new();
    for attempt in 0..5 {
        let mut tx = c
            .transaction()
            .map_err(|e| StoreError::Msg(format!("analysis tx begin: {e}")))?;

        // Lock session row so concurrent writers for the same bag serialize.
        let sess_row = tx
            .query_opt(
                "SELECT client_ip, visitor_terminal_id, meta_json FROM sessions WHERE session_id=$1 FOR UPDATE",
                &[&session_id],
            )
            .map_err(|e| StoreError::Msg(e.to_string()))?;

        let mut vt: Option<String> = None;
        if let Some(r) = sess_row {
            let sess_ip: Option<String> = r.get(0);
            vt = r.get(1);
            let meta_s: String = r.get(2);
            if sc.client_ip.is_none() {
                sc.client_ip = sess_ip;
            }
            if sc.site_id.is_none() || sc.inject_path.is_none() {
                if let Ok(meta) = serde_json::from_str::<Value>(&meta_s) {
                    if sc.site_id.is_none() {
                        sc.site_id = meta
                            .get("site_id")
                            .or_else(|| meta.get("siteId"))
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string());
                    }
                    if sc.inject_path.is_none() {
                        sc.inject_path = meta
                            .get("inject_path")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string());
                    }
                }
            }
        }

        let next: i64 = tx
            .query_one(
                "SELECT COALESCE(MAX(rev), 0) + 1 FROM analysis_results WHERE session_id=$1",
                &[&session_id],
            )
            .map_err(|e| StoreError::Msg(e.to_string()))?
            .get(0);

        match tx.execute(
            "INSERT INTO analysis_results(session_id, rev, result_json, created_ms,
                real_band, device_id, bot_verdict, device_confidence, client_ip,
                device_tier, collision_risk, product_version, digest_path, residual_entropy_ok)
             VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
            &[
                &session_id,
                &next,
                &s,
                &ts,
                &sc.real_band,
                &sc.device_id,
                &sc.bot_verdict,
                &sc.device_confidence,
                &sc.client_ip,
                &sc.device_tier,
                &sc.collision_risk,
                &sc.product_version,
                &sc.digest_path,
                &sc.residual_entropy_ok,
            ],
        ) {
            Ok(_) => {
                // Prune old revs — keep last N (default 3) to bound TOAST growth per session.
                let keep = crate::analysis_history_keep();
                if next > keep {
                    let min_keep = next - keep;
                    let _ = tx.execute(
                        "DELETE FROM analysis_results WHERE session_id=$1 AND rev <= $2",
                        &[&session_id, &min_keep],
                    );
                }
                tx.commit()
                    .map_err(|e| StoreError::Msg(format!("analysis tx commit: {e}")))?;
                // Best-effort query tables after primary write commits.
                if let Err(e) =
                    upsert_query_tables_after_analysis(c, session_id, next, ts, &sc, vt.as_deref())
                {
                    eprintln!("gr-store: upsert_query_tables_after_analysis: {e}");
                }
                return Ok(next);
            }
            Err(e) => {
                let msg = e.to_string();
                let _ = tx.rollback();
                // 23505 unique_violation — concurrent writer won the rev; retry.
                let is_dup = msg.contains("analysis_results_pkey")
                    || msg.contains("duplicate key")
                    || msg.contains("23505");
                if is_dup && attempt + 1 < 5 {
                    last_err = msg;
                    std::thread::sleep(std::time::Duration::from_millis(5 + attempt as u64 * 10));
                    continue;
                }
                return Err(StoreError::Msg(msg));
            }
        }
    }
    Err(StoreError::Msg(format!(
        "analysis_results rev race exhausted retries: {last_err}"
    )))
}

/// P0–P2: materialize latest analysis + device master + binder index (no TOAST).
fn upsert_query_tables_after_analysis(
    c: &mut Client,
    session_id: &str,
    rev: i64,
    ts: i64,
    sc: &crate::AnalysisReportScalars,
    vt: Option<&str>,
) -> Result<(), StoreError> {
    let prefix = sc.device_id.as_ref().and_then(|id| {
        if id.len() >= 2 {
            Some(id[..2].to_string())
        } else {
            None
        }
    });
    let vt_s = vt.map(|s| s.to_string());
    // NOTE: `analysis_latest.product_action` is created in ensure_schema() at startup.
    // Never run DDL on the analysis hot path (ACCESS EXCLUSIVE lock per analyze).
    c.execute(
        r#"
INSERT INTO analysis_latest(
  session_id, rev, created_ms, device_id, device_tier, device_prefix,
  collision_risk, residual_entropy_ok, real_band, bot_verdict, device_confidence,
  client_ip, digest_path, product_version, visitor_terminal_id, site_id, inject_path,
  association_level, authenticity_band, form_class, os_family, platform,
  os_score, br_score, rpa_score, os_status, br_status, rpa_status,
  country, asn, residual_algo, has_webrtc_host,
  mint_residual_ok, mint_host_ok, mint_silicon_ok,
  mint_conflict_pressure, mint_single_source_pressure,
  mint_ok_keys_n, mint_conf_only_keys_n, mint_conflict_keys_n, mint_gate_summary
) VALUES (
  $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,
  $18,$19,$20,$21,$22,$23,$24,$25,$26,$27,$28,$29,$30,$31,$32,
  $33,$34,$35,$36,$37,$38,$39,$40,$41
)
ON CONFLICT (session_id) DO UPDATE SET
  rev=EXCLUDED.rev,
  created_ms=EXCLUDED.created_ms,
  device_id=EXCLUDED.device_id,
  device_tier=EXCLUDED.device_tier,
  device_prefix=EXCLUDED.device_prefix,
  collision_risk=EXCLUDED.collision_risk,
  residual_entropy_ok=EXCLUDED.residual_entropy_ok,
  real_band=EXCLUDED.real_band,
  bot_verdict=EXCLUDED.bot_verdict,
  device_confidence=EXCLUDED.device_confidence,
  client_ip=EXCLUDED.client_ip,
  digest_path=EXCLUDED.digest_path,
  product_version=EXCLUDED.product_version,
  visitor_terminal_id=COALESCE(EXCLUDED.visitor_terminal_id, analysis_latest.visitor_terminal_id),
  site_id=COALESCE(EXCLUDED.site_id, analysis_latest.site_id),
  inject_path=COALESCE(EXCLUDED.inject_path, analysis_latest.inject_path),
  association_level=EXCLUDED.association_level,
  authenticity_band=EXCLUDED.authenticity_band,
  form_class=EXCLUDED.form_class,
  os_family=EXCLUDED.os_family,
  platform=EXCLUDED.platform,
  os_score=EXCLUDED.os_score,
  br_score=EXCLUDED.br_score,
  rpa_score=EXCLUDED.rpa_score,
  os_status=EXCLUDED.os_status,
  br_status=EXCLUDED.br_status,
  rpa_status=EXCLUDED.rpa_status,
  country=EXCLUDED.country,
  asn=EXCLUDED.asn,
  residual_algo=EXCLUDED.residual_algo,
  has_webrtc_host=EXCLUDED.has_webrtc_host,
  mint_residual_ok=EXCLUDED.mint_residual_ok,
  mint_host_ok=EXCLUDED.mint_host_ok,
  mint_silicon_ok=EXCLUDED.mint_silicon_ok,
  mint_conflict_pressure=EXCLUDED.mint_conflict_pressure,
  mint_single_source_pressure=EXCLUDED.mint_single_source_pressure,
  mint_ok_keys_n=EXCLUDED.mint_ok_keys_n,
  mint_conf_only_keys_n=EXCLUDED.mint_conf_only_keys_n,
  mint_conflict_keys_n=EXCLUDED.mint_conflict_keys_n,
  mint_gate_summary=EXCLUDED.mint_gate_summary
WHERE analysis_latest.rev <= EXCLUDED.rev
"#,
        &[
            &session_id,
            &rev,
            &ts,
            &sc.device_id,
            &sc.device_tier,
            &prefix,
            &sc.collision_risk,
            &sc.residual_entropy_ok,
            &sc.real_band,
            &sc.bot_verdict,
            &sc.device_confidence,
            &sc.client_ip,
            &sc.digest_path,
            &sc.product_version,
            &vt_s,
            &sc.site_id,
            &sc.inject_path,
            &sc.association_level,
            &sc.authenticity_band,
            &sc.form_class,
            &sc.os_family,
            &sc.platform,
            &sc.os_score,
            &sc.br_score,
            &sc.rpa_score,
            &sc.os_status,
            &sc.br_status,
            &sc.rpa_status,
            &sc.country,
            &sc.asn,
            &sc.residual_algo,
            &sc.has_webrtc_host,
            &sc.mint_residual_ok,
            &sc.mint_host_ok,
            &sc.mint_silicon_ok,
            &sc.mint_conflict_pressure,
            &sc.mint_single_source_pressure,
            &sc.mint_ok_keys_n,
            &sc.mint_conf_only_keys_n,
            &sc.mint_conflict_keys_n,
            &sc.mint_gate_summary,
        ],
    )
    .map_err(|e| StoreError::Msg(format!("analysis_latest upsert: {e}")))?;

    // Separate update keeps historical installs stable if product_action column lags.
    if let Some(ref act) = sc.product_action {
        let _ = c.execute(
            "UPDATE analysis_latest SET product_action=$1 WHERE session_id=$2",
            &[act, &session_id],
        );
    }

    if let Some(did) = sc.device_id.as_ref().filter(|s| !s.is_empty()) {
        // devices master
        c.execute(
            r#"
INSERT INTO devices(
  device_id, first_seen_ms, last_seen_ms, tier_last, collision_risk_last,
  residual_entropy_ok_last, session_count, distinct_ip_count, distinct_vt_count,
  last_client_ip, last_session_id, product_version_last, digest_path_last
) VALUES ($1,$2,$2,$3,$4,$5,1,
  CASE WHEN $6::text IS NULL OR $6 = '' THEN 0 ELSE 1 END,
  CASE WHEN $7::text IS NULL OR $7 = '' THEN 0 ELSE 1 END,
  $6,$8,$9,$10)
ON CONFLICT (device_id) DO UPDATE SET
  last_seen_ms = GREATEST(devices.last_seen_ms, EXCLUDED.last_seen_ms),
  first_seen_ms = LEAST(devices.first_seen_ms, EXCLUDED.first_seen_ms),
  tier_last = EXCLUDED.tier_last,
  collision_risk_last = EXCLUDED.collision_risk_last,
  residual_entropy_ok_last = EXCLUDED.residual_entropy_ok_last,
  last_client_ip = COALESCE(EXCLUDED.last_client_ip, devices.last_client_ip),
  last_session_id = EXCLUDED.last_session_id,
  product_version_last = COALESCE(EXCLUDED.product_version_last, devices.product_version_last),
  digest_path_last = COALESCE(EXCLUDED.digest_path_last, devices.digest_path_last)
"#,
            &[
                did,
                &ts,
                &sc.device_tier,
                &sc.collision_risk,
                &sc.residual_entropy_ok,
                &sc.client_ip,
                &vt_s,
                &session_id,
                &sc.product_version,
                &sc.digest_path,
            ],
        )
        .map_err(|e| StoreError::Msg(format!("devices upsert: {e}")))?;

        let ins = c
            .execute(
                r#"
INSERT INTO device_sessions(device_id, session_id, created_ms, client_ip, visitor_terminal_id, device_tier)
VALUES ($1,$2,$3,$4,$5,$6)
ON CONFLICT (device_id, session_id) DO NOTHING
"#,
                &[
                    did,
                    &session_id,
                    &ts,
                    &sc.client_ip,
                    &vt_s,
                    &sc.device_tier,
                ],
            )
            .map_err(|e| StoreError::Msg(format!("device_sessions: {e}")))?;
        if ins > 0 {
            // Refresh counts from membership (accurate, still cheap vs full JSON)
            c.execute(
                r#"
UPDATE devices d SET
  session_count = (SELECT count(*) FROM device_sessions ds WHERE ds.device_id = d.device_id),
  distinct_ip_count = (
    SELECT count(DISTINCT client_ip) FROM device_sessions ds
    WHERE ds.device_id = d.device_id AND client_ip IS NOT NULL AND client_ip <> ''
  ),
  distinct_vt_count = (
    SELECT count(DISTINCT visitor_terminal_id) FROM device_sessions ds
    WHERE ds.device_id = d.device_id AND visitor_terminal_id IS NOT NULL AND visitor_terminal_id <> ''
  )
WHERE d.device_id = $1
"#,
                &[did],
            )
            .map_err(|e| StoreError::Msg(format!("devices count: {e}")))?;
        }

        // binder reverse index (tenant default)
        let tenant = "default";
        for key in &sc.binder_keys {
            if key.is_empty() {
                continue;
            }
            let _ = c.execute(
                r#"
INSERT INTO device_index_keys(tenant_id, binder_key, device_id)
VALUES ($1,$2,$3)
ON CONFLICT (tenant_id, binder_key, device_id) DO NOTHING
"#,
                &[&tenant, key, did],
            );
        }
        let obs = serde_json::json!({
            "last_session_id": session_id,
            "device_tier": sc.device_tier,
            "digest_path": sc.digest_path,
            "collision_risk": sc.collision_risk,
            "updated_ms": ts,
        })
        .to_string();
        let _ = c.execute(
            r#"
INSERT INTO device_index_devices(tenant_id, device_id, binder_obs_json, updated_ms)
VALUES ($1,$2,$3,$4)
ON CONFLICT (tenant_id, device_id) DO UPDATE SET
  binder_obs_json = EXCLUDED.binder_obs_json,
  updated_ms = EXCLUDED.updated_ms
"#,
            &[&tenant, did, &obs, &ts],
        );
    }
    Ok(())
}

fn force_session_times_inner(
    c: &mut Client,
    session_id: &str,
    created_ms: i64,
    updated_ms: i64,
) -> Result<(), StoreError> {
    let n = c
        .execute(
            "UPDATE sessions SET created_ms=$1, updated_ms=$2 WHERE session_id=$3",
            &[&created_ms, &updated_ms, &session_id],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    if n == 0 {
        return Err(StoreError::NotFound(format!("session {session_id}")));
    }
    Ok(())
}

fn decode_stored_analysis_json(js: &str) -> Result<Value, StoreError> {
    crate::decode_analysis_result_json(js).map_err(StoreError::Msg)
}

fn session_meta_inner(
    c: &mut Client,
    session_id: &str,
) -> Result<Option<Value>, StoreError> {
    let row = c
        .query_opt(
            "SELECT meta_json FROM sessions WHERE session_id = $1",
            &[&session_id],
        )
        .map_err(|e| StoreError::Msg(format!("session meta: {e}")))?;
    match row {
        Some(r) => {
            let s: String = r.get(0);
            Ok(serde_json::from_str(&s).ok())
        }
        None => Ok(None),
    }
}

fn list_observation_events_inner(
    c: &mut Client,
    session_id: &str,
    limit: i64,
) -> Result<Vec<Value>, StoreError> {
    let lim = limit.clamp(1, 1000);
    let rows = c
        .query(
            r#"SELECT observation_id, batch_id, source_kind, realm_kind,
                      probe_method_id, envelope_json, created_ms
               FROM observation_events
               WHERE session_id = $1
               ORDER BY created_ms ASC
               LIMIT $2"#,
            &[&session_id, &lim],
        )
        .map_err(|e| StoreError::Msg(format!("obs list: {e}")))?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let env_s: String = r.get(5);
        let env = serde_json::from_str(&env_s).unwrap_or(json!({}));
        out.push(json!({
            "observation_id": r.get::<_, String>(0),
            "batch_id": r.get::<_, String>(1),
            "source_kind": r.get::<_, String>(2),
            "realm_kind": r.get::<_, String>(3),
            "probe_method_id": r.get::<_, String>(4),
            "envelope": env,
            "created_ms": r.get::<_, i64>(6),
        }));
    }
    Ok(out)
}

fn latest_analysis_inner(c: &mut Client, session_id: &str) -> Result<Option<Value>, StoreError> {
    // Prefer analysis_latest.site_id (always present on multi-node schema); fall back
    // when analysis_results lacks the extended denorm columns (older PG installs).
    let row = c
        .query_opt(
            r#"
            SELECT ar.rev, ar.result_json, ar.created_ms, al.site_id
            FROM analysis_results ar
            LEFT JOIN analysis_latest al ON al.session_id = ar.session_id
            WHERE ar.session_id=$1
            ORDER BY ar.rev DESC
            LIMIT 1
            "#,
            &[&session_id],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    Ok(match row {
        Some(r) => {
            let rev: i64 = r.get(0);
            let js: String = r.get(1);
            let created_ms: i64 = r.get(2);
            let denorm_site: Option<String> = r.get(3);
            let mut result = decode_stored_analysis_json(&js)?;
            if let Some(obj) = result.as_object_mut() {
                obj.insert("analysis_rev".into(), json!(rev));
                obj.insert("analyzed_ms".into(), json!(created_ms));
                if let Some(site) = denorm_site
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    let need = !obj
                        .get("meta")
                        .and_then(|m| m.get("site_id"))
                        .and_then(|v| v.as_str())
                        .map(|s| !s.is_empty())
                        .unwrap_or(false)
                        && !obj
                            .get("fields")
                            .and_then(|m| m.get("site_id"))
                            .and_then(|v| v.as_str())
                            .map(|s| !s.is_empty())
                            .unwrap_or(false);
                    if need {
                        let meta = obj.entry("meta".to_string()).or_insert_with(|| json!({}));
                        if let Some(m) = meta.as_object_mut() {
                            m.insert("site_id".into(), json!(site));
                        }
                    }
                }
            }
            Some(result)
        }
        None => None,
    })
}

fn list_analyses_inner(c: &mut Client, session_id: &str) -> Result<Vec<Value>, StoreError> {
    let rows = c
        .query(
            "SELECT rev, result_json, created_ms FROM analysis_results
             WHERE session_id=$1 ORDER BY rev ASC",
            &[&session_id],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    let mut out = Vec::new();
    for r in rows {
        let rev: i64 = r.get(0);
        let js: String = r.get(1);
        let created_ms: i64 = r.get(2);
        let mut result = decode_stored_analysis_json(&js)?;
        if let Some(obj) = result.as_object_mut() {
            obj.insert("analysis_rev".into(), json!(rev));
            obj.insert("analyzed_ms".into(), json!(created_ms));
        }
        out.push(json!({
            "rev": rev,
            "created_ms": created_ms,
            "real_band": result.get("real_band"),
            "device_id": result.pointer("/device/device_id"),
            "result": result,
        }));
    }
    Ok(out)
}

fn list_received_batches_inner(c: &mut Client, session_id: &str) -> Result<Vec<Value>, StoreError> {
    let rows = c
        .query(
            "SELECT batch_id, source, created_ms FROM probe_batches
             WHERE session_id=$1 ORDER BY created_ms ASC",
            &[&session_id],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    let mut out = Vec::new();
    for r in rows {
        let batch_id: String = r.get(0);
        let source: String = r.get(1);
        let created_ms: i64 = r.get(2);
        out.push(json!({
            "batch_id": batch_id,
            "source": source,
            "created_ms": created_ms,
            "dedupe_key": format!("{session_id}|{batch_id}|{source}")
        }));
    }
    Ok(out)
}

fn list_peer_session_ids_inner(
    c: &mut Client,
    session_id: &str,
    limit: usize,
) -> Result<Vec<String>, StoreError> {
    let row = c
        .query_opt(
            "SELECT visitor_terminal_id, meta_json FROM sessions WHERE session_id=$1",
            &[&session_id],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    let Some(row) = row else {
        return Ok(vec![]);
    };
    let vt: String = row.get(0);
    let meta_s: String = row.get(1);
    let harness = serde_json::from_str::<Value>(&meta_s)
        .ok()
        .and_then(|m| {
            m.get("harness_run")
                .or_else(|| m.pointer("/harness/harness_run"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_default();
    let lim = limit.max(1) as i64;
    let mut out: Vec<String> = Vec::new();
    if !vt.is_empty() {
        let rows = c
            .query(
                "SELECT session_id FROM sessions
                 WHERE session_id <> $1 AND visitor_terminal_id = $2
                 ORDER BY updated_ms DESC LIMIT $3",
                &[&session_id, &vt, &lim],
            )
            .map_err(|e| StoreError::Msg(e.to_string()))?;
        for r in rows {
            out.push(r.get(0));
        }
    }
    if out.len() < limit && !harness.is_empty() {
        let rows = c
            .query(
                "SELECT session_id FROM sessions
                 WHERE session_id <> $1
                   AND (
                     meta_json::jsonb->>'harness_run' = $2
                     OR meta_json::jsonb->'harness'->>'harness_run' = $2
                   )
                 ORDER BY updated_ms DESC LIMIT $3",
                &[&session_id, &harness, &lim],
            )
            .map_err(|e| StoreError::Msg(e.to_string()))?;
        for r in rows {
            let sid: String = r.get(0);
            if !out.contains(&sid) {
                out.push(sid);
            }
            if out.len() >= limit {
                break;
            }
        }
    }
    out.truncate(limit);
    Ok(out)
}

fn has_batch_inner(
    c: &mut Client,
    session_id: &str,
    batch_id: &str,
    source: &str,
) -> Result<bool, StoreError> {
    let n: i64 = c
        .query_one(
            "SELECT COUNT(1)::bigint FROM probe_batches
             WHERE session_id=$1 AND batch_id=$2 AND source=$3",
            &[&session_id, &batch_id, &source],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?
        .get(0);
    Ok(n > 0)
}

fn device_index_upsert_inner(
    c: &mut Client,
    tenant_id: &str,
    device_id: &str,
    binder_obs: &Value,
    binder_keys: &[String],
) -> Result<Value, StoreError> {
    let ts = now_ms();
    let obs_s = serde_json::to_string(binder_obs).map_err(|e| StoreError::Msg(e.to_string()))?;
    c.execute(
        "INSERT INTO device_index_devices(tenant_id, device_id, binder_obs_json, updated_ms)
         VALUES($1,$2,$3,$4)
         ON CONFLICT(tenant_id, device_id) DO UPDATE SET
           binder_obs_json=EXCLUDED.binder_obs_json,
           updated_ms=EXCLUDED.updated_ms",
        &[&tenant_id, &device_id, &obs_s, &ts],
    )
    .map_err(|e| StoreError::Msg(e.to_string()))?;
    for k in binder_keys {
        c.execute(
            "INSERT INTO device_index_keys(tenant_id, binder_key, device_id)
             VALUES($1,$2,$3)
             ON CONFLICT(tenant_id, binder_key, device_id) DO NOTHING",
            &[&tenant_id, k, &device_id],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    }
    Ok(json!({
        "ok": true,
        "tenant_id": tenant_id,
        "device_id": device_id,
        "keys_n": binder_keys.len(),
        "updated_ms": ts,
        "contract": "file_device_index_v1_store_skeleton",
        "backend": "postgres",
    }))
}

fn device_index_export_inner(c: &mut Client, tenant_id: &str) -> Result<Value, StoreError> {
    let mut devices = Map::new();
    let rows = c
        .query(
            "SELECT device_id, binder_obs_json FROM device_index_devices WHERE tenant_id=$1",
            &[&tenant_id],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    for r in rows {
        let id: String = r.get(0);
        let js: String = r.get(1);
        let obs: Value = serde_json::from_str(&js).unwrap_or(json!({}));
        devices.insert(id, obs);
    }
    let mut index = Map::new();
    let rows = c
        .query(
            "SELECT binder_key, device_id FROM device_index_keys WHERE tenant_id=$1",
            &[&tenant_id],
        )
        .map_err(|e| StoreError::Msg(e.to_string()))?;
    for r in rows {
        let k: String = r.get(0);
        let id: String = r.get(1);
        let arr = index.entry(k).or_insert_with(|| json!([]));
        if let Some(a) = arr.as_array_mut() {
            a.push(json!(id));
        }
    }
    Ok(json!({
        "version": "file_device_index_v1",
        "tenant_id": tenant_id,
        "algo": "link_or_mint_v1",
        "server_mint_algo": "server_mint_v1",
        "devices": devices,
        "index": index,
        "source": "gr_store_postgres_lab",
    }))
}


fn s_field(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}
fn i_field(v: &Value, k: &str, d: i64) -> i64 {
    v.get(k).and_then(|x| x.as_i64()).unwrap_or(d)
}
fn f_field(v: &Value, k: &str, d: f64) -> f64 {
    v.get(k).and_then(|x| x.as_f64()).unwrap_or(d)
}

fn insert_ops_client_event_inner(c: &mut Client, row: Value) -> Result<Value, StoreError> {
    let ts = now_ms();
    let event_id = {
        let e = s_field(&row, "event_id");
        if e.is_empty() { format!("oce_{}", hex_now()) } else { e }
    };
    let detail = row.get("detail_json").cloned().unwrap_or(json!({}));
    let detail_s = serde_json::to_string(&detail).unwrap_or_else(|_| "{}".into());
    c.execute(
        r#"INSERT INTO ops_client_events(
            event_id, ts_ms, server_recv_ms, site_id, visitor_terminal_id, session_id,
            product_version, inject_path, engine_family, ua_hash, stage, code, severity,
            detail_json, sample_rate, client_ip)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)"#,
        &[
            &event_id,
            &i_field(&row, "ts_ms", ts),
            &ts,
            &s_field(&row, "site_id"),
            &s_field(&row, "visitor_terminal_id"),
            &s_field(&row, "session_id"),
            &s_field(&row, "product_version"),
            &s_field(&row, "inject_path"),
            &s_field(&row, "engine_family"),
            &s_field(&row, "ua_hash"),
            &s_field(&row, "stage"),
            &s_field(&row, "code"),
            &{
                let s = s_field(&row, "severity");
                if s.is_empty() { "error".into() } else { s }
            },
            &detail_s,
            &f_field(&row, "sample_rate", 1.0),
            &{
                let ip = s_field(&row, "client_ip");
                if ip.is_empty() { None } else { Some(ip) }
            },
        ],
    )
    .map_err(|e| StoreError::Msg(format!("ops_client insert: {e}")))?;
    Ok(json!({"ok": true, "event_id": event_id, "server_recv_ms": ts}))
}

fn insert_ops_server_event_inner(c: &mut Client, row: Value) -> Result<Value, StoreError> {
    let ts = now_ms();
    let event_id = {
        let e = s_field(&row, "event_id");
        if e.is_empty() { format!("ose_{}", hex_now()) } else { e }
    };
    let detail = row.get("detail_json").cloned().unwrap_or(json!({}));
    let detail_s = serde_json::to_string(&detail).unwrap_or_else(|_| "{}".into());
    c.execute(
        r#"INSERT INTO ops_server_events(
            event_id, ts_ms, site_id, visitor_terminal_id, session_id,
            product_version, engine_family, stage, code, severity, detail_json, client_ip)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)"#,
        &[
            &event_id,
            &i_field(&row, "ts_ms", ts),
            &s_field(&row, "site_id"),
            &s_field(&row, "visitor_terminal_id"),
            &s_field(&row, "session_id"),
            &s_field(&row, "product_version"),
            &s_field(&row, "engine_family"),
            &s_field(&row, "stage"),
            &s_field(&row, "code"),
            &{
                let s = s_field(&row, "severity");
                if s.is_empty() { "error".into() } else { s }
            },
            &detail_s,
            &{
                let ip = s_field(&row, "client_ip");
                if ip.is_empty() { None } else { Some(ip) }
            },
        ],
    )
    .map_err(|e| StoreError::Msg(format!("ops_server insert: {e}")))?;
    Ok(json!({"ok": true, "event_id": event_id, "ts_ms": ts}))
}

fn insert_observation_event_inner(c: &mut Client, row: Value) -> Result<Value, StoreError> {
    let ts = now_ms();
    let observation_id = {
        let e = s_field(&row, "observation_id");
        if e.is_empty() {
            format!("obs_{}", hex_now())
        } else {
            e
        }
    };
    let envelope = row
        .get("envelope_json")
        .cloned()
        .or_else(|| row.get("envelope").cloned())
        .unwrap_or(json!({}));
    // iss/opus5 04-P0-2: pointer + summary by default (no full envelope copy).
    let envelope = slim_observation_envelope(&envelope);
    let envelope_s = serde_json::to_string(&envelope).unwrap_or_else(|_| "{}".into());
    c.execute(
        r#"INSERT INTO observation_events(
            observation_id, tenant_id, session_id, batch_id, source,
            source_kind, realm_kind, probe_method_id, capture_id, attempt_id,
            envelope_json, created_ms)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
         ON CONFLICT (observation_id) DO NOTHING"#,
        &[
            &observation_id,
            &s_field(&row, "tenant_id"),
            &s_field(&row, "session_id"),
            &s_field(&row, "batch_id"),
            &s_field(&row, "source"),
            &s_field(&row, "source_kind"),
            &s_field(&row, "realm_kind"),
            &s_field(&row, "probe_method_id"),
            &{
                let s = s_field(&row, "capture_id");
                if s.is_empty() { None } else { Some(s) }
            },
            &{
                let s = s_field(&row, "attempt_id");
                if s.is_empty() { None } else { Some(s) }
            },
            &envelope_s,
            &i_field(&row, "created_ms", ts),
        ],
    )
    .map_err(|e| StoreError::Msg(format!("observation insert: {e}")))?;
    Ok(json!({"ok": true, "observation_id": observation_id, "appended": true}))
}

fn lookup_api_idempotency_inner(
    c: &mut Client,
    tenant_id: &str,
    route: &str,
    idempotency_key: &str,
) -> Result<Option<Value>, StoreError> {
    let row = c
        .query_opt(
            "SELECT body_hash, status, response_json FROM api_idempotency
             WHERE tenant_id=$1 AND route=$2 AND idempotency_key=$3",
            &[&tenant_id, &route, &idempotency_key],
        )
        .map_err(|e| StoreError::Msg(format!("idempotency lookup: {e}")))?;
    Ok(row.map(|r| {
        let body_hash: String = r.get(0);
        let status: i32 = r.get(1);
        let response_s: String = r.get(2);
        let response: Value = serde_json::from_str(&response_s).unwrap_or(json!({}));
        json!({
            "hit": true,
            "body_hash": body_hash,
            "status": status as i64,
            "response": response,
        })
    }))
}

fn put_api_idempotency_inner(
    c: &mut Client,
    tenant_id: &str,
    route: &str,
    idempotency_key: &str,
    body_hash: &str,
    status: i64,
    response: &Value,
) -> Result<Value, StoreError> {
    let ts = now_ms();
    let response_s = serde_json::to_string(response).unwrap_or_else(|_| "{}".into());
    let status_i32 = status as i32;
    c.execute(
        r#"INSERT INTO api_idempotency(
            tenant_id, route, idempotency_key, body_hash, status, response_json, created_ms)
         VALUES($1,$2,$3,$4,$5,$6,$7)
         ON CONFLICT (tenant_id, route, idempotency_key) DO NOTHING"#,
        &[
            &tenant_id,
            &route,
            &idempotency_key,
            &body_hash,
            &status_i32,
            &response_s,
            &ts,
        ],
    )
    .map_err(|e| StoreError::Msg(format!("idempotency put: {e}")))?;
    Ok(json!({"ok": true}))
}

fn bump_rate_limit_window_inner(
    c: &mut Client,
    key: &str,
    window_ms: i64,
) -> Result<i64, StoreError> {
    // Lazy purge of windows older than the previous one — keeps the table bounded.
    let _ = c
        .execute(
            "DELETE FROM probe_rate_limit_windows WHERE window_ms < $1",
            &[&(window_ms - 2)],
        )
        .map_err(|e| StoreError::Msg(format!("rl cleanup: {e}")))?;
    let row = c
        .query_one(
            r#"INSERT INTO probe_rate_limit_windows(k, window_ms, count)
               VALUES($1, $2, 1)
               ON CONFLICT (k) DO UPDATE SET
                 count = CASE WHEN probe_rate_limit_windows.window_ms = $2
                              THEN probe_rate_limit_windows.count + 1 ELSE 1 END,
                 window_ms = CASE WHEN probe_rate_limit_windows.window_ms = $2
                              THEN probe_rate_limit_windows.window_ms ELSE $2 END
               RETURNING count"#,
            &[&key, &window_ms],
        )
        .map_err(|e| StoreError::Msg(format!("rl bump: {e}")))?;
    Ok(row.get::<_, i64>(0))
}

fn list_ops_events_inner(
    c: &mut Client,
    source: &str,
    limit: i64,
    code: Option<String>,
    site_id: Option<String>,
    since_ms: Option<i64>,
    product_version: Option<String>,
) -> Result<Value, StoreError> {
    let lim = limit.clamp(1, 500);
    let table = if source == "client" { "ops_client_events" } else { "ops_server_events" };
    let ts_col = if source == "client" { "server_recv_ms" } else { "ts_ms" };
    let mut sql = format!(
        "SELECT event_id, {ts_col}, site_id, visitor_terminal_id, session_id, product_version, engine_family, stage, code, severity, detail_json FROM {table} WHERE 1=1"
    );
    if code.as_ref().map(|s| !s.is_empty()).unwrap_or(false) {
        sql.push_str(&format!(" AND code = '{}'", code.as_ref().unwrap().replace('\'', "''")));
    }
    if site_id.as_ref().map(|s| !s.is_empty()).unwrap_or(false) {
        sql.push_str(&format!(" AND site_id = '{}'", site_id.as_ref().unwrap().replace('\'', "''")));
    }
    if product_version.as_ref().map(|s| !s.is_empty()).unwrap_or(false) {
        // Exact match or prefix (e.g. product_version=v5.8.95 matches full tags)
        let pv = product_version.as_ref().unwrap().replace('\'', "''");
        sql.push_str(&format!(" AND product_version LIKE '{pv}%'"));
    }
    if let Some(s) = since_ms {
        sql.push_str(&format!(" AND {ts_col} >= {s}"));
    }
    sql.push_str(&format!(" ORDER BY {ts_col} DESC LIMIT {lim}"));
    let rows = c.query(&sql, &[]).map_err(|e| StoreError::Msg(format!("ops list: {e}")))?;
    let mut events = Vec::new();
    for r in rows {
        events.push(json!({
            "event_id": r.get::<_, String>(0),
            "ts_ms": r.get::<_, i64>(1),
            "site_id": r.get::<_, String>(2),
            "visitor_terminal_id": r.get::<_, String>(3),
            "session_id": r.get::<_, String>(4),
            "product_version": r.get::<_, String>(5),
            "engine_family": r.get::<_, String>(6),
            "stage": r.get::<_, String>(7),
            "code": r.get::<_, String>(8),
            "severity": r.get::<_, String>(9),
            "detail_json": serde_json::from_str::<Value>(&r.get::<_, String>(10)).unwrap_or(json!({})),
        }));
    }
    Ok(json!({"ok": true, "source": source, "events": events, "n": events.len()}))
}

fn ops_outcome_distribution_inner(c: &mut Client, since_ms: i64) -> Result<Value, StoreError> {
    let bot_rows = c
        .query(
            r#"
            SELECT COALESCE(NULLIF(bot_verdict,''), '(none)') AS k, COUNT(*)::bigint AS n
            FROM analysis_latest
            WHERE created_ms >= $1
            GROUP BY 1
            ORDER BY n DESC
            LIMIT 50
            "#,
            &[&since_ms],
        )
        .map_err(|e| StoreError::Msg(format!("ops_outcome bot: {e}")))?;
    let band_rows = c
        .query(
            r#"
            SELECT COALESCE(NULLIF(real_band,''), '(none)') AS k, COUNT(*)::bigint AS n
            FROM analysis_latest
            WHERE created_ms >= $1
            GROUP BY 1
            ORDER BY n DESC
            LIMIT 50
            "#,
            &[&since_ms],
        )
        .map_err(|e| StoreError::Msg(format!("ops_outcome band: {e}")))?;
    let site_bot = c
        .query(
            r#"
            SELECT
              COALESCE(NULLIF(site_id,''), '(none)') AS site_id,
              COALESCE(NULLIF(bot_verdict,''), '(none)') AS bot_verdict,
              COUNT(*)::bigint AS n
            FROM analysis_latest
            WHERE created_ms >= $1
            GROUP BY 1, 2
            ORDER BY n DESC
            LIMIT 200
            "#,
            &[&since_ms],
        )
        .map_err(|e| StoreError::Msg(format!("ops_outcome site_bot: {e}")))?;
    let mut by_bot = Vec::new();
    let mut total = 0i64;
    for r in &bot_rows {
        let n: i64 = r.get(1);
        total += n;
        by_bot.push(json!({"key": r.get::<_, String>(0), "count": n}));
    }
    let mut by_band = Vec::new();
    for r in &band_rows {
        by_band.push(json!({"key": r.get::<_, String>(0), "count": r.get::<_, i64>(1)}));
    }
    // Prefer analyze-time product_action denorm when present; else bot_verdict proxy.
    // Note: live get_result recommended_action still applies panel strategy at read time.
    let mut by_action_proxy = Vec::new();
    let action_rows = c.query(
        r#"
            SELECT COALESCE(NULLIF(product_action,''), '(none)') AS k, COUNT(*)::bigint AS n
            FROM analysis_latest
            WHERE created_ms >= $1
              AND product_action IS NOT NULL AND product_action <> ''
            GROUP BY 1
            ORDER BY n DESC
            LIMIT 50
            "#,
        &[&since_ms],
    );
    let used_product_action = match action_rows {
        Ok(rows) if !rows.is_empty() => {
            for r in &rows {
                by_action_proxy.push(json!({
                    "action": r.get::<_, String>(0),
                    "count": r.get::<_, i64>(1),
                }));
            }
            true
        }
        _ => false,
    };
    if !used_product_action {
        let mut allow_n = 0i64;
        let mut challenge_n = 0i64;
        let mut deny_n = 0i64;
        let mut other_n = 0i64;
        for r in &bot_rows {
            let k: String = r.get(0);
            let n: i64 = r.get(1);
            let kl = k.to_ascii_lowercase();
            if kl.contains("bot") || kl == "deny" || kl.contains("automated") {
                deny_n += n;
            } else if kl.contains("suspect") || kl.contains("risk") || kl.contains("challenge") {
                challenge_n += n;
            } else if kl.contains("human") || kl == "allow" || kl.contains("real") || kl == "ok" {
                allow_n += n;
            } else if kl == "(none)" || kl.is_empty() {
                other_n += n;
            } else {
                challenge_n += n;
            }
        }
        if allow_n > 0 {
            by_action_proxy.push(json!({"action": "allow", "count": allow_n}));
        }
        if challenge_n > 0 {
            by_action_proxy.push(json!({"action": "challenge", "count": challenge_n}));
        }
        if deny_n > 0 {
            by_action_proxy.push(json!({"action": "deny", "count": deny_n}));
        }
        if other_n > 0 {
            by_action_proxy.push(json!({"action": "unknown", "count": other_n}));
        }
    }
    let mut by_site_bot = Vec::new();
    for r in site_bot {
        by_site_bot.push(json!({
            "site_id": r.get::<_, String>(0),
            "bot_verdict": r.get::<_, String>(1),
            "count": r.get::<_, i64>(2),
        }));
    }
    Ok(json!({
        "ok": true,
        "since_ms": since_ms,
        "sessions": total,
        "by_bot_verdict": by_bot,
        "by_real_band": by_band,
        "by_action_proxy": by_action_proxy,
        "by_site_bot": by_site_bot,
        "action_source": if used_product_action { "product_action_denorm" } else { "bot_verdict_proxy" },
        "note": "by_action_proxy prefers analysis_latest.product_action (analyze-time). Live recommended_action at get_result still applies panel strategy.",
    }))
}

fn ops_probe_completeness_inner(c: &mut Client, since_ms: i64) -> Result<Value, StoreError> {
    // Ensure velocity table exists (best-effort).
    let _ = c.batch_execute(
        r#"
        CREATE TABLE IF NOT EXISTS velocity_hits (
          id BIGSERIAL PRIMARY KEY,
          key_kind TEXT NOT NULL,
          key_value TEXT NOT NULL,
          session_id TEXT,
          hit_ms BIGINT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_velocity_hits_kv_ms
          ON velocity_hits(key_kind, key_value, hit_ms DESC);
        "#,
    );
    let rows = c
        .query(
            r#"
            SELECT
              COALESCE(NULLIF(site_id,''), '(none)') AS site_id,
              COALESCE(NULLIF(product_version,''), '(none)') AS product_version,
              COUNT(*)::bigint AS n,
              COUNT(*) FILTER (
                WHERE os_status IS NOT NULL AND os_status <> '' AND os_status <> 'missing_probe'
              )::bigint AS n_probed,
              COUNT(*) FILTER (
                WHERE os_status IS NULL OR os_status = '' OR os_status = 'missing_probe'
              )::bigint AS n_missing_probe,
              COUNT(*) FILTER (WHERE digest_path = 'gateway_only_v1')::bigint AS n_gateway_only,
              COUNT(*) FILTER (WHERE digest_path = 'real_curves_v1')::bigint AS n_real_curves,
              COUNT(*) FILTER (
                WHERE EXISTS (
                  SELECT 1 FROM probe_batches pb
                  WHERE pb.session_id = analysis_latest.session_id
                    AND pb.batch_id = 'B0_bootstrap'
                ) AND EXISTS (
                  SELECT 1 FROM probe_batches pb2
                  WHERE pb2.session_id = analysis_latest.session_id
                    AND pb2.batch_id = 'B10_hw_curves'
                )
              )::bigint AS n_main_complete
            FROM analysis_latest
            WHERE created_ms >= $1
            GROUP BY 1, 2
            ORDER BY n DESC
            LIMIT 200
            "#,
            &[&since_ms],
        )
        .map_err(|e| StoreError::Msg(format!("ops_probe_completeness: {e}")))?;
    let mut by = Vec::new();
    let mut total = 0i64;
    let mut total_main = 0i64;
    let mut total_missing = 0i64;
    for r in rows {
        let n: i64 = r.get(2);
        let n_probed: i64 = r.get(3);
        let n_miss: i64 = r.get(4);
        let n_gw: i64 = r.get(5);
        let n_rc: i64 = r.get(6);
        let n_main: i64 = r.get(7);
        total += n;
        total_main += n_main;
        total_missing += n_miss;
        by.push(json!({
            "site_id": r.get::<_, String>(0),
            "product_version": r.get::<_, String>(1),
            "sessions": n,
            "probed": n_probed,
            "missing_probe": n_miss,
            "missing_probe_rate": if n > 0 { n_miss as f64 / n as f64 } else { 0.0 },
            "gateway_only": n_gw,
            "gateway_only_rate": if n > 0 { n_gw as f64 / n as f64 } else { 0.0 },
            "real_curves": n_rc,
            "main_complete": n_main,
            "main_complete_rate": if n > 0 { n_main as f64 / n as f64 } else { 0.0 },
            "probed_rate": if n > 0 { n_probed as f64 / n as f64 } else { 0.0 },
        }));
    }
    Ok(json!({
        "ok": true,
        "since_ms": since_ms,
        "sessions": total,
        "main_complete": total_main,
        "main_complete_rate": if total > 0 { total_main as f64 / total as f64 } else { 0.0 },
        "missing_probe": total_missing,
        "missing_probe_rate": if total > 0 { total_missing as f64 / total as f64 } else { 0.0 },
        "by_site_version": by,
        "definition": {
            "main_complete": "has probe_batches B0_bootstrap AND B10_hw_curves",
            "missing_probe": "analysis_latest.os_status is null/empty/missing_probe",
            "probed": "os_status set and not missing_probe",
        }
    }))
}

fn velocity_record_inner(
    c: &mut Client,
    session_id: &str,
    device_id: Option<&str>,
    client_ip: Option<&str>,
    hit_ms: i64,
) -> Result<(), StoreError> {
    let _ = c.batch_execute(
        r#"
        CREATE TABLE IF NOT EXISTS velocity_hits (
          id BIGSERIAL PRIMARY KEY,
          key_kind TEXT NOT NULL,
          key_value TEXT NOT NULL,
          session_id TEXT,
          hit_ms BIGINT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_velocity_hits_kv_ms
          ON velocity_hits(key_kind, key_value, hit_ms DESC);
        "#,
    );
    if let Some(d) = device_id.map(str::trim).filter(|s| !s.is_empty()) {
        c.execute(
            "INSERT INTO velocity_hits(key_kind, key_value, session_id, hit_ms) VALUES('device_id',$1,$2,$3)",
            &[&d, &session_id, &hit_ms],
        )
        .map_err(|e| StoreError::Msg(format!("velocity_record device: {e}")))?;
    }
    if let Some(ip) = client_ip.map(str::trim).filter(|s| !s.is_empty()) {
        c.execute(
            "INSERT INTO velocity_hits(key_kind, key_value, session_id, hit_ms) VALUES('client_ip',$1,$2,$3)",
            &[&ip, &session_id, &hit_ms],
        )
        .map_err(|e| StoreError::Msg(format!("velocity_record ip: {e}")))?;
    }
    Ok(())
}

fn velocity_summary_inner(
    c: &mut Client,
    device_id: Option<&str>,
    client_ip: Option<&str>,
    now_ms: i64,
) -> Result<Value, StoreError> {
    let windows = [300_000i64, 3_600_000, 86_400_000]; // 5m, 1h, 24h
    let mut device = json!({});
    let mut ip = json!({});
    for w in windows {
        let since = now_ms - w;
        let label = match w {
            300_000 => "5m",
            3_600_000 => "1h",
            _ => "24h",
        };
        if let Some(d) = device_id.map(str::trim).filter(|s| !s.is_empty()) {
            let n: i64 = c
                .query_one(
                    "SELECT COUNT(*)::bigint FROM velocity_hits WHERE key_kind='device_id' AND key_value=$1 AND hit_ms>=$2",
                    &[&d, &since],
                )
                .map(|r| r.get(0))
                .unwrap_or(0);
            device[label] = json!(n);
        }
        if let Some(addr) = client_ip.map(str::trim).filter(|s| !s.is_empty()) {
            let n: i64 = c
                .query_one(
                    "SELECT COUNT(*)::bigint FROM velocity_hits WHERE key_kind='client_ip' AND key_value=$1 AND hit_ms>=$2",
                    &[&addr, &since],
                )
                .map(|r| r.get(0))
                .unwrap_or(0);
            ip[label] = json!(n);
        }
    }
    // Score 0..1 from device 1h hits (soft)
    let d1h = device
        .get("1h")
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
        .max(0) as f64;
    let score = (d1h / 20.0).clamp(0.0, 1.0);
    Ok(json!({
        "ok": true,
        "device_id": device,
        "client_ip": ip,
        "velocity_score": score,
        "windows_ms": { "5m": 300_000, "1h": 3_600_000, "24h": 86_400_000 },
    }))
}

fn ops_b10_health_inner(c: &mut Client, since_ms: i64, limit_sites: i64) -> Result<Value, StoreError> {
    // Sessions created since_ms: has B10 batch ratio, by engine if available in meta
    let rows = c.query(
        r#"
        WITH sess AS (
          SELECT s.session_id, s.visitor_terminal_id, s.created_ms, s.meta_json,
                 EXISTS(
                   SELECT 1 FROM probe_batches pb
                   WHERE pb.session_id = s.session_id AND pb.batch_id = 'B10_hw_curves'
                 ) AS has_b10
          FROM sessions s
          WHERE s.created_ms >= $1
        )
        SELECT
          COALESCE(
            NULLIF(meta_json::json->'belief'->'axes'->'engine'->>'claim',''),
            NULLIF(meta_json::json->>'engine_family',''),
            'unknown'
          ) AS engine,
          COUNT(*)::bigint AS n,
          COUNT(*) FILTER (WHERE has_b10)::bigint AS n_b10
        FROM sess
        GROUP BY 1
        ORDER BY n DESC
        LIMIT $2
        "#,
        &[&since_ms, &limit_sites],
    ).map_err(|e| StoreError::Msg(format!("ops_b10_health: {e}")))?;
    let mut by_engine = Vec::new();
    let mut total = 0i64;
    let mut total_b10 = 0i64;
    for r in rows {
        let n: i64 = r.get(1);
        let nb: i64 = r.get(2);
        total += n;
        total_b10 += nb;
        let rate = if n > 0 { nb as f64 / n as f64 } else { 0.0 };
        by_engine.push(json!({
            "engine": r.get::<_, String>(0),
            "sessions": n,
            "with_b10": nb,
            "b10_rate": rate,
        }));
    }
    // Top error codes last window
    let err_rows = c.query(
        r#"
        SELECT code, COUNT(*)::bigint AS n FROM (
          SELECT code, ts_ms AS t FROM ops_server_events WHERE ts_ms >= $1
          UNION ALL
          SELECT code, server_recv_ms AS t FROM ops_client_events WHERE server_recv_ms >= $1
        ) u GROUP BY code ORDER BY n DESC LIMIT 20
        "#,
        &[&since_ms],
    ).map_err(|e| StoreError::Msg(format!("ops_error_top: {e}")))?;
    let mut top_errors = Vec::new();
    for r in err_rows {
        top_errors.push(json!({"code": r.get::<_, String>(0), "n": r.get::<_, i64>(1)}));
    }
    let rate = if total > 0 { total_b10 as f64 / total as f64 } else { 0.0 };
    Ok(json!({
        "ok": true,
        "since_ms": since_ms,
        "sessions": total,
        "with_b10": total_b10,
        "b10_rate": rate,
        "by_engine": by_engine,
        "top_errors": top_errors,
    }))
}


fn redact_dsn(dsn: &str) -> String {
    if let Some(at) = dsn.find('@') {
        if let Some(scheme_end) = dsn.find("://") {
            let head = &dsn[..scheme_end + 3];
            let creds = &dsn[scheme_end + 3..at];
            let tail = &dsn[at..];
            if let Some(colon) = creds.find(':') {
                return format!("{}{}:***{}", head, &creds[..colon], tail);
            }
        }
    }
    "postgres".into()
}
