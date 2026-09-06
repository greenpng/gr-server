//! Shared rate limit for open / ingest / analyze / complete / result (per site + route).
//!
//! Multi-node: PostgreSQL `probe_rate_limit_windows` is the shared authority —
//! every node bumps the same per-minute counter, so the quota is cluster-wide.
//! SQLite / store outage falls back to a process-local window (single-node mode
//! or degraded best-effort; a DB blip must not turn into a site-wide 429).
//! Lab (`GR_REQUIRE_RESULT_TOKEN=0`) is open unless `GR_RATE_LIMIT_FORCE=1`.

use crate::handlers::AppState;
use dashmap::DashMap;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

struct Bucket {
    window_ms: AtomicU64,
    count: AtomicU64,
}

static BUCKETS: OnceLock<DashMap<String, Bucket>> = OnceLock::new();

/// Ops counters: checks performed while the limiter is enabled, and rejections.
static CHECKED: AtomicU64 = AtomicU64::new(0);
static REJECTED: AtomicU64 = AtomicU64::new(0);

fn buckets() -> &'static DashMap<String, Bucket> {
    BUCKETS.get_or_init(DashMap::new)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn now_window() -> i64 {
    (now_ms() / 60_000) as i64
}

fn limit_for(route: &str) -> u64 {
    let env_key = match route {
        "open" => "RATE_LIMIT_OPEN_PER_MIN",
        "ingest" => "RATE_LIMIT_INGEST_PER_MIN",
        "analyze" => "RATE_LIMIT_ANALYZE_PER_MIN",
        "complete" => "RATE_LIMIT_COMPLETE_PER_MIN",
        "result" => "RATE_LIMIT_RESULT_PER_MIN",
        "client_event" => "RATE_LIMIT_CLIENT_EVENT_PER_MIN",
        _ => "RATE_LIMIT_DEFAULT_PER_MIN",
    };
    gr_abi::env::get(env_key)
        .and_then(|s| s.parse().ok())
        .unwrap_or(match route {
            "open" => 120,
            "ingest" => 600,
            "analyze" => 60,
            "complete" => 60,
            "result" => 240,
            // iss/opus5 §2.4: matches the long-standing comment in
            // ops_client_event ("max 60 events/min/IP").
            "client_event" => 60,
            _ => 300,
        })
}

fn enabled() -> bool {
    if gr_abi::env::get("RATE_LIMIT_FORCE").as_deref() == Some("1") {
        return true;
    }
    if gr_abi::env::get("RATE_LIMIT_OFF").as_deref() == Some("1") {
        return false;
    }
    gr_probe_core::result_token_enforced()
}

fn rl_err(route: &str) -> String {
    format!("rate_limited:{route}")
}

/// Process-local window bucket (single-node SQLite, or fallback when the shared
/// store is unreachable). Not authoritative across processes.
fn check_local(site: &str, route: &str, cap: u64) -> Result<(), String> {
    let key = format!("{}:{}", site, route);
    let now = now_ms();
    let window = now / 60_000;
    let b = buckets().entry(key).or_insert_with(|| Bucket {
        window_ms: AtomicU64::new(window),
        count: AtomicU64::new(0),
    });
    let prev_w = b.window_ms.load(Ordering::Relaxed);
    if prev_w != window {
        b.window_ms.store(window, Ordering::Relaxed);
        b.count.store(0, Ordering::Relaxed);
    }
    let n = b.count.fetch_add(1, Ordering::Relaxed) + 1;
    if n > cap {
        return reject(route);
    }
    Ok(())
}

fn reject(route: &str) -> Result<(), String> {
    REJECTED.fetch_add(1, Ordering::Relaxed);
    Err(rl_err(route))
}

/// Cluster-wide check. PG backend bumps the shared per-minute counter; any store
/// error degrades to the local window rather than failing closed.
pub fn check(st: &AppState, site: &str, route: &str) -> Result<(), String> {
    if !enabled() {
        return Ok(());
    }
    CHECKED.fetch_add(1, Ordering::Relaxed);
    let cap = limit_for(route);
    if cap == 0 {
        return Ok(());
    }
    if st.store.backend_name() == "postgres" {
        let key = format!("{}:{}", site, route);
        match st.store.bump_rate_limit_window(&key, now_window()) {
            Ok(n) => {
                if n > cap as i64 {
                    return reject(route);
                }
                Ok(())
            }
            Err(_) => check_local(site, route, cap),
        }
    } else {
        check_local(site, route, cap)
    }
}

/// Ops view of the limiter: enabled state, per-route caps, counters, local bucket
/// count. Safe to expose to ops scope (no bucket keys).
pub fn stats() -> Value {
    let mut caps = serde_json::Map::new();
    for r in ["open", "ingest", "analyze", "complete", "result"] {
        caps.insert(r.into(), json!(limit_for(r)));
    }
    json!({
        "enabled": enabled(),
        "force": gr_abi::env::get("RATE_LIMIT_FORCE").as_deref() == Some("1"),
        "shared_backend": "postgres_probe_rate_limit_windows",
        "caps_per_min": Value::Object(caps),
        "checked": CHECKED.load(Ordering::Relaxed),
        "rejected": REJECTED.load(Ordering::Relaxed),
        "local_bucket_count": buckets().len(),
    })
}
