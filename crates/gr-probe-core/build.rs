//! Product stamp SSOT: workspace-root `VERSION`. Keep aligned with gr-probe-plane.
//! 布局可移植: 从 crate manifest 向上找「同时含 Cargo.toml + VERSION」的工作区根
//! (greenpng 编号布局 02-probe-analysis/... 与扁平发行仓 gr-server 两用)。
//! 旧实现固定 manifest/../../.. — 仅编号布局命中; 扁平发行仓会落到仓库上一级
//! → CI 发版二进制全部错戳 "dev" (178 v1.0.0 实测 product_version=dev)。
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    // 向上找工作区根 (含 Cargo.toml + VERSION), 最多 6 层
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
    let mut stamp_path: Option<PathBuf> = None;
    if let Some(r) = root.as_ref() {
        let p = r.join("VERSION");
        println!("cargo:rerun-if-changed={}", p.display());
        if let Ok(s) = fs::read_to_string(&p) {
            let t = s.trim().to_string();
            if !t.is_empty() {
                ver = t;
                stamp_path = Some(p);
            }
        }
    }
    if stamp_path.is_none() {
        // 兜底: fe/VERSION (保持旧行为; 正常不应走到)
        let p = manifest.join("../fe/VERSION");
        println!("cargo:rerun-if-changed={}", p.display());
        if let Ok(s) = fs::read_to_string(&p) {
            let t = s.trim().to_string();
            if !t.is_empty() {
                ver = t;
            }
        }
    }
    println!("cargo:rustc-env=GR_PRODUCT_VERSION={ver}");
    // P0-1: per-release build id (release script sets GR_BUILD_ID; dev = "dev").
    let bid = env::var("GR_BUILD_ID")
        .or_else(|_| env::var("GR_BUILD_ID"))
        .unwrap_or_else(|_| "dev".to_string());
    println!("cargo:rustc-env=GR_BUILD_ID={bid}");
    println!("cargo:rerun-if-env-changed=GR_BUILD_ID");
    println!("cargo:rerun-if-env-changed=GR_BUILD_ID");
    // P0-4: obfuscation salt → 64-char key (see crates/gr-obf).
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
