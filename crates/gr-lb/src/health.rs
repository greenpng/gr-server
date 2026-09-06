//! Phase-1 active health check: periodic plain-HTTP GET /v1/health per node.
//!
//! Failures are counted in [`crate::LbPool::report_health`]; a node leaves the
//! candidate set only after `health_check_fail_threshold` consecutive misses,
//! and recovers on the first success.

use crate::{host_of, LbPool};
use std::sync::Arc;
use std::time::Duration;

/// Spawn the probe loop on the current tokio runtime handle (panics outside
/// a runtime). Prefer [`probe_loop`] + an explicit handle when wiring from a
/// plain thread (gr-service control loop).
pub fn spawn_active_probe(pool: Arc<LbPool>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(probe_loop(pool))
}

/// Active health-check loop body: probe every node every interval, report
/// into the pool (threshold-based removal, single-success recovery).
pub async fn probe_loop(pool: Arc<LbPool>) {
    tracing::info!("gr-lb active health probe loop started");
    loop {
        let interval_ms = pool.config().health_check_interval_ms.max(1000);
        let timeout_ms = pool.config().health_check_timeout_ms.max(100);
        let targets = pool.probe_targets();
        for (id, addr) in targets {
            let ok = probe_node(&addr, timeout_ms).await;
            pool.report_health(&id, ok);
        }
        tokio::time::sleep(Duration::from_millis(interval_ms)).await;
    }
}

async fn probe_node(addr: &str, timeout_ms: u64) -> bool {
    let host = host_of(addr);
    let req = format!(
        "GET /v1/health HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nUser-Agent: gr-lb-health/1\r\n\r\n"
    );
    let timeout = Duration::from_millis(timeout_ms);
    let fut = async move {
        let mut s = tokio::net::TcpStream::connect(addr).await.map_err(|_| ())?;
        tokio::io::AsyncWriteExt::write_all(&mut s, req.as_bytes())
            .await
            .map_err(|_| ())?;
        let mut buf = [0u8; 4096];
        let mut got = Vec::new();
        loop {
            match tokio::io::AsyncReadExt::read(&mut s, &mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    got.extend_from_slice(&buf[..n]);
                    if got.len() > 8192 {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let head = String::from_utf8_lossy(&got);
        Ok::<bool, ()>(head.starts_with("HTTP/1.1 20") || head.starts_with("HTTP/1.0 20"))
    };
    match tokio::time::timeout(timeout, fut).await {
        Ok(Ok(ok)) => ok,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_split() {
        assert_eq!(host_of("10.0.0.1:28765"), "10.0.0.1");
        assert_eq!(host_of("[fd00::1]:28765"), "fd00::1");
        assert_eq!(host_of("gw.example.com:443"), "gw.example.com");
    }
}
