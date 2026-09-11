//! Runtime: load business modules (.so), hot-swap without process restart, worker atomics.

pub mod cluster_ota;
pub mod loader;
pub mod registry;
pub mod workers;

pub use loader::{LoadedModule, ModuleLoader};
pub use registry::ModuleRegistry;
pub use workers::WorkerPoolConfig;

use arc_swap::ArcSwap;
use gr_abi::RUNTIME_ABI;
use gr_admin::AdminHub;
use gr_cluster::ClusterHub;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub const PRODUCT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub data_dir: PathBuf,
    pub modules_dir: PathBuf,
    pub bind: String,
    pub role: String,
    pub cluster_key: String,
    pub advertise: String,
    pub release_base_url: String,
    pub pubkey: Vec<u8>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("data"),
            modules_dir: PathBuf::from("modules"),
            bind: "0.0.0.0:28680".into(),
            role: "all".into(),
            cluster_key: String::new(),
            advertise: "127.0.0.1:7900".into(),
            release_base_url: String::new(),
            pubkey: Vec::new(),
        }
    }
}

pub struct Runtime {
    pub cfg: RuntimeConfig,
    /// Hot-swappable release base URL for panel OTA (modules + FE + runtime pull).
    /// Survives without process restart so admin can point at a new GitHub tag.
    pub release_base_url: Arc<ArcSwap<String>>,
    pub registry: Arc<ModuleRegistry>,
    pub admin: Arc<AdminHub>,
    pub cluster: ClusterHub,
    pub analyze_workers: Arc<AtomicUsize>,
    pub ingest_workers: Arc<AtomicUsize>,
    pub gateway_workers: Arc<AtomicUsize>,
    pub config_version: Arc<AtomicUsize>,
    /// Shared JSON snapshot for modules (sites, crypto, …).
    pub live_config: Arc<ArcSwap<String>>,
}

