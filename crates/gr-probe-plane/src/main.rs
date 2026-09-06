//! gr-probe-plane standalone binary (optional; production uses gr-service).
use clap::Parser;
use gr_probe_plane::run::{mirror_env_aliases, run_with, Args};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    mirror_env_aliases();
    run_with(Args::parse());
}
