//! OTA: fetch GitHub release manifest, verify signatures, stage artifacts for hot-swap.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use gr_abi::{sha256_hex, ModuleArtifact, ModuleMeta, ReleaseManifest, RUNTIME_ABI};use semver::Version;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Normalize uname / rustc arch labels to release index keys.
pub fn normalize_host_arch(arch: &str) -> String {
    match arch.trim().to_ascii_lowercase().as_str() {
        "x86_64" | "amd64" | "x64" => "x86_64".into(),
        "aarch64" | "arm64" => "aarch64".into(),
        other => other.to_string(),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OtaError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("http: {0}")]
    Http(String),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("signature invalid")]
    BadSignature,
    #[error("sha256 mismatch for {0}")]
    BadHash(String),
    #[error("incompatible: {0}")]
    Incompatible(String),
    #[error("key: {0}")]
    Key(String),
    #[error("{0}")]
    Other(String),
}

/// Product requirement (design session #2, docs/05): keep the newest 3
/// pre-update versions for hot rollback; older ones are pruned on activation.
/// Rollback = `ota activate <name> <old-version>` (instant, no re-download).
pub const KEEP_ROLLBACK_VERSIONS: usize = 3;

/// sha256 of a file on disk (hex).
pub fn sha256_file_hex(path: &Path) -> Result<String, OtaError> {
    let bytes = fs::read(path)?;
    Ok(gr_abi::sha256_hex(&bytes))
}

/// Whole-bundle archives contain a single top-level directory
/// (`greenpng-<ver>-<arch>/`). Return it; fall back to the extract root when
/// the layout is already flat.
pub fn bundle_tree_root(extract_dir: &Path) -> PathBuf {
    let mut dirs = Vec::new();
    let mut files = 0;
    if let Ok(rd) = fs::read_dir(extract_dir) {
        for ent in rd.flatten() {
            match ent.file_type() {
                Ok(t) if t.is_dir() => dirs.push(ent.path()),
                Ok(t) if t.is_file() => files += 1,
                _ => {}
            }
        }
    }
    if files == 0 && dirs.len() == 1 {
        return dirs.pop().expect("one dir");
    }
    extract_dir.to_path_buf()
}

/// Safe tar.gz extraction (R-05): reject absolute paths, `..` components and
/// escaping symlinks before extracting. Shells out to the system `tar`.
pub fn safe_extract_tar_gz(tgz: &Path, dest: &Path) -> Result<(), String> {
    let list = std::process::Command::new("tar")
        .args(["-tzf", tgz.to_str().unwrap_or("")])
        .output()
        .map_err(|e| e.to_string())?;
    if !list.status.success() {
        return Err(format!(
            "tar -tzf failed: {}",
            String::from_utf8_lossy(&list.stderr)
        ));
    }
    for line in String::from_utf8_lossy(&list.stdout).lines() {
        let path = line.trim();
        if path.is_empty() {
            continue;
        }
        // Reject absolute paths and real `..` components only.
        // Do NOT use substring `..` — FE paths like `[[...path]]` are valid.
        if path.starts_with('/') {
            return Err(format!("tar entry rejected (absolute path): {path}"));
        }
        for comp in path.split(['/', '\\']) {
            if comp == ".." {
                return Err(format!("tar entry rejected (path escape): {path}"));
            }
        }
    }
    // Verbose list: allow only relative symlinks that do not escape dest.
    // Absolute / `..` targets rejected. Hard links not accepted.
    let vlist = std::process::Command::new("tar")
        .args(["-tvzf", tgz.to_str().unwrap_or("")])
        .output()
        .map_err(|e| e.to_string())?;
    if vlist.status.success() {
        for line in String::from_utf8_lossy(&vlist.stdout).lines() {
            let t = line.trim_start();
            if let Some(idx) = t.find(" -> ") {
                let target = t[idx + 4..].trim();
                if target.starts_with('/') || target.split(['/', '\\']).any(|c| c == "..") {
                    return Err(format!("tar entry rejected (symlink escape): {line}"));
                }
            }
        }
    }
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let status = std::process::Command::new("tar")
        .args(["-xzf", tgz.to_str().unwrap_or(""), "-C"])
        .arg(dest)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("tar extract failed: {status}"));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OtaConfig {
    /// e.g. https://github.com/OWNER/gr-releases/releases/latest/download
    pub release_base_url: String,
    pub modules_dir: PathBuf,
    pub pubkey_bytes: Vec<u8>,
    /// P0-2: per-release verifying key bound by the manifest certificate, filled
    /// in by `fetch_manifest_blocking`/`verify_manifest_file` after chain
    /// verification. `None` for legacy (root-only) manifests.
    #[serde(default)]
    pub release_key: Option<Vec<u8>>,
    /// Cache directory for whole-bundle archives (greenpng-<ver>-<arch>.tar.gz).
    /// Set by embedders (gr-runtime: data_dir/ota_staging/bundle); defaults to
    /// the system temp dir. Holds the downloaded archive + extracted tree.
    #[serde(default)]
    pub bundle_cache: Option<PathBuf>,
}

pub fn generate_signing_keypair() -> (SigningKey, VerifyingKey) {
    let mut csprng = rand::rngs::OsRng;
    let signing = SigningKey::generate(&mut csprng);
    let verifying = signing.verifying_key();
    (signing, verifying)
}

pub fn sign_bytes(signing: &SigningKey, msg: &[u8]) -> String {
    let sig = signing.sign(msg);
    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, sig.to_bytes())
}

pub fn verify_bytes(pubkey: &[u8], msg: &[u8], sig_b64: &str) -> Result<(), OtaError> {
    if pubkey.len() != 32 {
        return Err(OtaError::Key("ed25519 pubkey must be 32 bytes".into()));
    }
    let mut pk = [0u8; 32];
    pk.copy_from_slice(pubkey);
    let vk = VerifyingKey::from_bytes(&pk).map_err(|e| OtaError::Key(e.to_string()))?;
    let raw = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, sig_b64)
        .map_err(|_| OtaError::BadSignature)?;
    if raw.len() != 64 {
        return Err(OtaError::BadSignature);
    }
    let mut sb = [0u8; 64];
    sb.copy_from_slice(&raw);
    let sig = Signature::from_bytes(&sb);
    vk.verify(msg, &sig).map_err(|_| OtaError::BadSignature)
}

/// Legacy canonical bytes (name|version|sha256) — kept for verifying pre-v2 signatures.
pub fn artifact_sign_message_v1(art: &ModuleArtifact) -> Vec<u8> {
    format!("{}|{}|{}", art.name, art.version, art.sha256).into_bytes()
}

/// v2: bind asset path, ABI, domain and runtime range so metadata cannot drift from sig.
pub fn artifact_sign_message(art: &ModuleArtifact) -> Vec<u8> {
    format!(
        "v2|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        art.name,
        art.version,
        art.sha256,
        art.asset,
        art.abi,
        art.requires_major,
        art.min_runtime_minor,
        art.max_runtime_minor
            .map(|v| v.to_string())
            .unwrap_or_else(|| "none".into()),
        art.domain
    )
    .into_bytes()
}

pub fn verify_artifact_sig(pubkey: &[u8], art: &ModuleArtifact) -> Result<(), OtaError> {
    // Prefer v2; fall back to v1 so already-published modules still verify.
    if verify_bytes(pubkey, &artifact_sign_message(art), &art.sig).is_ok() {
        return Ok(());
    }
    verify_bytes(pubkey, &artifact_sign_message_v1(art), &art.sig)
}

