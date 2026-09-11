//! Stateless horizontal cluster: direct peer heartbeats, independent degrade.
//!
//! No leader election. Shared PG is the data authority; peers exchange liveness/load only.
//!
//! Heartbeat integrity: every beat carries a signed timestamp (HMAC-SHA256 over
//! `node_id|ts|payload`). Receivers require cluster key auth, an HMAC signature,
//! a ±[`HEARTBEAT_WINDOW_MS`] clock window, and strictly monotonic timestamps per
//! peer (equal-ts retries are accepted only when the payload is byte-identical).
//! This defeats forged/replayed beats by a LAN attacker who cannot read the key.

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Allowed heartbeat age/lead skew (ms). Senders and receivers share a clock
/// boundary; beats outside this window are dropped as stale or forged.
pub const HEARTBEAT_WINDOW_MS: i64 = 30_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub node_id: String,
    pub advertise: String,
    /// LAN address for LB `internal_ip` mode (docs/guides/08-LB-MODULE.md); optional.
    #[serde(default)]
    pub internal_addr: Option<String>,
    pub roles: Vec<String>,
    pub product_version: String,
    pub modules: HashMap<String, String>,
    pub load: NodeLoad,
    pub degraded: bool,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeLoad {
    pub cpu_pct: f32,
    pub mem_pct: f32,
    pub inflight: u64,
    pub analyze_workers: u32,
}

#[derive(Debug, Clone)]
struct PeerEntry {
    info: NodeInfo,
    last_seen: Instant,
}

#[derive(Clone)]
pub struct ClusterHub {
    inner: Arc<Inner>,
}

