//! Pre-TLS HAProxy PROXY protocol v1/v2 (v4 probe lineage).

use async_trait::async_trait;
use dashmap::DashMap;
use gr_probe_net::{try_parse_proxy, ProxyProtocolInfo};
use log::{debug, warn};
use pingora_core::listeners::PreTlsProcess;
use pingora_core::protocols::l4::stream::Stream as L4Stream;
use pingora_core::protocols::GetSocketDigest;
use pingora_core::Result;
use std::sync::{Arc, OnceLock};
use tokio::io::AsyncReadExt;

fn store() -> &'static DashMap<String, ProxyProtocolInfo> {
    static S: OnceLock<DashMap<String, ProxyProtocolInfo>> = OnceLock::new();
    S.get_or_init(DashMap::new)
}

fn key_from_stream(stream: &L4Stream) -> Option<String> {
    let dig = stream.get_socket_digest()?;
    #[cfg(unix)]
    {
        if let Ok(c) = dig.socket_cookie() {
            if c != 0 {
                return Some(format!("sockcookie:{c:x}"));
            }
        }
    }
    let peer = dig.peer_addr()?.as_inet()?;
    let local = dig.local_addr()?.as_inet()?;
    Some(format!(
        "tuple:{}:{}-{}:{}",
        peer.ip(),
        peer.port(),
        local.ip(),
        local.port()
    ))
}

/// Lookup PROXY info for a connection key (peer tuple or sockcookie).
pub fn get_proxy_info(connection_key: &str) -> Option<ProxyProtocolInfo> {
    store().get(connection_key).map(|e| e.value().clone())
}

/// Remove + return PROXY info (cleanup after request). Prefer `get_proxy_info` on hot path.
#[allow(dead_code)]
pub fn take_proxy_info(connection_key: &str) -> Option<ProxyProtocolInfo> {
    store().remove(connection_key).map(|(_, v)| v)
}

/// Also try latest by client IP (fallback when key mismatch).
pub fn latest_proxy_for_ip(ip: &str) -> Option<ProxyProtocolInfo> {
    store()
        .iter()
        .filter_map(|e| {
            let v = e.value().clone();
            if v.src_ip.as_deref() == Some(ip) {
                Some(v)
            } else {
                None
            }
        })
        .max_by_key(|p| p.raw_header_len)
}

pub struct ProxyProtocolListener {
    pub enabled: bool,
}

#[async_trait]
impl PreTlsProcess for ProxyProtocolListener {
    async fn process(&self, stream: &mut L4Stream) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let mut buf = vec![0u8; 512];
        let n = match AsyncReadExt::read(stream, &mut buf).await {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(e) => {
                warn!("proxy_protocol read error: {e}");
                return Ok(());
            }
        };
        buf.truncate(n);

        // PROXY v2: may need full header length
        if buf.starts_with(b"\r\n\r\n\0\r\nQUIT\n") && buf.len() >= 16 {
            let len = u16::from_be_bytes([buf[14], buf[15]]) as usize + 16;
            if buf.len() < len && len <= 4096 {
                let mut more = vec![0u8; len - buf.len()];
                match AsyncReadExt::read_exact(stream, &mut more).await {
                    Ok(_) => buf.extend_from_slice(&more),
                    Err(e) => {
                        warn!("proxy_protocol v2 incomplete: {e}");
                        stream.rewind(&buf);
                        return Ok(());
                    }
                }
            }
        }

        match try_parse_proxy(&buf) {
            Some((info, consumed)) => {
                let key = key_from_stream(stream).unwrap_or_else(|| "unknown".into());
                debug!(
                    "PROXY v{} ok={} src={:?}:{:?} key={key}",
                    info.version, info.parse_ok, info.src_ip, info.src_port
                );
                store().insert(key, info);
                if consumed < buf.len() {
                    stream.rewind(&buf[consumed..]);
                }
            }
            None => {
                stream.rewind(&buf);
            }
        }
        Ok(())
    }
}

pub fn new_callback(enabled: bool) -> Arc<dyn PreTlsProcess> {
    Arc::new(ProxyProtocolListener { enabled })
}
