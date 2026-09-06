//! Connection-scoped TLS ClientHello fingerprints (JA3/JA4).
//!
//! Fixes thread_local race (P-A4): ClientHello callback stores by SSL pointer;
//! `TlsAccept::handshake_complete_callback` moves into `SslDigest.extension`
//! (same pattern as v4 probe). Request path reads from session digest.

use async_trait::async_trait;
use dashmap::DashMap;
use log::debug;
use pingora::listeners::TlsAccept;
use pingora_core::protocols::tls::TlsRef;
use pingora_proxy::Session;
use std::any::Any;
use std::sync::{Arc, OnceLock};

#[derive(Clone, Debug, Default)]
pub struct TlsClientFp {
    pub ja4: String,
    pub ja3_hash: String,
    pub alpn: Option<String>,
    /// Unhashed JA4_r (lab / near-neighbor stack discrimination)
    pub ja4_r: String,
    /// ClientHello extension id order digest (comma-separated u16)
    pub tls_extensions_order: String,
    /// ClientHello cipher suite order digest (comma-separated u16, no GREASE)
    pub cipher_suites_order: String,
    /// Depth extensions (iss/45 B3) — key_share / psk / ech / alps / tickets
    pub key_share_groups: String,
    pub psk_modes: String,
    pub has_ech: bool,
    pub has_alps: bool,
    pub has_session_ticket: bool,
    pub has_early_data: bool,
    pub tls_ext_presence: String,
}

fn pending() -> &'static DashMap<usize, TlsClientFp> {
    static M: OnceLock<DashMap<usize, TlsClientFp>> = OnceLock::new();
    M.get_or_init(DashMap::new)
}

pub fn stash_pending(ssl_ptr: usize, fp: TlsClientFp) {
    pending().insert(ssl_ptr, fp);
}

/// Read fingerprints attached to this connection's SslDigest (preferred).
pub fn from_session(session: &Session) -> Option<TlsClientFp> {
    let dig = session.digest()?;
    let ssl = dig.ssl_digest.as_ref()?;
    ssl.extension.get::<TlsClientFp>().cloned()
}

/// Take any leftover pending entry (rare fallback if handshake_complete missed).
#[allow(dead_code)]
pub fn take_pending(ssl_ptr: usize) -> Option<TlsClientFp> {
    pending().remove(&ssl_ptr).map(|(_, v)| v)
}

/// Move pending → SslDigest.extension after handshake (v4 ProbeTlsAccept pattern).
pub struct GrTlsAccept;

#[async_trait]
impl TlsAccept for GrTlsAccept {
    async fn handshake_complete_callback(
        &self,
        ssl: &TlsRef,
    ) -> Option<Arc<dyn Any + Send + Sync>> {
        use foreign_types::ForeignTypeRef;
        let key = ssl.as_ptr() as usize;
        if let Some((_k, fp)) = pending().remove(&key) {
            debug!("tls_fp attached to SslDigest ja4={}", fp.ja4);
            Some(Arc::new(fp) as Arc<dyn Any + Send + Sync>)
        } else {
            None
        }
    }
}

/// Merge JA4/JA3/ALPN into request headers for gateway inject.
pub fn inject_into_headers(
    session: &Session,
    headers: &mut std::collections::HashMap<String, String>,
) {
    let fp = from_session(session);
    let Some(fp) = fp else {
        return;
    };
    headers
        .entry("x-tls-ja4".into())
        .or_insert_with(|| fp.ja4.clone());
    headers
        .entry("x-gr-tls-ja4-source".into())
        .or_insert_with(|| "gr_pingora".into());
    if !fp.ja3_hash.is_empty() {
        headers
            .entry("x-tls-ja3".into())
            .or_insert_with(|| fp.ja3_hash.clone());
    }
    if let Some(ref a) = fp.alpn {
        headers
            .entry("x-tls-alpn".into())
            .or_insert_with(|| a.clone());
    }
    if !fp.ja4_r.is_empty() {
        headers
            .entry("x-tls-ja4-r".into())
            .or_insert_with(|| fp.ja4_r.clone());
    }
    if !fp.tls_extensions_order.is_empty() {
        headers
            .entry("x-tls-ext-order".into())
            .or_insert_with(|| fp.tls_extensions_order.clone());
    }
    if !fp.cipher_suites_order.is_empty() {
        headers
            .entry("x-tls-cipher-order".into())
            .or_insert_with(|| fp.cipher_suites_order.clone());
    }
    if !fp.key_share_groups.is_empty() {
        headers
            .entry("x-tls-key-share-groups".into())
            .or_insert_with(|| fp.key_share_groups.clone());
    }
    if !fp.psk_modes.is_empty() {
        headers
            .entry("x-tls-psk-modes".into())
            .or_insert_with(|| fp.psk_modes.clone());
    }
    if fp.has_ech {
        headers.entry("x-tls-ech".into()).or_insert_with(|| "1".into());
    }
    if fp.has_alps {
        headers.entry("x-tls-alps".into()).or_insert_with(|| "1".into());
    }
    if fp.has_session_ticket {
        headers
            .entry("x-tls-session-ticket".into())
            .or_insert_with(|| "1".into());
    }
    if fp.has_early_data {
        headers
            .entry("x-tls-early-data".into())
            .or_insert_with(|| "1".into());
    }
    if !fp.tls_ext_presence.is_empty() {
        headers
            .entry("x-tls-ext-presence".into())
            .or_insert_with(|| fp.tls_ext_presence.clone());
    }
}
