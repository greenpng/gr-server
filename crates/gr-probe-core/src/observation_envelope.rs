//! ObservationEnvelope v1 — provenance contract shared by FE ingest, Pingora S0, CF edge.
//!
//! Field dictionary (do not collapse these):
//! - `source_kind`: producer class (`fe` | `gateway` | `cloud_edge` | `backend`)
//! - `realm_kind`: execution context (`document` | `iframe` | `worker` | `sandbox_iframe` | `h3_connection` | `none`)
//! - `probe_method_id`: how it was measured (not a source alias)
//! Client claims are stored under `claimed`; only `validated` is trusted.

use serde_json::{json, Map, Value};
use std::collections::HashMap;

pub const OBSERVATION_ENVELOPE_SCHEMA: &str = "gr.observation_envelope.v1";

const METHOD_RE: &str = r"^[a-z][a-z0-9_.-]{0,62}$";

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn sanitize_token(raw: Option<&str>, max: usize) -> Option<String> {
    let s = raw?.trim();
    if s.is_empty() {
        return None;
    }
    let t: String = s.chars().take(max).collect();
    if t.chars().any(|c| c.is_control()) {
        return None;
    }
    Some(t)
}

/// Producer class from legacy `source` / inject path. Never confuses realm with source.
pub fn infer_source_kind(source: &str, inject_path: Option<&str>) -> &'static str {
    let s = source.trim().to_ascii_lowercase();
    let inj = inject_path.unwrap_or("").to_ascii_lowercase();
    if s.contains("cloudflare")
        || s == "cf"
        || s.starts_with("cf_")
        || inj == "cf_worker"
        || inj == "cloudflare"
    {
        return "cloud_edge";
    }
    if s.contains("gateway") || s == "b8" || inj == "nginx" {
        return "gateway";
    }
    if s.contains("backend") || s == "sdk" {
        return "backend";
    }
    "fe"
}

/// Execution context from legacy `source` (main/worker/iframe/sandbox).
pub fn infer_realm_kind(source: &str) -> &'static str {
    let s = source.trim().to_ascii_lowercase();
    if s.contains("worker") {
        return "worker";
    }
    if s.contains("sandbox") {
        return "sandbox_iframe";
    }
    if s.contains("iframe") {
        return "iframe";
    }
    if s.contains("h3") {
        return "h3_connection";
    }
    if s.contains("gateway") || s.contains("cloudflare") || s == "cf" {
        return "none";
    }
    "document"
}

pub fn infer_source_id(source: &str, source_kind: &str, realm_kind: &str) -> String {
    match source_kind {
        "cloud_edge" => "gw.cloudflare".into(),
        "gateway" => "gw.edge.tls".into(),
        "backend" => "be.result".into(),
        _ => match realm_kind {
            "worker" => format!("fe.worker.{}", short_src(source)),
            "iframe" => format!("fe.iframe.{}", short_src(source)),
            "sandbox_iframe" => format!("fe.sandbox.{}", short_src(source)),
            _ => "fe.main".into(),
        },
    }
}

fn short_src(source: &str) -> String {
    source
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
        .take(32)
        .collect()
}

fn method_ok(id: &str) -> bool {
    let re = regex::Regex::new(METHOD_RE).expect("method re");
    re.is_match(id)
}

/// Allowlisted method: catalog pack/alias or conservative token. Unknown → None (untrusted).
pub fn validate_probe_method_id(claimed: Option<&str>, batch_id: &str) -> Option<String> {
    let raw = sanitize_token(claimed, 64)?;
    let id = raw.to_ascii_lowercase();
    if !method_ok(&id) {
        return None;
    }
    if let Ok(cat) = crate::catalog::load_catalog() {
        if cat.resolve(&id).is_some() || cat.resolve(batch_id).is_some() {
            // method_id may be finer than pack (b10_default); accept token shape.
            return Some(id);
        }
    }
    // Known method families even if catalog pack id differs.
    if id.starts_with("b10")
        || id.starts_with("b11")
        || id.starts_with("b7")
        || id.starts_with("b8")
        || id.starts_with("b0")
        || id.starts_with("b1")
        || id.starts_with("b12")
        || id.starts_with("cf_")
        || id.starts_with("gateway_")
        || id == "legacy_unknown"
        || id == "gateway_tls"
        || id == "cf_http_headers"
    {
        return Some(id);
    }
    None
}

pub fn make_observation_id(
    session_id: &str,
    batch_id: &str,
    source: &str,
    attempt_id: Option<&str>,
    capture_id: Option<&str>,
) -> String {
    let seed = format!(
        "{}|{}|{}|{}|{}",
        session_id,
        batch_id,
        source,
        attempt_id.unwrap_or(""),
        capture_id.unwrap_or("")
    );
    let h = crate::storage_bind::sha256_hex(seed.as_bytes());
    format!("obs_{}", &h[..16])
}

