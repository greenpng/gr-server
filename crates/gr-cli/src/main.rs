//! GR CLI: install bootstrap, keygen, status, module stage/activate.

use clap::{Parser, Subcommand};
use gr_admin::AdminHub;
use gr_ota::{
    artifact_sign_message, generate_signing_keypair, manifest_sign_message, release_cert_message,
    sign_bytes, verify_manifest_chain, OtaConfig, OtaEngine,
};
use gr_abi::{sha256_hex, ModuleArtifact, ReleaseManifest};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "gr-cli", about = "Green V7 ops CLI")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Initialize data dir + random admin console path/user/password
    Install {
        #[arg(long, default_value = "data")]
        data_dir: PathBuf,
    },
    /// Show bootstrap secrets path hint
    BootstrapInfo {
        #[arg(long, default_value = "data")]
        data_dir: PathBuf,
    },
    /// Generate ed25519 keypair for release signing
    Keygen {
        #[arg(long, default_value = "keys")]
        out_dir: PathBuf,
    },
    /// Sign a built .so and print ModuleArtifact JSON fragment
    SignModule {
        #[arg(long)]
        name: String,
        #[arg(long)]
        version: String,
        #[arg(long)]
        so: PathBuf,
        #[arg(long)]
        secret_key: PathBuf,
        #[arg(long, default_value = "")]
        domain: String,
        /// Per-release signing key (P0-2): adds `sig2` so new runtimes verify
        /// root + release-chain signatures.
        #[arg(long, default_value = "")]
        release_key: String,
    },
    /// Sign a per-release signing key certificate with the root key
    /// (P0-2): `root_sig(version|build_id|release_pubkey)`.
    SignCert {
        #[arg(long)]
        secret_key: PathBuf,
        #[arg(long)]
        version: String,
        #[arg(long)]
        build_id: String,
        #[arg(long)]
        release_pubkey: String,
    },
    /// Stage local so into modules tree (verify sig)
    Stage {
        #[arg(long)]
        modules_dir: PathBuf,
        #[arg(long)]
        pubkey: PathBuf,
        #[arg(long)]
        artifact_json: PathBuf,
        #[arg(long)]
        so: PathBuf,
        #[arg(long, default_value = "8.0.0")]
        runtime_version: String,
        /// Node data dir (paid-module license gate, docs/08).
        #[arg(long, default_value = "data")]
        data_dir: PathBuf,
    },
    /// Activate staged module version (hot path marker)
    Activate {
        #[arg(long)]
        modules_dir: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long)]
        version: String,
        /// Node data dir (paid-module license gate, docs/08).
        #[arg(long, default_value = "data")]
        data_dir: PathBuf,
    },
    /// Fetch + verify + stage ONE module from a signed release (free-tier
    /// module OTA; same verify chain as the paid panel `ota/install`).
    /// Usage: `gr-cli module update --name <m> [--version <v>] ...`
    #[command(subcommand)]
    Module(ModuleCmd),
    /// Verify a release manifest against the pinned OTA root key.
    VerifyManifest {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        pubkey: PathBuf,
        /// Persist the verified release-key binding for later module checks.
        #[arg(long)]
        modules_dir: Option<PathBuf>,
    },
    /// Sign release manifest.json in place (writes `sig` field)
    SignManifest {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        secret_key: PathBuf,
        #[arg(long, default_value = "")]
        build_id: String,
        #[arg(long, default_value = "")]
        release_pubkey: String,
        #[arg(long, default_value = "")]
        release_cert: String,
    },
}

