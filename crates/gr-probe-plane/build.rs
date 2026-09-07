//! Product stamp SSOT: workspace-root `VERSION`. Keep aligned with gr-probe-core.
//! 布局可移植: 向上找「Cargo.toml + VERSION」工作区根 (编号布局/扁平发行仓两用);
//! 旧固定 manifest/../../.. 在扁平发行仓落到仓库上一级 → 发版二进制错戳 "dev"。
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let mut root: Option<PathBuf> = None;
    {
        let mut dir = manifest.clone();
        for _ in 0..6 {
            if dir.join("Cargo.toml").is_file() && dir.join("VERSION").is_file() {
                root = Some(dir.clone());
                break;
            }
            if !dir.pop() {
                break;
            }
        }
    }
    let mut ver = "dev".to_string();
    if let Some(r) = root.as_ref() {
        let p = r.join("VERSION");
        println!("cargo:rerun-if-changed={}", p.display());
        if let Ok(s) = fs::read_to_string(&p) {
            let t = s.trim().to_string();
            if !t.is_empty() {
                ver = t;
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
