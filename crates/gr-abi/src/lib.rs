//! GR plugin ABI, version compatibility, and shared error types.
//!
//! Every loadable module exports `gr_module_entry` returning [`ModuleVTable`].
//!
//! GR naming migration (docs/guides/13-GR-MIGRATION.md): configuration reads go
//! through [`env`], which prefers `GR_*` and falls back to `GR_*`/`GR_*`
//! so 8.0 keeps reading legacy names while new deployments write only `GR_*`.

pub mod env;

use serde::{Deserialize, Serialize};
use semver::Version;
use sha2::{Digest, Sha256};
use std::ffi::CStr;
use std::os::raw::{c_char, c_void};

/// Bump only on breaking runtime↔module binary interface changes.
pub const RUNTIME_ABI: u32 = 1;
pub const PRODUCT_MAJOR: u64 = 1;

#[derive(Debug, thiserror::Error)]
pub enum AbiError {
    #[error("incompatible major: runtime={runtime} module={module}")]
    MajorMismatch { runtime: u64, module: u64 },
    #[error("runtime minor {runtime_minor} < module min {min}")]
    MinorTooLow { runtime_minor: u64, min: u64 },
    #[error("runtime minor {runtime_minor} > module max {max}")]
    MinorTooHigh { runtime_minor: u64, max: u64 },
    #[error("abi mismatch: runtime={runtime} module={module}")]
    AbiMismatch { runtime: u32, module: u32 },
    #[error("module error: {0}")]
    Module(String),
    #[error("invalid version: {0}")]
    InvalidVersion(String),
}

/// Declared by each module artifact (embedded + release manifest).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModuleMeta {
    pub name: String,
    pub version: String,
    pub abi: u32,
    pub requires_major: u64,
    /// Inclusive lower bound on runtime minor.
    pub min_runtime_minor: u64,
    /// Inclusive upper bound; `None` = no upper limit.
    pub max_runtime_minor: Option<u64>,
    /// Business domain tag for blast-radius grouping.
    pub domain: String,
}

impl ModuleMeta {
    pub fn parse_version(&self) -> Result<Version, AbiError> {
        Version::parse(&self.version).map_err(|e| AbiError::InvalidVersion(e.to_string()))
    }

    pub fn is_compatible(&self, runtime_version: &Version, runtime_abi: u32) -> Result<(), AbiError> {
        if self.abi != runtime_abi {
            return Err(AbiError::AbiMismatch {
                runtime: runtime_abi,
                module: self.abi,
            });
        }
        if self.requires_major != runtime_version.major {
            return Err(AbiError::MajorMismatch {
                runtime: runtime_version.major,
                module: self.requires_major,
            });
        }
        if runtime_version.minor < self.min_runtime_minor {
            return Err(AbiError::MinorTooLow {
                runtime_minor: runtime_version.minor,
                min: self.min_runtime_minor,
            });
        }
        if let Some(max) = self.max_runtime_minor {
            if runtime_version.minor > max {
                return Err(AbiError::MinorTooHigh {
                    runtime_minor: runtime_version.minor,
                    max,
                });
            }
        }
        Ok(())
    }
}

/// Host services passed into modules (opaque to keep ABI small).
#[repr(C)]
pub struct HostContext {
    pub user_data: *mut c_void,
    pub log_fn: Option<extern "C" fn(*mut c_void, u32 /*level*/, *const c_char)>,
    pub config_json: *const c_char,
}

/// Module lifecycle vtable — keep C ABI stable; extend only with new optional fields + abi bump.
#[repr(C)]
pub struct ModuleVTable {
    pub meta_json: *const c_char,
    pub init: Option<extern "C" fn(*const HostContext) -> i32>,
    pub shutdown: Option<extern "C" fn() -> i32>,
    /// Hot config apply (worker counts, site crypto, etc). JSON in, 0=ok.
    pub apply_config: Option<extern "C" fn(*const c_char) -> i32>,
    /// Optional HTTP-ish dispatch hook for domain modules; may be null.
    pub on_event: Option<extern "C" fn(*const c_char, *const u8, usize, *mut u8, usize) -> i32>,
}

