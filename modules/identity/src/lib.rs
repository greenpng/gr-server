//! Identity + per-site crypto matrix (business domain: identity).
//! Strengthens v5 challenge/seal isolation per site.

use gr_abi::{ModuleMeta, ModuleVTable, RUNTIME_ABI};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::OnceLock;

/// Stamped release version (aligned with GitHub tag / GR_RELEASE_VERSION).
pub const GR_MODULE_VERSION_PUBLIC: &str = env!("GR_MODULE_VERSION");

static META_C: OnceLock<CString> = OnceLock::new();
static STATE: OnceLock<RwLock<IdentityState>> = OnceLock::new();

#[derive(Default)]
struct IdentityState {
    /// site_id → material
    sites: HashMap<String, SiteCrypto>,
    global_challenge: String,
    global_seal: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct SiteCrypto {
    challenge_secret: String,
    seal_secret: String,
    suite: String,
}

fn state() -> &'static RwLock<IdentityState> {
    STATE.get_or_init(|| RwLock::new(IdentityState::default()))
}

fn meta_json() -> &'static CString {
    META_C.get_or_init(|| {
        let m = ModuleMeta {
            name: "identity".into(),
            version: GR_MODULE_VERSION_PUBLIC.into(),
            abi: RUNTIME_ABI,
            requires_major: gr_abi::PRODUCT_MAJOR,
            min_runtime_minor: 0,
            max_runtime_minor: None,
            domain: "identity".into(),
        };
        CString::new(serde_json::to_string(&m).unwrap()).unwrap()
    })
}

extern "C" fn init(_ctx: *const gr_abi::HostContext) -> i32 {
    0
}

extern "C" fn shutdown() -> i32 {
    0
}

extern "C" fn apply_config(json: *const c_char) -> i32 {
    let Some(s) = gr_abi::cstr_to_str(json) else {
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
    if let Some(arr) = v.get("sites").and_then(|x| x.as_array()) {
        st.sites.clear();
        for site in arr {
            let id = site.get("site_id").and_then(|x| x.as_str()).unwrap_or("");
            if id.is_empty() {
                continue;
            }
            let crypto = site.get("crypto").cloned().unwrap_or_default();
            st.sites.insert(
                id.to_string(),
                SiteCrypto {
                    challenge_secret: crypto
                        .get("challenge_secret")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    seal_secret: crypto
                        .get("seal_secret")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    suite: crypto
                        .get("suite")
                        .and_then(|x| x.as_str())
                        .unwrap_or("gr-seal-v2")
                        .to_string(),
                },
            );
        }
    }
    0
}

/// Derive site-isolated key material (never share across sites).
pub fn site_seal_key(site_id: &str, global_fallback: &str) -> String {
    let st = state().read();
    let base = st
        .sites
        .get(site_id)
        .map(|c| {
            if c.seal_secret.is_empty() {
                c.challenge_secret.clone()
            } else {
                c.seal_secret.clone()
            }
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| global_fallback.to_string());
    let mut h = Sha256::new();
    h.update(b"gr-site-seal-v2|");
    h.update(site_id.as_bytes());
    h.update(b"|");
    h.update(base.as_bytes());
    hex::encode(h.finalize())
}

pub fn site_challenge_key(site_id: &str, global_fallback: &str) -> String {
    let st = state().read();
    let base = st
        .sites
        .get(site_id)
        .map(|c| c.challenge_secret.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| global_fallback.to_string());
    let mut h = Sha256::new();
    h.update(b"gr-site-challenge-v2|");
    h.update(site_id.as_bytes());
    h.update(b"|");
    h.update(base.as_bytes());
    hex::encode(h.finalize())
}

static VT: OnceLock<ModuleVTable> = OnceLock::new();

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolation() {
        let a = site_seal_key("s1", "global");
        let b = site_seal_key("s2", "global");
        assert_ne!(a, b);
    }
}
