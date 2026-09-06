//! R100 template hub — memory catalog + optional Redis cache for pseudo-static packs.
//!
//! GET /v1/r100/pack/Rxx_spotcheck.js  → rendered JS
//! Redis keys: gr:r100:pack:{id} (JSON meta), set gr:r100:ids

use gr_probe_core::{
    is_r100_pack_id, render_register_pack_js, R100TemplateCatalog, R100_REDIS_PREFIX, R100_REDIS_SET,
};
use log::{info, warn};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Duration;

#[derive(Clone)]
pub struct R100Hub {
    catalog: Arc<RwLock<R100TemplateCatalog>>,
    redis_url: Option<String>,
    source: Arc<RwLock<String>>,
}

impl R100Hub {
    /// Load R100 templates. `static_dir` (usually `--static-dir fe`) anchors path
    /// resolution when process cwd is not the repo root (lab gateway pattern).
    pub fn load(
        templates_path: Option<&Path>,
        redis_url: Option<&str>,
        static_dir: Option<&Path>,
    ) -> Self {
        let mut source = "empty".to_string();
        let catalog = if let Some(cli) = templates_path {
            match R100TemplateCatalog::load_resolved(cli, static_dir) {
                Ok((c, path)) => {
                    source = format!("file:{}", path.display());
                    info!(
                        "r100 templates loaded packs={} from {}",
                        c.pack_count(),
                        path.display()
                    );
                    c
                }
                Err(e) => {
                    // Only WARN when every resolution path failed (true misconfig).
                    warn!("r100 templates load failed: {e}");
                    R100TemplateCatalog::load_default_with_static_dir(static_dir)
                        .map(|c| {
                            source = "file:default_search".into();
                            info!("r100 templates recovered via default_search packs={}", c.pack_count());
                            c
                        })
                        .unwrap_or_else(|e2| {
                            warn!("r100 templates default load failed: {e2}");
                            R100TemplateCatalog::default()
                        })
                }
            }
        } else {
            R100TemplateCatalog::load_default_with_static_dir(static_dir)
                .map(|c| {
                    source = "file:default_search".into();
                    info!("r100 templates loaded packs={} (default_search)", c.pack_count());
                    c
                })
                .unwrap_or_else(|e| {
                    warn!("r100 templates not loaded: {e}");
                    R100TemplateCatalog::default()
                })
        };

        let hub = Self {
            catalog: Arc::new(RwLock::new(catalog)),
            redis_url: redis_url
                .map(|s| s.to_string())
                .filter(|s| !s.trim().is_empty())
                .or_else(|| gr_abi::env::get("REDIS_URL").filter(|s| !s.is_empty())),
            source: Arc::new(RwLock::new(source)),
        };

        // Prefer Redis as hot store when configured: pull if non-empty, else seed from file.
        if hub.redis_url.is_some() {
            match hub.try_load_from_redis() {
                Ok(n) if n > 0 => {
                    if let Ok(mut s) = hub.source.write() {
                        *s = format!("redis:{n}_packs");
                    }
                    info!("r100 hub loaded {n} packs from redis");
                }
                Ok(_) | Err(_) => {
                    if let Err(e) = hub.seed_redis_from_memory() {
                        warn!("r100 redis seed skipped/failed: {e}");
                    } else {
                        let n = hub.pack_count();
                        if let Ok(mut s) = hub.source.write() {
                            *s = format!("redis_seeded_from_file:{n}");
                        }
                        info!("r100 hub seeded redis from file catalog n={n}");
                    }
                }
            }
        }

        info!(
            "r100 hub ready packs={} source={}",
            hub.pack_count(),
            hub.source_label()
        );
        hub
    }

    pub fn pack_count(&self) -> usize {
        self.catalog.read().map(|c| c.pack_count()).unwrap_or(0)
    }

    pub fn source_label(&self) -> String {
        self.source
            .read()
            .map(|s| s.clone())
            .unwrap_or_else(|_| "unknown".into())
    }

    pub fn status(&self) -> Value {
        let cat = self.catalog.read().ok();
        let mut base = cat
            .as_ref()
            .map(|c| c.status_json())
            .unwrap_or_else(|| json!({"ok": false}));
        if let Some(obj) = base.as_object_mut() {
            obj.insert("source".into(), json!(self.source_label()));
            obj.insert(
                "redis_configured".into(),
                json!(self.redis_url.is_some()),
            );
        }
        base
    }

    pub fn get_meta(&self, pack_id: &str) -> Option<Value> {
        if !is_r100_pack_id(pack_id) {
            return None;
        }
        // Redis first when configured
        if self.redis_url.is_some() {
            if let Ok(Some(meta)) = self.redis_get_meta(pack_id) {
                return Some(meta);
            }
        }
        self.catalog
            .read()
            .ok()
            .and_then(|c| c.get(pack_id).cloned())
    }

    pub fn render_js(&self, pack_id: &str) -> Result<String, String> {
        if !is_r100_pack_id(pack_id) {
            return Err(format!("invalid_pack_id:{pack_id}"));
        }
        if let Some(meta) = self.get_meta(pack_id) {
            return Ok(render_register_pack_js(pack_id, &meta));
        }
        Err(format!("unknown_r100_pack:{pack_id}"))
    }

