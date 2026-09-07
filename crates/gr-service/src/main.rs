//! gr-service — greenpng server process.
//!
//! - Control plane: random-path admin SPA, OTA, cluster, worker hot-config
//! - Probe plane: **in-tree** `gr-probe-plane` (source copied from v5, maintained only here)
//!
//! Does **not** path-depend on green-v5 or spawn external gr-service.

mod api;
mod cluster_ota_store;
mod lb_config_store;
mod mint_wire;
mod official_cloud;

use clap::Parser;
use gr_lb::{LbConfig, LbNode, LbPool};
use gr_probe_plane::run::{run_with, Args as ProbeArgs};
use gr_probe_store::ANALYZE_DEBOUNCE_MS;
use gr_runtime::{Runtime, RuntimeConfig};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::thread;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "gr-service",
    about = "greenpng server (in-tree probe + control plane)"
)]
struct Args {
    /// GR admin / control plane listen
    #[arg(long, default_value = "0.0.0.0:28680", env = "GR_BIND")]
    bind: String,

    /// Probe/ingest HTTP listen (Pingora) — lab nginx /g5 → this
    #[arg(long, default_value = "0.0.0.0:28765", env = "GR_PROBE_BIND")]
    probe_bind: String,

    /// Optional separate gateway worker (lab nginx /g5-gw → this). Empty = only probe_bind.
    #[arg(long, default_value = "", env = "GR_GATEWAY_BIND")]
    gateway_bind: String,

    /// Optional legacy probe admin console bind (empty = disabled; prefer control-plane random path)
    #[arg(long, default_value = "", env = "GR_PROBE_ADMIN_BIND")]
    probe_admin_bind: String,

    /// Probe/ingest HTTPS listen (Pingora TLS + ClientHello JA4 + SNI map) — the
    /// direct-bind pv/gv edge (panel-bound domains, NO reverse proxy).
    /// Default: 0.0.0.0:8443 when GR_TLS_CERT/GR_TLS_KEY are set; empty = plain HTTP only.
    #[arg(long, default_value = "", env = "GR_BIND_TLS")]
    bind_tls: String,

    /// Default TLS certificate PEM (SNI fallback for the HTTPS edge; per-domain
    /// panel-minted certs load from <data_dir>/sni-map.json)
    #[arg(long, default_value = "", env = "GR_TLS_CERT")]
    tls_cert: String,

    /// Default TLS private key PEM (SNI fallback)
    #[arg(long, default_value = "", env = "GR_TLS_KEY")]
    tls_key: String,

    /// Disable HTTP/2 on the TLS edge (kept on by default)
    #[arg(long, default_value_t = true, env = "GR_TLS_H2")]
    tls_h2: bool,

    #[arg(long, default_value = "data", env = "GR_DATA_DIR")]
    data_dir: PathBuf,

    #[arg(long, default_value = "modules", env = "GR_MODULES_DIR")]
    modules_dir: PathBuf,

    /// PostgreSQL DSN for probe store. Required (env: GR_DATABASE_URL).
    #[arg(long, default_value = "", env = "GR_DATABASE_URL")]
    database_url: String,

    /// Ignored. Probe store is PostgreSQL-only (`GR_DATABASE_URL`).
    #[arg(long, hide = true)]
    db: Option<PathBuf>,

    /// FE static dir (default: in-repo ./fe)
    #[arg(long, default_value = "", env = "GR_STATIC_DIR")]
    static_dir: PathBuf,

    #[arg(long, default_value = "all", env = "GR_ROLE")]
    role: String,

    #[arg(long, default_value_t = 1, env = "GR_ANALYZE_WORKERS")]
    analyze_workers: usize,

    #[arg(long, default_value = "", env = "GR_CLUSTER_KEY")]
    cluster_key: String,

    #[arg(long, default_value = "127.0.0.1:7900", env = "GR_ADVERTISE")]
    advertise: String,

    /// Optional LAN address (host:port) for LB internal-IP routing (docs/guides/08-LB-MODULE.md).
    #[arg(long, default_value = "", env = "GR_INTERNAL_ADDR")]
    internal_addr: String,

    #[arg(long, default_value = "", env = "GR_RELEASE_URL")]
    release_url: String,

    #[arg(long, default_value = "", env = "GR_PUBKEY_PATH")]
    pubkey_path: String,