impl Runtime {
    pub fn boot(cfg: RuntimeConfig) -> Result<Arc<Self>, String> {
        std::fs::create_dir_all(&cfg.data_dir).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&cfg.modules_dir).map_err(|e| e.to_string())?;
        let admin = AdminHub::open(&cfg.data_dir.join("admin"))?;
        let (aw, iw, gw) = admin.db.get_workers()?;
        let node_id = gr_abi::env::get("NODE_ID").unwrap_or_else(|| {
            format!("n-{}", &uuid_simple()[..12])
        });
        let prod = gr_abi::env::get("DEPLOY_ENV")
             .or_else(|| gr_abi::env::get("DEPLOY_ENV"))
            .map(|e| {
                let e = e.trim().to_ascii_lowercase();
                matches!(e.as_str(), "prod" | "production" | "live")
            })
            .unwrap_or(false);
        cluster_key_acceptable(&cfg.cluster_key, prod)?;
        let cluster = ClusterHub::new(
            node_id,
            if cfg.cluster_key.is_empty() {
                "dev-cluster"
            } else {
                &cfg.cluster_key
            },
            cfg.advertise.clone(),
            PRODUCT_VERSION.to_string(),
        );
        let registry = Arc::new(ModuleRegistry::new(PRODUCT_VERSION, RUNTIME_ABI));
        // Prefer panel-persisted URL (hot set survives restart); else CLI / env.
        let mut release_url = cfg.release_base_url.clone();
        let persist_path = cfg.data_dir.join("ota_release_url.txt");
        if let Ok(s) = std::fs::read_to_string(&persist_path) {
            let t = s.trim().trim_end_matches('/').to_string();
            if !t.is_empty() && (t.starts_with("https://") || t.starts_with("http://")) && release_url_allowed(&t).is_ok() {
                release_url = t;
            }
        }
        let rt = Arc::new(Self {
            cfg,
            release_base_url: Arc::new(ArcSwap::from_pointee(release_url)),
            registry,
            admin,
            cluster,
            analyze_workers: Arc::new(AtomicUsize::new(aw as usize)),
            ingest_workers: Arc::new(AtomicUsize::new(iw as usize)),
            gateway_workers: Arc::new(AtomicUsize::new(gw as usize)),
            config_version: Arc::new(AtomicUsize::new(1)),
            live_config: Arc::new(ArcSwap::from_pointee("{}".to_string())),
        });
        rt.refresh_live_config()?;
        // Load any active modules already on disk
        let _ = rt.registry.load_active_tree(&rt.cfg.modules_dir);
        // P0 (178 panel OTA, 1.0.8): the 1.0.7 binary's `install-runtime`
        // swaps only `bin/gr-service` — its code cannot know about data_tree.
        // Its bundle fetch leaves the verified 1.0.8 bundle staged under
        // `data/ota_staging/bundle/`; on the new binary's first boot we
        // overlay missing/changed product data files from it so a panel OTA
        // upgrade lands r100 templates + geoip without host-side scripts.
        if !rt.cfg.pubkey.is_empty() {
            let staging = rt.cfg.data_dir.join("ota_staging").join("bundle");
            match bootstrap_data_tree_from_staging(&staging, &rt.cfg.pubkey, &rt.cfg.data_dir) {
                Ok(v) if !v.is_empty() => tracing::info!(
                    files = ?v,
                    "boot data bootstrap: product data files installed from staged release bundle"
                ),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "boot data bootstrap skipped"),
            }
        }
        Ok(rt)
    }

    /// Current OTA release base URL (hot-readable).
    pub fn get_release_base_url(&self) -> String {
        self.release_base_url.load_full().as_ref().clone()
    }

    /// Hot-set OTA release base URL (no process restart). Optionally persist to data_dir.
    pub fn set_release_base_url(&self, url: &str) -> Result<(), String> {
        let u = url.trim().trim_end_matches('/').to_string();
        if u.is_empty() {
            return Err("release_base_url empty".into());
        }
        if !(u.starts_with("https://") || u.starts_with("http://")) {
            return Err("release_base_url must be http(s)".into());
        }
        // R-02: production forbids plain HTTP release URLs (supply-chain MITM).
        let prod = gr_abi::env::get("DEPLOY_ENV")
            .map(|e| {
                let e = e.trim().to_ascii_lowercase();
                matches!(e.as_str(), "prod" | "production" | "live")
            })
            .unwrap_or(false);
        let allow_http = gr_abi::env::get("ALLOW_HTTP_RELEASE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if prod && u.starts_with("http://") && !allow_http {
            return Err(
                "production forbids http:// release URL (set GR_ALLOW_HTTP_RELEASE=1; legacy GR_ALLOW_HTTP_RELEASE) only for lab"
                    .into(),
            );
        }
        release_url_allowed(&u)?;
        self.release_base_url.store(Arc::new(u.clone()));
        // Persist for next boot (best-effort)
        let path = self.cfg.data_dir.join("ota_release_url.txt");
        let _ = std::fs::write(&path, format!("{u}\n"));
        // Also patch INSTALL .env if present (common deploy layout)
        for env_path in [
            self.cfg.data_dir.join("../.env"),
            PathBuf::from("/opt/greenpng/.env"),
        ] {
            if env_path.is_file() {
                if let Ok(txt) = std::fs::read_to_string(&env_path) {
                    let mut has_gr = false;
                    let mut lines: Vec<String> = txt
                        .lines()
                        .map(|l| {
                            if l.starts_with("GR_RELEASE_URL=") {
                                has_gr = true;
                                format!("GR_RELEASE_URL={u}")
                            } else {
                                l.to_string()
                            }
                        })
                        .collect();
                    if !has_gr {
                        lines.push(format!("GR_RELEASE_URL={u}"));
                    }
                    let new_txt = format!("{}\n", lines.join("\n"));
                    let _ = std::fs::write(&env_path, new_txt);
                }
            }
        }
        Ok(())
    }

    pub fn refresh_live_config(&self) -> Result<(), String> {
        let sites = self.admin.db.list_sites()?;
        let (aw, iw, gw) = self.admin.db.get_workers()?;
        let body = serde_json::json!({
            "config_version": self.config_version.load(Ordering::Relaxed),
            "product_version": PRODUCT_VERSION,
            "abi": RUNTIME_ABI,
            "workers": {
                "analyze": aw,
                "ingest": iw,
                "gateway": gw,
            },
            "sites": sites,
        });
        let json = body.to_string();
        self.live_config.store(Arc::new(json.clone()));
        // Push to loaded .so modules without restart
        self.registry.apply_config_all(json.as_str());
        // Static-linked business modules (dev / default service binary)
        let _ = gr_module_ingest::apply_config_json(json.as_str());
        let _ = gr_module_identity::apply_config_json(json.as_str());
        let _ = gr_module_probe_assets::apply_config_json(json.as_str());
        let _ = gr_module_analyze::apply_config_json(json.as_str());
        Ok(())
    }

    /// Panel changes worker counts — hot apply.
    pub fn set_workers(&self, analyze: u32, ingest: u32, gateway: u32) -> Result<(), String> {
        self.admin.db.set_workers(analyze, ingest, gateway)?;
        self.analyze_workers
            .store(analyze.max(1) as usize, Ordering::Relaxed);
        self.ingest_workers
            .store(ingest.max(1) as usize, Ordering::Relaxed);
        self.gateway_workers
            .store(gateway.max(1) as usize, Ordering::Relaxed);
        self.config_version.fetch_add(1, Ordering::Relaxed);
        self.refresh_live_config()?;
        self.cluster.update_self(|n| {
            n.load.analyze_workers = analyze.max(1);
        });
        Ok(())
    }

    pub fn status_json(&self) -> serde_json::Value {
        let modules = self.registry.list_status();
        let nodes = self.cluster.snapshot();
        serde_json::json!({
            "product": "greenpng",
            "version": PRODUCT_VERSION,
            // P0-1: per-release build id — ops can confirm the running binary
            // matches the manifest's build_id (rotation evidence).
            "build_id": env!("GR_BUILD_ID"),
            "abi": RUNTIME_ABI,
            "role": self.cfg.role,
            "bind": self.cfg.bind,
            "config_version": self.config_version.load(Ordering::Relaxed),
            "workers": {
                "analyze": self.analyze_workers.load(Ordering::Relaxed),
                "ingest": self.ingest_workers.load(Ordering::Relaxed),
                "gateway": self.gateway_workers.load(Ordering::Relaxed),
            },
            "modules": modules,
            "cluster": nodes,
            "console_path": format!("/{}/", self.admin.auth.console_path),
            "release_base_url": self.get_release_base_url(),
            "ota_pubkey_configured": !self.cfg.pubkey.is_empty(),
        })
    }

    pub fn ota_engine(&self) -> gr_ota::OtaEngine {
        gr_ota::OtaEngine::new(gr_ota::OtaConfig {
            release_base_url: self.get_release_base_url(),
            modules_dir: self.cfg.modules_dir.clone(),
            pubkey_bytes: self.cfg.pubkey.clone(),
            release_key: None,
            bundle_cache: Some(self.cfg.data_dir.join("ota_staging").join("bundle")),
        })
    }

    /// P1-5: OtaEngine that also adopts the persisted per-release binding, so
    /// staging/verification outside a just-fetched manifest still enforces the
    /// release-key chain.
    pub fn ota_engine_adopted(&self) -> gr_ota::OtaEngine {
        let mut eng = self.ota_engine();
        if let Err(e) = eng.load_release_binding() {
            tracing::warn!("release binding load failed: {e}");
        }
        eng
    }

    /// Download FE tarball from current release base and extract over `static_dir` parent.
    /// Static assets only — **no process restart**.
    ///
    /// Integrity (R-01/R-05): when release manifest lists `fe.sha256`, verify before extract;
    /// production with pubkey requires sha; tar entries are path-sanitized.
    pub fn ota_install_fe(&self, static_dir: &std::path::Path) -> Result<serde_json::Value, String> {
        let base = self.get_release_base_url();
        if base.is_empty() {
            return Err("release_base_url not configured".into());
        }
        if base.starts_with("http://") {
            let prod = gr_abi::env::get("DEPLOY_ENV")
                .map(|e| {
                    let e = e.trim().to_ascii_lowercase();
                    matches!(e.as_str(), "prod" | "production" | "live")
                })
                .unwrap_or(false);
            let allow = gr_abi::env::get("ALLOW_HTTP_RELEASE")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            if prod && !allow {
                return Err("production forbids http FE download".into());
            }
        }
        // version from URL tail (…/v6.0.7) or VERSION file on release
        let ver = base
            .rsplit('/')
            .next()
            .unwrap_or("")
            .trim_start_matches('v')
            .to_string();
        let fe_name = if ver.is_empty() {
            "fe.tgz".into()
        } else {
            format!("fe-{ver}.tgz")
        };
        // Prefer manifest-declared asset name / hash when available.
        let (fe_name, expect_sha) = match self.ota_fetch_manifest() {
            Ok(man) => {
                if let Some(fe) = man.fe {
                    (fe.asset, Some(fe.sha256))
                } else {
                    (fe_name, None)
                }
            }
            Err(e) => {
                let prod = gr_abi::env::get("DEPLOY_ENV")
                     .or_else(|| gr_abi::env::get("DEPLOY_ENV"))
                    .map(|e| {
                        let e = e.trim().to_ascii_lowercase();
                        matches!(e.as_str(), "prod" | "production" | "live")
                    })
                    .unwrap_or(false);
                if prod || !self.cfg.pubkey.is_empty() {
                    return Err(format!("signed FE install requires manifest: {e}"));
                }
                (fe_name, None)
            }
        };
        let url = format!("{}/{}", base.trim_end_matches('/'), fe_name);
        let tmp_dir = self.cfg.data_dir.join("ota_staging").join("fe");
        std::fs::create_dir_all(&tmp_dir).map_err(|e| e.to_string())?;
        let tgz = tmp_dir.join(&fe_name);
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .map_err(|e| e.to_string())?;
        let bytes = client
            .get(&url)
            .send()
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .bytes()
            .map_err(|e| e.to_string())?;
        let got_sha = gr_abi::sha256_hex(&bytes);
        if let Some(ref expect) = expect_sha {
            if got_sha != expect.to_ascii_lowercase() && got_sha != *expect {
                return Err(format!("FE sha256 mismatch: got {got_sha} expect {expect}"));
            }
        } else if !self.cfg.pubkey.is_empty()
            && gr_abi::env::get("REQUIRE_FE_SHA")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or_else(|| {
                    gr_abi::env::get("DEPLOY_ENV")
                        .map(|e| {
                            let e = e.trim().to_ascii_lowercase();
                            matches!(e.as_str(), "prod" | "production" | "live")
                        })
                        .unwrap_or(false)
                })
        {
            return Err(
                "FE install requires manifest fe.sha256 when pubkey/prod is configured".into(),
            );
        }
        std::fs::write(&tgz, &bytes).map_err(|e| e.to_string())?;
        // Extract into parent of static_dir if it ends with /fe, else into static_dir
        let dest_root = if static_dir
            .file_name()
            .map(|s| s == "fe")
            .unwrap_or(false)
        {
            static_dir
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| static_dir.to_path_buf())
        } else {
            static_dir.to_path_buf()
        };
        // Stage into versioned dir then atomic rename of `fe/` when possible.
        // Unique per call (pid + seq + nanos): concurrent callers in this
        // process (manual install-fe + full-upgrade + auto-apply hot loop)
        // used to share the pid-only name and delete each other's in-flight
        // extraction — same race class as the gr-ota bundle stage fix.
        static FE_STAGE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let fe_now_nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let stage_root = tmp_dir.join(format!(
            "extract-{}-{fe_now_nanos}-{}",
            std::process::id(),
            FE_STAGE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&stage_root);
        std::fs::create_dir_all(&stage_root).map_err(|e| e.to_string())?;
        gr_ota::safe_extract_tar_gz(&tgz, &stage_root)?;
        // Prefer staged `fe/` subdir
        let staged_fe = if stage_root.join("fe").is_dir() {
            stage_root.join("fe")
        } else {
            stage_root.clone()
        };
        let final_fe = if dest_root.file_name().map(|s| s == "fe").unwrap_or(false) {
            dest_root.clone()
        } else {
            dest_root.join("fe")
        };
        // Backup previous FE
        if final_fe.is_dir() {
            let bak = dest_root.join(format!(
                "fe.bak.{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            ));
            let _ = std::fs::rename(&final_fe, &bak);
        }
        // Move staged into place
        if let Some(parent) = final_fe.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        // copy_dir recursive
        copy_dir_recursive(&staged_fe, &final_fe).map_err(|e| e.to_string())?;
        // FE content identity for bootstrap asset_base cache bust (independent of runtime binary).
        let asset_gen: String = got_sha.chars().take(12).collect();
        if !ver.is_empty() {
            let _ = std::fs::write(final_fe.join("VERSION"), ver.as_bytes());
            let _ = std::fs::write(dest_root.join("VERSION"), ver.as_bytes());
        }
        let _ = std::fs::write(final_fe.join("ASSET_GEN"), asset_gen.as_bytes());
        // Optional: also stamp race-derived gen if VERSION missing from release URL.
        if ver.is_empty() {
            if let Ok(race) = std::fs::read(final_fe.join("gr.race.min.js")) {
                let g: String = gr_abi::sha256_hex(&race).chars().take(12).collect();
                let _ = std::fs::write(final_fe.join("ASSET_GEN"), g.as_bytes());
            }
        }
        let fe_ver_on_disk = std::fs::read_to_string(final_fe.join("VERSION"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| ver.clone());
        let gen_on_disk = std::fs::read_to_string(final_fe.join("ASSET_GEN"))
            .ok()
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| asset_gen.clone());
        let _ = std::fs::remove_dir_all(&stage_root);
        Ok(serde_json::json!({
            "ok": true,
            "url": url,
            "bytes": bytes.len(),
            "sha256": got_sha,
            "sha_verified": expect_sha.is_some(),
            "dest": final_fe,
            "fe_version": fe_ver_on_disk,
            "asset_gen": gen_on_disk,
            "restart_required": false,
            "note": "FE updated hot; bootstrap asset_base includes fe_version/g/asset_gen — browsers auto-load new JS without hard refresh",
        }))
    }

    /// Download runtime binary from release; stage to versioned slot; optionally restart.
    /// **Requires process restart** for new CARGO_PKG_VERSION / plane code.
    ///
    /// Integrity (R-01/R-06): ELF + optional manifest sha256; write `bin/releases/<ver>/`
    /// then install to `bin/gr-service` with backup; restart script probes health.
    pub fn ota_install_runtime(
        &self,
        restart: bool,
        install_root: &std::path::Path,
    ) -> Result<serde_json::Value, String> {
        let base = self.get_release_base_url();
        if base.is_empty() {
            return Err("release_base_url not configured".into());
        }
        if base.starts_with("http://") {
            let prod = gr_abi::env::get("DEPLOY_ENV")
                .map(|e| {
                    let e = e.trim().to_ascii_lowercase();
                    matches!(e.as_str(), "prod" | "production" | "live")
                })
                .unwrap_or(false);
            let allow = gr_abi::env::get("ALLOW_HTTP_RELEASE")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            if prod && !allow {
                return Err("production forbids http runtime download".into());
            }
        }
        let ver = base
            .rsplit('/')
            .next()
            .unwrap_or("")
            .trim_start_matches('v')
            .to_string();
        let arch = std::env::consts::ARCH;
        let triple = format!("{arch}-linux-gnu");
        let mut asset = format!("gr-service-{ver}-{triple}");
        let mut expect_sha: Option<String> = None;
        // One engine for the whole flow: bundle releases resolve manifest +
        // assets from the verified extracted bundle tree.
        let mut eng = self.ota_engine();
        let mut fetched_man: Option<gr_abi::ReleaseManifest> = None;
        match eng.fetch_manifest_blocking() {
            Ok(man) => {
                if let Err(e) = eng.write_release_binding(&man) {
                    tracing::warn!("release binding write failed: {e}");
                }
                if let Some(a) = man.runtime.asset.clone() {
                    asset = a;
                }
                if let Some(h) = man.runtime.sha256.clone() {
                    expect_sha = Some(h);
                }
                fetched_man = Some(man);
            }
            Err(e) => {
                let prod = gr_abi::env::get("DEPLOY_ENV")
                     .or_else(|| gr_abi::env::get("DEPLOY_ENV"))
                    .map(|env| {
                        let env = env.trim().to_ascii_lowercase();
                        matches!(env.as_str(), "prod" | "production" | "live")
                    })
                    .unwrap_or(false);
                if prod || !self.cfg.pubkey.is_empty() {
                    return Err(format!("signed runtime install requires manifest: {e}"));
                }
            }
        }
        if asset.contains("..") || asset.starts_with('/') {
            return Err(format!("runtime asset path invalid: {asset}"));
        }
        let bin_dir = install_root.join("bin");
        let rel_dir = bin_dir.join("releases").join(&ver);
        std::fs::create_dir_all(&rel_dir).map_err(|e| e.to_string())?;
        let dest = bin_dir.join("gr-service");
        let staged = rel_dir.join("gr-service");
        // Whole-bundle releases: read the runtime binary straight from the
        // extracted bundle tree (already sha-verified as a whole by the index).
        let asset_source: String;
        let bytes: Vec<u8> = if let Some(bd) = eng.bundle_dir.clone() {
            asset_source = format!("bundle:{asset}");
            std::fs::read(bd.join(&asset)).map_err(|e| {
                format!("bundle local asset {asset} unreadable: {e}")
            })?
        } else {
            let url = format!("{}/{}", base.trim_end_matches('/'), asset);
            asset_source = url.clone();
            let client = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(600))
                .build()
                .map_err(|e| e.to_string())?;
            client
                .get(&url)
                .send()
                .map_err(|e| e.to_string())?
                .error_for_status()
                .map_err(|e| e.to_string())?
                .bytes()
                .map_err(|e| e.to_string())?
                .to_vec()
        };
        if bytes.len() < 4 || &bytes[0..4] != b"\x7fELF" {
            return Err("downloaded asset is not ELF".into());
        }
        let got_sha = gr_abi::sha256_hex(&bytes);
        if let Some(ref expect) = expect_sha {
            if got_sha != expect.to_ascii_lowercase() && got_sha != *expect {
                return Err(format!(
                    "runtime sha256 mismatch: got {got_sha} expect {expect}"
                ));
            }
        } else if !self.cfg.pubkey.is_empty()
            && gr_abi::env::get("REQUIRE_RUNTIME_SHA")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or_else(|| {
                    gr_abi::env::get("DEPLOY_ENV")
                        .map(|e| {
                            let e = e.trim().to_ascii_lowercase();
                            matches!(e.as_str(), "prod" | "production" | "live")
                        })
                        .unwrap_or(false)
                })
        {
            return Err(
                "runtime install requires manifest runtime.sha256 when pubkey/prod is configured"
                    .into(),
            );
        }
        std::fs::write(&staged, &bytes).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&staged)
                .map_err(|e| e.to_string())?
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&staged, perms).map_err(|e| e.to_string())?;
        }
        if dest.is_file() {
            let bak = bin_dir.join(format!(
                "gr-service.bak.{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            ));
            let _ = std::fs::copy(&dest, &bak);
        }
        // Linux: cannot open-write a running executable (ETXTBSY). Write sibling then rename
        // so the new inode replaces the path while the old mapping stays alive until restart.
        // Unique per call (pid + seq + nanos): concurrent callers in this process (panel
        // manual call + auto-apply hot loop + full-upgrade) used to share the pid-only
        // name and could corrupt each other's staged copy before the rename — same race
        // class as the gr-ota extract-stage collision (178 v1.0.8 concurrent OTA).
        static SWAP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let now_nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let install_tmp = bin_dir.join(format!(
            ".gr-service.new.{}-{now_nanos}-{}",
            std::process::id(),
            SWAP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        // Best-effort sweep of swap temps abandoned by failed calls.
        if let Ok(rd) = std::fs::read_dir(&bin_dir) {
            for ent in rd.flatten() {
                let stale = ent
                    .file_name()
                    .to_str()
                    .is_some_and(|n| n.starts_with(".gr-service.new."))
                    && ent
                        .metadata()
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age > std::time::Duration::from_secs(600));
                if stale {
                    let _ = std::fs::remove_file(ent.path());
                }
            }
        }
        std::fs::copy(&staged, &install_tmp).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&install_tmp)
                .map_err(|e| e.to_string())?
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&install_tmp, perms).map_err(|e| e.to_string())?;
        }
        std::fs::rename(&install_tmp, &dest).map_err(|e| e.to_string())?;
        // iss/audit OPR-01: VERSION write is gated on the health check when
        // restart=true (HEALTH_OK → new ver, rollback → previous ver). The
        // previous eager stamp left a new-VERSION / old-binary split brain on
        // HEALTH_FAIL: auto_upgrade_check's monotonic gate then read the new
        // version as "current" and never retried — a permanently locked-out,
        // half-upgraded host. restart=false keeps the immediate stamp (no
        // health gate to wait for; nothing serves the new binary before the
        // next restart anyway).
        let prev_version = std::fs::read_to_string(install_root.join("VERSION"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if !restart {
            let _ = std::fs::write(install_root.join("VERSION"), ver.as_bytes());
        }
        let _ = std::fs::write(rel_dir.join("sha256"), format!("{got_sha}\n"));
        // data_tree (1.0.8+): a panel runtime OTA must land the product data
        // files (r100 templates + geoip mmdb) too, not just the binary — read
        // them straight from the verified extracted bundle (the manifest chain
        // incl. `sig_data` was checked by fetch_manifest_blocking). 1.0.7-era
        // panels never reach this code (old binary runs during their upgrade);
        // that transition is closed by the boot-time staging bootstrap.
        let mut data_installed: Vec<String> = Vec::new();
        let mut data_error: Option<String> = None;
        if let (Some(bd), Some(man)) = (eng.bundle_dir.as_ref(), fetched_man.as_ref()) {
            match overlay_data_tree(bd, man, &install_root.join("data")) {
                Ok(v) => data_installed = v,
                Err(e) => {
                    tracing::warn!(error = %e, "data_tree overlay failed (boot bootstrap retries)");
                    data_error = Some(e);
                }
            }
        }
        let mut restarted = false;
        let mut restart_note = String::new();
        if restart {
            // R-06: after restart, probe health; on failure restore newest bak and restart again.
            // iss/audit OPR-01: the health gate owns the VERSION stamp —
            // HEALTH_OK stamps the new version; the rollback branch restores
            // the previous version alongside the .bak binary (split-brain lock).
            let bak_glob = bin_dir.join("gr-service.bak.*");
            let dest_s = dest.display().to_string();
            let bak_pat = bak_glob.display().to_string();
            let version_path = install_root.join("VERSION").display().to_string();
            let ver_q = shell_single_quote(ver.as_str());
            let prev_ver_q = shell_single_quote(&prev_version);
            let script = format!(
                r#"set -e
sleep 2
UNIT=greenpng
/bin/systemctl restart "$UNIT" || (/bin/systemctl kill -s SIGTERM "$UNIT"; sleep 1; /bin/systemctl start "$UNIT")
ok=0
for i in 1 2 3 4 5 6; do
  sleep 2
  if curl -fsS http://127.0.0.1:28680/v1/health >/tmp/gr-ota-health.json 2>/dev/null; then
    ok=1
    break
  fi
done
if [ "$ok" != "1" ]; then
  echo HEALTH_FAIL >>/tmp/gr-ota-restart.log
  newest=$(ls -1t {bak_pat} 2>/dev/null | head -1 || true)
  if [ -n "$newest" ] && [ -x "$newest" ]; then
    echo "ROLLBACK_TO $newest" >>/tmp/gr-ota-restart.log
    cp -a "$newest" "{dest_s}"
    chmod 755 "{dest_s}"
    if [ -n {prev_ver_q} ]; then
      printf '%s\n' {prev_ver_q} > "{version_path}" || true
      echo VERSION_ROLLBACK >>/tmp/gr-ota-restart.log
    fi
    /bin/systemctl restart "$UNIT" || true
    sleep 3
    curl -fsS http://127.0.0.1:28680/v1/health >/tmp/gr-ota-health-rollback.json 2>/dev/null \
      && echo ROLLBACK_HEALTH_OK >>/tmp/gr-ota-restart.log \
      || echo ROLLBACK_HEALTH_FAIL >>/tmp/gr-ota-restart.log
  else
    echo NO_BACKUP_FOR_ROLLBACK >>/tmp/gr-ota-restart.log
    if [ -n {prev_ver_q} ]; then
      printf '%s\n' {prev_ver_q} > "{version_path}" || true
    fi
  fi
else
  echo HEALTH_OK >>/tmp/gr-ota-restart.log
  printf '%s\n' {ver_q} > "{version_path}" || true
  echo "VERSION_STAMPED {ver_q}" >>/tmp/gr-ota-restart.log
fi
"#,
                bak_pat = bak_pat,
                dest_s = dest_s,
                version_path = version_path,
                ver_q = ver_q,
                prev_ver_q = prev_ver_q,
            );
            let spawned = std::process::Command::new("/bin/bash")
                .args([
                    "-c",
                    &format!(
                        "nohup bash -c {} >/tmp/gr-ota-restart.log 2>&1 &",
                        shell_single_quote(&script)
                    ),
                ])
                .spawn();
            match spawned {
                Ok(_) => {
                    restarted = true;
                    restart_note =
                        "scheduled systemctl restart + health probe with bak rollback on fail".into();
                }
                Err(e) => {
                    restart_note = format!("failed to schedule restart: {e}");
                }
            }
        }
        Ok(serde_json::json!({
            "ok": true,
            "url": asset_source,
            "path": dest,
            "release_slot": staged,
            "version": ver,
            "bytes": bytes.len(),
            "sha256": got_sha,
            "sha_verified": expect_sha.is_some(),
            "data_installed": data_installed,
            "data_error": data_error,
            "restart_requested": restart,
            "restarted": restarted,
            "restart_required": restart && !restarted,
            "note": if !restart {
                "binary staged+installed; set restart=true or systemctl restart greenpng".to_string()
            } else if restart_note.is_empty() {
                "binary installed".into()
            } else {
                format!("binary installed; {restart_note}")
            },
        }))
    }

    /// Stage a local .so as module@version (lab/dev: optional unsigned when pubkey empty).
    pub fn ota_stage_local(
        &self,
        name: &str,
        version: &str,
        domain: &str,
        src: &std::path::Path,
    ) -> Result<std::path::PathBuf, String> {
        if !src.is_file() {
            return Err(format!("source not found: {}", src.display()));
        }
        let data = std::fs::read(src).map_err(|e| e.to_string())?;
        let sha = gr_abi::sha256_hex(&data);
        let asset = src
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("lib{name}.so"));
        let domain_s = if domain.is_empty() {
            name.to_string()
        } else {
            domain.to_string()
        };
        let art = gr_abi::ModuleArtifact {
            name: name.to_string(),
            version: version.to_string(),
            abi: RUNTIME_ABI,
            requires_major: gr_abi::PRODUCT_MAJOR,
            min_runtime_minor: 0,
            max_runtime_minor: None,
            asset: asset.clone(),
            sha256: sha,
            sig: String::new(),
            sig2: None,
            domain: domain_s.clone(),
        };
        if !self.cfg.pubkey.is_empty() {
            // Production: panel stages only when no pubkey (lab) or via signed remote OTA path.
            return Err(
                "signed OTA staging requires pre-signed release asset; use activate on staged versions or clear pubkey for lab"
                    .into(),
            );
        }
        let eng = self.ota_engine();
        let dest_dir = eng.versions_dir(name).join(version);
        std::fs::create_dir_all(&dest_dir).map_err(|e| e.to_string())?;
        let dest = dest_dir.join(&asset);
        std::fs::copy(src, &dest).map_err(|e| e.to_string())?;
        std::fs::write(
            dest_dir.join("meta.json"),
            serde_json::to_vec_pretty(&art).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        self.admin.db.record_module(
            name,
            version,
            &dest.display().to_string(),
            false,
            &serde_json::json!({"domain": domain_s, "asset": asset, "unsigned_lab": true}),
        )?;
        Ok(dest)
    }

    /// Modules safe to `dlopen` while the process already has static-linked business crates.
    /// Others are staged + activation-marked only (static path / next restart).
    /// `analyze` is the critical hot mint path and must stay dynamic.
    fn should_hot_dlopen(name: &str) -> bool {
        matches!(name, "analyze")
    }

    /// Activate a staged module version and hot-load without process restart.
    ///
    /// R-10: for `analyze`, hot_load failure does **not** commit active marker as success —
    /// previous active remains authoritative in registry; DB records failure.
    pub fn ota_activate(&self, name: &str, version: &str) -> Result<serde_json::Value, String> {
        let eng = self.ota_engine();
        // Snapshot previous active marker for rollback on load failure.
        let prev_active = std::fs::read_to_string(eng.active_link(name)).ok();
        // Monotonic version floor (iss/opus5 P0-2): never silently activate an
        // older module over a newer one. Downgrade requires explicit opt-in.
        if let Some(prev_path) = prev_active.as_deref() {
            let prev_ver = std::path::PathBuf::from(prev_path.trim())
                .file_name()
                .map(|f| f.to_string_lossy().to_string());
            if let Some(pv) = prev_ver {
                let new_parsed = semver::Version::parse(version);
                let old_parsed = semver::Version::parse(&pv);
                if let (Ok(new_v), Ok(old_v)) = (new_parsed, old_parsed) {
                    let allow_downgrade = gr_abi::env::get("OTA_ALLOW_DOWNGRADE")
                        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                        .unwrap_or(false);
                    if new_v < old_v && !allow_downgrade {
                        return Err(format!(
                            "refusing downgrade of {name}: active {old_v} > requested {new_v} \
                             (set GR_OTA_ALLOW_DOWNGRADE=1 to force)"
                        ));
                    }
                }
            }
        }
        let ver_dir = eng
            .activate(name, version)
            .map_err(|e| e.to_string())?;
        let so = crate::loader::ModuleLoader::find_so_in_version_dir(&ver_dir)
            .or_else(|| {
                // also accept any file in dir
                std::fs::read_dir(&ver_dir).ok().and_then(|rd| {
                    rd.filter_map(|e| e.ok())
                        .map(|e| e.path())
                        .find(|p| {
                            p.extension()
                                .map(|x| x == "so")
                                .unwrap_or(false)
                        })
                })
            })
            .ok_or_else(|| format!("no .so in {}", ver_dir.display()))?;
        // Only dlopen analyze (mint_wire). Other plugin .so conflict with static-linked
        // crates and historically SEGV on re-load; activation marker is enough for bookkeeping.
        let (loaded, hot_note, commit_active) = if Self::should_hot_dlopen(name) {
            // Skip re-dlopen when the same path+version is already live (avoids churn).
            let already = self.registry.get(name).and_then(|m| {
                if m.version() == version && m.path == so {
                    Some(m)
                } else {
                    None
                }
            });
            if let Some(m) = already {
                (
                    Some(serde_json::json!({
                        "name": m.name(),
                        "version": m.version(),
                        "path": m.path,
                    })),
                    "already_loaded_same_version",
                    true,
                )
            } else {
                match self.registry.hot_load(name, &so) {
                    Ok(m) => (
                        Some(serde_json::json!({
                            "name": m.name(),
                            "version": m.version(),
                            "path": m.path,
                        })),
                        "hot_dlopen",
                        true,
                    ),
                    Err(e) => {
                        tracing::warn!(module = %name, error = %e, "hot_load failed; rolling back active marker");
                        // Restore previous active marker if we had one.
                        if let Some(prev) = prev_active.as_ref() {
                            let _ = std::fs::write(eng.active_link(name), prev);
                        }
                        (None, "hot_dlopen_failed", false)
                    }
                }
            }
        } else {
            tracing::info!(
                module = %name,
                version = %version,
                "activate without dlopen (static-link safe path)"
            );
            (None, "activation_marker_only_no_dlopen", true)
        };
        let _ = self.admin.db.record_module(
            name,
            version,
            &so.display().to_string(),
            commit_active,
            &serde_json::json!({
                "activated": commit_active,
                "hot_loaded": loaded.is_some(),
                "hot_note": hot_note,
            }),
        );
        if commit_active {
            let _ = self.admin.db.set_module_active(name, version);
            self.config_version.fetch_add(1, Ordering::Relaxed);
            self.refresh_live_config()?;
        }
        if !commit_active && Self::should_hot_dlopen(name) {
            return Err(format!(
                "activate {name}@{version} hot_load failed ({hot_note}); previous active retained"
            ));
        }
        Ok(serde_json::json!({
            "ok": true,
            "name": name,
            "version": version,
            "path": so,
            "restart_required": false,
            "loaded": loaded,
            "hot_note": hot_note,
        }))
    }

    pub fn ota_list_local(&self) -> Result<Vec<gr_ota::LocalModuleState>, String> {
        self.ota_engine().list_local().map_err(|e| e.to_string())
    }

    pub fn ota_fetch_manifest(&self) -> Result<gr_abi::ReleaseManifest, String> {
        if self.get_release_base_url().trim().is_empty() {
            return Err("release_base_url not configured".into());
        }
        let mut eng = self.ota_engine();
        let man = eng.fetch_manifest_blocking().map_err(|e| e.to_string())?;
        // P1-5: remember the verified per-release binding so boot-time
        // self-verification of active modules uses the full chain.
        if !self.cfg.pubkey.is_empty() {
            eng.write_release_binding(&man).map_err(|e| e.to_string())?;
        }
        Ok(man)
    }

    /// P1-5: integrity self-check of active modules (chain signatures + hashes)
    /// using the persisted release binding. Returns per-module verdicts.
    pub fn ota_verify_active_modules(&self) -> Result<serde_json::Value, String> {
        if self.cfg.pubkey.is_empty() {
            return Err("pubkey not configured — pass --pubkey-path for signed OTA".into());
        }
        let mut eng = self.ota_engine();
        eng.load_release_binding()
            .map_err(|e| format!("release binding load failed: {e}"))?;
        let verdict = eng.verify_active_modules().map_err(|e| e.to_string())?;
        let strict = gr_abi::env::get("STRICT_BOOT_VERIFY")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if strict {
            let legacy = verdict["legacy_unverifiable"]
                .as_array()
                .map(|a| a.len())
                .unwrap_or(0);
            if legacy > 0 {
                return Err(format!(
                    "GR_STRICT_BOOT_VERIFY=1 but {legacy} active module(s) lack meta.json"
                ));
            }
        }
        Ok(verdict)
    }

    /// Download signed module from release_base_url, verify ed25519+sha256, stage, optionally activate.
    pub fn ota_install_remote(
        &self,
        name: &str,
        version: Option<&str>,
        activate: bool,
    ) -> Result<serde_json::Value, String> {
        if self.cfg.pubkey.is_empty() {
            return Err("pubkey not configured — pass --pubkey-path for signed OTA".into());
        }
        if self.get_release_base_url().trim().is_empty() {
            return Err("release_base_url not configured".into());
        }
        let mut eng = self.ota_engine();
        // P0-2: verify the manifest chain and keep the verified release key in
        // the engine so artifact staging requires sig2 (release key) too.
        let man = eng.fetch_manifest_blocking().map_err(|e| e.to_string())?;
        eng.write_release_binding(&man).map_err(|e| e.to_string())?;
        // Same selection semantics as the CLI `module update`: exact match when
        // a version is given, else the highest semantic version for the name.
        let art = gr_ota::pick_module(&man, name, version)
            .map_err(|e| e.to_string())?
            .clone();
        let tmp_dir = self.cfg.modules_dir.join("staging").join(&art.name).join(&art.version);
        std::fs::create_dir_all(&tmp_dir).map_err(|e| e.to_string())?;
        let tmp = tmp_dir.join(&art.asset);
        eng.download_asset_blocking(&art, &tmp)
            .map_err(|e| e.to_string())?;
        let dest = eng
            .stage_local_file(&art, &tmp, PRODUCT_VERSION)
            .map_err(|e| e.to_string())?;
        self.admin.db.record_module(
            &art.name,
            &art.version,
            &dest.display().to_string(),
            false,
            &serde_json::json!({
                "domain": art.domain,
                "asset": art.asset,
                "signed": true,
                "source": "remote_ota",
                "sha256": art.sha256,
            }),
        )?;
        let mut out = serde_json::json!({
            "ok": true,
            "name": art.name,
            "version": art.version,
            "path": dest,
            "signed": true,
            "activated": false,
            "restart_required": false,
        });
        if activate {
            let act = self.ota_activate(&art.name, &art.version)?;
            out["activated"] = serde_json::json!(true);
            out["activate"] = act;
        }
        Ok(out)
    }

    /// Install every module listed in remote manifest (optional activate each).
    pub fn ota_install_all_remote(&self, activate: bool) -> Result<serde_json::Value, String> {
        let man = self.ota_fetch_manifest()?;
        let mut items = Vec::new();
        let mut errors = Vec::new();
        for m in &man.modules {
            match self.ota_install_remote(&m.name, Some(&m.version), activate) {
                Ok(v) => items.push(v),
                Err(e) => errors.push(serde_json::json!({"name": m.name, "version": m.version, "error": e})),
            }
        }
        Ok(serde_json::json!({
            "ok": errors.is_empty(),
            "installed": items,
            "errors": errors,
            "manifest_product": man.product,
            "manifest_channel": man.channel,
            "runtime": man.runtime,
        }))
    }

    /// Full panel upgrade: modules (+ optional FE + optional runtime binary).
    /// Modules/FE: hot (no restart). Runtime binary: requires restart when restart=true.
    pub fn ota_full_upgrade(
        &self,
        static_dir: &std::path::Path,
        install_root: &std::path::Path,
        install_modules: bool,
        install_fe: bool,
        install_runtime: bool,
        restart_runtime: bool,
        activate: bool,
    ) -> Result<serde_json::Value, String> {
        let base = self.get_release_base_url();
        if base.is_empty() {
            return Err("release_base_url not configured — POST ota/set-release-url first".into());
        }
        let mut out = serde_json::json!({
            "ok": true,
            "release_base_url": base,
            "steps": {},
        });
        if install_modules {
            match self.ota_install_all_remote(activate) {
                Ok(v) => {
                    if v.get("ok").and_then(|x| x.as_bool()) == Some(false) {
                        out["ok"] = serde_json::json!(false);
                    }
                    out["steps"]["modules"] = v;
                }
                Err(e) => {
                    out["ok"] = serde_json::json!(false);
                    out["steps"]["modules"] = serde_json::json!({"ok": false, "error": e});
                }
            }
        }
        if install_fe {
            match self.ota_install_fe(static_dir) {
                Ok(v) => out["steps"]["fe"] = v,
                Err(e) => {
                    out["ok"] = serde_json::json!(false);
                    out["steps"]["fe"] = serde_json::json!({"ok": false, "error": e});
                }
            }
        }
        if install_runtime {
            match self.ota_install_runtime(restart_runtime, install_root) {
                Ok(v) => out["steps"]["runtime"] = v,
                Err(e) => {
                    out["ok"] = serde_json::json!(false);
                    out["steps"]["runtime"] = serde_json::json!({"ok": false, "error": e});
                }
            }
        }
        out["note"] = serde_json::json!(
            "modules/FE: hot; runtime binary needs process restart (systemctl restart greenpng)"
        );
        Ok(out)
    }
}

