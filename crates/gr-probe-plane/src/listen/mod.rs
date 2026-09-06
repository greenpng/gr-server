//! Side-channel listeners (v4 probe lineage) for green-v5 Pingora edge.
//!
//! - PROXY protocol PreTls (real client IP before TLS)
//! - HTTP/2 SETTINGS capture (patched Pingora) → Akamai-H2 fingerprint
//! - QUIC/HTTP3 Initial (UDP) → JA4 over H3 when AEAD decrypt works
//! - WebRTC/STUN/DTLS (UDP)
//! - TCP SYN p0f-style (AF_PACKET, optional CAP_NET_RAW)

pub mod h2_capture;
pub mod h3_server;
pub mod proxy_proto;
pub mod quic;
pub mod side_store;
pub mod syn;
pub mod tcp_depth;
pub mod webrtc;

pub use h2_capture::{inject_h2_into_headers, inject_proxy_into_headers};
pub use h3_server::spawn_h3_server;
pub use proxy_proto::new_callback as new_proxy_protocol_callback;
pub use quic::spawn_quic_listener;
pub use side_store::{
    enrich_protocol_fields_scoped, lab_inject, recent_side_summary, side_lab_enabled, JoinScope,
};
pub use syn::spawn_syn_listener;
pub use webrtc::spawn_webrtc_listener;
