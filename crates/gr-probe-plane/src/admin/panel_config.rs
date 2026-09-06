//! Admin panel config surface: global + site overrides, publish, heartbeats.

use crate::admin::db::AdminDb;
use gr_probe_store::{
    cfg_from_stored, get_runtime_cfg, global_to_json, parse_global_patch, parse_site_override,
    set_runtime_cfg, sites_to_json, RuntimeCfg,
};
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

const KEY_GLOBAL: &str = "panel_config_global_json";
const KEY_SITES: &str = "panel_config_sites_json";
const KEY_VERSION: &str = "panel_config_version";
const KEY_ACTOR: &str = "panel_config_actor";
const KEY_UPDATED: &str = "panel_config_updated_ms";
const KEY_LAST_PURGE: &str = "panel_last_cold_purge_json";
const KEY_APPLY: &str = "panel_config_apply_json";
/// Per-node last-applied map: node key (worker_id/node_id) → {version, applied_ms, ok, …}.
const KEY_APPLY_NODES: &str = "panel_config_apply_nodes_json";

pub fn load_cfg_from_db(db: &AdminDb) -> RuntimeCfg {
    let global: Value = db
        .get_setting(KEY_GLOBAL)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(json!({}));
    let sites: Value = db
        .get_setting(KEY_SITES)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(json!({}));
    let version = db
        .get_setting(KEY_VERSION)
        .ok()
        .flatten()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let actor = db
        .get_setting(KEY_ACTOR)
        .ok()
        .flatten()
        .unwrap_or_else(|| "bootstrap".into());
    let updated = db
        .get_setting(KEY_UPDATED)
        .ok()
        .flatten()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    cfg_from_stored(&global, &sites, version, &actor, updated)
}

/// Apply store + core atomics + optional file overlay for multi-node readers.
pub fn apply_runtime_cfg(cfg: &RuntimeCfg) {
    set_runtime_cfg(cfg.clone());
    gr_probe_core::set_analyze_idle_upload_ms(cfg.analyze_idle_upload_ms);
    gr_probe_core::set_sdk_return_idle_ms(cfg.return_identity_idle_ms);
    gr_probe_core::set_complete_on_commercial_silicon(cfg.complete_on_commercial_silicon);
    // Mirror for workers that share admin data_dir / CONFIG path
    let body = json!({
        "panel": global_to_json(cfg),
        "sites": sites_to_json(cfg),
        "config_version": cfg.version,
        "fe_retry_policy": gr_probe_store::fe_retry_policy_json(cfg),
        "source": "admin_panel",
    });
    let _ = gr_probe_core::push_config_overlay(&body, None);
}

pub fn reload_and_apply(db: &AdminDb) -> RuntimeCfg {
    let c = load_cfg_from_db(db);
    apply_runtime_cfg(&c);
    c
}

pub fn get_config_view(db: &AdminDb) -> Value {
    let c = load_cfg_from_db(db);
    json!({
        "ok": true,
        "global": global_to_json(&c),
        "sites": sites_to_json(&c),
        "live": global_to_json(&get_runtime_cfg()),
        "defaults": global_to_json(&RuntimeCfg::default()),
    })
}

pub fn put_global(db: &AdminDb, actor: &str, patch: &Value) -> Result<Value, String> {
    let mut c = load_cfg_from_db(db);
    c = parse_global_patch(&c, patch);
    // draft only — version bumps on publish
    let g = global_to_json(&c);
    db.set_setting(KEY_GLOBAL, &g.to_string())?;
    db.set_setting(KEY_ACTOR, actor)?;
    db.set_setting(KEY_UPDATED, &now_ms().to_string())?;
    db.audit(actor, "config_global_save", "global", g.clone());
    Ok(json!({"ok": true, "global": g, "note": "saved draft; call POST config/publish to activate"}))
}

