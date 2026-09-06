//! Ops tool: verify a whole release tree (manifest chain + module chain sigs +
//! sha256 of shipped artifacts) against the root public key.
//!
//! Usage (run after `release/build_and_publish.sh` or CI merge):
//!   GR_RT_MANIFEST=dist/release-6.0.28/manifest.json \
//!   GR_RT_PUBKEY=keys/ota_ed25519.pk \
//!   GR_RT_DIR=dist/release-6.0.28 \
//!   cargo test -p gr-ota --test release_tree_verify -- --nocapture
//!
//! Skipped when the env vars are absent, so normal CI stays fast.

use gr_abi::ReleaseManifest;
use gr_ota::{verify_artifact_sig_chain, verify_manifest_chain};
use std::path::{Path, PathBuf};

fn env_path(gr: &str) -> Option<PathBuf> {
    // GR_* first (8.0), GR_* fallback (pre-8.0 CI envs).
    let legacy = gr.replace("GR_RT_", "GR_RT_");
    for name in [gr, legacy.as_str()] {
        if let Some(p) = env_path_one(name) {
            return Some(p);
        }
    }
    None
}

fn env_path_one(name: &str) -> Option<PathBuf> {
    let p = std::env::var_os(name).map(PathBuf::from);
    // cargo runs tests with CWD = crate dir; resolve relative paths against the
    // repo root so `dist/...`/`keys/...` work unchanged from docs.
    p.map(|p| {
        if p.is_absolute() {
            p
        } else {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../..")
                .join(p)
        }
    })
}

#[test]
fn verify_release_tree_if_env_provided() {
    let (Some(manifest_path), Some(pubkey_path), Some(dir)) = (
        env_path("GR_RT_MANIFEST"),
        env_path("GR_RT_PUBKEY"),
        env_path("GR_RT_DIR"),
    ) else {
        eprintln!("[release_tree_verify] skipped (set GR_RT_MANIFEST/PUBKEY/DIR)");
        return;
    };
    let root: Vec<u8> = std::fs::read(&pubkey_path).unwrap();
    assert_eq!(root.len(), 32, "root pubkey must be 32 bytes");
    let man: ReleaseManifest =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap())
            .expect("manifest parses");

    let released = verify_manifest_chain(&root, &man).expect("manifest chain verifies");
    // Installer contract: the root pubkey must ship as an asset (install.sh
    // dies without ota_ed25519.pk in the release).
    let pk_asset = std::fs::read(Path::new(&dir).join("ota_ed25519.pk")).expect("ota_ed25519.pk asset");
    assert_eq!(pk_asset, root, "ota_ed25519.pk asset must equal the root public key");
    eprintln!(
        "[release_tree_verify] build_id={:?} release_key_bound={}",
        man.build_id,
        released.is_some()
    );
    assert!(
        man.build_id.as_deref().map(|b| !b.is_empty()).unwrap_or(false),
        "new-format manifest must carry a build_id"
    );

    let mut checked = 0usize;
    for art in &man.modules {
        verify_artifact_sig_chain(&root, released.as_ref(), art)
            .unwrap_or_else(|e| panic!("module {}@{:?} chain failed: {e}", art.name, art.version));
        let asset = Path::new(&dir).join(&art.asset);
        let data = std::fs::read(&asset)
            .unwrap_or_else(|e| panic!("asset {} unreadable: {e}", asset.display()));
        let got = gr_abi::sha256_hex(&data);
        assert_eq!(
            got, art.sha256,
            "module {} sha256 mismatch (files under {} must be the staged copies)",
            art.name, dir.display()
        );
        checked += 1;
        eprintln!("[release_tree_verify] module ok: {}@{} ({})", art.name, art.version, art.asset);
    }
    if let Some(fe) = &man.fe {
        let data = std::fs::read(Path::new(&dir).join(&fe.asset)).expect("fe tgz readable");
        assert_eq!(gr_abi::sha256_hex(&data), fe.sha256, "fe sha256 mismatch");
        eprintln!("[release_tree_verify] fe ok: {}", fe.asset);
    }
    if let Some(runtime) = man.runtime.asset.as_deref() {
        if let Some(sha) = &man.runtime.sha256 {
            let data = std::fs::read(Path::new(&dir).join(runtime)).expect("runtime asset readable");
            assert_eq!(gr_abi::sha256_hex(&data), *sha, "runtime sha256 mismatch");
            eprintln!("[release_tree_verify] runtime ok: {runtime}");
        }
    }
    eprintln!("[release_tree_verify] OK: manifest chain + {checked} module(s) verified");
}
