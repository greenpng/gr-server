//! Seal suite v2 — epoch-bound session grants + envelope metadata gates.
//!
//! Strict browser path (default when `GR_SEAL_REQUIRE_V2=1` / sealed ingest prod):
//! - KDF binds `session_id|exp|fe_epoch|suite_id`
//! - Envelope carries `fe_epoch`, `suite_id`, `wasm_module_id`, `challenge_bind`, `pack_set_hash`
//! - Signature covers all AAD fields (anti-tamper of metadata)
//! - Server rejects legacy v1 envelopes and stale epochs

use crate::GR_PRODUCT_VERSION;
use crate::seal::{SealError, SealedEnvelope, SESSION_SEAL_TTL_MS};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Current seal suite (crypto algorithm matrix entry).
pub const SEAL_SUITE_S2_AESGCM_HKDF_V1: &str = "s2-aesgcm-hkdf-v1";
/// Versioned WASM crypto module id (must match FE load + allowlist).
pub const SEAL_WASM_MODULE_ID: &str = "wm-v2-s2-20260806";
/// Public path for WASM under first-party / versioned dist.
pub const SEAL_WASM_ASSET: &str = "gr_seal_v2.wasm";

/// Allowed B10 cpu_loop_algo prefixes for current epoch (content gate).
/// v3 = multi-round median fuse (DrawnApart-style reliability) — **required** by default.
/// Legacy v2 single-shot is rejected unless `GR_SEAL_ALLOW_B10_V2=1` (lab migration only).
pub const B10_ALLOWED_ALGO_PREFIXES: &[&str] = &["gr_cpu_curve_v3_multiround", "v3_multiround"];

/// Legacy prefixes accepted only when `GR_SEAL_ALLOW_B10_V2` is truthy.
pub const B10_LEGACY_V2_ALGO_PREFIXES: &[&str] =
    &["gr_cpu_curve_v2_multiworkload", "v2_multiworkload"];

