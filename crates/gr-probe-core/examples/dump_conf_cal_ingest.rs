//! iss/38 D-19: ingest labeled pairs JSON → offline conf_cal report.
//!
//!   cargo run -p gr-core --example dump_conf_cal_ingest -- /path/to/pairs.json
//! Default: spec/conf_cal_labeled_pairs_v1.json

use gr_probe_core::{calibrate_offline, pairs_from_json};
use serde_json::Value;
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let path = env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.pop();
        p.pop();
        p.join("spec/conf_cal_labeled_pairs_v1.json")
    });
    let doc: Value = serde_json::from_str(&fs::read_to_string(&path).expect("read pairs"))
        .expect("json");
    let pairs_v = doc
        .get("pairs")
        .cloned()
        .unwrap_or(doc.clone());
    let pairs = pairs_from_json(&pairs_v);
    let report = calibrate_offline(&pairs);
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "ok": true,
            "source": path.display().to_string(),
            "n_pairs": pairs.len(),
            "report": report,
        }))
        .unwrap()
    );
}
