//! Link-or-mint + ServerMint — system-internal commercial uniqueness.
//!
//! Demo proved: soft residual/unit **class-collides** across same-config VMs; OS instance
//! separates them. Pure FE hash alone cannot mint global-unique silicon ids.
//!
//! Layers:
//! 1. **Class binders** — residual / unit_surface / family (never L0 GPU labels)
//! 2. **score_link** — whether two observations may share an entity
//! 3. **ServerMint** — deterministic server-side id from class + OS/instance posture
//! 4. **MemoryDeviceIndex** — in-process candidate index (store adapter for tests/service)
//!
//! Soft unit collide + different `os_instance_hash` → **mint fork** (no auto-merge).
//! Real/gecko unit same + multiround stable → **link** allowed (cross-browser class).

use crate::stack_auth::stack_auth_from_fields;
use crate::trust::commercial_projection;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const LINK_OR_MINT_ALGO: &str = "link_or_mint_v1";
pub const SERVER_MINT_ALGO: &str = "server_mint_v1";
/// FE/B10 unit surface algo that may enter versioned commercial digest path.
pub const UNIT_SURFACE_ALGO_V1: &str = "gr_unit_v1";
pub const DIGEST_PATH_UNIT_V1: &str = "real_curves_unit_v1";
pub const DIGEST_PATH_SOFT_UNIT_V1: &str = "soft_aware_v4_unit_v1";
/// Honest path labels (iss/43 R4): never claim real_curves without curves/residual.
pub const DIGEST_PATH_GATEWAY: &str = "gateway_only_v1";
pub const DIGEST_PATH_THIN: &str = "thin_surface_v1";
pub const DIGEST_PATH_EMPTY: &str = "empty_anchor_v1";

/// Default link score threshold (overridable via policy / env — lab-calibrate).
pub const LINK_THRESHOLD: f64 = 0.60;

/// Runtime link threshold: `GR_LINK_THRESHOLD` env → `product_policy.json.link_threshold` → default.
pub fn link_threshold() -> f64 {
    if let Some(s) = gr_abi::env::get("LINK_THRESHOLD") {
        if let Ok(v) = s.parse::<f64>() {
            return v.clamp(0.35, 0.95);
        }
    }
    crate::policy::load_product_policy()
        .get("link_threshold")
        .and_then(|v| v.as_f64())
        .map(|v| v.clamp(0.35, 0.95))
        .unwrap_or(LINK_THRESHOLD)
}

/// Host separator present (WebRTC LAN hash or OS instance) — required for stable dh.
pub fn has_host_separator(obs: &BinderObs) -> bool {
    obs.os_instance_hash
        .as_ref()
        .is_some_and(|s| !s.is_empty())
        || obs
            .webrtc_host_ip_hash
            .as_ref()
            .is_some_and(|s| !s.is_empty())
}

/// Empty commercial anchor: residual + class body + versioned unit all absent.
/// ServerMint with rm_none|class_none|os_unknown produced the 178 constant pit (iss/43 R1).
pub fn is_empty_anchor(obs: &BinderObs) -> bool {
    let residual_empty = match obs.residual_class.as_deref() {
        None => true,
        Some(s) => s.is_empty() || s == "rm_none",
    };
    let class_empty = match obs.class_device_id.as_deref() {
        None => true,
        Some(id) => {
            let body = id
                .strip_prefix("dh_")
                .or_else(|| id.strip_prefix("dv_"))
                .or_else(|| id.strip_prefix("dg_"))
                .unwrap_or(id);
            body.is_empty() || body == "class_none"
        }
    };
    residual_empty && class_empty && !obs.unit_versioned_ok()
}

/// Thin surface: some form/env claim but no residual/unit silicon materials.
pub fn is_thin_surface(obs: &BinderObs) -> bool {
    if is_empty_anchor(obs) {
        return true;
    }
    let residual_thin = match obs.residual_class.as_deref() {
        None => true,
        Some(s) => s.is_empty() || s == "rm_none",
    };
    residual_thin && !obs.unit_versioned_ok()
}

/// Honest digest_path from actual mint materials (iss/43 R4 · iss/44 I-4).
pub fn honest_digest_path(obs: &BinderObs) -> &'static str {
    if is_empty_anchor(obs) && !has_host_separator(obs) {
        return DIGEST_PATH_EMPTY;
    }
    if obs.soft_class && obs.unit_versioned_ok() {
        return DIGEST_PATH_SOFT_UNIT_V1;
    }
    if obs.soft_class {
        return "soft_aware_v4";
    }
    if obs.unit_versioned_ok() {
        return DIGEST_PATH_UNIT_V1;
    }
    if obs
        .residual_class
        .as_ref()
        .is_some_and(|s| !s.is_empty() && s != "rm_none")
        || obs
            .class_device_id
            .as_ref()
            .is_some_and(|s| !s.is_empty())
    {
        return "real_curves_v1";
    }
    if has_host_separator(obs) {
        return DIGEST_PATH_THIN;
    }
    DIGEST_PATH_THIN
}

/// Observation binders used for link_or_mint indexing (no L0 GPU strings).
#[derive(Debug, Clone, Default)]
pub struct BinderObs {
    pub family: String,
    pub soft_class: bool,
    pub unit_surface_id: Option<String>,
    pub unit_surface_algo: Option<String>,
    pub unit_multiround_stable: bool,
    pub residual_class: Option<String>,
    pub os_instance_hash: Option<String>,
    pub os_family: Option<String>,
    pub cores_class: Option<String>,
    pub webrtc_host_ip_hash: Option<String>,
    /// Class commercial id from residual digest (pre-mint).
    pub class_device_id: Option<String>,
    /// Tenant namespace for production-lean mint pepper (never from L0 UA).
    pub tenant_id: Option<String>,
    /// Residual entropy gate — when true, residual-led mint may defer host seps (xbr).
    pub residual_entropy_ok: Option<bool>,
    /// Commercial silicon digests (from projection materials) — anti two-machine merge.
    pub hw_audio_stable: Option<String>,
    pub hw_webgl_stable: Option<String>,
}

impl BinderObs {
    pub fn unit_versioned_ok(&self) -> bool {
        self.unit_surface_id.is_some()
            && self.unit_multiround_stable
            && self
                .unit_surface_algo
                .as_deref()
                .map(|a| a == UNIT_SURFACE_ALGO_V1 || a.starts_with("gr_unit_v"))
                .unwrap_or(false)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "family": self.family,
            "soft_class": self.soft_class,
            "unit_surface_id": self.unit_surface_id,
            "unit_surface_algo": self.unit_surface_algo,
            "unit_multiround_stable": self.unit_multiround_stable,
            "residual_class": self.residual_class,
            "os_instance_hash": self.os_instance_hash,
            "os_family": self.os_family,
            "cores_class": self.cores_class,
            "webrtc_host_ip_hash": self.webrtc_host_ip_hash,
            "class_device_id": self.class_device_id,
            "tenant_id": self.tenant_id,
            "residual_entropy_ok": self.residual_entropy_ok,
            "hw_audio_stable": self.hw_audio_stable,
            "hw_webgl_stable": self.hw_webgl_stable,
        })
    }

    pub fn from_json(v: &Value) -> Self {
        let s = |k: &str| {
            v.get(k)
                .and_then(|x| x.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        };
        Self {
            family: s("family").unwrap_or_else(|| "unknown".into()),
            soft_class: v.get("soft_class").and_then(|x| x.as_bool()).unwrap_or(false),
            unit_surface_id: s("unit_surface_id"),
            unit_surface_algo: s("unit_surface_algo"),
            unit_multiround_stable: v
                .get("unit_multiround_stable")
                .and_then(|x| x.as_bool())
                .unwrap_or(false),
            residual_class: s("residual_class"),
            os_instance_hash: s("os_instance_hash"),
            os_family: s("os_family"),
            cores_class: s("cores_class"),
            webrtc_host_ip_hash: s("webrtc_host_ip_hash"),
            class_device_id: s("class_device_id"),
            tenant_id: s("tenant_id"),
            residual_entropy_ok: v
                .get("residual_entropy_ok")
                .and_then(|x| x.as_bool()),
            hw_audio_stable: s("hw_audio_stable"),
            hw_webgl_stable: s("hw_webgl_stable"),
        }
    }
}