fn seal_allow_b10_v2() -> bool {
    matches!(
        gr_abi::env::get("SEAL_ALLOW_B10_V2")
            .as_deref()
            .map(|s| s.trim()),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

fn hmac_new(key: &[u8]) -> HmacSha256 {
    // Fail-closed (iss/opus5 S-1): an empty seal master key must never silently
    // degrade to a well-known all-zero key — anyone could then forge valid
    // signatures while the system appears healthy. Startup config validation
    // makes this unreachable; the assert is defense-in-depth.
    assert!(
        !key.is_empty(),
        "{} — set GR_SEAL_SECRET (refusing fail-open zero key)",
        gr_obf::obf!("seal master key is empty").s()
    );
    <HmacSha256 as Mac>::new_from_slice(key).expect("hmac accepts keys of any length")
}

/// Env: require seal envelope v>=2 for browser session path.
pub fn seal_require_v2() -> bool {
    match gr_abi::env::get("SEAL_REQUIRE_V2").as_deref().map(|s| s.trim()) {
        Some("0") | Some("false") | Some("FALSE") | Some("no") | Some("NO") => false,
        Some(_) => true,
        // Default: follow sealed-ingest production posture
        None => match gr_abi::env::get("REQUIRE_SEALED_INGEST").as_deref().map(|s| s.trim()) {
            Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES") => true,
            // If sealed required by prod default (unset often means prod sets it later):
            // When ALLOW_PLAIN, don't force v2 for lab convenience unless explicit.
            _ => {
                let allow_plain = matches!(
                    gr_abi::env::get("ALLOW_PLAIN_INGEST").as_deref().map(|s| s.trim()),
                    Some("1") | Some("true") | Some("TRUE")
                );
                !allow_plain
            }
        },
    }
}

/// Env: allow extra epochs (comma-separated) during migrate / lab content stamps.
pub fn allowed_fe_epochs() -> Vec<String> {
    let mut out = vec![GR_PRODUCT_VERSION.to_string()];
    if let Some(extra) = gr_abi::env::get("SEAL_ALLOW_EPOCHS") {
        for p in extra.split(',') {
            let t = p.trim();
            if !t.is_empty() && !out.iter().any(|x| x == t) {
                out.push(t.to_string());
            }
        }
    }
    // Lab: also accept server product when only set via GR_* env aliases
    for key in ["GR_PRODUCT_VERSION", "GR_PRODUCT_VERSION"] {
        if let Ok(v) = std::env::var(key) {
            let t = v.trim();
            if !t.is_empty() && !out.iter().any(|x| x == t) {
                out.push(t.to_string());
            }
        }
    }
    out
}

fn labish_seal_env() -> bool {
    matches!(
        gr_abi::env::get("DEPLOY_ENV")
            .as_deref()
            .map(|s| s.trim().to_ascii_lowercase()),
        Some(ref s) if s == "lab" || s == "dev" || s == "local"
    ) || matches!(
        gr_abi::env::get("LAB_DISABLE_COOL")
            .or_else(|| gr_abi::env::get("ALLOW_LAB_CHALLENGE"))
            .as_deref()
            .map(|s| s.trim()),
        Some("1") | Some("true") | Some("TRUE") | Some("yes")
    )
}

/// Whether fe_impl_version is acceptable for B10 seal.
/// - always: product epoch allowlist (substring match either way)
/// - lab: also accept content stamps that embed product, or pure v5.8.* lab content ids
///   when `GR_SEAL_ALLOW_CONTENT_FE_IMPL=1` (default on in labish env)
fn fe_impl_epoch_ok(fe_impl: &str) -> bool {
    if fe_impl.is_empty() {
        return false;
    }
    let allowed = allowed_fe_epochs();
    if allowed
        .iter()
        .any(|e| fe_impl == e.as_str() || fe_impl.contains(e.as_str()) || e.contains(fe_impl))
    {
        return true;
    }
    let allow_content = matches!(
        gr_abi::env::get("SEAL_ALLOW_CONTENT_FE_IMPL")
            .as_deref()
            .map(|s| s.trim()),
        Some("1") | Some("true") | Some("TRUE") | Some("yes")
    ) || (labish_seal_env()
        && !matches!(
            gr_abi::env::get("SEAL_ALLOW_CONTENT_FE_IMPL")
                .as_deref()
                .map(|s| s.trim()),
            Some("0") | Some("false") | Some("FALSE") | Some("no")
        ));
    if allow_content {
        // Lab content stamps look like v5.8.NNN-YYYYMMDD-...; still bind to product
        // when the stamp embeds any allowed epoch token.
        if allowed.iter().any(|e| fe_impl.contains(e.as_str())) {
            return true;
        }
        // Pure lab content-id: accept only when labish (never auto-accept random strings in prod)
        if labish_seal_env()
            && (fe_impl.starts_with("v5.8.")
                || fe_impl.starts_with("v6.")
                || fe_impl.starts_with("6."))
        {
            return true;
        }
    }
    false
}

pub fn current_suite_id() -> &'static str {
    SEAL_SUITE_S2_AESGCM_HKDF_V1
}

pub fn current_wasm_module_id() -> &'static str {
    SEAL_WASM_MODULE_ID
}

/// Suite allowlist: current (+ optional canary via env).
pub fn suite_allowed(suite_id: &str) -> bool {
    if suite_id == SEAL_SUITE_S2_AESGCM_HKDF_V1 {
        return true;
    }
    if let Some(extra) = gr_abi::env::get("SEAL_ALLOW_SUITES") {
        return extra.split(',').any(|s| s.trim() == suite_id);
    }
    false
}

pub fn wasm_module_allowed(id: &str) -> bool {
    if id == SEAL_WASM_MODULE_ID {
        return true;
    }
    if let Some(extra) = gr_abi::env::get("SEAL_ALLOW_WASM_MODULES") {
        return extra.split(',').any(|s| s.trim() == id);
    }
    false
}

