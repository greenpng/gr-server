//! Soft edge backends: file (default) · store (SQLite/PG multi-worker) · Redis (optional).
//!
//! All implementations force Soft promote=false. Select via:
//!   --soft-store-backend file|store|redis
//!   GR_SOFT_STORE_BACKEND, GR_REDIS_URL

use gr_probe_core::{
    DeviceIdHeat, SoftEdge, SoftEdgeStore, SOFT_FUSE_OWNER, SOFT_MISRECALL_FUSE_DEFAULT,
    FileSoftEdgeStore,
};
use gr_probe_store::Store;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// Build soft store from backend name.
pub fn open_soft_store(
    backend: &str,
    file_dir: impl AsRef<Path>,
    store: Arc<Store>,
    redis_url: Option<&str>,
) -> Result<Arc<dyn SoftEdgeStore>, String> {
    let b = backend.trim().to_ascii_lowercase();
    match b.as_str() {
        "store" | "sqlite" | "postgres" | "pg" | "db" => {
            Ok(Arc::new(StoreSoftEdgeStore::new(store)))
        }
        "redis" => {
            let url = redis_url
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
                .or_else(|| gr_abi::env::get("REDIS_URL").filter(|s| !s.is_empty()))
                .unwrap_or_else(|| "redis://127.0.0.1:6379".into());
            Ok(Arc::new(RedisSoftEdgeStore::open(&url)?))
        }
        _ => {
            let fs = FileSoftEdgeStore::open(file_dir).map_err(|e| e)?;
            Ok(Arc::new(fs))
        }
    }
}

/// Soft edges/heat via gr-store (SQLite or PostgreSQL tables).
pub struct StoreSoftEdgeStore {
    store: Arc<Store>,
    fuse_threshold: i64,
    fuse_owner: String,
}

impl StoreSoftEdgeStore {
    pub fn new(store: Arc<Store>) -> Self {
        Self {
            store,
            fuse_threshold: SOFT_MISRECALL_FUSE_DEFAULT,
            fuse_owner: SOFT_FUSE_OWNER.into(),
        }
    }

    pub fn with_fuse(store: Arc<Store>, threshold: i64, owner: &str) -> Self {
        Self {
            store,
            fuse_threshold: threshold,
            fuse_owner: owner.into(),
        }
    }
}

