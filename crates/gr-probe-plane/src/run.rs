//! Probe plane process entry (library form for embedding in gr-service).

use clap::Parser;
use gr_probe_store::{Store, ANALYZE_DEBOUNCE_MS};
use crate::handlers::{analyze_worker_loop, AppState, ServiceRole};
use log::{info, warn};
use pingora::listeners::tls::TlsSettings;
use pingora::prelude::*;
use pingora::server::configuration::Opt as PingoraOpt;
use crate::service::GrService;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;
use std::time::Duration;

use crate::{admin, handlers, http_util, listen, r100_hub, sni_map, soft_backends, tls_fp, upstream};

#[derive(Parser, Debug)]
#[command(name = "gr-probe-plane", about = "Green V7 probe plane (Pingora gateway + API)")]
pub struct Args {
    /// Plain HTTP listen (Pingora TCP), e.g. 0.0.0.0:28765
    #[arg(long, default_value = "0.0.0.0:28765")]
    pub bind: String,

    /// Optional HTTPS listen (requires --tls-cert/--tls-key), e.g. 0.0.0.0:8443.
    /// Direct-bind pv/gv edge (panel-bound domains, Pingora TLS + JA4 + SNI map);
    /// no reverse proxy in front. Empty = plain HTTP only.
    #[arg(long, default_value = "", env = "GR_BIND_TLS")]
    pub bind_tls: String,

    /// Ignored. Probe store is PostgreSQL-only (`GR_DATABASE_URL` / `--database-url`; legacy GR_/GR_ accepted).
    #[arg(long, default_value = "unused", hide = true)]
    pub db: PathBuf,

    #[arg(long, default_value = "", env = "GR_DATABASE_URL")]
    pub database_url: String,

    /// Soft mid/deep amplify (server-side only; FE cannot override — A-BRAIN-3 / norm/04).
    #[arg(long, default_value_t = false, env = "GR_SOFT_V2_READY")]
    pub soft_v2_ready: bool,

    /// HMAC secret for H16 challenge seeds (required for production; lab fallback only if GR_ALLOW_LAB_CHALLENGE=1; legacy GR_/GR_ accepted).
    #[arg(long, default_value = "", env = "GR_CHALLENGE_SECRET")]
    pub challenge_secret: String,

    /// CORS policy (A-SVC-2). Multi-tenant production must **not** hardcode customer sites:
    /// register hostnames (+ SSL) in admin panel → dynamic CORS. Values:
    /// `*` = lab open; `admin` = only admin domains; comma list = extra fixed origins (lab/partners).
    #[arg(long, default_value = "*", env = "GR_CORS_ORIGINS")]
    pub cors_origins: String,

    /// Optional bearer for /v1/session/*/result and evidence (A-SVC-4). Empty = open (lab).
    /// Production: set token or `GR_REQUIRE_RESULT_TOKEN=1` / `GR_DEPLOY_ENV=prod`.
    #[arg(long, default_value = "", env = "GR_RESULT_TOKEN")]
    pub result_token: String,

    #[arg(long, default_value = "fe")]
    pub static_dir: PathBuf,

    /// Role: all | ingest | gateway | analyze
    #[arg(long, default_value = "all")]
    pub role: String,

    /// Per-process analyze claimer loops. Prefer 1 on multi-process deploy;
    /// total claimers = process_count × this (empty-queue poll cost scales linearly).
    #[arg(long, default_value_t = 1)]
    pub analyze_workers: usize,

    #[arg(long, default_value = "")]
    pub worker_id: String,

    #[arg(long, default_value_t = ANALYZE_DEBOUNCE_MS)]
    pub analyze_debounce_ms: i64,

    #[arg(long, default_value = "data/soft_store")]
    pub soft_store_dir: PathBuf,

    /// Soft store backend: file | store (SQLite/PG tables) | redis
    #[arg(long, default_value = "store", env = "GR_SOFT_STORE_BACKEND")]
    pub soft_store_backend: String,

    /// Redis URL when --soft-store-backend=redis and/or R100 templates (redis://host:port/db)
    #[arg(long, default_value = "", env = "GR_REDIS_URL")]
    pub redis_url: String,

    /// R100 template SSOT JSON (pseudo-static pack meta). Default: data/r100_templates.json
    #[arg(long, default_value = "data/r100_templates.json", env = "GR_R100_TEMPLATES")]
    pub r100_templates: PathBuf,

    /// TLS certificate PEM (default/SNI-fallback cert; enables --bind-tls JA4 path).
    /// Panel-minted per-domain certs load via --sni-map / GR_SNI_MAP (data/sni-map.json).
    #[arg(long, default_value = "", env = "GR_TLS_CERT")]
    pub tls_cert: String,

    /// TLS private key PEM (default/SNI-fallback)
    #[arg(long, default_value = "", env = "GR_TLS_KEY")]
    pub tls_key: String,

    /// Enable HTTP/2 on TLS listener
    #[arg(long, default_value_t = true, env = "GR_TLS_H2")]
    pub tls_h2: bool,

    /// QUIC/HTTP3 Initial passive UDP listen (empty=disabled). e.g. 0.0.0.0:28443
    #[arg(long, default_value = "")]
    pub quic_listen: String,

    /// Full HTTP/3 application server (Quinn+h3). Uses --tls-cert/--tls-key.
    /// e.g. 127.0.0.1:28495 — when set on same port as quic_listen, H3 app wins.
    #[arg(long, default_value = "")]
    pub h3_listen: String,

    /// WebRTC/STUN UDP listen (empty=disabled). e.g. 0.0.0.0:3478
    #[arg(long, default_value = "")]
    pub webrtc_listen: String,

    /// Enable TCP SYN AF_PACKET listen (needs CAP_NET_RAW)
    #[arg(long, default_value_t = false)]
    pub syn_listen: bool,

    /// Enable HAProxy PROXY protocol PreTls
    #[arg(long, default_value_t = false)]
    pub proxy_protocol: bool,

    /// Enable SO_REUSEPORT (multi-process horizontal scale) — P-A5
    #[arg(long, default_value_t = false)]
    pub reuse_port: bool,

    /// SNI multi-cert JSON map path (or env GR_SNI_MAP, legacy GR_/GR_) — P-V2
    #[arg(long, default_value = "", env = "GR_SNI_MAP")]
    pub sni_map: String,

    /// Proxy mode: origin (default) | transparent — P-V1
    #[arg(long, default_value = "origin")]
    pub mode: String,