/// v2 session secret: HMAC(master, "gr-session-seal-v2|"‖sid‖"|"‖exp‖"|"‖epoch‖"|"‖suite)
pub fn derive_session_seal_secret_v2(
    master: &[u8],
    session_id: &str,
    exp_ms: i64,
    fe_epoch: &str,
    suite_id: &str,
) -> Vec<u8> {
    let mut mac = hmac_new(master);
    mac.update(b"gr-session-seal-v2|");
    mac.update(session_id.as_bytes());
    mac.update(b"|");
    mac.update(exp_ms.to_string().as_bytes());
    mac.update(b"|");
    mac.update(fe_epoch.as_bytes());
    mac.update(b"|");
    mac.update(suite_id.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// Issue strict browser grant (epoch + suite bound).
pub fn issue_session_seal_grant_v2(
    master: &[u8],
    session_id: &str,
    now_ms: u64,
    ttl_ms: u64,
    fe_epoch: &str,
) -> Value {
    let ttl = if ttl_ms == 0 { SESSION_SEAL_TTL_MS } else { ttl_ms };
    let exp_ms = (now_ms.saturating_add(ttl)) as i64;
    let suite_id = current_suite_id();
    let wasm_id = current_wasm_module_id();
    let key = derive_session_seal_secret_v2(master, session_id, exp_ms, fe_epoch, suite_id);
    let key_b64 = B64.encode(&key);
    let mut mac = hmac_new(master);
    mac.update(b"gr-session-seal-grant-v2|");
    mac.update(session_id.as_bytes());
    mac.update(b"|");
    mac.update(exp_ms.to_string().as_bytes());
    mac.update(b"|");
    mac.update(fe_epoch.as_bytes());
    mac.update(b"|");
    mac.update(suite_id.as_bytes());
    mac.update(b"|");
    mac.update(wasm_id.as_bytes());
    mac.update(b"|");
    mac.update(key_b64.as_bytes());
    let grant_sig = B64.encode(mac.finalize().into_bytes());
    // Standard C: content-hash in filename only — never /dist/v/<version>/ (anti-leak).
    let (_, wasm_sha) = seal_wasm_file_integrity();
    let wasm_hash12 = wasm_sha
        .as_deref()
        .map(|s| s.chars().take(12).collect::<String>())
        .filter(|s| s.len() >= 8)
        .unwrap_or_else(|| "missing00000".into());
    let wasm_name = format!("gr_seal_v2.{wasm_hash12}.wasm");
    let loader_name = format!("gr_seal_v2_loader.{wasm_hash12}.js");
    json!({
        "algo": "gr_session_seal_v2",
        "key_mode": "session",
        "seal_protocol": 2,
        "session_id": session_id,
        "exp_ms": exp_ms,
        "ttl_ms": ttl,
        "key_b64": key_b64,
        "grant_sig_b64": grant_sig,
        "fe_epoch": fe_epoch,
        "product_version": fe_epoch,
        "suite_id": suite_id,
        "wasm_module_id": wasm_id,
        "wasm_url": format!("/g5/dist/{wasm_name}"),
        "wasm_url_flat": format!("/g5/dist/{wasm_name}"),
        "loader_url": format!("/g5/dist/{loader_name}"),
        "require_wasm": true,
        "note": "v2 epoch-bound grant; wasm URL is content-hash filename (no version path)",
    })
}

/// Sign v2 envelope: includes AAD metadata in MAC.
pub fn sign_envelope_v2(mac_key: &[u8; 32], envelope: &SealedEnvelope) -> Result<String, SealError> {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(mac_key)
        .map_err(|e| SealError::Crypto(e.to_string()))?;
    mac.update(b"v2|");
    mac.update(envelope.session_id.as_bytes());
    mac.update(b"|");
    mac.update(envelope.visitor_terminal_id.as_bytes());
    mac.update(b"|");
    mac.update(envelope.batch_id.as_bytes());
    mac.update(b"|");
    mac.update(envelope.suite_id.as_deref().unwrap_or("").as_bytes());
    mac.update(b"|");
    mac.update(envelope.fe_epoch.as_deref().unwrap_or("").as_bytes());
    mac.update(b"|");
    mac.update(envelope.wasm_module_id.as_deref().unwrap_or("").as_bytes());
    mac.update(b"|");
    mac.update(envelope.challenge_bind.as_deref().unwrap_or("").as_bytes());
    mac.update(b"|");
    mac.update(envelope.pack_set_hash.as_deref().unwrap_or("").as_bytes());
    mac.update(b"|");
    mac.update(envelope.nonce_b64.as_bytes());
    mac.update(b"|");
    mac.update(envelope.ciphertext_b64.as_bytes());
    Ok(B64.encode(mac.finalize().into_bytes()))
}

fn derive_keys_v2(secret: &[u8]) -> ([u8; 32], [u8; 32]) {
    use sha2::{Digest, Sha256};
    let mut h1 = Sha256::new();
    h1.update(b"gr-seal-enc-v2");
    h1.update(secret);
    let mut enc = [0u8; 32];
    enc.copy_from_slice(&h1.finalize());
    let mut h2 = Sha256::new();
    h2.update(b"gr-seal-mac-v2");
    h2.update(secret);
    let mut mac = [0u8; 32];
    mac.copy_from_slice(&h2.finalize());
    (enc, mac)
}

/// Seal with v2 metadata (browser path). Uses same AES-GCM body crypto with v2 key labels.
pub fn seal_probe_payload_v2(
    secret: &[u8],
    session_id: &str,
    visitor_terminal_id: &str,
    batch_id: &str,
    payload: &Value,
    seal_exp_ms: i64,
    fe_epoch: &str,
    suite_id: &str,
    wasm_module_id: &str,
    challenge_bind: &str,
    pack_set_hash: &str,
    use_deflate: bool,
) -> Result<SealedEnvelope, SealError> {
    // Reuse compress/encrypt via temporary seal then rewrite — cleaner: call ex2 then patch.
    // But ex2 uses v1 key labels. Implement body encrypt with v2 keys:
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};
    use flate2::write::DeflateEncoder;
    use flate2::Compression;
    use rand::RngCore;
    use std::io::Write;

    let plain = serde_json::to_vec(payload).map_err(|e| SealError::Json(e.to_string()))?;
    let pre_aes = if use_deflate {
        let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&plain)
            .map_err(|e| SealError::Io(e.to_string()))?;
        enc.finish().map_err(|e| SealError::Io(e.to_string()))?
    } else {
        plain
    };
    let (enc_key, mac_key) = derive_keys_v2(secret);
    let cipher = Aes256Gcm::new_from_slice(&enc_key)
        .map_err(|e| SealError::Crypto(e.to_string()))?;
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, pre_aes.as_ref())
        .map_err(|e| SealError::Crypto(format!("encrypt: {e}")))?;
    let alg = if use_deflate {
        "aes256gcm+hmacsha256+deflate"
    } else {
        "aes256gcm+hmacsha256+identity"
    };
    let mut env = SealedEnvelope {
        v: 2,
        alg: alg.into(),
        session_id: session_id.to_string(),
        visitor_terminal_id: visitor_terminal_id.to_string(),
        batch_id: batch_id.to_string(),
        nonce_b64: B64.encode(nonce_bytes),
        ciphertext_b64: B64.encode(ct),
        sig_b64: String::new(),
        key_mode: Some("session".into()),
        seal_exp_ms: Some(seal_exp_ms),
        suite_id: Some(suite_id.to_string()),
        fe_epoch: Some(fe_epoch.to_string()),
        wasm_module_id: Some(wasm_module_id.to_string()),
        challenge_bind: Some(challenge_bind.to_string()),
        pack_set_hash: Some(pack_set_hash.to_string()),
    };
    env.sig_b64 = sign_envelope_v2(&mac_key, &env)?;
    Ok(env)
}