// Pointers are to static meta strings / functions — safe to share across threads.
unsafe impl Send for ModuleVTable {}
unsafe impl Sync for ModuleVTable {}
unsafe impl Send for HostContext {}
unsafe impl Sync for HostContext {}

pub type ModuleEntryFn = unsafe extern "C" fn() -> *const ModuleVTable;

pub const MODULE_ENTRY_SYMBOL: &[u8] = b"gr_module_entry\0";

pub fn cstr_to_str<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(p) }.to_str().ok()
}

pub fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex::encode(h.finalize())
}

/// Root domain matching: `example.com` covers itself and `*.example.com`.
pub fn host_matches_root(host: &str, root: &str) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    let root = root.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || root.is_empty() {
        return false;
    }
    host == root || host.ends_with(&format!(".{root}"))
}

/// True if host matches any root domain of a site.
pub fn host_matches_any_root(host: &str, roots: &[String]) -> bool {
    roots.iter().any(|r| host_matches_root(host, r))
}

/// Release manifest (published to GitHub, not source).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseManifest {
    pub product: String,
    pub channel: String,
    /// Host arch this manifest targets (e.g. `x86_64`, `aarch64`). Multi-arch releases only.
    #[serde(default)]
    pub arch: Option<String>,
    /// Asset naming triple (e.g. `x86_64-linux-gnu`). Multi-arch releases only.
    #[serde(default)]
    pub triple: Option<String>,
    pub runtime: RuntimeManifest,
    pub modules: Vec<ModuleArtifact>,
    /// Optional FE asset integrity (required for signed FE install).
    #[serde(default)]
    pub fe: Option<FeManifest>,
    /// Expanded FE tree inside a whole-bundle release: per-file sha256.
    /// Signed as part of the canonical body when present (greenpng 1.0.0+).
    #[serde(default)]
    pub fe_tree: Option<TreeManifest>,
    /// Expanded admin SPA tree inside a whole-bundle release: per-file sha256.
    #[serde(default)]
    pub admin_tree: Option<TreeManifest>,
    /// Expanded spec tree inside a whole-bundle release: per-file sha256
    /// (analyze runtime data, greenpng 1.0.2+). Absent on 1.0.0/1.0.1
    /// bundles — build_and_publish.sh emitted it but `sign-manifest`
    /// round-tripped the JSON through this struct without the field and
    /// silently dropped it, so those bundles ship spec/ unsigned.
    #[serde(default)]
    pub spec_tree: Option<TreeManifest>,
    /// CLI binary integrity (P1-4). The `gr-cli` helper is downloaded by
    /// installers and performs module verification + staging, so a tampered
    /// CLI could bypass every downstream check. Releases built for the
    /// hardened installer must carry the CLI asset + sha256 here, and the
    /// canonical signature body covers it. Absent on pre-hardening releases
    /// (bodies without `cli` stay byte-identical for legacy verification).
    #[serde(default)]
    pub cli: Option<RuntimeManifest>,
    /// ed25519 signature over canonical body (optional at parse; verified by ota when present / required in prod).
    #[serde(default)]
    pub sig: Option<String>,
    /// Per-release random build id (P0-1). Differs on every build; services
    /// embed the same id so a patch from a previous release cannot be reused.
    #[serde(default)]
    pub build_id: Option<String>,
    /// Per-release Ed25519 pubkey (hex) — release keys rotate every build.
    /// Modules are double-signed: root sig (legacy path) + release sig.
    #[serde(default)]
    pub release_pubkey: Option<String>,
    /// Root-signed certificate over `release_pubkey||version||build_id` (b64).
    /// Verified against the built-in root key; binds the release key to this build.
    #[serde(default)]
    pub release_cert: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeManifest {
    pub version: String,
    pub abi: u32,
    /// Optional download asset name (e.g. gr-service-8.0.0-x86_64-linux-gnu).
    #[serde(default)]
    pub asset: Option<String>,
    /// Optional SHA-256 of runtime binary (hex).
    #[serde(default)]
    pub sha256: Option<String>,
}