/// P0 (178 panel OTA, 1.0.8): scan the OTA bundle staging area
/// (`<data_dir>/ota_staging/bundle`) for an extracted whole-bundle release
/// whose manifest carries a `data_tree`, verify the full chain against the
/// root pubkey (incl. the extended-body `sig_data`), and overlay the
/// declared product data files into `dest_data`.
///
/// Why this exists: a 1.0.7 panel `install-runtime` (the code that RUNS
/// during the 1.0.7 → 1.0.8 upgrade) stages and verifies the whole bundle
/// but installs only the runtime binary — it predates data_tree. This
/// bootstrap runs inside the NEW binary on its first boot after that swap,
/// closing the gap without any host-side script. Never deletes files (the
/// data dir also holds runtime state); never performs network I/O.
pub fn bootstrap_data_tree_from_staging(
    staging_bundle: &std::path::Path,
    pubkey: &[u8],
    dest_data: &std::path::Path,
) -> Result<Vec<String>, String> {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    let entries = match std::fs::read_dir(staging_bundle) {
        Ok(rd) => rd,
        Err(e) => return Err(format!("staging dir {}: {e}", staging_bundle.display())),
    };
    for ent in entries.flatten() {
        let Ok(ty) = ent.file_type() else { continue };
        if !ty.is_dir() {
            continue;
        }
        let d = ent.path();
        // fetch_bundle_blocking extracts tars to `<stem>.d/<top-dir>/…`;
        // bundle_tree_root resolves the single-top-dir shape. Also accept a
        // flat extraction (manifest directly inside).
        if d.join("manifest.json").is_file() {
            candidates.push(d);
        } else {
            let root = gr_ota::bundle_tree_root(&d);
            if root.join("manifest.json").is_file() {
                candidates.push(root);
            }
        }
    }
    // Newest first: repeated OTAs leave several staged versions.
    candidates.sort_by_key(|p| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0)
    });
    candidates.reverse();
    if candidates.is_empty() {
        return Err("no staged bundle manifest under ota_staging/bundle".into());
    }
    let mut last_err = String::from("no usable staged bundle manifest");
    for root in candidates {
        let parsed = std::fs::read_to_string(root.join("manifest.json"))
            .map_err(|e| format!("staged manifest read: {e}"))
            .and_then(|s| {
                serde_json::from_str::<gr_abi::ReleaseManifest>(&s)
                    .map_err(|e| format!("staged manifest parse: {e}"))
            });
        let man = match parsed {
            Ok(m) => m,
            Err(e) => {
                last_err = e;
                continue;
            }
        };
        if man.data_tree.is_none() {
            last_err = "staged manifest has no data_tree (pre-1.0.8 bundle)".into();
            continue;
        }
        if let Err(e) = gr_ota::verify_manifest_chain(pubkey, &man) {
            last_err = format!("staged manifest chain: {e}");
            continue;
        }
        return overlay_data_tree(&root, &man, dest_data);
    }
    Err(last_err)
}

