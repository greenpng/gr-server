//! Matrix ↔ scorer consumption contract (iss/25 rounds 3–4).
//!
//! Root-cause class: matrix growth can raise `axis_coverage` / density while scorers
//! never read the field → **blackhole evidence** (looks complete, decisions unchanged).
//!
//! General rule: every T0–T2 `material|conf|veto` row in `field_product_matrix.json`
//! must appear as a string literal in `gr-core/src`, unless listed under
//! `scorer_consumption.intentional_unconsumed` with a forgeability/physics reason.
//!
//! This is a **structural** check (shipped matrix + shipped source), not a fixture score fit.

use gr_probe_core::product_matrix::load_field_product_matrix;
use std::fs;
use std::path::PathBuf;

fn core_src_blob() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut parts = Vec::new();
    fn walk(dir: &PathBuf, out: &mut Vec<String>) {
        let rd = match fs::read_dir(dir) {
            Ok(r) => r,
            Err(_) => return,
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
                if let Ok(t) = fs::read_to_string(&p) {
                    out.push(t);
                }
            }
        }
    }
    walk(&root, &mut parts);
    parts.join("\n")
}

fn field_referenced(blob: &str, field: &str) -> bool {
    let q = format!("\"{field}\"");
    if blob.contains(&q) {
        return true;
    }
    // Nested automation.webdriver style: both segments as string literals
    if let Some((parent, child)) = field.split_once('.') {
        let pq = format!("\"{parent}\"");
        let cq = format!("\"{child}\"");
        if blob.contains(&pq) && blob.contains(&cq) {
            return true;
        }
    }
    false
}

#[test]
fn t0_t1_matrix_fields_are_referenced_in_scorers() {
    let m = load_field_product_matrix().expect("load field_product_matrix");
    let blob = core_src_blob();
    assert!(
        !blob.is_empty(),
        "gr-core/src must be readable for consumption scan"
    );

    let required = m.required_scorer_fields();
    assert!(
        !required.is_empty(),
        "required_scorer_fields must be non-empty (matrix SSOT loaded)"
    );

    let mut missing = Vec::new();
    for (field, tier, roles) in &required {
        if !field_referenced(&blob, field) {
            missing.push(format!("{field} ({tier}; {roles})"));
        }
    }

    assert!(
        missing.is_empty(),
        "T0–T2 matrix material/conf/veto fields must be consumed by scorers (or listed in \
         scorer_consumption.intentional_unconsumed with a physics/forgeability reason).\n\
         Unconsumed blackholes ({}):\n  - {}",
        missing.len(),
        missing.join("\n  - ")
    );
}

#[test]
fn intentional_unconsumed_requires_nonempty_reason() {
    let m = load_field_product_matrix().expect("matrix");
    for (field, reason) in &m.scorer_consumption.intentional_unconsumed {
        assert!(
            !reason.trim().is_empty(),
            "intentional_unconsumed[{field}] must have a non-empty reason"
        );
        assert!(
            !reason.to_ascii_lowercase().contains("fixture"),
            "intentional_unconsumed reason must not be fixture-fit: {field}: {reason}"
        );
        // Allowlisted fields should still exist in matrix
        assert!(
            m.by_field.contains_key(field),
            "intentional_unconsumed field {field} not in matrix"
        );
    }
}

#[test]
fn scorer_consumption_ssot_present() {
    let m = load_field_product_matrix().expect("matrix");
    assert!(
        !m.scorer_consumption.required_tiers.is_empty(),
        "required_tiers"
    );
    assert!(
        m.scorer_consumption
            .required_roles
            .iter()
            .any(|r| r == "material"),
        "material role required"
    );
    // Policy version should be non-default once JSON section is present
    assert!(
        m.scorer_consumption.version == "1" || m.scorer_consumption.version == "default",
        "unexpected scorer_consumption.version={}",
        m.scorer_consumption.version
    );
}

/// Sanity: commercial never_digest labels still never appear in digest_order.
#[test]
fn commercial_digest_order_excludes_never_digest() {
    let m = load_field_product_matrix().expect("matrix");
    let never: std::collections::HashSet<&str> = m
        .commercial_device_id
        .never_digest
        .iter()
        .map(|s| s.as_str())
        .collect();
    for k in &m.commercial_device_id.digest_order {
        assert!(
            !never.contains(k.as_str()),
            "digest_order must not include never_digest key {k}"
        );
    }
}
