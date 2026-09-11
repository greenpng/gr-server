//! Analyze domain — commercial mint + evaluate worker side.
//!
//! Hot-updatable surface (OTA):
//! - `on_event("select_device_segments", …)` → commercial device K/V mint SSOT
//! Host (`gr-service`) installs this as EXTERNAL_MINT so analyze.so can change
//! digests without restarting the runtime binary.

use gr_abi::{cstr_to_str, ModuleMeta, ModuleVTable, RUNTIME_ABI};
use serde_json::{json, Value};
use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

/// Stamped release version (aligned with GitHub tag / GR_RELEASE_VERSION).
pub const GR_MODULE_VERSION_PUBLIC: &str = env!("GR_MODULE_VERSION");

/// Event name for commercial multi-segment mint (host ↔ so contract).
pub const EVENT_SELECT_DEVICE_SEGMENTS: &str = "select_device_segments";

#[allow(dead_code)] // C-ABI entry point — called via dlopen vtable when built as .so (static-link builds see no Rust caller).
static META_C: OnceLock<CString> = OnceLock::new();
static WORKERS: AtomicUsize = AtomicUsize::new(1);
#[allow(dead_code)] // C-ABI entry point — called via dlopen vtable when built as .so (static-link builds see no Rust caller).
static VT: OnceLock<ModuleVTable> = OnceLock::new();

#[allow(dead_code)] // C-ABI entry point — called via dlopen vtable when built as .so (static-link builds see no Rust caller).
fn meta_json() -> &'static CString {
    META_C.get_or_init(|| {
        let m = ModuleMeta {
            name: "analyze".into(),
            version: GR_MODULE_VERSION_PUBLIC.into(),
            abi: RUNTIME_ABI,
            requires_major: gr_abi::PRODUCT_MAJOR,
            min_runtime_minor: 0,
            max_runtime_minor: None,
            domain: "analyze".into(),
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
    let Some(s) = cstr_to_str(json_c) else {
        return -1;
    };
    apply_config_json(s)
}

pub fn apply_config_json(s: &str) -> i32 {
    if let Ok(v) = serde_json::from_str::<Value>(s) {
        if let Some(n) = v.pointer("/workers/analyze").and_then(|x| x.as_u64()) {
            WORKERS.store((n as usize).max(1), Ordering::Relaxed);
        }
    }
    0
}

pub fn target_workers() -> usize {
    WORKERS.load(Ordering::Relaxed).max(1)
}

/// In-module mint (local path — never re-enters host EXTERNAL_MINT).
pub fn mint_device_segments(fields: &Value, evidence: Option<&Value>) -> Value {
    let mut out = gr_probe_core::device_segments::select_device_segments_local(fields, evidence);
    if let Some(obj) = out.as_object_mut() {
        obj.insert(
            "mint_source".into(),
            json!(format!("analyze_module@{}", GR_MODULE_VERSION_PUBLIC)),
        );
    }
    out
}

/// C ABI event handler.
///
/// Events:
/// - `select_device_segments` — body `{"fields":...,"evidence":...|null}` → segments JSON
///
/// Return: bytes written (>=0), -1 error, or required size if `out_cap` too small.
#[allow(dead_code)] // C-ABI entry point — called via dlopen vtable when built as .so (static-link builds see no Rust caller).
extern "C" fn on_event(
    event_c: *const c_char,
    in_ptr: *const u8,
    in_len: usize,
    out_ptr: *mut u8,
    out_cap: usize,
) -> i32 {
    let event = match cstr_to_str(event_c) {
        Some(s) => s,
        None => return -1,
    };
    if event != EVENT_SELECT_DEVICE_SEGMENTS {
        return -1;
    }
    if in_ptr.is_null() && in_len > 0 {
        return -1;
    }
    let in_slice = if in_len == 0 || in_ptr.is_null() {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(in_ptr, in_len) }
    };
    let req: Value = match serde_json::from_slice(in_slice) {
        Ok(v) => v,
        Err(_) => json!({}),
    };
    let fields = req.get("fields").cloned().unwrap_or(json!({}));
    let evidence = req.get("evidence").filter(|v| !v.is_null()).cloned();
    let result = mint_device_segments(&fields, evidence.as_ref());
    let bytes = match serde_json::to_vec(&result) {
        Ok(b) => b,
        Err(_) => return -1,
    };
    if bytes.len() > out_cap {
        // Signal required size to host grow-loop.
        return bytes.len() as i32;
    }
    if out_ptr.is_null() {
        return -1;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_ptr, bytes.len());
    }
    bytes.len() as i32
}

pub fn analyze_stub(session_id: &str) -> Value {
    json!({
        "session_id": session_id,
        "module": "analyze",
        "version": GR_MODULE_VERSION_PUBLIC,
        "status": "queued_or_bridged",
        "workers": target_workers(),
        "capabilities": ["select_device_segments"],
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
        on_event: Some(on_event),
    }) as *const _
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mint_via_module_local_marks_source() {
        let fields = json!({
            "residual_mean": 0.26,
            "hw_curve_webgl": (0..16).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>(),
            "audio_seed_delta_curve": (0..32).map(|i| 0.001 * (i as f64 + 1.0)).collect::<Vec<_>>(),
        });
        let out = mint_device_segments(&fields, None);
        assert!(out.get("device_id").and_then(|v| v.as_str()).is_some());
        let src = out.get("mint_source").and_then(|v| v.as_str()).unwrap_or("");
        assert!(src.starts_with("analyze_module@"), "src={src}");
    }
}
