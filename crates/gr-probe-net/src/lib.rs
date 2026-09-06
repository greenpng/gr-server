//! Network protocol fingerprinting for green-v5 Pingora edge (v4 probe lineage).
//!
//! Self-contained: no path deps on greenv4/probe.

pub mod akamai_h2;
pub mod proxy_protocol;
pub mod quic_decrypt;
pub mod quic_fingerprint;
pub mod stun;
pub mod tcp_syn;
pub mod tls_ext;
pub mod tls_fingerprint;
pub mod webrtc;

pub use akamai_h2::{fingerprint_h2_settings, fingerprint_h2_settings_ex, H2Fingerprint};
pub use proxy_protocol::{try_parse as try_parse_proxy, ProxyProtocolInfo};
pub use quic_fingerprint::{parse_quic_datagram, QuicInitialInfo};
pub use stun::{
    build_binding_success, fingerprint_dtls_client_hello, looks_like_dtls_client_hello,
    looks_like_stun, parse_stun, StunInfo,
};
pub use tcp_syn::{parse_tcp_syn_packet, TcpSynInfo};
pub use tls_ext::parse_client_hello_ext_detail;
pub use tls_fingerprint::{compute_fingerprints, ClientHelloFingerprint, ClientHelloParts};
pub use webrtc::WebrtcSideInfo;