/// Canonical bytes a root key signs to certify a per-release signing key.
/// Format: `gr-release-cert-v1|{version}|{build_id}|{release_pubkey_hex}`.
/// P0-4: the domain prefix is compile-time obfuscated (per-release salt) so a
/// dumped binary does not fingerprint the certificate scheme.
pub fn release_cert_message(version: &str, build_id: &str, pubkey_hex: &str) -> Vec<u8> {
    format!(
        "{}|{version}|{build_id}|{pubkey_hex}",
        gr_obf::obf!("gr-release-cert-v1").s()
    )
    .into_bytes()
}

/// Verify the release certificate carried by a manifest against the root
/// public key. Returns the per-release verifying key on success.
pub fn verify_release_cert(
    root_pubkey: &[u8],
    man: &ReleaseManifest,
) -> Result<ed25519_dalek::VerifyingKey, OtaError> {
    let Some(pk_hex) = man.release_pubkey.as_ref().filter(|s| !s.is_empty()) else {
        return Err(OtaError::Other("manifest release_pubkey missing".into()));
    };
    let Some(cert) = man.release_cert.as_ref().filter(|s| !s.is_empty()) else {
        return Err(OtaError::Other("manifest release_cert missing".into()));
    };
    let Some(bid) = man.build_id.as_ref().filter(|s| !s.is_empty()) else {
        return Err(OtaError::Other("manifest build_id missing".into()));
    };
    let msg = release_cert_message(&man.runtime.version, bid, pk_hex);
    verify_bytes(root_pubkey, &msg, cert)?;
    let raw = hex::decode(pk_hex).map_err(|_| OtaError::Key("release_pubkey not hex".into()))?;
    if raw.len() != 32 {
        return Err(OtaError::Key("release_pubkey must be 32 bytes".into()));
    }
    let mut pk = [0u8; 32];
    pk.copy_from_slice(&raw);
    ed25519_dalek::VerifyingKey::from_bytes(&pk)
        .map_err(|e| OtaError::Key(e.to_string()))
}

/// Full-chain verification for a release manifest: root sig on the manifest
/// body, and — when the manifest carries a release key — its certificate.
/// Returns the verified release key (if any) for artifact verification.
pub fn verify_manifest_chain(
    root_pubkey: &[u8],
    man: &ReleaseManifest,
) -> Result<Option<ed25519_dalek::VerifyingKey>, OtaError> {
    verify_manifest_sig(root_pubkey, man)?;
    if man.release_pubkey.as_ref().map(|s| !s.is_empty()).unwrap_or(false) {
        Ok(Some(verify_release_cert(root_pubkey, man)?))
    } else {
        Ok(None)
    }
}

/// Verify an artifact against the (possibly release-rotated) chain:
/// 1. root sig (`sig`, legacy-compatible).
/// 2. if a release key is bound by the manifest, `sig2` must be present and
///    verify with it (new releases are double-signed).
pub fn verify_artifact_sig_chain(
    root_pubkey: &[u8],
    release: Option<&ed25519_dalek::VerifyingKey>,
    art: &ModuleArtifact,
) -> Result<(), OtaError> {
    verify_artifact_sig(root_pubkey, art)?;
    if let Some(rk) = release {
        let sig2 = art
            .sig2
            .as_ref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| OtaError::BadSignature)?;
        verify_bytes(rk.as_bytes(), &artifact_sign_message(art), sig2)?;
    }
    Ok(())
}

/// Canonical body for manifest signature (excludes `sig` field).
pub fn manifest_sign_message(m: &ReleaseManifest) -> Result<Vec<u8>, OtaError> {
    // Deterministic JSON without sig. Built as an ordered map (serde_json Map
    // sorts keys) so the bytes match the Python mirror in install.sh /
    // update_runtime_from_github.sh. `cli` is included ONLY when present, so
    // bodies of legacy manifests (no cli) stay byte-identical and their
    // existing signatures keep verifying.
    let mut map = serde_json::Map::new();
    map.insert("product".into(), serde_json::json!(m.product));
    map.insert("channel".into(), serde_json::json!(m.channel));
    map.insert("build_id".into(), serde_json::json!(m.build_id.as_deref().unwrap_or("")));
    map.insert("release_pubkey".into(), serde_json::json!(m.release_pubkey.as_deref().unwrap_or("")));
    map.insert(
        "runtime".into(),
        serde_json::json!({
            "version": m.runtime.version,
            "abi": m.runtime.abi,
            "asset": m.runtime.asset,
            "sha256": m.runtime.sha256,
        }),
    );
    map.insert(
        "fe".into(),
        serde_json::json!(m.fe.as_ref().map(|f| serde_json::json!({
            "asset": f.asset.clone(),
            "sha256": f.sha256.clone(),
        }))),
    );
    // Whole-bundle releases (greenpng 1.0.0+): per-file trees are signed too.
    // Included ONLY when present so legacy manifest bodies stay byte-identical.
    // spec_tree joins in 1.0.2+ (install.sh / updater Python mirrors iterate
    // the same key list and also include it only when present).
    // data_tree joins in 1.0.8+ (r100 templates + geoip mmdb — analyze 运行时
    // 数据, 同 spec 先例)。1.0.7 及更早的安装器/升级器镜像不含此键 →
    // 1.0.8+ 整包必须用新 install.sh (raw main) 安装; 面板 runtime OTA 在
    // 新 updater 落地后恢复。
    for (key, tree) in [
        ("fe_tree", &m.fe_tree),
        ("admin_tree", &m.admin_tree),
        ("spec_tree", &m.spec_tree),
        ("data_tree", &m.data_tree),
    ] {
        if let Some(t) = tree {
            let mut obj = serde_json::Map::new();
            if let Some(epoch) = &t.epoch {
                obj.insert("epoch".into(), serde_json::json!(epoch));
            }
            obj.insert(
                "files".into(),
                serde_json::to_value(&t.files).expect("tree files serialize"),
            );
            map.insert(key.into(), serde_json::Value::Object(obj));
        }
    }
    if let Some(c) = &m.cli {
        map.insert(
            "cli".into(),
            serde_json::json!({
                "version": c.version,
                "abi": c.abi,
                "asset": c.asset,
                "sha256": c.sha256,
            }),
        );
    }
    map.insert(
        "modules".into(),
        serde_json::json!(m.modules.iter().map(|a| serde_json::json!({
            "name": a.name,
            "version": a.version,
            "abi": a.abi,
            "requires_major": a.requires_major,
            "min_runtime_minor": a.min_runtime_minor,
            "max_runtime_minor": a.max_runtime_minor,
            "asset": a.asset,
            "sha256": a.sha256,
            "domain": a.domain,
            // intentionally omit per-module sig from manifest body (verified separately)
        })).collect::<Vec<_>>()),
    );
    Ok(serde_json::to_vec(&serde_json::Value::Object(map))?)
}

pub fn verify_manifest_sig(pubkey: &[u8], m: &ReleaseManifest) -> Result<(), OtaError> {
    let Some(sig) = m.sig.as_ref().filter(|s| !s.is_empty()) else {
        return Err(OtaError::Other("manifest sig missing".into()));
    };
    verify_bytes(pubkey, &manifest_sign_message(m)?, sig)
}

pub fn verify_file_sha256(path: &Path, expect_hex: &str) -> Result<(), OtaError> {
    let data = fs::read(path)?;
    let got = sha256_hex(&data);
    if got != expect_hex.to_ascii_lowercase() && got != expect_hex {
        return Err(OtaError::BadHash(path.display().to_string()));
    }
    Ok(())
}

