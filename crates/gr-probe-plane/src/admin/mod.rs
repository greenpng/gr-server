//! Built-in admin console module (replaces max-probe :28770 Ops).

pub mod apply;
pub mod association_store;
pub mod biz_store;
pub mod dashboard;
pub mod db;
pub mod deploy_center;
pub mod facet;
pub mod panel_config;
pub mod sdk;
pub mod ssl;

use crate::admin::association_store::AssociationStore;
use crate::admin::biz_store::BizStore;
use crate::admin::db::AdminDb;
use crate::handlers::AppState;
use crate::sni_map;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

pub struct AdminHub {
    pub db: AdminDb,
    pub static_dir: PathBuf,
    /// Website business dashboard store (independent from probe DB).
    pub biz: Arc<BizStore>,
    /// Association edges (iss/48) — not probe evidence.
    pub association: Arc<AssociationStore>,
}

impl AdminHub {
    pub fn open(admin_db: &Path, static_dir: PathBuf) -> Result<Arc<Self>, String> {
        let db = AdminDb::open(admin_db)?;
        let association = Arc::new(AssociationStore::open(&db.data_dir)?);
        let biz = Arc::new(BizStore::open_auto(&db.data_dir)?);
        Ok(Arc::new(Self {
            db,
            static_dir,
            biz,
            association,
        }))
    }
}


/// AdminDispatch is still produced by the ingest/gateway service for ACME
/// HTTP-01 responses; every other mx-console route moved to the control
/// plane (tasks/adminapi/03-target.md).
pub enum AdminDispatch {
    Json(u16, Value),
    Bytes(u16, &'static str, Vec<u8>),
    File(PathBuf),
}

/// ACME HTTP-01 only. The mx-console SPA + admin API moved to the control
/// plane (`gr-service` axum `/{console}/api/*`); this port serves the public
/// data plane (open/ingest/result) and must not expose a second login.
/// ACME HTTP-01 only. The mx-console SPA + admin API moved to the control
/// plane (`gr-service` axum `/{console}/api/*`); this port serves the public
/// data plane (open/ingest/result) and must not expose a second login.
pub fn try_dispatch(
    st: &AppState,
    method: &str,
    path: &str,
    _headers: &HashMap<String, String>,
    _query: &HashMap<String, String>,
    _body: &[u8],
) -> Option<AdminDispatch> {
    if method == "GET" && path.starts_with("/.well-known/acme-challenge/") {
        let token = path.trim_start_matches("/.well-known/acme-challenge/");
        if let Some(admin) = st.admin.as_ref() {
            if let Ok(Some(content)) = admin.db.get_acme_challenge(token) {
                return Some(AdminDispatch::Bytes(200, "text/plain", content.into_bytes()));
            }
            let file = admin.db.data_dir.join("acme-challenge").join(token);
            if file.is_file() {
                if let Ok(b) = std::fs::read(&file) {
                    return Some(AdminDispatch::Bytes(200, "text/plain", b));
                }
            }
        }
        return Some(AdminDispatch::Bytes(404, "text/plain", b"not found".to_vec()));
    }
    None
}
