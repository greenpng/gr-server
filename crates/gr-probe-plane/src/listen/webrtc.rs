//! STUN / DTLS side-channel helpers + dedicated UDP listener.

use crate::listen::side_store::push_webrtc;
use gr_probe_net::{
    build_binding_success, fingerprint_dtls_client_hello, looks_like_dtls_client_hello,
    looks_like_stun, parse_stun, WebrtcSideInfo,
};
use log::{info, warn};
use std::net::SocketAddr;
use tokio::net::UdpSocket;

pub fn ingest_udp_datagram(
    data: &[u8],
    peer: SocketAddr,
    local: Option<SocketAddr>,
) -> Option<(WebrtcSideInfo, Option<Vec<u8>>)> {
    if looks_like_stun(data) {
        let stun = parse_stun(data);
        if !stun.parse_ok {
            return None;
        }
        let mut info = WebrtcSideInfo::from_stun(stun, &peer.ip().to_string(), peer.port());
        if let Some(l) = local {
            info.dst_ip = Some(l.ip().to_string());
            info.dst_port = Some(l.port());
        }
        let reply = if info.stun.as_ref().map(|s| s.is_binding_request).unwrap_or(false) {
            build_binding_success(data, peer)
        } else {
            None
        };
        return Some((info, reply));
    }
    if looks_like_dtls_client_hello(data) {
        let (fp, hash) = fingerprint_dtls_client_hello(data)
            .map(|(a, b)| (Some(a), Some(b)))
            .unwrap_or((None, None));
        let mut info =
            WebrtcSideInfo::from_dtls_hello_fp(&peer.ip().to_string(), peer.port(), fp, hash);
        if let Some(l) = local {
            info.dst_ip = Some(l.ip().to_string());
            info.dst_port = Some(l.port());
        }
        return Some((info, None));
    }
    None
}

pub fn spawn_webrtc_listener(addr: SocketAddr) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let sock = match UdpSocket::bind(addr).await {
            Ok(s) => s,
            Err(e) => {
                warn!("WebRTC/STUN UDP bind {addr} failed: {e}");
                return;
            }
        };
        info!("WebRTC/STUN listener on udp://{addr}");
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match sock.recv_from(&mut buf).await {
                Ok((n, peer)) => {
                    let local = sock.local_addr().ok();
                    if let Some((info, reply)) = ingest_udp_datagram(&buf[..n], peer, local) {
                        push_webrtc(info);
                        if let Some(r) = reply {
                            let _ = sock.send_to(&r, peer).await;
                        }
                    }
                }
                Err(e) => {
                    warn!("STUN recv error: {e}");
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            }
        }
    })
}