/// Build server-authoritative envelope. Client claims never overwrite validated.
pub fn stamp_observation_envelope(
    payload: &mut Value,
    session_id: &str,
    batch_id: &str,
    source: &str,
    inject_path: Option<&str>,
    claimed: &ClaimedEnvelope,
) -> Value {
    let inferred_kind = infer_source_kind(source, inject_path);
    let inferred_realm = infer_realm_kind(source);
    let claimed_kind = claimed
        .source_kind
        .as_deref()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| matches!(s.as_str(), "fe" | "gateway" | "cloud_edge" | "backend"));
    // Client cannot promote itself to gateway/cloud_edge.
    let validated_kind = match claimed_kind.as_deref() {
        Some("fe") if inferred_kind == "fe" => "fe",
        Some("backend") if inferred_kind == "backend" => "backend",
        _ => inferred_kind,
    };
    let claimed_realm = claimed.realm_kind.as_deref().map(|s| s.trim().to_ascii_lowercase());
    let validated_realm = match claimed_realm.as_deref() {
        Some(r)
            if inferred_kind == "fe"
                && matches!(
                    r,
                    "document" | "iframe" | "worker" | "sandbox_iframe"
                ) =>
        {
            r
        }
        _ => inferred_realm,
    };
    let validated_method = validate_probe_method_id(claimed.probe_method_id.as_deref(), batch_id)
        .unwrap_or_else(|| "legacy_unknown".into());
    let source_id = infer_source_id(source, validated_kind, validated_realm);
    let observation_id = claimed
        .observation_id
        .as_deref()
        .and_then(|s| sanitize_token(Some(s), 80))
        .filter(|s| s.starts_with("obs_"))
        .unwrap_or_else(|| {
            make_observation_id(
                session_id,
                batch_id,
                source,
                claimed.attempt_id.as_deref(),
                claimed.capture_id.as_deref(),
            )
        });
    let snap_keys: Vec<String> = payload
        .get("fields")
        .and_then(|v| v.as_object())
        .map(|o| {
            let mut k: Vec<String> = o.keys().cloned().collect();
            k.sort();
            k
        })
        .unwrap_or_default();
    let snap_hex = crate::storage_bind::sha256_hex(snap_keys.join(",").as_bytes());
    let env = json!({
        "schema": OBSERVATION_ENVELOPE_SCHEMA,
        "observation_id": observation_id,
        "session_id": session_id,
        "probe_pack_id": batch_id,
        "batch_id": batch_id,
        "attempt_id": claimed.attempt_id,
        "capture_id": claimed.capture_id,
        "material_generation": claimed.material_generation,
        "claimed": {
            "source": source,
            "source_kind": claimed.source_kind,
            "realm_id": claimed.realm_id,
            "realm_kind": claimed.realm_kind,
            "probe_method_id": claimed.probe_method_id,
            "method_version": claimed.method_version,
        },
        "validated": {
            "source_id": source_id,
            "source_kind": validated_kind,
            "realm_kind": validated_realm,
            "realm_id": claimed.realm_id.clone().filter(|_| validated_kind == "fe"),
            "probe_method_id": validated_method,
            "trust_class": if validated_kind == "fe" { "client_sealed_claim" } else { "server_edge" },
            "provenance_quality": if claimed.probe_method_id.is_some() { "partial" } else { "legacy_batch" },
        },
        "lineage": {
            "equivalence_group": format!("{validated_kind}:{validated_method}"),
            "input_snapshot_hash": format!("sha256:{}", &snap_hex[..16.min(snap_hex.len())]),
            "duplicate_weight_policy": "retain_candidates_limit_repeat_weight",
        },
        "stamped_at_ms": now_ms(),
    });
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("observation".into(), env.clone());
    }
    env
}

#[derive(Clone, Debug, Default)]
pub struct ClaimedEnvelope {
    pub source_kind: Option<String>,
    pub realm_id: Option<String>,
    pub realm_kind: Option<String>,
    pub probe_method_id: Option<String>,
    pub method_version: Option<String>,
    pub observation_id: Option<String>,
    pub attempt_id: Option<String>,
    pub capture_id: Option<String>,
    pub material_generation: Option<i64>,
}

/// Per-signal honesty used by Pingora + CF edge.
pub fn signal_meta(
    truth_level: &str,
    availability: &str,
    scope: &str,
    commercial_eligible: bool,
    reason: &str,
) -> Value {
    json!({
        "truth_level": truth_level,
        "availability": availability,
        "scope": scope,
        "commercial_eligible": commercial_eligible,
        "reason": reason,
    })
}

