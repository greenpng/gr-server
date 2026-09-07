//! greenpng probe plane — Pingora gateway + HTTP API (in-tree).
//!
//! Architecture:
//! - Cloudflare **Pingora** terminates HTTP/HTTPS
//! - Optional OpenSSL ClientHello callback → JA3/JA4 into gateway_fields
//! - Business handlers in `handlers` are framework-agnostic

pub mod admin;
pub mod dual_log;
pub mod embed_gate;
pub mod handlers;
pub mod http_util;
pub mod listen;
pub mod r100_hub;
pub mod run;
pub mod service;
pub mod sni_map;
pub mod soft_backends;
pub mod tls_fp;
pub mod upstream;
pub mod webhook_outbox;
pub mod rate_limit;
pub mod idempotency;

pub use run::{run_with, Args};