fn unseal_v2_body(secret: &[u8], envelope: &SealedEnvelope) -> Result<Value, SealError> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};

    let (enc_key, mac_key) = derive_keys_v2(secret);
    let expect = sign_envelope_v2(&mac_key, envelope)?;
    if expect != envelope.sig_b64 {
        return Err(SealError::BadSignature);
    }
    let cipher = Aes256Gcm::new_from_slice(&enc_key)
        .map_err(|e| SealError::Crypto(e.to_string()))?;
    let nonce_raw = B64
        .decode(&envelope.nonce_b64)
        .map_err(|e| SealError::Envelope(e.to_string()))?;
    if nonce_raw.len() != 12 {
        return Err(SealError::Envelope("nonce must be 12 bytes".into()));
    }
    let nonce = Nonce::from_slice(&nonce_raw);
    let ct = B64
        .decode(&envelope.ciphertext_b64)
        .map_err(|e| SealError::Envelope(e.to_string()))?;
    let pre = cipher
        .decrypt(nonce, ct.as_ref())
        .map_err(|_| SealError::Crypto("decrypt failed".into()))?;
    let a = envelope.alg.to_ascii_lowercase();
    let plain = if a.contains("deflate") && !a.contains("identity") {
        crate::seal::decompress_limited(&pre, crate::seal::MAX_DECOMPRESSED_BYTES)?
    } else {
        if pre.len() as u64 > crate::seal::MAX_DECOMPRESSED_BYTES {
            return Err(SealError::TooLarge);
        }
        pre
    };
    serde_json::from_slice(&plain).map_err(|e| SealError::Json(e.to_string()))
}