fn header_ci<'a>(headers: &'a HashMap<String, String>, name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
        .filter(|s| !s.is_empty())
}

/// Cloudflare HTTP headers are a first-class S0 `cloud_edge` source.
/// Never accept client-forged `cf_*` body fields as validated.
pub fn extract_cf_edge_fields(headers: &HashMap<String, String>) -> Map<String, Value> {
    let mut cf = Map::new();
    let pairs = [
        ("cf-connecting-ip", "cf_connecting_ip"),
        ("true-client-ip", "true_client_ip"),
        ("cf-ipcountry", "country"),
        ("cf-ray", "cf_ray"),
        ("cf-visitor", "cf_visitor"),
        ("cf-asn", "asn"),
        ("cf-ipcity", "city"),
        ("cf-region", "region"),
        ("cf-region-code", "region_code"),
        ("cf-timezone", "timezone"),
        ("cf-iplatitude", "latitude"),
        ("cf-iplongitude", "longitude"),
        ("cf-ipcontinent", "continent"),
        ("cf-metro-code", "metro_code"),
        ("cf-postal-code", "postal_code"),
        ("cf-colo", "colo"),
        ("cf-pseudo-ipv4", "pseudo_ipv4"),
        ("cf-bot-score", "bot_score"),
        ("x-bot-score", "bot_score"),
        ("cf-threat-score", "threat_score"),
        ("cf-verified-bot", "verified_bot"),
        ("cf-verified-bot-category", "verified_bot_category"),
        ("cf-ja3-hash", "ja3"),
        ("cf-ja4", "ja4"),
        ("cf-ja4-digest", "ja4_digest"),
        ("cf-worker", "cf_worker"),
        ("cf-ew-via", "cf_ew_via"),
        ("cf-cache-status", "cache_status"),
        ("cdn-loop", "cdn_loop"),
        ("cf-as-organization", "as_organization"),
        ("cf-tls-version", "tls_version"),
        ("cf-tls-cipher", "tls_cipher"),
        ("cf-http-protocol", "http_protocol"),
    ];
    for (hdr, key) in pairs {
        if let Some(v) = header_ci(headers, hdr) {
            // iss/opus5 05-S-4: IP-bearing headers are truncated to the network
            // class at extraction (single masked form everywhere downstream).
            let masked;
            let v = if matches!(key, "cf_connecting_ip" | "true_client_ip" | "pseudo_ipv4") {
                masked = crate::privacy::apply_ip_policy(v);
                masked.as_str()
            } else {
                v
            };
            cf.entry(key.to_string()).or_insert_with(|| json!(v));
        }
    }
    let present = cf.contains_key("cf_connecting_ip")
        || cf.contains_key("cf_ray")
        || cf.contains_key("cf_visitor")
        || cf.contains_key("country")
        || cf.contains_key("cf_worker")
        || cf.contains_key("bot_score");
    if !present {
        return Map::new();
    }
    cf.insert("cf_edge_present".into(), json!(true));
    cf.insert(
        "s0_observation".into(),
        json!({
            "source_id": "gw.cloudflare",
            "source_kind": "cloud_edge",
            "trust_class": "cdn_edge_header",
            "truth_level": "observed",
            "availability": "observed",
            "scope": "request",
            "commercial_eligible": false,
            "reason": "cf_http_headers_are_network_axis_not_device_mint",
        }),
    );
    // IP from CF is egress at CF, not silicon.
    if cf.contains_key("cf_connecting_ip") {
        cf.insert(
            "cf_connecting_ip_meta".into(),
            signal_meta(
                "observed",
                "observed",
                "request",
                false,
                "cloudflare_connecting_ip_egress",
            ),
        );
    }
    if cf.contains_key("country") {
        cf.insert(
            "country_meta".into(),
            signal_meta(
                "observed",
                "observed",
                "request",
                false,
                "cloudflare_ipcountry_network",
            ),
        );
    }
    cf
}

