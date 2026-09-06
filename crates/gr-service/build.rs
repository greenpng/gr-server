//! P0-4: per-release obfuscation salt injection for gr-service (same
//! derivation as crates/gr-obf so `obf![]` cover strings in the service).
use std::env;

fn main() {
    // GR naming migration (docs/13): GR_OBF_SALT preferred; GR_OBF_SALT fallback.
    let salt = env::var("GR_OBF_SALT")
        .or_else(|_| env::var("GR_OBF_SALT"))
        .unwrap_or_else(|_| "dev".to_string());
    let mut key = String::new();
    while key.len() < 64 {
        key.push_str(&salt);
    }
    key.truncate(64);
    println!("cargo:rustc-env=GR_OBF_KEY={key}");
    println!("cargo:rustc-env=GR_OBF_KEY={key}");
    println!("cargo:rerun-if-env-changed=GR_OBF_SALT");
    println!("cargo:rerun-if-env-changed=GR_OBF_SALT");
}
