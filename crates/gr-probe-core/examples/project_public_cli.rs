//! CLI: project full evaluate JSON → product_public (for PG replay / lab parity).
//!
//! ```bash
//! echo '{"session_id":"x","bot":{"verdict":"human"},"product":{...}}' \
//!   | cargo run -p gr-probe-core --example project_public_cli -- balanced standard
//! ```

use serde_json::Value;
use std::env;
use std::io::{self, Read};

fn main() {
    let mut args = env::args().skip(1);
    let strategy = args.next().unwrap_or_else(|| "balanced".into());
    let profile = args.next().unwrap_or_else(|| "standard".into());
    let mut buf = String::new();
    io::stdin().read_to_string(&mut buf).expect("read stdin");
    let full: Value = serde_json::from_str(&buf).expect("json");
    let sid = full
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let out = gr_probe_core::project_public_with(
        &full,
        &strategy,
        &profile,
        sid.as_deref(),
    );
    println!("{}", serde_json::to_string(&out).expect("serialize"));
}