/// Overlay the manifest-declared product data files from a VERIFIED bundle
/// tree into `dest_data`. Only writes files that are missing locally or
/// whose sha256 differs from the signed manifest entry (self-healing on
/// every boot); never deletes anything. Source bytes are re-verified against
/// the manifest hashes before copying — callers must have verified the
/// manifest chain (legacy `sig` + `sig_data`) beforehand.
pub fn overlay_data_tree(
    bundle_root: &std::path::Path,
    man: &gr_abi::ReleaseManifest,
    dest_data: &std::path::Path,
) -> Result<Vec<String>, String> {
    let Some(tree) = &man.data_tree else {
        return Ok(Vec::new());
    };
    let mut installed = Vec::new();
    for (rel, want) in &tree.files {
        if rel.contains("..") || rel.starts_with('/') || rel.contains('\\') {
            return Err(format!("data_tree bad relative path: {rel}"));
        }
        let src = bundle_root.join("data").join(rel);
        let src_bytes =
            std::fs::read(&src).map_err(|e| format!("bundle data file {rel}: {e}"))?;
        let got = gr_abi::sha256_hex(&src_bytes);
        if got != want.to_ascii_lowercase() && got != *want {
            return Err(format!("data_tree sha mismatch: {rel}"));
        }
        let dest = dest_data.join(rel);
        let needs_write = match std::fs::read(&dest) {
            Ok(cur) => gr_abi::sha256_hex(&cur) != got,
            Err(_) => true,
        };
        if needs_write {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(&dest, &src_bytes).map_err(|e| format!("install {rel}: {e}"))?;
            installed.push(rel.clone());
        }
    }
    Ok(installed)
}

fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{t:x}")
}

/// GR-DEP-018: production cluster keys must be random secrets, not placeholders.
pub fn cluster_key_acceptable(key: &str, prod: bool) -> Result<(), String> {
    if !prod {
        return Ok(());
    }
    let t = key.trim();
    let weak = t.is_empty()
        || t.len() < 16
        || t.eq_ignore_ascii_case("change-me-cluster")
        || t.eq_ignore_ascii_case("dev-cluster")
        || t.eq_ignore_ascii_case("changeme")
        || t.eq_ignore_ascii_case("cluster");
    if weak {
        return Err(
            "production GR_CLUSTER_KEY must be a random secret (≥16 chars; not a placeholder)"
                .into(),
        );
    }
    Ok(())
}

/// GR-OTA-005: parse scheme/host/path; never match by raw string prefix.
pub fn release_url_allowed(url: &str) -> Result<(), String> {
    let prod = gr_abi::env::get("DEPLOY_ENV")
         .or_else(|| gr_abi::env::get("DEPLOY_ENV"))
        .map(|e| {
            let e = e.trim().to_ascii_lowercase();
            matches!(e.as_str(), "prod" | "production" | "live")
        })
        .unwrap_or(false);
    let raw = gr_abi::env::get("RELEASE_URL_ALLOWLIST").unwrap_or_default();
    if raw.trim().is_empty() && !prod {
        return Ok(());
    }
    let raw = if raw.trim().is_empty() {
        // Production default allowlist: the signed public release repos —
        // gr-server (live, v1.0.6+) and the greenpng install repo (legacy
        // layouts). The legacy private v6 archive (kullyeilert-jpg) is
        // intentionally no longer allowed. 178 v1.0.6: the default pointed
        // only at the retired install repo and rejected every gr-server URL
        // until ops set GR_RELEASE_URL_ALLOWLIST by hand.
        "https://github.com/greenpng/gr-server/,https://github.com/greenpng/install/".to_string()
    } else {
        raw
    };
    if raw.split(',').any(|p| p.trim() == "*") {
        if prod {
            return Err("production refuses wildcard release allowlist".into());
        }
        return Ok(());
    }
    let got = parse_http_url(url).ok_or_else(|| "release_base_url is not a valid http(s) URL".to_string())?;
    if got.scheme == "http" && prod {
        return Err("production refuses http:// release URL".into());
    }
    for entry in raw.split(',') {
        let e = entry.trim();
        if e.is_empty() {
            continue;
        }
        if let Some(allow) = parse_http_url(e) {
            if got.scheme != allow.scheme || got.host != allow.host {
                continue;
            }
            let prefix = allow.path.trim_end_matches('/');
            if prefix.is_empty() {
                return Ok(());
            }
            if got.path == prefix || got.path.starts_with(&format!("{prefix}/")) {
                return Ok(());
            }
        }
    }
    Err("release_base_url host/path not allowlisted (GR_RELEASE_URL_ALLOWLIST / legacy GR_RELEASE_URL_ALLOWLIST)".into())
}