/// Validate v2 policy gates (before or after crypto).
pub fn validate_envelope_v2_policy(envelope: &SealedEnvelope) -> Result<(), SealError> {
    if envelope.v < 2 {
        return Err(SealError::Envelope(
            "seal_v1_rejected: require seal protocol v2 (upgrade FE pin/WASM)".into(),
        ));
    }
    let suite = envelope.suite_id.as_deref().unwrap_or("");
    if !suite_allowed(suite) {
        return Err(SealError::Envelope(format!(
            "seal_suite_rejected: suite_id={suite}"
        )));
    }
    let epoch = envelope.fe_epoch.as_deref().unwrap_or("");
    if epoch.is_empty() {
        return Err(SealError::Envelope("seal_epoch_missing".into()));
    }
    let allowed = allowed_fe_epochs();
    if !allowed.iter().any(|e| e == epoch) {
        return Err(SealError::Envelope(format!(
            "seal_epoch_rejected: fe_epoch={epoch} allowed={allowed:?}"
        )));
    }
    let wasm_id = envelope.wasm_module_id.as_deref().unwrap_or("");
    if wasm_id.is_empty() || !wasm_module_allowed(wasm_id) {
        return Err(SealError::Envelope(format!(
            "seal_wasm_rejected: wasm_module_id={wasm_id}"
        )));
    }
    // challenge_bind + pack_set_hash required non-empty for browser session seals
    if envelope
        .challenge_bind
        .as_deref()
        .unwrap_or("")
        .is_empty()
    {
        return Err(SealError::Envelope("seal_challenge_bind_missing".into()));
    }
    if envelope.pack_set_hash.as_deref().unwrap_or("").is_empty() {
        return Err(SealError::Envelope("seal_pack_set_hash_missing".into()));
    }
    Ok(())
}

/// Content gate after unseal (critical packs).
pub fn validate_unsealed_content(batch_id: &str, payload: &Value) -> Result<(), SealError> {
    let bid = batch_id.trim();
    if bid != "B10_hw_curves" {
        return Ok(());
    }
    // Locate fields.cpu_loop_algo
    let fields = payload
        .get("payload")
        .and_then(|p| p.get("fields"))
        .or_else(|| payload.get("fields"))
        .cloned()
        .unwrap_or(json!({}));
    let algo = fields
        .get("cpu_loop_algo")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if algo.is_empty() {
        return Err(SealError::Envelope(
            "b10_algo_missing: cpu_loop_algo required for B10 under seal v2".into(),
        ));
    }
    let ok_v3 = B10_ALLOWED_ALGO_PREFIXES.iter().any(|p| algo.contains(p));
    let ok_v2 = seal_allow_b10_v2() && B10_LEGACY_V2_ALGO_PREFIXES.iter().any(|p| algo.contains(p));
    if !(ok_v3 || ok_v2) {
        return Err(SealError::Envelope(format!(
            "b10_algo_rejected: cpu_loop_algo={algo} (require v3_multiround; set GR_SEAL_ALLOW_B10_V2=1 only for lab migration)"
        )));
    }
    // Prefer fe_impl present for honesty (soft: warn via allow empty only if env)
    let fe_impl = fields
        .get("fe_impl_version")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if fe_impl.is_empty()
        && !matches!(
            gr_abi::env::get("SEAL_ALLOW_EMPTY_FE_IMPL").as_deref().map(|s| s.trim()),
            Some("1") | Some("true")
        )
    {
        return Err(SealError::Envelope(
            "fe_impl_missing: fe_impl_version required on B10 under seal v2".into(),
        ));
    }
    if !fe_impl.is_empty() && !fe_impl_epoch_ok(fe_impl) {
        return Err(SealError::Envelope(format!(
            "fe_impl_epoch_rejected: fe_impl_version={fe_impl} allowed={:?}",
            allowed_fe_epochs()
        )));
    }
    Ok(())
}

/// Production unseal auto for v1+v2 with strict policy.
pub fn unseal_probe_payload_auto_v2(
    master: &[u8],
    master_prev: Option<&[u8]>,
    envelope: &SealedEnvelope,
    now_ms: i64,
) -> Result<Value, SealError> {
    let require_v2 = seal_require_v2();
    if require_v2 && envelope.v < 2 {
        return Err(SealError::Envelope(
            "seal_v1_rejected: GR_SEAL_REQUIRE_V2 — upgrade FE to pin+WASM seal v2".into(),
        ));
    }
    if envelope.v >= 2 {
        validate_envelope_v2_policy(envelope)?;
        let exp = envelope.seal_exp_ms.unwrap_or(0);
        if exp > 0 && now_ms > exp {
            return Err(SealError::Expired);
        }
        let epoch = envelope.fe_epoch.as_deref().unwrap_or("");
        let suite = envelope.suite_id.as_deref().unwrap_or(SEAL_SUITE_S2_AESGCM_HKDF_V1);
        let sid = &envelope.session_id;
        let try_unseal = |master_key: &[u8]| -> Result<Value, SealError> {
            let sess =
                derive_session_seal_secret_v2(master_key, sid, exp, epoch, suite);
            unseal_v2_body(&sess, envelope)
        };
        match try_unseal(master) {
            Ok(v) => {
                validate_unsealed_content(&envelope.batch_id, &v)?;
                return Ok(v);
            }
            Err(SealError::BadSignature) => {
                if let Some(prev) = master_prev {
                    if !prev.is_empty() {
                        if let Ok(v) = try_unseal(prev) {
                            validate_unsealed_content(&envelope.batch_id, &v)?;
                            return Ok(v);
                        }
                    }
                }
                return Err(SealError::BadSignature);
            }
            Err(e) => return Err(e),
        }
    }
    // Legacy v1 path (lab / edge master)
    crate::seal::unseal_probe_payload_auto_legacy(master, master_prev, envelope, now_ms)
}

