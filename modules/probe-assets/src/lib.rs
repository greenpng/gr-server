//! Probe FE assets module: fixed entry filename + versioned subresources + cache meta.

use gr_abi::{ModuleMeta, ModuleVTable, RUNTIME_ABI};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::OnceLock;

/// Stamped release version (aligned with GitHub tag / GR_RELEASE_VERSION).
pub const GR_MODULE_VERSION_PUBLIC: &str = env!("GR_MODULE_VERSION");

#[allow(dead_code)] // C-ABI entry point — called via dlopen vtable when built as .so (static-link builds see no Rust caller).
static META_C: OnceLock<CString> = OnceLock::new();
static STATE: OnceLock<RwLock<AssetsState>> = OnceLock::new();
#[allow(dead_code)] // C-ABI entry point — called via dlopen vtable when built as .so (static-link builds see no Rust caller).
static VT: OnceLock<ModuleVTable> = OnceLock::new();

#[derive(Default)]
struct AssetsState {
    /// site_id → entry filename (default g.js)
    entry: HashMap<String, String>,
    /// site_id → fe_load mode
    fe_load: HashMap<String, String>,
    asset_version: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AssetResolve {
    pub entry_name: String,
    pub fe_load: String,
    pub versioned_prefix: String,
    pub cache_control_entry: String,
    pub cache_control_versioned: String,
}

fn state() -> &'static RwLock<AssetsState> {
    STATE.get_or_init(|| {
        RwLock::new(AssetsState {
            entry: HashMap::new(),
            fe_load: HashMap::new(),
            asset_version: GR_MODULE_VERSION_PUBLIC.to_string(),
        })
    })
}

#[allow(dead_code)] // C-ABI entry point — called via dlopen vtable when built as .so (static-link builds see no Rust caller).
fn meta_json() -> &'static CString {
    META_C.get_or_init(|| {
        let m = ModuleMeta {
            name: "probe_assets".into(),
            version: GR_MODULE_VERSION_PUBLIC.into(),
            abi: RUNTIME_ABI,
            requires_major: gr_abi::PRODUCT_MAJOR,
            min_runtime_minor: 0,
            max_runtime_minor: None,
            domain: "probe_assets".into(),
        };
        CString::new(serde_json::to_string(&m).unwrap()).unwrap()
    })
}

#[allow(dead_code)] // C-ABI entry point — called via dlopen vtable when built as .so (static-link builds see no Rust caller).
extern "C" fn init(_: *const gr_abi::HostContext) -> i32 {
    0
}
#[allow(dead_code)] // C-ABI entry point — called via dlopen vtable when built as .so (static-link builds see no Rust caller).
extern "C" fn shutdown() -> i32 {
    0
}
#[allow(dead_code)] // C-ABI entry point — called via dlopen vtable when built as .so (static-link builds see no Rust caller).
extern "C" fn apply_config(json_c: *const c_char) -> i32 {
    let Some(s) = gr_abi::cstr_to_str(json_c) else {
        return -1;
    };
    apply_config_json(s)
}

pub fn apply_config_json(s: &str) -> i32 {
    let v: serde_json::Value = match serde_json::from_str(s) {
        Ok(v) => v,
        Err(_) => return -2,
    };
    let mut st = state().write();
    if let Some(sites) = v.get("sites").and_then(|x| x.as_array()) {
        st.entry.clear();
        st.fe_load.clear();
        for site in sites {
            let id = site.get("site_id").and_then(|x| x.as_str()).unwrap_or("");
            if id.is_empty() {
                continue;
            }
            let entry = site
                .get("entry_js")
                .and_then(|x| x.as_str())
                .unwrap_or("g.js");
            let fe = site
                .get("fe_load")
                .and_then(|x| x.as_str())
                .unwrap_or("first_party");
            st.entry.insert(id.to_string(), entry.to_string());
            st.fe_load.insert(id.to_string(), fe.to_string());
        }
    }
    0
}

pub fn resolve_for_site(site_id: &str) -> AssetResolve {
    let st = state().read();
    let entry_name = st
        .entry
        .get(site_id)
        .cloned()
        .unwrap_or_else(|| "g.js".into());
    let fe_load = st
        .fe_load
        .get(site_id)
        .cloned()
        .unwrap_or_else(|| "first_party".into());
    AssetResolve {
        entry_name,
        fe_load,
        versioned_prefix: format!("v/{}", st.asset_version),
        // fixed entry: revalidate; versioned: immutable
        cache_control_entry: "no-cache".into(),
        cache_control_versioned: "public, max-age=31536000, immutable".into(),
    }
}

pub fn manifest_json() -> serde_json::Value {
    let st = state().read();
    json!({
        "module": "probe_assets",
        "asset_version": st.asset_version,
        "sites": st.entry.keys().collect::<Vec<_>>(),
    })
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
