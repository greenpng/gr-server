//! iss/39 R7: challenge / pack version rotation skeleton (lab/canary).

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Rotate challenge secret epoch without touching Soft/digest redlines.
pub fn rotate_challenge_epoch(tenant: &str, epoch: u64, base_secret: &str) -> Value {
    let mut h = Sha256::new();
    h.update(b"gr_challenge_epoch|");
    h.update(tenant.as_bytes());
    h.update(b"|");
    h.update(epoch.to_string().as_bytes());
    h.update(b"|");
    h.update(base_secret.as_bytes());
    let dig = format!("{:x}", h.finalize());
    let derived = format!("gr-epoch-{epoch}-{}", &dig[..24]);
    json!({
        "algo": "challenge_pack_rotate_v1",
        "tenant": tenant,
        "epoch": epoch,
        "challenge_secret_hint": &derived[..16.min(derived.len())],
        "challenge_secret_derived": derived,
        "pack_catalog_epoch": format!("packs_e{epoch}"),
        "fe_cache_bust": format!("e{epoch}"),
        "auto_apply": false,
        "note": "caller injects derived secret into challenge issuer; FE cache-bust optional",
        "redlines": {"soft_promote": false, "never_digest_edit": false}
    })
}

/// Suggest next epoch from current.
pub fn next_rotation(tenant: &str, current_epoch: u64, base_secret: &str) -> Value {
    let mut out = rotate_challenge_epoch(tenant, current_epoch.saturating_add(1), base_secret);
    if let Some(obj) = out.as_object_mut() {
        obj.insert("previous_epoch".into(), json!(current_epoch));
    }
    out
}
