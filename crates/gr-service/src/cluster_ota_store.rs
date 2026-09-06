//! Persist cluster OTA desired state in shared admin Postgres so docker
//! nodes apply the same GitHub release the panel selected.

use gr_runtime::cluster_ota::{version_from_release_url, ClusterOtaDesired, CLUSTER_OTA_SETTING_KEY};
use serde_json::json;

fn admin_pg_url() -> Option<String> {
    for k in ["GR_ADMIN_DATABASE_URL", "GR_ADMIN_DATABASE_URL", "GR_DATABASE_URL"] {
        if let Ok(u) = std::env::var(k) {
            let u = u.trim().to_string();
            if !u.is_empty() {
                return Some(u);
            }
        }
    }
    None
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn ensure_table(c: &mut postgres::Client) -> Result<(), String> {
    c.batch_execute(
        r#"
        CREATE TABLE IF NOT EXISTS admin_settings (
          key TEXT PRIMARY KEY,
          value TEXT NOT NULL
        );
        "#,
    )
    .map_err(|e| e.to_string())
}

pub fn write_desired(release_url: &str, activate: bool) -> Result<ClusterOtaDesired, String> {
    let url = release_url.trim().trim_end_matches('/').to_string();
    if url.is_empty() {
        return Err("release_url empty".into());
    }
    let desired = ClusterOtaDesired {
        version: version_from_release_url(&url),
        release_url: url,
        activate,
        written_ms: now_ms(),
    };
    let Some(dsn) = admin_pg_url() else {
        return Ok(desired);
    };
    let mut c = postgres::Client::connect(&dsn, postgres::NoTls).map_err(|e| e.to_string())?;
    // P1-3 (iss/grok4.6/05): this poll runs every few seconds; keep the
    // per-connection ensure_table NOTICE out of node logs.
    let _ = c.batch_execute("SET client_min_messages = warning");
    ensure_table(&mut c)?;
    let body = serde_json::to_string(&desired).map_err(|e| e.to_string())?;
    c.execute(
        r#"
        INSERT INTO admin_settings(key, value) VALUES ($1, $2)
        ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value
        "#,
        &[&CLUSTER_OTA_SETTING_KEY, &body],
    )
    .map_err(|e| e.to_string())?;
    Ok(desired)
}

pub fn read_desired() -> Result<Option<ClusterOtaDesired>, String> {
    let Some(dsn) = admin_pg_url() else {
        return Ok(None);
    };
    let mut c = postgres::Client::connect(&dsn, postgres::NoTls).map_err(|e| e.to_string())?;
    // P1-3 (iss/grok4.6/05): this poll runs every few seconds; keep the
    // per-connection ensure_table NOTICE out of node logs.
    let _ = c.batch_execute("SET client_min_messages = warning");
    ensure_table(&mut c)?;
    let row = c.query_opt(
        "SELECT value FROM admin_settings WHERE key = $1",
        &[&CLUSTER_OTA_SETTING_KEY],
    )
    .map_err(|e| e.to_string())?;
    let Some(row) = row else {
        return Ok(None);
    };
    let raw: String = row.get(0);
    let d: ClusterOtaDesired = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    Ok(Some(d))
}

pub fn desired_status_json(current_url: &str, analyze_version: Option<&str>) -> serde_json::Value {
    match read_desired() {
        Ok(Some(d)) => json!({
            "ok": true,
            "desired": d,
            "should_apply": gr_runtime::cluster_ota::should_apply_cluster_ota(
                current_url,
                analyze_version,
                &d,
            ),
        }),
        Ok(None) => json!({"ok": true, "desired": null, "should_apply": false}),
        Err(e) => json!({"ok": false, "error": e}),
    }
}