/// Best-effort sha256 + byte length of the shipped seal wasm (for FE integrity checks).
///
/// Path resolution must not depend on process cwd alone. Lab often starts from `$HOME`
/// while prod systemd uses `WorkingDirectory=/opt/green-v6`. Prefer env/static-dir.
pub fn seal_wasm_file_integrity() -> (Option<u64>, Option<String>) {
    use sha2::{Digest, Sha256};
    let env_path = gr_abi::env::get("SEAL_WASM_PATH").unwrap_or_default();
    let static_dir = gr_abi::env::get("STATIC_DIR")
         .or_else(|| gr_abi::env::get("STATIC_DIR"))
        .unwrap_or_default();
    let mut candidates: Vec<String> = Vec::new();
    if !env_path.trim().is_empty() {
        candidates.push(env_path);
    }
    if !static_dir.trim().is_empty() {
        let base = static_dir.trim_end_matches('/');
        candidates.push(format!("{base}/gr_seal_v2.wasm"));
        candidates.push(format!("{base}/dist/gr_seal_v2.wasm"));
        candidates.push(format!(
            "{base}/dist/v/{GR_PRODUCT_VERSION}/gr_seal_v2.wasm"
        ));
    }
    // Relative to cwd (prod: WorkingDirectory=/opt/green-v6)
    candidates.push("fe/gr_seal_v2.wasm".into());
    candidates.push("fe/dist/gr_seal_v2.wasm".into());
    candidates.push(format!("fe/dist/v/{GR_PRODUCT_VERSION}/gr_seal_v2.wasm"));
    // Compile-time workspace location for local development.
    let workspace_fe =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../probe/fe");
    candidates.push(workspace_fe.join("gr_seal_v2.wasm").display().to_string());
    candidates.push(
        workspace_fe
            .join("dist/gr_seal_v2.wasm")
            .display()
            .to_string(),
    );
    // Common install roots.
    candidates.push("/opt/green-v6/fe/gr_seal_v2.wasm".into());
    candidates.push("/opt/green-v6/fe/dist/gr_seal_v2.wasm".into());
    // next to current binary: ../fe/gr_seal_v2.wasm
    if let Ok(exe) = std::env::current_exe() {
        if let Some(bin_dir) = exe.parent() {
            if let Some(root) = bin_dir.parent() {
                candidates.push(root.join("fe/gr_seal_v2.wasm").display().to_string());
                candidates.push(root.join("fe/dist/gr_seal_v2.wasm").display().to_string());
            }
        }
    }
    // greenpng/v8 FE source files use dot names (gr.seal_v2.wasm) while the
    // wire/protocol logical name keeps the underscore form. Append dot-named
    // variants for every candidate so the integrity hash resolves either layout.
    let mut extra: Vec<String> = Vec::new();
    for p in &candidates {
        if let Some(prefix) = p.strip_suffix("gr_seal_v2.wasm") {
            extra.push(format!("{prefix}gr.seal_v2.wasm"));
        }
    }
    candidates.extend(extra);
    for p in candidates {
        if p.trim().is_empty() {
            continue;
        }
        if let Ok(bytes) = std::fs::read(&p) {
            if bytes.len() >= 8
                && bytes[0] == 0
                && bytes[1] == b'a'
                && bytes[2] == b's'
                && bytes[3] == b'm'
            {
                let mut h = Sha256::new();
                h.update(&bytes);
                return (Some(bytes.len() as u64), Some(format!("{:x}", h.finalize())));
            }
        }
    }
    (None, None)
}

