//! iss/38 D-15: service-level self capability bitmap (not session envelope).

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::contracts::{find_spec_dir, ContractError};

static CAP: OnceLock<Result<Value, String>> = OnceLock::new();

pub fn load_self_capability() -> Result<&'static Value, ContractError> {
    let r = CAP.get_or_init(|| {
        let path = PathBuf::from(find_spec_dir()).join("self_capability.json");
        let raw = fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        serde_json::from_str(&raw).map_err(|e| format!("parse self_capability: {e}"))
    });
    r.as_ref().map_err(|e| ContractError::Msg(e.clone()))
}

pub fn self_capability_json() -> Value {
    load_self_capability()
        .cloned()
        .unwrap_or_else(|_| {
            serde_json::json!({
                "algo": "self_capability_v1",
                "loaded": false,
                "capabilities": {}
            })
        })
}
