//! Pull client H2 SETTINGS (from patched Pingora settings_capture) → akamai_h2 fingerprint.

use gr_probe_net::fingerprint_h2_settings_ex;
use log::debug;
use pingora_proxy::Session;
use serde_json::{json, Map, Value};

/// Build connection key matching settings_capture (sockcookie or peer-local tuple).
pub fn connection_key_from_session(session: &Session) -> Option<String> {
    let dig = session.digest()?;
    let sock = dig.socket_digest.as_ref()?;
    #[cfg(unix)]
    {
        if let Ok(c) = sock.socket_cookie() {
            if c != 0 {
                return Some(format!("sockcookie:{c:x}"));
            }
        }
    }
    let peer = sock.peer_addr()?.as_inet()?;
    let local = sock.local_addr()?.as_inet()?;
    Some(format!(
        "tuple:{}:{}-{}:{}",
        peer.ip(),
        peer.port(),
        local.ip(),
        local.port()
    ))
}

/// Inject H2 SETTINGS / Akamai fingerprint into fields (gateway evidence).
/// Alternate to header inject — kept for callers that already hold a fields map.
#[allow(dead_code)]
pub fn inject_h2_into_fields(fields: &mut Map<String, Value>, session: &Session) {
    let Some(fp) = capture_h2(session) else {
        return;
    };
    apply_h2_to_fields(fields, &fp);
}

/// Also put H2 fingerprint into trusted headers for inject_protocol_from_headers.
pub fn inject_h2_into_headers(
    headers: &mut std::collections::HashMap<String, String>,
    session: &Session,
) {
    let Some(fp) = capture_h2(session) else {
        return;
    };
    headers.insert("x-h2-fingerprint".into(), fp.fingerprint.clone());
    headers.insert("x-http2-fingerprint".into(), fp.fingerprint.clone());
    headers.insert("x-gr-h2-hash".into(), fp.fingerprint_hash.clone());
    headers.insert("x-gr-h2-capture-version".into(), "v2_priority_real".into());
    if fp.preface_ok {
        headers.insert("x-gr-h2-preface-ok".into(), "1".into());
    }
    if let Some(wu) = fp.window_update {
        headers.insert("x-gr-h2-window-update".into(), wu.to_string());
    }
    if !fp.priority_fingerprint.is_empty() {
        headers.insert(
            "x-gr-h2-priority-fp".into(),
            fp.priority_fingerprint.clone(),
        );
    }
    if !fp.frame_type_sequence.is_empty() {
        let seq = fp
            .frame_type_sequence
            .iter()
            .map(|b| b.to_string())
            .collect::<Vec<_>>()
            .join(",");
        headers.insert("x-gr-h2-frame-seq".into(), seq);
    }
}

struct H2Cap {
    fingerprint: String,
    fingerprint_hash: String,
    settings_part: String,
    preface_ok: bool,
    window_update: Option<u32>,
    raw_pairs: Vec<(u16, u32)>,
    frame_type_sequence: Vec<u8>,
    priority_fingerprint: String,
    capture_version: String,
}

fn capture_h2(session: &Session) -> Option<H2Cap> {
    let key = connection_key_from_session(session)?;
    let s = pingora_core::protocols::http::v2::get_client_h2_settings(&key)?;
    // Use real PRIORITY fingerprint when capture provides it (iss/45 B2).
    // Empty → synthetic "0" placeholder (iss/46: diagnostic-only, never hard uniqueness).
    let prio = if s.priority_fingerprint.is_empty() {
        "0"
    } else {
        s.priority_fingerprint.as_str()
    };
    // iss/46: HPACK pseudo wire-order not exposed by Pingora app layer.
    // Prefer any edge-injected order; else best-effort from captured frame metadata;
    // constant "m,a,s,p" remains diagnostic-only.
    let pseudo = gr_abi::env::get("H2_PSEUDO_ORDER_OVERRIDE")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "m,a,s,p".into());
    let fp = fingerprint_h2_settings_ex(
        &s.raw_pairs,
        s.connection_window_update,
        prio,
        &pseudo,
    );
    debug!(
        "h2 fingerprint key={key} prio={prio} fp={}",
        fp.fingerprint.chars().take(80).collect::<String>()
    );
    Some(H2Cap {
        fingerprint: fp.fingerprint,
        fingerprint_hash: fp.fingerprint_hash,
        settings_part: fp.settings_part,
        preface_ok: s.preface_ok,
        window_update: s.connection_window_update,
        raw_pairs: s.raw_pairs,
        frame_type_sequence: s.frame_type_sequence,
        priority_fingerprint: s.priority_fingerprint,
        capture_version: "h2_partial_v2_real_priority_synth_pseudo".into(),
    })
}

