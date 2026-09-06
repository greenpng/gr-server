//! P0-1 / P0-4: per-release build id + obfuscation salt injection (status_json
//! exposes `build_id`; future obf![] uses share the same key derivation).
use std::env;

fn main() {
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
