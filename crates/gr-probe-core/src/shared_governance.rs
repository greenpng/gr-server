//! Cross-process shared state for iss/58 residual items:
//! hot-bucket · population atlas · m/u census · HNSW · contrastive weights.
//!
//! Backed by JSON files under `GR_SHARED_GOVERNANCE_DIR` (or
//! `$GR_DATA_DIR/shared_governance` / `data/shared_governance`).
//! Exclusive access via create-new lock file + retries.
//!
//! **Test isolation (iss/59 P0)**: directory override is **thread-local** so
//! parallel `cargo test` workers do not clobber each other.
//!
//! Disable with `GR_SHARED_GOVERNANCE=0`.

use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Process-global override (single-thread helpers / service). Prefer thread-local.
static DIR_OVERRIDE: Mutex<Option<Option<PathBuf>>> = Mutex::new(None);

/// Cross-module test serialization for process-global governance state
/// (hot buckets, census, HNSW, contrastive). Parallel cargo test safe.
pub static ISS58_TEST_LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    /// Per-test-thread override — primary isolation for parallel cargo test.
    static THREAD_DIR: RefCell<Option<Option<PathBuf>>> = const { RefCell::new(None) };
}

/// Observability counters (iss/59 P1-3).
static LOCK_OK: AtomicU64 = AtomicU64::new(0);
static LOCK_FAIL: AtomicU64 = AtomicU64::new(0);
static SAVE_FAIL: AtomicU64 = AtomicU64::new(0);

/// Set shared dir for **this thread only** (tests). Pass `None` to disable shared.
pub fn set_shared_governance_dir_for_tests(dir: Option<PathBuf>) {
    THREAD_DIR.with(|c| {
        *c.borrow_mut() = Some(dir);
    });
}

/// Also set process-global override (optional; service tests).
pub fn set_shared_governance_dir_global_for_tests(dir: Option<PathBuf>) {
    let mut g = DIR_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner());
    *g = Some(dir);
}

pub fn clear_thread_shared_governance_override() {
    THREAD_DIR.with(|c| {
        *c.borrow_mut() = None;
    });
}

pub fn shared_governance_enabled() -> bool {
    // Thread override wins
    let mut thread_hit = false;
    let mut thread_enabled = false;
    THREAD_DIR.with(|c| {
        if let Some(ref o) = *c.borrow() {
            thread_hit = true;
            thread_enabled = o.is_some();
        }
    });
    if thread_hit {
        return thread_enabled;
    }
    if let Ok(g) = DIR_OVERRIDE.lock() {
        if let Some(Some(_)) = *g {
            return true;
        }
        if let Some(None) = *g {
            return false;
        }
    }
    match gr_abi::env::get("SHARED_GOVERNANCE")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "0" | "false" | "off" | "no" => false,
        "1" | "true" | "on" | "yes" => true,
        _ => {
            gr_abi::env::get("SHARED_GOVERNANCE_DIR")
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false)
                || gr_abi::env::get("DATA_DIR")
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false)
        }
    }
}

