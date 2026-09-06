//! Self-developed hardening pipeline (no commercial packers).
//!
//! **R-04 status:** `.grm` is a custom Blake3-stream XOR envelope with a trailing
//! SHA-256 digest. That is **not** AEAD: the digest is not a secret MAC and cannot
//! authenticate under key compromise / ciphertext malleation models. There is also
//! **no runtime decrypt/load path** — production loads signed plaintext `.so` only.
//!
//! Default behavior refuses to emit packages unless `--allow-insecure-xor` is set
//! (lab/legacy only). Prefer signed `.so` + Ed25519 OTA; if confidentiality is
//! required, replace this tool with a standard AEAD container and a real loader.

use clap::Parser;
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

const MAGIC: &[u8; 4] = b"GRM\0";
const VERSION: u32 = 1;

#[derive(Parser, Debug)]
#[command(
    name = "gr-harden",
    about = "DEPRECATED lab envelope: XOR stream pack (not AEAD; not used by runtime load)"
)]
struct Args {
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    output: PathBuf,
    /// Optional passphrase; random key written beside output if empty.
    #[arg(long, default_value = "")]
    passphrase: String,
    /// Required opt-in: admit that `.grm` is not production-grade AEAD.
    #[arg(long, default_value_t = false)]
    allow_insecure_xor: bool,
}

fn derive_key(pass: &[u8], salt: &[u8; 16]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"gr-harden-v1");
    h.update(salt);
    h.update(pass);
    *h.finalize().as_bytes()
}

fn stream_xor(data: &[u8], key: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut counter = 0u64;
    let mut offset = 0usize;
    let mut block = [0u8; 32];
    while offset < data.len() {
        let mut h = blake3::Hasher::new();
        h.update(key);
        h.update(&counter.to_le_bytes());
        block.copy_from_slice(h.finalize().as_bytes());
        let n = (data.len() - offset).min(32);
        for i in 0..n {
            out.push(data[offset + i] ^ block[i]);
        }
        offset += n;
        counter = counter.wrapping_add(1);
    }
    out
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    if !args.allow_insecure_xor {
        anyhow::bail!(
            "gr-harden refused: .grm is not AEAD and has no runtime load path. \
             Production uses signed .so only. Pass --allow-insecure-xor for lab/legacy experiments."
        );
    }
    eprintln!(
        "WARNING: emitting insecure XOR .grm (R-04); do not treat as supply-chain security"
    );
    let plain = std::fs::read(&args.input)?;
    let mut salt = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt);
    let pass = if args.passphrase.is_empty() {
        let mut p = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut p);
        let key_path = args.output.with_extension("grm.key");
        std::fs::write(&key_path, hex::encode(p))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600));
        }
        eprintln!("wrote random key {}", key_path.display());
        hex::encode(p)
    } else {
        args.passphrase.clone()
    };
    let key = derive_key(pass.as_bytes(), &salt);
    let cipher = stream_xor(&plain, &key);
    let mut body = Vec::new();
    body.extend_from_slice(MAGIC);
    body.extend_from_slice(&VERSION.to_le_bytes());
    body.extend_from_slice(&salt);
    body.extend_from_slice(&(cipher.len() as u64).to_le_bytes());
    body.extend_from_slice(&cipher);
    let mut hasher = Sha256::new();
    hasher.update(&body);
    let dig = hasher.finalize();
    body.extend_from_slice(&dig);
    if let Some(parent) = args.output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&args.output, &body)?;
    println!(
        "packed {} -> {} ({} bytes, sha256={})",
        args.input.display(),
        args.output.display(),
        body.len(),
        hex::encode(dig)
    );
    Ok(())
}
