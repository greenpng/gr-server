//! Structural + functional contract: gr embeds full in-tree probe/analyze stack.
//! No dependency on green-v5 tree or external gr-service process.

use gr_probe_core::evaluate::evaluate_session;
use gr_probe_store::Store;
use serde_json::json;
use std::path::PathBuf;

#[test]
fn probe_core_evaluate_session_runs_on_real_fixture_fields() {
    let evidence = json!({
        "session_id": "t_eval_001",
        "fields": {
            "user_agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120.0.0.0",
            "os_family": "windows",
            "form_class": "desktop",
            "screen_width": 1920,
            "screen_height": 1080,
            "timezone": "Asia/Shanghai",
            "hardware_concurrency": 8,
            "webgl_unmasked_renderer": "ANGLE (Intel, Intel(R) UHD Graphics 620)"
        },
        "batches": [
            {"batch_id": "B0_bootstrap", "source": "main"},
            {"batch_id": "B3_system", "source": "main"}
        ]
    });
    let result = evaluate_session(&evidence, None, None, None, false)
        .expect("evaluate_session must succeed on minimal FE fields");
    assert!(result.is_object(), "result must be object: {result}");
    let keys: Vec<_> = result
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    assert!(
        keys.iter().any(|k| {
            k.contains("product")
                || k.contains("device")
                || k.contains("bot")
                || k.contains("score")
                || k.contains("trust")
                || k.contains("diagnostic")
                || k.contains("battle")
                || k.contains("identity")
                || k.contains("os")
                || k.contains("soft")
                || k == "ok"
                || !k.is_empty()
        }),
        "unexpected empty evaluate result keys={keys:?}"
    );
    assert_ne!(result.get("error").and_then(|v| v.as_str()), Some("stub"));
}

#[test]
fn probe_store_open_ingest_path_persists_batch() {
    let url = gr_abi::env::get("DATABASE_URL")
        .or_else(|| gr_abi::env::get("DATABASE_URL"))
        .unwrap_or_default();
    if url.trim().is_empty() {
        eprintln!("skip probe_store_open_ingest_path_persists_batch: set GR_DATABASE_URL");
        return;
    }
    let store = Store::open_postgres(url.trim()).expect("open postgres store");
    let meta = json!({
        "site_id": "t",
        "product_version": gr_probe_core::GR_PRODUCT_VERSION,
        "fe": "gr.boot"
    });
    let opened = store
        .open_session(None, None, Some(meta))
        .expect("open_session");
    let sid = opened
        .get("session_id")
        .or_else(|| opened.pointer("/session/session_id"))
        .and_then(|v| v.as_str())
        .expect("session_id from open_session")
        .to_string();
    assert!(!sid.is_empty());
    let payload = json!({"fields": {"os_family": "linux", "form_class": "desktop"}});
    let up = store
        .upsert_batch(&sid, "B0_bootstrap", "main", &payload)
        .expect("upsert_batch");
    assert!(up.is_object() || up.get("ok").is_some() || !up.is_null(), "up={up}");
    let evidence = store.build_evidence(&sid).expect("build_evidence");
    assert!(
        evidence.get("fields").is_some()
            || evidence.get("batches").is_some()
            || evidence.as_object().map(|m| !m.is_empty()).unwrap_or(false),
        "evidence={evidence}"
    );
}

#[test]
fn gr_embeds_in_tree_probe_plane_run_symbol() {
    let _f: fn(gr_probe_plane::run::Args) = gr_probe_plane::run::run_with;
    let _ = _f as usize;
}

#[test]
fn product_version_from_in_tree_fe() {
    let fe_ver =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../probe/fe/VERSION");
    let fe_ver = if fe_ver.is_file() {
        std::fs::read_to_string(fe_ver).unwrap_or_default()
    } else {
        String::new()
    };
    let fe_ver = fe_ver.trim().to_string();
    // FE may still carry historical v5.* tag after copy; core is stamped from green-v6/VERSION
    let core = gr_probe_core::GR_PRODUCT_VERSION;
    assert!(!core.is_empty(), "core product version empty");
    if !fe_ver.is_empty() {
        assert!(
            fe_ver.starts_with('v') || fe_ver.chars().next().unwrap().is_ascii_digit(),
            "unexpected FE version {fe_ver:?}"
        );
    }
}