/// Bootstrap/open metadata for FE.
pub fn seal_v2_public_meta() -> Value {
    let epoch = GR_PRODUCT_VERSION;
    let (wasm_bytes, wasm_sha256) = seal_wasm_file_integrity();
    // Standard C: content-hash in filename under flat /g5/dist/ (no /v/<version>/).
    let wasm_hash12 = wasm_sha256
        .as_deref()
        .map(|s| s.chars().take(12).collect::<String>())
        .filter(|s| s.len() >= 8)
        .unwrap_or_else(|| "missing00000".into());
    let wasm_name = format!("gr_seal_v2.{wasm_hash12}.wasm");
    // loader hash may differ; bootstrap overwrites with exact file hash when static_dir known.
    let loader_name = format!("gr_seal_v2_loader.{wasm_hash12}.js");
    let mut meta = json!({
        "seal_protocol": 2,
        "require_seal_v2": seal_require_v2(),
        "suite_id": current_suite_id(),
        "fe_epoch": epoch,
        "wasm_module_id": current_wasm_module_id(),
        "wasm_url": format!("/g5/dist/{wasm_name}"),
        "wasm_url_flat": format!("/g5/dist/{wasm_name}"),
        "loader_url": format!("/g5/dist/{loader_name}"),
        "loader_url_flat": format!("/g5/dist/{loader_name}"),
        "allowed_epochs": allowed_fe_epochs(),
        "b10_allowed_algo_prefixes": B10_ALLOWED_ALGO_PREFIXES,
    });
    if let Some(o) = meta.as_object_mut() {
        if let Some(n) = wasm_bytes {
            o.insert("wasm_bytes".into(), json!(n));
        }
        if let Some(h) = wasm_sha256 {
            o.insert("wasm_sha256".into(), json!(h));
        }
    }
    meta
}

/// Compute pack_set_hash (server helper / tests): SHA256 hex of joined ids.
pub fn compute_pack_set_hash(parts: &[&str]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"gr-pack-set-v2|");
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            h.update(b"|");
        }
        h.update(p.as_bytes());
    }
    to_hex(&h.finalize())
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