    /// Upstream host:port for transparent mode
    #[arg(long, default_value = "")]
    pub upstream: String,

    /// Upstream uses TLS
    #[arg(long, default_value_t = false)]
    pub upstream_tls: bool,

    /// Upstream SNI (default: host part of --upstream)
    #[arg(long, default_value = "")]
    pub upstream_sni: String,

    /// Admin console listen (ops UI). Empty = disabled. Default :28770
    #[arg(long, default_value = "0.0.0.0:28770", env = "GR_ADMIN_BIND")]
    pub admin_bind: String,

    /// Admin config store data-dir marker. PostgreSQL via `GR_ADMIN_DATABASE_URL` (legacy GR_/GR_ accepted).
    /// Parent directory is still used for TLS cert files.
    #[arg(long, default_value = "run/admin/admin.pg", env = "GR_ADMIN_DB")]
    pub admin_db: PathBuf,

    /// Shared LB pool (wired by gr-service control plane; not a CLI arg).
    #[arg(skip)]
    pub lb_pool: Option<Arc<gr_lb::LbPool>>,
}

fn mirror_one_env_alias(name: &str) {
    let gr = format!("GR_{name}");
    let gr = format!("GR_{name}");
    let gr = format!("GR_{name}");
    let Some(value) = std::env::var(&gr)
        .ok()
        .or_else(|| std::env::var(&gr).ok())
        .or_else(|| std::env::var(&gr).ok())
    else {
        return;
    };
    std::env::set_var(&gr, &value);
    std::env::set_var(&gr, &value);
    std::env::set_var(&gr, &value);
}

/// Mirror GR/GR/GR env aliases before clap parses single-env fields.
/// New code should set GR_*; legacy names remain accepted for standalone probes.
pub fn mirror_env_aliases() {
    for name in [
        "DATABASE_URL",
        "SOFT_V2_READY",
        "CHALLENGE_SECRET",
        "ALLOW_LAB_CHALLENGE",
        "CORS_ORIGINS",
        "RESULT_TOKEN",
        "SITE_RESULT_TOKENS",
        "REQUIRE_RESULT_TOKEN",
        "SOFT_STORE_BACKEND",
        "REDIS_URL",
        "R100_TEMPLATES",
        "SNI_MAP",
        "ADMIN_BIND",
        "ADMIN_DB",
        "ADMIN_DATABASE_URL",
        "BIZ_DATABASE_URL",
        "ASSOCIATION_DATABASE_URL",
        "DEPLOY_ENV",
    ] {
        mirror_one_env_alias(name);
    }
}

fn is_prod_deploy_env() -> bool {
    // GR naming migration (docs/13): GR_DEPLOY_ENV preferred, GR_/GR_
    // fallbacks keep old deployments readable; bare DEPLOY_ENV last.
    let env = gr_abi::env::get("DEPLOY_ENV")
        .or_else(|| std::env::var("DEPLOY_ENV").ok())
        .unwrap_or_default();
    let e = env.trim().to_ascii_lowercase();
    matches!(e.as_str(), "prod" | "production" | "live")
}

