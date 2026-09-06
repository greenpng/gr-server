//! JA4H — HTTP header fingerprint (iss/45 B8).
//!
//! FoxIO-inspired partial: method + cookie/referer flags + accept family +
//! header-name order digest. True HPACK wire-order / case entropy still unavailable
//! at HTTP app layer (names lowercased). Product surface never commercial mint.

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

pub const JA4H_LITE_ALGO: &str = "ja4h_lite_v1";
pub const JA4H_ALGO: &str = "ja4h_partial_v2";

fn s(fo: &Map<String, Value>, keys: &[&str]) -> String {
    for k in keys {
        if let Some(v) = fo.get(*k).and_then(|x| x.as_str()) {
            if !v.is_empty() {
                return v.to_string();
            }
        }
    }
    String::new()
}

fn flag_cookie(order: &str, fields: &Map<String, Value>) -> char {
    if order.split(',').any(|h| h == "cookie")
        || fields.contains_key("cookie")
        || fields.get("cookie_present").and_then(|v| v.as_bool()).unwrap_or(false)
    {
        'c'
    } else {
        'n'
    }
}

fn flag_referer(order: &str, fields: &Map<String, Value>) -> char {
    if order.split(',').any(|h| h == "referer" || h == "referrer")
        || fields.contains_key("referer")
    {
        'r'
    } else {
        'n'
    }
}

fn accept_lang_bucket(fields: &Map<String, Value>) -> String {
    let al = s(fields, &["accept_language", "gateway_accept_language", "accept-language"]);
    if al.is_empty() {
        return "00".into();
    }
    let mut h = Sha256::new();
    h.update(al.as_bytes());
    format!("{:x}", h.finalize())[..2].to_string()
}

/// Full product-facing JA4H partial + lite order surface.
pub fn ja4h_lite_product(fields: &Value) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let order = s(
        &fo,
        &["ja4h_lite_order", "http_header_order", "gateway_http_header_order"],
    );
    let pre_hash = s(&fo, &["ja4h_lite", "http_header_order_hash"]);
    let order_digest = if !pre_hash.is_empty() {
        pre_hash.trim_start_matches("h_").to_string()
    } else if !order.is_empty() {
        let mut h = Sha256::new();
        h.update(order.as_bytes());
        format!("{:x}", h.finalize())[..16].to_string()
    } else {
        String::new()
    };
    let n_headers = if order.is_empty() {
        0
    } else {
        order.split(',').filter(|s| !s.is_empty()).count()
    };
    let method = s(&fo, &["http_method", "gateway_http_method", "method"]);
    let method = if method.is_empty() {
        "get".to_string()
    } else {
        method.to_ascii_lowercase()
    };
    let http_ver_raw = s(&fo, &["http_version", "gateway_http_version"]);
    let http_ver: String = if http_ver_raw.is_empty() {
        "2".to_string()
    } else if http_ver_raw.contains('3') {
        "3".to_string()
    } else if http_ver_raw.contains('2') {
        "2".to_string()
    } else {
        "1".to_string()
    };
    let cookie_f = flag_cookie(&order, &fo);
    let referer_f = flag_referer(&order, &fo);
    let al_b = accept_lang_bucket(&fo);
    // FoxIO-like composite: method_ver_cookie_referer_nhdrs_alang_order12
    let composite_raw = format!(
        "{method}_{http_ver}_{cookie_f}{referer_f}_{n_headers:02}_{al_b}_{}",
        if order_digest.len() >= 12 {
            &order_digest[..12]
        } else {
            &order_digest
        }
    );
    let mut h = Sha256::new();
    h.update(composite_raw.as_bytes());
    let ja4h = format!("h2_{}", &format!("{:x}", h.finalize())[..12]);

    let present = !order_digest.is_empty() || !order.is_empty() || fo.contains_key("http_method");
    json!({
        "algo": JA4H_ALGO,
        "lite_algo": JA4H_LITE_ALGO,
        "present": present,
        "ja4h": if present { json!(ja4h) } else { Value::Null },
        "ja4h_lite": if order_digest.is_empty() { Value::Null } else { json!(format!("h_{order_digest}")) },
        "header_order": if order.is_empty() { Value::Null } else { json!(order) },
        "n_header_names": n_headers,
        "method": method,
        "http_version_part": http_ver,
        "cookie_flag": cookie_f.to_string(),
        "referer_flag": referer_f.to_string(),
        "accept_lang_bucket": al_b,
        "role": "header_order_device_segment_oi",
        // Opaque digests may fill device_segments oi (not UA/IP/header values).
        "commercial_mint": true,
        "commercial_mint_scope": "device_segment_oi_protocol_header_digest_only",
        "hard_browser_uniqueness": false,
        "case_entropy_available": false,
        "full_foxio_ja4h": false,
        "foxio_partial": true,
        "hpack_pseudo_wire_order": false,
        "note": "iss/45 B8: order digest OK for commercial oi segment; never raw header values/UA/IP",
    })
}

/// Annotate fields map with ja4h keys when order/hash present.
pub fn ensure_ja4h_fields(fields: &mut Map<String, Value>) {
    let order = fields
        .get("http_header_order")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let hash = fields
        .get("http_header_order_hash")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if !order.is_empty() {
        fields.insert("ja4h_lite_order".into(), json!(order));
    }
    if order.is_empty() && hash.is_empty() && !fields.contains_key("http_method") {
        return;
    }
    if !fields.contains_key("ja4h_lite") {
        let dig = if !hash.is_empty() {
            hash.trim_start_matches("h_").to_string()
        } else if !order.is_empty() {
            let mut h = Sha256::new();
            h.update(order.as_bytes());
            format!("{:x}", h.finalize())[..16].to_string()
        } else {
            String::new()
        };
        if !dig.is_empty() {
            fields.insert("ja4h_lite".into(), json!(format!("h_{dig}")));
        }
    }
    fields.insert("ja4h_lite_role".into(), json!("header_order_device_segment_oi"));
    fields.insert("ja4h_lite__commercial_mint".into(), json!(true));
    fields.insert(
        "ja4h_lite__commercial_mint_scope".into(),
        json!("device_segment_oi_digest_only"),
    );
    let prod = ja4h_lite_product(&Value::Object(fields.clone()));
    if let Some(j) = prod.get("ja4h") {
        fields.insert("ja4h".into(), j.clone());
        fields.insert("ja4h_role".into(), json!("http_header_fp_device_segment_oi"));
        fields.insert("ja4h__commercial_mint".into(), json!(true));
        fields.insert(
            "ja4h__commercial_mint_scope".into(),
            json!("device_segment_oi_digest_only"),
        );
        fields.insert("ja4h__foxio_partial".into(), json!(true));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_digest_stable() {
        let f = json!({
            "http_header_order": "host,user-agent,accept,accept-language,cookie",
            "http_method": "POST",
            "http_version": "HTTP/2.0",
            "accept_language": "en-US,en;q=0.9",
        });
        let p = ja4h_lite_product(&f);
        assert_eq!(p["present"], true);
        assert_eq!(p["commercial_mint"], true);
        assert_eq!(
            p["commercial_mint_scope"],
            "device_segment_oi_protocol_header_digest_only"
        );
        assert_eq!(p["foxio_partial"], true);
        assert!(p["ja4h"].as_str().unwrap().starts_with("h2_"));
        assert_eq!(p["cookie_flag"], "c");
        assert_eq!(p["n_header_names"], 5);
    }
}