pub fn merge_cf_into_gateway_fields(fields: &mut Map<String, Value>, headers: &HashMap<String, String>) {
    let cf = extract_cf_edge_fields(headers);
    if cf.is_empty() {
        fields.insert("cf_edge_present".into(), json!(false));
        return;
    }
    fields.insert("cf_edge_present".into(), json!(true));
    if let Some(ip) = cf.get("cf_connecting_ip").cloned() {
        fields.entry("cf_connecting_ip".to_string()).or_insert(ip);
    }
    if let Some(cc) = cf.get("country").cloned() {
        fields.entry("server_country".to_string()).or_insert(cc.clone());
        fields.insert("server_country_source".into(), json!("cf"));
        fields.insert("cf_ipcountry".into(), cc);
    }
    if let Some(asn) = cf.get("asn").cloned() {
        fields.entry("server_asn".to_string()).or_insert(asn.clone());
        fields.insert("server_asn_source".into(), json!("cf"));
        fields.insert("cf_asn".into(), asn);
    }
    if let Some(ray) = cf.get("cf_ray").cloned() {
        fields.insert("cf_ray".into(), ray);
    }
    fields.insert("cf_fields".into(), Value::Object(cf));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_kind_is_not_realm() {
        assert_eq!(infer_source_kind("main", None), "fe");
        assert_eq!(infer_realm_kind("main"), "document");
        assert_eq!(infer_source_kind("worker", None), "fe");
        assert_eq!(infer_realm_kind("worker"), "worker");
        assert_eq!(infer_source_kind("gateway", None), "gateway");
        assert_eq!(infer_realm_kind("gateway"), "none");
        assert_eq!(infer_source_kind("cloudflare", Some("cf_worker")), "cloud_edge");
    }

    #[test]
    fn client_cannot_claim_gateway() {
        let mut payload = json!({"fields": {}});
        let claimed = ClaimedEnvelope {
            source_kind: Some("gateway".into()),
            probe_method_id: Some("trusted_tls".into()),
            ..Default::default()
        };
        let env = stamp_observation_envelope(&mut payload, "s1", "B0_bootstrap", "main", None, &claimed);
        assert_eq!(env["validated"]["source_kind"], "fe");
        assert_eq!(env["claimed"]["source_kind"], "gateway");
        assert_eq!(env["validated"]["trust_class"], "client_sealed_claim");
    }

    #[test]
    fn site_capture_backend_source_is_distinct_from_fe() {
        let mut payload = json!({
            "fields": {
                "external_observation_schema": "gr.external_observation.v1"
            }
        });
        let claimed = ClaimedEnvelope {
            source_kind: Some("backend".into()),
            probe_method_id: Some("site_capture_browser_v1".into()),
            method_version: Some("1".into()),
            ..Default::default()
        };
        let env = stamp_observation_envelope(
            &mut payload,
            "s1",
            "ops.site_capture_observation",
            "backend:site_capture",
            None,
            &claimed,
        );
        assert_eq!(env["validated"]["source_kind"], "backend");
        assert_eq!(env["validated"]["source_id"], "be.result");
        assert_eq!(env["validated"]["trust_class"], "server_edge");
        assert_eq!(env["claimed"]["source_kind"], "backend");
    }

    #[test]
    fn cf_headers_are_cloud_edge() {
        let mut h = HashMap::new();
        h.insert("cf-connecting-ip".into(), "203.0.113.9".into());
        h.insert("cf-ipcountry".into(), "SG".into());
        h.insert("cf-ray".into(), "abc123".into());
        h.insert("cf-bot-score".into(), "29".into());
        h.insert("cf-ipcontinent".into(), "AS".into());
        h.insert("cf-verified-bot".into(), "false".into());
        let cf = extract_cf_edge_fields(&h);
        assert_eq!(cf["cf_edge_present"], json!(true));
        assert_eq!(cf["s0_observation"]["source_kind"], "cloud_edge");
        assert_eq!(cf["s0_observation"]["commercial_eligible"], json!(false));
        assert_eq!(cf["bot_score"], json!("29"));
        assert_eq!(cf["continent"], json!("AS"));
        assert_eq!(cf["verified_bot"], json!("false"));
    }

    #[test]
    fn cf_and_gateway_methods_are_allowlisted() {
        assert_eq!(
            validate_probe_method_id(Some("cf_http_headers"), "B8_gateway").as_deref(),
            Some("cf_http_headers")
        );
        assert_eq!(
            validate_probe_method_id(Some("b8_gateway_tls"), "B8_gateway").as_deref(),
            Some("b8_gateway_tls")
        );
    }

    #[test]
    fn envelope_carries_equivalence_lineage() {
        let mut payload = json!({"fields": {"ja4": "t13d"}});
        let env = stamp_observation_envelope(
            &mut payload,
            "s1",
            "B8_gateway",
            "gateway",
            Some("nginx"),
            &ClaimedEnvelope {
                probe_method_id: Some("b8_gateway_tls".into()),
                ..Default::default()
            },
        );
        assert_eq!(env["validated"]["source_kind"], "gateway");
        assert_eq!(env["lineage"]["equivalence_group"], "gateway:b8_gateway_tls");
        assert!(env["lineage"]["input_snapshot_hash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:"));
    }
}