fn apply_h2_to_fields(fields: &mut Map<String, Value>, fp: &H2Cap) {
    fields.insert("h2_settings_available".into(), json!(true));
    fields.insert("h2_preface_ok".into(), json!(fp.preface_ok));
    fields.insert("h2_settings_raw".into(), json!(fp.raw_pairs));
    fields.insert("h2_capture_version".into(), json!(fp.capture_version.clone()));
    // Partial fingerprint: settings+window+priority real; pseudo synthetic unless override.
    fields.insert("h2_fingerprint_partial_v1".into(), json!(fp.fingerprint.clone()));
    let pseudo = gr_abi::env::get("H2_PSEUDO_ORDER_OVERRIDE")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "m,a,s,p".into());
    let synth = pseudo == "m,a,s,p";
    fields.insert("h2_pseudo_order".into(), json!(pseudo));
    fields.insert(
        "h2_pseudo_order_role".into(),
        json!(if synth {
            "synthetic_hardcoded"
        } else {
            "edge_or_lab_override"
        }),
    );
    fields.insert("h2_pseudo_order__diagnostic_only".into(), json!(true));
    fields.insert("h2_pseudo_order__commercial_mint".into(), json!(false));
    fields.insert("h2_pseudo_order__hard_browser_uniqueness".into(), json!(false));
    fields.insert("h2_pseudo_order__hpack_wire".into(), json!(!synth));
    fields.insert("h2_fingerprint_partial_v1__diagnostic_only".into(), json!(true));
    if let Some(wu) = fp.window_update {
        fields.insert("h2_connection_window_update".into(), json!(wu));
    }
    if !fp.frame_type_sequence.is_empty() {
        fields.insert(
            "h2_frame_type_sequence".into(),
            json!(fp.frame_type_sequence.clone()),
        );
    }
    if !fp.priority_fingerprint.is_empty() {
        fields.insert(
            "h2_priority_fingerprint".into(),
            json!(fp.priority_fingerprint.clone()),
        );
    }
    fields.insert("h2_fingerprint".into(), json!(fp.fingerprint.clone()));
    fields.insert("http2_fingerprint".into(), json!(fp.fingerprint.clone()));
    fields.insert("h2_fingerprint_hash".into(), json!(fp.fingerprint_hash.clone()));
    fields.insert("h2_settings_part".into(), json!(fp.settings_part.clone()));
    if !fields.contains_key("protocol_fp_source") {
        fields.insert("protocol_fp_source".into(), json!("gr_h2_capture"));
    }
}

/// Inject PROXY real client into headers for gateway inject path.
pub fn inject_proxy_into_headers(
    headers: &mut std::collections::HashMap<String, String>,
    session: &Session,
) {
    let key = connection_key_from_session(session);
    let info = key
        .as_deref()
        .and_then(crate::listen::proxy_proto::get_proxy_info)
        .or_else(|| {
            // fallback by peer IP
            session
                .client_addr()
                .and_then(|a| a.as_inet())
                .map(|a| a.ip().to_string())
                .and_then(|ip| crate::listen::proxy_proto::latest_proxy_for_ip(&ip))
        });
    let Some(info) = info else {
        return;
    };
    if !info.parse_ok {
        return;
    }
    headers.insert("x-gr-proxy-protocol".into(), "1".into());
    headers.insert("x-gr-proxy-version".into(), info.version.to_string());
    if let Some(ref ip) = info.src_ip {
        headers.insert("x-real-ip".into(), ip.clone());
        headers.insert("x-forwarded-for".into(), ip.clone());
        headers.insert("x-gr-proxy-src-ip".into(), ip.clone());
    }
    if let Some(p) = info.src_port {
        headers.insert("x-gr-proxy-src-port".into(), p.to_string());
    }
    if let Some(ref dip) = info.dst_ip {
        headers.insert("x-gr-proxy-dst-ip".into(), dip.clone());
    }
}
