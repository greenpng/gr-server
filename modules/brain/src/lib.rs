//! Brain scheduling domain module (route_plan / mid decisions).
//! Full evaluate path bridges to v5 brain logic via service integration.

use gr_abi::{ModuleMeta, ModuleVTable, RUNTIME_ABI};
use serde_json::json;
use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

/// Stamped release version (aligned with GitHub tag / GR_RELEASE_VERSION).
pub const GR_MODULE_VERSION_PUBLIC: &str = env!("GR_MODULE_VERSION");

static META_C: OnceLock<CString> = OnceLock::new();
static PLAN_REV: AtomicU64 = AtomicU64::new(0);
static VT: OnceLock<ModuleVTable> = OnceLock::new();

fn meta_json() -> &'static CString {
    META_C.get_or_init(|| {
        let m = ModuleMeta {
            name: "brain".into(),
            version: GR_MODULE_VERSION_PUBLIC.into(),
            abi: RUNTIME_ABI,
            requires_major: gr_abi::PRODUCT_MAJOR,
            min_runtime_minor: 0,
            max_runtime_minor: None,
            domain: "brain".into(),
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
    PLAN_REV.fetch_add(1, Ordering::Relaxed);
    0
}

/// Placeholder plan — production wires gr-core brain.
pub fn schedule_plan(session_id: &str, coverage: f64) -> serde_json::Value {
    let rev = PLAN_REV.load(Ordering::Relaxed);
    json!({
        "session_id": session_id,
        "rev": rev,
        "module": "brain",
        "version": GR_MODULE_VERSION_PUBLIC,
        "actions": if coverage < 0.4 {
            vec!["static_kick", "mid_hw", "gateway_b8"]
        } else if coverage < 0.8 {
            vec!["mid_deep", "sandbox"]
        } else {
            vec!["hold"]
        }
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
