//! H13 protocol-edge: trusted reverse-proxy TLS/H2 fingerprints → gateway fields.
//!
//! Trust model: only values from **server-seen request headers** (nginx/CF/edge) are
//! authoritative. Client body must never invent JA4.

use serde_json::{json, Map, Value};

/// Header names accepted from reverse proxy (case-insensitive match by caller).
pub const TRUSTED_JA4_HEADERS: &[&str] = &[
    "x-tls-ja4",
    "x-ja4",
    "cf-ja4",
    "x-client-ja4",
];
pub const TRUSTED_JA3_HEADERS: &[&str] = &["x-tls-ja3", "x-ja3", "cf-ja3"];
pub const TRUSTED_H2_HEADERS: &[&str] = &[
    "x-http2-fingerprint",
    "x-h2-fingerprint",
    "x-akamai-h2",
    "cf-http2-fingerprint",
];
pub const TRUSTED_ALPN_HEADERS: &[&str] = &["x-tls-alpn", "x-alpn"];
pub const TRUSTED_TLS_VERSION_HEADERS: &[&str] = &["x-tls-version", "x-ssl-protocol"];

/// iss/21 T-ENG-1: map brand/protocol labels → engine family (not browser product name).
/// Families: blink | gecko | webkit | unknown
pub fn brand_to_engine_family(label: &str) -> &'static str {
    let l = label.to_ascii_lowercase();
    if l.is_empty() || l == "unknown" || l == "absent" || l == "unknown_tls13" {
        return "unknown";
    }
    if l.contains("firefox")
        || l.contains("gecko")
        || l.contains("nss")
        || l.starts_with("ff")
        || l.contains("mullvad")
        || l.contains("librewolf")
        || l.contains("tor")
    {
        return "gecko";
    }
    if l.contains("safari") || l.contains("webkit") || l.contains("apple") {
        return "webkit";
    }
    if l.contains("chrome")
        || l.contains("chromium")
        || l.contains("blink")
        || l.contains("edge")
        || l.starts_with("edg")
        || l.contains("brave")
        || l.contains("opera")
        || l.contains("boringssl")
        || l.starts_with("cr")
    {
        return "blink";
    }
    "unknown"
}

/// Derive engine_claim (from UA/CH) and engine_obs (from capability/protocol tells).
/// Returns (engine_claim, engine_obs). Both ∈ {blink,gecko,webkit,unknown}.
pub fn derive_engine_claim_obs(fields: &Map<String, Value>) -> (&'static str, &'static str) {
    let ua = fields
        .get("user_agent")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let claim = if ua.contains("firefox") {
        "gecko"
    } else if ua.contains("edg/") || ua.contains("edgios") {
        "blink"
    } else if ua.contains("safari") && !ua.contains("chrome") && !ua.contains("chromium") {
        "webkit"
    } else if ua.contains("chrome") || ua.contains("chromium") || ua.contains("crios") {
        "blink"
    } else if !ua.is_empty() {
        // unknown shell — do not invent
        "unknown"
    } else {
        "unknown"
    };

    // Observations (capability / protocol), not brand names
    let mut obs_votes: Vec<&str> = Vec::new();
    if fields
        .get("chrome_runtime")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        obs_votes.push("blink");
    }
    if let Some(ee) = fields.get("errors_engine").and_then(|v| v.as_str()) {
        let e = ee.to_ascii_lowercase();
        if e.contains("chrome") || e.contains("blink") {
            obs_votes.push("blink");
        } else if e.contains("firefox") || e.contains("gecko") {
            obs_votes.push("gecko");
        } else if e.contains("safari") || e.contains("webkit") {
            obs_votes.push("webkit");
        }
    }
    if let Some(pe) = fields
        .get("protocol_engine")
        .and_then(|v| v.as_str())
    {
        obs_votes.push(brand_to_engine_family(pe));
    }
    // CSS/API-ish: InstallTrigger historically gecko; document.documentMode IE(blink legacy skip)
    if fields
        .get("install_trigger_present")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        obs_votes.push("gecko");
    }
    if fields
        .get("safari_push_notification")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        obs_votes.push("webkit");
    }

    let obs = if obs_votes.iter().any(|o| *o == "gecko")
        && !obs_votes.iter().any(|o| *o == "blink" || *o == "webkit")
    {
        "gecko"
    } else if obs_votes.iter().any(|o| *o == "webkit")
        && !obs_votes.iter().any(|o| *o == "blink" || *o == "gecko")
    {
        "webkit"
    } else if obs_votes.iter().any(|o| *o == "blink") {
        "blink"
    } else if obs_votes.iter().any(|o| *o != "unknown") {
        // mixed → unknown (do not force)
        "unknown"
    } else {
        "unknown"
    };
    (claim, obs)
}

