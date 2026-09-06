use std::env;
fn main() {
    // GR naming migration (docs/13): GR_* preferred; GR_* fallbacks keep
    // lab/CI scripts that still set legacy names working during rollout.
    let ver = env::var("GR_RELEASE_VERSION")
        .or_else(|_| env::var("GR_RELEASE_VERSION"))
        .or_else(|_| env::var("GR_MODULE_VERSION"))
        .or_else(|_| env::var("GR_MODULE_VERSION"))
        .unwrap_or_else(|_| env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into()));
    let ver = ver.trim().to_string();
    println!("cargo:warning=stamping module version {ver}");
    println!("cargo:rustc-env=GR_MODULE_VERSION={ver}");
    println!("cargo:rustc-env=GR_MODULE_VERSION={ver}");
    println!("cargo:rerun-if-env-changed=GR_RELEASE_VERSION");
    println!("cargo:rerun-if-env-changed=GR_MODULE_VERSION");
    println!("cargo:rerun-if-env-changed=GR_RELEASE_VERSION");
    println!("cargo:rerun-if-env-changed=GR_MODULE_VERSION");
}
