//! SNI multi-certificate map (P-V2).
//!
//! Load via `--sni-map path.json` or env `GR_SNI_MAP`.
//!
//! ```json
//! {
//!   "example.com": { "cert": "/path/fullchain.pem", "key": "/path/privkey.pem" },
//!   "*.example.com": { "cert": "...", "key": "..." }
//! }
//! ```
//!
//! Applied inside the OpenSSL ClientHello callback. `ssl.servername()` is often
//! empty at that point, so we also parse the SNI extension (type 0) from the
//! raw ClientHello (same path as JA4).

use foreign_types::ForeignTypeRef;
use log::{debug, info, warn};
use openssl::pkey::{PKey, Private};
use openssl::ssl::{NameType, SslRef};
use openssl::x509::X509;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

#[derive(Clone)]
struct SniBinding {
    host_pattern: String,
    cert_path: PathBuf,
    #[allow(dead_code)] // retained for ops/debug of which key file backs the binding
    key_path: PathBuf,
    /// Pre-parsed leaf + intermediates
    chain: Arc<Vec<X509>>,
    key: Arc<PKey<Private>>,
}

static MAP: OnceLock<RwLock<HashMap<String, SniBinding>>> = OnceLock::new();

fn map() -> &'static RwLock<HashMap<String, SniBinding>> {
    MAP.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Load SNI map from JSON file. Empty path is no-op.
pub fn load_sni_map(path: &str) -> Result<usize, String> {
    if path.is_empty() {
        return Ok(0);
    }
    let p = Path::new(path);
    if !p.is_file() {
        return Err(format!("sni-map not found: {path}"));
    }
    let text = fs::read_to_string(p).map_err(|e| format!("read sni-map: {e}"))?;
    let raw: HashMap<String, serde_json::Value> =
        serde_json::from_str(&text).map_err(|e| format!("parse sni-map: {e}"))?;
    let mut m = HashMap::new();
    for (host, v) in raw {
        let cert = v
            .get("cert")
            .or_else(|| v.get("fullchain"))
            .and_then(|x| x.as_str())
            .ok_or_else(|| format!("host {host}: missing cert"))?;
        let key = v
            .get("key")
            .and_then(|x| x.as_str())
            .ok_or_else(|| format!("host {host}: missing key"))?;
        let binding = load_binding(&host, PathBuf::from(cert), PathBuf::from(key))?;
        m.insert(host.to_ascii_lowercase(), binding);
    }
    let n = m.len();
    *map().write().unwrap() = m;
    info!("SNI map loaded n={n} from {path}");
    Ok(n)
}

fn load_binding(host: &str, cert_path: PathBuf, key_path: PathBuf) -> Result<SniBinding, String> {
    let chain_pem =
        fs::read(&cert_path).map_err(|e| format!("host {host} read cert {:?}: {e}", cert_path))?;
    let key_pem =
        fs::read(&key_path).map_err(|e| format!("host {host} read key {:?}: {e}", key_path))?;
    let stack = X509::stack_from_pem(&chain_pem).map_err(|e| format!("host {host} parse chain: {e}"))?;
    if stack.is_empty() {
        return Err(format!("host {host}: empty chain"));
    }
    let pkey = PKey::private_key_from_pem(&key_pem)
        .map_err(|e| format!("host {host} parse key: {e}"))?;
    // Verify key matches leaf (catches PEM mixups early)
    if !stack[0]
        .public_key()
        .ok()
        .map(|pk| pk.public_eq(&pkey))
        .unwrap_or(false)
    {
        return Err(format!(
            "host {host}: cert/key mismatch ({:?} / {:?})",
            cert_path, key_path
        ));
    }
    Ok(SniBinding {
        host_pattern: host.to_ascii_lowercase(),
        cert_path,
        key_path,
        chain: Arc::new(stack),
        key: Arc::new(pkey),
    })
}