pub fn put_site(db: &AdminDb, actor: &str, site_id: &str, patch: &Value) -> Result<Value, String> {
    if site_id.trim().is_empty() {
        return Err("site_id required".into());
    }
    let mut c = load_cfg_from_db(db);
    let ov = parse_site_override(patch);
    // empty object = delete override
    let empty = patch.as_object().map(|o| o.is_empty()).unwrap_or(false)
        || (ov.cycle_cool_ms.is_none()
            && ov.cold_ttl_ms.is_none()
            && ov.analyze_idle_upload_ms.is_none()
            && ov.return_identity_idle_ms.is_none()
            && ov.rpa_idle_analyze_ms.is_none()
            && ov.hard_max_attempts.is_none()
            && ov.soft_max_attempts.is_none()
            && ov.collect_enabled.is_none());
    if empty {
        c.sites.remove(site_id);
    } else {
        c.sites.insert(site_id.to_string(), ov);
    }
    let s = sites_to_json(&c);
    db.set_setting(KEY_SITES, &s.to_string())?;
    db.set_setting(KEY_ACTOR, actor)?;
    db.set_setting(KEY_UPDATED, &now_ms().to_string())?;
    db.audit(
        actor,
        "config_site_save",
        site_id,
        json!({"site_id": site_id, "override": patch}),
    );
    Ok(json!({"ok": true, "sites": s, "note": "saved; publish to activate"}))
}

pub fn publish(db: &AdminDb, actor: &str) -> Result<Value, String> {
    let mut c = load_cfg_from_db(db);
    c.version = c.version.saturating_add(1);
    c.actor = actor.into();
    c.updated_ms = now_ms();
    let g = global_to_json(&c);
    let s = sites_to_json(&c);
    db.set_setting(KEY_GLOBAL, &g.to_string())?;
    db.set_setting(KEY_SITES, &s.to_string())?;
    db.set_setting(KEY_VERSION, &c.version.to_string())?;
    db.set_setting(KEY_ACTOR, actor)?;
    db.set_setting(KEY_UPDATED, &c.updated_ms.to_string())?;
    apply_runtime_cfg(&c);
    let node_id = gr_abi::env::get("NODE_ID")
        .unwrap_or_else(|| "local".into());
    let apply = json!({
        "node_id": node_id,
        "version": c.version,
        "ok": true,
        "applied_ms": now_ms(),
        "actor": actor,
    });
    let _ = db.set_setting(KEY_APPLY, &apply.to_string());
    let _ = note_node_applied(
        db,
        json!({
            "node_id": node_id,
            "version": c.version,
            "actor": actor,
            "source": "publish",
        }),
    );
    crate::dual_log::emit(
        crate::dual_log::Channel::System,
        "config_publish_applied",
        json!({
            "node_id": node_id,
            "version": c.version,
            "actor": actor,
        }),
    );
    db.audit(
        actor,
        "config_publish",
        "global",
        json!({"version": c.version, "global": g, "apply": apply}),
    );
    Ok(json!({
        "ok": true,
        "version": c.version,
        "global": g,
        "sites": s,
        "applied": true,
        "apply": apply,
    }))
}

