//! Full HTTP/3 application-layer fields must move stack/product/brain.

use gr_probe_core::bot::score_bot;
use gr_probe_core::brain::build_frontier;
use gr_probe_core::product_scores::{score_br, score_os};
use gr_probe_core::stack_auth::stack_auth_from_fields;
use gr_probe_core::xsrc::evaluate_xsrc;
use serde_json::json;

#[test]
fn h3_app_hits_stack_and_product() {
    let fields = json!({
        "user_agent": "Mozilla/5.0 Chrome/150",
        "platform": "Linux x86_64",
        "form_class": "desktop",
        "webdriver": false,
        "h3_app_present": true,
        "h3_alpn": "h3",
        "h3_settings_fp": "abc123def456",
        "h3_pseudo_order": ":method,:path,:authority,:scheme",
        "h3_rtt_ms": 12.5,
        "quic_listen_present": true,
        "quic_tls_ja4": "t13i0310h3_test",
    });
    let stack = stack_auth_from_fields(&fields);
    let rs = stack.reasons.join(",");
    assert!(rs.contains("h3_app_layer_present"), "{rs}");
    assert!(rs.contains("h3_pseudo_order_present"), "{rs}");

    let bot = score_bot(&fields, "balanced").unwrap();
    let truth = evaluate_xsrc(
        &json!({
            "fields": fields,
            "sources": ["main","gateway"],
            "has_gateway": true,
            "batches": [
                {"batch_id":"B0_bootstrap","source":"main"},
                {"batch_id":"B8_gateway","source":"gateway"}
            ]
        }),
        None,
    )
    .unwrap();
    let br = score_br(&fields, &stack, &bot, &truth, &[]);
    let os = score_os(&fields, &stack, &truth, &[]);
    let hits: Vec<String> = br["field_hits"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(os["field_hits"].as_array().into_iter().flatten())
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    assert!(
        hits.iter().any(|h| h == "h3_app_layer")
            || br["reasons"]
                .as_array()
                .map(|a| a.iter().any(|r| r.as_str().unwrap_or("").contains("h3_app")))
                .unwrap_or(false),
        "br={br} os={os}"
    );
}

#[test]
fn need_h3_app_gap_when_quic_without_app() {
    let evidence = json!({
        "session_id": "s_h3",
        "fields": {
            "user_agent": "Mozilla/5.0 Chrome/150",
            "platform": "Linux x86_64",
            "form_class": "desktop",
            "webdriver": false,
            "ja4": "t13i_tcp",
            "quic_tls_ja4": "t13i_quic",
            "quic_listen_present": true,
        },
        "batches": [{"batch_id":"B0_bootstrap","source":"main"}],
        "sources": ["main"],
    });
    let frontier = build_frontier(&evidence, true, None, 8).expect("frontier");
    let has = frontier.gaps.iter().any(|g| g.code == "need_h3_app");
    assert!(
        has,
        "expected need_h3_app gap, got {:?}",
        frontier.gaps.iter().map(|g| &g.code).collect::<Vec<_>>()
    );
}