struct HttpParts {
    scheme: String,
    host: String,
    path: String,
}

fn parse_http_url(url: &str) -> Option<HttpParts> {
    let u = url.trim();
    if u.contains('@') {
        return None;
    }
    let (scheme, rest) = u.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "https" && scheme != "http" {
        return None;
    }
    let (hostport, path) = match rest.split_once('/') {
        Some((h, p)) => (h, format!("/{p}")),
        None => (rest, String::new()),
    };
    let host = hostport
        .split(':')
        .next()
        .unwrap_or("")
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host.is_empty() || host.contains('/') {
        return None;
    }
    Some(HttpParts { scheme, host, path })
}

#[cfg(test)]
mod release_url_tests {
    use super::*;
    use std::sync::Mutex;
    static ENV: Mutex<()> = Mutex::new(());

    #[test]
    fn prefix_confusion_rejected() {
        let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("GR_DEPLOY_ENV", "production");
        std::env::remove_var("GR_RELEASE_URL_ALLOWLIST");
        // Default allowlist = gr-server (live) + greenpng install repo (legacy).
        assert!(release_url_allowed(
            "https://github.com/greenpng/install/releases/download/v7.0.1"
        )
        .is_ok());
        assert!(release_url_allowed(
            "https://github.com/greenpng/install/releases/download/v7.0.2/fe-7.0.2.tgz"
        )
        .is_ok());
        assert!(release_url_allowed(
            "https://github.com/greenpng/gr-server/releases/download/v1.0.6"
        )
        .is_ok());
        assert!(release_url_allowed(
            "https://github.com/greenpng/gr-server/releases/download/v1.0.6/fe-1.0.6.tgz"
        )
        .is_ok());
        // Legacy private archive must NOT be reachable by default.
        assert!(release_url_allowed(
            "https://github.com/kullyeilert-jpg/gr-releases/releases/download/v6.0.28"
        )
        .is_err());
        assert!(release_url_allowed("https://github.com/greenpng.evil.com/x").is_err());
        assert!(release_url_allowed("https://greenpng.com/install/x").is_err());
        assert!(release_url_allowed("https://evil.com/greenpng/install/x").is_err());
        assert!(release_url_allowed("https://greenpng.com/gr-server/x").is_err());
        assert!(release_url_allowed("https://evil.com/greenpng/gr-server/x").is_err());
        std::env::remove_var("GR_DEPLOY_ENV");
    }

