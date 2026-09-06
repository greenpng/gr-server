//! P0-4: inject the per-release obfuscation key (see crates/gr-obf). The
//! release script exports GR_OBF_SALT; the same derivation here as in
//! gr-probe-core/build.rs and gr-obf/build.rs keeps every crate on one key.
use std::env;

fn main() {
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
