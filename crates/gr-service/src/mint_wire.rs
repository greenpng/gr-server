//! Wire commercial device mint through hot-loaded `analyze.so` when available.
//!
//! Flow:
//! 1. Runtime loads analyze module (boot active tree / OTA activate).
//! 2. If module exports `on_event`, host installs EXTERNAL_MINT provider that
//!    marshals JSON → so → `select_device_segments_local` inside the so.
//! 3. `gr_probe_core::select_device_segments` (used by evaluate) then hits so.
//! 4. Future algorithm OTA = only re-ship analyze.so + panel activate.

use gr_module_analyze::EVENT_SELECT_DEVICE_SEGMENTS;
use gr_runtime::Runtime;
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::{info, warn};

/// Install or clear EXTERNAL_MINT based on currently loaded analyze module.
pub fn wire_analyze_mint(rt: &Arc<Runtime>) {
    let Some(m) = rt.registry.get("analyze") else {
        gr_probe_core::device_segments::clear_device_segments_provider();
        info!("mint_wire: no analyze module loaded — using runtime-local mint");
        return;
    };
    if !m.has_on_event() {
        gr_probe_core::device_segments::clear_device_segments_provider();
        warn!(
            version = m.version(),
            "mint_wire: analyze.so has no on_event — using runtime-local mint"
        );
        return;
    }
    let mod_arc = m.clone();
    let ver = m.version().to_string();
    gr_probe_core::device_segments::install_device_segments_provider(move |fields, evidence| {
        let body = json!({
            "fields": fields,
            "evidence": evidence,
        });
        let payload = match serde_json::to_vec(&body) {
            Ok(b) => b,
            Err(_) => {
                return gr_probe_core::device_segments::select_device_segments_local(
                    fields, evidence,
                );
            }
        };
        match mod_arc.on_event(EVENT_SELECT_DEVICE_SEGMENTS, &payload) {
            Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                Ok(v) => v,
                Err(e) => {
                    warn!(error = %e, "mint_wire: bad JSON from analyze.so — local fallback");
                    gr_probe_core::device_segments::select_device_segments_local(fields, evidence)
                }
            },
            Err(e) => {
                warn!(error = %e, "mint_wire: on_event failed — local fallback");
                gr_probe_core::device_segments::select_device_segments_local(fields, evidence)
            }
        }
    });
    info!(
        version = %ver,
        path = %m.path.display(),
        "mint_wire: commercial mint routed via analyze.so (OTA-capable)"
    );
}

/// Status snapshot for admin/health.
pub fn mint_wire_status(rt: &Arc<Runtime>) -> Value {
    let mod_info = rt.registry.get("analyze").map(|m| {
        json!({
            "loaded": true,
            "version": m.version(),
            "path": m.path,
            "has_on_event": m.has_on_event(),
        })
    });
    json!({
        "provider_active": gr_probe_core::device_segments::device_segments_provider_active(),
        "analyze": mod_info.unwrap_or_else(|| json!({"loaded": false})),
        "fallback": "runtime_local_select_device_segments",
    })
}
