//! L2 shared store for multi-worker governance (iss/60 R2).
//!
//! Backends (env, first match wins):
//! - `GR_SHARED_L2_URL` / `GR_REDIS_URL` → Redis RESP (preferred for multi-worker)
//! - else file path via `shared_governance` (L1)
//!
//! Keys: `gr:sg:{name}` JSON payload. No hard dependency on redis crate —
//! minimal RESP client (same style as r100_hub).

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub const SHARED_L2_ALGO: &str = "shared_l2_redis_v1";
const KEY_PREFIX: &str = "gr:sg:";

static L2_OK: AtomicU64 = AtomicU64::new(0);
static L2_FAIL: AtomicU64 = AtomicU64::new(0);

/// Redis URL if L2 enabled.
pub fn l2_redis_url() -> Option<String> {
    for k in ["GR_SHARED_L2_URL", "GR_REDIS_URL", "REDIS_URL"] {
        if let Ok(v) = std::env::var(k) {
            let v = v.trim();
            if !v.is_empty() && !matches!(v.to_ascii_lowercase().as_str(), "0" | "off" | "false") {
                if v.starts_with("redis://") || !v.contains("://") {
                    return Some(if v.starts_with("redis://") {
                        v.to_string()
                    } else {
                        format!("redis://{v}")
                    });
                }
            }
        }
    }
    // Default lab: try local redis if GR_SHARED_L2=1 or auto when shared governance on
    match gr_abi::env::get("SHARED_L2")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "1" | "true" | "on" | "yes" | "redis" | "auto" => Some("redis://127.0.0.1:6379/0".into()),
        _ => None,
    }
}

pub fn l2_enabled() -> bool {
    l2_redis_url().is_some()
}

pub fn l2_metrics() -> Value {
    json!({
        "algo": SHARED_L2_ALGO,
        "enabled": l2_enabled(),
        "url_configured": l2_redis_url().is_some(),
        "ok": L2_OK.load(Ordering::Relaxed),
        "fail": L2_FAIL.load(Ordering::Relaxed),
    })
}

fn parse_redis_url(url: &str) -> Result<(String, u16, u8, Option<String>), String> {
    let raw = url.trim().trim_start_matches("redis://");
    let (auth, rest) = if let Some(i) = raw.rfind('@') {
        (Some(&raw[..i]), &raw[i + 1..])
    } else {
        (None, raw)
    };
    let password = auth.map(|a| a.trim_start_matches(':').to_string());
    let (hostport, db) = if let Some((hp, d)) = rest.split_once('/') {
        (hp, d.parse().unwrap_or(0))
    } else {
        (rest, 0u8)
    };
    let (host, port) = if let Some((h, p)) = hostport.split_once(':') {
        (h.to_string(), p.parse().unwrap_or(6379))
    } else {
        (hostport.to_string(), 6379u16)
    };
    Ok((host, port, db, password))
}

/// Resolve a redis host:port with getaddrinfo so docker-DNS hostnames like
/// `redis` (GR_REDIS_URL=redis://redis:6379/0) work — `SocketAddr::parse`
/// only accepts literal IPs, which made every containerized hostname fail.
fn resolve_redis_addr(host: &str, port: u16) -> Result<Vec<SocketAddr>, String> {
    format!("{host}:{port}")
        .to_socket_addrs()
        .map(|i| i.collect())
        .map_err(|e| format!("redis addr: {e}"))
}