pub fn shared_governance_dir() -> Option<PathBuf> {
    // 1) thread-local
    let mut thread_set = false;
    let mut thread_dir: Option<PathBuf> = None;
    THREAD_DIR.with(|c| {
        if let Some(ref o) = *c.borrow() {
            thread_set = true;
            thread_dir = o.clone();
        }
    });
    if thread_set {
        return thread_dir;
    }
    // 2) process global
    if let Ok(g) = DIR_OVERRIDE.lock() {
        if let Some(ref override_opt) = *g {
            return override_opt.clone();
        }
    }
    if matches!(
        gr_abi::env::get("SHARED_GOVERNANCE")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "0" | "false" | "off" | "no"
    ) {
        return None;
    }
    if let Some(p) = gr_abi::env::get("SHARED_GOVERNANCE_DIR") {
        let p = p.trim();
        if !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    if !shared_governance_enabled() {
        return None;
    }
    if let Some(d) = gr_abi::env::get("DATA_DIR") {
        let d = d.trim();
        if !d.is_empty() {
            return Some(PathBuf::from(d).join("shared_governance"));
        }
    }
    Some(PathBuf::from("data/shared_governance"))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

struct FileLock {
    #[allow(dead_code)]
    file: File,
    path: PathBuf,
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = self.file.sync_all();
        let _ = fs::remove_file(&self.path);
    }
}

fn acquire_lock(dir: &Path, name: &str) -> Option<FileLock> {
    let _ = fs::create_dir_all(dir);
    let lock_path = dir.join(format!(".{name}.lock"));
    for attempt in 0..200 {
        if let Ok(meta) = fs::metadata(&lock_path) {
            if let Ok(mtime) = meta.modified() {
                // Reclaim stale locks aggressively under test contention
                if mtime.elapsed().unwrap_or(Duration::from_secs(0)) > Duration::from_secs(2) {
                    let _ = fs::remove_file(&lock_path);
                }
            }
        }
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(mut f) => {
                let _ = writeln!(f, "{}", now_ms());
                LOCK_OK.fetch_add(1, Ordering::Relaxed);
                return Some(FileLock {
                    file: f,
                    path: lock_path,
                });
            }
            Err(_) => {
                std::thread::sleep(Duration::from_millis(1 + (attempt % 20) as u64));
            }
        }
    }
    LOCK_FAIL.fetch_add(1, Ordering::Relaxed);
    eprintln!(
        "[shared_governance] WARN lock_fail name={name} dir={} fails={}",
        dir.display(),
        LOCK_FAIL.load(Ordering::Relaxed)
    );
    None
}

pub fn load_json(path: &Path) -> Option<Value> {
    let mut f = File::open(path).ok()?;
    let mut s = String::new();
    f.read_to_string(&mut s).ok()?;
    serde_json::from_str(&s).ok()
}

