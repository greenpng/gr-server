//! WebRTC / STUN / DTLS side-channel info (scheme C · R4).

use crate::stun::StunInfo;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WebrtcSideInfo {
    /// true when a side-channel listener has attached a sample
    pub present: bool,
    pub kind: Option<String>,
    pub src_ip: Option<String>,
    pub src_port: Option<u16>,
    pub dst_ip: Option<String>,
    pub dst_port: Option<u16>,
    /// host / srflx / relay candidate type hints (when parsed)
    pub ice_candidate_types: Vec<String>,
    pub has_host: Option<bool>,
    pub has_srflx: Option<bool>,
    pub has_relay: Option<bool>,
    pub dtls_ja3: Option<String>,
    pub dtls_ja4: Option<String>,
    pub stun: Option<StunInfo>,
    pub is_stun: bool,
    pub is_dtls_client_hello: bool,
    pub ice_binding: bool,
    pub note_zh: String,
}

impl WebrtcSideInfo {
    pub fn absent() -> Self {
        Self {
            present: false,
            note_zh: "WebRTC/STUN 侧信道未观测到（可 POST 挑战后打 STUN 到探针）".into(),
            ..Default::default()
        }
    }

    pub fn from_stun(stun: StunInfo, src: &str, sport: u16) -> Self {
        let ice = stun.has_username
            || stun.has_priority
            || stun.has_ice_controlling
            || stun.has_ice_controlled
            || stun.has_use_candidate;
        let mut types = Vec::new();
        // Binding from client → we learn host path; XOR-MAPPED on response would be srflx
        types.push("host".into());
        if ice {
            types.push("ice".into());
        }
        Self {
            present: true,
            kind: Some(if ice { "stun_ice".into() } else { "stun".into() }),
            src_ip: Some(src.into()),
            src_port: Some(sport),
            ice_candidate_types: types,
            has_host: Some(true),
            has_srflx: None,
            has_relay: Some(false),
            is_stun: true,
            is_dtls_client_hello: false,
            ice_binding: ice,
            stun: Some(stun),
            note_zh: if ice {
                "观测到 ICE/STUN Binding（WebRTC R4）".into()
            } else {
                "观测到 STUN Binding（WebRTC R4）".into()
            },
            ..Default::default()
        }
    }

    pub fn from_dtls_hello(src: &str, sport: u16) -> Self {
        Self::from_dtls_hello_fp(src, sport, None, None)
    }

    pub fn from_dtls_hello_fp(
        src: &str,
        sport: u16,
        fp: Option<String>,
        fp_hash: Option<String>,
    ) -> Self {
        Self {
            present: true,
            kind: Some("dtls_client_hello".into()),
            src_ip: Some(src.into()),
            src_port: Some(sport),
            is_stun: false,
            is_dtls_client_hello: true,
            ice_candidate_types: vec!["dtls".into()],
            dtls_ja4: fp_hash.clone().or(fp.clone()),
            dtls_ja3: fp,
            note_zh: if fp_hash.is_some() {
                "观测到 DTLS ClientHello（已粗指纹）".into()
            } else {
                "观测到 DTLS ClientHello（WebRTC R4）".into()
            },
            ..Default::default()
        }
    }
}
