//! Ingest domain: open/ingest gating by business domain (403 if not configured).

use gr_abi::{ModuleMeta, ModuleVTable, RUNTIME_ABI};
use gr_admin::domain::{is_business_host, normalize_hostname};
use parking_lot::RwLock;
use serde_json::json;
use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::OnceLock;

/// Stamped release version (aligned with GitHub tag / GR_RELEASE_VERSION).
pub const GR_MODULE_VERSION_PUBLIC: &str = env!("GR_MODULE_VERSION");

static META_C: OnceLock<CString> = OnceLock::new();
static ROOTS: OnceLock<RwLock<Vec<(String, String, bool)>>> = OnceLock::new();
static VT: OnceLock<ModuleVTable> = OnceLock::new();

fn roots() -> &'static RwLock<Vec<(String, String, bool)>> {
    ROOTS.get_or_init(|| RwLock::new(Vec::new()))
}

fn meta_json() -> &'static CString {
    META_C.get_or_init(|| {
        let m = ModuleMeta {
            name: "ingest".into(),
            version: GR_MODULE_VERSION_PUBLIC.into(),
            abi: RUNTIME_ABI,
            requires_major: gr_abi::PRODUCT_MAJOR,
            min_runtime_minor: 0,
            max_runtime_minor: None,
            domain: "ingest".into(),
        };
        CString::new(serde_json::to_string(&m).unwrap()).unwrap()
    })
}

extern "C" fn init(_: *const gr_abi::HostContext) -> i32 {
    0
}
extern "C" fn shutdown() -> i32 {
    0
}
extern "C" fn apply_config(json_c: *const c_char) -> i32 {
    let Some(s) = gr_abi::cstr_to_str(json_c) else {
        return -1;
    };
    apply_config_json(s)
}

/// Apply live config from runtime (static-link path; not only via .so entry).
pub fn apply_config_json(s: &str) -> i32 {
    let v: serde_json::Value = match serde_json::from_str(s) {
        Ok(v) => v,
        Err(_) => return -2,
    };
    let mut list = Vec::new();
    if let Some(sites) = v.get("sites").and_then(|x| x.as_array()) {
        for site in sites {
            let sid = site
                .get("site_id")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let enabled = site
                .get("collect_enabled")
                .and_then(|x| x.as_bool())
                .unwrap_or(true);
            if let Some(rds) = site.get("root_domains").and_then(|x| x.as_array()) {
                for r in rds {
                    if let Some(root) = r.as_str() {
                        list.push((normalize_hostname(root), sid.clone(), enabled));
                    }
                }
            }
        }
    }
    *roots().write() = list;
    0
}

/// Returns Ok(site_id) or Err("forbidden") for non-business hosts.
pub fn gate_host(host: &str) -> Result<String, &'static str> {
    let g = roots().read();
    if g.is_empty() {
        // lab open if no sites configured yet
        return Ok("_lab".into());
    }
    if !is_business_host(host, &g) {
        return Err("forbidden");
    }
    gr_admin::domain::resolve_site_for_host(host, &g).ok_or("forbidden")
}

pub fn forbidden_body() -> serde_json::Value {
    json!({"ok": false, "error": "forbidden", "code": 403, "message": "not a business domain"})
}

#[cfg(feature = "plugin")]
#[no_mangle]
pub extern "C" fn gr_module_entry() -> *const ModuleVTable {
    VT.get_or_init(|| ModuleVTable {
        meta_json: meta_json().as_ptr(),
        init: Some(init),
        shutdown: Some(shutdown),
        apply_config: Some(apply_config),
        on_event: None,
    }) as *const _
}