    pub fn pick_js(&self, seed: u64) -> Result<(String, String), String> {
        let id = self
            .catalog
            .read()
            .ok()
            .and_then(|c| c.pick_pack_id(seed))
            .ok_or_else(|| "r100_catalog_empty".to_string())?;
        let js = self.render_js(&id)?;
        Ok((id, js))
    }

    pub fn seed_redis_from_memory(&self) -> Result<usize, String> {
        let url = self
            .redis_url
            .as_ref()
            .ok_or_else(|| "redis_not_configured".to_string())?;
        let cat = self
            .catalog
            .read()
            .map_err(|e| e.to_string())?
            .clone();
        if cat.pack_count() == 0 {
            return Err("empty_catalog".into());
        }
        let mut n = 0usize;
        // DEL set then SADD all ids
        let _ = redis_cmd(url, &["DEL", R100_REDIS_SET]);
        for (id, meta) in &cat.packs {
            let key = format!("{R100_REDIS_PREFIX}{id}");
            let body = serde_json::to_string(meta).map_err(|e| e.to_string())?;
            redis_cmd(url, &["SET", &key, &body])?;
            redis_cmd(url, &["SADD", R100_REDIS_SET, id])?;
            n += 1;
        }
        Ok(n)
    }

    fn try_load_from_redis(&self) -> Result<usize, String> {
        let url = self
            .redis_url
            .as_ref()
            .ok_or_else(|| "redis_not_configured".to_string())?;
        let members = redis_smembers(url, R100_REDIS_SET)?;
        if members.is_empty() {
            return Ok(0);
        }
        let mut packs = std::collections::HashMap::new();
        for id in members {
            if !is_r100_pack_id(&id) {
                continue;
            }
            if let Some(meta) = self.redis_get_meta(&id)? {
                packs.insert(id, meta);
            }
        }
        let n = packs.len();
        if n == 0 {
            return Ok(0);
        }
        let mut cat = self.catalog.write().map_err(|e| e.to_string())?;
        cat.packs = packs;
        cat.version = cat.version.max(1);
        Ok(n)
    }

    fn redis_get_meta(&self, pack_id: &str) -> Result<Option<Value>, String> {
        let url = match &self.redis_url {
            Some(u) => u,
            None => return Ok(None),
        };
        let key = format!("{R100_REDIS_PREFIX}{pack_id}");
        let resp = redis_cmd(url, &["GET", &key])?;
        parse_redis_bulk_json(&resp)
    }
}

// ── minimal RESP (shared style with soft_backends) ──────────────────────────

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

/// Resolve a redis host:port via getaddrinfo so docker-DNS hostnames like
/// `redis` (GR_REDIS_URL=redis://redis:6379/0) work — SocketAddr::parse
/// only accepts literal IPs (same fix as gr-probe-core shared_l2).
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
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(2))).ok();
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
    let mut buf = [0u8; 256 * 1024];
    let n = stream
        .read(&mut buf)
        .map_err(|e| format!("redis read: {e}"))?;
    Ok(String::from_utf8_lossy(&buf[..n]).to_string())
}

fn redis_cmd(url: &str, args: &[&str]) -> Result<String, String> {
    let mut stream = redis_connect(url)?;
    redis_write(&mut stream, args)?;
    redis_read(&mut stream)
}

fn redis_smembers(url: &str, key: &str) -> Result<Vec<String>, String> {
    let resp = redis_cmd(url, &["SMEMBERS", key])?;
    // *N\r\n$len\r\nval\r\n...
    let mut out = Vec::new();
    let mut lines = resp.split("\r\n");
    let first = lines.next().unwrap_or("");
    if !first.starts_with('*') {
        return Ok(out);
    }
    while let Some(line) = lines.next() {
        if line.starts_with('$') {
            if let Some(val) = lines.next() {
                if !val.is_empty() {
                    out.push(val.to_string());
                }
            }
        }
    }
    Ok(out)
}

fn parse_redis_bulk_json(resp: &str) -> Result<Option<Value>, String> {
    if resp.starts_with("$-1") {
        return Ok(None);
    }
    if let Some(rest) = resp.strip_prefix('$') {
        if let Some(nl) = rest.find("\r\n") {
            let body = rest[nl + 2..].trim_end_matches("\r\n").trim_end_matches('\n');
            if body.is_empty() {
                return Ok(None);
            }
            let v: Value = serde_json::from_str(body).map_err(|e| format!("redis json: {e}"))?;
            return Ok(Some(v));
        }
    }
    Ok(None)
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
    fn parse_redis_url_auth_and_db() {
        let (h, p, d, pw) = parse_redis_url("redis://:sekret@redis:6379/3").unwrap();
        assert_eq!((h.as_str(), p), ("redis", 6379));
        assert_eq!(d, 3);
        assert_eq!(pw.as_deref(), Some("sekret"));
    }
}
