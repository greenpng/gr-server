//! Ops tool: verify a greenpng release (whole-bundle or extracted tree)
//! against the root public key: manifest chain + module chain sigs + sha256
//! of every shipped file.
//!
//! Whole-bundle mode (greenpng 1.0.0+ asset model):
//!   GR_RT_BUNDLE=dist/release-1.0.0/greenpng-1.0.0-x86_64.tar.gz \
//!   GR_RT_PUBKEY=keys/ota_ed25519.pk \
//!   cargo test -p gr-ota --test release_tree_verify -- --nocapture
//!   → extracts the bundle to a temp dir and verifies manifest chain,
//!     runtime/cli sha, module chain sigs + sha, fe_tree + admin_tree files.
//!
//! Extracted/flat mode (fixtures, local staging):
//!   GR_RT_MANIFEST=<...>/manifest.json GR_RT_PUBKEY=<...>/ota_ed25519.pk \
//!   GR_RT_DIR=<tree root where manifest asset paths resolve>
//!
//! Skipped when the env vars are absent, so normal CI stays fast.

use gr_abi::ReleaseManifest;
use gr_ota::{bundle_tree_root, safe_extract_tar_gz, verify_artifact_sig_chain, verify_manifest_chain};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn env_path(gr: &str) -> Option<PathBuf> {
    let p = std::env::var_os(gr).map(PathBuf::from);
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

fn sha(p: &Path) -> String {
    gr_abi::sha256_hex(&std::fs::read(p).unwrap_or_else(|e| panic!("read {}: {e}", p.display())))
}

fn verify_tree(man: &ReleaseManifest, root: &Path, root_pk: &[u8]) {
    let released =
        verify_manifest_chain(root_pk, man).expect("manifest chain verifies");
    // Installer contract: the root pubkey must ship inside the bundle.
    let pk_asset = std::fs::read(root.join("ota_ed25519.pk")).expect("ota_ed25519.pk in tree");
    assert_eq!(pk_asset, root_pk, "ota_ed25519.pk must equal the root public key");
    eprintln!(
        "[release_tree_verify] build_id={:?} release_key_bound={}",
        man.build_id,
        released.is_some()
    );
    assert!(
        man.build_id.as_deref().map(|b| !b.is_empty()).unwrap_or(false),
        "new-format manifest must carry a build_id"
    );

    // runtime + cli sha (paths like bin/gr-service, bin/gr-cli)
    for (label, entry) in [("runtime", &man.runtime)] {
        let asset = entry.asset.as_deref().unwrap_or_else(|| panic!("{label} asset missing"));
        let want = entry.sha256.as_deref().unwrap_or_else(|| panic!("{label} sha256 missing"));
        let p = root.join(asset);
        assert!(p.is_file(), "{label} asset missing in tree: {asset}");
        assert_eq!(sha(&p), want, "{label} sha256 mismatch ({asset})");
        eprintln!("[release_tree_verify] {label} ok: {asset}");
    }
    if let Some(cli) = &man.cli {
        let p = root.join(cli.asset.as_deref().expect("cli asset"));
        assert!(p.is_file(), "cli asset missing: {}", cli.asset.clone().unwrap_or_default());
        assert_eq!(sha(&p), cli.sha256.clone().expect("cli sha256"), "cli sha256 mismatch");
        eprintln!("[release_tree_verify] cli ok: {}", cli.asset.clone().unwrap_or_default());
    }

    // modules: chain sig + sha (manifest `asset` is the SIGNED flat name;
    // inside a bundle tree the .so lives under modules/<asset>)
    let mut checked = 0usize;
    for art in &man.modules {
        verify_artifact_sig_chain(root_pk, released.as_ref(), art)
            .unwrap_or_else(|e| panic!("module {}@{:?} chain failed: {e}", art.name, art.version));
        let p = root.join("modules").join(&art.asset);
        let p = if p.is_file() { p } else { root.join(&art.asset) };
        let data = std::fs::read(&p)
            .unwrap_or_else(|e| panic!("asset {} unreadable: {e}", p.display()));
        assert_eq!(
            gr_abi::sha256_hex(&data),
            art.sha256,
            "module {} sha256 mismatch (files under {} must be the staged copies)",
            art.name,
            root.display()
        );
        checked += 1;
        eprintln!("[release_tree_verify] module ok: {}@{} ({})", art.name, art.version, art.asset);
    }

    // fe_tree + admin_tree + spec_tree: per-file sha of the expanded dirs
    // (typed fields — part of the signed canonical body on whole-bundle
    // releases; spec_tree joins in 1.0.2+, older bundles legitimately lack it)
    for (label, tree, sub) in [
        ("fe_tree", &man.fe_tree, "fe"),
        ("admin_tree", &man.admin_tree, "admin"),
        ("spec_tree", &man.spec_tree, "spec"),
    ] {
        let Some(tree) = tree else {
            if label == "fe_tree" {
                // flat fixtures may omit fe_tree; bundle manifests must have it
                eprintln!("[release_tree_verify] note: manifest has no fe_tree (flat fixture?)");
            } else {
                eprintln!("[release_tree_verify] note: manifest has no {label} (pre-1.0.2 bundle?)");
            }
            continue;
        };
        assert!(!tree.files.is_empty(), "{label}.files must not be empty");
        let dir = root.join(sub);
        for (rel, want) in &tree.files {
            let p = dir.join(rel);
            assert!(p.is_file(), "{label} file missing: {sub}/{rel}");
            assert_eq!(sha(&p), want.as_str(), "{label} sha mismatch: {sub}/{rel}");
        }
        eprintln!("[release_tree_verify] {label} ok: {} files under {sub}/", tree.files.len());
    }

    // shared fe tgz (sibling release asset): inside bundles it is not present
    // (fe_tree covers the expanded copy); flat fixtures ship it in the tree.
    if let Some(fe) = &man.fe {
        let p = root.join(&fe.asset);
        if p.is_file() {
            assert_eq!(sha(&p), fe.sha256, "fe tgz sha256 mismatch");
            eprintln!("[release_tree_verify] fe tgz ok: {}", fe.asset);
        } else if man.fe_tree.is_some() {
            eprintln!("[release_tree_verify] fe tgz not in tree (shared sibling asset; fe_tree verified) — skipped");
        } else {
            panic!("fe tgz missing in tree: {}", fe.asset);
        }
    }
    eprintln!("[release_tree_verify] OK: manifest chain + {checked} module(s) verified");
}

#[test]
fn verify_release_tree_if_env_provided() {
    let Some(pubkey_path) = env_path("GR_RT_PUBKEY") else {
        eprintln!("[release_tree_verify] skipped (set GR_RT_BUNDLE or GR_RT_MANIFEST/PUBKEY/DIR)");
        return;
    };
    let root_pk: Vec<u8> = std::fs::read(&pubkey_path).unwrap();
    assert_eq!(root_pk.len(), 32, "root pubkey must be 32 bytes");

    // ---- whole-bundle mode ----
    if let Some(bundle) = env_path("GR_RT_BUNDLE") {
        let tmp = std::env::temp_dir().join(format!(
            "gr-rt-verify-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&tmp).expect("temp dir");
        safe_extract_tar_gz(&bundle, &tmp)
            .unwrap_or_else(|e| panic!("bundle extract failed ({}): {e}", bundle.display()));
        let root = bundle_tree_root(&tmp);
        let manifest_path = root.join("manifest.json");
        let man: ReleaseManifest = serde_json::from_str(
            &std::fs::read_to_string(&manifest_path)
                .unwrap_or_else(|e| panic!("bundle manifest unreadable: {e}")),
        )
        .expect("manifest parses");
        verify_tree(&man, &root, &root_pk);
        let _ = std::fs::remove_dir_all(&tmp);
        return;
    }

    // ---- extracted / flat mode ----
    let (Some(manifest_path), Some(dir)) = (env_path("GR_RT_MANIFEST"), env_path("GR_RT_DIR"))
    else {
        eprintln!("[release_tree_verify] skipped (set GR_RT_BUNDLE or GR_RT_MANIFEST/PUBKEY/DIR)");
        return;
    };
    let man: ReleaseManifest =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap())
            .expect("manifest parses");
    verify_tree(&man, &dir, &root_pk);
}