/// FE tarball integrity entry in release manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeManifest {
    pub asset: String,
    pub sha256: String,
    /// Optional ed25519 signature over `fe|{asset}|{sha256}` (or reuse manifest sig only).
    #[serde(default)]
    pub sig: Option<String>,
}

/// Expanded directory tree inside a whole-bundle release (fe/ or admin/):
/// per-file sha256 map, signed as part of the manifest canonical body.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TreeManifest {
    /// Version epoch the tree was built for (fe/VERSION semantics). Optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<String>,
    pub files: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleArtifact {
    pub name: String,
    pub version: String,
    pub abi: u32,
    pub requires_major: u64,
    pub min_runtime_minor: u64,
    #[serde(default)]
    pub max_runtime_minor: Option<u64>,
    pub asset: String,
    pub sha256: String,
    pub sig: String,
    /// Optional per-release-key signature (P0-2). Present on releases built
    /// after the key-rotation change: `sig` (root) + `sig2` (release key whose
    /// cert chains to the root). Old runtimes verify `sig` only; new runtimes
    /// require both when the manifest carries a release_pubkey.
    #[serde(default)]
    pub sig2: Option<String>,
    #[serde(default)]
    pub domain: String,
}

impl ModuleArtifact {
    pub fn to_meta(&self) -> ModuleMeta {
        ModuleMeta {
            name: self.name.clone(),
            version: self.version.clone(),
            abi: self.abi,
            requires_major: self.requires_major,
            min_runtime_minor: self.min_runtime_minor,
            max_runtime_minor: self.max_runtime_minor,
            domain: self.domain.clone(),
        }
    }
}

/// Known module names (business domains).
pub mod modules {
    pub const IDENTITY: &str = "identity";
    pub const INGEST: &str = "ingest";
    pub const EDGE: &str = "edge";
    pub const BRAIN: &str = "brain";
    pub const ANALYZE: &str = "analyze";
    pub const PROBE_ASSETS: &str = "probe_assets";
    pub const ADMIN_API: &str = "admin_api";
    /// Paid multi-node load-balancer (docs/guides/08-LB-MODULE.md); private distribution.
    pub const LB: &str = "lb";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_cover() {
        assert!(host_matches_root("example.com", "example.com"));
        assert!(host_matches_root("a.example.com", "example.com"));
        assert!(host_matches_root("a.b.example.com", "example.com"));
        assert!(!host_matches_root("evil-example.com", "example.com"));
        assert!(!host_matches_root("example.com.hacker.test", "example.com"));
    }

    #[test]
    fn compat_matrix() {
        // greenpng line: PRODUCT_MAJOR=1, module/runtime majors must match.
        let meta = ModuleMeta {
            name: "analyze".into(),
            version: "1.1.0".into(),
            abi: 1,
            requires_major: PRODUCT_MAJOR,
            min_runtime_minor: 0,
            max_runtime_minor: Some(2),
            domain: "analyze".into(),
        };
        let rt = Version::parse("1.0.0").unwrap();
        assert!(meta.is_compatible(&rt, 1).is_ok());
        let rt2 = Version::parse("1.3.0").unwrap();
        assert!(meta.is_compatible(&rt2, 1).is_err());
        let rt3 = Version::parse("2.9.0").unwrap();
        assert!(meta.is_compatible(&rt3, 1).is_err());
        // legacy 8.x runtime against a greenpng-line module must be rejected
        let rt4 = Version::parse("8.0.0").unwrap();
        assert!(meta.is_compatible(&rt4, 1).is_err());
    }
}
