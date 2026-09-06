//! Admin control-plane: auth bootstrap, sites/domains, metrics, OTA surface, config.
//!
//! Single admin user. Console path + credentials randomly generated at install.

pub mod auth;
pub mod db;
pub mod domain;
pub mod metrics;
pub mod sites;

pub use auth::{AdminAuth, BootstrapSecrets};
pub use db::AdminDb;
pub use domain::{host_allowed_for_site, normalize_hostname};
pub use metrics::SystemMetrics;
pub use sites::{
    mint_embed_token, normalize_fe_load, normalize_upload_ingest, reject_proxied_upload,
    CryptoProfile, SiteRecord,
};

use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;

pub struct AdminHub {
    pub db: AdminDb,
    pub auth: AdminAuth,
}

impl AdminHub {
    pub fn open(data_dir: &Path) -> Result<Arc<Self>, String> {
        std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
        let db = AdminDb::open_postgres()?;
        let secrets_file = data_dir.join("admin_bootstrap_once.txt");
        let auth = AdminAuth::bootstrap(&db, &secrets_file)?;
        tracing::info!(
            console_path = %auth.console_path,
            "admin hub ready (login path is random; see bootstrap file once)"
        );
        Ok(Arc::new(Self { db, auth }))
    }

    pub fn health_json(&self) -> Value {
        json!({
            "ok": true,
            "product": "green-v7",
            "console_path_set": !self.auth.console_path.is_empty(),
        })
    }
}