/// Resolve challenge HMAC secret (A-SVC-1 / norm production gate).
fn resolve_challenge_secret(cli: &str) -> String {
    let from_cli = cli.trim();
    if !from_cli.is_empty() {
        if from_cli.len() < 16 {
            warn!("GR_CHALLENGE_SECRET short (<16); prefer long random secret");
        }
        return from_cli.to_string();
    }
    if let Some(e) = gr_abi::env::get("CHALLENGE_SECRET") {
        let t = e.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    // Lab-only fallback — production must set env
    // Default: allow lab when not prod; prod deploy or explicit 0 forbids fallback.
    let allow_lab = if is_prod_deploy_env() {
        gr_abi::env::get("ALLOW_LAB_CHALLENGE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    } else {
        gr_abi::env::get("ALLOW_LAB_CHALLENGE")
            .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
            .unwrap_or(true)
    };
    if !allow_lab {
        panic!(
            "GR_CHALLENGE_SECRET required in production (set secret or GR_ALLOW_LAB_CHALLENGE=1 only for lab)"
        );
    }
    warn!(
        "using DEFAULT_CHALLENGE_SECRET lab constant — set GR_CHALLENGE_SECRET for production (A-SVC-1)"
    );
    gr_probe_core::DEFAULT_CHALLENGE_SECRET.to_string()
}

/// Production result API must not stay open (A-SVC-4).
fn enforce_result_token_policy(result_token: &str) {
    let require = is_prod_deploy_env()
        || gr_abi::env::get("REQUIRE_RESULT_TOKEN")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
    if !require {
        return;
    }
    let tok = result_token.trim();
    let env_tok = gr_abi::env::get("RESULT_TOKEN").unwrap_or_default();
    let site_map = gr_abi::env::get("SITE_RESULT_TOKENS").unwrap_or_default();
    if tok.is_empty() && env_tok.trim().is_empty() && site_map.trim().is_empty() {
        panic!(
            "GR_RESULT_TOKEN (or GR_SITE_RESULT_TOKENS) required when GR_DEPLOY_ENV=prod or GR_REQUIRE_RESULT_TOKEN=1"
        );
    }
}

/// Periodic commercial retention purge (analysis/session/velocity/cold) — small batches only.
/// Interval / batch size / TTLs come from shared `panel_policy.json` written by the control plane.
fn spawn_retention_purge_loop(
    store: Arc<Store>,
    worker_id: &str,
    admin: Option<Arc<admin::AdminHub>>,
) {
    let wid = worker_id.to_string();
    std::thread::Builder::new()
        .name("gr-retention-purge".into())
        .spawn(move || {
            info!("retention purge loop started worker={wid}");
            // Stagger start so multi-node nodes don't purge in lockstep.
            let stagger_ms = {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                let mut h = DefaultHasher::new();
                wid.hash(&mut h);
                5_000 + (h.finish() % 25_000)
            };
            std::thread::sleep(Duration::from_millis(stagger_ms));
            // iss/opus5 04-P1-6: adaptive batch multiplier (1x..4x), see loop body.
            let mut purge_scale: i64 = 1;
            loop {
                let data_dir = admin.as_ref().map(|a| {
                    a.db.data_dir
                        .parent()
                        .map(|p| p.to_path_buf())
                        .unwrap_or_else(|| a.db.data_dir.clone())
                });
                let policy = gr_probe_core::load_panel_policy(data_dir.as_deref());
                let ret = &policy.retention;
                let interval_sec = ret.purge_interval_sec.max(30) as u64;
                if !ret.enabled {
                    std::thread::sleep(Duration::from_secs(interval_sec));
                    continue;
                }
                let now = gr_probe_store::hot_now_ms();
                let keep_days = (ret.analysis_retention_days.min(ret.session_retention_days)).max(1)
                    as i64;
                let older_than_ms = now.saturating_sub(keep_days * 86_400_000);
                let velocity_older =
                    Some(now.saturating_sub(ret.velocity_retention_days.max(1) as i64 * 86_400_000));
                // iss/opus5 04-P1-6: windows for the previously unbounded tables.
                let ops_older =
                    Some(now.saturating_sub(ret.ops_retention_days.max(1) as i64 * 86_400_000));
                let master_older =
                    Some(now.saturating_sub(ret.master_retention_days.max(1) as i64 * 86_400_000));
                // iss/opus5 04-P1-6 adaptive batch: grow while the backlog keeps
                // filling whole batches (up to 4x), shrink back when a tick
                // deletes less than the batch size.
                let base_limit = ret.batch_delete_limit.clamp(10, 5000) as i64;
                let limit = (base_limit * purge_scale).clamp(10, 5000);
                // One batch per tick — never multi-batch storms in the background loop.
                match store.retention_purge_batch(
                    older_than_ms,
                    limit,
                    velocity_older,
                    ops_older,
                    master_older,
                ) {
                    Ok(v) => {
                        let get = |k: &str| v.get(k).and_then(|x| x.as_i64()).unwrap_or(0);
                        let n_a = get("deleted_analysis");
                        let n_s = get("deleted_sessions");
                        let n_v = get("deleted_velocity");
                        let n_c = get("deleted_cold");
                        let n_ext = get("deleted_page_results")
                            + get("deleted_observation_events")
                            + get("deleted_ops_client_events")
                            + get("deleted_ops_server_events")
                            + get("deleted_api_idempotency")
                            + get("deleted_devices")
                            + get("deleted_device_sessions")
                            + get("deleted_soft_edges")
                            + get("deleted_soft_heat")
                            + get("deleted_device_index_devices")
                            + get("deleted_device_index_keys");
                        let total = n_a + n_s + n_v + n_c + n_ext;
                        if total > 0 {
                            info!(
                                "retention purge worker={wid} analysis={n_a} sessions={n_s} velocity={n_v} cold={n_c} other={n_ext} limit={limit} keep_days={keep_days}"
                            );
                        }
                        // Full batch → backlog likely remains: scale up (max 4x).
                        // Partial batch → caught up: decay back toward base.
                        if total >= limit {
                            purge_scale = (purge_scale * 2).min(4);
                        } else if purge_scale > 1 {
                            purge_scale = (purge_scale / 2).max(1);
                        }
                    }
                    Err(e) => warn!("retention purge failed worker={wid}: {e}"),
                }
                std::thread::sleep(Duration::from_secs(interval_sec));
            }
        })
        .ok();
}

/// Periodic L3 cold purge (shared Postgres). Interval 0 disables.
/// Multi-node safe: DELETE WHERE created_ms < cutoff is idempotent.
fn spawn_cold_purge_loop(
    store: Arc<Store>,
    worker_id: &str,
    admin: Option<Arc<admin::AdminHub>>,
) {
    let interval_ms: u64 = gr_abi::env::get("COLD_PURGE_INTERVAL_MS")
        .and_then(|s| s.parse().ok())
        .or_else(|| {
            let c = gr_probe_store::get_runtime_cfg();
            if c.version > 0 {
                Some(c.cold_purge_interval_ms)
            } else {
                None
            }
        })
        .unwrap_or(300_000); // 5 min
    if interval_ms == 0 {
        info!("cold purge loop disabled (GR_COLD_PURGE_INTERVAL_MS=0)");
        return;
    }
    let wid = worker_id.to_string();
    std::thread::Builder::new()
        .name("gr-cold-purge".into())
        .spawn(move || {
            info!(
                "cold purge loop started interval_ms={interval_ms} worker={wid} ttl_ms={}",
                gr_probe_store::cold_ttl_ms()
            );
            loop {
                // Re-read interval from panel config when published
                let sleep_ms = {
                    let c = gr_probe_store::get_runtime_cfg();
                    if c.version > 0 && c.cold_purge_interval_ms > 0 {
                        c.cold_purge_interval_ms
                    } else {
                        interval_ms
                    }
                };
                std::thread::sleep(Duration::from_millis(sleep_ms.max(10_000)));
                let ttl = gr_probe_store::cold_ttl_ms();
                let cutoff = gr_probe_store::hot_now_ms() - ttl;
                match store.purge_expired_cold(cutoff) {
                    Ok(n) => {
                        if n > 0 {
                            info!("cold purge deleted={n} cutoff_ms={cutoff} worker={wid}");
                        }
                        if let Some(ref hub) = admin {
                            admin::panel_config::record_purge(&hub.db, n, ttl);
                        }
                    }
                    Err(e) => warn!("cold purge failed: {e}"),
                }
            }
        })
        .ok();
}

/// Reload panel config from shared admin DB + write cluster heartbeat.
/// Handle for one hot-scalable analyze worker loop (gpt5.5 P1).
struct AnalyzeWorkerHandle {
    cancel: Arc<AtomicBool>,
    done: tokio::task::JoinHandle<()>,
}

/// Spawn one analyze worker and register it in the hot-scale registry.
fn push_analyze_worker(
    rt: &tokio::runtime::Handle,
    store: &Arc<Store>,
    soft_v2_ready: bool,
    worker_base: &str,
    analyze_runs: &Arc<AtomicU64>,
    seq: &Arc<std::sync::atomic::AtomicU64>,
    registry: &mut Vec<AnalyzeWorkerHandle>,
) {
    let id = seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let cancel = Arc::new(AtomicBool::new(false));
    let wid = format!("{worker_base}-a{id}");
    let store = store.clone();
    let runs = analyze_runs.clone();
    let cancel_task = cancel.clone();
    let done = rt.spawn(async move {
        analyze_worker_loop(store, soft_v2_ready, wid, runs, cancel_task).await;
    });
    registry.push(AnalyzeWorkerHandle { cancel, done });
}

fn spawn_config_and_heartbeat(hub: Arc<admin::AdminHub>, state: Arc<handlers::AppState>) {
    std::thread::Builder::new()
        .name("gr-cfg-hb".into())
        .spawn(move || {
            // initial load
            let c0 = admin::panel_config::reload_and_apply(&hub.db);
            let _ = admin::panel_config::note_node_applied(
                &hub.db,
                serde_json::json!({
                    "worker_id": state.worker_id,
                    "node_id": gr_abi::env::get("NODE_ID")
                        .unwrap_or_else(|| "local".into()),
                    "role": format!("{:?}", state.role).to_ascii_lowercase(),
                    "version": c0.version,
                    "source": "startup",
                }),
            );
            refresh_cors_from_admin(&hub);
            let mut tick: u64 = 0;
            loop {
                // CORS every 5s so multi-worker picks up panel domain changes quickly;
                // full panel config + heartbeat every 30s.
                std::thread::sleep(Duration::from_secs(5));
                tick = tick.wrapping_add(1);
                refresh_cors_from_admin(&hub);
                if tick % 6 != 0 {
                    continue;
                }
                let c = admin::panel_config::reload_and_apply(&hub.db);
                // gpt5.5 P1: panel Workers setting feeds the live analyze
                // target (≤30s); the analyze supervisor reconciles every 2s.
                // Absent table (legacy sqlite/lab) keeps the boot-time target.
                match hub.db.get_worker_settings() {
                    Ok((a, _i, _g)) => {
                        let target = a.max(1) as usize;
                        let prev = state
                            .analyze_workers_target
                            .load(std::sync::atomic::Ordering::Relaxed);
                        if prev != target {
                            state
                                .analyze_workers_target
                                .store(target, std::sync::atomic::Ordering::Relaxed);
                            info!(
                                "analyze workers target {prev} -> {target} (panel worker_settings)"
                            );
                        }
                    }
                    Err(e) => {
                        // Absent table (legacy sqlite/lab) keeps the boot target.
                        log::debug!("worker_settings poll unavailable: {e}");
                    }
                }
                let beat = serde_json::json!({
                    "worker_id": state.worker_id,
                    "role": format!("{:?}", state.role).to_ascii_lowercase(),
                    "product_version": gr_probe_core::GR_PRODUCT_VERSION,
                    "config_version": c.version,
                    "soft_backend": state.soft_store_backend,
                    "backend": state.store.backend_name(),
                    "hot_vts": state.hot_probe.len_hot(),
                    "analyze_runs": state.analyze_runs.load(std::sync::atomic::Ordering::Relaxed),
                });
                if let Err(e) = admin::panel_config::heartbeat_upsert(&hub.db, beat) {
                    warn!("heartbeat upsert: {e}");
                }
                // Per-node applied view: record the version this node is running.
                if let Err(e) = admin::panel_config::note_node_applied(
                    &hub.db,
                    serde_json::json!({
                        "worker_id": state.worker_id,
                        "node_id": gr_abi::env::get("NODE_ID")
                            .unwrap_or_else(|| "local".into()),
                        "role": format!("{:?}", state.role).to_ascii_lowercase(),
                        "version": c.version,
                        "source": "periodic",
                    }),
                ) {
                    warn!("note_node_applied: {e}");
                }
            }
        })
        .ok();
}

/// Load collect-enabled hostnames from admin DB into dynamic CORS allowlist.
fn refresh_cors_from_admin(hub: &admin::AdminHub) {
    match hub.db.list_domains(None, None) {
        Ok(rows) => {
            let mut hosts = Vec::new();
            for r in rows {
                let enabled = r
                    .get("collect_enabled")
                    .and_then(|v| v.as_bool())
                    .or_else(|| r.get("collect_enabled").and_then(|v| v.as_i64()).map(|i| i != 0))
                    .unwrap_or(true);
                if !enabled {
                    continue;
                }
                if let Some(h) = r.get("hostname").and_then(|v| v.as_str()) {
                    hosts.push(h.to_string());
                }
            }
            let n = hosts.len();
            // P1-3 (iss/grok4.6/05): this refresh runs on a short poll — log
            // only when the host set actually changes (was ~58% of node logs).
            static LAST_CORS_HOSTS: std::sync::Mutex<Option<Vec<String>>> =
                std::sync::Mutex::new(None);
            let changed = {
                let mut last = LAST_CORS_HOSTS.lock().unwrap_or_else(|p| p.into_inner());
                let changed = last.as_ref() != Some(&hosts);
                if changed {
                    *last = Some(hosts.clone());
                }
                changed
            };
            http_util::set_dynamic_cors_hosts(hosts);
            if changed {
                info!("cors dynamic hosts loaded from admin domains n={n}");
            }
        }
        Err(e) => warn!("cors dynamic hosts load failed: {e}"),
    }
}

pub fn run_with(args: Args) {
    let role = ServiceRole::parse(&args.role);
    let worker_id = if args.worker_id.is_empty() {
        format!(
            "{}-{}-{:?}",
            std::env::var("HOSTNAME").unwrap_or_else(|_| "host".into()),
            std::process::id(),
            role
        )
    } else {
        args.worker_id.clone()
    };

    let mut db_url = args.database_url.trim().to_string();
    if db_url.is_empty() {
        db_url = gr_abi::env::get("DATABASE_URL").unwrap_or_default();
    }
    let store = Arc::new(
        Store::open_auto(
            &args.db,
            if db_url.trim().is_empty() {
                None
            } else {
                Some(db_url.trim())
            },
        )
        .unwrap_or_else(|e| panic!("open store: {e}")),
    );
    info!(
        "store opened backend={} db={}",
        store.backend_name(),
        store.backend_label()
    );

    let soft_backend = args.soft_store_backend.clone();
    let soft_store = soft_backends::open_soft_store(
        &soft_backend,
        &args.soft_store_dir,
        store.clone(),
        if args.redis_url.is_empty() {
            None
        } else {
            Some(args.redis_url.as_str())
        },
    )
    .unwrap_or_else(|e| {
        warn!("soft store backend={soft_backend} open failed: {e}; fallback file");
        soft_backends::open_soft_store(
            "file",
            &args.soft_store_dir,
            store.clone(),
            None,
        )
        .expect("soft store file fallback")
    });
    info!(
        "soft_store backend={} fuse={}",
        soft_backend,
        soft_store.fuse_threshold()
    );

    let r100 = Arc::new(r100_hub::R100Hub::load(
        Some(args.r100_templates.as_path()),
        if args.redis_url.is_empty() {
            None
        } else {
            Some(args.redis_url.as_str())
        },
        // Anchor path resolution to static-dir parent (repo root / deploy root)
        // so gateway started with cwd≠repo still finds data/r100_templates.json.
        Some(args.static_dir.as_path()),
    ));

    let challenge_secret = resolve_challenge_secret(&args.challenge_secret);
    enforce_result_token_policy(&args.result_token);
    // Production defaults (override only with explicit env=0 before start).
    if is_prod_deploy_env() {
        // Force sealed ingest unless operator explicitly set 0 before start.
        if gr_abi::env::get("REQUIRE_SEALED_INGEST").is_none()
            && gr_abi::env::get("REQUIRE_SEALED_INGEST").is_none()
        {
            std::env::set_var("GR_REQUIRE_SEALED_INGEST", "1");
            info!("prod default: GR_REQUIRE_SEALED_INGEST=1 (set 0 + ALLOW_PLAIN for lab break-glass)");
        }
        // Tighten CORS when still lab-open.
        let cors = gr_abi::env::get("CORS_ORIGINS").unwrap_or_default();
        if cors.trim().is_empty() || cors.trim() == "*" {
            if !gr_abi::env::get("ALLOW_OPEN_CORS")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false)
            {
                std::env::set_var("GR_CORS_ORIGINS", "admin");
                info!("prod default: GR_CORS_ORIGINS=admin (registered domains only)");
            }
        }
        if gr_abi::env::get("REQUIRE_OPS_AUTH").is_none() {
            std::env::set_var("GR_REQUIRE_OPS_AUTH", "1");
            info!("prod default: GR_REQUIRE_OPS_AUTH=1 (X-Gr-Ops-Token / result token / admin session)");
        }
    }
    let analyze_runs = Arc::new(AtomicU64::new(0));
    let analyze_workers_target =
        Arc::new(std::sync::atomic::AtomicUsize::new(args.analyze_workers.max(1)));
    // CORS / result-token policy (process-wide via env for http_util + handlers)
    // Multi-tenant: customer domains live in admin panel; env is lab extras / open / admin-only.
    if !args.cors_origins.is_empty() {
        std::env::set_var("GR_CORS_ORIGINS", &args.cors_origins);
    }
    if is_prod_deploy_env() && args.cors_origins.trim() == "*" {
        warn!(
            "GR_CORS_ORIGINS=* under production deploy — prefer admin domains (GR_CORS_ORIGINS=admin) so only panel-registered hosts are allowed"
        );
    }
    if !args.result_token.is_empty() {
        std::env::set_var("GR_RESULT_TOKEN", &args.result_token);
    }

    let static_dir_early = if args.static_dir.is_dir() {
        args.static_dir.clone()
    } else {
        PathBuf::from("fe")
    };
    // BizStore + AdminDb (SDK keys / sites) must attach on ingest/gateway workers too.
    // Previously hub only opened when --admin-bind was set; multi-worker then returned
    // biz_store_unavailable on POST /v1/biz/visit and skipped open/pixel 辅写.
    // Admin TCP listen remains gated on non-empty admin_bind (one process only).
    // Analyze / all roles also need shared admin DB for panel config reload + heartbeat + cold purge audit.
    let need_biz_hub = role.serves_ingest()
        || role.serves_gateway()
        || role.runs_analyze_workers()
        || !args.admin_bind.trim().is_empty();
    let admin_hub = if need_biz_hub {
        // 多节点冷启动竞态: admin schema 初始化瞬时失败 (PG 连接/DDL 冲突) 时,
        // 一次性失败会让该平面永久丢失 admin hub (无心跳/面板配置/动态 CORS)。
        // 有限重试后再放弃 — 2s × 5 ≈ 10s, 覆盖同批容器 schema 初始化窗口。
        let mut hub: Option<std::sync::Arc<admin::AdminHub>> = None;
        let mut last_err = String::new();
        for attempt in 1..=5 {
            match admin::AdminHub::open(&args.admin_db, static_dir_early.clone()) {
                Ok(h) => {
                    hub = Some(h);
                    break;
                }
                Err(e) => {
                    last_err = e;
                    warn!(
                        "admin/biz hub open attempt {attempt}/5 failed: {last_err}; retrying in 2s"
                    );
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
            }
        }
        match hub {
            Some(h) => {
                if !args.admin_bind.trim().is_empty() {
                    let _ = admin::apply::seed_runtime_from_live(
                        &h.db,
                        &args.bind,
                        &args.bind_tls,
                        &args.admin_bind,
                        &args.role,
                        args.analyze_workers.max(1),
                        args.soft_v2_ready,
                    );
                }
                // Customer domains/SSL configured in admin → dynamic CORS hosts
                refresh_cors_from_admin(&h);
                // Multi-worker: ingest/gateway re-read domains when Origin denied (throttled).
                {
                    let hub_cors = h.clone();
                    http_util::set_cors_lazy_refresh(move || {
                        refresh_cors_from_admin(&hub_cors);
                    });
                }
                info!(
                    "biz hub attached admin_db={} admin_cfg={} bind_admin={} worker={}",
                    args.admin_db.display(),
                    h.db.backend_name(),
                    !args.admin_bind.trim().is_empty(),
                    worker_id
                );
                Some(h)
            }
            None => {
                warn!("admin/biz hub disabled after 5 attempts: {last_err}");
                None
            }
        }
    } else {
        None
    };

    let seal_secret = gr_abi::env::get("SEAL_SECRET")
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| challenge_secret.clone());
    // Fail-closed (iss/opus5 S-1): never boot with an empty seal master key —
    // downstream HMAC must not degrade to a well-known zero key. Also emit a
    // non-secret key fingerprint so ops can verify the intended key is loaded.
    if seal_secret.trim().is_empty() {
        panic!("GR_SEAL_SECRET resolved empty — refusing to start (fail-closed)");
    }
    {
        use sha2::{Digest, Sha256};
        let fp = format!("{:x}", Sha256::digest(seal_secret.as_bytes()));
        let src = if gr_abi::env::get("SEAL_SECRET").is_some() {
            "env"
        } else {
            "challenge_secret_fallback"
        };
        info!("seal secret loaded source={} sha256_fp8={}", src, &fp[..8]);
    }
    let state = Arc::new(AppState {
        store: store.clone(),
        soft_store,
        soft_store_backend: soft_backend,
        r100,
        soft_v2_ready: args.soft_v2_ready,
        challenge_secret,
        seal_secret,
        role: role.clone(),
        worker_id: worker_id.clone(),
        analyze_runs: analyze_runs.clone(),
        analyze_workers_target: analyze_workers_target.clone(),
        admin: admin_hub,
        hot_probe: Arc::new(gr_probe_store::HotProbeCache::new()),
        draining: Arc::new(AtomicBool::new(false)),
        boot_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0),
    });
    let challenge_src = if !args.challenge_secret.trim().is_empty()
        || gr_abi::env::get("CHALLENGE_SECRET")
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    {
        "env/cli"
    } else {
        "lab_default"
    };
    info!(
        "control-plane soft_v2_ready={} challenge_secret_source={}",
        args.soft_v2_ready, challenge_src
    );

    // Analyze workers on a multi-thread tokio runtime (Pingora owns its own RT for IO)
    if role.runs_analyze_workers() {
        let n = args.analyze_workers.max(1);
        // Tokio worker threads are sized for the initial target; hot-scaled
        // loops beyond this multiplex fine (evaluate runs on the blocking
        // pool, claims are short PG queries).
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(n.clamp(2, 8))
            .enable_all()
            .thread_name("gr-analyze")
            .build()
            .expect("tokio runtime");
        let rt_handle = rt.handle().clone();
        // Hot-scaling registry (gpt5.5 P1): panel Workers / runtime-apply feed
        // `analyze_workers_target`; a supervisor reconciles the live loop
        // count against it every 2s — spawns on scale-up, cancels gracefully
        // on scale-down.
        let registry: Arc<std::sync::Mutex<Vec<AnalyzeWorkerHandle>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let analyze_seq = Arc::new(std::sync::atomic::AtomicU64::new(0));
        {
            let mut reg = registry.lock().unwrap_or_else(|p| p.into_inner());
            for _ in 0..n {
                push_analyze_worker(
                    &rt_handle,
                    &store,
                    args.soft_v2_ready,
                    &worker_id,
                    &analyze_runs,
                    &analyze_seq,
                    &mut reg,
                );
            }
        }
        // Supervisor: reconcile live workers vs target every 2s.
        {
            let sup_registry = registry.clone();
            let sup_target = analyze_workers_target.clone();
            let sup_store = store.clone();
            let sup_runs = analyze_runs.clone();
            let sup_soft = args.soft_v2_ready;
            let sup_base = worker_id.clone();
            let sup_seq = analyze_seq.clone();
            let sup_rt = rt_handle.clone();
            rt_handle.spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    let target = sup_target
                        .load(std::sync::atomic::Ordering::Relaxed)
                        .max(1);
                    let mut reg = match sup_registry.lock() {
                        Ok(g) => g,
                        Err(p) => p.into_inner(),
                    };
                    // Reap finished workers first (exited or cancelled).
                    reg.retain(|h| !h.done.is_finished());
                    let stopping = reg
                        .iter()
                        .filter(|h| h.cancel.load(std::sync::atomic::Ordering::Relaxed))
                        .count();
                    let live = reg.len() - stopping;
                    if live < target {
                        let add = target - live;
                        for _ in 0..add {
                            push_analyze_worker(
                                &sup_rt,
                                &sup_store,
                                sup_soft,
                                &sup_base,
                                &sup_runs,
                                &sup_seq,
                                &mut reg,
                            );
                        }
                        info!(
                            "analyze supervisor scaled up +{add} (live={live} target={target})"
                        );
                    } else if live > target {
                        let mut retired = 0;
                        for h in reg.iter_mut().rev() {
                            if retired >= live - target {
                                break;
                            }
                            if !h.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                                h.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                                retired += 1;
                            }
                        }
                        info!(
                            "analyze supervisor retiring {retired} workers (live={live} target={target})"
                        );
                    }
                }
            });
        }
        // Keep runtime alive for process lifetime
        std::mem::forget(rt);
        info!("analyze workers spawned n={n} (hot-scale supervisor active)");
        // Storage maintenance: cold TTL purge (shared PG; safe multi-node DELETE).
        // Only on analyze-capable processes so ingest pure nodes stay lean.
        spawn_cold_purge_loop(store.clone(), &worker_id, state.admin.clone());
        // Commercial retention: panel_policy-driven small-batch purge (analysis/session/velocity).
        spawn_retention_purge_loop(store.clone(), &worker_id, state.admin.clone());
    }

    // Config reload + node heartbeat (any process with admin-db)
    if let Some(ref hub) = state.admin {
        spawn_config_and_heartbeat(hub.clone(), state.clone());
    }

    if !role.needs_listen() {
        info!("analyze-only role: no Pingora listener (worker_id={worker_id})");
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    }

    // Side-channel listeners + optional full HTTP/3 app server
    start_side_listeners(&args, state.clone());

    let static_dir = if args.static_dir.is_dir() {
        args.static_dir.clone()
    } else {
        PathBuf::from("fe")
    };

    let opt = PingoraOpt::default();
    let mut server = Server::new(Some(opt)).expect("create pingora server");
    server.bootstrap();

    // SNI multi-cert map (P-V2)
    if !args.sni_map.is_empty() {
        match sni_map::load_sni_map(&args.sni_map) {
            Ok(n) => info!("SNI map ready n={n}"),
            Err(e) => warn!("sni-map load failed: {e}"),
        }
    }

    let raw_mode = upstream::ProxyMode::parse(&args.mode);
    let upstream_cfg = if raw_mode == upstream::ProxyMode::Transparent {
        match upstream::UpstreamConfig::parse(&args.upstream, args.upstream_tls, &args.upstream_sni)
        {
            Ok(u) => {
                info!(
                    "transparent mode upstream={} tls={} sni={}",
                    u.addr, u.tls, u.sni
                );
                Some(u)
            }
            Err(e) => {
                warn!("transparent mode upstream invalid ({e}); falling back to origin");
                None
            }
        }
    } else {
        None
    };
    // Lb mode wins over the legacy transparent/upstream pair (docs/08 paid LB).
    let mode = match raw_mode {
        upstream::ProxyMode::Lb => upstream::ProxyMode::Lb,
        upstream::ProxyMode::Transparent if upstream_cfg.is_some() => {
            upstream::ProxyMode::Transparent
        }
        _ => upstream::ProxyMode::Origin,
    };
    if mode == upstream::ProxyMode::Lb {
        match args.lb_pool.as_ref() {
            Some(pool) => info!("LB mode active (pool wired, enabled={})", pool.is_enabled()),
            None => warn!("LB mode requested without lb_pool — origin fallback"),
        }
    }

    let svc = GrService {
        state: state.clone(),
        static_dir: static_dir.clone(),
        mode,
        upstream: upstream_cfg,
        lb_pool: args.lb_pool.clone(),
    };
    let mut proxy = pingora_proxy::http_proxy_service(&server.configuration, svc);

    // PROXY protocol before TLS/HTTP accept (v4 probe pattern)
    if args.proxy_protocol {
        proxy
            .endpoints()
            .set_pre_tls_callback(listen::new_proxy_protocol_callback(true));
        info!("PROXY protocol PreTls enabled");
    }

    let sock_opt = if args.reuse_port {
        let mut o = pingora_core::listeners::TcpSocketOptions::default();
        o.so_reuseport = Some(true);
        info!("SO_REUSEPORT enabled on listeners");
        Some(o)
    } else {
        None
    };

    // Plain HTTP
    info!(
        "Pingora HTTP listen {} role={:?} mode={:?} worker_id={worker_id} reuse_port={}",
        args.bind, role, mode, args.reuse_port
    );
    if let Some(ref opt) = sock_opt {
        proxy.add_tcp_with_settings(&args.bind, opt.clone());
    } else {
        proxy.add_tcp(&args.bind);
    }

    // Admin console TCP listener removed (P1: single admin plane on the
    // control-port axum router). `admin_bind` is accepted but ignored so
    // legacy start scripts don't break; the port is no longer opened.
    if !args.admin_bind.trim().is_empty() {
        warn!(
            "admin_bind={} ignored — mx-console HTTP moved to the control plane (tasks/adminapi/04-plan.md P1)",
            args.admin_bind
        );
    }

    // Optional TLS with ClientHello JA4 + SNI map
    let tls_on = !args.bind_tls.is_empty()
        && !args.tls_cert.is_empty()
        && !args.tls_key.is_empty()
        && std::path::Path::new(&args.tls_cert).is_file()
        && std::path::Path::new(&args.tls_key).is_file();

    if tls_on {
        match build_tls_settings(&args.tls_cert, &args.tls_key, args.tls_h2) {
            Ok(tls) => {
                info!(
                    "Pingora HTTPS listen {} cert={} (ClientHello JA4 + SNI map n={})",
                    args.bind_tls,
                    args.tls_cert,
                    sni_map::sni_map_len()
                );
                proxy.add_tls_with_settings(&args.bind_tls, sock_opt.clone(), tls);
            }
            Err(e) => warn!("TLS settings failed: {e}; HTTPS disabled"),
        }
    } else if !args.bind_tls.is_empty() {
        warn!("bind_tls set but cert/key missing — HTTPS skipped");
    }

    let _ = args.analyze_debounce_ms;
    server.add_service(proxy);
    info!(
        "gr-probe-plane (Pingora) serving static={:?} soft_v2={} mode={:?}",
        static_dir, args.soft_v2_ready, mode
    );
    server.run_forever();
}

