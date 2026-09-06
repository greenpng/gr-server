//! Site model with multi-root domains and per-site crypto isolation.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CryptoProfile {
    /// Per-site challenge HMAC material (or key id referencing sealed store).
    #[serde(default)]
    pub challenge_secret: String,
    #[serde(default)]
    pub seal_secret: String,
    /// Algorithm suite id (strengthened matrix vs v5 defaults).
    #[serde(default = "default_suite")]
    pub suite: String,
    /// Extra matrix knobs (salt rotation days, seal version, etc.)
    #[serde(default)]
    pub params: serde_json::Value,
}

fn default_suite() -> String {
    "gr-seal-v2".into()
}

impl Default for CryptoProfile {
    fn default() -> Self {
        Self {
            challenge_secret: String::new(),
            seal_secret: String::new(),
            suite: default_suite(),
            params: serde_json::json!({
                "seal_version": 2,
                "require_sealed_ingest": true,
                "challenge_rotate_hours": 24
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteRecord {
    pub site_id: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub collect_enabled: bool,
    #[serde(default)]
    pub notes: String,
    /// first_party | hybrid | dual_domain
    #[serde(default = "default_edge")]
    pub edge_mode: String,
    #[serde(default)]
    pub pv_base: String,
    #[serde(default)]
    pub gv_base: String,
    /// Wire: `pv` | `first_party` | `gv`. New sites default `pv`.
    #[serde(default = "default_fe_load")]
    pub fe_load: String,
    /// Wire: `gv` | `pv` | `first_party`. New sites default `gv`.
    /// `first_party` means proxied upload and is rejected unless
    /// `GR_ALLOW_FIRST_PARTY_UPLOAD=1`.
    #[serde(default = "default_upload")]
    pub upload_ingest: String,
    #[serde(default = "default_poll")]
    pub poll_method: String,
    /// Fixed entry filename; routing still config-driven.
    #[serde(default = "default_entry")]
    pub entry_js: String,
    #[serde(default)]
    pub root_domains: Vec<String>,
    #[serde(default)]
    pub crypto: CryptoProfile,
    /// Cookie allowlist for business-identifier binding. The probe script
    /// reads these names from `document.cookie` (pv context or same-origin
    /// script proxy) and POSTs them in the open/ingest body. The server
    /// allowlists + truncates and echoes them in `sdk_projection.cookie_fields`.
    /// Empty = no cookie capture. Names: [a-zA-Z0-9_-.], ≤16 entries, values
    /// truncated server-side to 256 chars.
    #[serde(default)]
    pub cookie_fields: Vec<String>,
    /// Site embed token (`grst_…`). Minted on create; carried as `?grt=` on
    /// `/gr.js`. Soft protection (visible in HTML). Empty = legacy, no gate.
    #[serde(default)]
    pub embed_token: String,
    /// iss/opus5 02-§5 consent gate: the site owner must confirm the probe
    /// behavior has been disclosed to visitors BEFORE a site can be created.
    /// Required on create; preserved (ignored) on update.
    #[serde(default)]
    pub consent_confirmed: bool,
    /// Notice version the owner confirmed (bumped when the contract changes).
    #[serde(default = "default_notice")]
    pub consent_notice_version: String,
}

fn default_notice() -> String {
    "v1".into()
}

fn default_true() -> bool {
    true
}
fn default_edge() -> String {
    "dual_domain".into()
}
fn default_fe_load() -> String {
    "pv".into()
}
fn default_upload() -> String {
    "gv".into()
}
fn default_poll() -> String {
    "both".into()
}
fn default_entry() -> String {
    "gr.js".into()
}

/// Panel / API labels → wire `fe_load`.
pub fn normalize_fe_load(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "pv" | "probe" | "cdn" | "cross_origin" | "dual" | "dual_domain"
        | "third_party" | "third-party" | "3p" => "pv".into(),
        "gv" | "unified" => "gv".into(),
        "hybrid" | "first_party" | "first-party" | "fp" | "same_origin" | "nginx" | "" => {
            "first_party".into()
        }
        _ => "first_party".into(),
    }
}

/// Panel / API labels → wire `upload_ingest`. Empty → `gv`.
pub fn normalize_upload_ingest(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "pv" | "probe" | "cdn" | "third_party" | "third-party" => "pv".into(),
        "first_party" | "first-party" | "fp" | "g5" | "same_origin" | "hybrid" => {
            "first_party".into()
        }
        "dual_domain" | "dual" | "gv" | "unified" | "unified_gv" | "gateway" | "" => "gv".into(),
        _ => "gv".into(),
    }
}

/// Proxied upload (`upload_ingest=first_party`) is forbidden unless the
/// operator explicitly opts in. Error code is stable for the panel.
pub fn reject_proxied_upload(upload_ingest: &str) -> Option<&'static str> {
    if normalize_upload_ingest(upload_ingest) != "first_party" {
        return None;
    }
    if gr_abi::env::flag("ALLOW_FIRST_PARTY_UPLOAD") {
        return None;
    }
    Some("upload_ingest_first_party_forbidden")
}

/// `grst_` + 32 random bytes, base64url without padding.
pub fn mint_embed_token() -> String {
    use rand::RngCore;
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    let mut out = String::from("grst_");
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut acc = 0u32;
    let mut bits = 0u32;
    for byte in b {
        acc = (acc << 8) | u32::from(byte);
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            out.push(T[((acc >> bits) & 0x3f) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(T[((acc << (6 - bits)) & 0x3f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_site_defaults_are_pv_gv() {
        let rec: SiteRecord = serde_json::from_value(serde_json::json!({
            "site_id": "s1",
            "name": "n",
        }))
        .unwrap();
        assert_eq!(rec.fe_load, "pv");
        assert_eq!(rec.upload_ingest, "gv");
        assert_eq!(rec.edge_mode, "dual_domain");
        assert_eq!(rec.entry_js, "gr.js");
        assert!(rec.embed_token.is_empty());
    }

    #[test]
    fn mint_embed_token_shape() {
        let a = mint_embed_token();
        let b = mint_embed_token();
        assert!(a.starts_with("grst_"), "{a}");
        assert!(a.len() > 20);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
    }

    #[test]
    fn dual_domain_upload_maps_to_gv() {
        assert_eq!(normalize_upload_ingest("dual_domain"), "gv");
        assert_eq!(normalize_upload_ingest(""), "gv");
        assert_eq!(normalize_fe_load("dual_domain"), "pv");
        assert_eq!(normalize_fe_load("first_party"), "first_party");
    }

    #[test]
    fn proxied_upload_rejected_without_override() {
        assert_eq!(reject_proxied_upload("gv"), None);
        assert_eq!(reject_proxied_upload("dual_domain"), None);
        assert_eq!(reject_proxied_upload("pv"), None);
        if !gr_abi::env::flag("ALLOW_FIRST_PARTY_UPLOAD") {
            assert_eq!(
                reject_proxied_upload("first_party"),
                Some("upload_ingest_first_party_forbidden")
            );
        }
    }
}