/// Build binders from FE/server fields (+ optional commercial projection).
pub fn binder_obs_from_fields(fields: &Value) -> BinderObs {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let stack = stack_auth_from_fields(fields);
    let soft = stack.soft_stack
        || stack.residual_soft_like == Some(true)
        || crate::stack_auth::is_soft_renderer_class(&stack.renderer_class)
        || fo
            .get("residual_soft_like")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

    let cores = fo
        .get("hardware_concurrency")
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
        .map(|c| {
            if c <= 2.0 {
                "c0"
            } else if c <= 4.0 {
                "c1"
            } else if c <= 8.0 {
                "c2"
            } else {
                "c3"
            }
            .to_string()
        });

    // Family: soft vs engine residual class — not L0 GPU brand.
    // Prefer FE engine_family (B10) so WebKitGTK/Safari-UA shells map to webkit not blink.
    let family = if soft {
        "chromium_soft".into()
    } else if let Some(ef) = fo
        .get("engine_family")
        .and_then(|v| v.as_str())
        .map(|s| s.to_ascii_lowercase())
    {
        if ef.contains("gecko") {
            "gecko".into()
        } else if ef.contains("webkit") {
            "webkit".into()
        } else if ef.contains("blink") {
            "blink_or_real".into()
        } else {
            "real_residual".into()
        }
    } else if let Some(ua) = fo.get("user_agent").and_then(|v| v.as_str()) {
        let u = ua.to_ascii_lowercase();
        if u.contains("firefox") {
            "gecko".into()
        } else if u.contains("edg/") {
            // Edge is still blink silicon path for residual:real cross-link keys.
            "blink_or_real".into()
        } else if u.contains("safari/") && !u.contains("chrome/") && !u.contains("chromium") {
            "webkit".into()
        } else {
            "blink_or_real".into()
        }
    } else {
        "real_residual".into()
    };

    let proj = commercial_projection(fields);
    let class_id = proj
        .get("device_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    // residual_class for ServerMint / cross-browser link:
    // **Precision noise correction** (not blind force-merge):
    // - residual_std → 1e-6 bucket for conf / single-silicon same-engine
    // - residual_mean fine → 1e-5 (analysis / thin path)
    // - dual silicon **xbr** mean → **0.005** quanta (lab: blink/gecko rm_0.26004 vs
    //   WebKit rm_0.26100 must share mint residual class; audio digests still fork machines)
    // Distinct GPUs still separate via hw_audio_stable (primary) + optional webgl when
    // audio is absent. Commercial projection keeps fine residual_mean_bucket for analysis.
    let residual_from_std = |s: f64| -> String {
        let b = (s * 1_000_000.0).round() / 1_000_000.0;
        format!("rs_{b:.6}")
    };
    let residual_from_mean_fine = |m: f64| -> String {
        let b = (m * 100_000.0).round() / 100_000.0;
        format!("rm_{b:.5}")
    };
    // Cross-engine residual class (dual silicon). 0.005 quanta absorbs WebKit GL mean drift.
    let residual_from_mean_xbr = |m: f64| -> String {
        let b = (m / 0.005).round() * 0.005;
        format!("rm_{b:.3}")
    };
    let mean_from_fields = || -> Option<f64> {
        fo.get("residual_mean")
            .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
            .filter(|m| m.is_finite())
            .or_else(|| {
                proj.pointer("/materials/residual_mean")
                    .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
                    .filter(|m| m.is_finite())
            })
            .or_else(|| {
                proj.get("residual_mean")
                    .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
                    .filter(|m| m.is_finite())
            })
    };
    let has_dual_silicon = proj
        .pointer("/materials/hw_webgl_stable")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty())
        && proj
            .pointer("/materials/hw_audio_stable")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty());
    let residual_entropy_ok = proj
        .pointer("/materials/webgl_residual_entropy_ok")
        .and_then(|v| v.as_bool())
        .or_else(|| fo.get("webgl_residual_entropy_ok").and_then(|v| v.as_bool()))
        .unwrap_or(false);
    let residual_class = if has_dual_silicon {
        // Dual silicon: always recompute **xbr** mean bucket (do not trust fine
        // residual_mean_bucket from commercial materials — that forks WebKit).
        mean_from_fields()
            .map(residual_from_mean_xbr)
            .or_else(|| {
                // Parse fine bucket string rm_0.26004 → xbr rebucket if raw mean missing.
                proj.pointer("/materials/residual_mean_bucket")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.strip_prefix("rm_"))
                    .and_then(|n| n.parse::<f64>().ok())
                    .filter(|m| m.is_finite())
                    .map(residual_from_mean_xbr)
            })
            .or_else(|| {
                fo.get("residual_std")
                    .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
                    .filter(|s| s.is_finite() && *s > 0.0)
                    .map(residual_from_std)
            })
    } else {
        fo.get("residual_std")
            .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
            .filter(|s| s.is_finite() && *s > 0.0)
            .map(residual_from_std)
            .or_else(|| mean_from_fields().map(residual_from_mean_fine))
            .or_else(|| {
                fo.get("soft_residual_bucket")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .or_else(|| {
                proj.pointer("/materials/residual_mean_bucket")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
    };

    BinderObs {
        family,
        soft_class: soft,
        tenant_id: fo
            .get("tenant_id")
            .or_else(|| fo.get("tenant"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
        unit_surface_id: fo
            .get("unit_surface_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
        unit_surface_algo: fo
            .get("unit_surface_algo")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        unit_multiround_stable: fo
            .get("unit_multiround_stable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        residual_class,
        residual_entropy_ok: Some(residual_entropy_ok),
        hw_audio_stable: proj
            .pointer("/materials/hw_audio_stable")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .or_else(|| {
                fo.get("hw_audio_stable")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
            }),
        hw_webgl_stable: proj
            .pointer("/materials/hw_webgl_stable")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .or_else(|| {
                fo.get("hw_webgl_stable")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
            }),
        os_instance_hash: fo
            .get("os_instance_hash")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
        os_family: fo
            .get("os_family")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_ascii_lowercase()),
        cores_class: cores,
        // Prefer longer v2 host hash when FE multipath produced it (still accept v1).
        webrtc_host_ip_hash: fo
            .get("webrtc_host_ip_hash_v2")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .or_else(|| {
                fo.get("webrtc_host_ip_hash")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
            }),
        class_device_id: class_id,
    }
}

/// Index keys for candidate lookup (demo-aligned).
pub fn binder_keys(obs: &BinderObs) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(ref u) = obs.unit_surface_id {
        keys.push(format!("unit:{}:{}", obs.family, u));
        // Cross-family unit key for real gecko↔camoufox style link when stable.
        if !obs.soft_class && obs.unit_multiround_stable {
            keys.push(format!("unit:real:{}", u));
        }
    }
    if let Some(ref r) = obs.residual_class {
        keys.push(format!("residual:{}:{}", obs.family, r));
        if !obs.soft_class {
            keys.push(format!("residual:real:{}", r));
        }
    }
    if let Some(ref o) = obs.os_instance_hash {
        keys.push(format!("os:{}", o));
    }
    keys
}

/// Legacy additive link score 0..1 (kept for fallback / A-B vs composite).
pub fn score_link_legacy(existing: &BinderObs, obs: &BinderObs) -> f64 {
    let mut s = 0.0_f64;

    if let (Some(a), Some(b)) = (&existing.os_instance_hash, &obs.os_instance_hash) {
        if a == b {
            s += 0.55;
        }
    }

    if let (Some(a), Some(b)) = (&existing.unit_surface_id, &obs.unit_surface_id) {
        if a == b {
            if obs.soft_class || existing.soft_class {
                s += 0.15;
            } else {
                s += 0.35;
                if obs.unit_multiround_stable && existing.unit_multiround_stable {
                    s += 0.12;
                }
            }
        }
    }

    if let (Some(a), Some(b)) = (&existing.residual_class, &obs.residual_class) {
        if a == b {
            if obs.soft_class || existing.soft_class {
                s += 0.10;
            } else {
                s += 0.25;
            }
        }
    }

    // Same class commercial id body (residual-led) — only when residual also agrees.
    // X-11: never treat L0/UA similarity as merge; residual mismatch hard-bans link.
    let residual_agree = match (&existing.residual_class, &obs.residual_class) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    };
    let residual_disagree = match (&existing.residual_class, &obs.residual_class) {
        (Some(a), Some(b)) => a != b,
        _ => false,
    };
    if residual_disagree {
        // Distinct residual class → never auto-link (regardless of UA/L0 labels).
        s = s.min(0.15);
    } else if residual_agree {
        if let (Some(a), Some(b)) = (&existing.class_device_id, &obs.class_device_id) {
            if a == b && !obs.soft_class {
                s += 0.15;
            } else if a == b && obs.soft_class {
                s += 0.08;
            }
        }
    }

    // Cross-family without OS match: penalize hard.
    if existing.family != obs.family
        && existing.os_instance_hash.is_some()
        && obs.os_instance_hash.is_some()
        && existing.os_instance_hash != obs.os_instance_hash
    {
        s *= 0.2;
    }

    // Cross-family ladder (iss/32 · phase C): same residual + same host sep across
    // gecko↔blink real families → allow env/machine link (not soft auto-merge).
    if existing.family != obs.family
        && !existing.soft_class
        && !obs.soft_class
        && residual_agree
    {
        let host_match = match (
            existing
                .os_instance_hash
                .as_ref()
                .or(existing.webrtc_host_ip_hash.as_ref()),
            obs.os_instance_hash
                .as_ref()
                .or(obs.webrtc_host_ip_hash.as_ref()),
        ) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        };
        if host_match {
            s = (s + 0.28).min(1.0);
        } else if existing.webrtc_host_ip_hash.is_some()
            && obs.webrtc_host_ip_hash.is_some()
            && existing.webrtc_host_ip_hash == obs.webrtc_host_ip_hash
        {
            s = (s + 0.22).min(1.0);
        }
    }

    // Different tenants never link (production pepper isolation).
    if let (Some(a), Some(b)) = (&existing.tenant_id, &obs.tenant_id) {
        if a != b {
            s = 0.0;
        }
    }

    // Soft unit collide + different OS instance → force below link threshold.
    if (obs.soft_class || existing.soft_class)
        && existing.unit_surface_id.is_some()
        && existing.unit_surface_id == obs.unit_surface_id
        && existing.os_instance_hash.is_some()
        && obs.os_instance_hash.is_some()
        && existing.os_instance_hash != obs.os_instance_hash
    {
        s = s.min(0.25);
    }

    // Soft residual class collide + different OS → no auto-merge.
    if (obs.soft_class || existing.soft_class)
        && existing.residual_class.is_some()
        && existing.residual_class == obs.residual_class
        && existing.os_instance_hash.is_some()
        && obs.os_instance_hash.is_some()
        && existing.os_instance_hash != obs.os_instance_hash
    {
        s = s.min(0.25);
    }

    // Soft + different residual class → no auto-merge (distinct mint materials).
    if (obs.soft_class || existing.soft_class)
        && existing.residual_class.is_some()
        && obs.residual_class.is_some()
        && existing.residual_class != obs.residual_class
    {
        s = s.min(0.25);
    }

    // Soft + both versioned units present but differ → no auto-merge.
    if (obs.soft_class || existing.soft_class)
        && existing.unit_versioned_ok()
        && obs.unit_versioned_ok()
        && existing.unit_surface_id != obs.unit_surface_id
    {
        s = s.min(0.25);
    }

    // Completeness-adaptive gate (production):
    // - Thin observations must not auto-link (missing fields are "how we use them", not
    //   "fields are useless" — wait for richer materials instead of coarse merge).
    // - Both rich + residual disagree: keep hard ban (already applied).
    // - Dual-silicon residual is xbr-coarse (0.005); audio digests still separate machines.
    let comp = |o: &BinderObs| -> f64 {
        let mut c = 0.0;
        if o.residual_class
            .as_ref()
            .is_some_and(|r| !r.is_empty() && r != "rm_none")
        {
            c += 0.4;
        }
        if o.webrtc_host_ip_hash
            .as_ref()
            .is_some_and(|h| !h.is_empty())
        {
            c += 0.3;
        }
        if o.unit_versioned_ok() {
            c += 0.15;
        }
        if o.cores_class.as_ref().is_some_and(|x| !x.is_empty()) {
            c += 0.1;
        }
        if o.os_family.as_ref().is_some_and(|x| !x.is_empty()) {
            c += 0.05;
        }
        c
    };
    let c_ex = comp(existing);
    let c_ob = comp(obs);
    if c_ex < 0.45 || c_ob < 0.45 {
        // Incomplete: conf-only — do not auto-merge two thin devices into one id.
        s = s.min(0.34);
    }

    s.min(1.0)
}

/// Link score 0..1 — prefers `composite_association_v1` when both field maps are supplied.
/// Binder-only callers keep legacy additive scoring.
pub fn score_link(existing: &BinderObs, obs: &BinderObs) -> f64 {
    score_link_legacy(existing, obs)
}

/// Fields-aware link score: composite primary, legacy retained for diagnostics.
pub fn score_link_fields(fields_a: &Value, fields_b: &Value) -> (f64, Value) {
    let a = binder_obs_from_fields(fields_a);
    let b = binder_obs_from_fields(fields_b);
    let legacy = score_link_legacy(&a, &b);
    let composite = crate::composite_association::composite_associate(fields_a, fields_b);
    let assoc = composite
        .get("assoc_score")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let public_ok = composite
        .get("public_device_link")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let wg_disagree = composite
        .get("wg_disagree")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // Consume composite for linkability; never let legacy alone authorize public merge
    // across different commercial floors.
    let score = if wg_disagree {
        assoc.min(link_threshold() - 0.01).min(legacy)
    } else if public_ok {
        assoc.max(legacy * 0.85).min(1.0)
    } else {
        // Blend: composite leads, legacy as soft floor when materials thin.
        (assoc * 0.75 + legacy * 0.25).min(1.0)
    };
    let meta = json!({
        "algo": "score_link_composite_v1",
        "score": (score * 10000.0).round() / 10000.0,
        "legacy_score": (legacy * 10000.0).round() / 10000.0,
        "composite_association": composite,
    });
    (score, meta)
}

// Note on residual coarsening: dual-silicon xbr uses 0.005 mean quanta for residual_class
// only (measurement noise absorption). Mint body always includes **both** audio+webgl
// commercial digests when dual HW is present (residual_led_xbr_v5). Never audio-only.

fn sha_hex(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update(b"|");
    }
    format!("{:x}", h.finalize())
}

/// Deterministic ServerMint commercial id (system-internal uniqueness).
///
/// Soft: class residual/unit **plus** OS/instance separator → forks multi-VM soft collision.
/// Real: residual-led class; versioned unit surface may refine under `gr_unit_v1`.
///
/// **Empty anchor ban (iss/43 R1)**: residual+class+unit all absent → returns `""`
/// so evaluate / product must not publish a stable commercial id from constants.
pub fn server_mint_commercial_id(obs: &BinderObs) -> String {
    // P0 empty-anchor: never mint the constant pit (rm_none|class_none|os_unknown).
    if is_empty_anchor(obs) {
        return String::new();
    }

    let prefix = obs
        .class_device_id
        .as_deref()
        .and_then(|id| {
            // Multi-segment public surface (dv0-|dv4-|dv5-|dv6-) maps to commercial family.
            if crate::device_segments::is_multi_segment_id(id) {
                Some("dv")
            } else {
                crate::device_tier::commercial_family(id).map(|f| match f {
                    "multi" => "dv",
                    other => other,
                })
            }
        })
        .unwrap_or("dv");

    let residual = obs.residual_class.as_deref().unwrap_or("rm_none");
    let residual_led = residual != "rm_none" && !residual.is_empty();
    // Version gate: unversioned/unstable unit is conf-only — must not fork mint ids.
    let unit_mint = if obs.unit_versioned_ok() {
        obs.unit_surface_id.as_deref().unwrap_or("unit_none")
    } else {
        "unit_conf_only"
    };
    // Prefer WebRTC LAN host hash over os_instance for mint forking.
    // FE proxy_composite os_instance (AudioContext/baseLatency/UA-CH) is browser-noisy and
    // was forking same residual+webrtc hosts into distinct dh_ (local multi-browser lab).
    // Injected true machine-id still works when webrtc is absent.
    let os_i = obs
        .webrtc_host_ip_hash
        .as_deref()
        .or(obs.os_instance_hash.as_deref())
        .unwrap_or("os_unknown");

    let tenant = obs.tenant_id.as_deref().unwrap_or("default");
    // Cross-browser machine mint priority (lab 2026-08 multi-browser):
    // 1) residual-led xbr when residual class present — **before** unit_surface.
    //    Unit multi-seed surface is engine-noisy (Chrome may lack unit while Opera has
    //    unit_multiround_stable) and was forking same residual+webrtc into distinct dh_.
    // 2) versioned unit only when residual class is absent
    // 3) fine commercial class body as last resort
    let dig = if obs.soft_class {
        // Soft multi-VM: residual class + OS instance forks ServerMint id.
        // unit_surface only when versioned (mirrors commercial_projection gate).
        // tenant pepper namespaces commercial body across tenants (X-13 production-lean).
        sha_hex(&[
            SERVER_MINT_ALGO,
            "soft",
            tenant,
            residual,
            // Soft path still allows unit as conf material; residual remains primary.
            if residual_led {
                "unit_conf_only"
            } else {
                unit_mint
            },
            os_i,
            obs.os_family.as_deref().unwrap_or(""),
            obs.cores_class.as_deref().unwrap_or(""),
        ])
    } else if residual_led {
        // Residual-led xbr (v5): commercial body for **dual hardware**, not audio-alone.
        //
        // Product redline (2026-08):
        // - Never mint a machine-stable commercial body on a **single** hardware curve
        //   (same sound-card model across hosts would collide if residual+audio-only).
        // - Always fold **both** commercial digests when present: hw_audio_stable +
        //   hw_webgl_stable. Engine-layout webgl forks are accepted; probe calibration
        //   reduces them — we do **not** force same-host WebKit=Blink by dropping webgl.
        // - Host seps (webrtc / os_instance) remain engine-noisy; deferred under dual HW
        //   + residual_xbr / residual_entropy_ok (host is dh **gate**, not mint body).
        // - Incomplete dual HW → explicit incomplete tokens so body cannot look "full".
        let residual_xbr = obs
            .residual_class
            .as_deref()
            .is_some_and(|s| s.starts_with("rm_"));
        let audio = obs
            .hw_audio_stable
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("audio_none");
        let webgl = obs
            .hw_webgl_stable
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("webgl_none");
        let has_audio = audio != "audio_none";
        let has_webgl = webgl != "webgl_none";
        let dual_hw = has_audio && has_webgl;
        // Host defer only when dual HW is present (otherwise host may still separate thin).
        let host_for_mint = if dual_hw
            && (obs.residual_entropy_ok.unwrap_or(false) || residual_xbr)
        {
            "xbr_host_deferred"
        } else {
            os_i
        };
        // Pair completeness marker — single-HW residual_led is conf-grade uniqueness only
        // (tier layer must not promote to dh without dual HW materials).
        let pair = if dual_hw {
            "dual_hw_audio_webgl"
        } else if has_audio {
            "single_hw_audio_incomplete"
        } else if has_webgl {
            "single_hw_webgl_incomplete"
        } else {
            "no_hw_curve_digest"
        };
        sha_hex(&[
            SERVER_MINT_ALGO,
            "real_xbr",
            tenant,
            residual,
            "residual_led_xbr_v5",
            pair,
            obs.os_family.as_deref().unwrap_or(""),
            audio,
            webgl,
            host_for_mint,
        ])
    } else if obs.unit_versioned_ok() {
        // No residual class: versioned unit surface + host sep.
        sha_hex(&[
            SERVER_MINT_ALGO,
            DIGEST_PATH_UNIT_V1,
            tenant,
            residual,
            unit_mint,
            UNIT_SURFACE_ALGO_V1,
            obs.os_family.as_deref().unwrap_or(""),
            os_i,
        ])
    } else {
        // No residual, no versioned unit — fine commercial class (engine-noisy fallback).
        let class_body = obs
            .class_device_id
            .as_deref()
            .map(|id| {
                id.strip_prefix("dh_")
                    .or_else(|| id.strip_prefix("dv_"))
                    .or_else(|| id.strip_prefix("dg_"))
                    .unwrap_or(id)
            })
            .unwrap_or("class_none");
        sha_hex(&[
            SERVER_MINT_ALGO,
            "real",
            tenant,
            residual,
            class_body,
            obs.os_family.as_deref().unwrap_or(""),
            os_i,
        ])
    };

    format!("{prefix}_{}", &dig[..16])
}

/// Pure pair decision without persistent store.
pub fn link_or_mint_pair(fields_a: &Value, fields_b: &Value) -> Value {
    let a = binder_obs_from_fields(fields_a);
    let b = binder_obs_from_fields(fields_b);
    let (score, score_meta) = score_link_fields(fields_a, fields_b);
    let composite = score_meta
        .get("composite_association")
        .cloned()
        .unwrap_or(json!({}));
    let public_link = composite
        .get("public_device_link")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let id_a = server_mint_commercial_id(&a);
    let id_b = server_mint_commercial_id(&b);
    let same_id = id_a == id_b;
    let thr = link_threshold();
    // Soft association may score high across engines; only "link" when composite
    // allows public_device_link (same commercial floor) AND mint ids agree.
    let action = if score >= thr && same_id && public_link {
        "link"
    } else if score >= thr && same_id && !public_link {
        "soft_associate_no_public_link"
    } else if score >= thr && !same_id {
        // High score but mint materials diverge (should be rare) — prefer mint materials.
        "mint_materials_diverge"
    } else if a.soft_class
        && b.soft_class
        && a.unit_surface_id.is_some()
        && a.unit_surface_id == b.unit_surface_id
        && a.os_instance_hash.is_some()
        && b.os_instance_hash.is_some()
        && a.os_instance_hash != b.os_instance_hash
    {
        "mint_fork_soft_os"
    } else if !same_id {
        "mint_fork"
    } else {
        "mint_or_same_class"
    };

    let mut reasons: Vec<String> = Vec::new();
    if a.soft_class || b.soft_class {
        reasons.push("soft_class_present".into());
    }
    if a.os_instance_hash.is_some()
        && b.os_instance_hash.is_some()
        && a.os_instance_hash != b.os_instance_hash
    {
        reasons.push("os_instance_differs".into());
    }
    if a.unit_surface_id.is_some() && a.unit_surface_id == b.unit_surface_id {
        reasons.push("unit_surface_match".into());
    }
    if a.unit_versioned_ok() && b.unit_versioned_ok() {
        reasons.push("unit_surface_versioned_stable".into());
    }
    if score >= thr {
        reasons.push("score_above_link_threshold".into());
    } else {
        reasons.push("score_below_link_threshold".into());
    }

    json!({
        "algo": LINK_OR_MINT_ALGO,
        "server_mint_algo": SERVER_MINT_ALGO,
        "composite_algo": crate::composite_association::COMPOSITE_ASSOCIATION_ALGO,
        "action": action,
        "score": (score * 10000.0).round() / 10000.0,
        "legacy_score": score_meta.get("legacy_score").cloned().unwrap_or(json!(0.0)),
        "link_threshold": thr,
        "public_device_link": public_link,
        "likely_same_machine": composite.get("likely_same_machine").cloned().unwrap_or(json!(false)),
        "continuity_band": composite.get("continuity_band").cloned().unwrap_or(json!("distinct")),
        "same_server_mint_id": same_id,
        "device_id_a": id_a,
        "device_id_b": id_b,
        "soft_a": a.soft_class,
        "soft_b": b.soft_class,
        "unit_versioned_a": a.unit_versioned_ok(),
        "unit_versioned_b": b.unit_versioned_ok(),
        "reasons": reasons,
        "keys_a": binder_keys(&a),
        "keys_b": binder_keys(&b),
        "composite_association": composite,
        "score_meta": score_meta,
    })
}

/// Apply ServerMint on a single observation (evaluate path).
/// Returns commercial device_id + mint diagnostics.
///
/// Empty-anchor observations return `device_id: null` and `mint_eligible: false`
/// so callers fall back to gateway `dg_*` (independent of ServerMint body).
///
/// Multi-source mint gate: residual / host sep enter mint body only when dual-channel
/// or multi-source agree (see `multi_source_mint`). Single-source conf-only materials
/// are stripped before ServerMint so spoofed one-surface fields cannot alone mint.
pub fn apply_server_mint(fields: &Value) -> Value {
    apply_server_mint_with_evidence(fields, None)
}

/// Same as [`apply_server_mint`] with optional evidence (fields_by_source / conflicts).
///
/// Commercial body is driven by **algorithm-group selector** ([`crate::algo_groups`]):
/// priority-ordered groups with equal core-dimension strictness. Dual-HW residual,
/// single-core WebGL/audio + support, unit, FE partial, and gateway provisional paths
/// each map to a group. Soft never promotes.
pub fn apply_server_mint_with_evidence(fields: &Value, evidence: Option<&Value>) -> Value {
    // Resolve multi-source priorities first (worker/gateway over main soft; silicon conflict exclude).
    let multi_res = crate::multi_source_mint::resolve_fields_multi_source(fields, evidence);
    let fields_resolved = multi_res
        .get("resolved_fields")
        .cloned()
        .unwrap_or_else(|| fields.clone());
    let gate = crate::multi_source_mint::assess_mint_gate(&fields_resolved, evidence);
    let mut obs = binder_obs_from_fields(&fields_resolved);
    // Strip residual from mint if not dual/multi-source eligible OR silicon hard conflict.
    let silicon_poison = gate
        .get("silicon_hard_conflict")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !crate::multi_source_mint::residual_allowed_for_mint(&gate) || silicon_poison {
        obs.residual_class = None;
        obs.hw_webgl_stable = None;
        obs.hw_audio_stable = None;
        obs.unit_surface_id = None;
        obs.residual_entropy_ok = Some(false);
    }
    // Strip host sep from mint if not dual/multi-source eligible.
    if !crate::multi_source_mint::host_sep_allowed_for_mint(&gate) {
        // Keep for has_host_separator diagnostics on raw fields; mint uses obs.
        obs.webrtc_host_ip_hash = None;
        // os_instance alone is browser-noisy — only keep if dual would have allowed it.
        // When host gate fails, drop both.
        obs.os_instance_hash = None;
    }
    let class_id = obs.class_device_id.clone().unwrap_or_default();
    // Enrich fields with projection materials so algorithm groups see commercial digests.
    let residual_mint_ok = crate::multi_source_mint::residual_allowed_for_mint(&gate);
    let host_mint_ok = crate::multi_source_mint::host_sep_allowed_for_mint(&gate);
    let proj = commercial_projection(&fields_resolved);
    let mut enriched = fields_resolved.clone();
    if let Some(obj) = enriched.as_object_mut() {
        if let Some(mat) = proj.get("materials").and_then(|m| m.as_object()) {
            for k in [
                "hw_webgl_stable",
                "hw_audio_stable",
                "residual_mean",
                "residual_std",
                "form_class",
            ] {
                if !obj.contains_key(k) {
                    if let Some(v) = mat.get(k) {
                        if !v.is_null() {
                            obj.insert(k.into(), v.clone());
                        }
                    }
                }
            }
        }
        // Prefer binder-resolved digests when projection nested them only on obs.
        // Digests are commercial floors (not single-source residual/host conf-only).
        if !obj.contains_key("hw_webgl_stable") {
            if let Some(ref s) = obs.hw_webgl_stable {
                obj.insert("hw_webgl_stable".into(), json!(s));
            }
        }
        if !obj.contains_key("hw_audio_stable") {
            if let Some(ref s) = obs.hw_audio_stable {
                obj.insert("hw_audio_stable".into(), json!(s));
            }
        }
        // Multi-source mint gate must apply to **group** commercial body the same way
        // as legacy ServerMint obs: conf-only residual/host cannot enter dh recipe.
        // Silicon hard conflict poisons **all** residual/curve siblings (not just conflicted keys).
        let silicon_poison = gate
            .get("silicon_hard_conflict")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if residual_mint_ok && !silicon_poison {
            if !obj.contains_key("residual_entropy_ok") {
                if let Some(ok) = obs.residual_entropy_ok {
                    obj.insert("residual_entropy_ok".into(), json!(ok));
                }
            }
        } else {
            obj.remove("residual_mean");
            obj.remove("residual_std");
            obj.remove("residual_ok");
            obj.remove("residual_algo");
            obj.remove("residual_entropy_ok");
            obj.remove("webgl_residual_entropy_ok");
            obj.remove("hw_curve_webgl");
            obj.remove("hw_curve_audio");
            obj.remove("hw_curve_cpu");
            obj.remove("hw_webgl_stable");
            obj.remove("hw_audio_stable");
            obj.remove("unit_surface_id");
            // Strip any hw_curve_* siblings
            let curve_keys: Vec<String> = obj
                .keys()
                .filter(|k| k.starts_with("hw_curve_"))
                .cloned()
                .collect();
            for k in curve_keys {
                obj.remove(&k);
            }
            obj.insert("residual_entropy_ok".into(), json!(false));
            obj.insert("_mint_gate_residual_conf_only".into(), json!(true));
            if silicon_poison {
                obj.insert("_mint_gate_silicon_hard_conflict_poison".into(), json!(true));
            }
        }
        if !host_mint_ok {
            obj.remove("webrtc_host_ip_hash");
            obj.remove("webrtc_host_ip_hash_v2");
            obj.remove("os_instance_hash");
            obj.insert("_mint_gate_host_conf_only".into(), json!(true));
        }
    }
    let group_sel = crate::algo_groups::select_identity_group(&enriched);
    let group_json = group_sel.to_json();
    let host_sep = has_host_separator(&binder_obs_from_fields(fields));

    // Group-driven commercial id (primary). Empty/provisional without silicon still marks empty.
    let empty_group = group_sel.group_id == "G_DG_EMPTY"
        || (group_sel.provisional
            && group_sel.tier == crate::algo_groups::CompletenessTier::Dg
            && group_sel.group_id != "G_DG_GATEWAY");
    let empty = is_empty_anchor(&obs) && empty_group;
    let legacy_minted = server_mint_commercial_id(&obs);
    let group_minted = crate::algo_groups::commercial_id_from_selection(&group_sel);
    // Prefer algorithm-group body when group is silicon-eligible or gateway provisional.
    let use_group = group_sel.eligible
        && (group_sel.tier == crate::algo_groups::CompletenessTier::Dh
            || group_sel.tier == crate::algo_groups::CompletenessTier::Dv
            || group_sel.group_id == "G_DG_GATEWAY");
    let minted = if use_group && !group_minted.is_empty() {
        group_minted.clone()
    } else {
        legacy_minted.clone()
    };
    let mint_eligible = !minted.is_empty() && !(empty && group_sel.group_id == "G_DG_EMPTY");
    // Soft without host sep, OR real residual/class without host sep → collision risk
    // (iss/43 R6: residual class alone must not claim unique machine).
    let collision_risk = if empty {
        true
    } else if obs.soft_class {
        !host_sep
    } else if group_sel.tier == crate::algo_groups::CompletenessTier::Dh && !host_sep {
        // Single-core groups require host in selector; dual may defer — still flag risk.
        !matches!(
            group_sel.group_id.as_str(),
            "G_DH_DUAL_RESIDUAL" | "G_DH_CORE_WEBGL" | "G_DH_CORE_AUDIO"
        ) && !host_sep
    } else {
        !host_sep && group_sel.tier != crate::algo_groups::CompletenessTier::Dg
    };
    // Digest path honesty: prefer group recipe labels for single-core / provisional.
    let digest_path = if group_sel.provisional && group_sel.group_id == "G_DG_GATEWAY" {
        DIGEST_PATH_GATEWAY
    } else if empty || group_sel.group_id == "G_DG_EMPTY" {
        DIGEST_PATH_EMPTY
    } else if group_sel.group_id.starts_with("G_DH_") {
        "real_curves_v1"
    } else if group_sel.group_id.starts_with("G_DV_") {
        DIGEST_PATH_THIN
    } else {
        honest_digest_path(&obs)
    };
    let mut posture = vec![
        format!("server_mint={SERVER_MINT_ALGO}"),
        format!("algo_group={}", group_sel.group_id),
        format!("algo_group_recipe={}", group_sel.mint_recipe),
        format!("algo_groups={}", crate::algo_groups::ALGO_GROUPS_ALGO),
    ];
    if empty {
        posture.push("empty_anchor_no_stable_mint".into());
    }
    if group_sel.provisional {
        posture.push("provisional_gateway_or_thin".into());
    }
    if obs.soft_class {
        posture.push("soft_class_server_mint".into());
    }
    if collision_risk && obs.soft_class {
        posture.push("soft_collision_risk_no_os_separator".into());
    }
    if collision_risk && !obs.soft_class && !empty {
        posture.push("real_collision_risk_no_host_sep".into());
    }
    if host_sep {
        posture.push("host_separator_present".into());
    }
    if obs.unit_versioned_ok() {
        posture.push(format!("unit_surface_versioned={UNIT_SURFACE_ALGO_V1}"));
    } else if obs.unit_surface_id.is_some() {
        posture.push("unit_surface_conf_only_unversioned_or_unstable".into());
    }
    // Re-mint readiness: when host sep or unit arrives later, materials change → new body.
    posture.push(if host_sep {
        "remint_host_sep_armed".into()
    } else {
        "remint_await_host_sep_or_unit".into()
    });

    if !residual_mint_ok {
        posture.push("residual_single_source_or_conflict_conf_only".into());
        posture.push("algo_group_residual_stripped_from_commercial_body".into());
    } else {
        posture.push("residual_multi_source_or_dual_channel_mint_ok".into());
    }
    if !host_mint_ok {
        posture.push("host_sep_single_source_conf_only".into());
        posture.push("algo_group_host_stripped_from_commercial_body".into());
    } else {
        posture.push("host_sep_multi_source_or_dual_channel_mint_ok".into());
    }
    // Refuse dh claim when group still says dh but residual/host gates were conf-only
    // and selection only succeeded via un-gated materials (defensive).
    if group_sel.tier == crate::algo_groups::CompletenessTier::Dh
        && (!residual_mint_ok || !host_mint_ok)
        && matches!(
            group_sel.group_id.as_str(),
            "G_DH_CORE_WEBGL" | "G_DH_CORE_AUDIO" | "G_DH_CORE_RESIDUAL"
        )
    {
        // Should not happen if enrichment strip works; demote if it does.
        posture.push("algo_group_dh_demoted_mint_gate".into());
    }

    json!({
        "device_id": if mint_eligible { json!(minted) } else { Value::Null },
        "class_device_id": if class_id.is_empty() { Value::Null } else { json!(class_id) },
        "server_mint": mint_eligible,
        "mint_eligible": mint_eligible,
        "empty_anchor": empty || (group_sel.group_id == "G_DG_EMPTY"),
        "thin_surface": is_thin_surface(&obs),
        "has_host_separator": host_sep,
        "server_mint_algo": SERVER_MINT_ALGO,
        "link_or_mint_algo": LINK_OR_MINT_ALGO,
        "soft_class": obs.soft_class,
        "collision_risk": collision_risk,
        "digest_path": digest_path,
        "unit_surface_id": obs.unit_surface_id,
        "unit_surface_algo": obs.unit_surface_algo,
        "unit_versioned_ok": obs.unit_versioned_ok(),
        "os_instance_hash": obs.os_instance_hash,
        "webrtc_host_ip_hash": obs.webrtc_host_ip_hash,
        "residual_class": obs.residual_class,
        "family": obs.family,
        "binder_keys": binder_keys(&obs),
        "analysis_posture": posture,
        "multi_source_mint_gate": gate,
        "algo_group": group_json,
        "algo_group_id": group_sel.group_id,
        "algo_group_tier": group_sel.tier.as_str(),
        "provisional_gateway": group_sel.provisional
            && group_sel.tier == crate::algo_groups::CompletenessTier::Dg,
        "legacy_server_mint_id": if legacy_minted.is_empty() { Value::Null } else { json!(legacy_minted) },
        "multi_source_resolution": {
            "algo": multi_res.get("algo"),
            "summary": multi_res.get("summary"),
            "policy": multi_res.get("policy"),
            // Per-key chosen_source / conflict (trimmed in evaluate if huge)
            "resolutions": multi_res.get("resolutions"),
        },
        "uniqueness_marker": if mint_eligible {
            json!(format!("{SERVER_MINT_ALGO}:{}:{}", group_sel.group_id, minted))
        } else {
            json!(format!("{SERVER_MINT_ALGO}:empty_anchor"))
        },
    })
}

/// In-memory device index (store facade for tests / single-process service).
#[derive(Debug, Default)]
pub struct MemoryDeviceIndex {
    /// device_id → binder snapshot
    pub devices: HashMap<String, BinderObs>,
    /// device_id → fields snapshot (for composite_association_v1)
    pub device_fields: HashMap<String, Value>,
    /// binder key → device_ids
    pub index: HashMap<String, Vec<String>>,
}

impl MemoryDeviceIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Link to existing candidate or ServerMint a new commercial id; index binders.
    /// iss/58 B1: Fellegi–Sunter resolver is primary for merge/split/abstain;
    /// legacy composite score retained as soft assist only.
    pub fn link_or_mint(&mut self, fields: &Value) -> Value {
        let obs = binder_obs_from_fields(fields);
        let keys = binder_keys(&obs);
        let mut candidates: Vec<String> = Vec::new();
        for k in &keys {
            if let Some(ids) = self.index.get(k) {
                for id in ids {
                    if !candidates.contains(id) {
                        candidates.push(id.clone());
                    }
                }
            }
        }

        let mut best: Option<String> = None;
        let mut best_s = 0.0_f64;
        let mut best_public = false;
        let mut best_composite = Value::Null;
        for did in &candidates {
            if let Some(ex) = self.devices.get(did) {
                let (sc, public, comp) = if let Some(ex_fields) = self.device_fields.get(did) {
                    let (s, meta) = score_link_fields(ex_fields, fields);
                    let c = meta
                        .get("composite_association")
                        .cloned()
                        .unwrap_or(Value::Null);
                    let p = c
                        .get("public_device_link")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    (s, p, c)
                } else {
                    (score_link(ex, &obs), true, Value::Null)
                };
                if sc > best_s {
                    best_s = sc;
                    best = Some(did.clone());
                    best_public = public;
                    best_composite = comp;
                }
            }
        }

        let minted = server_mint_commercial_id(&obs);
        let empty = is_empty_anchor(&obs);

        // iss/58 B1 hot-path resolver (FS v2 + LSH/body recall + B6 promote)
        let body_key = crate::identity_governance::body_heat_key(&minted, None);
        let fs_resolve = crate::identity_governance::resolve_identity_candidates(
            fields,
            &minted,
            &body_key,
            &candidates,
            &self.device_fields,
        );
        let fs_dec = fs_resolve
            .get("decision")
            .and_then(|v| v.as_str())
            .unwrap_or("mint");
        let fs_best = fs_resolve
            .get("best_device_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let fs_score = fs_resolve
            .get("best_score")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let b6_promote = fs_resolve
            .get("b6_promote_ephemeral")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let (action, device_id) = if empty || minted.is_empty() {
            ("mint_refused_empty_anchor".to_string(), String::new())
        } else if fs_dec == "merge" {
            if let Some(ref did) = fs_best {
                // Soft OS fork still blocks public link
                if let Some(ex) = self.devices.get(did) {
                    if (obs.soft_class || ex.soft_class)
                        && ex.os_instance_hash.is_some()
                        && obs.os_instance_hash.is_some()
                        && ex.os_instance_hash != obs.os_instance_hash
                    {
                        ("mint_fork_soft_os".to_string(), minted.clone())
                    } else {
                        ("link_fs".to_string(), did.clone())
                    }
                } else {
                    ("link_fs".to_string(), did.clone())
                }
            } else {
                ("mint".to_string(), minted.clone())
            }
        } else if fs_dec == "abstain" {
            // merge_averse: mint new id rather than force-link (prefer split)
            ("mint_fs_abstain".to_string(), minted.clone())
        } else if let Some(ref did) = best {
            // Legacy fallback only when FS has no candidates
            if candidates.is_empty() && best_s >= link_threshold() && best_public {
                if let Some(ex) = self.devices.get(did) {
                    let existing_mint = server_mint_commercial_id(ex);
                    if existing_mint != minted {
                        ("mint_materials_diverge".to_string(), minted.clone())
                    } else {
                        ("link".to_string(), did.clone())
                    }
                } else {
                    ("mint".to_string(), minted.clone())
                }
            } else {
                ("mint_fs_split".to_string(), minted.clone())
            }
        } else {
            ("mint".to_string(), minted)
        };

        if !device_id.is_empty() {
            self.devices.insert(device_id.clone(), obs.clone());
            self.device_fields
                .insert(device_id.clone(), fields.clone());
            for k in keys {
                self.index.entry(k).or_default().push(device_id.clone());
            }
            let eph = device_id.starts_with("dve-");
            crate::identity_governance::catalog_register(
                &device_id,
                fields,
                &body_key,
                eph && !b6_promote,
            );
        }

        json!({
            "action": action,
            "device_id": if device_id.is_empty() { Value::Null } else { json!(device_id) },
            "score": (best_s * 10000.0).round() / 10000.0,
            "fs_score": (fs_score * 1000.0).round() / 1000.0,
            "fs_decision": fs_dec,
            "fs_resolve": fs_resolve,
            "b6_promote_ephemeral": b6_promote,
            "candidates_n": candidates.len(),
            "algo": LINK_OR_MINT_ALGO,
            "composite_algo": crate::composite_association::COMPOSITE_ASSOCIATION_ALGO,
            "server_mint_algo": SERVER_MINT_ALGO,
            "server_mint": action.starts_with("mint") && !device_id.is_empty(),
            "mint_eligible": !device_id.is_empty(),
            "empty_anchor": empty,
            "soft_class": obs.soft_class,
            "unit_versioned_ok": obs.unit_versioned_ok(),
            "public_device_link": best_public,
            "composite_association": best_composite,
            "uniqueness_marker": if device_id.is_empty() {
                format!("{SERVER_MINT_ALGO}:empty_anchor")
            } else {
                format!("{SERVER_MINT_ALGO}:{device_id}")
            },
            "binder_keys": binder_keys(&obs),
        })
    }

    pub fn device_count(&self) -> usize {
        self.devices.len()
    }
}

/// Durable file-backed DeviceIndex (iss/32 X-13 + tenant pepper).
/// JSON on disk; reopen path reloads candidates for link/fork across process sessions.
/// `tenant_id` namespaces commercial mint bodies (production-lean multi-tenant isolation).
#[derive(Debug)]
pub struct FileDeviceIndex {
    path: PathBuf,
    tenant_id: String,
    inner: MemoryDeviceIndex,
}

impl FileDeviceIndex {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        Self::open_for_tenant(path, "default")
    }

    pub fn open_for_tenant(path: impl AsRef<Path>, tenant_id: impl Into<String>) -> Result<Self, String> {
        let path = path.as_ref().to_path_buf();
        let tenant_id = tenant_id.into();
        let mut inner = MemoryDeviceIndex::new();
        if path.exists() {
            let text = fs::read_to_string(&path).map_err(|e| format!("read index: {e}"))?;
            let raw: Value =
                serde_json::from_str(&text).map_err(|e| format!("parse index: {e}"))?;
            if let Some(devs) = raw.get("devices").and_then(|v| v.as_object()) {
                for (id, obs_v) in devs {
                    inner.devices.insert(id.clone(), BinderObs::from_json(obs_v));
                }
            }
            if let Some(dfs) = raw.get("device_fields").and_then(|v| v.as_object()) {
                for (id, fv) in dfs {
                    inner.device_fields.insert(id.clone(), fv.clone());
                }
            }
            if let Some(idx) = raw.get("index").and_then(|v| v.as_object()) {
                for (k, arr) in idx {
                    let ids: Vec<String> = arr
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect();
                    inner.index.insert(k.clone(), ids);
                }
            }
        }
        Ok(Self {
            path,
            tenant_id,
            inner,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    pub fn device_count(&self) -> usize {
        self.inner.device_count()
    }

    pub fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
        }
        let mut devices = Map::new();
        for (id, obs) in &self.inner.devices {
            devices.insert(id.clone(), obs.to_json());
        }
        let mut device_fields = Map::new();
        for (id, fv) in &self.inner.device_fields {
            device_fields.insert(id.clone(), fv.clone());
        }
        let mut index = Map::new();
        for (k, ids) in &self.inner.index {
            index.insert(k.clone(), json!(ids));
        }
        let raw = json!({
            "version": "file_device_index_v1",
            "tenant_id": self.tenant_id,
            "algo": LINK_OR_MINT_ALGO,
            "server_mint_algo": SERVER_MINT_ALGO,
            "devices": devices,
            "device_fields": device_fields,
            "index": index,
        });
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(&raw).map_err(|e| e.to_string())?)
            .map_err(|e| format!("write tmp: {e}"))?;
        fs::rename(&tmp, &self.path).map_err(|e| format!("rename index: {e}"))?;
        Ok(())
    }

    /// link_or_mint then flush to disk (durable across reopen).
    /// Injects store `tenant_id` into fields when caller omitted it (pepper namespace).
    pub fn link_or_mint(&mut self, fields: &Value) -> Result<Value, String> {
        let mut fields = fields.clone();
        if let Some(obj) = fields.as_object_mut() {
            if !obj
                .get("tenant_id")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty())
            {
                obj.insert("tenant_id".into(), json!(self.tenant_id));
            }
        }
        let mut out = self.inner.link_or_mint(&fields);
        self.save()?;
        if let Some(obj) = out.as_object_mut() {
            obj.insert("durable".into(), json!(true));
            obj.insert("store_path".into(), json!(self.path.display().to_string()));
            obj.insert("tenant_id".into(), json!(self.tenant_id));
            obj.insert("tenant_pepper".into(), json!(true));
            obj.insert("device_count_after".into(), json!(self.device_count()));
        }
        Ok(out)
    }
}

