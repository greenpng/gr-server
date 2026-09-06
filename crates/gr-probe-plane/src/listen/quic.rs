//! UDP QUIC Initial listener (HTTP/3 path).

use crate::listen::side_store::push_quic;
use crate::listen::webrtc::ingest_udp_datagram;
use gr_probe_net::parse_quic_datagram;
use log::{debug, info, warn};
use std::net::SocketAddr;
use tokio::net::UdpSocket;

pub fn spawn_quic_listener(addr: SocketAddr) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let sock = match UdpSocket::bind(addr).await {
            Ok(s) => s,
            Err(e) => {
                warn!("QUIC UDP bind {addr} failed: {e} (HTTP/3 listen disabled)");
                return;
            }
        };
        info!("QUIC/HTTP3 Initial listener on udp://{addr}");
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match sock.recv_from(&mut buf).await {
                Ok((n, peer)) => {
                    let local = sock.local_addr().ok();
                    // Demux STUN/DTLS on same port
                    if let Some((winfo, reply)) = ingest_udp_datagram(&buf[..n], peer, local) {
                        crate::listen::side_store::push_webrtc(winfo);
                        if let Some(r) = reply {
                            let _ = sock.send_to(&r, peer).await;
                        }
                        continue;
                    }
                    let mut info = parse_quic_datagram(&buf[..n]);
                    info.src_ip = Some(peer.ip().to_string());
                    info.src_port = Some(peer.port());
                    if let Some(l) = local {
                        info.dst_ip = Some(l.ip().to_string());
                        info.dst_port = Some(l.port());
                    }
                    if info.parse_ok {
                        debug!(
                            "quic initial peer={} aead_ok={} ja4={:?}",
                            peer, info.aead_ok, info.tls_ja4
                        );
                        push_quic(info);
                    }
                }
                Err(e) => {
                    warn!("QUIC recv error: {e}");
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            }
        }
    })
}