fn build_tls_settings(
    cert: &str,
    key: &str,
    h2: bool,
) -> std::result::Result<TlsSettings, Box<dyn std::error::Error>> {
    // v4 pattern: with_callbacks so handshake_complete can attach SslDigest.extension
    use pingora::listeners::TlsAcceptCallbacks;
    let mut tls = TlsSettings::with_callbacks(Box::new(tls_fp::GrTlsAccept) as TlsAcceptCallbacks)?;
    tls.set_private_key_file(key, openssl::ssl::SslFiletype::PEM)
        .map_err(|e| format!("tls key {key}: {e}"))?;
    tls.set_certificate_chain_file(cert)
        .map_err(|e| format!("tls cert {cert}: {e}"))?;
    if h2 {
        tls.enable_h2();
    }
    // ClientHello → pending map by SSL ptr (connection-scoped, not thread_local)
    install_client_hello_ja4(&mut tls);
    Ok(tls)
}

/// Capture ClientHello via OpenSSL; stash by SSL pointer for handshake_complete attach.
fn install_client_hello_ja4(tls: &mut TlsSettings) {
    use foreign_types::ForeignTypeRef;
    use openssl::ssl::{ClientHelloResponse, SslRef};

    tls.set_client_hello_callback(|ssl: &mut SslRef, _alert| {
        // P-V2 SNI multi-cert before fingerprint extract
        sni_map::apply_sni_certificate(ssl);
        if let Some(client_fp) = extract_fp(ssl) {
            let key = ssl.as_ptr() as usize;
            tls_fp::stash_pending(key, client_fp);
        }
        Ok(ClientHelloResponse::SUCCESS)
    });
}