fn redis_connect(url: &str) -> Result<TcpStream, String> {
    let (host, port, db, password) = parse_redis_url(url)?;
    let addrs = resolve_redis_addr(&host, port)?;
    let addr = addrs
        .first()
        .ok_or_else(|| format!("redis addr: {host}:{port} resolved to no addresses"))?;
    let mut stream = TcpStream::connect_timeout(addr, Duration::from_secs(2))
        .map_err(|e| format!("redis connect: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_millis(1500))).ok();
    stream.set_write_timeout(Some(Duration::from_millis(1500))).ok();
    if let Some(pw) = password {
        redis_write(&mut stream, &["AUTH", &pw])?;
        let _ = redis_read(&mut stream)?;
    }
    if db != 0 {
        let d = db.to_string();
        redis_write(&mut stream, &["SELECT", &d])?;
        let _ = redis_read(&mut stream)?;
    }
    Ok(stream)
}

fn redis_write(stream: &mut TcpStream, args: &[&str]) -> Result<(), String> {
    let mut buf = format!("*{}\r\n", args.len());
    for a in args {
        buf.push_str(&format!("${}\r\n{}\r\n", a.len(), a));
    }
    stream
        .write_all(buf.as_bytes())
        .map_err(|e| format!("redis write: {e}"))
}

fn redis_read(stream: &mut TcpStream) -> Result<String, String> {
    let mut buf = vec![0u8; 65536];
    let n = stream
        .read(&mut buf)
        .map_err(|e| format!("redis read: {e}"))?;
    if n == 0 {
        return Err("redis eof".into());
    }
    String::from_utf8(buf[..n].to_vec()).map_err(|e| format!("redis utf8: {e}"))
}

fn redis_cmd(url: &str, args: &[&str]) -> Result<String, String> {
    let mut s = redis_connect(url)?;
    redis_write(&mut s, args)?;
    redis_read(&mut s)
}

fn parse_bulk(resp: &str) -> Option<String> {
    // $+n\r\nbody\r\n or $-1
    let mut lines = resp.split("\r\n");
    let head = lines.next()?;
    if head == "$-1" || head.starts_with("-") {
        return None;
    }
    if let Some(rest) = head.strip_prefix('$') {
        let n: isize = rest.parse().ok()?;
        if n < 0 {
            return None;
        }
        let body = lines.next()?;
        return Some(body.to_string());
    }
    // simple +OK
    if head.starts_with('+') {
        return Some(head[1..].to_string());
    }
    None
}

pub fn l2_get_json(name: &str) -> Option<Value> {
    let url = l2_redis_url()?;
    let key = format!("{KEY_PREFIX}{name}");
    match redis_cmd(&url, &["GET", &key]) {
        Ok(resp) => {
            let body = parse_bulk(&resp)?;
            match serde_json::from_str(&body) {
                Ok(v) => {
                    L2_OK.fetch_add(1, Ordering::Relaxed);
                    Some(v)
                }
                Err(_) => {
                    L2_FAIL.fetch_add(1, Ordering::Relaxed);
                    None
                }
            }
        }
        Err(e) => {
            L2_FAIL.fetch_add(1, Ordering::Relaxed);
            eprintln!("[shared_l2] GET {name}: {e}");
            None
        }
    }
}

pub fn l2_set_json(name: &str, v: &Value) -> Result<(), String> {
    let url = l2_redis_url().ok_or_else(|| "l2_not_configured".to_string())?;
    let key = format!("{KEY_PREFIX}{name}");
    let body = serde_json::to_string(v).map_err(|e| e.to_string())?;
    let resp = redis_cmd(&url, &["SET", &key, &body])?;
    if resp.contains("OK") || resp.starts_with("+OK") {
        L2_OK.fetch_add(1, Ordering::Relaxed);
        Ok(())
    } else {
        L2_FAIL.fetch_add(1, Ordering::Relaxed);
        Err(format!("redis set unexpected: {resp}"))
    }
}

/// Load-merge-save via Redis when L2 enabled.
pub fn with_l2_json<R>(
    name: &str,
    default: Value,
    f: impl FnOnce(&mut Value) -> R,
) -> Option<R> {
    if !l2_enabled() {
        return None;
    }
    let mut v = l2_get_json(name).unwrap_or(default);
    if !v.is_object() {
        v = json!({});
    }
    let out = f(&mut v);
    if let Err(e) = l2_set_json(name, &v) {
        eprintln!("[shared_l2] SET {name}: {e}");
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_redis_addr_hostname_or_ip() {
        // hostname path — the failure SocketAddr::parse hit in prod (redis://redis:6379/0)
        let a = resolve_redis_addr("localhost", 6379).unwrap();
        assert!(!a.is_empty() && a.iter().any(|x| x.ip().is_loopback()));

        // literal IP path keeps prior behaviour
        let b = resolve_redis_addr("127.0.0.1", 6380).unwrap();
        assert_eq!(b[0], "127.0.0.1:6380".parse::<SocketAddr>().unwrap());

        // unresolvable name yields Err or empty set — never a panic/ugly parse error
        let r = resolve_redis_addr("no_such_gr_host.invalid", 6379);
        assert!(r.is_err() || r.unwrap().is_empty());
    }

    #[test]
    fn parse_redis_url_db_and_auth() {
        let (h, p, d, pw) = parse_redis_url("redis://:sekret@redis:6379/3").unwrap();
        assert_eq!((h.as_str(), p), ("redis", 6379));
        assert_eq!(d, 3);
        assert_eq!(pw.as_deref(), Some("sekret"));

        let (h, p, d, pw) = parse_redis_url("redis://127.0.0.1:6380").unwrap();
        assert_eq!((h.as_str(), p), ("127.0.0.1", 6380));
        assert_eq!(d, 0);
        assert_eq!(pw, None);
    }

    #[test]
    fn redis_roundtrip_if_available() {
        // Skip if no redis
        std::env::set_var("GR_SHARED_L2", "1");
        // hostname (not literal IP) — reproduces the prod container DNS path
        std::env::set_var("GR_REDIS_URL", "redis://localhost:6379/15");
        if l2_redis_url().is_none() {
            return;
        }
        let name = format!("test_rt_{}", std::process::id());
        let ok = with_l2_json(&name, json!({}), |v| {
            v.as_object_mut().unwrap().insert("k".into(), json!(42));
        });
        if ok.is_none() {
            // redis down in CI — soft skip
            return;
        }
        let g = l2_get_json(&name);
        assert_eq!(g.unwrap()["k"], 42);
        let _ = redis_cmd(
            &l2_redis_url().unwrap(),
            &["DEL", &format!("{KEY_PREFIX}{name}")],
        );
    }
}
