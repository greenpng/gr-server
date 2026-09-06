//! Akamai-style HTTP/2 fingerprint (standalone, no probe event types).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct H2Fingerprint {
    pub fingerprint: String,
    pub fingerprint_hash: String,
    pub settings_part: String,
    pub window_update_part: String,
    pub priority_part: String,
    pub pseudo_order_part: String,
}

/// Build fingerprint from SETTINGS (id,value) wire order + optional WINDOW_UPDATE +
/// **real** priority tree fingerprint + pseudo header order.
///
/// `priority_part` must come from capture (`priority_fingerprint`); pass `"0"` only when
/// capture truly saw no PRIORITY frames — never hard-code when real data exists.
pub fn fingerprint_h2_settings(
    settings: &[(u16, u32)],
    window_update: Option<u32>,
    pseudo_order: &str,
) -> H2Fingerprint {
    fingerprint_h2_settings_ex(settings, window_update, "0", pseudo_order)
}

/// Extended builder with explicit priority segment (iss/45 B2 / iss/46).
pub fn fingerprint_h2_settings_ex(
    settings: &[(u16, u32)],
    window_update: Option<u32>,
    priority_part: &str,
    pseudo_order: &str,
) -> H2Fingerprint {
    let settings_part = settings
        .iter()
        .map(|(id, v)| format!("{id}:{v}"))
        .collect::<Vec<_>>()
        .join(";");
    let window_update_part = window_update
        .map(|w| w.to_string())
        .unwrap_or_else(|| "0".into());
    let priority_part = if priority_part.is_empty() {
        "0".to_string()
    } else {
        priority_part.to_string()
    };
    let pseudo_order_part = if pseudo_order.is_empty() {
        // Partial: true order requires HPACK-layer capture; mark as partial default.
        "m,a,s,p".into()
    } else {
        pseudo_order.to_string()
    };
    let fingerprint = format!(
        "{settings_part}|{window_update_part}|{priority_part}|{pseudo_order_part}"
    );
    let mut h = Sha256::new();
    h.update(fingerprint.as_bytes());
    let fingerprint_hash = hex::encode(h.finalize());
    H2Fingerprint {
        fingerprint,
        fingerprint_hash: fingerprint_hash[..16.min(fingerprint_hash.len())].to_string(),
        settings_part,
        window_update_part,
        priority_part,
        pseudo_order_part,
    }
}
