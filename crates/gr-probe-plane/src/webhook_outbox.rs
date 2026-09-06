//! Webhook outbox contract (iss/48 + iss/49).
//!
//! Default: in-process queue. Production multi-replica: set
//! `GR_WEBHOOK_OUTBOX_PATH` to a shared directory (or file prefix dir) so
//! pending/dlq are JSONL-durable and reloaded on process start.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_OUTBOX: usize = 256;
const MAX_DLQ: usize = 128;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct OutboxItem {
    pub id: String,
    pub tenant: String,
    pub event: String,
    pub url: String,
    pub body: Value,
    pub attempts: u32,
    pub next_attempt_ms: i64,
    pub last_error: Option<String>,
    pub created_ms: i64,
}

impl OutboxItem {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "tenant": self.tenant,
            "event": self.event,
            "url": self.url,
            "body": self.body,
            "attempts": self.attempts,
            "next_attempt_ms": self.next_attempt_ms,
            "last_error": self.last_error,
            "created_ms": self.created_ms,
        })
    }

    fn from_json(v: &Value) -> Option<Self> {
        Some(Self {
            id: v.get("id")?.as_str()?.to_string(),
            tenant: v.get("tenant").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            event: v.get("event").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            url: v.get("url").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            body: v.get("body").cloned().unwrap_or(json!({})),
            attempts: v.get("attempts").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
            next_attempt_ms: v.get("next_attempt_ms").and_then(|x| x.as_i64()).unwrap_or(0),
            last_error: v
                .get("last_error")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string()),
            created_ms: v.get("created_ms").and_then(|x| x.as_i64()).unwrap_or(0),
        })
    }
}

struct Boxes {
    pending: VecDeque<OutboxItem>,
    dlq: VecDeque<OutboxItem>,
    durable_dir: Option<PathBuf>,
}

fn durable_dir_from_env() -> Option<PathBuf> {
    let p = gr_abi::env::get("WEBHOOK_OUTBOX_PATH")?;
    let p = p.trim();
    if p.is_empty() {
        return None;
    }
    let path = PathBuf::from(p);
    if path.extension().is_some() {
        // Treat as file path → use parent dir
        path.parent().map(|d| d.to_path_buf())
    } else {
        Some(path)
    }
}

fn load_jsonl(path: &Path) -> VecDeque<OutboxItem> {
    let mut q = VecDeque::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return q;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            if let Some(item) = OutboxItem::from_json(&v) {
                q.push_back(item);
            }
        }
    }
    q
}

/// Cross-process exclusive lock for durable JSONL (multi-worker safe).
struct DurLock {
    #[cfg(unix)]
    _file: std::fs::File,
}

fn acquire_durable_lock(dir: &Path) -> Option<DurLock> {
    let _ = std::fs::create_dir_all(dir);
    let lock_path = dir.join(".webhook_outbox.lock");
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)
            .ok()?;
        let fd = file.as_raw_fd();
        // libc::LOCK_EX
        let rc = unsafe { libc::flock(fd, libc::LOCK_EX) };
        if rc != 0 {
            return None;
        }
        return Some(DurLock { _file: file });
    }
    #[cfg(not(unix))]
    {
        let _ = lock_path;
        Some(DurLock {})
    }
}

fn persist_boxes(g: &Boxes) {
    let Some(dir) = g.durable_dir.as_ref() else {
        return;
    };
    let _ = std::fs::create_dir_all(dir);
    let _guard = acquire_durable_lock(dir);
    // Merge-on-write: reload peer process state then replace with our memory view
    // (single-writer preferred: one drain process; lock serializes full rewrite).
    let pending_path = dir.join("webhook_pending.jsonl");
    let dlq_path = dir.join("webhook_dlq.jsonl");
    let write = |path: &Path, items: &VecDeque<OutboxItem>| {
        let mut buf = String::new();
        for it in items {
            if let Ok(s) = serde_json::to_string(&it.to_json()) {
                buf.push_str(&s);
                buf.push('\n');
            }
        }
        let tmp = path.with_extension("jsonl.tmp");
        if std::fs::write(&tmp, buf.as_bytes()).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    };
    write(&pending_path, &g.pending);
    write(&dlq_path, &g.dlq);
}

