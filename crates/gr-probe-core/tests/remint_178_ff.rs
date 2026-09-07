//! Offline remint of 178 Firefox multi-site field dumps.
use serde_json::Value;
use std::fs;

fn slots(device_id: &str) -> Vec<String> {
    device_id
        .strip_prefix("dv0-")
        .unwrap_or(device_id)
        .split('-')
        .map(|s| s.to_string())
        .collect()
}

#[test]
fn remint_178_firefox_four_sites() {
    let names = ["chinaallied", "searchchina", "sozhan", "zhanso"];
    // Offline remint utility: requires operator-provided 178 field dumps in
    // /tmp. Skip (pass) when the dumps are not present on this machine.
    if !names.iter().all(|n| std::path::Path::new(&format!("/tmp/fields_{n}.json")).exists()) {
        eprintln!("skip: /tmp/fields_*.json dumps not present (offline remint utility)");
        return;
    }
    let mut rows = Vec::new();
    for name in names {
        let path = format!("/tmp/fields_{name}.json");
        let raw = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        let v: Value = serde_json::from_str(&raw).expect("json");
        let fields = v.get("fields").cloned().unwrap_or(v);
        let out = gr_probe_core::device_segments::select_device_segments_local(&fields, None);
        let did = out
            .get("device_id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let notes = out
            .get("curve_notes")
            .cloned()
            .unwrap_or(Value::Array(vec![]));
        let s = slots(&did);
        println!("=== {name} ===");
        println!("device_id={did}");
        if s.len() >= 10 {
            println!(
                "res={} wg={} au={} cp={} of={} ar={} cc={} tz={} oi={} rtc={}",
                s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7], s[8], s[9]
            );
        }
        println!("notes={}", notes);
        rows.push((name.to_string(), s, did));
    }
    // Compare commercial silicon head
    let aus: Vec<_> = rows.iter().map(|r| r.1.get(2).cloned().unwrap_or_default()).collect();
    let ars: Vec<_> = rows.iter().map(|r| r.1.get(5).cloned().unwrap_or_default()).collect();
    let ccs: Vec<_> = rows.iter().map(|r| r.1.get(6).cloned().unwrap_or_default()).collect();
    let ress: Vec<_> = rows.iter().map(|r| r.1.get(0).cloned().unwrap_or_default()).collect();
    let wgs: Vec<_> = rows.iter().map(|r| r.1.get(1).cloned().unwrap_or_default()).collect();
    println!("res set={:?}", ress.iter().collect::<std::collections::BTreeSet<_>>());
    println!("wg set={:?}", wgs.iter().collect::<std::collections::BTreeSet<_>>());
    println!("au set={:?}", aus.iter().collect::<std::collections::BTreeSet<_>>());
    println!("ar set={:?}", ars.iter().collect::<std::collections::BTreeSet<_>>());
    println!("cc set={:?}", ccs.iter().collect::<std::collections::BTreeSet<_>>());

    // Expected after fix: same seed_delta → same au; thin B18 → same ar; same tex/rb → same cc
    // (searchchina dump may lack gl_max → cc=0; sites with tex+rb must share one non-zero)
    assert_eq!(
        aus.iter().collect::<std::collections::BTreeSet<_>>().len(),
        1,
        "au must be stable across B46 presence: {aus:?}"
    );
    assert_eq!(
        ars.iter().collect::<std::collections::BTreeSet<_>>().len(),
        1,
        "ar must be stable across thin B18: {ars:?}"
    );
    let ccs_nonzero: Vec<_> = ccs.iter().filter(|c| c.as_str() != "0").cloned().collect();
    if !ccs_nonzero.is_empty() {
        assert_eq!(
            ccs_nonzero
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            1,
            "cc with complete tex+rb must be stable: {ccs:?}"
        );
    }
}