/// Classify TLS/JA4 string into coarse browser engine family for claim-obs.
pub fn classify_protocol_engine(ja4: &str, h2: &str, alpn: &str) -> &'static str {
    let j = ja4.to_ascii_lowercase();
    let h = h2.to_ascii_lowercase();
    let a = alpn.to_ascii_lowercase();
    if j.contains("firefox")
        || j.starts_with("ff_")
        || j.contains("|ff|")
        || h.contains("firefox")
        || j.contains("nss")
    {
        return "firefox";
    }
    if j.contains("safari") || j.contains("apple") || h.contains("safari") {
        return "safari";
    }
    if j.contains("edge") || j.starts_with("ed_") {
        return "edge";
    }
    if j.contains("chrome")
        || j.starts_with("cr_")
        || j.contains("|cr|")
        || j.contains("boringssl")
        || h.contains("chrome")
        || a.contains("h2") && j.contains("t13d")
    {
        // t13d* is common Chrome JA4 shape; not exclusive but useful when labeled chrome
        if j.contains("chrome") || j.starts_with("cr_") || h.contains("chrome") {
            return "chrome";
        }
    }
    // FoxIO-style JA4: t13d####h#_* — cipher/ext density often distinguishes stacks.
    // Lab Pingora sees bare t13d… without brand tag; use coarse density heuristics
    // (not a hard chrome claim — stored as unknown_tls13 + optional soft hint).
    if !j.is_empty()
        && (j.starts_with("t13d")
            || j.starts_with("t13i")
            || j.starts_with("t12d")
            || j.starts_with("t12i"))
    {
        // h1 vs h2 in JA4_a often correlates with ALPN preference
        if j.contains("h2_") || j.contains("h2,") || a.contains("h2") {
            // Still untagged engine — keep unknown_tls13 for strict compare
            return "unknown_tls13";
        }
        return "unknown_tls13";
    }
    if !j.is_empty() || !h.is_empty() {
        return "unknown";
    }
    "absent"
}

