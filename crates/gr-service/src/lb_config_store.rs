//! Persist LB panel config in shared admin Postgres so every
//! cluster node applies the same balancing policy.

use gr_lb::LbConfig;

pub const LB_CONFIG_SETTING_KEY: &str = "lb_config_v1";

fn admin_pg_url() -> Option<String> {
    for k in [
        "GR_ADMIN_DATABASE_URL",
        "GR_ADMIN_DATABASE_URL",
        "GR_DATABASE_URL",
    ] {
        if let Ok(u) = std::env::var(k) {
            let u = u.trim().to_string();
            if !u.is_empty() {
                return Some(u);
            }
        }
    }
    None
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

pub fn write_lb_config(cfg: &LbConfig) -> Result<(), String> {
    let Some(dsn) = admin_pg_url() else {
        return Err("admin pg url missing".into());
    };
    let mut c = postgres::Client::connect(&dsn, postgres::NoTls).map_err(|e| e.to_string())?;
    // P1-3 (iss/grok4.6/05): per-poll connection — suppress ensure NOTICE.
    let _ = c.batch_execute("SET client_min_messages = warning");
    ensure_table(&mut c)?;
    let body = serde_json::to_string(cfg).map_err(|e| e.to_string())?;
    c.execute(
        r#"
        INSERT INTO admin_settings(key, value) VALUES ($1, $2)
        ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value
        "#,
        &[&LB_CONFIG_SETTING_KEY, &body],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn read_lb_config() -> Result<Option<LbConfig>, String> {
    let Some(dsn) = admin_pg_url() else {
        return Ok(None);
    };
    let mut c = postgres::Client::connect(&dsn, postgres::NoTls).map_err(|e| e.to_string())?;
    // P1-3 (iss/grok4.6/05): per-poll connection — suppress ensure NOTICE.
    let _ = c.batch_execute("SET client_min_messages = warning");
    ensure_table(&mut c)?;
    let row = c
        .query_opt(
            "SELECT value FROM admin_settings WHERE key = $1",
            &[&LB_CONFIG_SETTING_KEY],
        )
        .map_err(|e| e.to_string())?;
    let Some(row) = row else {
        return Ok(None);
    };
    let raw: String = row.get(0);
    let cfg: LbConfig = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    Ok(Some(cfg))
}
