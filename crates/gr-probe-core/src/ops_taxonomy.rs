//! Ops event taxonomy — commercial dimensions for error/warn triage (iss/63 ops, v5-docs2/08-ops).
//!
//! **Why**: FE sticky versions + browser `Failed to fetch` flood used to look like API 5xx.
//! Server normalizes dimensions on ingest so dashboards can slice by:
//!
//! | Dimension | Role |
//! |-----------|------|
//! | `fail_class` | What failed (network_env / edge / seal / exhaust / sla / …) |
//! | `net_class` | Transport subtype (client_fetch_fail / offline / edge_challenge / …) |
//! | `impact_band` | commercial_critical · deepen_optional · lifecycle · noise |
//! | `actionability` | page_ops · infra · client_env · expected · investigate |
//! | `cause_class` | program · network_env · short_visit · expected (triage root) |
//! | `severity` | normalized (may demote false upload_5xx → warn) |
//!
//! Never hard-blocks sessions. Enrichment only.

use serde_json::{json, Map, Value};

pub const OPS_TAXONOMY_ALGO: &str = "ops_taxonomy_v3";

/// Enrich client/server ops detail + optionally re-normalize code/severity.
/// Returns (code, severity, detail_with_dims).
pub fn enrich_ops_event(code: &str, severity: &str, detail: &Value) -> (String, String, Value) {
    let mut d = detail.as_object().cloned().unwrap_or_default();
    let mut code_out = code.trim().to_string();
    let mut sev_out = severity.trim().to_ascii_lowercase();
    if sev_out.is_empty() {
        sev_out = "error".into();
    }

    // ── sticky-FE repair: false upload_5xx with http:0 / network ──────────
    let http = d
        .get("http")
        .or_else(|| d.get("status"))
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)));
    let network = d.get("network").and_then(|v| v.as_bool()).unwrap_or(false)
        || d.get("transport")
            .and_then(|v| v.as_str())
            .map(|s| s == "no_http_response")
            .unwrap_or(false);
    let err = d
        .get("err")
        .or_else(|| d.get("error"))
        .or_else(|| d.get("message"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if code_out == "upload_5xx" && (http == Some(0) || network || err.contains("failed to fetch")) {
        code_out = "upload_network".into();
        sev_out = "warn".into();
        d.entry("net_class")
            .or_insert_with(|| json!("client_fetch_fail"));
        d.entry("transport")
            .or_insert_with(|| json!("no_http_response"));
        d.insert("severity_reclass".into(), json!("sticky_fe_false_5xx"));
    }

    // Connection closed mid-stream mislabeled as 5xx
    if code_out == "upload_5xx"
        && (err.contains("connection closed")
            || err.contains("broken pipe")
            || err.contains("econnreset")
            || err.contains("upstream prematurely"))
    {
        code_out = "upload_upstream_closed".into();
        sev_out = if d.get("final").and_then(|v| v.as_bool()) == Some(true) {
            "error".into()
        } else {
            "warn".into()
        };
        d.entry("net_class")
            .or_insert_with(|| json!("upstream_closed"));
        d.insert("severity_reclass".into(), json!("upstream_closed_not_app_5xx"));
    }

    // Ensure net_class for upload_network
    if code_out == "upload_network" {
        if d.get("net_class").and_then(|v| v.as_str()).is_none() {
            let nc = if d.get("online").and_then(|v| v.as_bool()) == Some(false) {
                "offline"
            } else if err.contains("challenge") || err.contains("cloudflare") {
                "edge_challenge"
            } else if err.contains("abort") {
                "abort"
            } else {
                "client_fetch_fail"
            };
            d.insert("net_class".into(), json!(nc));
        }
        d.entry("transport")
            .or_insert_with(|| json!("no_http_response"));
        if sev_out == "error" {
            // Network without HTTP is never true API 5xx
            sev_out = "warn".into();
            d.insert("severity_reclass".into(), json!("network_not_api_error"));
        }
    }

    // micro_fetch abort → always warn + net_class
    if code_out == "micro_fetch_fail" {
        if err.contains("abort") {
            d.entry("net_class").or_insert_with(|| json!("abort"));
        }
        if sev_out == "error" {
            sev_out = "warn".into();
        }
    }

    // Pack reload fail during unload / abort is environmental (instant-close matrix).
    if code_out == "fe_pack_reload_fail" {
        let unloading = d.get("unloading").and_then(|v| v.as_bool()).unwrap_or(false)
            || d.get("aborted").and_then(|v| v.as_bool()).unwrap_or(false)
            || err.contains("abort")
            || err.contains("failed to fetch")
            || err.contains("networkerror");
        if unloading && sev_out == "error" {
            sev_out = "warn".into();
            d.insert(
                "severity_reclass".into(),
                json!("pack_reload_abort_or_unload"),
            );
        }
    }

    // seal wasm asset/network miss → warn (incomplete dist / heal storm); not app panic
    if code_out == "seal_wasm_fail"
        && (err.contains("wasm_http_404") || err.contains("failed to fetch") || err.contains("networkerror"))
        && sev_out == "error"
    {
        sev_out = "warn".into();
        d.insert(
            "severity_reclass".into(),
            json!("seal_wasm_asset_or_network"),
        );
    }

    // Soft/deepen exhaust should never be error when B10 already ok
    if matches!(
        code_out.as_str(),
        "upload_soft_exhausted" | "upload_deepen_exhausted"
    ) {
        if sev_out == "error" {
            sev_out = "warn".into();
            d.insert("severity_reclass".into(), json!("soft_exhaust_not_error"));
        }
    }

    // CF / edge challenge is environmental — never treat as app error
    if code_out == "upload_4xx"
        && (d.get("cf_challenge").and_then(|v| v.as_bool()) == Some(true)
            || d.get("net_class").and_then(|v| v.as_str()) == Some("edge_challenge")
            || err.contains("challenge")
            || err.contains("cloudflare")
            || err.contains("under attack"))
    {
        d.entry("net_class")
            .or_insert_with(|| json!("edge_challenge"));
        d.insert("cf_challenge".into(), json!(true));
        if sev_out == "error" {
            sev_out = "warn".into();
            d.insert("severity_reclass".into(), json!("edge_challenge_not_app_error"));
        }
    }

    // Lifecycle remint is noise
    if code_out == "cycle_remint" && sev_out == "error" {
        sev_out = "info".into();
    }

    // b10x_must_land / short visit — warn only
    if code_out == "b10x_must_land" && sev_out == "error" {
        sev_out = "warn".into();
    }

    // Deep-freeze restored shared-ref churn — self-heal, not a warn flood.
    if code_out == "capture_mutation_restored" {
        sev_out = "info".into();
        d.insert("severity_reclass".into(), json!("capture_restore_noise"));
    }

    // wave2 pack-schedule lifecycle — never error; demote warn → info (expected).
    if matches!(code_out.as_str(), "wave2_defer_b10" | "wave2_empty") {
        if sev_out == "error" || sev_out == "warn" {
            sev_out = "info".into();
            d.insert("severity_reclass".into(), json!("pack_schedule_lifecycle"));
        }
    }

    // Successful or partial pack reload is self-heal noise, not program fault.
    // Real failures use fe_pack_reload_fail.
    if code_out == "fe_pack_reload" {
        let ok = d.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        let algo_ok = d.get("algo_ok").and_then(|v| v.as_bool()).unwrap_or(false);
        let algo = d
            .get("algo")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let algo_is_known = algo.contains("v3_multiround")
            || algo.contains("v2_multiworkload")
            || algo.contains("gr_cpu_curve_v3")
            || algo.contains("gr_cpu_curve_v2");
        if ok || algo_ok || algo_is_known {
            if sev_out != "info" {
                sev_out = "info".into();
                d.insert("severity_reclass".into(), json!("pack_reload_ok_or_known_algo"));
            }
        } else if sev_out == "error" {
            sev_out = "warn".into();
            d.insert("severity_reclass".into(), json!("pack_reload_incomplete"));
        }
    }

    // b10_content_gate: only warn when content still bad after reload attempt.
    // False positive: expected=v2 while live algo is already v3.
    if code_out == "b10_content_gate" {
        let ok = d.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        let algo = d
            .get("algo")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let expected = d
            .get("expected")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let algo_ok = algo.contains("v3_multiround")
            || algo.contains("gr_cpu_curve_v3")
            || algo.contains("v2_multiworkload")
            || algo.contains("gr_cpu_curve_v2");
        let legacy_mismatch = algo.contains("v3")
            && (expected.contains("v2_multiworkload") || expected.contains("v2"));
        if ok || algo_ok || legacy_mismatch {
            if sev_out != "info" {
                sev_out = "info".into();
                d.insert(
                    "severity_reclass".into(),
                    json!(if legacy_mismatch {
                        "b10_gate_v3_vs_stale_expected_v2"
                    } else {
                        "b10_gate_content_ok"
                    }),
                );
            }
        } else if sev_out == "error" {
            sev_out = "warn".into();
        }
    }

    // Non-final upstream_closed / network with will_retry → keep warn (already),
    // stamp hint for dashboards (transient vs exhausted).
    if matches!(
        code_out.as_str(),
        "upload_upstream_closed" | "upload_network"
    ) {
        let will_retry = d.get("will_retry").and_then(|v| v.as_bool()) == Some(true);
        let final_fail = d.get("final").and_then(|v| v.as_bool()) == Some(true);
        if will_retry && !final_fail {
            d.entry("transient_network").or_insert_with(|| json!(true));
            if sev_out == "error" {
                sev_out = "warn".into();
            }
        }
    }

    // upload_4xx fe_impl_epoch skew during rollout — operational, not app panic.
    if code_out == "upload_4xx" {
        let epoch_rej = err.contains("fe_impl_epoch_rejected")
            || err.contains("fe_impl_missing")
            || err.contains("b10_algo_rejected");
        if epoch_rej {
            d.entry("fail_class_hint")
                .or_insert_with(|| json!("version_skew"));
            if sev_out == "error" {
                sev_out = "warn".into();
                d.insert("severity_reclass".into(), json!("fe_impl_epoch_skew"));
            }
        }
    }

    // b10_sla_miss during short visit / unload is environmental, not app SLA panic.
    if code_out == "b10_sla_miss" {
        let detail_s = d
            .get("detail")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let short = d.get("short_visit").and_then(|v| v.as_bool()) == Some(true)
            || d.get("unloading").and_then(|v| v.as_bool()) == Some(true)
            || d.get("pagehide").and_then(|v| v.as_bool()) == Some(true)
            || d.get("visibility_hidden").and_then(|v| v.as_bool()) == Some(true)
            || d.get("dwell_ms")
                .and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f as u64)))
                .map(|ms| ms > 0 && ms < 4000)
                .unwrap_or(false)
            // need_b10_without_batch = session closed before any B10 land (race / bounce)
            || detail_s.contains("need_b10_without_batch")
            || detail_s.contains("without_batch");
        if short {
            d.insert("short_visit".into(), json!(true));
            if sev_out == "error" {
                sev_out = "warn".into();
                d.insert("severity_reclass".into(), json!("b10_sla_short_visit"));
            }
        }
    }

    let dims = compute_dimensions(&code_out, &sev_out, &d);
    for (k, v) in dims {
        // Do not overwrite richer FE-supplied values
        d.entry(k).or_insert(v);
    }
    d.insert("taxonomy_algo".into(), json!(OPS_TAXONOMY_ALGO));

    (code_out, sev_out, Value::Object(d))
}