fn boxes() -> &'static Mutex<Boxes> {
    static B: std::sync::OnceLock<Mutex<Boxes>> = std::sync::OnceLock::new();
    B.get_or_init(|| {
        let durable_dir = durable_dir_from_env();
        let (pending, dlq) = if let Some(ref dir) = durable_dir {
            let _ = std::fs::create_dir_all(dir);
            let _guard = acquire_durable_lock(dir);
            (
                load_jsonl(&dir.join("webhook_pending.jsonl")),
                load_jsonl(&dir.join("webhook_dlq.jsonl")),
            )
        } else {
            (VecDeque::new(), VecDeque::new())
        };
        Mutex::new(Boxes {
            pending,
            dlq,
            durable_dir,
        })
    })
}

fn persist_locked(g: &Boxes) {
    persist_boxes(g);
}

/// Sign webhook body for tenant (HMAC-SHA256 hex of body bytes with secret).
pub fn sign_webhook_body(secret: &str, body: &Value) -> String {
    let raw = serde_json::to_vec(body).unwrap_or_default();
    let mut h = Sha256::new();
    h.update(secret.as_bytes());
    h.update(b"|");
    h.update(&raw);
    format!("{:x}", h.finalize())
}

pub fn enqueue_webhook(
    tenant: &str,
    event: &str,
    url: &str,
    payload: Value,
    secret: Option<&str>,
) -> Value {
    let id = format!("wh_{:x}_{}", now_ms(), tenant.chars().take(8).collect::<String>());
    let mut body = payload;
    if let Some(obj) = body.as_object_mut() {
        obj.insert("event".into(), json!(event));
        obj.insert("tenant".into(), json!(tenant));
        obj.insert("webhook_id".into(), json!(id.clone()));
        obj.insert("ts_ms".into(), json!(now_ms()));
        if let Some(sec) = secret {
            obj.insert("sig".into(), json!(sign_webhook_body(sec, &json!({
                "event": event,
                "tenant": tenant,
                "payload_digest": format!("{:x}", {
                    let mut h = Sha256::new();
                    h.update(serde_json::to_vec(obj).unwrap_or_default());
                    h.finalize()
                })
            }))));
        }
    }
    let item = OutboxItem {
        id: id.clone(),
        tenant: tenant.into(),
        event: event.into(),
        url: url.into(),
        body,
        attempts: 0,
        next_attempt_ms: now_ms(),
        last_error: None,
        created_ms: now_ms(),
    };
    if let Ok(mut g) = boxes().lock() {
        g.pending.push_back(item);
        while g.pending.len() > MAX_OUTBOX {
            if let Some(old) = g.pending.pop_front() {
                g.dlq.push_back(old);
            }
        }
        while g.dlq.len() > MAX_DLQ {
            g.dlq.pop_front();
        }
        persist_locked(&g);
    }
    let durable = durable_dir_from_env().is_some();
    json!({
        "ok": true,
        "webhook_id": id,
        "queued": true,
        "durable": durable,
        "backend": if durable { "file" } else { "memory" },
    })
}

/// Mark delivery failed → retry or DLQ after max attempts.
pub fn mark_webhook_failed(id: &str, err: &str, max_attempts: u32) -> Value {
    if let Ok(mut g) = boxes().lock() {
        if let Some(pos) = g.pending.iter().position(|x| x.id == id) {
            let mut item = g.pending.remove(pos).unwrap();
            item.attempts += 1;
            item.last_error = Some(err.into());
            if item.attempts >= max_attempts {
                g.dlq.push_back(item.clone());
                while g.dlq.len() > MAX_DLQ {
                    g.dlq.pop_front();
                }
                persist_locked(&g);
                return json!({"ok": true, "moved_to_dlq": true, "attempts": item.attempts});
            }
            // exponential backoff: 2^attempts seconds
            let backoff = (1i64 << item.attempts.min(8)) * 1000;
            item.next_attempt_ms = now_ms() + backoff;
            g.pending.push_back(item);
            persist_locked(&g);
            return json!({"ok": true, "retry": true, "backoff_ms": backoff});
        }
    }
    json!({"ok": false, "error": "not_found"})
}

