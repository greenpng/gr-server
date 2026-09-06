//! Product stamp SSOT: root `VERSION` (semver 8.x.y). Keep aligned with gr-probe-core.
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.join("../../..");
    let candidates = [
        root.join("VERSION"),
        root.join("fe/VERSION"),
        manifest.join("../../../VERSION"),
    ];
    let mut ver = "dev".to_string();
    for p in &candidates {
        println!("cargo:rerun-if-changed={}", p.display());
        if let Ok(s) = fs::read_to_string(p) {
            let t = s.trim().to_string();
            if !t.is_empty() {
                ver = t;
                break;
            }
        }
    }
    println!("cargo:rustc-env=GR_PRODUCT_VERSION={ver}");
    // P0-1 / P0-4: build id + obfuscation salt (see crates/gr-obf).
    let bid = env::var("GR_BUILD_ID")
        .or_else(|_| env::var("GR_BUILD_ID"))
        .unwrap_or_else(|_| "dev".to_string());
    println!("cargo:rustc-env=GR_BUILD_ID={bid}");
    println!("cargo:rerun-if-env-changed=GR_BUILD_ID");
    println!("cargo:rerun-if-env-changed=GR_BUILD_ID");
    let salt = env::var("GR_OBF_SALT")
        .or_else(|_| env::var("GR_OBF_SALT"))
        .unwrap_or_else(|_| "dev".to_string());
    let mut key = String::new();
    while key.len() < 64 {
        key.push_str(&salt);
    }
    key.truncate(64);
    println!("cargo:rustc-env=GR_OBF_KEY={key}");
    println!("cargo:rerun-if-env-changed=GR_OBF_SALT");
    println!("cargo:rerun-if-env-changed=GR_OBF_SALT");
}