/// Record a node's last successfully applied config version into the shared admin
/// DB. Key = worker_id (preferred) or node_id; entries older than 24 h are pruned
/// so the map stays bounded. Called by `publish` (acting node) and by every node's
/// periodic `spawn_config_and_heartbeat` reload.
pub fn note_node_applied(db: &AdminDb, node: Value) -> Result<(), String> {
    let mut map: Value = db
        .get_setting(KEY_APPLY_NODES)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(json!({}));
    let id = node
        .get("worker_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .or_else(|| node.get("node_id").and_then(|v| v.as_str()))
        .unwrap_or("unknown")
        .to_string();
    if let Some(obj) = map.as_object_mut() {
        let mut n = node;
        if let Some(o) = n.as_object_mut() {
            o.insert("applied_ms".into(), json!(now_ms()));
            o.insert("ok".into(), json!(true));
        }
        obj.insert(id, n);
        // prune > 24 h stale entries
        let cutoff = now_ms() - 24 * 60 * 60 * 1000;
        obj.retain(|_, v| {
            v.get("applied_ms")
                .and_then(|x| x.as_i64())
                .map(|t| t >= cutoff)
                .unwrap_or(false)
        });
    }
    db.set_setting(KEY_APPLY_NODES, &map.to_string())?;
    Ok(())
}

pub fn apply_status(db: &AdminDb) -> Value {
    let apply: Value = db
        .get_setting(KEY_APPLY)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(json!({}));
    let nodes: Value = db
        .get_setting(KEY_APPLY_NODES)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(json!({}));
    let mut list: Vec<Value> = match nodes {
        Value::Object(map) => map.into_values().collect(),
        _ => Vec::new(),
    };
    list.sort_by(|a, b| {
        let ta = a.get("applied_ms").and_then(|v| v.as_i64()).unwrap_or(0);
        let tb = b.get("applied_ms").and_then(|v| v.as_i64()).unwrap_or(0);
        tb.cmp(&ta)
    });
    let version = db
        .get_setting(KEY_VERSION)
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    json!({
        "ok": true,
        "config_version": version,
        "apply": apply,
        "applied_nodes": list,
    })
}

pub fn effective(db: &AdminDb, site_id: &str) -> Value {
    let _ = load_cfg_from_db(db);
    let c = gr_probe_store::effective_for_site(site_id);
    json!({
        "ok": true,
        "site_id": site_id,
        "effective": global_to_json(&c),
        "version": c.version,
    })
}

pub fn record_purge(db: &AdminDb, deleted: i64, ttl_ms: i64) {
    let j = json!({
        "deleted": deleted,
        "ttl_ms": ttl_ms,
        "at_ms": now_ms(),
    });
    let _ = db.set_setting(KEY_LAST_PURGE, &j.to_string());
}

pub fn last_purge(db: &AdminDb) -> Value {
    db.get_setting(KEY_LAST_PURGE)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(json!({"deleted": null, "at_ms": null}))
}

// --- node heartbeats (cluster visibility) ---

const KEY_HEARTBEATS: &str = "panel_node_heartbeats_json";

pub fn heartbeat_upsert(db: &AdminDb, node: Value) -> Result<(), String> {
    let mut map: Value = db
        .get_setting(KEY_HEARTBEATS)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(json!({}));
    let id = node
        .get("worker_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    if let Some(obj) = map.as_object_mut() {
        let mut n = node;
        if let Some(o) = n.as_object_mut() {
            o.insert("last_beat_ms".into(), json!(now_ms()));
        }
        obj.insert(id, n);
        // prune > 15 min stale
        let cutoff = now_ms() - 15 * 60 * 1000;
        obj.retain(|_, v| {
            v.get("last_beat_ms")
                .and_then(|x| x.as_i64())
                .map(|t| t >= cutoff)
                .unwrap_or(false)
        });
    }
    db.set_setting(KEY_HEARTBEATS, &map.to_string())?;
    Ok(())
}

pub fn list_heartbeats(db: &AdminDb) -> Value {
    let map: Value = db
        .get_setting(KEY_HEARTBEATS)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(json!({}));
    let mut nodes = Vec::new();
    if let Some(obj) = map.as_object() {
        for (_, v) in obj {
            nodes.push(v.clone());
        }
    }
    nodes.sort_by(|a, b| {
        let ta = a.get("last_beat_ms").and_then(|x| x.as_i64()).unwrap_or(0);
        let tb = b.get("last_beat_ms").and_then(|x| x.as_i64()).unwrap_or(0);
        tb.cmp(&ta)
    });
    json!({"ok": true, "nodes": nodes, "n": nodes.len()})
}

/// Redact DSN for system health UI.
pub fn redact_dsn(s: &str) -> String {
    if s.is_empty() {
        return "(empty)".into();
    }
    if let Some(at) = s.find('@') {
        if let Some(scheme) = s.find("://") {
            return format!("{}***:***{}", &s[..scheme + 3], &s[at..]);
        }
    }
    if s.len() > 12 {
        format!("{}…", &s[..8])
    } else {
        "(set)".into()
    }
}

pub fn storage_health_json(soft_backend: &str) -> Value {
    let greenv5 = gr_abi::env::get("DATABASE_URL").unwrap_or_default();
    let biz = gr_abi::env::get("BIZ_DATABASE_URL").unwrap_or_default();
    let assoc = gr_abi::env::get("ASSOCIATION_DATABASE_URL").unwrap_or_default();
    let redis = gr_abi::env::get("REDIS_URL").unwrap_or_default();
    json!({
        "greenv5_dsn": redact_dsn(&greenv5),
        "greenv5_configured": !greenv5.is_empty(),
        "biz_dsn": redact_dsn(&biz),
        "biz_configured": !biz.is_empty(),
        "association_dsn": redact_dsn(&assoc),
        "association_configured": !assoc.is_empty(),
        "association_multi_node": !assoc.is_empty(),
        "soft_store_backend": soft_backend,
        "redis_configured": !redis.is_empty(),
        "redis_dsn": if redis.is_empty() { Value::Null } else { json!(redact_dsn(&redis)) },
        "challenge_secret_configured": !gr_abi::env::get("CHALLENGE_SECRET").unwrap_or_default().trim().is_empty(),
        "seal_secret_configured": !gr_abi::env::get("SEAL_SECRET").unwrap_or_default().trim().is_empty(),
        "result_token_configured": !gr_abi::env::get("RESULT_TOKEN").unwrap_or_default().trim().is_empty(),
        "note": "secrets never shown in plain text",
    })
}
