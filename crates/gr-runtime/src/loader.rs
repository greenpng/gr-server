//! dlopen-based module loader with refcounted unload after hot-swap.

use gr_abi::{
    cstr_to_str, ModuleEntryFn, ModuleMeta, ModuleVTable, MODULE_ENTRY_SYMBOL, RUNTIME_ABI,
};
use libloading::Library;
use semver::Version;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct LoadedModule {
    pub meta: ModuleMeta,
    pub path: PathBuf,
    _lib: Library,
    vtable: *const ModuleVTable,
}

// VTable is immutable static from module; Library ownership keeps it valid.
unsafe impl Send for LoadedModule {}
unsafe impl Sync for LoadedModule {}

impl LoadedModule {
    pub fn open(path: &Path, runtime_version: &str) -> Result<Arc<Self>, String> {
        // RTLD_LOCAL|DEEPBIND: isolate plugin symbols so multiple .so modules can load together.
        #[cfg(target_os = "linux")]
        let lib = {
            use libloading::os::unix::{Library as UnixLibrary, RTLD_LAZY, RTLD_LOCAL};
            // DEEPBIND may not exist on all platforms; use libc flag when available.
            const RTLD_DEEPBIND: i32 = 0x00008;
            let flags = RTLD_LAZY | RTLD_LOCAL | RTLD_DEEPBIND;
            let unix = unsafe { UnixLibrary::open(Some(path), flags) }
                .map_err(|e| format!("dlopen {}: {e}", path.display()))?;
            Library::from(unix)
        };
        #[cfg(not(target_os = "linux"))]
        let lib = unsafe { Library::new(path) }
            .map_err(|e| format!("dlopen {}: {e}", path.display()))?;
        let entry: libloading::Symbol<ModuleEntryFn> = unsafe {
            lib.get(MODULE_ENTRY_SYMBOL)
                .map_err(|e| format!("missing gr_module_entry: {e}"))?
        };
        let vt = unsafe { entry() };
        if vt.is_null() {
            return Err("null vtable".into());
        }
        let meta_ptr = unsafe { (*vt).meta_json };
        let meta_str = cstr_to_str(meta_ptr).ok_or("invalid meta_json")?;
        let meta: ModuleMeta = serde_json::from_str(meta_str).map_err(|e| e.to_string())?;
        let rt = Version::parse(runtime_version).map_err(|e| e.to_string())?;
        meta.is_compatible(&rt, RUNTIME_ABI)
            .map_err(|e| e.to_string())?;
        let loaded = Arc::new(Self {
            meta,
            path: path.to_path_buf(),
            _lib: lib,
            vtable: vt,
        });
        if let Some(init) = unsafe { (*vt).init } {
            let code = init(std::ptr::null());
            if code != 0 {
                return Err(format!("module init failed code={code}"));
            }
        }
        Ok(loaded)
    }

    pub fn apply_config(&self, json: &str) -> i32 {
        let vt = unsafe { &*self.vtable };
        if let Some(f) = vt.apply_config {
            let c = std::ffi::CString::new(json).unwrap_or_default();
            return f(c.as_ptr());
        }
        0
    }

    /// Domain event dispatch (e.g. analyze `select_device_segments`). Returns response bytes.
    pub fn on_event(&self, event: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        let vt = unsafe { &*self.vtable };
        let f = vt
            .on_event
            .ok_or_else(|| format!("module {} has no on_event", self.name()))?;
        let c_event = std::ffi::CString::new(event).map_err(|e| e.to_string())?;
        let mut cap = 256 * 1024usize;
        for _ in 0..5 {
            let mut buf = vec![0u8; cap];
            let n = f(
                c_event.as_ptr(),
                payload.as_ptr(),
                payload.len(),
                buf.as_mut_ptr(),
                buf.len(),
            );
            if n < 0 {
                return Err(format!(
                    "on_event {event} failed code={n} module={}",
                    self.name()
                ));
            }
            let n = n as usize;
            if n <= buf.len() {
                buf.truncate(n);
                return Ok(buf);
            }
            // Module may return required size when buffer too small.
            cap = n.max(cap.saturating_mul(2));
        }
        Err(format!("on_event {event} response too large"))
    }

    pub fn has_on_event(&self) -> bool {
        unsafe { (*self.vtable).on_event.is_some() }
    }

    pub fn name(&self) -> &str {
        &self.meta.name
    }

    pub fn version(&self) -> &str {
        &self.meta.version
    }
}

impl Drop for LoadedModule {
    fn drop(&mut self) {
        let vt = unsafe { &*self.vtable };
        if let Some(shutdown) = vt.shutdown {
            let _ = shutdown();
        }
    }
}

pub struct ModuleLoader;

impl ModuleLoader {
    pub fn find_so_in_version_dir(dir: &Path) -> Option<PathBuf> {
        let rd = std::fs::read_dir(dir).ok()?;
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("so") {
                return Some(p);
            }
        }
        None
    }
}
