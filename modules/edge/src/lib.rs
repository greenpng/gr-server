//! Edge / gateway domain (early B8 path). Side-channel listen stays in service bridge.

use gr_abi::{ModuleMeta, ModuleVTable, RUNTIME_ABI};
use serde_json::json;
use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::OnceLock;

/// Stamped release version (aligned with GitHub tag / GR_RELEASE_VERSION).
pub const GR_MODULE_VERSION_PUBLIC: &str = env!("GR_MODULE_VERSION");

static META_C: OnceLock<CString> = OnceLock::new();
static VT: OnceLock<ModuleVTable> = OnceLock::new();

fn meta_json() -> &'static CString {
    META_C.get_or_init(|| {
        let m = ModuleMeta {
            name: "edge".into(),
            version: GR_MODULE_VERSION_PUBLIC.into(),
            abi: RUNTIME_ABI,
            requires_major: gr_abi::PRODUCT_MAJOR,
            min_runtime_minor: 0,
            max_runtime_minor: None,
            domain: "edge".into(),
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
extern "C" fn apply_config(_: *const c_char) -> i32 {
    0
}

pub fn gateway_early_ack(session_hint: &str) -> serde_json::Value {
    json!({
        "ok": true,
        "module": "edge",
        "path": "/v1/gateway/early",
        "session_hint": session_hint,
        "version": GR_MODULE_VERSION_PUBLIC,
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