/// Back-compat: prefer session digest (via inject_into_headers). Legacy take still works.
pub fn take_tls_fingerprints() -> (Option<String>, Option<String>, Option<String>) {
    (None, None, None)
}

fn extract_fp(ssl: &mut openssl::ssl::SslRef) -> Option<tls_fp::TlsClientFp> {
    use openssl::ssl::SslVersion;

    let ciphers = ssl.client_hello_ciphers().map(|b| b.to_vec()).unwrap_or_default();
    let legacy = ssl
        .client_hello_legacy_version()
        .map(|v| {
            if v == SslVersion::TLS1_3 {
                0x0304u16
            } else if v == SslVersion::TLS1_2 {
                0x0303
            } else if v == SslVersion::TLS1_1 {
                0x0302
            } else if v == SslVersion::TLS1 {
                0x0301
            } else {
                0x0303
            }
        })
        .unwrap_or(0x0303);

    let cipher_list = gr_probe_core::tls_ja4::parse_u16_list(&ciphers);
    let mut parts = gr_probe_core::ClientHelloParts {
        legacy_version: legacy,
        ciphers: cipher_list.clone(),
        ..Default::default()
    };

    // Full extension payloads for depth parse (iss/45 B3).
    let mut ext_payloads: Vec<(u16, Vec<u8>)> = Vec::new();
    if let Some(ids) = client_hello_extension_ids(ssl) {
        parts.extensions = ids.clone();
        for &ext_id in &ids {
            if let Some(data) = client_hello_ext_data(ssl, ext_id as u32) {
                match ext_id {
                    0 => parts.sni = gr_probe_core::tls_ja4::parse_sni(&data),
                    10 => parts.supported_groups = gr_probe_core::tls_ja4::parse_supported_groups(&data),
                    11 => parts.ec_point_formats = gr_probe_core::tls_ja4::parse_ec_point_formats(&data),
                    13 => {
                        parts.signature_algorithms =
                            gr_probe_core::tls_ja4::parse_signature_algorithms(&data)
                    }
                    16 => parts.alpn = gr_probe_core::tls_ja4::parse_alpn(&data),
                    43 => {
                        parts.supported_versions =
                            gr_probe_core::tls_ja4::parse_supported_versions(&data)
                    }
                    _ => {}
                }
                ext_payloads.push((ext_id, data));
            }
        }
    }
    let ext_order = parts
        .extensions
        .iter()
        .map(|x| x.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let cipher_order = cipher_list
        .iter()
        .filter(|&&c| !gr_probe_core::tls_ja4::is_grease_u16(c))
        .map(|x| x.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let fp = gr_probe_core::compute_fingerprints(&parts);
    let detail = gr_probe_net::parse_client_hello_ext_detail(&ext_payloads);
    Some(tls_fp::TlsClientFp {
        ja4: fp.ja4,
        ja3_hash: fp.ja3_hash,
        alpn: fp.alpn.first().cloned(),
        ja4_r: fp.ja4_r,
        tls_extensions_order: ext_order,
        cipher_suites_order: cipher_order,
        key_share_groups: detail
            .key_share_groups
            .iter()
            .map(|g| g.to_string())
            .collect::<Vec<_>>()
            .join(","),
        psk_modes: detail.psk_modes_names.join(","),
        has_ech: detail.has_encrypted_client_hello,
        has_alps: detail.has_application_settings,
        has_session_ticket: detail.has_session_ticket,
        has_early_data: detail.has_early_data,
        tls_ext_presence: detail.extension_presence.join(","),
    })
}

fn client_hello_extension_ids(ssl: &openssl::ssl::SslRef) -> Option<Vec<u16>> {
    use foreign_types::ForeignTypeRef;
    unsafe {
        let mut out: *mut libc::c_int = std::ptr::null_mut();
        let mut outlen: usize = 0;
        let rc = openssl_sys::SSL_client_hello_get1_extensions_present(
            ssl.as_ptr(),
            &mut out as *mut _ as *mut *mut libc::c_int,
            &mut outlen,
        );
        if rc != 1 || out.is_null() || outlen == 0 {
            return None;
        }
        let slice = std::slice::from_raw_parts(out, outlen);
        let ids: Vec<u16> = slice.iter().map(|&x| x as u16).collect();
        openssl_sys::OPENSSL_free(out as *mut _);
        Some(ids)
    }
}

fn client_hello_ext_data(ssl: &openssl::ssl::SslRef, type_: u32) -> Option<Vec<u8>> {
    use foreign_types::ForeignTypeRef;
    unsafe {
        let mut ptr: *const libc::c_uchar = std::ptr::null();
        let mut len: usize = 0;
        let rc = openssl_sys::SSL_client_hello_get0_ext(ssl.as_ptr(), type_, &mut ptr, &mut len);
        if rc != 1 || ptr.is_null() || len == 0 {
            return None;
        }
        Some(std::slice::from_raw_parts(ptr, len).to_vec())
    }
}


fn start_side_listeners(args: &Args, state: Arc<AppState>) {
    if args.syn_listen {
        let mut ports = Vec::new();
        if let Ok(a) = args.bind.parse::<std::net::SocketAddr>() {
            ports.push(a.port());
        }
        if !args.bind_tls.is_empty() {
            if let Ok(a) = args.bind_tls.parse::<std::net::SocketAddr>() {
                ports.push(a.port());
            }
        }
        if ports.is_empty() {
            ports.push(28765);
        }
        listen::spawn_syn_listener(ports);
    }

    let quic = args.quic_listen.clone();
    let h3 = args.h3_listen.clone();
    let webrtc = args.webrtc_listen.clone();
    if quic.is_empty() && h3.is_empty() && webrtc.is_empty() {
        return;
    }
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("gr-listen")
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            warn!("listen runtime failed: {e}");
            return;
        }
    };
    let _enter = rt.enter();

    // Full H3 app server (takes precedence over passive Initial listen on same addr)
    let mut h3_addr: Option<std::net::SocketAddr> = None;
    if !h3.is_empty() {
        match h3.parse::<std::net::SocketAddr>() {
            Ok(addr) => {
                if args.tls_cert.is_empty() || args.tls_key.is_empty() {
                    warn!("h3_listen set but --tls-cert/--tls-key missing");
                } else {
                    match listen::spawn_h3_server(
                        addr,
                        &args.tls_cert,
                        &args.tls_key,
                        state.clone(),
                    ) {
                        Ok(_h) => {
                            info!("HTTP/3 app server on {addr}");
                            h3_addr = Some(addr);
                            // Advertise Alt-Svc for TCP TLS clients
                            if gr_abi::env::get("H3_ALT_SVC").is_none() {
                                std::env::set_var(
                                    "GR_H3_ALT_SVC",
                                    format!("h3=\":{}\"; ma=86400", addr.port()),
                                );
                            }
                        }
                        Err(e) => warn!("h3_listen failed: {e}"),
                    }
                }
            }
            Err(e) => warn!("h3_listen invalid: {e}"),
        }
    }

    if !quic.is_empty() {
        match quic.parse::<std::net::SocketAddr>() {
            Ok(addr) => {
                if h3_addr == Some(addr) {
                    info!("quic_listen {addr} skipped (H3 app owns port; Initial still handled inside Quinn)");
                } else {
                    let _h = listen::spawn_quic_listener(addr);
                    info!("QUIC Initial passive listener on {addr}");
                }
            }
            Err(e) => warn!("quic_listen invalid: {e}"),
        }
    }
    if !webrtc.is_empty() {
        match webrtc.parse::<std::net::SocketAddr>() {
            Ok(addr) => {
                let _h = listen::spawn_webrtc_listener(addr);
                info!("WebRTC/STUN listener on {addr}");
            }
            Err(e) => warn!("webrtc_listen invalid: {e}"),
        }
    }
    std::mem::forget(rt);
}