/// Field-role weight snapshot for multi-algorithm fusion (SSOT roles → numeric weights).
/// Low-reliability fields get **low weight**, never absolute zero discard in product analysis.
pub fn field_algorithm_weights() -> Value {
    json!({
        "note": "Per-algorithm weights; never_digest commercial ≠ unused — reference_aux on os/br/rpa/device diag",
        "algorithms": {
            "commercial_projection": {
                "residual_mean": 0.80,
                "hw_curve_webgl": 0.85,
                "form_class": 0.70,
                "unit_surface_id": 0.45,
                "webgl_unmasked_renderer": 0.0,
                "ja3": 0.0,
                "ja4": 0.0,
                "webdriver": 0.0,
            },
            "score_os": {
                "residual_soft_like": 0.22,
                "soft_stack": 0.22,
                "vm_score": 0.35,
                "webgl_unmasked_renderer": 0.03,
                "gpu_label_vs_residual_claim": 0.16,
                "unit_surface_id": 0.04,
                "os_instance_hash": 0.20,
            },
            "score_br": {
                "gpu_label_untrusted": 0.18,
                "gpu_label_vs_residual": 0.20,
                "claim_capability_incoherence": 0.16,
                "webgl2_support": 0.16,
                "webgl_max_texture": 0.14,
                "ja4": 0.12,
                "webdriver": 0.08,
                "unit_multiround_stable": 0.06,
            },
            "score_rpa": {
                "webdriver": 0.32,
                "cdp_runtime_hint": 0.24,
                "playwright": 0.22,
                "headless_likely": 0.16,
                "soft_residual_farm_corroboration": 0.10,
                "behavior_events": 0.22,
                "residual_soft_like_alone": 0.0,
            },
            "link_or_mint": {
                "os_instance_hash": 0.55,
                "unit_surface_id_real": 0.35,
                "unit_surface_id_soft": 0.15,
                "residual_class_real": 0.25,
                "residual_class_soft": 0.10,
                "l0_gpu_label": 0.0,
            }
        }
    })
}