fn compute_dimensions(code: &str, severity: &str, d: &Map<String, Value>) -> Map<String, Value> {
    let net_class = d
        .get("net_class")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let commercial_hard = d
        .get("commercial_hard")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let deepen = d.get("deepen").and_then(|v| v.as_bool()).unwrap_or(false)
        || code.contains("deepen");
    let b10_ok = d
        .get("b10_already_ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let short_visit = d.get("short_visit").and_then(|v| v.as_bool()).unwrap_or(false)
        || d.get("unloading").and_then(|v| v.as_bool()).unwrap_or(false)
        || d.get("pagehide").and_then(|v| v.as_bool()).unwrap_or(false);
    let transient_net = d
        .get("transient_network")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || (d.get("will_retry").and_then(|v| v.as_bool()) == Some(true)
            && d.get("final").and_then(|v| v.as_bool()) != Some(true));

    let fail_class = match code {
        "upload_network" => match net_class.as_str() {
            "edge_challenge" => "edge_challenge",
            "offline" => "client_offline",
            "abort" | "backgrounded" => "client_abort",
            "upstream_closed" => "upstream_closed",
            _ => "network_env",
        },
        "upload_upstream_closed" => "upstream_closed",
        "upload_soft_exhausted" => "soft_exhaust",
        "upload_deepen_exhausted" => "deepen_exhaust",
        "upload_hard_exhausted" => "hard_exhaust",
        "upload_5xx" => "server_5xx",
        "upload_4xx" => {
            let epoch_skew = d.get("fail_class_hint").and_then(|v| v.as_str())
                == Some("version_skew")
                || d
                    .get("err")
                    .or_else(|| d.get("error"))
                    .and_then(|v| v.as_str())
                    .map(|s| {
                        let s = s.to_ascii_lowercase();
                        s.contains("fe_impl_epoch") || s.contains("b10_algo_rejected")
                    })
                    .unwrap_or(false);
            if epoch_skew {
                "version_skew"
            } else if d.get("cf_challenge").and_then(|v| v.as_bool()) == Some(true)
                || net_class == "edge_challenge"
            {
                "edge_challenge"
            } else {
                "client_or_auth_4xx"
            }
        }
        "upload_seal_fail" | "seal_wasm_fail" | "seal_wasm_unsupported" => "seal",
        "fe_pack_reload_fail" | "loader_script_fail" | "micro_fetch_fail" => "pack_or_bootstrap",
        "fe_pack_reload" => "expected_lifecycle",
        "b10_content_gate" => {
            // Stale expected=v2 vs live v3, or content already ok → lifecycle; else pack schedule.
            let algo = d
                .get("algo")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let expected = d
                .get("expected")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let ok = d.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            let algo_ok = algo.contains("v3_multiround")
                || algo.contains("gr_cpu_curve_v3")
                || algo.contains("v2_multiworkload")
                || algo.contains("gr_cpu_curve_v2");
            if ok
                || algo_ok
                || (algo.contains("v3")
                    && (expected.contains("v2_multiworkload") || expected.contains("v2")))
            {
                "expected_lifecycle"
            } else {
                "pack_schedule"
            }
        }
        "b10_sla_miss" if short_visit => "short_visit_silicon",
        "b10_sla_miss" => "server_sla",
        "b10x_must_land" => "short_visit_silicon",
        "capture_mutation_restored" => "expected_lifecycle",
        "capture_mutated_blocked" => "probe_runtime",
        "wave2_defer_b10" => "pack_schedule",
        "cycle_remint" | "vt_mint" | "rpa_quiet" | "cool_without_silicon" => "expected_lifecycle",
        "webgl_context_lost" => "probe_runtime",
        "wave2_empty" => "pack_schedule",
        c if c.starts_with("atlas_") || c == "E11" => "atlas_adjudication",
        _ => "other",
    };

    let impact_band = if code == "b10_sla_miss" && short_visit {
        "lifecycle"
    } else if code == "capture_mutation_restored" || severity == "info" {
        "noise"
    } else if (fail_class == "upstream_closed" || fail_class == "network_env") && transient_net {
        // Retriable blip — not commercial critical until exhausted.
        if commercial_hard {
            "ops_attention"
        } else {
            "deepen_optional"
        }
    } else if commercial_hard || code == "upload_hard_exhausted" || code == "b10_sla_miss" {
        "commercial_critical"
    } else if deepen
        || code == "upload_deepen_exhausted"
        || code == "b10x_must_land"
        || code.starts_with("B10x")
    {
        "deepen_optional"
    } else if matches!(
        fail_class,
        "expected_lifecycle" | "short_visit_silicon" | "pack_schedule" | "version_skew"
    ) {
        "lifecycle"
    } else if fail_class == "expected_lifecycle" {
        "noise"
    } else if b10_ok && matches!(fail_class, "soft_exhaust" | "deepen_exhaust" | "network_env") {
        // B10 already landed — non-critical residual fails
        "deepen_optional"
    } else {
        "ops_attention"
    };

    let actionability = match fail_class {
        "edge_challenge" | "upstream_closed" | "server_5xx" => "infra",
        "network_env" | "client_offline" | "client_abort" => "client_env",
        "expected_lifecycle" | "short_visit_silicon" | "pack_schedule" | "version_skew" => {
            "expected"
        }
        "hard_exhaust" | "server_sla" | "seal" | "pack_or_bootstrap" => "investigate",
        "probe_runtime" if code == "capture_mutated_blocked" => "investigate",
        "soft_exhaust" | "deepen_exhaust" if b10_ok => "expected",
        "soft_exhaust" | "deepen_exhaust" => "page_ops",
        "atlas_adjudication" => "investigate",
        _ => "investigate",
    };

    // Root-cause bucket for dashboards: program vs env vs short-visit vs expected.
    let cause_class = if matches!(
        fail_class,
        "expected_lifecycle" | "pack_schedule" | "short_visit_silicon" | "version_skew"
    ) || code == "capture_mutation_restored"
    {
        if fail_class == "short_visit_silicon" || short_visit {
            "short_visit"
        } else {
            "expected"
        }
    } else if matches!(
        fail_class,
        "network_env"
            | "upstream_closed"
            | "client_offline"
            | "client_abort"
            | "edge_challenge"
            | "server_5xx"
    ) {
        "network_env"
    } else if matches!(
        fail_class,
        "probe_runtime" | "hard_exhaust" | "server_sla" | "seal" | "pack_or_bootstrap"
    ) {
        "program"
    } else if matches!(fail_class, "soft_exhaust" | "deepen_exhaust") {
        if short_visit {
            "short_visit"
        } else if b10_ok {
            "expected"
        } else {
            "program"
        }
    } else {
        "program"
    };

    let mut m = Map::new();
    m.insert("fail_class".into(), json!(fail_class));
    m.insert("impact_band".into(), json!(impact_band));
    m.insert("actionability".into(), json!(actionability));
    m.insert("cause_class".into(), json!(cause_class));
    if !net_class.is_empty() {
        m.insert("net_class".into(), json!(net_class));
    }
    // Query-friendly rollup key
    m.insert(
        "dim_key".into(),
        json!(format!(
            "{code}|{fail_class}|{impact_band}|{cause_class}|{}",
            if net_class.is_empty() {
                "-"
            } else {
                net_class.as_str()
            }
        )),
    );
    m
}

/// Aggregate helper for reports / KPI scripts.
pub fn summarize_fail_class_hist(events: &[(String, String, Value)]) -> Value {
    let mut by_fc: Map<String, Value> = Map::new();
    for (code, sev, detail) in events {
        let (_, _, enriched) = enrich_ops_event(code, sev, detail);
        let fc = enriched
            .get("fail_class")
            .and_then(|v| v.as_str())
            .unwrap_or("other");
        let n = by_fc.get(fc).and_then(|v| v.as_u64()).unwrap_or(0) + 1;
        by_fc.insert(fc.into(), json!(n));
    }
    json!({
        "algo": OPS_TAXONOMY_ALGO,
        "by_fail_class": by_fc,
        "n": events.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reclass_false_upload_5xx_http0() {
        let (c, s, d) = enrich_ops_event(
            "upload_5xx",
            "error",
            &json!({"http": 0, "err": "Failed to fetch", "network": true}),
        );
        assert_eq!(c, "upload_network");
        assert_eq!(s, "warn");
        assert_eq!(d["fail_class"], "network_env");
        assert_eq!(d["net_class"], "client_fetch_fail");
        assert_eq!(d["actionability"], "client_env");
    }

    #[test]
    fn edge_challenge_dims() {
        let (c, s, d) = enrich_ops_event(
            "upload_4xx",
            "error",
            &json!({"http": 403, "cf_challenge": true, "net_class": "edge_challenge"}),
        );
        assert_eq!(c, "upload_4xx");
        // severity stays unless we want demote — FE already warns; keep if error only when not reclass path
        assert_eq!(d["fail_class"], "edge_challenge");
        assert_eq!(d["actionability"], "infra");
        let _ = s;
    }

    #[test]
    fn soft_exhaust_b10_ok_is_expected() {
        let (c, s, d) = enrich_ops_event(
            "upload_soft_exhausted",
            "error", // sticky wrong severity
            &json!({"batch_id": "B19_eme_media", "b10_already_ok": true, "commercial_hard": false}),
        );
        assert_eq!(c, "upload_soft_exhausted");
        assert_eq!(s, "warn");
        assert_eq!(d["fail_class"], "soft_exhaust");
        assert_eq!(d["impact_band"], "deepen_optional");
        assert_eq!(d["actionability"], "expected");
    }

    #[test]
    fn hard_exhaust_commercial_critical() {
        let (_, s, d) = enrich_ops_event(
            "upload_hard_exhausted",
            "error",
            &json!({"commercial_hard": true, "batch_id": "B10_hw_curves"}),
        );
        assert_eq!(s, "error");
        assert_eq!(d["fail_class"], "hard_exhaust");
        assert_eq!(d["impact_band"], "commercial_critical");
        assert_eq!(d["actionability"], "investigate");
    }

    #[test]
    fn upstream_closed_from_5xx_body() {
        let (c, s, d) = enrich_ops_event(
            "upload_5xx",
            "error",
            &json!({"http": 500, "err": "connection closed before message completed"}),
        );
        assert_eq!(c, "upload_upstream_closed");
        assert_eq!(s, "warn");
        assert_eq!(d["fail_class"], "upstream_closed");
    }

    #[test]
    fn deepen_exhaust_dims() {
        let (_, s, d) = enrich_ops_event(
            "upload_deepen_exhausted",
            "warn",
            &json!({"batch_id": "B10x_silicon_ulp", "deepen": true, "b10_already_ok": true}),
        );
        assert_eq!(s, "warn");
        assert_eq!(d["fail_class"], "deepen_exhaust");
        assert_eq!(d["impact_band"], "deepen_optional");
    }

    #[test]
    fn lifecycle_cycle_remint_info() {
        let (c, s, d) = enrich_ops_event("cycle_remint", "error", &json!({}));
        assert_eq!(c, "cycle_remint");
        assert_eq!(s, "info");
        assert_eq!(d["fail_class"], "expected_lifecycle");
        assert_eq!(d["cause_class"], "expected");
    }

    #[test]
    fn capture_restore_is_expected_noise() {
        let (c, s, d) = enrich_ops_event(
            "capture_mutation_restored",
            "warn",
            &json!({"batch_id": "B0_bootstrap", "restored": true}),
        );
        assert_eq!(c, "capture_mutation_restored");
        assert_eq!(s, "info");
        assert_eq!(d["fail_class"], "expected_lifecycle");
        assert_eq!(d["impact_band"], "noise");
        assert_eq!(d["cause_class"], "expected");
        assert_eq!(d["actionability"], "expected");
    }

    #[test]
    fn capture_blocked_is_program() {
        let (_, s, d) = enrich_ops_event(
            "capture_mutated_blocked",
            "warn",
            &json!({"batch_id": "B1_conflict", "restored": false}),
        );
        assert_eq!(s, "warn");
        assert_eq!(d["fail_class"], "probe_runtime");
        assert_eq!(d["cause_class"], "program");
        assert_eq!(d["actionability"], "investigate");
    }

    #[test]
    fn b10_sla_short_visit_demoted() {
        let (c, s, d) = enrich_ops_event(
            "b10_sla_miss",
            "error",
            &json!({"short_visit": true, "dwell_ms": 1800}),
        );
        assert_eq!(c, "b10_sla_miss");
        assert_eq!(s, "warn");
        assert_eq!(d["fail_class"], "short_visit_silicon");
        assert_eq!(d["impact_band"], "lifecycle");
        assert_eq!(d["cause_class"], "short_visit");
        assert_eq!(d["actionability"], "expected");
    }

    #[test]
    fn upstream_closed_transient_not_critical() {
        let (c, s, d) = enrich_ops_event(
            "upload_upstream_closed",
            "warn",
            &json!({
                "http": 500,
                "err": "connection closed before message completed",
                "will_retry": true,
                "final": false,
                "commercial_hard": true,
                "batch_id": "B10_hw_curves"
            }),
        );
        assert_eq!(c, "upload_upstream_closed");
        assert_eq!(s, "warn");
        assert_eq!(d["fail_class"], "upstream_closed");
        assert_eq!(d["cause_class"], "network_env");
        assert_eq!(d["impact_band"], "ops_attention");
        assert_eq!(d["transient_network"], true);
    }

    #[test]
    fn wave2_defer_is_pack_schedule_expected() {
        let (_, s, d) = enrich_ops_event(
            "wave2_defer_b10",
            "warn",
            &json!({"reason": "packs_content_not_ok"}),
        );
        // Lifecycle demote: warn → info
        assert_eq!(s, "info");
        assert_eq!(d["fail_class"], "pack_schedule");
        assert_eq!(d["cause_class"], "expected");
        assert_eq!(d["actionability"], "expected");
    }

    #[test]
    fn wave2_empty_demoted_to_info() {
        let (_, s, d) = enrich_ops_event("wave2_empty", "warn", &json!({"attempt": 1}));
        assert_eq!(s, "info");
        assert_eq!(d["fail_class"], "pack_schedule");
        assert_eq!(d["actionability"], "expected");
    }

    #[test]
    fn fe_pack_reload_ok_is_info_lifecycle() {
        let (_, s, d) = enrich_ops_event(
            "fe_pack_reload",
            "warn",
            &json!({"ok": true, "algo_ok": true, "algo": "gr_cpu_curve_v3_multiround_median"}),
        );
        assert_eq!(s, "info");
        assert_eq!(d["fail_class"], "expected_lifecycle");
        assert_eq!(d["cause_class"], "expected");
        assert_eq!(d["actionability"], "expected");
    }

    #[test]
    fn b10_content_gate_v3_vs_stale_expected_v2_is_info() {
        let (_, s, d) = enrich_ops_event(
            "b10_content_gate",
            "warn",
            &json!({
                "ok": false,
                "algo": "gr_cpu_curve_v3_multiround_median",
                "expected": "gr_cpu_curve_v2_multiworkload"
            }),
        );
        assert_eq!(s, "info");
        assert_eq!(d["fail_class"], "expected_lifecycle");
        assert_eq!(d["cause_class"], "expected");
        assert_eq!(
            d["severity_reclass"],
            "b10_gate_v3_vs_stale_expected_v2"
        );
    }

    #[test]
    fn upload_4xx_fe_impl_epoch_is_version_skew_warn() {
        let (c, s, d) = enrich_ops_event(
            "upload_4xx",
            "error",
            &json!({
                "http": 422,
                "err": "seal_policy: fe_impl_epoch_rejected: fe_impl_version=v5.8.161-x"
            }),
        );
        assert_eq!(c, "upload_4xx");
        assert_eq!(s, "warn");
        assert_eq!(d["fail_class"], "version_skew");
        assert_eq!(d["actionability"], "expected");
        assert_eq!(d["impact_band"], "lifecycle");
    }

    #[test]
    fn b10_sla_need_without_batch_demoted() {
        let (_, s, d) = enrich_ops_event(
            "b10_sla_miss",
            "error",
            &json!({"detail": "need_b10_without_batch", "cause_class": "program"}),
        );
        assert_eq!(s, "warn");
        assert_eq!(d["fail_class"], "short_visit_silicon");
        assert_eq!(d["impact_band"], "lifecycle");
        assert_eq!(d["actionability"], "expected");
    }

    #[test]
    fn summarize_hist() {
        let events = vec![
            (
                "upload_network".into(),
                "warn".into(),
                json!({"net_class": "client_fetch_fail"}),
            ),
            (
                "upload_5xx".into(),
                "error".into(),
                json!({"http": 0, "err": "Failed to fetch"}),
            ),
            (
                "upload_hard_exhausted".into(),
                "error".into(),
                json!({"commercial_hard": true}),
            ),
        ];
        let s = summarize_fail_class_hist(&events);
        assert_eq!(s["n"], 3);
        // first two become network_env after reclass
        assert!(s["by_fail_class"]["network_env"].as_u64().unwrap() >= 2);
        assert_eq!(s["by_fail_class"]["hard_exhaust"], 1);
    }
}
