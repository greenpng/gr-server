//! SSL: self-signed, PEM save, ACME HTTP-01 challenge + Let's Encrypt issue.

use crate::admin::db::AdminDb;
use crate::sni_map;
use openssl::asn1::Asn1Time;
use openssl::pkey::PKey;
use openssl::x509::X509;
use rand::RngCore;
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, SanType};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn certs_dir(db: &AdminDb, hostname: &str) -> PathBuf {
    db.data_dir.join("certs").join(hostname)
}

pub fn mint_self_signed(db: &AdminDb, domain_id: &str) -> Result<Value, String> {
    let dom = db
        .get_domain(domain_id)?
        .ok_or_else(|| "domain_not_found".to_string())?;
    let hostname = dom
        .get("hostname")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "hostname_missing".to_string())?;
    let dir = certs_dir(db, hostname);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut params = CertificateParams::new(vec![hostname.to_string()]).map_err(|e| e.to_string())?;
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(DnType::CommonName, hostname);
    params.subject_alt_names = vec![SanType::DnsName(
        hostname
            .try_into()
            .map_err(|e| format!("san: {e}"))?,
    )];
    // ~90 days
    let not_after = time::OffsetDateTime::now_utc() + time::Duration::days(90);
    params.not_after = not_after;
    let key_pair = KeyPair::generate().map_err(|e| e.to_string())?;
    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| e.to_string())?;
    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();
    let cert_path = dir.join("fullchain.pem");
    let key_path = dir.join("privkey.pem");
    std::fs::write(&cert_path, &cert_pem).map_err(|e| e.to_string())?;
    std::fs::write(&key_path, &key_pem).map_err(|e| e.to_string())?;
    let expires_ms = not_after.unix_timestamp() * 1000;
    let row = db.update_domain_ssl(
        domain_id,
        "self_signed",
        Some(expires_ms),
        &cert_path.to_string_lossy(),
        &key_path.to_string_lossy(),
    )?;
    let _ = sni_map::upsert_binding(hostname, &cert_path, &key_path);
    sync_sni_json(db)?;
    Ok(json!({
        "ok": true,
        "domain": row,
        "cert_path": cert_path,
        "key_path": key_path,
        "applied_sni": true,
    }))
}

pub fn save_pem(
    db: &AdminDb,
    domain_id: &str,
    cert_pem: &str,
    key_pem: &str,
    set_active: bool,
) -> Result<Value, String> {
    let dom = db
        .get_domain(domain_id)?
        .ok_or_else(|| "domain_not_found".to_string())?;
    let hostname = dom
        .get("hostname")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "hostname_missing".to_string())?;
    validate_pem_pair(cert_pem, key_pem)?;
    let dir = certs_dir(db, hostname);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let cert_path = dir.join("fullchain.pem");
    let key_path = dir.join("privkey.pem");
    std::fs::write(&cert_path, cert_pem).map_err(|e| e.to_string())?;
    std::fs::write(&key_path, key_pem).map_err(|e| e.to_string())?;
    let row = db.update_domain_ssl(
        domain_id,
        "uploaded",
        None,
        &cert_path.to_string_lossy(),
        &key_path.to_string_lossy(),
    )?;
    let mut applied = false;
    if set_active {
        sni_map::upsert_binding(hostname, &cert_path, &key_path)?;
        sync_sni_json(db)?;
        applied = true;
    }
    Ok(json!({
        "ok": true,
        "domain": row,
        "cert_path": cert_path,
        "key_path": key_path,
        "applied_sni": applied,
    }))
}

fn validate_pem_pair(cert_pem: &str, key_pem: &str) -> Result<(), String> {
    let cert = X509::from_pem(cert_pem.as_bytes()).map_err(|_| "invalid_certificate_pem")?;
    let key =
        PKey::private_key_from_pem(key_pem.as_bytes()).map_err(|_| "invalid_private_key_pem")?;
    if !cert
        .public_key()
        .map_err(|_| "certificate_key_unreadable")?
        .public_eq(&key)
    {
        return Err("certificate_key_mismatch".into());
    }
    let now = Asn1Time::days_from_now(0).map_err(|_| "clock_error")?;
    if cert
        .not_after()
        .compare(&now)
        .map_err(|_| "certificate_expiry_unreadable")?
        == std::cmp::Ordering::Less
    {
        return Err("certificate_expired".into());
    }
    Ok(())
}