fn binding_for_host(host: &str) -> Option<SniBinding> {
    let g = map().read().ok()?;
    let h = host.to_ascii_lowercase();
    if let Some(b) = g.get(&h) {
        return Some(b.clone());
    }
    // single-label wildcard: *.example.com matches foo.example.com
    if let Some((_, rest)) = h.split_once('.') {
        let wild = format!("*.{rest}");
        if let Some(b) = g.get(&wild) {
            return Some(b.clone());
        }
    }
    None
}

/// Resolve hostname from SSL during ClientHello callback.
fn sni_from_ssl(ssl: &SslRef) -> Option<String> {
    if let Some(s) = ssl
        .servername(NameType::HOST_NAME)
        .map(|s| s.to_ascii_lowercase())
        .filter(|s| !s.is_empty())
    {
        return Some(s);
    }
    // ClientHello callback often runs before SSL_get_servername is populated.
    sni_from_client_hello_ext(ssl)
}

fn sni_from_client_hello_ext(ssl: &SslRef) -> Option<String> {
    // TLS extension type 0 = server_name
    let data = client_hello_ext_data(ssl, 0)?;
    // Prefer shared JA4 parser when available
    if let Some(s) = gr_probe_core::tls_ja4::parse_sni(&data) {
        let s = s.to_ascii_lowercase();
        if !s.is_empty() {
            return Some(s);
        }
    }
    None
}

fn client_hello_ext_data(ssl: &SslRef, type_: u32) -> Option<Vec<u8>> {
    unsafe {
        let mut ptr: *const libc::c_uchar = std::ptr::null();
        let mut len: usize = 0;
        let rc = openssl_sys::SSL_client_hello_get0_ext(ssl.as_ptr(), type_, &mut ptr, &mut len);
        if rc != 1 || ptr.is_null() || len == 0 {
            return None;
        }
        Some(std::slice::from_raw_parts(ptr, len).to_vec())
    }
}

/// Apply SNI certificate during ClientHello.
pub fn apply_sni_certificate(ssl: &mut SslRef) {
    if sni_map_len() == 0 {
        return;
    }
    let Some(host) = sni_from_ssl(ssl) else {
        debug!("sni: no hostname in ClientHello");
        return;
    };
    let Some(binding) = binding_for_host(&host) else {
        debug!("sni: no map entry for host={host}");
        return;
    };
    if let Err(e) = apply_binding(ssl, &binding, &host) {
        warn!("sni cert apply host={host}: {e}");
    }
}

fn apply_binding(ssl: &mut SslRef, b: &SniBinding, host: &str) -> Result<(), String> {
    let chain = b.chain.as_ref();
    if chain.is_empty() {
        return Err("empty chain".into());
    }
    ssl.set_certificate(&chain[0])
        .map_err(|e| format!("set_certificate: {e}"))?;
    ssl.set_private_key(b.key.as_ref())
        .map_err(|e| format!("set_private_key: {e}"))?;
    let mut intermediates = 0u32;
    for cert in chain.iter().skip(1) {
        match ssl.add_chain_cert(cert.clone()) {
            Ok(()) => intermediates += 1,
            Err(e) => debug!("sni add_chain host={host}: {e}"),
        }
    }
    info!(
        "sni cert applied host={host} pattern={} intermediates={intermediates} cert={:?}",
        b.host_pattern, b.cert_path
    );
    Ok(())
}

/// Upsert a single host binding and hot-apply into the process SNI map.
pub fn upsert_binding(host: &str, cert_path: &Path, key_path: &Path) -> Result<(), String> {
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() {
        return Err("empty_host".into());
    }
    let binding = load_binding(&host, cert_path.to_path_buf(), key_path.to_path_buf())?;
    let mut g = map().write().map_err(|e| e.to_string())?;
    g.insert(host.clone(), binding);
    info!(
        "SNI upsert host={host} cert={:?} key={:?}",
        cert_path, key_path
    );
    Ok(())
}

/// List hostnames currently in the SNI map.
pub fn sni_hosts() -> Vec<String> {
    map()
        .read()
        .map(|g| g.keys().cloned().collect())
        .unwrap_or_default()
}

pub fn sni_map_len() -> usize {
    map().read().map(|g| g.len()).unwrap_or(0)
}
