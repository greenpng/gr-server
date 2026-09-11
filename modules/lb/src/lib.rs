//! Paid multi-node load-balancer module (docs/guides/08-LB-MODULE.md).
//!
//! 私域分发模块: NOT part of public releases. The module itself is an
//! entitlement gate — the actual balancing engine lives in the runtime
//! (crates/gr-lb + gr-probe-plane ProxyMode::Lb). If a copy of this .so
//! leaks, it still refuses to run without a signed `lb.enabled` token:
//!   - `on_event("gate", {"token": ..., "pubkey_hex": ...})` verifies the
//!     official `grlic1.*` token and grants the module in-process.
//!   - `on_event("status", {})` reports grant state (panel shows it).
//!   - Boot-time license check in gr-runtime refuses to activate the module
//!     when no verified system token exists.

use gr_abi::{ModuleMeta, ModuleVTable, RUNTIME_ABI};
use gr_probe_core::license_token::{verify_token, LicenseState};
use serde::{Deserialize, Serialize};
use std::ffi::CString;
use std::os::raw::{c_char, c_uchar};
use std::sync::{Mutex, OnceLock};

/// Stamped release version (aligned with GitHub tag / GR_RELEASE_VERSION).
pub const GR_MODULE_VERSION_PUBLIC: &str = env!("GR_MODULE_VERSION");

#[allow(dead_code)] // C-ABI entry point — called via dlopen vtable when built as .so (static-link builds see no Rust caller).
static META_C: OnceLock<CString> = OnceLock::new();
#[allow(dead_code)] // C-ABI entry point — called via dlopen vtable when built as .so (static-link builds see no Rust caller).
static VT: OnceLock<ModuleVTable> = OnceLock::new();

#[derive(Debug, Default)]
#[allow(dead_code)] // lb entitlement gate machinery — live only via the dlopen'd on_event ("gate"/"status") path.
struct LbGateState {
    granted: bool,
    license_id: String,
    modes: Vec<String>,
    expiry_ms: i64,
}
#[allow(dead_code)] // lb entitlement gate machinery — live only via the dlopen'd on_event ("gate"/"status") path.
static GATE: Mutex<Option<LbGateState>> = Mutex::new(None);

