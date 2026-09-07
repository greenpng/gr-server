//! Lab issuer for the paid multi-node load-balancer license (docs/guides/08-LB-MODULE.md).
//!
//! Signs a system-level (`site_id="__system__"`) token that enables the LB
//! module on a node, writes the token cache + pubkey into the node's data dir:
//!
//! ```sh
//! # 1) generate a lab issuer key (or export GR_LICENSE_SIGNING_KEY / GR_LICENSE_SIGNING_KEY=hex64)
//! cargo run -p gr-probe-core --example issue_lb_token -- --data-dir /opt/greenpng/data
//! #     prints the public key hex + token; keep the private key secret
//! # 2) the node verifies via <data-dir>/license_ed25519.pk (auto-written) or
//! #    export GR_LICENSE_PUBKEY_B64=<hex64 of the pubkey>  # legacy GR_* also accepted
//! ```
//!
//! Production issuance happens on the official site with the same claim shape.

use ed25519_dalek::SigningKey;
use gr_probe_core::license_token::{
    cache_token, sign_token, LicenseClaims, LicenseQuotas, LbEntitlement,
};
use std::path::PathBuf;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn main() -> Result<(), String> {
    let mut data_dir = PathBuf::from("data");
    let mut modes: Vec<String> = Vec::new();
    let mut days: i64 = 30;
    let mut ttl_hours: i64 = 24;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--data-dir" => data_dir = PathBuf::from(args.next().ok_or("--data-dir value")?),
            "--mode" => modes.push(args.next().ok_or("--mode value")?),
            "--days" => days = args.next().ok_or("--days value")?.parse().map_err(|_| "bad days")?,
            "--ttl-hours" => ttl_hours = args.next().ok_or("--ttl-hours value")?.parse().map_err(|_| "bad ttl")?,
            other => return Err(format!("unknown arg {other}")),
        }
    }
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;

    // Lab signing key: GR_LICENSE_SIGNING_KEY (legacy GR_* accepted) or derived fixed key.
    let sk_bytes: [u8; 32] = if let Some(v) = gr_abi::env::get("LICENSE_SIGNING_KEY") {
        let t = v.trim();
        if t.len() != 64 || !t.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("GR_LICENSE_SIGNING_KEY must be 64 hex chars".into());
        }
        let mut a = [0u8; 32];
        for i in 0..32 {
            a[i] = u8::from_str_radix(&t[i * 2..i * 2 + 2], 16).map_err(|_| "hex")?;
        }
        a
    } else {
        // Fixed lab-only key; do NOT reuse for production signing.
        [0x0d; 32]
    };
    let sk = SigningKey::from_bytes(&sk_bytes);
    let pk = sk.verifying_key().to_bytes();

    let iat = now_ms();
    let claims = LicenseClaims {
        v: 1,
        license_id: "lic-lab-lb".into(),
        site_id: "__system__".into(),
        domain: String::new(),
        plan: "paid".into(),
        rpa_enabled: true,
        device_precisions: vec!["dv0".into(), "dv4".into(), "dv5".into(), "dv6".into()],
        quotas: LicenseQuotas {
            sessions_per_month: None,
            sites_max: None,
            retention_days_max: None,
            nodes_max: None,
        },
        lb: Some(LbEntitlement {
            enabled: true,
            modes: if modes.is_empty() {
                vec!["proxy".into(), "redirect".into(), "internal_ip".into()]
            } else {
                modes
            },
        }),
        iat_ms: iat,
        exp_ms: iat + ttl_hours * 3600_000,
    };
    let tok = sign_token(&sk_bytes, &claims)?;
    cache_token(Some(&data_dir), &tok)?;
    std::fs::write(data_dir.join("license_ed25519.pk"), pk).map_err(|e| e.to_string())?;

    println!("pubkey_hex={}", hex::encode(pk));
    println!("token={tok}");
    println!(
        "cache=<data-dir>/license_tokens.json (key __system__); pubkey file written. Valid {}d (refresh within {days}d).",
        days
    );
    Ok(())
}