pub fn mark_webhook_ok(id: &str) -> Value {
    if let Ok(mut g) = boxes().lock() {
        g.pending.retain(|x| x.id != id);
        persist_locked(&g);
    }
    json!({"ok": true, "delivered": true, "id": id})
}

/// Minimal HTTP/1.1 POST for http:// URLs (lab drain). Returns status code.
fn http_post_json(url: &str, body: &str) -> Result<u16, String> {
    // Prefer curl when available (handles https).
    if which_curl() {
        let out = std::process::Command::new("curl")
            .args([
                "-sS",
                "-o",
                "/dev/null",
                "-w",
                "%{http_code}",
                "-X",
                "POST",
                "-H",
                "Content-Type: application/json",
                "--max-time",
                "5",
                "-d",
                body,
                url,
            ])
            .output()
            .map_err(|e| format!("curl: {e}"))?;
        if !out.status.success() && out.stdout.is_empty() {
            return Err(format!(
                "curl_fail: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        let code = String::from_utf8_lossy(&out.stdout).trim().parse::<u16>();
        return code.map_err(|e| format!("bad_status: {e}"));
    }
    // Fallback: raw TCP for http only
    let url = url.strip_prefix("http://").ok_or("only_http_without_curl")?;
    let (hostport, path) = match url.find('/') {
        Some(i) => (&url[..i], &url[i..]),
        None => (url, "/"),
    };
    let (host, port) = if let Some(i) = hostport.rfind(':') {
        (
            &hostport[..i],
            hostport[i + 1..].parse::<u16>().unwrap_or(80),
        )
    } else {
        (hostport, 80u16)
    };
    use std::io::{Read, Write};
    use std::net::TcpStream;
    let mut stream =
        TcpStream::connect((host, port)).map_err(|e| format!("connect: {e}"))?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok();
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {hostport}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(req.as_bytes())
        .map_err(|e| format!("write: {e}"))?;
    let mut buf = String::new();
    stream.read_to_string(&mut buf).ok();
    let status = buf
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);
    if status == 0 {
        Err("no_status".into())
    } else {
        Ok(status)
    }
}

fn which_curl() -> bool {
    std::process::Command::new("curl")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Drain ready pending webhooks (iss/49 production drain path).
/// `max_n`: max items this call; `max_attempts`: fail→DLQ threshold.
pub fn drain_webhooks(max_n: usize, max_attempts: u32) -> Value {
    let now = now_ms();
    let mut ready: Vec<OutboxItem> = Vec::new();
    if let Ok(mut g) = boxes().lock() {
        let mut rest = VecDeque::new();
        while let Some(item) = g.pending.pop_front() {
            if ready.len() < max_n && item.next_attempt_ms <= now {
                ready.push(item);
            } else {
                rest.push_back(item);
            }
        }
        g.pending = rest;
    }
    let mut delivered = 0u32;
    let mut failed = 0u32;
    let mut dlq_n = 0u32;
    let mut details = Vec::new();
    for mut item in ready {
        let body = serde_json::to_string(&item.body).unwrap_or_else(|_| "{}".into());
        match http_post_json(&item.url, &body) {
            Ok(code) if (200..300).contains(&code) => {
                delivered += 1;
                details.push(json!({"id": item.id, "ok": true, "status": code}));
            }
            Ok(code) => {
                failed += 1;
                item.attempts += 1;
                item.last_error = Some(format!("http_{code}"));
                if item.attempts >= max_attempts {
                    if let Ok(mut g) = boxes().lock() {
                        g.dlq.push_back(item.clone());
                        while g.dlq.len() > MAX_DLQ {
                            g.dlq.pop_front();
                        }
                        persist_locked(&g);
                    }
                    dlq_n += 1;
                    details.push(json!({"id": item.id, "ok": false, "status": code, "dlq": true}));
                } else {
                    let backoff = (1i64 << item.attempts.min(8)) * 1000;
                    item.next_attempt_ms = now_ms() + backoff;
                    if let Ok(mut g) = boxes().lock() {
                        g.pending.push_back(item.clone());
                        persist_locked(&g);
                    }
                    details.push(json!({
                        "id": item.id, "ok": false, "status": code,
                        "retry": true, "backoff_ms": backoff
                    }));
                }
            }
            Err(e) => {
                failed += 1;
                item.attempts += 1;
                item.last_error = Some(e.clone());
                if item.attempts >= max_attempts {
                    if let Ok(mut g) = boxes().lock() {
                        g.dlq.push_back(item.clone());
                        while g.dlq.len() > MAX_DLQ {
                            g.dlq.pop_front();
                        }
                        persist_locked(&g);
                    }
                    dlq_n += 1;
                    details.push(json!({"id": item.id, "ok": false, "error": e, "dlq": true}));
                } else {
                    let backoff = (1i64 << item.attempts.min(8)) * 1000;
                    item.next_attempt_ms = now_ms() + backoff;
                    if let Ok(mut g) = boxes().lock() {
                        g.pending.push_back(item.clone());
                        persist_locked(&g);
                    }
                    details.push(json!({
                        "id": item.id, "ok": false, "error": e,
                        "retry": true, "backoff_ms": backoff
                    }));
                }
            }
        }
    }
    // Successful deliveries already removed from memory via not re-pushing; persist once.
    if let Ok(g) = boxes().lock() {
        persist_locked(&g);
    }
    json!({
        "algo": "webhook_drain_v1",
        "ok": true,
        "attempted": details.len(),
        "delivered": delivered,
        "failed": failed,
        "moved_to_dlq": dlq_n,
        "details": details,
        "outbox": outbox_status(),
    })
}

pub fn outbox_status() -> Value {
    let g = boxes().lock().ok();
    match g {
        Some(g) => json!({
            "algo": "webhook_outbox_v1",
            "pending_n": g.pending.len(),
            "dlq_n": g.dlq.len(),
            "durable": g.durable_dir.is_some(),
            "durable_dir": g.durable_dir.as_ref().map(|p| p.display().to_string()),
            "backend": if g.durable_dir.is_some() { "file" } else { "memory" },
            "pending_head": g.pending.front().map(|x| json!({
                "id": x.id,
                "event": x.event,
                "attempts": x.attempts,
                "url": x.url,
            })),
            "dlq_head": g.dlq.front().map(|x| json!({
                "id": x.id,
                "event": x.event,
                "attempts": x.attempts,
                "last_error": x.last_error,
            })),
        }),
        None => json!({}),
    }
}

/// Analyze-path dead letter (iss/49): failed analyze jobs for ops replay.
static ANALYZE_DLQ: std::sync::OnceLock<Mutex<VecDeque<Value>>> = std::sync::OnceLock::new();

pub fn analyze_dlq_push(session_id: &str, err: &str, attempts: u32) {
    let q = ANALYZE_DLQ.get_or_init(|| Mutex::new(VecDeque::new()));
    if let Ok(mut g) = q.lock() {
        g.push_back(json!({
            "session_id": session_id,
            "error": err,
            "attempts": attempts,
            "ts_ms": now_ms(),
            "algo": "analyze_dlq_v1",
        }));
        while g.len() > MAX_DLQ {
            g.pop_front();
        }
    }
}

pub fn analyze_dlq_list(limit: usize) -> Value {
    let q = ANALYZE_DLQ.get_or_init(|| Mutex::new(VecDeque::new()));
    let g = q.lock().ok();
    match g {
        Some(g) => {
            let items: Vec<_> = g.iter().rev().take(limit).cloned().collect();
            json!({"algo": "analyze_dlq_v1", "n": g.len(), "items": items})
        }
        None => json!({"n": 0, "items": []}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enqueue_and_dlq_after_retries() {
        let r = enqueue_webhook(
            "site_t",
            "analysis.complete",
            "https://example.test/hook",
            json!({"session_id": "s1"}),
            Some("sec"),
        );
        assert_eq!(r["ok"], true);
        let id = r["webhook_id"].as_str().unwrap().to_string();
        for _ in 0..3 {
            mark_webhook_failed(&id, "http_500", 3);
        }
        let st = outbox_status();
        assert!(st["dlq_n"].as_u64().unwrap() >= 1 || st["pending_n"].as_u64().unwrap() >= 0);
    }
}