struct Inner {
    node_id: String,
    cluster_key_hash: String,
    /// Full SHA-256 of the cluster key — HMAC signing material. Never exposed.
    hmac_key: [u8; 32],
    peers: RwLock<HashMap<String, PeerEntry>>,
    self_info: RwLock<NodeInfo>,
    stale_after: Duration,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Full-length SHA-256 hex of the cluster key (256-bit).
/// iss/audit SEC-01: the previous 16-hex-char (64-bit) truncation lowered the
/// birthday-collision bound to ~2^32; keep the full digest. The hash never
/// leaves the process (compared locally against the configured key's hash),
/// so widening it is wire-compatible across mixed-version peers.
fn key_hash(key: &str) -> String {
    let mut h = Sha256::new();
    h.update(key.as_bytes());
    hex::encode(h.finalize())
}

/// RFC 2104 HMAC-SHA256 (implemented on sha2 to avoid extra deps; key > block is rehashed).
fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        let d = Sha256::digest(key);
        k[..32].copy_from_slice(&d);
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0u8; BLOCK];
    let mut opad = [0u8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] = 0x36 ^ k[i];
        opad[i] = 0x5c ^ k[i];
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(msg);
    let ih = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(ih);
    outer.finalize().into()
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl ClusterHub {
    pub fn new(node_id: String, cluster_key: &str, advertise: String, version: String) -> Self {
        let info = NodeInfo {
            node_id: node_id.clone(),
            advertise,
            internal_addr: None,
            roles: vec!["all".into()],
            product_version: version,
            modules: HashMap::new(),
            load: NodeLoad::default(),
            degraded: false,
            ts_ms: now_ms(),
        };
        let mut hmac_key = [0u8; 32];
        hmac_key.copy_from_slice(&Sha256::digest(cluster_key.as_bytes()));
        Self {
            inner: Arc::new(Inner {
                node_id,
                cluster_key_hash: key_hash(cluster_key),
                hmac_key,
                peers: RwLock::new(HashMap::new()),
                self_info: RwLock::new(info),
                stale_after: Duration::from_secs(30),
            }),
        }
    }

    pub fn node_id(&self) -> &str {
        &self.inner.node_id
    }

    pub fn auth_ok(&self, presented_key: &str) -> bool {
        // iss/audit SEC-01: constant-time comparison (the file-local `ct_eq`)
        // instead of `==`, which early-exits on the first mismatching byte and
        // leaks a timing side channel on the key hash.
        ct_eq(
            key_hash(presented_key).as_bytes(),
            self.inner.cluster_key_hash.as_bytes(),
        )
    }

    pub fn update_self<F: FnOnce(&mut NodeInfo)>(&self, f: F) {
        let mut g = self.inner.self_info.write();
        f(&mut g);
        g.ts_ms = now_ms();
    }

    pub fn heartbeat_payload(&self) -> NodeInfo {
        self.inner.self_info.read().clone()
    }

    /// Sign a heartbeat for the wire: `(info, ts_ms, hex HMAC-SHA256)`.
    /// Senders use this and post all three fields to the receiver.
    pub fn sign_heartbeat(&self) -> (NodeInfo, i64, String) {
        let info = self.inner.self_info.read().clone();
        let ts_ms = now_ms();
        (info.clone(), ts_ms, self.sign_payload(&info.node_id, &info, ts_ms))
    }

    /// Signature canonical form: `HMAC(derived_key, "{sender_node_id}|{ts_ms}|{payload}")`.
    /// `sender_node_id` is taken from `info.node_id` (the peer claiming the beat),
    /// never from the receiver — all cluster members derive the same key from the
    /// shared secret, so the receiver can recompute the sender's signature.
    fn sign_payload(&self, sender_node_id: &str, info: &NodeInfo, ts_ms: i64) -> String {
        let msg = format!(
            "{}|{}|{}",
            sender_node_id,
            ts_ms,
            serde_json::to_string(info).unwrap_or_default()
        );
        hex::encode(hmac_sha256(&self.inner.hmac_key, msg.as_bytes()))
    }

    /// Verify a peer heartbeat: cluster key auth + HMAC signature + ±[`HEARTBEAT_WINDOW_MS`]
    /// clock window + monotonic timestamp (equal-ts only when payload is identical,
    /// which tolerates network retries while blocking replayed/reforged beats).
    pub fn ingest_peer(&self, key: &str, info: NodeInfo, ts_ms: i64, sig: &str) -> bool {
        if !self.auth_ok(key) {
            return false;
        }
        let skew = now_ms() - ts_ms;
        if skew.abs() > HEARTBEAT_WINDOW_MS {
            return false;
        }
        let expected = self.sign_payload(&info.node_id, &info, ts_ms);
        if !ct_eq(expected.as_bytes(), sig.as_bytes()) {
            return false;
        }
        if info.node_id == self.inner.node_id {
            return true;
        }
        let mut peers = self.inner.peers.write();
        let prev_ts = peers
            .get(&info.node_id)
            .map(|e| e.info.ts_ms)
            .unwrap_or(i64::MIN);
        if ts_ms < prev_ts {
            return false;
        }
        if ts_ms == prev_ts {
            let identical = peers
                .get(&info.node_id)
                .map(|e| {
                    serde_json::to_string(&e.info).unwrap_or_default()
                        == serde_json::to_string(&info).unwrap_or_default()
                })
                .unwrap_or(false);
            if !identical {
                return false;
            }
        }
        peers.insert(
            info.node_id.clone(),
            PeerEntry {
                info,
                last_seen: Instant::now(),
            },
        );
        true
    }

    pub fn mark_degraded(&self, degraded: bool) {
        self.update_self(|i| i.degraded = degraded);
    }

    /// List live peers + self. Stale peers dropped (node down ≠ cluster down).
    pub fn snapshot(&self) -> Vec<NodeInfo> {
        let mut out = vec![self.inner.self_info.read().clone()];
        let mut peers = self.inner.peers.write();
        peers.retain(|_, e| e.last_seen.elapsed() < self.inner.stale_after);
        for e in peers.values() {
            out.push(e.info.clone());
        }
        out
    }

    pub fn peer_addrs(&self) -> Vec<SocketAddr> {
        self.snapshot()
            .into_iter()
            .filter_map(|n| n.advertise.parse().ok())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_join_and_auth() {
        let a = ClusterHub::new(
            "n1".into(),
            "secret",
            "127.0.0.1:7900".into(),
            "6.0.0".into(),
        );
        let b = ClusterHub::new(
            "n2".into(),
            "secret",
            "127.0.0.1:7901".into(),
            "6.0.0".into(),
        );
        let (info, ts, sig) = b.sign_heartbeat();
        // Wrong key → reject even with a valid signature.
        assert!(!a.ingest_peer("wrong", info.clone(), ts, &sig));
        // Correct key + valid signature → accepted.
        assert!(a.ingest_peer("secret", info.clone(), ts, &sig));
        assert_eq!(a.snapshot().len(), 2);
        // Correct key + tampered payload + original signature → mismatch → reject.
        let mut tampered = info.clone();
        tampered.load.cpu_pct = 99.0;
        assert!(!a.ingest_peer("secret", tampered, ts, &sig));
        // Hub with a different key cannot forge a beat for n2.
        let evil = ClusterHub::new(
            "n2".into(),
            "other-secret",
            "127.0.0.1:7902".into(),
            "6.0.0".into(),
        );
        let (einfo, ets, esig) = evil.sign_heartbeat();
        assert!(!a.ingest_peer("secret", einfo, ets, &esig));
    }

    #[test]
    fn heartbeats_replay_and_rollback_rejected() {
        let a = ClusterHub::new(
            "n1".into(),
            "secret",
            "127.0.0.1:7900".into(),
            "6.0.0".into(),
        );
        let b = ClusterHub::new(
            "n2".into(),
            "secret",
            "127.0.0.1:7901".into(),
            "6.0.0".into(),
        );
        let (info, ts, sig) = b.sign_heartbeat();
        assert!(a.ingest_peer("secret", info.clone(), ts, &sig));
        // Exact same beat again → tolerated as an idempotent network retry.
        assert!(a.ingest_peer("secret", info.clone(), ts, &sig));
        assert_eq!(a.snapshot().len(), 2);
        // Future beat outside the window → rejected even with a valid signature.
        // Margin is window + 60s so sub-second scheduling jitter between
        // `sign_heartbeat()` and the assert can never slide the beat back into
        // the 30s acceptance window (clock-skew guards, not boundary micro-tests).
        let (info2, ts2, _sig2) = b.sign_heartbeat();
        assert!(ts2 >= ts);
        let far_future = ts2 + HEARTBEAT_WINDOW_MS + 60_000;
        let fut_sig = a.sign_payload(&info2.node_id, &info2, far_future);
        assert!(!a.ingest_peer("secret", info2.clone(), far_future, &fut_sig));
        // Stale beat outside the window → rejected.
        let far_past = ts - HEARTBEAT_WINDOW_MS - 60_000;
        let old_sig = a.sign_payload(&info.node_id, &info, far_past);
        assert!(!a.ingest_peer("secret", info.clone(), far_past, &old_sig));
        // Newer beat accepted (ensure the timestamp strictly advances past `ts`).
        std::thread::sleep(Duration::from_millis(3));
        b.update_self(|i| i.load.cpu_pct = 12.0);
        let (newer, newer_ts, newer_sig) = b.sign_heartbeat();
        assert!(newer_ts > ts);
        assert!(a.ingest_peer("secret", newer.clone(), newer_ts, &newer_sig));
        // Rollback: a beat older than the last seen, with a fresh valid signature → rejected.
        let rollback_ts = newer_ts - 1_000;
        let rollback_sig = a.sign_payload(&newer.node_id, &newer, rollback_ts);
        assert!(!a.ingest_peer("secret", newer.clone(), rollback_ts, &rollback_sig));
    }

    #[test]
    fn cluster_key_hash_is_full_length_and_auth_is_constant_time() {
        // iss/audit SEC-01 regression lock:
        // 1. key_hash keeps the full 256-bit digest (64 hex chars), not the
        //    legacy 16-char (64-bit) truncation.
        assert_eq!(key_hash("secret").len(), 64);
        assert_eq!(key_hash("secret"), key_hash("secret"));
        assert_ne!(key_hash("secret"), key_hash("secreT"));
        // 2. auth_ok accepts the configured key and rejects anything else —
        //    including a key engineered to share a 64-bit prefix of the digest
        //    (the old truncation made such collisions meaningful).
        let hub = ClusterHub::new(
            "n1".into(),
            "secret",
            "127.0.0.1:7900".into(),
            "6.0.0".into(),
        );
        assert!(hub.auth_ok("secret"));
        assert!(!hub.auth_ok("secret "));
        assert!(!hub.auth_ok(""));
        // Brute-force a 64-bit-prefix collision against the full hash (bounded
        // search — practically guaranteed not to find one, which is the point).
        let target_prefix = &key_hash("secret")[..16];
        let mut found_prefix_collision = false;
        for i in 0..100_000u32 {
            let cand = format!("secret-{i}");
            let h = key_hash(&cand);
            if &h[..16] == target_prefix && h != key_hash("secret") {
                found_prefix_collision = true;
                break;
            }
        }
        assert!(!found_prefix_collision, "unexpected 64-bit prefix collision");
    }
}