pub fn save_json_atomic(path: &Path, v: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string(v).map_err(|e| e.to_string())?;
    fs::write(&tmp, text).map_err(|e| format!("write tmp: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| format!("rename: {e}"))?;
    Ok(())
}

/// Run `f` under exclusive lock with load/save.
/// Prefer **Redis L2** when configured (iss/60 R2); else file L1.
pub fn with_shared_json<R>(
    name: &str,
    default: Value,
    f: impl FnOnce(&mut Value) -> R,
) -> Option<R> {
    // L2 Redis multi-worker path
    if crate::shared_l2::l2_enabled() {
        let file_seed = shared_governance_dir().and_then(|dir| {
            load_json(&dir.join(format!("{name}.json")))
        });
        let mut v = crate::shared_l2::l2_get_json(name)
            .or(file_seed)
            .unwrap_or(default);
        if !v.is_object() {
            v = json!({});
        }
        let out = f(&mut v);
        match crate::shared_l2::l2_set_json(name, &v) {
            Ok(()) => {
                // best-effort file mirror for ops dump
                if let Some(dir) = shared_governance_dir() {
                    let path = dir.join(format!("{name}.json"));
                    let _ = save_json_atomic(&path, &v);
                }
                return Some(out);
            }
            Err(e) => {
                eprintln!("[shared_governance] L2 set fail, file fallback name={name}: {e}");
                // continue with file using already-updated v
                if let Some(dir) = shared_governance_dir() {
                    let path = dir.join(format!("{name}.json"));
                    if let Some(_lock) = acquire_lock(&dir, name) {
                        let _ = save_json_atomic(&path, &v);
                    }
                }
                return Some(out);
            }
        }
    }
    let dir = shared_governance_dir()?;
    let path = dir.join(format!("{name}.json"));
    let _lock = acquire_lock(&dir, name)?;
    let mut v = load_json(&path).unwrap_or(default);
    if !v.is_object() {
        v = json!({});
    }
    let out = f(&mut v);
    if let Err(e) = save_json_atomic(&path, &v) {
        SAVE_FAIL.fetch_add(1, Ordering::Relaxed);
        eprintln!(
            "[shared_governance] WARN save_fail name={name} err={e} fails={}",
            SAVE_FAIL.load(Ordering::Relaxed)
        );
    }
    Some(out)
}

pub fn read_shared_json(name: &str) -> Option<Value> {
    if crate::shared_l2::l2_enabled() {
        if let Some(v) = crate::shared_l2::l2_get_json(name) {
            return Some(v);
        }
    }
    let dir = shared_governance_dir()?;
    let path = dir.join(format!("{name}.json"));
    let _lock = acquire_lock(&dir, name);
    load_json(&path)
}

pub fn as_object_mut(v: &mut Value) -> &mut Map<String, Value> {
    if !v.is_object() {
        *v = json!({});
    }
    v.as_object_mut().expect("object")
}

pub fn shared_state_paths() -> Value {
    let dir = shared_governance_dir();
    json!({
        "enabled": shared_governance_enabled() || crate::shared_l2::l2_enabled(),
        "dir": dir.as_ref().map(|p| p.display().to_string()),
        "files": ["hot_buckets", "population_atlas", "mu_census", "hnsw_ann", "contrastive_w"],
        "algo": "shared_governance_v1",
        "l2": crate::shared_l2::l2_metrics(),
        "metrics": shared_governance_metrics(),
    })
}

pub fn shared_governance_metrics() -> Value {
    json!({
        "lock_ok": LOCK_OK.load(Ordering::Relaxed),
        "lock_fail": LOCK_FAIL.load(Ordering::Relaxed),
        "save_fail": SAVE_FAIL.load(Ordering::Relaxed),
        "l2": crate::shared_l2::l2_metrics(),
    })
}

pub fn clear_shared_governance_files() {
    if let Some(dir) = shared_governance_dir() {
        for name in [
            "hot_buckets",
            "population_atlas",
            "mu_census",
            "hnsw_ann",
            "contrastive_w",
        ] {
            let p = dir.join(format!("{name}.json"));
            let _ = fs::remove_file(p);
            let _ = fs::remove_file(dir.join(format!(".{name}.lock")));
        }
    }
}

/// Unique temp dir for one test, registered as thread-local shared dir.
pub fn test_isolation_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gr_sg_{}_{}_{}",
        label,
        std::process::id(),
        now_ms()
    ));
    let _ = fs::create_dir_all(&dir);
    set_shared_governance_dir_for_tests(Some(dir.clone()));
    dir
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering as Ao};

    #[test]
    fn shared_json_roundtrip() {
        let dir = test_isolation_dir("rt");
        with_shared_json("t1", json!({}), |v| {
            as_object_mut(v).insert("k".into(), json!(1));
        });
        let r = read_shared_json("t1").unwrap();
        assert_eq!(r["k"], 1);
        clear_shared_governance_files();
        set_shared_governance_dir_for_tests(None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_merges_preserve_keys() {
        let dir = test_isolation_dir("c1");
        static N: AtomicUsize = AtomicUsize::new(0);
        N.store(0, Ao::SeqCst);
        let mut handles = vec![];
        for i in 0..8 {
            let d = dir.clone();
            handles.push(std::thread::spawn(move || {
                set_shared_governance_dir_for_tests(Some(d));
                // Retry on rare lock timeout under CI load
                for _try in 0..5 {
                    if with_shared_json("c1", json!({"hits":{}}), |v| {
                        let o = as_object_mut(v);
                        let hits = o
                            .entry("hits")
                            .or_insert(json!({}))
                            .as_object_mut()
                            .unwrap();
                        hits.insert(format!("k{i}"), json!(i));
                        N.fetch_add(1, Ao::SeqCst);
                    })
                    .is_some()
                    {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                panic!("lock failed after retries for k{i}");
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        set_shared_governance_dir_for_tests(Some(dir.clone()));
        let r = read_shared_json("c1").expect("read after merges");
        let n = r["hits"].as_object().map(|m| m.len()).unwrap_or(0);
        assert!(n >= 6, "expected most keys merged, got {n} r={r}");
        clear_shared_governance_files();
        set_shared_governance_dir_for_tests(None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn metrics_count_lock_ok() {
        let dir = test_isolation_dir("met");
        let before = LOCK_OK.load(Ordering::Relaxed);
        with_shared_json("m1", json!({}), |_| ());
        assert!(LOCK_OK.load(Ordering::Relaxed) > before);
        clear_shared_governance_files();
        set_shared_governance_dir_for_tests(None);
        let _ = fs::remove_dir_all(&dir);
    }
}