    #[test]
    fn cluster_placeholder_rejected_in_prod() {
        assert!(cluster_key_acceptable("dev-cluster", true).is_err());
        assert!(cluster_key_acceptable("change-me-cluster", true).is_err());
        assert!(cluster_key_acceptable("short", true).is_err());
        assert!(cluster_key_acceptable("0123456789abcdef0123456789abcdef", true).is_ok());
        assert!(cluster_key_acceptable("dev-cluster", false).is_ok());
    }
}

/// Single-quote a string for safe embedding in `bash -c '...'`.
fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}


/// R-05: safe tar.gz extract — reject absolute paths and `..` members before extract.
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for ent in std::fs::read_dir(src).map_err(|e| e.to_string())? {
        let ent = ent.map_err(|e| e.to_string())?;
        let ty = ent.file_type().map_err(|e| e.to_string())?;
        let from = ent.path();
        let to = dst.join(ent.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else if ty.is_file() {
            std::fs::copy(&from, &to).map_err(|e| e.to_string())?;
        }
        // skip symlinks intentionally
    }
    Ok(())
}

#[cfg(test)]
mod data_bootstrap_tests {
    use super::*;

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "gr_rt_data_{}_{}_{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|x| x.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Build a staged whole-bundle layout exactly like the one the 1.0.7
    /// panel `install-runtime` leaves behind: `…/bundle/<name>.d/<top>/`
    /// with manifest.json + data files, signed with a fresh root key.

