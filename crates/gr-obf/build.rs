//! Salt → key injection for gr-obf.
//!
//! Reads `GR_OBF_SALT` (legacy `GR_OBF_SALT`) and injects `GR_OBF_KEY`
//! (64-char hex). Local/dev builds
//! use the stable default so incremental builds and tests stay deterministic.
//!
//! NOTE: `cargo:rustc-env` only applies to the crate owning this build.rs.
//! Every crate that uses the `obf![]` macro needs an identical block (see
//! gr-probe-core, gr-service, ...).
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