pub fn prepare_challenge(db: &AdminDb, domain_id: &str) -> Result<Value, String> {
    let _ = db
        .get_domain(domain_id)?
        .ok_or_else(|| "domain_not_found".to_string())?;
    let mut tok = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut tok);
    let token = hex::encode(tok);
    let mut content_b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut content_b);
    let content = format!("{}.{}", hex::encode(&content_b[..16]), hex::encode(&content_b[16..]));
    db.put_acme_challenge(&token, &content, domain_id, 3_600_000)?;
    let well = format!("/.well-known/acme-challenge/{token}");
    Ok(json!({
        "ok": true,
        "token": token,
        "content": content,
        "path": well,
        "note": "HTTP-01 challenge stored; LE issue will overwrite with real key authorization",
    }))
}

pub fn sync_sni_json(db: &AdminDb) -> Result<(), String> {
    let domains = db.list_domains(None, None)?;
    let mut map = serde_json::Map::new();
    for d in domains {
        let host = d.get("hostname").and_then(|v| v.as_str()).unwrap_or("");
        let cert = d.get("cert_path").and_then(|v| v.as_str()).unwrap_or("");
        let key = d.get("key_path").and_then(|v| v.as_str()).unwrap_or("");
        if host.is_empty() || cert.is_empty() || key.is_empty() {
            continue;
        }
        if !std::path::Path::new(cert).is_file() || !std::path::Path::new(key).is_file() {
            continue;
        }
        map.insert(
            host.to_string(),
            json!({"cert": cert, "key": key}),
        );
    }
    let path = db.data_dir.join("sni-map.json");
    std::fs::write(&path, serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_default())
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Issue Let's Encrypt cert via HTTP-01 (async). Staging by default unless production=true.
pub async fn issue_letsencrypt(
    db: &AdminDb,
    domain_id: &str,
    email: &str,
    production: bool,
) -> Result<Value, String> {
    use instant_acme::{
        Account, ChallengeType, Identifier, LetsEncrypt, NewAccount, NewOrder,
    };

    let dom = db
        .get_domain(domain_id)?
        .ok_or_else(|| "domain_not_found".to_string())?;
    let hostname = dom
        .get("hostname")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "hostname_missing".to_string())?
        .to_string();

    let directory = if production {
        LetsEncrypt::Production.url()
    } else {
        LetsEncrypt::Staging.url()
    };

    let email = if email.is_empty() {
        format!("admin@{hostname}")
    } else {
        email.to_string()
    };

    let (account, _creds) = Account::create(
        &NewAccount {
            contact: &[&format!("mailto:{email}")],
            terms_of_service_agreed: true,
            only_return_existing: false,
        },
        directory,
        None,
    )
    .await
    .map_err(|e| format!("acme_account: {e}"))?;

    let identifier = Identifier::Dns(hostname.clone());
    let mut order = account
        .new_order(&NewOrder {
            identifiers: &[identifier],
        })
        .await
        .map_err(|e| format!("acme_order: {e}"))?;

    let authorizations = order
        .authorizations()
        .await
        .map_err(|e| format!("acme_authz: {e}"))?;
    let mut challenge_token = String::new();
    for authz in &authorizations {
        let challenge = authz
            .challenges
            .iter()
            .find(|c| c.r#type == ChallengeType::Http01)
            .ok_or_else(|| "no_http01_challenge".to_string())?;
        let token = challenge.token.clone();
        let key_auth = order.key_authorization(challenge).as_str().to_string();
        db.put_acme_challenge(&token, &key_auth, domain_id, 3_600_000)?;
        // Also write file for nginx/static fallbacks
        let ch_dir = db.data_dir.join("acme-challenge");
        let _ = std::fs::create_dir_all(&ch_dir);
        let _ = std::fs::write(ch_dir.join(&token), &key_auth);
        challenge_token = token;
        order
            .set_challenge_ready(&challenge.url)
            .await
            .map_err(|e| format!("acme_ready: {e}"))?;
    }

    // Poll order
    let mut tries = 0;
    loop {
        tries += 1;
        if tries > 40 {
            return Err("acme_timeout".into());
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let state = order
            .refresh()
            .await
            .map_err(|e| format!("acme_poll: {e}"))?;
        match state.status {
            instant_acme::OrderStatus::Ready => break,
            instant_acme::OrderStatus::Invalid => {
                return Err(format!("acme_invalid challenge={challenge_token}"));
            }
            _ => continue,
        }
    }

    // CSR via rcgen
    let mut params =
        CertificateParams::new(vec![hostname.clone()]).map_err(|e| e.to_string())?;
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(DnType::CommonName, hostname.clone());
    params.subject_alt_names = vec![SanType::DnsName(
        hostname
            .clone()
            .try_into()
            .map_err(|e| format!("san: {e}"))?,
    )];
    let key_pair = KeyPair::generate().map_err(|e| e.to_string())?;
    let csr_der = params
        .serialize_request(&key_pair)
        .map_err(|e| e.to_string())?
        .der()
        .to_vec();

    order
        .finalize(csr_der.as_slice())
        .await
        .map_err(|e| format!("acme_finalize: {e}"))?;

    let mut cert_pem = String::new();
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if let Some(chain) = order
            .certificate()
            .await
            .map_err(|e| format!("acme_cert: {e}"))?
        {
            cert_pem = chain;
            break;
        }
    }
    if cert_pem.is_empty() {
        return Err("acme_cert_empty".into());
    }
    let key_pem = key_pair.serialize_pem();
    let dir = certs_dir(db, &hostname);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let cert_path = dir.join("fullchain.pem");
    let key_path = dir.join("privkey.pem");
    std::fs::write(&cert_path, &cert_pem).map_err(|e| e.to_string())?;
    std::fs::write(&key_path, &key_pem).map_err(|e| e.to_string())?;

    let status = if production {
        "lets_encrypt"
    } else {
        "lets_encrypt_staging"
    };
    let expires_ms = now_ms() + 90i64 * 24 * 3600 * 1000;
    let row = db.update_domain_ssl(
        domain_id,
        status,
        Some(expires_ms),
        &cert_path.to_string_lossy(),
        &key_path.to_string_lossy(),
    )?;
    sni_map::upsert_binding(&hostname, &cert_path, &key_path)?;
    sync_sni_json(db)?;
    Ok(json!({
        "ok": true,
        "domain": row,
        "production": production,
        "cert_path": cert_path,
        "key_path": key_path,
        "applied_sni": true,
        "directory": directory,
    }))
}

#[cfg(test)]
mod tests {
    use super::{prepare_challenge, validate_pem_pair};
    use crate::admin::db::AdminDb;
    use rcgen::{CertificateParams, KeyPair};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(label: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("gr_ssl_{label}_{nonce}"))
    }

    fn certificate_pair(offset_days: i64) -> (String, String) {
        let mut params = CertificateParams::new(vec!["example.test".to_string()]).expect("params");
        let now = time::OffsetDateTime::now_utc();
        params.not_before = now - time::Duration::days(2);
        params.not_after = now + time::Duration::days(offset_days);
        let key = KeyPair::generate().expect("key");
        let cert = params.self_signed(&key).expect("certificate");
        (cert.pem(), key.serialize_pem())
    }

    #[test]
    fn rejects_invalid_and_mismatched_pem() {
        let (cert, key) = certificate_pair(30);
        assert_eq!(
            validate_pem_pair("not a certificate", &key).unwrap_err(),
            "invalid_certificate_pem"
        );
        let (_, other_key) = certificate_pair(30);
        assert_eq!(
            validate_pem_pair(&cert, &other_key).unwrap_err(),
            "certificate_key_mismatch"
        );
    }

    #[test]
    fn rejects_expired_certificate() {
        let (cert, key) = certificate_pair(-1);
        assert_eq!(
            validate_pem_pair(&cert, &key).unwrap_err(),
            "certificate_expired"
        );
    }

    #[test]
    fn accepts_valid_pair() {
        let (cert, key) = certificate_pair(30);
        assert!(validate_pem_pair(&cert, &key).is_ok());
    }

    #[test]
    fn stores_http01_challenge_for_probe_serving() {
        let root = temp_path("challenge");
        let db_path = root.join("admin.sqlite");
        let db = AdminDb::open_sqlite(&db_path, root.clone()).expect("sqlite admin db");
        db.upsert_domain(
            Some("dom_ssl_test"),
            "site_ssl_test",
            "example.test",
            "SSL test",
            true,
        )
        .expect("domain");
        let challenge = prepare_challenge(&db, "dom_ssl_test").expect("challenge");
        let token = challenge["token"].as_str().expect("token");
        let content = challenge["content"].as_str().expect("content");
        assert_eq!(challenge["path"], format!("/.well-known/acme-challenge/{token}"));
        assert_eq!(db.get_acme_challenge(token).expect("lookup").as_deref(), Some(content));
        let _ = std::fs::remove_dir_all(root);
    }
}