    #[arg(long, default_value = "admin-spa", env = "GR_ADMIN_SPA")]
    admin_spa: PathBuf,

    /// Soft v2 ready (server gate)
    #[arg(long, default_value_t = false, env = "GR_SOFT_V2_READY")]
    soft_v2_ready: bool,

    /// CORS: lab default `*`; production defaults to `admin` (panel domains) unless set.
    #[arg(long, default_value = "", env = "GR_CORS_ORIGINS")]
    cors_origins: String,

    /// Disable embedding probe plane (admin-only mode)
    #[arg(long, default_value_t = false)]
    admin_only: bool,
}

fn resolve_static_dir(cli: &PathBuf) -> PathBuf {
    if !cli.as_os_str().is_empty() && cli.is_dir() {
        return cli.clone();
    }
    for c in [
        PathBuf::from("fe"),
        PathBuf::from("./fe"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../probe/fe"),
    ] {
        if c.is_dir() {
            return c;
        }
    }
    PathBuf::from("fe")
}

fn resolve_database_url(cli: &str) -> String {
    let t = cli.trim();
    if !t.is_empty() {
        return t.to_string();
    }
    gr_abi::env::get("DATABASE_URL").unwrap_or_default()
}

fn require_pg_companion_urls() -> anyhow::Result<()> {
    for (name, what) in [
        ("ADMIN_DATABASE_URL", "admin panel"),
        ("BIZ_DATABASE_URL", "biz dashboard"),
        ("ASSOCIATION_DATABASE_URL", "association"),
    ] {
        let v = gr_abi::env::get(name).unwrap_or_default();
        if v.trim().is_empty() {
            anyhow::bail!(
                "{} is required for {what} (PostgreSQL); SQLite was removed",
                gr_abi::env::resolved_name(name)
            );
        }
    }
    Ok(())
}

fn is_prod_deploy_env() -> bool {
    let env = gr_abi::env::get("DEPLOY_ENV").unwrap_or_default();
    let e = env.trim().to_ascii_lowercase();
    matches!(e.as_str(), "prod" | "production" | "live")
}

// 历史 mirror_one_env_alias/mirror_env_aliases 已随批次2 clean-break 改名删除:
// 旧实现把多个遗留前缀别名镜像到 clap 认的单一 env 名; 盲替换后三个前缀
// 全坍缩为 GR_ 同名 (恒等 set_var, 纯死代码)。env 解析统一走 gr_abi::env
// (GR_ 主名优先)。

fn map_env_aliases() {
    let prod = is_prod_deploy_env();
    if prod {
        // R-02 / supply-chain: require HTTPS release channel by default.
        if gr_abi::env::get("REQUIRE_MANIFEST_SIG").is_none() {
            std::env::set_var("GR_REQUIRE_MANIFEST_SIG", "1");
        }
        if gr_abi::env::get("REQUIRE_FE_SHA").is_none() {
            std::env::set_var("GR_REQUIRE_FE_SHA", "1");
        }
        if gr_abi::env::get("REQUIRE_RUNTIME_SHA").is_none() {
            std::env::set_var("GR_REQUIRE_RUNTIME_SHA", "1");
        }
        // Production hardening defaults (only fill when unset).
        if gr_abi::env::get("ALLOW_LAB_CHALLENGE").is_none() {
            std::env::set_var("GR_ALLOW_LAB_CHALLENGE", "0");
        }
        if gr_abi::env::get("REQUIRE_SEALED_INGEST").is_none() {
            std::env::set_var("GR_REQUIRE_SEALED_INGEST", "1");
            tracing::info!("prod default: GR_REQUIRE_SEALED_INGEST=1");
        }
        if gr_abi::env::get("REQUIRE_RESULT_TOKEN").is_none() {
            std::env::set_var("GR_REQUIRE_RESULT_TOKEN", "1");
            tracing::info!("prod default: GR_REQUIRE_RESULT_TOKEN=1");
        }
        // CORS: never leave lab-open * in prod unless explicitly forced.
        let cors = gr_abi::env::get("CORS_ORIGINS").unwrap_or_default();
        if cors.trim().is_empty() || cors.trim() == "*" {
            if gr_abi::env::get("ALLOW_OPEN_CORS")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false)
            {
                tracing::warn!("prod with open CORS forced by GR_ALLOW_OPEN_CORS=1 (legacy GR_ALLOW_OPEN_CORS)");
            } else {
                std::env::set_var("GR_CORS_ORIGINS", "admin");
                tracing::info!("prod default: CORS=admin (registered domains only; override with GR_CORS_ORIGINS)");
            }
        }
        if gr_abi::env::get("COOKIE_SECURE").is_none() {
            std::env::set_var("GR_COOKIE_SECURE", "1");
        }
    } else {
        // Lab defaults
        if gr_abi::env::get("ALLOW_LAB_CHALLENGE").is_none() {
            std::env::set_var("GR_ALLOW_LAB_CHALLENGE", "1");
        }
        if gr_abi::env::get("REQUIRE_SEALED_INGEST").is_none() {
            std::env::set_var("GR_REQUIRE_SEALED_INGEST", "0");
        }
        if gr_abi::env::get("REQUIRE_RESULT_TOKEN").is_none() {
            std::env::set_var("GR_REQUIRE_RESULT_TOKEN", "0");
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse()?))
        .try_init();
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).try_init();

    map_env_aliases();
    let args = Args::parse();
    let mut pubkey = Vec::new();
    if !args.pubkey_path.is_empty() {
        pubkey = std::fs::read(&args.pubkey_path)?;
    }
    // Direct-bind TLS edge (panel-bound pv/gv; Pingora TLS + JA4 + SNI map):
    // default 0.0.0.0:8443 whenever a default cert is provided and GR_BIND_TLS unset.
    let bind_tls_effective = if args.bind_tls.trim().is_empty() {
        if !args.tls_cert.trim().is_empty() && !args.tls_key.trim().is_empty() {
            "0.0.0.0:8443".to_string()
        } else {
            String::new()
        }
    } else {
        args.bind_tls.trim().to_string()
    };
    // R-07: production fail-closed for public binds unless explicitly allowed.
    if is_prod_deploy_env() {
        let allow_public = gr_abi::env::get("ALLOW_PUBLIC_BIND")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let binds = [
            &args.bind,
            &args.probe_bind,
            &args.gateway_bind,
            &bind_tls_effective,
        ];
        for b in binds {
            let b = b.trim();
            if b.is_empty() {
                continue;
            }
            if (b.starts_with("0.0.0.0:") || b.starts_with("[::]:")) && !allow_public {
                anyhow::bail!(
                    "production refuses public bind {b}; use reverse-proxy / private bind or set GR_ALLOW_PUBLIC_BIND=1 (legacy GR_ accepted)"
                );
            }
        }
        if args.release_url.starts_with("http://") {
            let allow_http = gr_abi::env::get("ALLOW_HTTP_RELEASE")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            if !allow_http {
                anyhow::bail!("production refuses http:// GR_RELEASE_URL (use https or GR_ALLOW_HTTP_RELEASE=1 for lab; legacy GR_ accepted)");
            }
        }
        gr_runtime::cluster_key_acceptable(&args.cluster_key, true)
            .map_err(|e| anyhow::anyhow!(e))?;
        let tok = gr_abi::env::get("RESULT_TOKEN").unwrap_or_default();
        let site_map = gr_abi::env::get("SITE_RESULT_TOKENS").unwrap_or_default();
        if tok.trim().is_empty() && site_map.trim().is_empty() {
            anyhow::bail!("production requires GR_RESULT_TOKEN (or GR_SITE_RESULT_TOKENS)");
        }
        if pubkey.is_empty() {
            anyhow::bail!("production requires OTA pubkey (refusing unsigned module/FE/runtime updates)");
        }
    }

    std::fs::create_dir_all(&args.data_dir)?;
    let _ = &args.db;
    let static_dir = resolve_static_dir(&args.static_dir);
    let database_url = resolve_database_url(&args.database_url);
    if database_url.trim().is_empty() {
        anyhow::bail!("GR_DATABASE_URL is required (PostgreSQL); SQLite probe store was removed");
    }
    if gr_abi::env::get("DATABASE_URL").is_none() {
        std::env::set_var("GR_DATABASE_URL", &database_url);
    }
    require_pg_companion_urls()?;
    // Apply CLI CORS after env defaults (empty CLI → env / prod admin / lab *)
    let cors_final = {
        let c = args.cors_origins.trim();
        if !c.is_empty() {
            c.to_string()
        } else {
            gr_abi::env::get("CORS_ORIGINS")
                .unwrap_or_else(|| {
                    if is_prod_deploy_env() {
                        "admin".into()
                    } else {
                        "*".into()
                    }
                })
        }
    };
    std::env::set_var("GR_CORS_ORIGINS", &cors_final);
    // P1-5: remember whether signed-OTA boot verification applies (pubkey
    // moves into `cfg` below).
    let has_ota_pubkey = !pubkey.is_empty();

    let cfg = RuntimeConfig {
        data_dir: args.data_dir.clone(),
        modules_dir: args.modules_dir.clone(),
        bind: args.bind.clone(),
        role: args.role.clone(),
        cluster_key: args.cluster_key.clone(),
        advertise: args.advertise.clone(),
        release_base_url: args.release_url.clone(),
        pubkey,
    };
    let rt = Runtime::boot(cfg).map_err(|e| anyhow::anyhow!(e))?;
    // LB (docs/guides/08-LB-MODULE.md): publish this node's LAN address for internal-IP mode.
    if !args.internal_addr.trim().is_empty() {
        let ia = args.internal_addr.trim().to_string();
        rt.cluster.update_self(|n| n.internal_addr = Some(ia));
    }
    seed_builtin_modules(&rt);
    // Shared multi-node LB pool (docs/guides/08-LB-MODULE.md) — ungated, runs on every full node.
    let lb_pool = Arc::new(LbPool::new());
    if !args.admin_only {
        let rt_handle = tokio::runtime::Handle::current();
        spawn_lb_control_loop(rt.clone(), lb_pool.clone(), rt_handle);
    }
    // Multi-node discovery (docs/guides/08-LB-MODULE.md): post signed heartbeats to configured
    // peer control planes every 5s so every node's ClusterHub snapshot (and
    // the LB pool) converges on the same healthy set.
    if !args.admin_only {
        spawn_cluster_peer_sync(rt.clone());
    }
    // P1-5: verify active modules (chain sigs + sha256) against the persisted
    // release binding before serving. Fails boot only when
    // GR_STRICT_BOOT_VERIFY=1 (legacy GR_ accepted) or a signed module fails verification.
    if has_ota_pubkey {
        match rt.ota_verify_active_modules() {
            Ok(v) => tracing::info!(
                verified = ?v.get("verified").and_then(|x| x.as_array()).map(|a| a.len()).unwrap_or(0),
                legacy = ?v.get("legacy_unverifiable").and_then(|x| x.as_array()).map(|a| a.len()).unwrap_or(0),
                "boot active-module integrity check"
            ),
            Err(e) => {
                if is_prod_deploy_env() || gr_abi::env::get("STRICT_BOOT_VERIFY")
                    .map(|x| x == "1" || x.eq_ignore_ascii_case("true"))
                    .unwrap_or(false)
                {
                    anyhow::bail!("boot active-module integrity check failed: {e}");
                }
                tracing::warn!(error = %e, "boot active-module integrity check failed (non-fatal in lab)");
            }
        }
    }
    // Route commercial mint through analyze.so when hot-loaded (OTA path).
    mint_wire::wire_analyze_mint(&rt);
    let _ = rt.set_workers(args.analyze_workers as u32, 1, 1);
    {
        let rt_ota = rt.clone();
        thread::Builder::new()
            .name("gr-cluster-ota".into())
            .spawn(move || loop {
                thread::sleep(std::time::Duration::from_secs(20));
                let Ok(Some(d)) = cluster_ota_store::read_desired() else {
                    continue;
                };
                let av = rt_ota
                    .registry
                    .get("analyze")
                    .map(|m| m.version().to_string());
                if !gr_runtime::cluster_ota::should_apply_cluster_ota(
                    &rt_ota.get_release_base_url(),
                    av.as_deref(),
                    &d,
                ) {
                    continue;
                }
                if let Err(e) = rt_ota.set_release_base_url(&d.release_url) {
                    tracing::warn!(error = %e, "cluster ota set-release-url failed");
                    continue;
                }
                match rt_ota.ota_install_remote("analyze", Some(&d.version), d.activate) {
                    Ok(_) => {
                        mint_wire::wire_analyze_mint(&rt_ota);
                        tracing::info!(version = %d.version, "cluster ota applied GitHub analyze");
                    }
                    Err(e) => tracing::warn!(error = %e, "cluster ota install analyze failed"),
                }
            })
            .ok();
    }

    // --- In-tree probe/analyze plane (blocking Pingora) on dedicated thread(s) ---
    if !args.admin_only {
        let db = args.data_dir.join("probe_pg_unused");
        // Probe-plane admin DSN is GR_ADMIN_DATABASE_URL.
        // This path is only the cert/data directory parent (not a SQLite file).
        let admin_db = args.data_dir.join("probe_admin").join("admin.pg");
        let soft_dir = args.data_dir.join("soft_store");
        let r100 = {
            let candidates = [
                PathBuf::from("data/r100_templates.json"),
                args.data_dir.join("r100_templates.json"),
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../../data/r100_templates.json"),
            ];
            candidates
                .into_iter()
                .find(|p| p.is_file())
                .unwrap_or_else(|| PathBuf::from("data/r100_templates.json"))
        };
        let soft = args.soft_v2_ready;
        let cors = cors_final.clone();
        let static_dir_probe = static_dir.clone();
        let database_url_c = database_url.clone();
        let redis = gr_abi::env::get("REDIS_URL").unwrap_or_default();
        let n_workers = args.analyze_workers;
        let pid = std::process::id();
        // worker_id 需要跨节点唯一 (共享 PG 心跳 map 按 worker_id 键控):
        // 单节点无 GR_NODE_ID → gr-probe-<pid>;
        // 多节点设置 GR_NODE_ID → gr-probe-<pid>-<node_id>, 避免同 PID 容器互相覆盖。
        let node_tag = gr_abi::env::get("NODE_ID").unwrap_or_default();
        let worker_id = if node_tag.is_empty() {
            format!("{pid}")
        } else {
            format!("{pid}-{node_tag}")
        };
        let worker_id_probe = format!("gr-probe-{worker_id}");
        let worker_id_gw = format!("gr-gw-{worker_id}");

        // Direct-bind TLS edge: panel-minted per-domain certs live in
        // <data_dir>/sni-map.json (admin ssl.rs / apply.rs sync it); the
        // default cert (SNI fallback) comes from GR_TLS_CERT/GR_TLS_KEY.
        let sni_map_path = args
            .data_dir
            .join("sni-map.json")
            .to_string_lossy()
            .to_string();
        let bind_tls_edge = bind_tls_effective.clone();
        let tls_cert_edge = args.tls_cert.clone();
        let tls_key_edge = args.tls_key.clone();
        let tls_h2_edge = args.tls_h2;
        let sni_map_path_gw = sni_map_path.clone();

        // Primary: role from CLI (default all) on probe_bind
        {
            let bind = args.probe_bind.clone();
            let role = args.role.clone();
            let db = db.clone();
            let admin_db = admin_db.clone();
            let soft_dir = soft_dir.clone();
            let r100 = r100.clone();
            let static_dir_probe = static_dir_probe.clone();
            let database_url_c = database_url_c.clone();
            let cors = cors.clone();
            let redis = redis.clone();
            let probe_admin = args.probe_admin_bind.clone();
            let lb_pool_plane = lb_pool.clone();
            thread::Builder::new()
                .name("gr-probe-plane".into())
                .spawn(move || {
                    log::info!("starting in-tree probe plane role={role} bind={bind}");
                    run_with(make_probe_args(
                        bind,
                        role,
                        db,
                        database_url_c,
                        soft,
                        cors,
                        static_dir_probe,
                        n_workers,
                        worker_id_probe,
                        soft_dir,
                        redis,
                        r100,
                        probe_admin,
                        admin_db,
                        lb_pool_plane,
                        bind_tls_edge,
                        tls_cert_edge,
                        tls_key_edge,
                        tls_h2_edge,
                        sni_map_path,
                    ));
                })
                .map_err(|e| anyhow::anyhow!("spawn probe plane: {e}"))?;
        }

        // Optional gateway-only worker
        if !args.gateway_bind.trim().is_empty() {
            let bind = args.gateway_bind.clone();
            let db = db.clone();
            let admin_db = admin_db.clone();
            let soft_dir = soft_dir.clone();
            let r100 = r100.clone();
            let static_dir_probe = static_dir_probe.clone();
            let database_url_c = database_url_c.clone();
            let cors = cors.clone();
            let redis = redis.clone();
            let lb_pool_gw = lb_pool.clone();
            thread::Builder::new()
                .name("gr-gateway-plane".into())
                .spawn(move || {
                    log::info!("starting in-tree gateway plane bind={bind}");
                    run_with(make_probe_args(
                        bind,
                        "gateway".into(),
                        db,
                        database_url_c,
                        soft,
                        cors,
                        static_dir_probe,
                        1,
                        worker_id_gw,
                        soft_dir,
                        redis,
                        r100,
                        String::new(),
                        admin_db,
                        lb_pool_gw,
                        // gateway-only worker stays plain HTTP (optional
                        // internal hop); the direct edge TLS lives on the
                        // primary probe plane listener above.
                        String::new(),
                        String::new(),
                        String::new(),
                        true,
                        sni_map_path_gw,
                    ));
                })
                .map_err(|e| anyhow::anyhow!("spawn gateway plane: {e}"))?;
        }

        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    }

    // 2026-09 product decision: official-site license sync retired — every
    // install is full-featured without a signed token, so the periodic
    // entitlement refresh loop is no longer spawned.
    // crate::official_cloud::spawn_license_sync_loop();

    let console = rt.admin.auth.console_path.clone();
    let install_root = resolve_install_root(&args.data_dir);
    let app = api::router(
        rt.clone(),
        args.admin_spa,
        args.probe_bind.clone(),
        static_dir.clone(),
        install_root.clone(),
        lb_pool,
    );
    let addr: SocketAddr = args.bind.parse()?;
    tracing::info!(
        %addr,
        probe = %args.probe_bind,
        console = %format!("/{console}/"),
        static_dir = %static_dir.display(),
        install_root = %install_root.display(),
        release_url = %rt.get_release_base_url(),
        "gr-service: independent process (control plane + in-tree probe plane)"
    );
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;
    Ok(())
}