/// Merge trusted protocol fingerprints into a gateway fields object.
/// `headers` is a map of lowercased header name → value.
pub fn inject_protocol_from_headers(
    gateway_fields: &mut Map<String, Value>,
    headers: &Map<String, Value>,
) {
    let pick = |names: &[&str]| -> Option<String> {
        for n in names {
            if let Some(v) = headers.get(*n).and_then(|x| x.as_str()) {
                let t = v.trim();
                if !t.is_empty() {
                    return Some(t.to_string());
                }
            }
        }
        None
    };
    let ja4 = pick(TRUSTED_JA4_HEADERS);
    let ja3 = pick(TRUSTED_JA3_HEADERS);
    let h2 = pick(TRUSTED_H2_HEADERS);
    let alpn = pick(TRUSTED_ALPN_HEADERS);
    let tls_ver = pick(TRUSTED_TLS_VERSION_HEADERS);

    if let Some(ref j) = ja4 {
        gateway_fields.insert("ja4".into(), json!(j));
        gateway_fields.insert("tls_ja4".into(), json!(j));
        // Prefer explicit native source from gr-tls-edge; else trusted reverse-proxy header.
        let src = headers
            .get("x-gr-tls-ja4-source")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("edge_header");
        gateway_fields.insert("protocol_fp_source".into(), json!(src));
        gateway_fields.insert("tls_fingerprint_available".into(), json!(true));
    } else {
        gateway_fields
            .entry("tls_fingerprint_available")
            .or_insert(json!(false));
    }
    if let Some(ref j) = ja3 {
        gateway_fields.insert("ja3".into(), json!(j));
        gateway_fields.insert("tls_ja3".into(), json!(j));
    }
    if let Some(ref h) = h2 {
        gateway_fields.insert("h2_fingerprint".into(), json!(h));
        gateway_fields.insert("http2_fingerprint".into(), json!(h));
    }
    if let Some(ref a) = alpn {
        gateway_fields.insert("tls_alpn".into(), json!(a));
    }
    if let Some(ref v) = tls_ver {
        gateway_fields.insert("tls_version".into(), json!(v));
    }
    // S0 depth: JA4_r + ClientHello order digests (iss/18 P0 / F-7)
    if let Some(r) = pick(&["x-tls-ja4-r", "x-ja4-r"]) {
        gateway_fields.insert("ja4_r".into(), json!(r));
    }
    if let Some(o) = pick(&["x-tls-ext-order", "x-tls-extensions-order"]) {
        gateway_fields.insert("tls_extensions_order".into(), json!(o));
    }
    if let Some(o) = pick(&["x-tls-cipher-order", "x-tls-ciphers-order"]) {
        gateway_fields.insert("cipher_suites_order".into(), json!(o));
    }

    let eng = classify_protocol_engine(
        ja4.as_deref().unwrap_or(""),
        h2.as_deref().unwrap_or(""),
        alpn.as_deref().unwrap_or(""),
    );
    if eng != "absent" {
        gateway_fields.insert("protocol_engine".into(), json!(eng));
    }
    annotate_protocol_export_honesty(gateway_fields);
}

/// Keys that may be hard-coded / partial and must never feed commercial mint
/// or be treated as hard browser uniqueness (iss/46 H2, iss/50 H1/H2).
pub const PROTOCOL_DIAGNOSTIC_ONLY_KEYS: &[&str] = &[
    "h2_priority_fingerprint",
    "h2_pseudo_order",
    "h3_pseudo_order",
    "h3_settings_fp",
    "h2_fingerprint_partial_v1",
];

/// Depth keys that are real when captured (export honesty for H1 wiring).
pub const PROTOCOL_DEPTH_EXPORT_KEYS: &[&str] = &[
    "ja4",
    "tls_ja4",
    "ja3",
    "ja4_r",
    "ja4t",
    "tcp_syn_ja4t",
    "tcp_syn_p0f_sig",
    "tcp_syn_p0f_hash",
    "quic_tp_summary",
    "quic_tp_map",
    "quic_key_share_groups",
    "h2_fingerprint",
    "h2_connection_window_update",
    "tls_extensions_order",
    "cipher_suites_order",
];