/// Module operations: free-tier per-module OTA (`update`), plus the
/// pre-existing top-level `stage` / `activate` for local files.
#[derive(Subcommand)]
enum ModuleCmd {
    Update {
        #[arg(long)]
        name: String,
        /// Exact module version. Omitted = highest semantic version in manifest.
        #[arg(long)]
        version: Option<String>,
        /// Release base URL (e.g. https://github.com/OWNER/install/releases/latest/download).
        /// Default: $GR_RELEASE_BASE (legacy $GR_RELEASE_BASE accepted).
        #[arg(long)]
        base: Option<String>,
        /// OTA root public key (32 raw bytes, e.g. $PREFIX/ota_ed25519.pk).
        /// Default: $GR_PUBKEY_PATH (legacy $GR_PUBKEY_PATH accepted).
        #[arg(long)]
        pubkey: Option<PathBuf>,
        #[arg(long)]
        modules_dir: PathBuf,
        /// Runtime version the module must be compatible with (semver).
        #[arg(long, default_value = "8.0.0")]
        runtime_version: String,
        /// Activate the staged version immediately (hot path).
        #[arg(long)]
        activate: bool,
        /// Node data dir (paid-module license gate, docs/08).
        #[arg(long, default_value = "data")]
        data_dir: PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Install { data_dir } => {
            let hub = AdminHub::open(&data_dir.join("admin")).map_err(|e| anyhow::anyhow!(e))?;
            let secrets = data_dir.join("admin").join("admin_bootstrap_once.txt");
            println!("ok install");
            println!("console_path=/{}/", hub.auth.console_path);
            println!("secrets_file={}", secrets.display());
            if secrets.is_file() {
                println!("--- secrets ---");
                print!("{}", std::fs::read_to_string(&secrets)?);
            }
        }
        Cmd::BootstrapInfo { data_dir } => {
            let p = data_dir.join("admin").join("admin_bootstrap_once.txt");
            println!("{}", p.display());
            if p.is_file() {
                print!("{}", std::fs::read_to_string(p)?);
            }
        }
        Cmd::Keygen { out_dir } => {
            std::fs::create_dir_all(&out_dir)?;
            let (sk, vk) = generate_signing_keypair();
            let sk_path = out_dir.join("ota_ed25519.sk");
            let pk_path = out_dir.join("ota_ed25519.pk");
            std::fs::write(&sk_path, sk.to_bytes())?;
            std::fs::write(&pk_path, vk.as_bytes())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&sk_path, std::fs::Permissions::from_mode(0o600));
            }
            println!("secret_key={}", sk_path.display());
            println!("public_key={}", pk_path.display());
            println!("public_key_hex={}", hex::encode(vk.as_bytes()));
        }
        Cmd::SignCert {
            secret_key,
            version,
            build_id,
            release_pubkey,
        } => {
            let sk_bytes = std::fs::read(&secret_key)?;
            if sk_bytes.len() != 32 {
                anyhow::bail!("secret key must be 32 bytes");
            }
            let mut sb = [0u8; 32];
            sb.copy_from_slice(&sk_bytes);
            let sk = ed25519_dalek::SigningKey::from_bytes(&sb);
            let msg = release_cert_message(&version, &build_id, &release_pubkey);
            println!("{}", sign_bytes(&sk, &msg));
        }
        Cmd::SignModule {
            name,
            version,
            so,
            secret_key,
            domain,
            release_key,
        } => {
            let sk_bytes = std::fs::read(secret_key)?;
            if sk_bytes.len() != 32 {
                anyhow::bail!("secret key must be 32 bytes");
            }
            let mut sb = [0u8; 32];
            sb.copy_from_slice(&sk_bytes);
            let sk = ed25519_dalek::SigningKey::from_bytes(&sb);
            let data = std::fs::read(&so)?;
            let hash = sha256_hex(&data);
            let asset = so
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| format!("libgr_{name}.so"));
            let mut art = ModuleArtifact {
                name: name.clone(),
                version: version.clone(),
                abi: gr_abi::RUNTIME_ABI,
                requires_major: gr_abi::PRODUCT_MAJOR,
                min_runtime_minor: 0,
                max_runtime_minor: None,
                asset,
                sha256: hash,
                sig: String::new(),
                sig2: None,
                domain: if domain.is_empty() {
                    name.clone()
                } else {
                    domain
                },
            };
            art.sig = sign_bytes(&sk, &artifact_sign_message(&art));
            if !release_key.is_empty() {
                let rk_bytes = std::fs::read(&release_key)?;
                if rk_bytes.len() != 32 {
                    anyhow::bail!("release key must be 32 bytes");
                }
                let mut rb = [0u8; 32];
                rb.copy_from_slice(&rk_bytes);
                let rk = ed25519_dalek::SigningKey::from_bytes(&rb);
                art.sig2 = Some(sign_bytes(&rk, &artifact_sign_message(&art)));
            }
            println!("{}", serde_json::to_string_pretty(&art)?);
        }
        Cmd::Stage {
            modules_dir,
            pubkey,
            artifact_json,
            so,
            runtime_version,
            data_dir: _,
        } => {
            let pk = std::fs::read(pubkey)?;
            let art: ModuleArtifact = serde_json::from_str(&std::fs::read_to_string(artifact_json)?)?;
            let mut eng = OtaEngine::new(OtaConfig {
                release_base_url: String::new(),
                modules_dir,
                pubkey_bytes: pk,
                release_key: None,
            });
            let dest = eng
                .stage_local_file(&art, &so, &runtime_version)
                .map_err(|e| anyhow::anyhow!(e))?;
            println!("staged {}", dest.display());
        }
        Cmd::Activate {
            modules_dir,
            name,
            version,
            data_dir: _,
        } => {
            let eng = OtaEngine::new(OtaConfig {
                release_base_url: String::new(),
                modules_dir,
                pubkey_bytes: vec![0u8; 32],
                release_key: None,
            });
            let p = eng
                .activate(&name, &version)
                .map_err(|e| anyhow::anyhow!(e))?;
            println!("activated {}@{} -> {}", name, version, p.display());
        }
        Cmd::Module(cmd) => match cmd {
            ModuleCmd::Update {
                name,
                version,
                base,
                pubkey,
                modules_dir,
                runtime_version,
                activate,
                data_dir: _,
            } => {
            let base = base
                .or_else(|| gr_abi::env::get("RELEASE_BASE").filter(|s| !s.is_empty()))
                .ok_or_else(|| {
                    anyhow::anyhow!("release base URL required: pass --base or set GR_RELEASE_BASE (legacy GR_RELEASE_BASE accepted)")
                })?;
            let pk_path = pubkey
                .or_else(|| {
                    gr_abi::env::get("PUBKEY_PATH")
                        .filter(|s| !s.is_empty())
                        .map(PathBuf::from)
                })
                .ok_or_else(|| {
                    anyhow::anyhow!("OTA root public key required: pass --pubkey or set GR_PUBKEY_PATH (legacy GR_PUBKEY_PATH accepted)")
                })?;
            let pk = std::fs::read(&pk_path)?;
            if pk.len() != 32 {
                anyhow::bail!(
                    "OTA root public key must be 32 bytes: {}",
                    pk_path.display()
                );
            }
            // Same flow as the service's `ota_install_remote`: verified
            // manifest chain → release-key binding → module selection →
            // download → check_compat + double-sig chain + sha256 → stage.
            let mut eng = OtaEngine::new(OtaConfig {
                release_base_url: base.clone(),
                modules_dir: modules_dir.clone(),
                pubkey_bytes: pk,
                release_key: None,
            });
            let man = eng
                .fetch_manifest_blocking()
                .map_err(|e| anyhow::anyhow!("manifest fetch/verify failed: {e}"))?;
            eng.write_release_binding(&man)
                .map_err(|e| anyhow::anyhow!("persist release binding failed: {e}"))?;
            let art = gr_ota::pick_module(&man, &name, version.as_deref())
                .map_err(|e| anyhow::anyhow!(e))?
                .clone();
            let tmp_dir = modules_dir
                .join("staging")
                .join(&art.name)
                .join(&art.version);
            std::fs::create_dir_all(&tmp_dir)?;
            let tmp = tmp_dir.join(&art.asset);
            eng.download_asset_blocking(&art, &tmp)
                .map_err(|e| anyhow::anyhow!("asset download failed: {e}"))?;
            let dest = eng
                .stage_local_file(&art, &tmp, &runtime_version)
                .map_err(|e| anyhow::anyhow!("stage failed: {e}"))?;
            let mut out = serde_json::json!({
                "ok": true,
                "name": art.name,
                "version": art.version,
                "path": dest,
                "signed": true,
                "activated": false,
                "restart_required": false,
            });
            if activate {
                let p = eng
                    .activate(&art.name, &art.version)
                    .map_err(|e| anyhow::anyhow!(e))?;
                out["activated"] = serde_json::json!(true);
                out["activate_path"] = serde_json::json!(p);
            }
            println!("{}", serde_json::to_string_pretty(&out)?);
            }
        },
        Cmd::VerifyManifest {
            manifest,
            pubkey,
            modules_dir,
        } => {
            let pk = std::fs::read(pubkey)?;
            if pk.len() != 32 {
                anyhow::bail!("OTA root public key must be 32 bytes");
            }
            let man: ReleaseManifest =
                serde_json::from_str(&std::fs::read_to_string(&manifest)?)?;
            let release = verify_manifest_chain(&pk, &man)
                .map_err(|e| anyhow::anyhow!("manifest verification failed: {e}"))?;
            if let Some(dir) = modules_dir {
                let mut eng = OtaEngine::new(OtaConfig {
                    release_base_url: String::new(),
                    modules_dir: dir,
                    pubkey_bytes: pk,
                    release_key: release.map(|k| k.to_bytes().to_vec()),
                });
                eng.write_release_binding(&man)
                    .map_err(|e| anyhow::anyhow!("persist release binding failed: {e}"))?;
            }
            println!(
                "verified_manifest product={} version={} arch={}",
                man.product,
                man.runtime.version,
                man.arch.as_deref().unwrap_or("unknown")
            );
        }
        Cmd::SignManifest {
            manifest,
            secret_key,
            build_id,
            release_pubkey,
            release_cert,
        } => {
            let sk_bytes = std::fs::read(&secret_key)?;
            if sk_bytes.len() != 32 {
                anyhow::bail!("secret key must be 32 bytes");
            }
            let mut sb = [0u8; 32];
            sb.copy_from_slice(&sk_bytes);
            let sk = ed25519_dalek::SigningKey::from_bytes(&sb);
            let raw = std::fs::read_to_string(&manifest)?;
            let mut man: ReleaseManifest = serde_json::from_str(&raw)?;
            if !build_id.is_empty() {
                man.build_id = Some(build_id.clone());
            }
            if !release_pubkey.is_empty() {
                man.release_pubkey = Some(release_pubkey.clone());
            }
            if !release_cert.is_empty() {
                man.release_cert = Some(release_cert.clone());
            }
            man.sig = None;
            let msg = manifest_sign_message(&man).map_err(|e| anyhow::anyhow!(e))?;
            man.sig = Some(sign_bytes(&sk, &msg));
            std::fs::write(&manifest, serde_json::to_string_pretty(&man)?)?;
            println!("signed_manifest={}", manifest.display());
            println!("sig={}", man.sig.as_deref().unwrap_or(""));
        }
    }
    Ok(())
}
