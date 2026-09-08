//! ArcSwap registry: hot-swap modules without restarting the process.

use crate::loader::{LoadedModule, ModuleLoader};
use arc_swap::ArcSwap;
use parking_lot::RwLock;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

pub struct ModuleRegistry {
    runtime_version: String,
    #[allow(dead_code)]
    runtime_abi: u32,
    slots: RwLock<HashMap<String, Arc<ArcSwap<Option<Arc<LoadedModule>>>>>>,
}

impl ModuleRegistry {
    pub fn new(runtime_version: &str, runtime_abi: u32) -> Self {
        Self {
            runtime_version: runtime_version.to_string(),
            runtime_abi,
            slots: RwLock::new(HashMap::new()),
        }
    }

    fn slot(&self, name: &str) -> Arc<ArcSwap<Option<Arc<LoadedModule>>>> {
        let mut g = self.slots.write();
        g.entry(name.to_string())
            .or_insert_with(|| Arc::new(ArcSwap::from_pointee(None)))
            .clone()
    }

    pub fn get(&self, name: &str) -> Option<Arc<LoadedModule>> {
        let g = self.slots.read();
        g.get(name)
            .and_then(|s| s.load_full().as_ref().as_ref().cloned())
    }

    /// Names of currently loaded modules. The unattended OTA thread iterates
    /// these for its all-modules convergence check, but reads VERSIONS from
    /// the active markers (static-path modules keep their boot version in the
    /// registry until restart — see apply_cluster_ota_tick).
    pub fn loaded_names(&self) -> Vec<String> {
        let g = self.slots.read();
        g.iter()
            .filter(|(_, s)| s.load_full().is_some())
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// Load so and atomically activate (old module dropped when refs gone).
    pub fn hot_load(&self, name: &str, so_path: &Path) -> Result<Arc<LoadedModule>, String> {
        let loaded = LoadedModule::open(so_path, &self.runtime_version)?;
        if loaded.name() != name && name != loaded.meta.domain {
            // allow path name override only if meta name matches expected
            if loaded.name() != name {
                return Err(format!(
                    "module name mismatch: expected {name}, got {}",
                    loaded.name()
                ));
            }
        }
        let slot = self.slot(loaded.name());
        slot.store(Arc::new(Some(loaded.clone())));
        tracing::info!(
            module = loaded.name(),
            version = loaded.version(),
            path = %so_path.display(),
            "module hot-loaded"
        );
        Ok(loaded)
    }

    pub fn apply_config_all(&self, json: &str) {
        let g = self.slots.read();
        for (name, slot) in g.iter() {
            if let Some(m) = slot.load_full().as_ref() {
                let code = m.apply_config(json);
                if code != 0 {
                    tracing::warn!(module = %name, code, "apply_config failed");
                }
            }
        }
    }

    /// Unload a module (paid-gate enforcement etc.): drops the slot's loaded
    /// module; the old .so stays on disk and can be hot-loaded again later.
    pub fn unload(&self, name: &str) -> bool {
        let g = self.slots.read();
        if let Some(slot) = g.get(name) {
            let had = slot.load_full().as_ref().is_some();
            slot.store(Arc::new(None));
            if had {
                tracing::warn!(module = %name, "module unloaded");
            }
            return had;
        }
        false
    }

    pub fn list_status(&self) -> Vec<Value> {
        let g = self.slots.read();
        let mut out = Vec::new();
        for (name, slot) in g.iter() {
            match slot.load_full().as_ref() {
                Some(m) => out.push(json!({
                    "name": name,
                    "version": m.version(),
                    "path": m.path,
                    "domain": m.meta.domain,
                    "loaded": true,
                })),
                None => out.push(json!({"name": name, "loaded": false})),
            }
        }
        out
    }

    /// Read modules/active/* markers and load.
    ///
    /// Marker content may be:
    /// - absolute path to version dir or `.so`
    /// - relative to `modules_dir` (e.g. `versions/analyze/6.0.2` or `.../lib.so`)
    pub fn load_active_tree(&self, modules_dir: &Path) -> Result<usize, String> {
        let active = modules_dir.join("active");
        if !active.is_dir() {
            return Ok(0);
        }
        let mut n = 0;
        for ent in std::fs::read_dir(&active).map_err(|e| e.to_string())? {
            let ent = ent.map_err(|e| e.to_string())?;
            let name = ent.file_name().to_string_lossy().to_string();
            let content = std::fs::read_to_string(ent.path()).map_err(|e| e.to_string())?;
            let raw = content.trim();
            if raw.is_empty() {
                continue;
            }
            let p = Path::new(raw);
            let resolved = if p.is_absolute() {
                p.to_path_buf()
            } else {
                // Prefer modules_dir-relative, then cwd-relative.
                let under_mod = modules_dir.join(p);
                if under_mod.exists() {
                    under_mod
                } else {
                    p.to_path_buf()
                }
            };
            let so = if resolved.is_file()
                && resolved
                    .extension()
                    .and_then(|x| x.to_str())
                    .map(|e| e == "so")
                    .unwrap_or(false)
            {
                Some(resolved.clone())
            } else if resolved.is_dir() {
                ModuleLoader::find_so_in_version_dir(&resolved)
            } else if let Some(parent) = resolved.parent() {
                // marker pointed at so path that doesn't exist as file — try parent dir
                ModuleLoader::find_so_in_version_dir(parent)
            } else {
                None
            };
            if let Some(so) = so {
                // Only dlopen modules that are designed for live swap. Other plugins
                // are static-linked into gr-service; re-dlopen can SEGV.
                if name != "analyze" {
                    tracing::info!(
                        module = %name,
                        path = %so.display(),
                        "skip boot dlopen (activation marker only; static-link path)"
                    );
                    continue;
                }
                match self.hot_load(&name, &so) {
                    Ok(_) => n += 1,
                    Err(e) => tracing::warn!(module = %name, error = %e, "skip active module"),
                }
            } else {
                tracing::warn!(module = %name, path = %resolved.display(), "no .so for active marker");
            }
        }
        Ok(n)
    }
}