#[allow(dead_code)] // C-ABI entry point — called via dlopen vtable when built as .so (static-link builds see no Rust caller).
fn meta_json() -> &'static CString {
    META_C.get_or_init(|| {
        let m = ModuleMeta {
            name: "lb".into(),
            version: GR_MODULE_VERSION_PUBLIC.into(),
            abi: RUNTIME_ABI,
            requires_major: gr_abi::PRODUCT_MAJOR,
            min_runtime_minor: 0,
            max_runtime_minor: None,
            domain: "lb".into(),
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
extern "C" fn apply_config(_: *const c_char) -> i32 {
    0
}

#[derive(Deserialize)]
#[allow(dead_code)] // lb entitlement gate machinery — live only via the dlopen'd on_event ("gate"/"status") path.
struct GateReq {
    token: String,
    /// Optional lab/ops override key (hex 64). Production builds embed the
    /// official key via GR_LICENSE_PUBKEY_B64 at build time instead.
    #[serde(default)]
    pubkey_hex: String,
}

#[derive(Serialize)]
#[allow(dead_code)] // lb entitlement gate machinery — live only via the dlopen'd on_event ("gate"/"status") path.
struct GateResp {
    granted: bool,
    license_id: String,
    modes: Vec<String>,
    expiry_ms: i64,
    state: String,
}

#[allow(dead_code)] // lb entitlement gate machinery — live only via the dlopen'd on_event ("gate"/"status") path.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[allow(dead_code)] // lb entitlement gate machinery — live only via the dlopen'd on_event ("gate"/"status") path.
fn resolve_pubkey(hex_override: &str) -> Option<[u8; 32]> {
    if !hex_override.is_empty() {
        let t = hex_override.trim();
        if t.len() == 64 && t.chars().all(|c| c.is_ascii_hexdigit()) {
            let mut a = [0u8; 32];
            for i in 0..32 {
                a[i] = u8::from_str_radix(&t[i * 2..i * 2 + 2], 16).ok()?;
            }
            return Some(a);
        }
        return None;
    }
    gr_probe_core::license_token::license_pubkey(None)
}

#[allow(dead_code)] // lb entitlement gate machinery — live only via the dlopen'd on_event ("gate"/"status") path.
fn gate(token: &str, pubkey_hex: &str) -> GateResp {
    let pk = match resolve_pubkey(pubkey_hex) {
        Some(pk) => pk,
        None => {
            let mut g = GATE.lock().unwrap();
            if let Some(s) = g.as_mut() {
                s.granted = false;
                s.expiry_ms = 0;
            }
            return GateResp {
                granted: false,
                license_id: String::new(),
                modes: Vec::new(),
                expiry_ms: 0,
                state: "no_pubkey".into(),
            };
        }
    };
    let mut st = GATE.lock().unwrap();
    let (granted, license_id, modes, expiry_ms, state) = match verify_token(token, &pk, now_ms()) {
        Ok(v) => {
            let ent = v.claims.lb.unwrap_or_default();
            (
                ent.enabled,
                v.claims.license_id.clone(),
                ent.modes,
                v.claims.exp_ms,
                match v.state {
                    LicenseState::Active => "active",
                    LicenseState::OfflineGrace => "offline_grace",
                }
                .to_string(),
            )
        }
        Err(_) => (false, String::new(), Vec::new(), 0, "rejected".into()),
    };
    let g = st.get_or_insert_with(LbGateState::default);
    g.granted = granted;
    g.license_id = license_id.clone();
    g.modes = modes.clone();
    // Never keep a grant alive past the grace deadline: recompute handles it,
    // but also zero out when the token is dead.
    g.expiry_ms = if granted { expiry_ms } else { 0 };
    GateResp {
        granted,
        license_id,
        modes,
        expiry_ms,
        state,
    }
}

#[allow(dead_code)] // lb entitlement gate machinery — live only via the dlopen'd on_event ("gate"/"status") path.
fn status() -> GateResp {
    let st = GATE.lock().unwrap();
    match st.as_ref() {
        Some(g) => GateResp {
            granted: g.granted,
            license_id: g.license_id.clone(),
            modes: g.modes.clone(),
            expiry_ms: g.expiry_ms,
            state: if g.granted { "granted" } else { "denied" }.into(),
        },
        None => GateResp {
            granted: false,
            license_id: String::new(),
            modes: Vec::new(),
            expiry_ms: 0,
            state: "never_gated".into(),
        },
    }
}

#[allow(dead_code)] // C-ABI entry point — called via dlopen vtable when built as .so (static-link builds see no Rust caller).
extern "C" fn on_event(
    event_c: *const c_char,
    payload: *const c_uchar,
    payload_len: usize,
    out: *mut c_uchar,
    out_len: usize,
) -> i32 {
    let event = match gr_abi::cstr_to_str(event_c) {
        Some(e) => e.to_string(),
        None => return -2,
    };
    let bytes = if payload.is_null() || payload_len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(payload, payload_len) }.to_vec()
    };
    let resp = match event.as_str() {
        "gate" => match serde_json::from_slice::<GateReq>(&bytes) {
            Ok(req) => gate(&req.token, &req.pubkey_hex),
            Err(_) => {
                let mut g = GATE.lock().unwrap();
                if let Some(s) = g.as_mut() {
                    s.granted = false;
                }
                GateResp {
                    granted: false,
                    license_id: String::new(),
                    modes: Vec::new(),
                    expiry_ms: 0,
                    state: "bad_gate_payload".into(),
                }
            }
        },
        "status" => status(),
        _other => {
            return -1; // unknown event
        }
    };
    let out_buf = unsafe { std::slice::from_raw_parts_mut(out, out_len) };
    match serde_json::to_vec(&resp) {
        Ok(data) if data.len() <= out_buf.len() => {
            out_buf[..data.len()].copy_from_slice(&data);
            data.len() as i32
        }
        _ => -3,
    }
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