/// Challenge bind: SHA256 hex of challenge material.
pub fn compute_challenge_bind(session_id: &str, challenge_seed: &str, fe_epoch: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"gr-challenge-bind-v2|");
    h.update(session_id.as_bytes());
    h.update(b"|");
    h.update(challenge_seed.as_bytes());
    h.update(b"|");
    h.update(fe_epoch.as_bytes());
    to_hex(&h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn v2_roundtrip() {
        std::env::set_var("GR_SEAL_REQUIRE_V2", "1");
        std::env::set_var("GR_REQUIRE_SEALED_INGEST", "1");
        let master = b"lab-seal-secret-for-tests-only!!";
        let sid = "cycle_v2_1";
        let epoch = GR_PRODUCT_VERSION;
        let now = 1_700_000_000_000u64;
        let grant = issue_session_seal_grant_v2(master, sid, now, 60_000, epoch);
        let exp = grant["exp_ms"].as_i64().unwrap();
        let key = B64.decode(grant["key_b64"].as_str().unwrap()).unwrap();
        let payload = json!({
            "fields": {
                "cpu_loop_algo": "gr_cpu_curve_v3_multiround_median",
                "fe_impl_version": epoch,
                "a": 1
            },
            "source": "main"
        });
        let ch = compute_challenge_bind(sid, "seedabc", epoch);
        let psh = compute_pack_set_hash(&[epoch, "B10_hw_curves", "v3"]);
        let env = seal_probe_payload_v2(
            &key,
            sid,
            "vt1",
            "B10_hw_curves",
            &payload,
            exp,
            epoch,
            SEAL_SUITE_S2_AESGCM_HKDF_V1,
            SEAL_WASM_MODULE_ID,
            &ch,
            &psh,
            false,
        )
        .unwrap();
        assert_eq!(env.v, 2);
        let out = unseal_probe_payload_auto_v2(master, None, &env, now as i64 + 100).unwrap();
        assert_eq!(out["fields"]["a"], 1);
    }

    #[test]
    fn v1_rejected_when_require_v2() {
        std::env::set_var("GR_SEAL_REQUIRE_V2", "1");
        let master = b"lab-seal-secret-for-tests-only!!";
        let payload = json!({"fields": {}});
        let env = crate::seal::seal_probe_payload(master, "s", "v", "B0", &payload).unwrap();
        let err = unseal_probe_payload_auto_v2(master, None, &env, 0).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("seal_v1") || msg.contains("v2"), "{msg}");
    }

    #[test]
    fn b10_v1_algo_rejected() {
        std::env::set_var("GR_SEAL_REQUIRE_V2", "1");
        let master = b"lab-seal-secret-for-tests-only!!";
        let sid = "cycle_v2_bad";
        let epoch = GR_PRODUCT_VERSION;
        let now = 1_700_000_000_000u64;
        let grant = issue_session_seal_grant_v2(master, sid, now, 60_000, epoch);
        let exp = grant["exp_ms"].as_i64().unwrap();
        let key = B64.decode(grant["key_b64"].as_str().unwrap()).unwrap();
        let payload = json!({
            "fields": {
                "cpu_loop_algo": "gr_cpu_curve_v1_full_chunked",
                "fe_impl_version": epoch
            }
        });
        let ch = compute_challenge_bind(sid, "s", epoch);
        let psh = compute_pack_set_hash(&["x"]);
        let env = seal_probe_payload_v2(
            &key, sid, "vt", "B10_hw_curves", &payload, exp, epoch,
            SEAL_SUITE_S2_AESGCM_HKDF_V1, SEAL_WASM_MODULE_ID, &ch, &psh, false,
        )
        .unwrap();
        let err = unseal_probe_payload_auto_v2(master, None, &env, now as i64 + 1).unwrap_err();
        assert!(err.to_string().contains("b10_algo"), "{err}");
    }

    #[test]
    fn b10_v3_multiround_algo_accepted() {
        std::env::set_var("GR_SEAL_REQUIRE_V2", "1");
        std::env::set_var("GR_REQUIRE_SEALED_INGEST", "1");
        std::env::remove_var("GR_SEAL_ALLOW_B10_V2");
        let master = b"lab-seal-secret-for-tests-only!!";
        let sid = "cycle_v3_ok";
        let epoch = GR_PRODUCT_VERSION;
        let now = 1_700_000_000_000u64;
        let grant = issue_session_seal_grant_v2(master, sid, now, 60_000, epoch);
        let exp = grant["exp_ms"].as_i64().unwrap();
        let key = B64.decode(grant["key_b64"].as_str().unwrap()).unwrap();
        let payload = json!({
            "fields": {
                "cpu_loop_algo": "gr_cpu_curve_v3_multiround_median",
                "fe_impl_version": epoch,
                "cpu_timing_rounds": [[1.0, 1.1, 0.9, 1.05, 0.95, 1.02, 0.98, 1.0]]
            },
            "source": "main"
        });
        let ch = compute_challenge_bind(sid, "seedv3", epoch);
        let psh = compute_pack_set_hash(&[epoch, "B10_hw_curves", "v3"]);
        let env = seal_probe_payload_v2(
            &key,
            sid,
            "vt1",
            "B10_hw_curves",
            &payload,
            exp,
            epoch,
            SEAL_SUITE_S2_AESGCM_HKDF_V1,
            SEAL_WASM_MODULE_ID,
            &ch,
            &psh,
            false,
        )
        .unwrap();
        let out = unseal_probe_payload_auto_v2(master, None, &env, now as i64 + 100).unwrap();
        assert_eq!(
            out["fields"]["cpu_loop_algo"],
            "gr_cpu_curve_v3_multiround_median"
        );
    }

    #[test]
    fn b10_v2_algo_rejected_by_default() {
        std::env::set_var("GR_SEAL_REQUIRE_V2", "1");
        std::env::set_var("GR_REQUIRE_SEALED_INGEST", "1");
        std::env::remove_var("GR_SEAL_ALLOW_B10_V2");
        let master = b"lab-seal-secret-for-tests-only!!";
        let sid = "cycle_v2_legacy";
        let epoch = GR_PRODUCT_VERSION;
        let now = 1_700_000_000_000u64;
        let grant = issue_session_seal_grant_v2(master, sid, now, 60_000, epoch);
        let exp = grant["exp_ms"].as_i64().unwrap();
        let key = B64.decode(grant["key_b64"].as_str().unwrap()).unwrap();
        let payload = json!({
            "fields": {
                "cpu_loop_algo": "gr_cpu_curve_v2_multiworkload",
                "fe_impl_version": epoch
            }
        });
        let ch = compute_challenge_bind(sid, "s", epoch);
        let psh = compute_pack_set_hash(&["x"]);
        let env = seal_probe_payload_v2(
            &key, sid, "vt", "B10_hw_curves", &payload, exp, epoch,
            SEAL_SUITE_S2_AESGCM_HKDF_V1, SEAL_WASM_MODULE_ID, &ch, &psh, false,
        )
        .unwrap();
        let err = unseal_probe_payload_auto_v2(master, None, &env, now as i64 + 1).unwrap_err();
        assert!(err.to_string().contains("b10_algo"), "{err}");
    }
}