/// Mark synthetic / partial protocol fields as diagnostic-only; list real depth exports.
/// Never allows these materials into commercial device body (consumers must check).
pub fn annotate_protocol_export_honesty(fields: &mut Map<String, Value>) {
    let mut diag_present = Vec::new();
    let mut depth_present = Vec::new();
    for k in PROTOCOL_DIAGNOSTIC_ONLY_KEYS {
        if field_nonempty(fields, k) {
            diag_present.push((*k).to_string());
            // Per-key flag for safe consumers
            fields.insert(format!("{k}__diagnostic_only"), json!(true));
            fields.insert(format!("{k}__commercial_mint"), json!(false));
            fields.insert(format!("{k}__hard_browser_uniqueness"), json!(false));
        }
    }
    for k in PROTOCOL_DEPTH_EXPORT_KEYS {
        if field_nonempty(fields, k) {
            depth_present.push((*k).to_string());
        }
    }
    // H3 pseudo often padded with synthetic :method/:path — always diagnostic-only when present
    if field_nonempty(fields, "h3_pseudo_order") {
        fields.insert("h3_pseudo_order_role".into(), json!("diagnostic_partial"));
    }
    if field_nonempty(fields, "h2_priority_fingerprint") {
        let p = fields
            .get("h2_priority_fingerprint")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        // "0" or empty priority is synthetic placeholder from capture when real tree absent
        if p.is_empty() || p == "0" {
            fields.insert("h2_priority_fingerprint_role".into(), json!("synthetic_placeholder"));
        } else {
            fields.insert("h2_priority_fingerprint_role".into(), json!("captured_or_partial"));
        }
    }
    fields.insert(
        "protocol_export_honesty".into(),
        json!({
            "algo": "protocol_export_honesty_v1",
            "depth_exported": depth_present,
            "diagnostic_only": diag_present,
            "commercial_mint_from_protocol_diagnostic": false,
            "hard_browser_uniqueness_from_h2h3_pseudo_priority": false,
            "note": "H2 PRIORITY / H3 pseudo may be partial or padded; never mint or hard-uniqueness",
        }),
    );
    if !diag_present.is_empty() {
        fields.insert("protocol_has_diagnostic_only".into(), json!(true));
    }
    if !depth_present.is_empty() {
        fields.insert("protocol_depth_export_n".into(), json!(depth_present.len()));
    }
}

fn field_nonempty(fields: &Map<String, Value>, key: &str) -> bool {
    match fields.get(key) {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(_) => true,
    }
}

/// Helper: build header map from list of (name, value) pairs (HTTP headers).
pub fn headers_map_from_pairs(pairs: &[(String, String)]) -> Map<String, Value> {
    let mut m = Map::new();
    for (k, v) in pairs {
        m.insert(k.to_ascii_lowercase(), json!(v));
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injects_ja4_and_classifies_firefox() {
        let mut gw = Map::new();
        let headers = headers_map_from_pairs(&[
            ("X-TLS-JA4".into(), "ff_t13d1516h2_abcdef".into()),
            ("X-HTTP2-Fingerprint".into(), "firefox-h2".into()),
        ]);
        inject_protocol_from_headers(&mut gw, &headers);
        assert_eq!(gw.get("ja4").and_then(|v| v.as_str()).unwrap(), "ff_t13d1516h2_abcdef");
        assert_eq!(gw.get("protocol_engine").and_then(|v| v.as_str()).unwrap(), "firefox");
        assert_eq!(gw.get("protocol_fp_source").and_then(|v| v.as_str()).unwrap(), "edge_header");
    }

    #[test]
    fn chrome_vs_firefox_engine_differs() {
        assert_eq!(
            classify_protocol_engine("cr_t13d1516h2_xxx", "", ""),
            "chrome"
        );
        assert_eq!(
            classify_protocol_engine("ff_t13d1516h2_xxx", "", ""),
            "firefox"
        );
    }

    #[test]
    fn protocol_diagnostic_markers_never_mint() {
        let mut gw = Map::new();
        gw.insert("h2_priority_fingerprint".into(), json!("0"));
        gw.insert("h3_pseudo_order".into(), json!(":method,:path"));
        gw.insert("ja4".into(), json!("t13d1516h2_real"));
        gw.insert("quic_tp_summary".into(), json!("tp=1"));
        annotate_protocol_export_honesty(&mut gw);
        let honesty = gw.get("protocol_export_honesty").unwrap();
        assert_eq!(
            honesty
                .get("commercial_mint_from_protocol_diagnostic")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            honesty
                .get("hard_browser_uniqueness_from_h2h3_pseudo_priority")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            gw.get("h2_priority_fingerprint__diagnostic_only")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        let depth = honesty
            .get("depth_exported")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(depth.iter().any(|v| v.as_str() == Some("ja4")));
        assert!(depth.iter().any(|v| v.as_str() == Some("quic_tp_summary")));
    }
}
