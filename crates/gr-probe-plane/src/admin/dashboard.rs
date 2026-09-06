//! Website business dashboard aggregates — reads **only** gr_biz (never probe store).
//!
//! Product posture: V5 admin is **config + light operational views** (sites, domains, SSL,
//! vtid/session list, whether product summary was SDK-synced). Deep algorithm data stays
//! on `/ops.html`. SDK owns data-centric consumption graphs for merchants.

use crate::admin::biz_store::BizStore;
use crate::handlers::AppState;
use serde_json::{json, Value};

fn biz(st: &AppState) -> Result<&BizStore, Value> {
    st.admin
        .as_ref()
        .map(|a| a.biz.as_ref())
        .ok_or_else(|| json!({"ok": false, "error": "biz_store_unavailable"}))
}

/// Derive light product + SDK consumption flags from `biz_visits.summary_json`.
/// Does **not** open greenv5 probe store (V5 panel stays non-data-warehouse).
pub fn enrich_visit_row(mut visit: Value) -> Value {
    let summary = visit
        .get("summary")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let source = summary
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let event = summary
        .get("event")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let device_id = summary
        .get("device_id")
        .or_else(|| summary.pointer("/product/device_id"))
        .or_else(|| summary.pointer("/device/device_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let os = summary.get("os").cloned().or_else(|| summary.pointer("/product/os").cloned());
    let br = summary.get("br").cloned().or_else(|| summary.pointer("/product/br").cloned());
    let rpa = summary.get("rpa").cloned().or_else(|| summary.pointer("/page/rpa").cloned());
    let explicit_sync = summary
        .get("sdk_synced")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let sdk_synced = explicit_sync
        || source == "backend_sync"
        || event == "backend_sync"
        || source.contains("sdk")
        || event.contains("sdk")
        || (device_id.is_some() && source != "session_open" && source != "pixel");
    let consumption = if sdk_synced {
        "sdk_synced"
    } else if device_id.is_some() || os.is_some() || br.is_some() {
        "product_partial"
    } else if source == "session_open" || source == "pixel" || source == "gateway" {
        "open_only"
    } else if !source.is_empty() {
        "biz_only"
    } else {
        "none"
    };
    if let Some(obj) = visit.as_object_mut() {
        obj.insert("sdk_synced".into(), json!(sdk_synced));
        obj.insert("consumption_state".into(), json!(consumption));
        obj.insert(
            "product_lite".into(),
            json!({
                "device_id": device_id,
                "os": os,
                "br": br,
                "rpa": rpa,
                "summary_source": source,
                "summary_event": event,
            }),
        );
        obj.insert(
            "view_policy".into(),
            json!({
                "v5_panel": "config_and_light_ops",
                "deep_probe": "/ops.html",
                "data_centric": "sdk",
                "note": "Full association / multi-session graphs are SDK responsibility"
            }),
        );
    }
    visit
}

fn enrich_visits_payload(mut v: Value) -> Value {
    if let Some(arr) = v.get_mut("visits").and_then(|a| a.as_array_mut()) {
        for row in arr.iter_mut() {
            *row = enrich_visit_row(row.clone());
        }
        let mirror = v.get("visits").cloned().unwrap_or(json!([]));
        if let Some(obj) = v.as_object_mut() {
            obj.insert("sessions".into(), mirror);
            obj.insert(
                "panel_role".into(),
                json!({
                    "surface": "v5_business_console",
                    "focus": "sites_domains_ssl_sdk_keys_light_visits",
                    "not_focus": "algorithm_warehouse",
                }),
            );
        }
    }
    v
}

pub fn facet_stats(st: &AppState, site_id: Option<&str>, limit: usize) -> Value {
    match biz(st).and_then(|b| b.facet_stats(site_id, limit).map_err(|e| json!({"ok": false, "error": e}))) {
        Ok(v) => v,
        Err(e) => e,
    }
}

pub fn list_visits(
    st: &AppState,
    facet: Option<&str>,
    site_id: Option<&str>,
    q: Option<&str>,
    limit: usize,
) -> Value {
    match biz(st).and_then(|b| {
        b.list_visits(site_id, facet, q, limit)
            .map_err(|e| json!({"ok": false, "error": e, "visits": []}))
    }) {
        Ok(v) => enrich_visits_payload(v),
        Err(e) => e,
    }
}

/// Backward-compatible name used by admin routes.
#[allow(dead_code)]
pub fn list_sessions_filtered(
    st: &AppState,
    facet: Option<&str>,
    site_id: Option<&str>,
    q: Option<&str>,
    limit: usize,
) -> Value {
    list_visits(st, facet, site_id, q, limit)
}