pub fn check_compat(art: &ModuleArtifact, runtime_ver: &str) -> Result<ModuleMeta, OtaError> {
    let meta = art.to_meta();
    let rt = Version::parse(runtime_ver).map_err(|e| OtaError::Other(e.to_string()))?;
    meta.is_compatible(&rt, RUNTIME_ABI)
        .map_err(|e| OtaError::Incompatible(e.to_string()))?;
    Ok(meta)
}

/// Pick a module from a verified manifest by name (+ optional exact version).
///
/// Without a version, the highest **semantic** version wins — a lexicographic
/// string compare would wrongly select "6.0.9" over "6.0.28". Used by both the
/// service OTA path (`ota_install_remote`) and the CLI `module update`.
pub fn pick_module<'a>(
    man: &'a ReleaseManifest,
    name: &str,
    version: Option<&str>,
) -> Result<&'a ModuleArtifact, OtaError> {
    if let Some(v) = version {
        // Exact version requested: first match is the answer, and a wrong
        // version is an error (prefer explict failure over silently upgrading).
        return man
            .modules
            .iter()
            .find(|m| m.name == name && m.version == v)
            .ok_or_else(|| OtaError::Other(format!("module {name}@{v} not in remote manifest")));
    }
    // No version: pick the highest **semantic** version for the name — a
    // lexicographic string compare would wrongly select "6.0.9" over "6.0.28".
    let mut cands: Vec<_> = man.modules.iter().filter(|m| m.name == name).collect();
    cands.sort_by(|a, b| {
        match (a.to_meta().parse_version(), b.to_meta().parse_version()) {
            (Ok(va), Ok(vb)) => vb.cmp(&va),
            // Unparseable versions sort below any valid semver.
            (Ok(_), Err(_)) => std::cmp::Ordering::Less,
            (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
            (Err(_), Err(_)) => b.version.cmp(&a.version),
        }
    });
    cands
        .into_iter()
        .next()
        .ok_or_else(|| OtaError::Other(format!("module {name} not in remote manifest")))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalModuleState {
    pub name: String,
    pub version: String,
    pub path: PathBuf,
    pub active: bool,
}

pub struct OtaEngine {
    pub cfg: OtaConfig,
    /// Root of the extracted whole-bundle tree when the active manifest was
    /// resolved from a bundle archive (contains `manifest.json`, `bin/`,
    /// `modules/`, `fe/`, `admin/`). Asset lookups then prefer local files
    /// over HTTP downloads.
    pub bundle_dir: Option<PathBuf>,
}

impl OtaEngine {
    pub fn new(cfg: OtaConfig) -> Self {
        Self {
            cfg,
            bundle_dir: None,
        }
    }

    pub fn staging_dir(&self, name: &str, ver: &str) -> PathBuf {
        self.cfg.modules_dir.join("staging").join(name).join(ver)
    }

    pub fn versions_dir(&self, name: &str) -> PathBuf {
        self.cfg.modules_dir.join("versions").join(name)
    }

    pub fn active_link(&self, name: &str) -> PathBuf {
        self.cfg.modules_dir.join("active").join(name)
    }
    /// Enumerate the active module set straight from the markers: (name,
    /// version) for every `active/<name>` file whose target resolves. This is
    /// the node's real module universe — only `analyze` hot-dlopen-swaps, so
    /// the loaded registry is NOT a complete inventory (static-path modules
    /// are marker + next-boot). Unreadable markers are skipped.
    pub fn active_modules(&self) -> Vec<(String, String)> {
        let dir = self.cfg.modules_dir.join("active");
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for ent in rd.flatten() {
            let name = ent.file_name().to_string_lossy().to_string();
            if name.is_empty() {
                continue;
            }
            if let Ok(target) = std::fs::read_to_string(ent.path()) {
                if let Some(ver) = std::path::Path::new(target.trim())
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .filter(|v| !v.is_empty())
                {
                    out.push((name, ver));
                }
            }
        }
        out.sort();
        out
    }

    /// Resolve which manifest file to fetch for this host.
    /// Prefer `manifest-index.json` → arch-specific file; fall back to `manifest.json`.
    pub fn resolve_manifest_url(&self) -> Result<String, OtaError> {
        let base = self.cfg.release_base_url.trim_end_matches('/');
        let arch = normalize_host_arch(std::env::consts::ARCH);
        let index_url = format!("{base}/manifest-index.json");
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| OtaError::Http(e.to_string()))?;
        if let Ok(res) = client.get(&index_url).send() {
            if res.status().is_success() {
                if let Ok(idx) = res.json::<serde_json::Value>() {
                    if let Some(name) = idx
                        .pointer(&format!("/architectures/{arch}/manifest"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                    {
                        // Reject path escapes in index-provided names.
                        if name.contains('/') || name.contains("..") {
                            return Err(OtaError::Other(format!(
                                "manifest-index bad name: {name}"
                            )));
                        }
                        return Ok(format!("{base}/{name}"));
                    }
                    // Also try common aliases (amd64 → x86_64 already normalized).
                    if let Some(obj) = idx.get("architectures").and_then(|v| v.as_object()) {
                        for (k, v) in obj {
                            if normalize_host_arch(k) == arch {
                                if let Some(name) = v.get("manifest").and_then(|m| m.as_str()) {
                                    if !name.contains('/') && !name.contains("..") {
                                        return Ok(format!("{base}/{name}"));
                                    }
                                }
                            }
                        }
                    }
                    // Fallback: try well-known per-arch filename even if index incomplete.
                    let guess = format!("manifest-{arch}-linux-gnu.json");
                    let guess_url = format!("{base}/{guess}");
                    if let Ok(probe) = client.head(&guess_url).send() {
                        if probe.status().is_success() {
                            return Ok(guess_url);
                        }
                    }
                }
            }
        }
        Ok(format!("{base}/manifest.json"))
    }

    /// Resolve the whole-bundle entry for this host from `manifest-index.json`.
    /// Returns `(bundle_name, bundle_sha256)` when the index advertises a
    /// bundle for this arch, `None` for flat (per-file) releases.
    fn resolve_bundle_entry(&self) -> Result<Option<(String, String)>, OtaError> {
        let base = self.cfg.release_base_url.trim_end_matches('/');
        let arch = normalize_host_arch(std::env::consts::ARCH);
        let index_url = format!("{base}/manifest-index.json");
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| OtaError::Http(e.to_string()))?;
        let res = client
            .get(&index_url)
            .send()
            .map_err(|e| OtaError::Http(e.to_string()))?;
        if !res.status().is_success() {
            return Ok(None);
        }
        let idx: serde_json::Value = res.json().map_err(|e| OtaError::Http(e.to_string()))?;
        let entry: Option<&serde_json::Value> = idx
            .pointer(&format!("/architectures/{arch}"))
            .or_else(|| {
                // alias scan (amd64 → x86_64 etc. already normalized)
                idx.get("architectures").and_then(|v| v.as_object()).and_then(|obj| {
                    obj.iter()
                        .find(|(k, _)| normalize_host_arch(k) == arch)
                        .map(|(_, v)| v)
                })
            });
        let Some(entry) = entry else {
            return Ok(None);
        };
        let Some(name) = entry.get("bundle").and_then(|v| v.as_str()) else {
            return Ok(None);
        };
        if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
            return Err(OtaError::Other(format!("manifest-index bad bundle name: {name}")));
        }
        let sha = entry
            .get("bundle_sha256")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if sha.len() != 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(OtaError::Other(format!(
                "manifest-index bundle_sha256 missing/invalid for {name}"
            )));
        }
        Ok(Some((name.to_string(), sha)))
    }

    /// Download + verify + extract the whole-bundle archive for this host.
    /// Idempotent: reuses a previously extracted tree in the cache dir.
    /// Sets `self.bundle_dir` and returns it on success; `Ok(None)` when the
    /// release is flat (no bundle in the index).
    pub fn fetch_bundle_blocking(&mut self) -> Result<Option<PathBuf>, OtaError> {
        let Some((name, expect_sha)) = self.resolve_bundle_entry()? else {
            return Ok(None);
        };
        let cache = self
            .cfg
            .bundle_cache
            .clone()
            .unwrap_or_else(|| std::env::temp_dir().join("gr-ota-bundle"));
        fs::create_dir_all(&cache)?;
        let stem = name.trim_end_matches(".tar.gz");
        let extracted = cache.join(format!("{stem}.d"));
        // Reuse a complete extraction (manifest present = complete marker).
        if extracted.join("manifest.json").is_file() {
            let root = bundle_tree_root(&extracted);
            self.bundle_dir = Some(root.clone());
            return Ok(Some(root));
        }
        let tgz = cache.join(&name);
        if !tgz.is_file() || sha256_file_hex(&tgz)? != expect_sha {
            let base = self.cfg.release_base_url.trim_end_matches('/');
            let url = format!("{base}/{name}");
            let client = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(600))
                .build()
                .map_err(|e| OtaError::Http(e.to_string()))?;
            let bytes = client
                .get(&url)
                .send()
                .map_err(|e| OtaError::Http(e.to_string()))?
                .error_for_status()
                .map_err(|e| OtaError::Http(e.to_string()))?
                .bytes()
                .map_err(|e| OtaError::Http(e.to_string()))?;
            let got = gr_abi::sha256_hex(&bytes);
            if got != expect_sha {
                return Err(OtaError::Other(format!(
                    "bundle sha256 mismatch: got {got} expect {expect_sha} ({name})"
                )));
            }
            fs::write(&tgz, &bytes)?;
        }
        let stage = cache.join(format!("{stem}.extract-{}", std::process::id()));
        let _ = fs::remove_dir_all(&stage);
        fs::create_dir_all(&stage)?;
        safe_extract_tar_gz(&tgz, &stage).map_err(OtaError::Other)?;
        let _ = fs::remove_dir_all(&extracted);
        fs::rename(&stage, &extracted)?;
        let root = bundle_tree_root(&extracted);
        self.bundle_dir = Some(root.clone());
        Ok(Some(root))
    }

    /// Download manifest JSON for this host arch.
    /// When pubkey is configured and manifest carries `sig`, verify it (R-03).
    /// Release verifying key parsed from `cfg.release_key`, if a release was
    /// bound by a verified manifest certificate.
    fn release_vk(&self) -> Result<Option<ed25519_dalek::VerifyingKey>, OtaError> {
        match self.cfg.release_key.as_ref() {
            None => Ok(None),
            Some(bytes) => {
                if bytes.len() != 32 {
                    return Err(OtaError::Key("release_key must be 32 bytes".into()));
                }
                let mut arr = [0u8; 32];
                arr.copy_from_slice(bytes);
                Ok(Some(
                    ed25519_dalek::VerifyingKey::from_bytes(&arr)
                        .map_err(|e| OtaError::Key(e.to_string()))?,
                ))
            }
        }
    }

    /// P1-5: boot-time integrity self-check of every *active* module.
    /// For each `active/<name>` marker: read the staged version dir, load its
    /// signed `meta.json`, verify the artifact chain (root + release key when
    /// bound) and re-hash the shipped `.so`. Returns the verified modules.
    /// Modules staged before this feature (no `meta.json`) are listed under
    /// `legacy` in the returned summary instead of failing the boot.
    pub fn verify_active_modules(&self) -> Result<serde_json::Value, OtaError> {
        let released = self.release_vk()?;
        let active_dir = self.cfg.modules_dir.join("active");
        let mut ok: Vec<String> = Vec::new();
        let mut legacy: Vec<String> = Vec::new();
        if active_dir.is_dir() {
            for ent in fs::read_dir(&active_dir)? {
                let ent = ent?;
                if !ent.file_type()?.is_file() {
                    continue;
                }
                let name = ent.file_name().to_string_lossy().to_string();
                let marker = fs::read_to_string(ent.path())?;
                let ver_dir = PathBuf::from(marker.trim());
                if !ver_dir.is_dir() {
                    return Err(OtaError::Other(format!(
                        "active marker {name} → missing {ver_dir:?}"
                    )));
                }
                let meta = ver_dir.join("meta.json");
                let Some(art): Option<ModuleArtifact> = fs::read_to_string(&meta)
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok())
                else {
                    legacy.push(format!("{name} (staged pre-P1-5, no meta.json)"));
                    continue;
                };
                verify_artifact_sig_chain(
                    &self.cfg.pubkey_bytes,
                    released.as_ref(),
                    &art,
                )?;
                verify_file_sha256(&ver_dir.join(&art.asset), &art.sha256)?;
                ok.push(format!("{}@{}", art.name, art.version));
            }
        }
        Ok(serde_json::json!({
            "verified": ok,
            "legacy_unverifiable": legacy,
            "release_key_bound": released.is_some(),
        }))
    }

    /// Persist the verified per-release binding (build_id + release pubkey)
    /// next to the modules tree, so `verify_active_modules` at next boot can
    /// re-check with the release key offline. Call only after chain
    /// verification of the manifest succeeded.
    pub fn write_release_binding(&mut self, man: &ReleaseManifest) -> Result<(), OtaError> {
        let Some(pk) = man.release_pubkey.as_ref().filter(|s| !s.is_empty()) else {
            return Ok(());
        };
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        fs::create_dir_all(&self.cfg.modules_dir)?;
        let binding = serde_json::json!({
            "build_id": man.build_id.as_deref().unwrap_or(""),
            "release_pubkey": pk,
            "verified_ms": now_ms,
        });
        fs::write(
            self.cfg.modules_dir.join("release_binding.json"),
            serde_json::to_string_pretty(&binding)?,
        )?;
        // Load the persisted binding for offline boot checks.
        self.load_release_binding()?;
        Ok(())
    }

    /// Read the persisted release binding (written by `write_release_binding`)
    /// into `cfg.release_key`. No-op when absent (legacy root-only installs).
    pub fn load_release_binding(&mut self) -> Result<(), OtaError> {
        let path = self.cfg.modules_dir.join("release_binding.json");
        let Ok(body) = fs::read_to_string(&path) else {
            return Ok(());
        };
        let v: serde_json::Value = serde_json::from_str(&body)?;
        let Some(pk) = v.get("release_pubkey").and_then(|p| p.as_str()) else {
            return Ok(());
        };
        let raw = hex::decode(pk).map_err(|_| OtaError::Key("bound release_pubkey not hex".into()))?;
        if raw.len() != 32 {
            return Err(OtaError::Key("bound release_pubkey must be 32 bytes".into()));
        }
        self.cfg.release_key = Some(raw);
        Ok(())
    }

    pub fn fetch_manifest_blocking(&mut self) -> Result<ReleaseManifest, OtaError> {
        // Whole-bundle releases: manifest.json lives inside the verified
        // bundle archive (index pins the bundle sha256). Flat releases keep
        // the per-file manifest URL flow.
        let (body, source) = if let Some(root) = self.fetch_bundle_blocking()? {
            (
                fs::read_to_string(root.join("manifest.json"))?,
                root.join("manifest.json").display().to_string(),
            )
        } else {
            let url = self.resolve_manifest_url()?;
            let text = reqwest::blocking::Client::new()
                .get(&url)
                .send()
                .map_err(|e| OtaError::Http(e.to_string()))?
                .error_for_status()
                .map_err(|e| OtaError::Http(e.to_string()))?
                .text()
                .map_err(|e| OtaError::Http(e.to_string()))?;
            (text, url)
        };
        let man: ReleaseManifest = serde_json::from_str(&body)?;
        // Soft check: if manifest declares arch, it must match host.
        if let Some(ref declared) = man.arch {
            let host = normalize_host_arch(std::env::consts::ARCH);
            if normalize_host_arch(declared) != host {
                return Err(OtaError::Incompatible(format!(
                    "manifest arch {declared} != host {host} (source={source})"
                )));
            }
        }
        if self.cfg.pubkey_bytes.is_empty() {
            // GR naming migration (docs/13): GR_REQUIRE_MANIFEST_SIG preferred,
            // GR_* / GR_* fall back until 8.0 rollout completes.
            if gr_abi::env::flag("REQUIRE_MANIFEST_SIG") {
                return Err(OtaError::Key(
                    "GR_REQUIRE_MANIFEST_SIG=1 but OTA pubkey is empty".into(),
                ));
            }
        } else {
            let require = gr_abi::env::flag("REQUIRE_MANIFEST_SIG");
            if require || man.sig.as_ref().map(|s| !s.is_empty()).unwrap_or(false) {
                // P0-2: chain verification — root sig + (when present) the
                // release certificate; the verified release key is remembered
                // for artifact staging / boot checks.
                let rel = verify_manifest_chain(&self.cfg.pubkey_bytes, &man)?;
                self.cfg.release_key = rel.map(|k| k.to_bytes().to_vec());
            }
        }
        Ok(man)
    }

    pub fn load_manifest_file(&mut self, path: &Path) -> Result<ReleaseManifest, OtaError> {
        let body = fs::read_to_string(path)?;
        let man: ReleaseManifest = serde_json::from_str(&body)?;
        // Same chain policy as fetch: any present signature must verify
        // (root + cert chain). Keeps local staging runs honest in CI.
        if !self.cfg.pubkey_bytes.is_empty()
            && man.sig.as_ref().map(|s| !s.is_empty()).unwrap_or(false)
        {
            let rel = verify_manifest_chain(&self.cfg.pubkey_bytes, &man)?;
            self.cfg.release_key = rel.map(|k| k.to_bytes().to_vec());
        }
        Ok(man)
    }

    /// Stage one module: verify sig chain + hash, copy into versions tree.
    pub fn stage_local_file(
        &mut self,
        art: &ModuleArtifact,
        src: &Path,
        runtime_ver: &str,
    ) -> Result<PathBuf, OtaError> {
        self.load_release_binding()?;
        check_compat(art, runtime_ver)?;
        // P0-2: when a release key is bound (verified manifest, in-memory or
        // persisted binding), artifacts must carry the release-key sig2 in
        // addition to the root sig.
        verify_artifact_sig_chain(&self.cfg.pubkey_bytes, self.release_vk()?.as_ref(), art)?;
        verify_file_sha256(src, &art.sha256)?;

        let dest_dir = self.versions_dir(&art.name).join(&art.version);
        fs::create_dir_all(&dest_dir)?;
        let dest = dest_dir.join(&art.asset);
        // Never write in-place over a possibly-mmap'd (dlopen'd) .so — that SEGVs.
        // Write a sibling temp file then rename so existing mappings keep the old inode.
        let tmp = dest_dir.join(format!(
            ".{}.tmp.{}",
            art.asset,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::copy(src, &tmp)?;
        fs::rename(&tmp, &dest)?;
        // write meta sidecar
        let meta_path = dest_dir.join("meta.json");
        fs::write(meta_path, serde_json::to_vec_pretty(&art)?)?;
        Ok(dest)
    }

    /// Point `active/<name>` at a staged version (symlink or marker file).
    ///
    /// After activation, older versions are pruned so that the newest
    /// `KEEP_ROLLBACK_VERSIONS` (plus the active one) survive — the customer
    /// can always roll back via another `activate` call with an older version.
    pub fn activate(&self, name: &str, version: &str) -> Result<PathBuf, OtaError> {
        let ver_dir = self.versions_dir(name).join(version);
        if !ver_dir.is_dir() {
            return Err(OtaError::Other(format!("version not staged: {name}@{version}")));
        }
        let active_dir = self.cfg.modules_dir.join("active");
        fs::create_dir_all(&active_dir)?;
        let link = self.active_link(name);
        let marker = format!("{}\n", ver_dir.display());
        fs::write(&link, marker)?;
        // iss/opus5 06-P1-7: keep the last N pre-update versions for rollback.
        let _ = self.prune_versions(name, KEEP_ROLLBACK_VERSIONS);
        Ok(ver_dir)
    }

    /// Delete module versions beyond the active one and the newest
    /// `keep_old` others (semver-ordered, newest first). Returns how many
    /// version dirs were removed. Missing/empty dir → Ok(0).
    pub fn prune_versions(&self, name: &str, keep_old: usize) -> Result<usize, OtaError> {
        let dir = self.versions_dir(name);
        if !dir.is_dir() {
            return Ok(0);
        }
        let mut versions: Vec<String> = Vec::new();
        for ent in fs::read_dir(&dir)? {
            let ent = ent?;
            if ent.file_type()?.is_dir() {
                versions.push(ent.file_name().to_string_lossy().to_string());
            }
        }
        let active_ver = fs::read_to_string(self.active_link(name))
            .ok()
            .and_then(|s| {
                PathBuf::from(s.trim())
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
            })
            .unwrap_or_default();
        // Descending order: newest semver first; non-semver dirs last (lexical).
        versions.sort_by(|a, b| {
            use semver::Version;
            match (Version::parse(a), Version::parse(b)) {
                (Ok(x), Ok(y)) => y.cmp(&x),
                (Ok(_), Err(_)) => std::cmp::Ordering::Less,
                (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
                (Err(_), Err(_)) => b.cmp(a),
            }
        });
        let mut kept = 0usize;
        let mut removed = 0usize;
        for v in &versions {
            if *v == active_ver {
                continue;
            }
            if kept < keep_old {
                kept += 1;
                continue;
            }
            let target = dir.join(v);
            if let Err(e) = fs::remove_dir_all(&target) {
                eprintln!("ota prune: skip {name}@{v}: {e}");
                continue;
            }
            removed += 1;
        }
        Ok(removed)
    }

    pub fn list_local(&self) -> Result<Vec<LocalModuleState>, OtaError> {
        let mut out = Vec::new();
        let versions_root = self.cfg.modules_dir.join("versions");
        if !versions_root.is_dir() {
            return Ok(out);
        }
        for name_ent in fs::read_dir(versions_root)? {
            let name_ent = name_ent?;
            if !name_ent.file_type()?.is_dir() {
                continue;
            }
            let name = name_ent.file_name().to_string_lossy().to_string();
            let active_ver = fs::read_to_string(self.active_link(&name))
                .ok()
                .and_then(|s| {
                    let p = PathBuf::from(s.trim());
                    p.file_name()
                        .map(|f| f.to_string_lossy().to_string())
                });
            for ver_ent in fs::read_dir(name_ent.path())? {
                let ver_ent = ver_ent?;
                if !ver_ent.file_type()?.is_dir() {
                    continue;
                }
                let version = ver_ent.file_name().to_string_lossy().to_string();
                let active = active_ver.as_ref() == Some(&version);
                out.push(LocalModuleState {
                    name: name.clone(),
                    version,
                    path: ver_ent.path(),
                    active,
                });
            }
        }
        Ok(out)
    }

    pub fn download_asset_blocking(&self, art: &ModuleArtifact, dest: &Path) -> Result<(), OtaError> {
        // Whole-bundle releases: the module .so ships inside the extracted
        // bundle tree. The manifest `asset` is the SIGNED flat name; the
        // physical path is modules/<asset> (legacy trees may also carry the
        // flat file at the root — try both).
        if let Some(bd) = &self.bundle_dir {
            let p = bd.join("modules").join(&art.asset);
            let p = if p.is_file() { p } else { bd.join(&art.asset) };
            if p.is_file() {
                let bytes = fs::read(&p)?;
                let got = gr_abi::sha256_hex(&bytes);
                if !got.eq_ignore_ascii_case(&art.sha256) {
                    return Err(OtaError::BadHash(format!(
                        "{} (bundle local copy: got {got} expect {})",
                        art.asset, art.sha256
                    )));
                }
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(dest, &bytes)?;
                return Ok(());
            }
        }
        let url = format!(
            "{}/{}",
            self.cfg.release_base_url.trim_end_matches('/'),
            art.asset
        );
        let bytes = reqwest::blocking::Client::new()
            .get(&url)
            .send()
            .map_err(|e| OtaError::Http(e.to_string()))?
            .error_for_status()
            .map_err(|e| OtaError::Http(e.to_string()))?
            .bytes()
            .map_err(|e| OtaError::Http(e.to_string()))?;
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(dest, &bytes)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gr_abi::ModuleArtifact;

    fn art(name: &str, version: &str) -> ModuleArtifact {
        ModuleArtifact {
            name: name.into(),
            version: version.into(),
            abi: RUNTIME_ABI,
            requires_major: gr_abi::PRODUCT_MAJOR,
            min_runtime_minor: 0,
            max_runtime_minor: None,
            asset: format!("lib{name}-{version}.so"),
            sha256: "0".repeat(64),
            sig: String::new(),
            sig2: None,
            domain: name.into(),
        }
    }

    fn tmp_engine() -> (tempfile::TempDir, OtaEngine) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = OtaConfig {
            release_base_url: "https://example.invalid/releases".into(),
            modules_dir: dir.path().join("modules"),
            pubkey_bytes: vec![0u8; 32],
            release_key: None,
            bundle_cache: None,
        };
        (dir, OtaEngine { cfg, bundle_dir: None })
    }

    #[test]
    fn pick_module_semver_highest_without_version() {
        let man = ReleaseManifest {
            product: "green-v7".into(),
            channel: "stable".into(),
            arch: Some("x86_64".into()),
            triple: Some("x86_64-linux-gnu".into()),
            build_id: None,
            release_pubkey: None,
            release_cert: None,
            runtime: gr_abi::RuntimeManifest {
                version: "7.0.0".into(),
                abi: RUNTIME_ABI,
                asset: None,
                sha256: None,
            },
            fe: None,
            fe_tree: None,
            admin_tree: None,
            spec_tree: None,
            data_tree: None,
            cli: None,
            sig: None,
            modules: vec![
                art("analyze", "6.0.9"),
                art("analyze", "6.0.28"),
                art("identity", "7.0.1"),
            ],
        };
        let got = pick_module(&man, "analyze", None).unwrap();
        assert_eq!(got.version, "6.0.28", "lexicographic compare would pick 6.0.9");
        assert_eq!(
            pick_module(&man, "identity", None).unwrap().version,
            "7.0.1"
        );
    }

    #[test]
    fn pick_module_exact_version_and_errors() {
        let man = ReleaseManifest {
            product: "green-v7".into(),
            channel: "stable".into(),
            arch: None,
            triple: None,
            build_id: None,
            release_pubkey: None,
            release_cert: None,
            runtime: gr_abi::RuntimeManifest {
                version: "7.0.0".into(),
                abi: RUNTIME_ABI,
                asset: None,
                sha256: None,
            },
            fe: None,
            fe_tree: None,
            admin_tree: None,
            spec_tree: None,
            data_tree: None,
            cli: None,
            sig: None,
            modules: vec![art("identity", "7.0.1"), art("identity", "7.0.2")],
        };
        assert_eq!(
            pick_module(&man, "identity", Some("7.0.1")).unwrap().version,
            "7.0.1"
        );
        assert!(
            pick_module(&man, "identity", Some("9.9.9")).is_err(),
            "missing exact version must error"
        );
        assert!(pick_module(&man, "nosuch", None).is_err());
    }

    #[test]
    fn prune_keeps_active_plus_three() {
        let (_dir, eng) = tmp_engine();
        let name = "analyze";
        let vdir = eng.versions_dir(name);
        for v in ["1.0.0", "2.0.0", "3.0.0", "4.0.0", "5.0.0", "6.0.0"] {
            std::fs::create_dir_all(vdir.join(v)).unwrap();
        }
        // Activate 6.0.0 → prune keeps active + newest 3 (5.0.0, 4.0.0, 3.0.0)
        // and removes 1.0.0 + 2.0.0.
        eng.activate(name, "6.0.0").unwrap();
        let mut left: Vec<String> = std::fs::read_dir(&vdir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        left.sort();
        assert_eq!(
            left,
            ["3.0.0", "4.0.0", "5.0.0", "6.0.0"],
            "rollback set = active + last 3 pre-update versions"
        );
        // Rollback via re-activate to an older kept version works after pruning.
        eng.activate(name, "3.0.0").unwrap();
        assert_eq!(
            std::fs::read_to_string(eng.active_link(name)).unwrap().trim(),
            eng.versions_dir(name).join("3.0.0").display().to_string(),
            "activation still points at the chosen older version"
        );
    }

    #[test]
    fn activate_prunes_older_beyond_keep() {
        let (_dir, eng) = tmp_engine();
        let name = "lb";
        let vdir = eng.versions_dir(name);
        for v in ["0.9.0", "1.0.0", "1.1.0", "1.2.0", "1.3.0", "1.4.0"] {
            std::fs::create_dir_all(vdir.join(v)).unwrap();
        }
        // activate itself prunes (KEEP_ROLLBACK_VERSIONS=3): the two oldest
        // versions (0.9.0, 1.0.0) must be gone afterwards; a redundant manual
        // prune then finds nothing.
        eng.activate(name, "1.4.0").unwrap();
        let n_after = eng.prune_versions(name, 3).unwrap();
        assert_eq!(n_after, 0, "activate already pruned; nothing left to remove");
        let mut left: Vec<String> = std::fs::read_dir(&vdir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        left.sort();
        assert_eq!(left, ["1.1.0", "1.2.0", "1.3.0", "1.4.0"]);
    }

    #[test]
    fn normalize_arch_aliases() {
        assert_eq!(normalize_host_arch("amd64"), "x86_64");
        assert_eq!(normalize_host_arch("arm64"), "aarch64");
        assert_eq!(normalize_host_arch("x86_64"), "x86_64");
    }

    #[test]
    fn sign_verify_roundtrip() {
        let (sk, vk) = generate_signing_keypair();
        let art = ModuleArtifact {
            name: "analyze".into(),
            version: "7.0.0".into(),
            abi: 1,
            requires_major: gr_abi::PRODUCT_MAJOR,
            min_runtime_minor: 0,
            max_runtime_minor: None,
            asset: "libgr_analyze.so".into(),
            sha256: "abc".into(),
            sig: String::new(),
            sig2: None,
            domain: "analyze".into(),
        };
        let msg = artifact_sign_message(&art);
        let sig = sign_bytes(&sk, &msg);
        verify_bytes(vk.as_bytes(), &msg, &sig).unwrap();
        // v1 legacy still verifies via verify_artifact_sig fallback path
        let mut art_v1 = art.clone();
        art_v1.sig = sign_bytes(&sk, &artifact_sign_message_v1(&art_v1));
        verify_artifact_sig(vk.as_bytes(), &art_v1).unwrap();
    }

    #[test]
    fn https_only_reject_is_caller_policy_manifest_sig_roundtrip() {
        let (sk, vk) = generate_signing_keypair();
        let man = ReleaseManifest {
            product: "green-v6".into(),
            channel: "stable".into(),
            arch: Some("x86_64".into()),
            triple: Some("x86_64-linux-gnu".into()),
            runtime: gr_abi::RuntimeManifest {
                version: "6.0.9".into(),
                abi: 1,
                asset: Some("gr-service".into()),
                sha256: Some("deadbeef".into()),
            },
            modules: vec![],
            fe: Some(gr_abi::FeManifest {
                asset: "fe-6.0.9.tgz".into(),
                sha256: "cafebabe".into(),
                sig: None,
            }),
            cli: None,
            fe_tree: None,
            admin_tree: None,
            spec_tree: None,
            data_tree: None,
            sig: None,
            build_id: None,
            release_pubkey: None,
            release_cert: None,
        };
        let msg = manifest_sign_message(&man).unwrap();
        let sig = sign_bytes(&sk, &msg);
        let mut man2 = man;
        man2.sig = Some(sig);
        verify_manifest_sig(vk.as_bytes(), &man2).unwrap();
    }

    #[test]
    fn manifest_body_cli_conditional() {
        // P1-4: the canonical body includes `cli` ONLY when the manifest does.
        // Adding the field must not change the legacy (no cli) body bytes, so
        // previously signed releases keep verifying after this hardening.
        let (_, _) = generate_signing_keypair();
        let man = ReleaseManifest {
            product: "green-v7".into(),
            channel: "stable".into(),
            arch: Some("x86_64".into()),
            triple: Some("x86_64-linux-gnu".into()),
            runtime: gr_abi::RuntimeManifest {
                version: "7.0.2".into(),
                abi: 1,
                asset: Some("gr-service-7.0.2-x86_64-linux-gnu".into()),
                sha256: Some("aa".into()),
            },
            modules: vec![],
            fe: None,
            fe_tree: None,
            admin_tree: None,
            spec_tree: None,
            data_tree: None,
            cli: None,
            sig: None,
            build_id: None,
            release_pubkey: None,
            release_cert: None,
        };
        let legacy = manifest_sign_message(&man).unwrap();
        let legacy_json: serde_json::Value = serde_json::from_slice(&legacy).unwrap();
        assert!(legacy_json.get("cli").is_none(), "legacy body must not carry cli");

        let mut with_cli = man.clone();
        with_cli.cli = Some(gr_abi::RuntimeManifest {
            version: "7.0.2".into(),
            abi: 1,
            asset: Some("gr-7.0.2-x86_64-linux-gnu".into()),
            sha256: Some("bb".into()),
        });
        let body = manifest_sign_message(&with_cli).unwrap();
        let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(j["cli"]["asset"], "gr-7.0.2-x86_64-linux-gnu");
        assert_eq!(j["cli"]["sha256"], "bb");
        assert_eq!(j["runtime"]["version"], "7.0.2");
        // Dropping cli back must reproduce the legacy bytes exactly.
        let mut back = with_cli.clone();
        back.cli = None;
        assert_eq!(manifest_sign_message(&back).unwrap(), legacy);
    }

    #[test]
    fn manifest_body_spec_tree_conditional_and_roundtrip() {
        // 1.0.2 hardening: `spec_tree` joins the canonical body ONLY when
        // present (legacy bodies stay byte-identical), and — the actual
        // 1.0.1 regression — the sign-manifest flow (gr-cli) parses the
        // build-script JSON into ReleaseManifest and rewrites it; before the
        // struct carried the field that round-trip silently dropped it, so
        // bundles shipped spec/ physically present but unsigned/absent in
        // manifest.json.
        let man = ReleaseManifest {
            product: "greenpng".into(),
            channel: "stable".into(),
            arch: Some("x86_64".into()),
            triple: Some("x86_64-linux-gnu".into()),
            runtime: gr_abi::RuntimeManifest {
                version: "1.0.2".into(),
                abi: 1,
                asset: Some("bin/gr-service".into()),
                sha256: Some("aa".into()),
            },
            modules: vec![],
            fe: None,
            fe_tree: None,
            admin_tree: None,
            spec_tree: None,
            data_tree: None,
            cli: None,
            sig: None,
            build_id: None,
            release_pubkey: None,
            release_cert: None,
        };
        let legacy = manifest_sign_message(&man).unwrap();
        let legacy_json: serde_json::Value = serde_json::from_slice(&legacy).unwrap();
        assert!(legacy_json.get("spec_tree").is_none(), "legacy body must not carry spec_tree");

        let mut with_spec = man.clone();
        with_spec.spec_tree = Some(gr_abi::TreeManifest {
            epoch: None,
            files: [
                ("spec/sources.json".to_string(), "11".to_string()),
                ("spec/weights.json".to_string(), "22".to_string()),
            ]
            .into_iter()
            .collect(),
        });
        let body = manifest_sign_message(&with_spec).unwrap();
        let j: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(j["spec_tree"]["files"]["spec/sources.json"], "11");
        assert_eq!(j["spec_tree"]["files"]["spec/weights.json"], "22");
        // dropping it back must reproduce legacy bytes exactly
        let mut back = with_spec.clone();
        back.spec_tree = None;
        assert_eq!(manifest_sign_message(&back).unwrap(), legacy);

        // sign-manifest round-trip: JSON → struct → pretty JSON → struct
        // must keep spec_tree (this is what gr-cli does on every build).
        let raw = serde_json::to_string(&with_spec).unwrap();
        let reread: ReleaseManifest = serde_json::from_str(&raw).unwrap();
        assert!(
            reread.spec_tree.is_some(),
            "spec_tree must survive the serde round-trip (1.0.1 regression)"
        );
        assert_eq!(
            reread.spec_tree.as_ref().unwrap().files["spec/sources.json"],
            "11"
        );
        let rewritten = serde_json::to_string_pretty(&reread).unwrap();
        let final_man: ReleaseManifest = serde_json::from_str(&rewritten).unwrap();
        assert_eq!(
            serde_json::to_value(&final_man.spec_tree).unwrap(),
            serde_json::to_value(&with_spec.spec_tree).unwrap()
        );
    }

    #[test]
    fn release_cert_chain_roundtrip() {
        // root
        let (root_sk, root_vk) = generate_signing_keypair();
        // per-release
        let (rel_sk, rel_vk) = generate_signing_keypair();
        let build_id = "b1234";
        let pk_hex = hex::encode(rel_vk.as_bytes());
        let man = ReleaseManifest {
            product: "green-v6".into(),
            channel: "stable".into(),
            arch: None,
            triple: None,
            runtime: gr_abi::RuntimeManifest {
                version: "6.0.28".into(),
                abi: 1,
                asset: Some("gr-service".into()),
                sha256: Some("deadbeef".into()),
            },
            modules: vec![],
            fe: None,
            fe_tree: None,
            admin_tree: None,
            spec_tree: None,
            data_tree: None,
            cli: None,
            sig: None,
            build_id: Some(build_id.into()),
            release_pubkey: Some(pk_hex.clone()),
            release_cert: None,
        };
        // cert: root signs (pubkey||version||build_id)
        let cert_msg = release_cert_message("6.0.28", build_id, &pk_hex);
        let cert = sign_bytes(&root_sk, &cert_msg);
        let mut man2 = man;
        man2.release_cert = Some(cert);
        // manifest sig: root
        let msg = manifest_sign_message(&man2).unwrap();
        man2.sig = Some(sign_bytes(&root_sk, &msg));

        let rel = verify_manifest_chain(root_vk.as_bytes(), &man2).unwrap();
        let rel = rel.expect("release key present");
        assert_eq!(rel.as_bytes(), rel_vk.as_bytes());

        // artifact double-signed: root sig + release sig2
        let mut art = ModuleArtifact {
            name: "analyze".into(),
            version: "7.0.0".into(),
            abi: 1,
            requires_major: gr_abi::PRODUCT_MAJOR,
            min_runtime_minor: 0,
            max_runtime_minor: None,
            asset: "libgr_analyze.so".into(),
            sha256: "abc".into(),
            sig: String::new(),
            sig2: None,
            domain: "analyze".into(),
        };
        art.sig = sign_bytes(&root_sk, &artifact_sign_message(&art));
        art.sig2 = Some(sign_bytes(&rel_sk, &artifact_sign_message(&art)));
        verify_artifact_sig_chain(root_vk.as_bytes(), Some(&rel), &art).unwrap();

        // tampered cert message → chain must fail
        let bad = release_cert_message("6.0.27", build_id, &pk_hex);
        let man_bad_cert = {
            let mut m = man2.clone();
            m.release_cert = Some(sign_bytes(&root_sk, &bad));
            m
        };
        assert!(verify_manifest_chain(root_vk.as_bytes(), &man_bad_cert).is_err());

        // release key mismatch on sig2 → fail
        let (other_sk, _other_vk) = generate_signing_keypair();
        let mut art_bad = art.clone();
        art_bad.sig2 = Some(sign_bytes(&other_sk, &artifact_sign_message(&art_bad)));
        assert!(verify_artifact_sig_chain(root_vk.as_bytes(), Some(&rel), &art_bad).is_err());

        // missing sig2 when release key bound → fail
        let mut art_no2 = art.clone();
        art_no2.sig2 = None;
        assert!(verify_artifact_sig_chain(root_vk.as_bytes(), Some(&rel), &art_no2).is_err());
    }

    #[test]
    fn boot_verify_active_modules_chain_and_binding() {
        // Build a fully signed release: root + per-release key + cert + manifest.
        let (root_sk, root_vk) = generate_signing_keypair();
        let (rel_sk, rel_vk) = generate_signing_keypair();
        let build_id = "boot-1";
        let pk_hex = hex::encode(rel_vk.as_bytes());

        let dir = std::env::temp_dir().join(format!(
            "gr_ota_boot_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let modules_dir = dir.join("modules");
        std::fs::create_dir_all(&modules_dir).unwrap();

        let mut eng = OtaEngine::new(OtaConfig {
            release_base_url: String::new(),
            modules_dir: modules_dir.clone(),
            pubkey_bytes: root_vk.as_bytes().to_vec(),
            release_key: None,
            bundle_cache: None,
        });

        // Stage a module artifact (double-signed) + fake .so, then activate.
        // Historical-shape fixture: pairs with the 8.0.0 runtime below, so the
        // major is pinned literally (PRODUCT_MAJOR is 1 on the greenpng line).
        let mut art = ModuleArtifact {
            name: "analyze".into(),
            version: "8.0.0".into(),
            abi: 1,
            requires_major: 8,
            min_runtime_minor: 0,
            max_runtime_minor: None,
            asset: "libgr_analyze.so".into(),
            sha256: String::new(),
            sig: String::new(),
            sig2: None,
            domain: "analyze".into(),
        };
        let body = b"fake module bytes for boot verify";
        art.sha256 = sha256_hex(body);
        let art_path = dir.join(&art.asset);
        std::fs::write(&art_path, body).unwrap();
        art.sig = sign_bytes(&root_sk, &artifact_sign_message(&art));
        art.sig2 = Some(sign_bytes(&rel_sk, &artifact_sign_message(&art)));

        // Persist binding (simulates a verified manifest fetch)
        let man = ReleaseManifest {
            product: "green-v6".into(),
            channel: "stable".into(),
            arch: None,
            triple: None,
            runtime: gr_abi::RuntimeManifest {
                version: "8.0.0".into(),
                abi: 1,
                asset: Some("gr-service".into()),
                sha256: None,
            },
            modules: vec![art.clone()],
            fe: None,
            fe_tree: None,
            admin_tree: None,
            spec_tree: None,
            data_tree: None,
            cli: None,
            sig: Some(sign_bytes(&root_sk, &manifest_sign_message(&ReleaseManifest {
                product: "green-v6".into(),
                channel: "stable".into(),
                arch: None,
                triple: None,
                runtime: gr_abi::RuntimeManifest {
                    version: "8.0.0".into(),
                    abi: 1,
                    asset: Some("gr-service".into()),
                    sha256: None,
                },
                modules: vec![art.clone()],
                fe: None,
                fe_tree: None,
                admin_tree: None,
                spec_tree: None,
            data_tree: None,
                cli: None,
                sig: None,
                build_id: Some(build_id.into()),
                release_pubkey: Some(pk_hex.clone()),
                release_cert: Some(sign_bytes(
                    &root_sk,
                    &release_cert_message("8.0.0", build_id, &pk_hex),
                )),
            }).unwrap())),
            build_id: Some(build_id.into()),
            release_pubkey: Some(pk_hex.clone()),
            release_cert: Some(sign_bytes(
                &root_sk,
                &release_cert_message("8.0.0", build_id, &pk_hex),
            )),
        };
        // Chain-verify the fixture manifest first (proves fixture consistency).
        verify_manifest_chain(root_vk.as_bytes(), &man).unwrap();
        // Fresh engine adopts the binding like boot does.
        let mut eng2 = OtaEngine::new(OtaConfig {
            release_base_url: String::new(),
            modules_dir: modules_dir.clone(),
            pubkey_bytes: root_vk.as_bytes().to_vec(),
            release_key: None,
            bundle_cache: None,
        });
        eng.write_release_binding(&man).unwrap();
        eng2.load_release_binding().unwrap();
        assert_eq!(eng2.release_vk().unwrap().unwrap().as_bytes(), rel_vk.as_bytes());

        // Stage via the adopted engine (requires sig2) and activate.
        let dest = eng2
            .stage_local_file(&art, &art_path, "8.0.0")
            .unwrap();
        eng2.activate("analyze", "8.0.0").unwrap();

        // Boot verification: chain + hash OK.
        let v = eng2.verify_active_modules().unwrap();
        assert_eq!(
            v["verified"].as_array().unwrap().len(),
            1,
            "one verified module: {v}"
        );
        assert!(v["legacy_unverifiable"].as_array().unwrap().is_empty());
        assert_eq!(v["release_key_bound"], true);

        // Tamper the shipped .so → boot verify must fail.
        let so = dest.parent().unwrap().join(&art.asset);
        std::fs::write(&so, b"tampered!").unwrap();
        assert!(eng2.verify_active_modules().is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