    #[test]
    fn boot_bootstrap_installs_missing_and_preserves_state() {
        let (root_sk, root_vk) = gr_ota::generate_signing_keypair();
        let base = tmp_dir("boot");
        let staging = base.join("ota_staging").join("bundle");
        let extracted = staging.join("greenpng-1.0.8-x86_64.d");
        let root = extracted.join("greenpng-1.0.8-x86_64");
        let bdata = root.join("data");
        std::fs::create_dir_all(bdata.join("geo")).unwrap();
        let r100 = b"r100 template bytes";
        let mmdb = b"fake mmdb bytes";
        std::fs::write(bdata.join("r100_templates.json"), r100).unwrap();
        std::fs::write(bdata.join("geo/dbip-country-lite.mmdb"), mmdb).unwrap();

        let mut files = std::collections::BTreeMap::new();
        files.insert(
            "r100_templates.json".to_string(),
            gr_abi::sha256_hex(r100),
        );
        files.insert(
            "geo/dbip-country-lite.mmdb".to_string(),
            gr_abi::sha256_hex(mmdb),
        );
        let mut man = gr_abi::ReleaseManifest {
            product: "greenpng".into(),
            channel: "stable".into(),
            arch: Some("x86_64".into()),
            triple: Some("x86_64-linux-gnu".into()),
            build_id: Some("boot-bs-1".into()),
            release_pubkey: None,
            release_cert: None,
            runtime: gr_abi::RuntimeManifest {
                version: "1.0.8".into(),
                abi: 1,
                asset: Some("bin/gr-service".into()),
                sha256: None,
            },
            modules: vec![],
            fe: None,
            fe_tree: None,
            admin_tree: None,
            spec_tree: None,
            data_tree: Some(gr_abi::TreeManifest { epoch: None, files }),
            sig_data: None,
            cli: None,
            sig: None,
        };
        man.sig = Some(gr_ota::sign_bytes(
            &root_sk,
            &gr_ota::manifest_sign_message(&man).unwrap(),
        ));
        man.sig_data = Some(gr_ota::sign_bytes(
            &root_sk,
            &gr_ota::manifest_sign_message_data(&man).unwrap(),
        ));
        std::fs::write(
            root.join("manifest.json"),
            serde_json::to_string_pretty(&man).unwrap(),
        )
        .unwrap();

        // Destination data dir pre-holds runtime state that must survive.
        let dest = base.join("data");
        std::fs::create_dir_all(dest.join("admin")).unwrap();
        std::fs::write(dest.join("admin/bootstrap.json"), b"{\"state\":true}").unwrap();

        let installed =
            bootstrap_data_tree_from_staging(&staging, root_vk.as_bytes(), &dest).unwrap();
        assert_eq!(
            installed,
            vec![
                "geo/dbip-country-lite.mmdb".to_string(),
                "r100_templates.json".to_string()
            ],
            "both P0 data files installed (BTreeMap order)"
        );
        assert_eq!(
            std::fs::read(dest.join("r100_templates.json")).unwrap(),
            r100.to_vec()
        );
        assert_eq!(
            std::fs::read(dest.join("geo/dbip-country-lite.mmdb")).unwrap(),
            mmdb.to_vec()
        );
        assert!(
            dest.join("admin/bootstrap.json").is_file(),
            "runtime state must never be touched"
        );

        // Idempotent: second boot rewrites nothing.
        let again =
            bootstrap_data_tree_from_staging(&staging, root_vk.as_bytes(), &dest).unwrap();
        assert!(again.is_empty(), "no rewrites when sha matches: {again:?}");

        // Self-healing: local tamper is repaired from the verified bundle.
        std::fs::write(dest.join("r100_templates.json"), b"tampered local").unwrap();
        let healed =
            bootstrap_data_tree_from_staging(&staging, root_vk.as_bytes(), &dest).unwrap();
        assert_eq!(healed, vec!["r100_templates.json".to_string()]);
        assert_eq!(
            std::fs::read(dest.join("r100_templates.json")).unwrap(),
            r100.to_vec()
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn boot_bootstrap_rejects_unsigned_and_tampered_staging() {
        let (root_sk, root_vk) = gr_ota::generate_signing_keypair();
        let base = tmp_dir("reject");
        let staging = base.join("ota_staging").join("bundle");
        let extracted = staging.join("greenpng-1.0.8-x86_64.d");
        let root = extracted.join("greenpng-1.0.8-x86_64");
        std::fs::create_dir_all(root.join("data")).unwrap();
        let payload = b"payload";
        std::fs::write(root.join("data/r100_templates.json"), payload).unwrap();
        let mut files = std::collections::BTreeMap::new();
        files.insert(
            "r100_templates.json".to_string(),
            gr_abi::sha256_hex(payload),
        );
        let mut man = gr_abi::ReleaseManifest {
            product: "greenpng".into(),
            channel: "stable".into(),
            arch: None,
            triple: None,
            build_id: Some("boot-bs-2".into()),
            release_pubkey: None,
            release_cert: None,
            runtime: gr_abi::RuntimeManifest {
                version: "1.0.8".into(),
                abi: 1,
                asset: None,
                sha256: None,
            },
            modules: vec![],
            fe: None,
            fe_tree: None,
            admin_tree: None,
            spec_tree: None,
            data_tree: Some(gr_abi::TreeManifest { epoch: None, files }),
            sig_data: None,
            cli: None,
            sig: None,
        };
        man.sig = Some(gr_ota::sign_bytes(
            &root_sk,
            &gr_ota::manifest_sign_message(&man).unwrap(),
        ));
        // deliberately NO sig_data → chain must reject (data_tree unsigned)
        std::fs::write(
            root.join("manifest.json"),
            serde_json::to_string_pretty(&man).unwrap(),
        )
        .unwrap();
        let dest = base.join("data");
        std::fs::create_dir_all(&dest).unwrap();
        let err =
            bootstrap_data_tree_from_staging(&staging, root_vk.as_bytes(), &dest).unwrap_err();
        assert!(err.contains("sig_data"), "must demand sig_data: {err}");
        assert!(!dest.join("r100_templates.json").is_file());

        // Wrong root key → chain fails, nothing installed.
        let (_, other_vk) = gr_ota::generate_signing_keypair();
        let err2 =
            bootstrap_data_tree_from_staging(&staging, other_vk.as_bytes(), &dest).unwrap_err();
        assert!(err2.contains("chain") || err2.contains("sig"), "chain rejected: {err2}");

        let _ = std::fs::remove_dir_all(&base);
    }
}