impl SoftEdgeStore for StoreSoftEdgeStore {
    fn put_edge(&self, tenant: &str, edge: &SoftEdge) -> Result<(), String> {
        let n = self
            .store
            .soft_edge_count(tenant)
            .map_err(|e| e.to_string())?;
        if n >= self.fuse_threshold {
            return Err(format!(
                "soft_misrecall_fuse: tenant={tenant} edges>={} owner={}",
                self.fuse_threshold, self.fuse_owner
            ));
        }
        self.store
            .soft_edge_put(
                tenant,
                &edge.a_session,
                &edge.b_session,
                &edge.priority,
                edge.confidence,
                &edge.reason,
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn list_edges(&self, tenant: &str) -> Result<Vec<SoftEdge>, String> {
        let rows = self.store.soft_edge_list(tenant).map_err(|e| e.to_string())?;
        Ok(rows
            .iter()
            .map(|v| SoftEdge {
                a_session: v
                    .get("a_session")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                b_session: v
                    .get("b_session")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                priority: v
                    .get("priority")
                    .and_then(|x| x.as_str())
                    .unwrap_or("p1")
                    .to_string(),
                promote_to_commercial_id: false,
                confidence: v.get("confidence").and_then(|x| x.as_f64()).unwrap_or(0.0),
                reason: v
                    .get("reason")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
            })
            .collect())
    }

    fn record_device_id_sighting(
        &self,
        tenant: &str,
        device_id: &str,
        session_id: &str,
    ) -> Result<i64, String> {
        if !gr_probe_core::is_commercial_device_id(device_id) {
            return Err("heat only tracks commercial device ids".into());
        }
        self.store
            .soft_heat_record(tenant, device_id, session_id)
            .map_err(|e| e.to_string())
    }

    fn device_id_heat(&self, tenant: &str, device_id: &str) -> Result<DeviceIdHeat, String> {
        let v = self
            .store
            .soft_heat_get(tenant, device_id)
            .map_err(|e| e.to_string())?;
        let sessions: Vec<String> = v
            .get("sessions")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let n = sessions.len() as i64;
        Ok(DeviceIdHeat {
            device_id: device_id.to_string(),
            session_count: n,
            sessions,
            collision_style: n >= 2,
            soft_promote: false,
        })
    }

    fn fuse_threshold(&self) -> i64 {
        self.fuse_threshold
    }

    fn fuse_owner(&self) -> &str {
        &self.fuse_owner
    }
}

/// Minimal Redis Soft store (JSON blobs per tenant). RESP over TCP — no extra crate.
pub struct RedisSoftEdgeStore {
    host: String,
    port: u16,
    db: u8,
    password: Option<String>,
    fuse_threshold: i64,
    fuse_owner: String,
}

impl RedisSoftEdgeStore {
    pub fn open(url: &str) -> Result<Self, String> {
        // redis://[:password@]host:port[/db]
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
        let s = Self {
            host,
            port,
            db,
            password,
            fuse_threshold: SOFT_MISRECALL_FUSE_DEFAULT,
            fuse_owner: SOFT_FUSE_OWNER.into(),
        };
        // Connectivity probe (optional — soft-fail if redis down at boot? Prefer fail open to file.)
        let _ = s.cmd(&["PING"]);
        Ok(s)
    }

    fn connect(&self) -> Result<TcpStream, String> {
        let addr = format!("{}:{}", self.host, self.port);
        let mut stream = TcpStream::connect_timeout(
            &addr
                .parse()
                .map_err(|e| format!("redis addr {addr}: {e}"))?,
            Duration::from_secs(2),
        )
        .map_err(|e| format!("redis connect: {e}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .ok();
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .ok();
        if let Some(ref pw) = self.password {
            self.write_cmd(&mut stream, &["AUTH", pw])?;
            let _ = self.read_simple(&mut stream)?;
        }
        if self.db != 0 {
            let db = self.db.to_string();
            self.write_cmd(&mut stream, &["SELECT", &db])?;
            let _ = self.read_simple(&mut stream)?;
        }
        Ok(stream)
    }

    fn write_cmd(&self, stream: &mut TcpStream, args: &[&str]) -> Result<(), String> {
        let mut buf = format!("*{}\r\n", args.len());
        for a in args {
            buf.push_str(&format!("${}\r\n{}\r\n", a.len(), a));
        }
        stream
            .write_all(buf.as_bytes())
            .map_err(|e| format!("redis write: {e}"))
    }

    fn read_simple(&self, stream: &mut TcpStream) -> Result<String, String> {
        let mut buf = [0u8; 65536];
        let n = stream
            .read(&mut buf)
            .map_err(|e| format!("redis read: {e}"))?;
        Ok(String::from_utf8_lossy(&buf[..n]).to_string())
    }

    fn cmd(&self, args: &[&str]) -> Result<String, String> {
        let mut stream = self.connect()?;
        self.write_cmd(&mut stream, args)?;
        self.read_simple(&mut stream)
    }

    fn edges_key(tenant: &str) -> String {
        format!("gr:soft:edges:{tenant}")
    }
    fn heat_key(tenant: &str, device_id: &str) -> String {
        format!("gr:soft:heat:{tenant}:{device_id}")
    }

    fn get_json_array(&self, key: &str) -> Result<Vec<serde_json::Value>, String> {
        let resp = self.cmd(&["GET", key])?;
        // bulk string: $<n>\r\n<body>\r\n or $-1
        if resp.starts_with("$-1") || resp.trim() == "$-1" {
            return Ok(Vec::new());
        }
        if let Some(rest) = resp.strip_prefix('$') {
            if let Some(nl) = rest.find("\r\n") {
                let body = &rest[nl + 2..];
                let body = body.trim_end_matches("\r\n").trim_end_matches('\n');
                if body.is_empty() {
                    return Ok(Vec::new());
                }
                let v: serde_json::Value =
                    serde_json::from_str(body).map_err(|e| format!("redis json: {e}"))?;
                return Ok(v.as_array().cloned().unwrap_or_default());
            }
        }
        // Fallback: try parse whole
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(resp.trim()) {
            return Ok(v.as_array().cloned().unwrap_or_default());
        }
        Ok(Vec::new())
    }

    fn set_json_array(&self, key: &str, arr: &[serde_json::Value]) -> Result<(), String> {
        let s = serde_json::to_string(arr).map_err(|e| e.to_string())?;
        let resp = self.cmd(&["SET", key, &s])?;
        if resp.starts_with("+OK") || resp.contains("OK") {
            Ok(())
        } else {
            Err(format!("redis SET failed: {resp}"))
        }
    }
}

impl SoftEdgeStore for RedisSoftEdgeStore {
    fn put_edge(&self, tenant: &str, edge: &SoftEdge) -> Result<(), String> {
        let key = Self::edges_key(tenant);
        let mut arr = self.get_json_array(&key)?;
        if arr.len() as i64 >= self.fuse_threshold {
            return Err(format!(
                "soft_misrecall_fuse: tenant={tenant} edges>={} owner={}",
                self.fuse_threshold, self.fuse_owner
            ));
        }
        let mut e = edge.to_value();
        if let Some(obj) = e.as_object_mut() {
            obj.insert("promote_to_commercial_id".into(), serde_json::json!(false));
        }
        arr.push(e);
        self.set_json_array(&key, &arr)
    }

    fn list_edges(&self, tenant: &str) -> Result<Vec<SoftEdge>, String> {
        let arr = self.get_json_array(&Self::edges_key(tenant))?;
        Ok(arr
            .iter()
            .map(|v| SoftEdge {
                a_session: v
                    .get("a_session")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                b_session: v
                    .get("b_session")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                priority: v
                    .get("priority")
                    .and_then(|x| x.as_str())
                    .unwrap_or("p1")
                    .to_string(),
                promote_to_commercial_id: false,
                confidence: v.get("confidence").and_then(|x| x.as_f64()).unwrap_or(0.0),
                reason: v
                    .get("reason")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
            })
            .collect())
    }

    fn record_device_id_sighting(
        &self,
        tenant: &str,
        device_id: &str,
        session_id: &str,
    ) -> Result<i64, String> {
        if !gr_probe_core::is_commercial_device_id(device_id) {
            return Err("heat only tracks commercial device ids".into());
        }
        let key = Self::heat_key(tenant, device_id);
        let mut arr = self.get_json_array(&key)?;
        if !arr.iter().any(|x| x.as_str() == Some(session_id)) {
            arr.push(serde_json::json!(session_id));
        }
        self.set_json_array(&key, &arr)?;
        Ok(arr.len() as i64)
    }

    fn device_id_heat(&self, tenant: &str, device_id: &str) -> Result<DeviceIdHeat, String> {
        let arr = self.get_json_array(&Self::heat_key(tenant, device_id))?;
        let sessions: Vec<String> = arr
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect();
        let n = sessions.len() as i64;
        Ok(DeviceIdHeat {
            device_id: device_id.to_string(),
            session_count: n,
            sessions,
            collision_style: n >= 2,
            soft_promote: false,
        })
    }

    fn fuse_threshold(&self) -> i64 {
        self.fuse_threshold
    }

    fn fuse_owner(&self) -> &str {
        &self.fuse_owner
    }
}