/// Merge class projection + ServerMint into evaluate device block helpers.
pub fn merge_mint_into_device(device: &mut Map<String, Value>, mint: &Value) {
    if let Some(id) = mint.get("device_id").and_then(|v| v.as_str()) {
        if !id.is_empty() {
            device.insert("device_id".into(), json!(id));
        }
    }
    device.insert("server_mint".into(), mint.get("server_mint").cloned().unwrap_or(json!(true)));
    device.insert(
        "server_mint_algo".into(),
        mint.get("server_mint_algo").cloned().unwrap_or(json!(SERVER_MINT_ALGO)),
    );
    device.insert(
        "link_or_mint_algo".into(),
        json!(LINK_OR_MINT_ALGO),
    );
    device.insert(
        "uniqueness_marker".into(),
        mint.get("uniqueness_marker").cloned().unwrap_or(Value::Null),
    );
    device.insert(
        "class_device_id".into(),
        mint.get("class_device_id").cloned().unwrap_or(Value::Null),
    );
    if let Some(dp) = mint.get("digest_path") {
        device.insert("digest_path".into(), dp.clone());
    }
    if mint.get("collision_risk").and_then(|v| v.as_bool()) == Some(true) {
        device.insert("collision_risk".into(), json!(true));
    }
    // Merge posture
    let mut posture: Vec<Value> = device
        .get("analysis_posture")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if let Some(arr) = mint.get("analysis_posture").and_then(|v| v.as_array()) {
        for x in arr {
            if !posture.iter().any(|p| p == x) {
                posture.push(x.clone());
            }
        }
    }
    device.insert("analysis_posture".into(), json!(posture));
    device.insert("link_or_mint".into(), mint.clone());
}
