//! Apply desired runtime config: SNI hot reload, worker target, restart hook.

use crate::admin::db::AdminDb;
use crate::handlers::AppState;
use crate::sni_map;
use serde_json::{json, Value};
use std::process::Command;
use std::sync::atomic::Ordering;

pub fn apply_runtime(st: &AppState, actor: &str) -> Result<Value, String> {
    let admin = st.admin.as_ref().ok_or_else(|| "admin_disabled".to_string())?;
    let db = &admin.db;
    let desired = db.get_runtime_desired()?;
    let mut notes = Vec::new();
    let mut needs_restart = false;

    // Hot: analyze workers target
    if let Some(n) = desired
        .get("analyze_workers")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
    {
        let n = n.max(1);
        st.analyze_workers_target.store(n, Ordering::Relaxed);
        notes.push(format!("analyze_workers_target={n}"));
    }

    // Hot: soft_v2 via setting reflected in env for next reads — AppState is immutable soft flag;
    // persist note only
    if let Some(s) = desired.get("soft_v2_ready").and_then(|v| v.as_bool()) {
        let _ = db.set_setting("soft_v2_ready_desired", if s { "1" } else { "0" });
        notes.push(format!("soft_v2_ready_desired={s} (restart to bind AppState)"));
        needs_restart = true;
    }

    // Hot: reload SNI from admin-generated map
    let sni_path = db.data_dir.join("sni-map.json");
    if sni_path.is_file() {
        match sni_map::load_sni_map(&sni_path.to_string_lossy()) {
            Ok(n) => notes.push(format!("sni_reloaded n={n}")),
            Err(e) => notes.push(format!("sni_reload_err={e}")),
        }
    }

    // Also upsert each domain cert
    if let Ok(domains) = db.list_domains(None, None) {
        for d in domains {
            let host = d.get("hostname").and_then(|v| v.as_str()).unwrap_or("");
            let cert = d.get("cert_path").and_then(|v| v.as_str()).unwrap_or("");
            let key = d.get("key_path").and_then(|v| v.as_str()).unwrap_or("");
            if host.is_empty() || cert.is_empty() || key.is_empty() {
                continue;
            }
            if let Err(e) = sni_map::upsert_binding(host, std::path::Path::new(cert), std::path::Path::new(key))
            {
                notes.push(format!("sni_upsert_err {host}: {e}"));
            }
        }
    }

    // Ports / role / binds require process restart
    for k in [
        "bind",
        "bind_tls",
        "admin_bind",
        "role",
        "tls_cert",
        "tls_key",
        "quic_listen",
        "h3_listen",
    ] {
        if desired.get(k).is_some() {
            needs_restart = true;
            notes.push(format!("{k}_change_requires_restart"));
        }
    }

    let mut restart_ran = false;
    let mut restart_out = Value::Null;
    if needs_restart {
        if let Some(cmd) = gr_abi::env::get("ADMIN_RESTART_CMD") {
            let cmd = cmd.trim();
            if !cmd.is_empty() {
                match Command::new("sh").arg("-c").arg(cmd).output() {
                    Ok(o) => {
                        restart_ran = true;
                        restart_out = json!({
                            "status": o.status.code(),
                            "stdout": String::from_utf8_lossy(&o.stdout),
                            "stderr": String::from_utf8_lossy(&o.stderr),
                        });
                        notes.push("restart_cmd_executed".into());
                    }
                    Err(e) => notes.push(format!("restart_cmd_err={e}")),
                }
            } else {
                notes.push("set GR_ADMIN_RESTART_CMD to auto-restart after bind/role changes".into());
            }
        } else {
            notes.push("set GR_ADMIN_RESTART_CMD to auto-restart after bind/role changes".into());
        }
    }

    db.audit(actor, "runtime_apply", "", json!({"notes": notes}));
    Ok(json!({
        "ok": true,
        "desired": desired,
        "notes": notes,
        "needs_restart": needs_restart,
        "restart_ran": restart_ran,
        "restart": restart_out,
        "sni_map_len": sni_map::sni_map_len(),
        "analyze_workers_target": st.analyze_workers_target.load(Ordering::Relaxed),
    }))
}

pub fn seed_runtime_from_live(
    db: &AdminDb,
    bind: &str,
    bind_tls: &str,
    admin_bind: &str,
    role: &str,
    analyze_workers: usize,
    soft_v2_ready: bool,
) -> Result<(), String> {
    let existing = db.get_runtime_desired()?;
    if existing.as_object().map(|o| !o.is_empty()).unwrap_or(false) {
        return Ok(());
    }
    db.set_runtime_desired(&json!({
        "bind": bind,
        "bind_tls": bind_tls,
        "admin_bind": admin_bind,
        "role": role,
        "analyze_workers": analyze_workers,
        "soft_v2_ready": soft_v2_ready,
    }))
}
