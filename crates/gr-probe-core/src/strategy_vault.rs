//! In-memory strategy vault — plaintext strategy bodies must not hit disk.
//!
//! Bundles are pulled from the official site (AES-GCM + Ed25519). Nodes keep
//! decrypted payloads in process memory only (`Arc<RwLock<…>>`).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultEntry {
    pub site_id: String,
    pub domain: String,
    pub plan: String,
    pub profile_cap: String,
    pub strategy_version_id: String,
    pub strategy: Value,
    pub allowed_strategy_ids: Vec<String>,
    pub expires_at_ms: i64,
    pub loaded_at_ms: i64,
    #[serde(default)]
    pub rpa_enabled: bool,
    #[serde(default)]
    pub device_precisions: Vec<String>,
}

#[derive(Default)]
pub struct StrategyVault {
    by_domain: RwLock<HashMap<String, VaultEntry>>,
    by_site: RwLock<HashMap<String, VaultEntry>>,
}

impl StrategyVault {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(&self, entry: VaultEntry) {
        let mut d = self.by_domain.write().unwrap_or_else(|e| e.into_inner());
        let mut s = self.by_site.write().unwrap_or_else(|e| e.into_inner());
        d.insert(entry.domain.to_ascii_lowercase(), entry.clone());
        s.insert(entry.site_id.clone(), entry);
    }

    pub fn get_by_domain(&self, domain: &str) -> Option<VaultEntry> {
        let d = self.by_domain.read().unwrap_or_else(|e| e.into_inner());
        d.get(&domain.to_ascii_lowercase()).cloned().filter(|e| !expired(e))
    }

    pub fn get_by_site(&self, site_id: &str) -> Option<VaultEntry> {
        let s = self.by_site.read().unwrap_or_else(|e| e.into_inner());
        s.get(site_id).cloned().filter(|e| !expired(e))
    }

    pub fn clear(&self) {
        self.by_domain.write().unwrap_or_else(|e| e.into_inner()).clear();
        self.by_site.write().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// Metadata only — never expose strategy body to admin SPA.
    pub fn list_domains_meta(&self) -> Vec<serde_json::Value> {
        let d = self.by_domain.read().unwrap_or_else(|e| e.into_inner());
        d.values()
            .filter(|e| !expired(e))
            .map(|e| {
                serde_json::json!({
                    "site_id": e.site_id,
                    "domain": e.domain,
                    "plan": e.plan,
                    "profile_cap": e.profile_cap,
                    "strategy_version_id": e.strategy_version_id,
                    "strategy_id": e.strategy.get("strategy_id").and_then(|v| v.as_str()).unwrap_or(""),
                    "expires_at_ms": e.expires_at_ms,
                })
            })
            .collect()
    }
}

fn expired(e: &VaultEntry) -> bool {
    now_ms() > e.expires_at_ms
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub type SharedVault = Arc<StrategyVault>;

pub fn shared_vault() -> SharedVault {
    Arc::new(StrategyVault::new())
}

use std::sync::OnceLock;

static GLOBAL_VAULT: OnceLock<SharedVault> = OnceLock::new();

/// Process-wide vault (control plane loads; probe plane reads).
pub fn global_vault() -> SharedVault {
    GLOBAL_VAULT
        .get_or_init(|| Arc::new(StrategyVault::new()))
        .clone()
}

/// Decrypt official-site bundle using lab wrap key (AES-256-GCM: ct||tag).
pub fn decrypt_bundle(
    wrap_key: &[u8],
    iv: &[u8],
    ciphertext_and_tag: &[u8],
) -> Result<Value, String> {
    use aes_gcm::{
        aead::{Aead, KeyInit},
        Aes256Gcm, Nonce,
    };
    if wrap_key.len() != 32 {
        return Err("wrap_key must be 32 bytes".into());
    }
    if ciphertext_and_tag.len() < 16 {
        return Err("ciphertext too short".into());
    }
    let (ct, tag) = ciphertext_and_tag.split_at(ciphertext_and_tag.len() - 16);
    let cipher = Aes256Gcm::new_from_slice(wrap_key).map_err(|e| e.to_string())?;
    let nonce = Nonce::from_slice(iv);
    // aes-gcm crate expects tag appended to ciphertext for decrypt in some APIs —
    // use payload = ct || tag
    let mut payload = ct.to_vec();
    payload.extend_from_slice(tag);
    let pt = cipher
        .decrypt(nonce, payload.as_ref())
        .map_err(|_| "decrypt_failed".to_string())?;
    serde_json::from_slice(&pt).map_err(|e| e.to_string())
}

pub fn entry_from_payload(payload: &Value, header_exp_unix: i64) -> Result<VaultEntry, String> {
    let plan = payload
        .get("plan")
        .and_then(|v| v.as_str())
        .unwrap_or("free")
        .to_string();
    // Keep legacy plan metadata parseable, but do not let it gate capabilities.
    let ent = crate::plan_entitlement::PlanEntitlement::from_plan(&plan);
    let now = now_ms();
    let expires_at_ms = header_exp_unix.saturating_mul(1000);
    if header_exp_unix > 0 && now > expires_at_ms {
        return Err("bundle_expired".into());
    }
    Ok(VaultEntry {
        site_id: payload
            .get("site_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .into(),
        domain: payload
            .get("domain")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .into(),
        plan: ent.plan.clone(),
        profile_cap: "advanced".into(),
        strategy_version_id: payload
            .get("strategy_version_id")
            .and_then(|v| v.as_str())
            .unwrap_or("builtin_plan@1")
            .into(),
        strategy: payload
            .get("strategy")
            .cloned()
            .unwrap_or(Value::Null),
        allowed_strategy_ids: payload
            .get("allowed_strategy_ids")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        expires_at_ms: if expires_at_ms > 0 {
            expires_at_ms
        } else {
            now + 15 * 60 * 1000
        },
        loaded_at_ms: now,
        rpa_enabled: ent.rpa_enabled,
        device_precisions: ent.device_precisions.clone(),
    })
}