/// Directory that `ota/install-runtime` writes `bin/gr-service` into.
/// Prefer GR_INSTALL_ROOT (legacy GR_INSTALL_ROOT), then `<exe>/..` when running from `…/bin/gr-service`
/// (docker `/app/bin`, host after panel OTA). Do not treat container `/data`
/// as meaning install root `/` — that used to drop the signed binary in `/bin`
/// while the process kept exec'ing `/app/bin/gr-service`.
fn resolve_install_root(data_dir: &std::path::Path) -> PathBuf {
    if let Some(v) = gr_abi::env::get("INSTALL_ROOT") {
        let p = PathBuf::from(v.trim());
        if !p.as_os_str().is_empty() {
            return p;
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(bin_dir) = exe.parent() {
            if bin_dir.file_name().is_some_and(|s| s == "bin") {
                if let Some(root) = bin_dir.parent() {
                    if root != std::path::Path::new("/") {
                        return root.to_path_buf();
                    }
                }
            }
        }
    }
    let data = data_dir
        .canonicalize()
        .unwrap_or_else(|_| data_dir.to_path_buf());
    if data.file_name().is_some_and(|s| s == "data") {
        if let Some(parent) = data.parent() {
            if parent != std::path::Path::new("/") && looks_like_install_root(parent) {
                return parent.to_path_buf();
            }
        }
    }
    if PathBuf::from("/opt/greenpng").is_dir() {
        return PathBuf::from("/opt/greenpng");
    }
    if PathBuf::from("/app/bin/gr-service").is_file() {
        return PathBuf::from("/app");
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn looks_like_install_root(p: &std::path::Path) -> bool {
    p.join("bin").is_dir()
        || p.join("fe").is_dir()
        || p.join("VERSION").is_file()
        || p.join("admin-spa").is_dir()
}

fn make_probe_args(
    bind: String,
    role: String,
    db: PathBuf,
    database_url: String,
    soft_v2_ready: bool,
    cors_origins: String,
    static_dir: PathBuf,
    analyze_workers: usize,
    worker_id: String,
    soft_store_dir: PathBuf,
    redis_url: String,
    r100_templates: PathBuf,
    admin_bind: String,
    admin_db: PathBuf,
    lb_pool: Arc<LbPool>,
    bind_tls: String,
    tls_cert: String,
    tls_key: String,
    tls_h2: bool,
    sni_map: String,
) -> ProbeArgs {
    ProbeArgs {
        bind,
        bind_tls,
        db,
        database_url,
        soft_v2_ready,
        challenge_secret: String::new(),
        cors_origins,
        result_token: String::new(),
        static_dir,
        role,
        analyze_workers,
        worker_id,
        analyze_debounce_ms: ANALYZE_DEBOUNCE_MS,
        soft_store_dir,
        soft_store_backend: "store".into(),
        redis_url,
        r100_templates,
        tls_cert,
        tls_key,
        tls_h2,
        quic_listen: String::new(),
        h3_listen: String::new(),
        webrtc_listen: String::new(),
        syn_listen: false,
        proxy_protocol: false,
        reuse_port: false,
        sni_map,
        // LB gateway mode is opt-in per node: GR_PROBE_MODE=lb (legacy GR_) turns this
        // plane into the multi-node load balancer (docs/guides/08-LB-MODULE.md).
        mode: gr_abi::env::get("PROBE_MODE").unwrap_or_else(|| "origin".into()),
        upstream: String::new(),
        upstream_tls: false,
        upstream_sni: String::new(),
        admin_bind,
        admin_db,
        lb_pool: Some(lb_pool),
    }
}

/// LB control loop (docs/guides/08-LB-MODULE.md): every 5s reconcile
/// 1. persisted config from shared admin PG (panel PUT applies cluster-wide)
/// 2. cluster snapshot → pool nodes (self excluded) + trusted-peer list for XFF
/// 3. active health probe task (spawned once when enabled in config)
/// Post signed heartbeats to peer control planes (`GR_CLUSTER_PEERS`, legacy `GR_`,
/// comma-separated `host:port`). Receivers verify key + HMAC + clock window
/// before ingesting; the LB control loop consumes the resulting snapshot.
fn spawn_cluster_peer_sync(rt: Arc<Runtime>) {
    let peers: Vec<String> = gr_abi::env::get("CLUSTER_PEERS")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if peers.is_empty() {
        tracing::info!("cluster peer sync disabled (GR_CLUSTER_PEERS empty; legacy GR_ accepted) — single-node mode");
        return;
    }
    thread::Builder::new()
        .name("gr-cluster-peer-sync".into())
        .spawn(move || {
            tracing::info!(?peers, "cluster peer sync started");
            loop {
                thread::sleep(std::time::Duration::from_secs(5));
                let (info, ts_ms, sig) = rt.cluster.sign_heartbeat();
                let key = rt.cfg.cluster_key.clone();
                let body = serde_json::json!({
                    "key": key,
                    "info": info,
                    "ts_ms": ts_ms,
                    "sig": sig,
                });
                for peer in &peers {
                    let url = format!("http://{peer}/v1/cluster/heartbeat");
                    // Best-effort; failures are logged once, no retry storm.
                    match gr_service_client_post(&url, &body) {
                        Ok(200) => {}
                        Ok(code) => tracing::debug!(url = %url, code, "cluster heartbeat rejected"),
                        Err(e) => tracing::debug!(url = %url, error = %e, "cluster heartbeat post failed"),
                    }
                }
            }
        })
        .ok();
}

fn gr_service_client_post(url: &str, body: &serde_json::Value) -> Result<u16, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(4))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(url)
        .json(body)
        .send()
        .map_err(|e| e.to_string())?;
    Ok(resp.status().as_u16())
}

fn spawn_lb_control_loop(
    rt: Arc<Runtime>,
    pool: Arc<LbPool>,
    rt_handle: tokio::runtime::Handle,
) {
    static PROBE_SPAWNED: OnceLock<()> = OnceLock::new();
    thread::Builder::new()
        .name("gr-lb-control".into())
        .spawn(move || {
            tracing::info!("lb control loop started (multi-node LB, docs/08)");
            let mut last_applied: Option<LbConfig> = None;
            loop {
                thread::sleep(std::time::Duration::from_secs(5));
                // 1) Persisted panel config (shared PG).
                let persisted = lb_config_store::read_lb_config()
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                let mut applied = persisted.clone();
                if Some(&applied) != last_applied.as_ref() {
                    if let Err(e) = applied.validate() {
                        tracing::warn!(error = %e, "lb config invalid — keeping defaults");
                        applied = LbConfig::default();
                        if last_applied.is_some() {
                            applied.enabled = false;
                        }
                    }
                    pool.set_config(applied.clone());
                    tracing::info!(
                        enabled = applied.enabled,
                        mode = %applied.mode,
                        strategy = %applied.strategy,
                        "lb pool config applied"
                    );
                    last_applied = Some(applied);
                }
                // 3) Cluster nodes → pool + trust list (XFF passthrough chain).
                let self_id = rt.cluster.node_id().to_string();
                let snap = rt.cluster.snapshot();
                let mut nodes: Vec<LbNode> = Vec::new();
                let mut trust: Vec<String> = Vec::new();
                for n in snap {
                    if n.node_id == self_id {
                        continue;
                    }
                    nodes.push(LbNode {
                        node_id: n.node_id.clone(),
                        advertise: n.advertise.clone(),
                        internal_addr: n.internal_addr.clone(),
                        load_inflight: n.load.inflight,
                        degraded: n.degraded,
                    });
                    trust.push(n.advertise.clone());
                    if let Some(ia) = n.internal_addr.as_ref() {
                        trust.push(ia.clone());
                    }
                }
                // Lab/dev static peers: GR_LB_STATIC_NODES="echo=127.0.0.1:8099,..." (legacy GR_)
                // (echo/staging servers outside the cluster; merged, not exclusive).
                if let Some(static_csv) = gr_abi::env::get("LB_STATIC_NODES") {
                    for part in static_csv.split(',') {
                        let part = part.trim();
                        if part.is_empty() || !part.contains('=') {
                            continue;
                        }
                        let mut it = part.splitn(2, '=');
                        let sid = it.next().unwrap().trim().to_string();
                        let addr = it.next().unwrap().trim().to_string();
                        if sid.is_empty() || addr.is_empty() {
                            continue;
                        }
                        let (internal, advertise) = if let Some(rest) = addr.split_once('@') {
                            (Some(rest.0.to_string()), rest.1.to_string())
                        } else {
                            (None, addr.clone())
                        };
                        if nodes.iter().any(|n| n.node_id == sid) {
                            continue;
                        }
                        nodes.push(LbNode {
                            node_id: sid,
                            advertise,
                            internal_addr: internal,
                            load_inflight: 0,
                            degraded: false,
                        });
                        trust.push(addr.clone());
                    }
                }
                pool.set_nodes(nodes);
                gr_probe_core::client_ip::set_cluster_trusted_ips(trust);
                // 4) Active probe task (spawned on the main tokio runtime via
                // the captured handle — this thread itself is not inside one).
                if pool.config().active_health_check {
                    let _ = PROBE_SPAWNED.get_or_init(|| {
                        rt_handle.spawn(gr_lb::health::probe_loop(pool.clone()));
                        tracing::info!("lb active health probe scheduled on main runtime");
                    });
                }
            }
        })
        .ok();
}

fn seed_builtin_modules(rt: &Arc<Runtime>) {
    // Disk OTA analyze is loaded in Runtime::boot before this. Recording builtin
    // analyze as active=true would UPDATE all analyze rows to inactive and make
    // the panel show CARGO_PKG_VERSION builtin after every restart.
    let ota_analyze_loaded = rt.registry.get("analyze").is_some();
    for (name, domain) in [
        ("identity", "identity"),
        ("brain", "brain"),
        ("analyze", "analyze"),
        ("ingest", "ingest"),
        ("edge", "edge"),
        ("probe_assets", "probe_assets"),
        ("probe_plane", "probe_core"),
    ] {
        let active = !(name == "analyze" && ota_analyze_loaded);
        let _ = rt.admin.db.record_module(
            name,
            env!("CARGO_PKG_VERSION"),
            if name == "probe_plane" {
                "in_tree_gr_probe_plane"
            } else {
                "builtin"
            },
            active,
            &serde_json::json!({
                "domain": domain,
                "mode": if name == "probe_plane" { "in_tree" } else { "static_link" }
            }),
        );
    }
    if let Some(m) = rt.registry.get("analyze") {
        let _ = rt.admin.db.set_module_active("analyze", m.version());
    }
    let _ = rt.refresh_live_config();
}
