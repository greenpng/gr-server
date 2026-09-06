//! Named `sdk_projection` module (iss/49 P0) — SSOT re-export of slim return surface.
//!
//! Implementation lives in [`crate::return_gate`]; this file exists so platform
//! docs and `gr doctor` can point at a stable path `sdk_projection.rs`.

pub use crate::return_gate::{
    apply_identity_return_gate, sdk_slim_projection, should_analyze_page_rpa,
    should_return_identity_to_sdk, should_return_identity_to_sdk_ex, RPA_IDLE_ANALYZE_MS,
    SDK_RETURN_IDLE_MS,
};

/// Versioned algorithm id for SDK contract.
pub const SDK_PROJECTION_ALGO: &str = "sdk_slim_projection_v1";
