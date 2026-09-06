//! Algorithm-group identity architecture (v1).
//!
//! # Redline reinterpretation
//! Commercial placement is driven by **priority-ordered algorithm groups**, not a
//! single fixed field checklist. Each group has equal strictness on its **core
//! dimension** (collision control parity with commercial 0.001-class floors on that
//! core). Soft association never promotes into commercial ids.
//!
//! Completeness tiers `dh` / `dv` / `dg` each map to group families. A higher-tier
//! public prefix cannot be claimed without a winning group eligible for that tier.
//!
//! Field roles: `core` | `support` | `conf` | `deferred` — conf-only materials
//! never alone authorize commercial merge.

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const ALGO_GROUPS_ALGO: &str = "algo_groups_v1";
pub const FIELD_REGISTRY_ALGO: &str = "field_group_registry_v1";

/// Role of a probe field inside algorithm groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldRole {
    /// Participates in commercial bucket key when group wins.
    Core,
    /// Required corroboration for group eligibility (equal strictness bar).
    Support,
    /// Boosts confidence / os-br-rpa / soft only — never sole merge key.
    Conf,
    /// Registered but not consumed yet (explicit non-waste).
    Deferred,
}

impl FieldRole {
    pub fn as_str(self) -> &'static str {
        match self {
            FieldRole::Core => "core",
            FieldRole::Support => "support",
            FieldRole::Conf => "conf",
            FieldRole::Deferred => "deferred",
        }
    }
}

/// Target surface that consumes a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldSurface {
    Identity,
    OsScore,
    BrScore,
    RpaScore,
    SoftAssoc,
    Protocol,
}

/// One registry row: field → surfaces + default role.
#[derive(Debug, Clone)]
pub struct FieldReg {
    pub field: &'static str,
    pub role: FieldRole,
    pub surfaces: &'static [FieldSurface],
    pub pack_hint: &'static str,
    pub note: &'static str,
}

/// Static field→group registry (major probe families). New packs must register here.
pub fn field_registry() -> &'static [FieldReg] {
    &[
        // --- identity core / support ---
        FieldReg {
            field: "residual_mean",
            role: FieldRole::Core,
            surfaces: &[FieldSurface::Identity, FieldSurface::SoftAssoc],
            pack_hint: "B10_hw_curves",
            note: "primary residual anchor; xbr quanta 0.005 for class",
        },
        FieldReg {
            field: "residual_std",
            role: FieldRole::Core,
            surfaces: &[FieldSurface::Identity, FieldSurface::SoftAssoc],
            pack_hint: "B10_hw_curves",
            note: "residual dispersion",
        },
        FieldReg {
            field: "hw_curve_webgl",
            role: FieldRole::Core,
            surfaces: &[FieldSurface::Identity, FieldSurface::SoftAssoc],
            pack_hint: "B10_hw_curves",
            note: "webgl curve family; commercial floor 0.001 via hw_webgl_stable",
        },
        FieldReg {
            field: "hw_curve_audio",
            role: FieldRole::Core,
            surfaces: &[FieldSurface::Identity, FieldSurface::SoftAssoc],
            pack_hint: "B10_hw_curves",
            note: "audio curve family",
        },
        FieldReg {
            field: "hw_webgl_stable",
            role: FieldRole::Core,
            surfaces: &[FieldSurface::Identity],
            pack_hint: "projection",
            note: "commercial webgl digest wg_*",
        },
        FieldReg {
            field: "hw_audio_stable",
            role: FieldRole::Core,
            surfaces: &[FieldSurface::Identity],
            pack_hint: "projection",
            note: "commercial audio digest",
        },
        FieldReg {
            field: "webrtc_host_ip_hash",
            role: FieldRole::Support,
            surfaces: &[FieldSurface::Identity, FieldSurface::SoftAssoc, FieldSurface::OsScore],
            pack_hint: "B2/B55",
            note: "host separator; mint gate / soft host",
        },
        FieldReg {
            field: "os_instance_hash",
            role: FieldRole::Support,
            surfaces: &[FieldSurface::Identity, FieldSurface::SoftAssoc, FieldSurface::OsScore],
            pack_hint: "B10/B2",
            note: "host sep fallback (engine-noisy)",
        },
        FieldReg {
            field: "form_class",
            role: FieldRole::Support,
            surfaces: &[FieldSurface::Identity, FieldSurface::OsScore],
            pack_hint: "B0/B2",
            note: "desktop/mobile form",
        },
        FieldReg {
            field: "hardware_concurrency",
            role: FieldRole::Support,
            surfaces: &[FieldSurface::Identity, FieldSurface::OsScore, FieldSurface::BrScore],
            pack_hint: "B0/B2",
            note: "cores class support",
        },
        FieldReg {
            field: "device_memory",
            role: FieldRole::Support,
            surfaces: &[FieldSurface::OsScore, FieldSurface::Identity],
            pack_hint: "B0/B2",
            note: "RAM class",
        },
        FieldReg {
            field: "unit_surface_id",
            role: FieldRole::Core,
            surfaces: &[FieldSurface::Identity],
            pack_hint: "B10 unit",
            note: "versioned unit surface",
        },
        FieldReg {
            field: "unit_surface_algo",
            role: FieldRole::Support,
            surfaces: &[FieldSurface::Identity],
            pack_hint: "B10 unit",
            note: "must be gr_unit_v1 for mint",
        },
        FieldReg {
            field: "unit_multiround_stable",
            role: FieldRole::Support,
            surfaces: &[FieldSurface::Identity],
            pack_hint: "B10 unit",
            note: "unit multiround gate",
        },
        // --- conf for identity / scores ---
        FieldReg {
            field: "webgl_unmasked_renderer",
            role: FieldRole::Conf,
            surfaces: &[FieldSurface::BrScore, FieldSurface::OsScore],
            pack_hint: "B2",
            note: "label conf only — never commercial body",
        },
        FieldReg {
            field: "platform",
            role: FieldRole::Conf,
            surfaces: &[FieldSurface::OsScore, FieldSurface::BrScore],
            pack_hint: "B0",
            note: "platform string",
        },
        FieldReg {
            field: "timezone",
            role: FieldRole::Conf,
            surfaces: &[FieldSurface::OsScore, FieldSurface::BrScore],
            pack_hint: "B0",
            note: "tz conf",
        },
        FieldReg {
            field: "user_agent",
            role: FieldRole::Conf,
            surfaces: &[FieldSurface::BrScore, FieldSurface::RpaScore],
            pack_hint: "B0/gateway",
            note: "UA conf / bot route — not commercial key",
        },
        FieldReg {
            field: "engine_family",
            role: FieldRole::Conf,
            surfaces: &[FieldSurface::BrScore, FieldSurface::SoftAssoc],
            pack_hint: "B0",
            note: "blink/gecko/webkit conf",
        },
        FieldReg {
            field: "ja4",
            role: FieldRole::Conf,
            surfaces: &[FieldSurface::BrScore, FieldSurface::Protocol],
            pack_hint: "B8 gateway",
            note: "TLS JA4 protocol conf",
        },
        FieldReg {
            field: "ja3",
            role: FieldRole::Conf,
            surfaces: &[FieldSurface::BrScore, FieldSurface::Protocol],
            pack_hint: "B8 gateway",
            note: "TLS JA3 protocol conf",
        },
        FieldReg {
            field: "http_header_order_hash",
            role: FieldRole::Conf,
            surfaces: &[FieldSurface::BrScore, FieldSurface::Protocol],
            pack_hint: "B8 gateway",
            note: "H2/header order",
        },
        FieldReg {
            field: "protocol_engine",
            role: FieldRole::Conf,
            surfaces: &[FieldSurface::BrScore, FieldSurface::Protocol],
            pack_hint: "gateway",
            note: "edge protocol engine claim",
        },
        FieldReg {
            field: "server_client_ip",
            role: FieldRole::Conf,
            surfaces: &[FieldSurface::SoftAssoc, FieldSurface::Protocol],
            pack_hint: "gateway",
            note: "network conf only — never commercial sole key",
        },
        FieldReg {
            field: "media_device_count",
            role: FieldRole::Conf,
            surfaces: &[FieldSurface::OsScore, FieldSurface::RpaScore],
            pack_hint: "B2/B28",
            note: "media device counts",
        },
        FieldReg {
            field: "media_input_count",
            role: FieldRole::Conf,
            surfaces: &[FieldSurface::OsScore, FieldSurface::RpaScore],
            pack_hint: "B28",
            note: "input devices",
        },
        FieldReg {
            field: "screen_width",
            role: FieldRole::Conf,
            surfaces: &[FieldSurface::OsScore, FieldSurface::BrScore],
            pack_hint: "B0",
            note: "screen geometry",
        },
        FieldReg {
            field: "screen_height",
            role: FieldRole::Conf,
            surfaces: &[FieldSurface::OsScore, FieldSurface::BrScore],
            pack_hint: "B0",
            note: "screen geometry",
        },
        // --- rpa / behavior ---
        FieldReg {
            field: "B11_interaction",
            role: FieldRole::Conf,
            surfaces: &[FieldSurface::RpaScore],
            pack_hint: "B11",
            note: "interaction pack presence proxy via fields",
        },
        FieldReg {
            field: "pointer_samples",
            role: FieldRole::Conf,
            surfaces: &[FieldSurface::RpaScore],
            pack_hint: "B11",
            note: "pointer behavior",
        },
        FieldReg {
            field: "keydown_samples",
            role: FieldRole::Conf,
            surfaces: &[FieldSurface::RpaScore],
            pack_hint: "B11",
            note: "keyboard behavior",
        },
        FieldReg {
            field: "webdriver",
            role: FieldRole::Conf,
            surfaces: &[FieldSurface::RpaScore, FieldSurface::BrScore],
            pack_hint: "B12",
            note: "automation flag",
        },
        FieldReg {
            field: "automation_keys",
            role: FieldRole::Conf,
            surfaces: &[FieldSurface::RpaScore],
            pack_hint: "B12",
            note: "anti-auto surface",
        },
        // --- dense / mid deferred or conf ---
        FieldReg {
            field: "hw_curve_cpu",
            role: FieldRole::Conf,
            surfaces: &[FieldSurface::Identity, FieldSurface::OsScore],
            pack_hint: "B10x",
            note: "cpu timing conf; not sole commercial core yet",
        },
        FieldReg {
            field: "B18_webgpu",
            role: FieldRole::Deferred,
            surfaces: &[FieldSurface::Identity],
            pack_hint: "B18",
            note: "webgpu deferred commercial core",
        },
        FieldReg {
            field: "B55_webrtc_ice_deep",
            role: FieldRole::Conf,
            surfaces: &[FieldSurface::SoftAssoc, FieldSurface::OsScore],
            pack_hint: "B55",
            note: "deep ICE conf for host",
        },
        FieldReg {
            field: "B10x_silicon_ulp",
            role: FieldRole::Conf,
            surfaces: &[FieldSurface::Identity],
            pack_hint: "B10x",
            note: "silicon deepen conf",
        },
        FieldReg {
            field: "canvas_hash",
            role: FieldRole::Conf,
            surfaces: &[FieldSurface::BrScore, FieldSurface::SoftAssoc],
            pack_hint: "mid/canvas",
            note: "canvas conf",
        },
        FieldReg {
            field: "audio_sample_rate",
            role: FieldRole::Conf,
            surfaces: &[FieldSurface::OsScore, FieldSurface::BrScore],
            pack_hint: "B16",
            note: "audio context rate",
        },
    ]
}

/// Completeness tier for public commercial prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletenessTier {
    Dh,
    Dv,
    Dg,
}

impl CompletenessTier {
    pub fn as_str(self) -> &'static str {
        match self {
            CompletenessTier::Dh => "dh",
            CompletenessTier::Dv => "dv",
            CompletenessTier::Dg => "dg",
        }
    }
}

/// One algorithm group definition.
#[derive(Debug, Clone)]
pub struct AlgoGroupDef {
    pub id: &'static str,
    /// Lower = higher priority (tried first among eligible).
    pub priority: u16,
    pub tier: CompletenessTier,
    pub core_fields: &'static [&'static str],
    pub support_fields: &'static [&'static str],
    pub conf_fields: &'static [&'static str],
    pub description: &'static str,
}

/// Identity algorithm groups (priority ordered in registry; selector sorts by priority).
pub fn identity_groups() -> &'static [AlgoGroupDef] {
    &[
        // --- dh family ---
        AlgoGroupDef {
            id: "G_DH_DUAL_RESIDUAL",
            priority: 10,
            tier: CompletenessTier::Dh,
            core_fields: &["residual_mean", "hw_webgl_stable", "hw_audio_stable"],
            support_fields: &["webrtc_host_ip_hash", "os_instance_hash", "form_class"],
            conf_fields: &["hardware_concurrency", "device_memory", "engine_family"],
            description: "Dual commercial HW digests + residual (classic residual_led_xbr)",
        },
        AlgoGroupDef {
            id: "G_DH_CORE_WEBGL",
            priority: 20,
            tier: CompletenessTier::Dh,
            core_fields: &["hw_webgl_stable", "residual_mean"],
            support_fields: &[
                "webrtc_host_ip_hash",
                "os_instance_hash",
                "form_class",
                "hardware_concurrency",
            ],
            conf_fields: &["hw_audio_stable", "engine_family", "device_memory"],
            description: "Primary WebGL floor full match + residual + host/SW support",
        },
        AlgoGroupDef {
            id: "G_DH_CORE_AUDIO",
            priority: 30,
            tier: CompletenessTier::Dh,
            core_fields: &["hw_audio_stable", "residual_mean"],
            support_fields: &[
                "webrtc_host_ip_hash",
                "os_instance_hash",
                "form_class",
                "hardware_concurrency",
            ],
            conf_fields: &["hw_webgl_stable", "engine_family", "device_memory"],
            description: "Primary audio floor full match + residual + host/SW support",
        },
        AlgoGroupDef {
            id: "G_DH_CORE_RESIDUAL",
            priority: 40,
            tier: CompletenessTier::Dh,
            core_fields: &["residual_mean", "residual_std"],
            support_fields: &[
                "webrtc_host_ip_hash",
                "os_instance_hash",
                "form_class",
                "hw_curve_webgl",
            ],
            conf_fields: &["hw_webgl_stable", "hw_audio_stable", "hardware_concurrency"],
            description: "Residual-led with curve support when commercial digests partial",
        },
        AlgoGroupDef {
            id: "G_DH_UNIT",
            priority: 50,
            tier: CompletenessTier::Dh,
            core_fields: &["unit_surface_id"],
            support_fields: &["unit_surface_algo", "unit_multiround_stable", "webrtc_host_ip_hash"],
            conf_fields: &["form_class", "residual_mean"],
            description: "Versioned unit surface + host",
        },
        // --- dv family ---
        AlgoGroupDef {
            id: "G_DV_PARTIAL_HW",
            priority: 100,
            tier: CompletenessTier::Dv,
            core_fields: &["hw_curve_webgl"],
            support_fields: &["form_class", "platform"],
            conf_fields: &["residual_mean", "hardware_concurrency"],
            description: "Partial HW curves / incomplete dual — FE valuable",
        },
        AlgoGroupDef {
            id: "G_DV_PARTIAL_AUDIO",
            priority: 110,
            tier: CompletenessTier::Dv,
            core_fields: &["hw_curve_audio"],
            support_fields: &["form_class", "platform"],
            conf_fields: &["residual_mean"],
            description: "Audio-only partial surface",
        },
        AlgoGroupDef {
            id: "G_DV_FE_SURFACE",
            priority: 120,
            tier: CompletenessTier::Dv,
            core_fields: &["form_class", "platform"],
            support_fields: &["hardware_concurrency", "timezone"],
            conf_fields: &["user_agent", "screen_width"],
            description: "FE ran with env surface, thin silicon",
        },
        // --- dg family ---
        AlgoGroupDef {
            id: "G_DG_GATEWAY",
            priority: 200,
            tier: CompletenessTier::Dg,
            core_fields: &["ja4"],
            support_fields: &["user_agent", "server_client_ip"],
            conf_fields: &["ja3", "http_header_order_hash", "protocol_engine"],
            description: "Gateway/TLS protocol observation — provisional non-silicon",
        },
        AlgoGroupDef {
            id: "G_DG_EMPTY",
            priority: 250,
            tier: CompletenessTier::Dg,
            core_fields: &[],
            support_fields: &[],
            conf_fields: &["user_agent", "server_client_ip"],
            description: "Empty anchor / insufficient materials",
        },
    ]
}

/// Shared scoring algorithm groups (os / br / rpa) — never rewrite device_id.
pub fn score_groups() -> &'static [AlgoGroupDef] {
    &[
        AlgoGroupDef {
            id: "G_OS_ENV_SURFACE",
            priority: 10,
            tier: CompletenessTier::Dv,
            core_fields: &["platform", "os_family", "timezone", "hardware_concurrency"],
            support_fields: &["device_memory", "form_class", "screen_width"],
            conf_fields: &["webrtc_host_ip_hash", "media_device_count", "audio_sample_rate"],
            description: "OS environment scoring materials",
        },
        AlgoGroupDef {
            id: "G_BR_ENGINE_PROTOCOL",
            priority: 10,
            tier: CompletenessTier::Dv,
            core_fields: &["engine_family", "user_agent", "webgl_unmasked_renderer"],
            support_fields: &["ja4", "platform", "screen_width"],
            conf_fields: &["ja3", "http_header_order_hash", "canvas_hash", "protocol_engine"],
            description: "Browser/engine + protocol scoring",
        },
        AlgoGroupDef {
            id: "G_RPA_BEHAVIOR",
            priority: 10,
            tier: CompletenessTier::Dv,
            core_fields: &["webdriver", "pointer_samples", "keydown_samples"],
            support_fields: &["automation_keys", "media_input_count"],
            conf_fields: &["user_agent", "B11_interaction"],
            description: "RPA/human behavior scoring",
        },
    ]
}

fn f_has(fo: &Map<String, Value>, key: &str) -> bool {
    match fo.get(key) {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(Value::Bool(true)) => true,
        Some(Value::Number(_)) => true,
        Some(_) => false,
    }
}

fn f_str(fo: &Map<String, Value>, key: &str) -> Option<String> {
    fo.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn f_f64(fo: &Map<String, Value>, key: &str) -> Option<f64> {
    fo.get(key).and_then(|v| {
        v.as_f64()
            .or_else(|| v.as_i64().map(|i| i as f64))
            .or_else(|| v.as_u64().map(|u| u as f64))
    })
}

fn curve_len(fo: &Map<String, Value>, key: &str) -> usize {
    fo.get(key)
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0)
}

/// Materials snapshot extracted for group matching.
#[derive(Debug, Clone, Default)]
pub struct GroupMaterials {
    pub residual_mean: Option<f64>,
    pub residual_std: Option<f64>,
    pub residual_class: Option<String>,
    pub hw_webgl_stable: Option<String>,
    pub hw_audio_stable: Option<String>,
    pub webgl_curve_n: usize,
    pub audio_curve_n: usize,
    pub webrtc: Option<String>,
    pub os_instance: Option<String>,
    pub form_class: Option<String>,
    pub cores: Option<String>,
    pub unit_id: Option<String>,
    pub unit_algo: Option<String>,
    pub unit_stable: bool,
    pub ja4: Option<String>,
    pub user_agent: Option<String>,
    pub platform: Option<String>,
    pub soft_stack: bool,
    pub residual_entropy_ok: bool,
    pub gateway_only: bool,
    pub js_ran: bool,
}

impl GroupMaterials {
    pub fn from_fields(fields: &Value) -> Self {
        let fo = fields.as_object().cloned().unwrap_or_default();
        let stack = crate::stack_auth::stack_auth_from_fields(fields);
        let residual_mean = f_f64(&fo, "residual_mean");
        let residual_std = f_f64(&fo, "residual_std");
        let residual_class = residual_mean.map(|rm| {
            let q = (rm / 0.005).round() * 0.005;
            format!("rm_{q:.5}")
        });
        let webgl_curve_n = curve_len(&fo, "hw_curve_webgl");
        let audio_curve_n = curve_len(&fo, "hw_curve_audio");
        // Commercial digests may already be on fields or nested materials
        let hw_webgl_stable = f_str(&fo, "hw_webgl_stable").or_else(|| {
            fo.get("materials")
                .and_then(|m| m.get("hw_webgl_stable"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        });
        let hw_audio_stable = f_str(&fo, "hw_audio_stable").or_else(|| {
            fo.get("materials")
                .and_then(|m| m.get("hw_audio_stable"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        });
        let webrtc = f_str(&fo, "webrtc_host_ip_hash").or_else(|| f_str(&fo, "webrtc_host_ip_hash_v2"));
        let os_instance = f_str(&fo, "os_instance_hash");
        let form_class = f_str(&fo, "form_class");
        let cores = fo
            .get("hardware_concurrency")
            .map(|v| v.to_string())
            .filter(|s| s != "null");
        let unit_id = f_str(&fo, "unit_surface_id");
        let unit_algo = f_str(&fo, "unit_surface_algo");
        let unit_stable = fo
            .get("unit_multiround_stable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let ja4 = f_str(&fo, "ja4")
            .or_else(|| f_str(&fo, "tls_ja4"))
            .or_else(|| f_str(&fo, "gateway_ja4"));
        let user_agent = f_str(&fo, "user_agent");
        let platform = f_str(&fo, "platform");
        let residual_entropy_ok = fo
            .get("residual_entropy_ok")
            .or_else(|| fo.get("webgl_residual_entropy_ok"))
            .and_then(|v| v.as_bool())
            .unwrap_or_else(|| residual_mean.is_some() && residual_std.is_some_and(|s| s > 0.0));

        let early_kick = fo
            .get("early_kick")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || fo
                .get("micro_kick")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            || fo
                .get("b8_path_kind")
                .and_then(|v| v.as_str())
                .is_some();
        // Silicon / FE surface signals (not protocol-only).
        let has_silicon = residual_mean.is_some()
            || webgl_curve_n > 0
            || audio_curve_n > 0
            || unit_id.is_some()
            || hw_webgl_stable.is_some()
            || hw_audio_stable.is_some();
        let has_fe_env = platform.is_some()
            || f_has(&fo, "screen_width")
            || f_has(&fo, "hardware_concurrency")
            || f_has(&fo, "device_memory");
        // Gateway-only: early B8 / protocol surface without silicon FE materials.
        // Note: projection may inject default form_class — do not treat form alone as js_ran.
        let gateway_only = (early_kick || ja4.is_some() || f_has(&fo, "server_client_ip"))
            && !has_silicon
            && !has_fe_env;
        let js_ran = !gateway_only
            && (has_silicon
                || has_fe_env
                || (form_class.is_some() && platform.is_some()));

        Self {
            residual_mean,
            residual_std,
            residual_class,
            hw_webgl_stable,
            hw_audio_stable,
            webgl_curve_n,
            audio_curve_n,
            webrtc,
            os_instance,
            form_class,
            cores,
            unit_id,
            unit_algo,
            unit_stable,
            ja4,
            user_agent,
            platform,
            soft_stack: stack.soft_stack,
            residual_entropy_ok,
            gateway_only,
            js_ran,
        }
    }

    pub fn has_host_sep(&self) -> bool {
        self.webrtc.as_ref().is_some_and(|s| !s.is_empty())
            || self.os_instance.as_ref().is_some_and(|s| !s.is_empty())
    }

    pub fn dual_hw_digests(&self) -> bool {
        self.hw_webgl_stable.as_ref().is_some_and(|s| !s.is_empty())
            && self.hw_audio_stable.as_ref().is_some_and(|s| !s.is_empty())
    }

    pub fn webgl_core_ok(&self) -> bool {
        // Single-core commercial path requires BOTH:
        // - commercial digest (wg_* / non-empty stable floor), and
        // - curve family (≥8) so free-form digest strings alone cannot mint dh.
        let has_digest = self.hw_webgl_stable.as_ref().is_some_and(|s| {
            let t = s.trim();
            !t.is_empty()
                && t != "webgl_none"
                && !t.contains("none")
                && (t.starts_with("wg_") || t.len() >= 12)
        });
        has_digest && self.webgl_curve_n >= 8
    }

    pub fn audio_core_ok(&self) -> bool {
        let has_digest = self.hw_audio_stable.as_ref().is_some_and(|s| {
            let t = s.trim();
            !t.is_empty()
                && t != "audio_none"
                && !t.contains("none")
                && (t.starts_with("au_") || t.len() >= 12)
        });
        has_digest && self.audio_curve_n >= 16
    }

    pub fn residual_ok(&self) -> bool {
        // residual_entropy_ok must be true AND residual_mean present.
        // Callers strip residual_* when multi_source residual_mint_ok=false so conf-only
        // residual cannot authorize G_DH_* commercial body.
        self.residual_mean.is_some() && self.residual_entropy_ok
    }

    pub fn unit_ok(&self) -> bool {
        self.unit_id.is_some()
            && self.unit_stable
            && self
                .unit_algo
                .as_deref()
                .is_some_and(|a| a == "gr_unit_v1" || a.starts_with("gr_unit_v"))
    }

    pub fn support_sw_ok(&self) -> bool {
        // Equal strictness: need form OR platform, plus cores-or-memory-ish signal when available
        let form_ok = self.form_class.is_some() || self.platform.is_some();
        let env_ok = self.cores.is_some() || form_ok;
        form_ok && env_ok
    }

    pub fn host_or_soft_host_ok(&self) -> bool {
        self.has_host_sep() || (self.residual_ok() && self.support_sw_ok())
    }
}

/// Result of algorithm-group selection.
#[derive(Debug, Clone)]
pub struct GroupSelection {
    pub group_id: String,
    pub priority: u16,
    pub tier: CompletenessTier,
    pub eligible: bool,
    pub provisional: bool,
    pub reason: String,
    pub core_materials: BTreeMap<String, String>,
    pub support_materials: BTreeMap<String, String>,
    pub conf_materials: BTreeMap<String, String>,
    /// Commercial bucket body parts for ServerMint (ordered).
    pub bucket_parts: Vec<String>,
    pub mint_recipe: String,
}

impl GroupSelection {
    pub fn to_json(&self) -> Value {
        json!({
            "algo": ALGO_GROUPS_ALGO,
            "group_id": self.group_id,
            "priority": self.priority,
            "tier": self.tier.as_str(),
            "eligible": self.eligible,
            "provisional": self.provisional,
            "reason": self.reason,
            "core_materials": self.core_materials,
            "support_materials": self.support_materials,
            "conf_materials": self.conf_materials,
            "bucket_parts": self.bucket_parts,
            "mint_recipe": self.mint_recipe,
            "policy": {
                "soft_never_promote": true,
                "conf_not_sole_merge": true,
                "strictness": "equal_on_core_dimension",
            },
        })
    }
}

fn mat_put(map: &mut BTreeMap<String, String>, k: &str, v: Option<&str>) {
    if let Some(s) = v.filter(|s| !s.is_empty()) {
        map.insert(k.into(), s.to_string());
    }
}

fn group_matches(g: &AlgoGroupDef, m: &GroupMaterials) -> (bool, String) {
    if m.soft_stack && g.tier == CompletenessTier::Dh {
        // Soft stacks cannot win pure hardware dh groups (env-class only via other paths).
        if g.id.starts_with("G_DH_") && g.id != "G_DH_UNIT" {
            return (false, "soft_stack_blocks_dh_hardware_group".into());
        }
    }
    match g.id {
        "G_DH_DUAL_RESIDUAL" => {
            let ok = m.dual_hw_digests() && m.residual_ok() && m.support_sw_ok();
            // Host preferred but dual+residual_entropy may defer host into body.
            let host_ok = m.has_host_sep() || m.residual_entropy_ok;
            (
                ok && host_ok,
                if ok && host_ok {
                    "dual_hw_residual_support_ok".into()
                } else {
                    "dual_hw_or_residual_or_support_missing".into()
                },
            )
        }
        "G_DH_CORE_WEBGL" => {
            // Single primary HW family: commercial webgl digest + curves + residual
            // (mint-gate residual) + support SW + host sep (mint-gate host).
            // Free-form digest without curves is rejected by webgl_core_ok.
            let core = m.webgl_core_ok() && m.residual_ok();
            let support = m.support_sw_ok() && m.has_host_sep();
            (
                core && support,
                if core && support {
                    "core_webgl_full_match_with_support".into()
                } else {
                    "core_webgl_or_support_incomplete".into()
                },
            )
        }
        "G_DH_CORE_AUDIO" => {
            let core = m.audio_core_ok() && m.residual_ok();
            let support = m.support_sw_ok() && m.has_host_sep();
            (
                core && support,
                if core && support {
                    "core_audio_full_match_with_support".into()
                } else {
                    "core_audio_or_support_incomplete".into()
                },
            )
        }
        "G_DH_CORE_RESIDUAL" => {
            // Residual-led only when residual is mint-eligible (present after gate strip)
            // and curve family corroborates; host required (not entropy-defer alone).
            let core = m.residual_ok() && m.webgl_curve_n >= 8;
            let support = m.support_sw_ok() && m.has_host_sep();
            (
                core && support,
                if core && support {
                    "residual_core_with_curve_support".into()
                } else {
                    "residual_core_incomplete".into()
                },
            )
        }
        "G_DH_UNIT" => {
            let ok = m.unit_ok() && m.has_host_sep();
            (
                ok,
                if ok {
                    "unit_versioned_with_host".into()
                } else {
                    "unit_or_host_missing".into()
                },
            )
        }
        "G_DV_PARTIAL_HW" => {
            let ok = m.js_ran
                && !m.gateway_only
                && (m.webgl_curve_n >= 4 || m.webgl_core_ok());
            (
                ok,
                if ok {
                    "partial_webgl_fe".into()
                } else {
                    "no_partial_webgl".into()
                },
            )
        }
        "G_DV_PARTIAL_AUDIO" => {
            let ok = m.js_ran
                && !m.gateway_only
                && (m.audio_curve_n >= 8 || m.audio_core_ok());
            (
                ok,
                if ok {
                    "partial_audio_fe".into()
                } else {
                    "no_partial_audio".into()
                },
            )
        }
        "G_DV_FE_SURFACE" => {
            // Require real FE env (platform/screen/cores), not projection-default form alone.
            let ok = m.js_ran
                && !m.gateway_only
                && (m.platform.is_some()
                    || m.cores.is_some()
                    || (m.form_class.is_some() && m.platform.is_some()));
            (
                ok,
                if ok {
                    "fe_surface".into()
                } else {
                    "no_fe_surface".into()
                },
            )
        }
        "G_DG_GATEWAY" => {
            let ok = m.gateway_only
                || (!m.js_ran
                    && !m.residual_ok()
                    && m.webgl_curve_n == 0
                    && m.audio_curve_n == 0
                    && (m.ja4.is_some() || m.user_agent.is_some()));
            (
                ok,
                if ok {
                    "gateway_protocol".into()
                } else {
                    "not_gateway_only".into()
                },
            )
        }
        "G_DG_EMPTY" => (true, "fallback_empty".into()),
        _ => (false, "unknown_group".into()),
    }
}

fn build_bucket_parts(g: &AlgoGroupDef, m: &GroupMaterials) -> (Vec<String>, String) {
    let mut parts = vec![ALGO_GROUPS_ALGO.to_string(), g.id.to_string()];
    let recipe;
    match g.id {
        "G_DH_DUAL_RESIDUAL" => {
            recipe = "residual_led_dual_hw_v1".to_string();
            parts.push(recipe.clone());
            parts.push(
                m.residual_class
                    .clone()
                    .unwrap_or_else(|| "rm_none".into()),
            );
            parts.push(
                m.hw_audio_stable
                    .clone()
                    .unwrap_or_else(|| "audio_none".into()),
            );
            parts.push(
                m.hw_webgl_stable
                    .clone()
                    .unwrap_or_else(|| "webgl_none".into()),
            );
            // Host deferred under dual+entropy
            if m.residual_entropy_ok && m.dual_hw_digests() {
                parts.push("xbr_host_deferred".into());
            } else {
                parts.push(
                    m.webrtc
                        .clone()
                        .or_else(|| m.os_instance.clone())
                        .unwrap_or_else(|| "host_unknown".into()),
                );
            }
        }
        "G_DH_CORE_WEBGL" => {
            // Single-core group: webgl is THE commercial floor; audio is conf-only in body.
            recipe = "core_webgl_support_v1".to_string();
            parts.push(recipe.clone());
            parts.push(
                m.residual_class
                    .clone()
                    .unwrap_or_else(|| "rm_none".into()),
            );
            parts.push(
                m.hw_webgl_stable
                    .clone()
                    .unwrap_or_else(|| "webgl_none".into()),
            );
            parts.push(
                m.webrtc
                    .clone()
                    .or_else(|| m.os_instance.clone())
                    .unwrap_or_else(|| "host_unknown".into()),
            );
            parts.push(m.form_class.clone().unwrap_or_else(|| "form_unk".into()));
            parts.push(m.cores.clone().unwrap_or_else(|| "cores_unk".into()));
        }
        "G_DH_CORE_AUDIO" => {
            recipe = "core_audio_support_v1".to_string();
            parts.push(recipe.clone());
            parts.push(
                m.residual_class
                    .clone()
                    .unwrap_or_else(|| "rm_none".into()),
            );
            parts.push(
                m.hw_audio_stable
                    .clone()
                    .unwrap_or_else(|| "audio_none".into()),
            );
            parts.push(
                m.webrtc
                    .clone()
                    .or_else(|| m.os_instance.clone())
                    .unwrap_or_else(|| "host_unknown".into()),
            );
            parts.push(m.form_class.clone().unwrap_or_else(|| "form_unk".into()));
            parts.push(m.cores.clone().unwrap_or_else(|| "cores_unk".into()));
        }
        "G_DH_CORE_RESIDUAL" => {
            recipe = "core_residual_curve_v1".to_string();
            parts.push(recipe.clone());
            parts.push(
                m.residual_class
                    .clone()
                    .unwrap_or_else(|| "rm_none".into()),
            );
            parts.push(format!("wgl_n{}", m.webgl_curve_n));
            parts.push(
                m.webrtc
                    .clone()
                    .or_else(|| m.os_instance.clone())
                    .unwrap_or_else(|| "host_unknown".into()),
            );
            parts.push(m.form_class.clone().unwrap_or_else(|| "form_unk".into()));
        }
        "G_DH_UNIT" => {
            recipe = "unit_host_v1".to_string();
            parts.push(recipe.clone());
            parts.push(m.unit_id.clone().unwrap_or_else(|| "unit_none".into()));
            parts.push(
                m.webrtc
                    .clone()
                    .or_else(|| m.os_instance.clone())
                    .unwrap_or_else(|| "host_unknown".into()),
            );
        }
        "G_DV_PARTIAL_HW" | "G_DV_PARTIAL_AUDIO" => {
            recipe = "dv_partial_hw_v1".to_string();
            parts.push(recipe.clone());
            parts.push(format!("wgl_n{}", m.webgl_curve_n));
            parts.push(format!("aud_n{}", m.audio_curve_n));
            parts.push(m.form_class.clone().unwrap_or_else(|| "form_unk".into()));
            parts.push(m.platform.clone().unwrap_or_else(|| "plat_unk".into()));
        }
        "G_DV_FE_SURFACE" => {
            recipe = "dv_fe_surface_v1".to_string();
            parts.push(recipe.clone());
            parts.push(m.form_class.clone().unwrap_or_else(|| "form_unk".into()));
            parts.push(m.platform.clone().unwrap_or_else(|| "plat_unk".into()));
            parts.push(m.cores.clone().unwrap_or_else(|| "cores_unk".into()));
        }
        "G_DG_GATEWAY" => {
            recipe = "dg_gateway_provisional_v1".to_string();
            parts.push(recipe.clone());
            parts.push(m.ja4.clone().unwrap_or_else(|| "ja4_none".into()));
            // UA hash conf only — still provisional dg, not silicon
            parts.push(m.user_agent.clone().map(|u| {
                let mut h = Sha256::new();
                h.update(u.as_bytes());
                format!("ua_{:x}", h.finalize()).chars().take(20).collect()
            }).unwrap_or_else(|| "ua_none".into()));
        }
        _ => {
            recipe = "dg_empty_v1".to_string();
            parts.push(recipe.clone());
            parts.push("empty".into());
        }
    }
    (parts, recipe)
}

/// Select winning algorithm group for identity (priority order among eligible).
pub fn select_identity_group(fields: &Value) -> GroupSelection {
    let m = GroupMaterials::from_fields(fields);
    let mut ranked: Vec<&AlgoGroupDef> = identity_groups().iter().collect();
    ranked.sort_by_key(|g| g.priority);

    for g in ranked {
        let (ok, reason) = group_matches(g, &m);
        if !ok {
            continue;
        }
        let mut core = BTreeMap::new();
        let mut support = BTreeMap::new();
        let mut conf = BTreeMap::new();
        for k in g.core_fields {
            let v = match *k {
                "residual_mean" => m.residual_mean.map(|x| format!("{x}")),
                "residual_std" => m.residual_std.map(|x| format!("{x}")),
                "hw_webgl_stable" => m.hw_webgl_stable.clone(),
                "hw_audio_stable" => m.hw_audio_stable.clone(),
                "unit_surface_id" => m.unit_id.clone(),
                "hw_curve_webgl" => Some(format!("len{}", m.webgl_curve_n)),
                "hw_curve_audio" => Some(format!("len{}", m.audio_curve_n)),
                "form_class" => m.form_class.clone(),
                "platform" => m.platform.clone(),
                "ja4" => m.ja4.clone(),
                _ => None,
            };
            mat_put(&mut core, k, v.as_deref());
        }
        for k in g.support_fields {
            let v = match *k {
                "webrtc_host_ip_hash" => m.webrtc.clone(),
                "os_instance_hash" => m.os_instance.clone(),
                "form_class" => m.form_class.clone(),
                "hardware_concurrency" => m.cores.clone(),
                "platform" => m.platform.clone(),
                "user_agent" => m.user_agent.clone().map(|_| "ua_present".into()),
                "server_client_ip" => Some("ip_conf".into()),
                "unit_surface_algo" => m.unit_algo.clone(),
                "unit_multiround_stable" => Some(m.unit_stable.to_string()),
                "timezone" => Some("tz".into()),
                "hw_curve_webgl" => Some(format!("len{}", m.webgl_curve_n)),
                _ => None,
            };
            mat_put(&mut support, k, v.as_deref());
        }
        for k in g.conf_fields {
            conf.insert((*k).into(), "present_or_absent".into());
        }
        let (bucket_parts, mint_recipe) = build_bucket_parts(g, &m);
        let provisional = g.tier == CompletenessTier::Dg
            || g.id == "G_DG_GATEWAY"
            || g.id == "G_DG_EMPTY";
        return GroupSelection {
            group_id: g.id.into(),
            priority: g.priority,
            tier: g.tier,
            eligible: true,
            provisional,
            reason,
            core_materials: core,
            support_materials: support,
            conf_materials: conf,
            bucket_parts,
            mint_recipe,
        };
    }
    // Absolute fallback
    GroupSelection {
        group_id: "G_DG_EMPTY".into(),
        priority: 250,
        tier: CompletenessTier::Dg,
        eligible: true,
        provisional: true,
        reason: "no_group_matched".into(),
        core_materials: BTreeMap::new(),
        support_materials: BTreeMap::new(),
        conf_materials: BTreeMap::new(),
        bucket_parts: vec![
            ALGO_GROUPS_ALGO.into(),
            "G_DG_EMPTY".into(),
            "dg_empty_v1".into(),
            "empty".into(),
        ],
        mint_recipe: "dg_empty_v1".into(),
    }
}

/// Hash bucket parts into commercial body hex (no prefix).
pub fn bucket_body_hex(parts: &[String]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update(b"|");
    }
    format!("{:x}", h.finalize())
}

/// Commercial id from group selection (with tier prefix).
pub fn commercial_id_from_selection(sel: &GroupSelection) -> String {
    if sel.group_id == "G_DG_EMPTY" && sel.bucket_parts.iter().any(|p| p == "empty") {
        // empty commercial: caller may still emit dg from IP path
    }
    let body = bucket_body_hex(&sel.bucket_parts);
    let short = &body[..16.min(body.len())];
    format!("{}_{}", sel.tier.as_str(), short)
}

/// Whether two field maps land in the same commercial bucket under group selection.
pub fn same_commercial_bucket(fields_a: &Value, fields_b: &Value) -> Value {
    let sa = select_identity_group(fields_a);
    let sb = select_identity_group(fields_b);
    let id_a = commercial_id_from_selection(&sa);
    let id_b = commercial_id_from_selection(&sb);
    let same = sa.group_id == sb.group_id
        && sa.bucket_parts == sb.bucket_parts
        && id_a == id_b
        && !sa.provisional;
    json!({
        "algo": ALGO_GROUPS_ALGO,
        "same_bucket": same,
        "a": {
            "group_id": sa.group_id,
            "tier": sa.tier.as_str(),
            "device_id": id_a,
            "mint_recipe": sa.mint_recipe,
            "provisional": sa.provisional,
        },
        "b": {
            "group_id": sb.group_id,
            "tier": sb.tier.as_str(),
            "device_id": id_b,
            "mint_recipe": sb.mint_recipe,
            "provisional": sb.provisional,
        },
    })
}

/// Field registry as JSON for docs/ops.
pub fn field_registry_json() -> Value {
    let rows: Vec<Value> = field_registry()
        .iter()
        .map(|r| {
            json!({
                "field": r.field,
                "role": r.role.as_str(),
                "surfaces": r.surfaces.iter().map(|s| match s {
                    FieldSurface::Identity => "identity",
                    FieldSurface::OsScore => "os",
                    FieldSurface::BrScore => "br",
                    FieldSurface::RpaScore => "rpa",
                    FieldSurface::SoftAssoc => "soft",
                    FieldSurface::Protocol => "protocol",
                }).collect::<Vec<_>>(),
                "pack_hint": r.pack_hint,
                "note": r.note,
            })
        })
        .collect();
    json!({
        "algo": FIELD_REGISTRY_ALGO,
        "fields": rows,
        "identity_groups": identity_groups().iter().map(|g| json!({
            "id": g.id,
            "priority": g.priority,
            "tier": g.tier.as_str(),
            "core": g.core_fields,
            "support": g.support_fields,
            "conf": g.conf_fields,
            "description": g.description,
        })).collect::<Vec<_>>(),
        "score_groups": score_groups().iter().map(|g| json!({
            "id": g.id,
            "priority": g.priority,
            "core": g.core_fields,
            "support": g.support_fields,
            "conf": g.conf_fields,
            "description": g.description,
        })).collect::<Vec<_>>(),
    })
}

/// Probe materials for shared score groups (os/br/rpa) — does not touch device_id.
pub fn score_materials_boost(fields: &Value) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let mut os_hits = Vec::new();
    let mut br_hits = Vec::new();
    let mut rpa_hits = Vec::new();
    for g in score_groups() {
        for k in g.core_fields.iter().chain(g.support_fields.iter()).chain(g.conf_fields.iter()) {
            if f_has(&fo, k) {
                if g.id.starts_with("G_OS") {
                    os_hits.push(*k);
                } else if g.id.starts_with("G_BR") {
                    br_hits.push(*k);
                } else if g.id.starts_with("G_RPA") {
                    rpa_hits.push(*k);
                }
            }
        }
    }
    // Also scan registry conf fields tagged for scores
    for r in field_registry() {
        if !f_has(&fo, r.field) {
            continue;
        }
        for s in r.surfaces {
            match s {
                FieldSurface::OsScore => {
                    if !os_hits.contains(&r.field) {
                        os_hits.push(r.field);
                    }
                }
                FieldSurface::BrScore => {
                    if !br_hits.contains(&r.field) {
                        br_hits.push(r.field);
                    }
                }
                FieldSurface::RpaScore => {
                    if !rpa_hits.contains(&r.field) {
                        rpa_hits.push(r.field);
                    }
                }
                _ => {}
            }
        }
    }
    // Numeric boosts: more relevant materials → higher conf contribution (0..0.12)
    let os_boost = (os_hits.len() as f64 * 0.012).min(0.12);
    let br_boost = (br_hits.len() as f64 * 0.012).min(0.12);
    let rpa_boost = (rpa_hits.len() as f64 * 0.015).min(0.12);
    json!({
        "algo": "score_materials_boost_v1",
        "os": { "fields": os_hits, "boost": os_boost },
        "br": { "fields": br_hits, "boost": br_boost },
        "rpa": { "fields": rpa_hits, "boost": rpa_boost },
        "policy": "boost_only_never_rewrite_device_id",
    })
}

/// Prefer silicon-complete analysis over latest B8-only for VT display.
///
/// `candidates`: list of objects with at least
/// `session_id`, `device_tier`, `digest_path`, `created_ms` (or `updated_ms`),
/// optional `batches_n`, `mint_silicon_ok`, `provisional_gateway`.
pub fn select_vt_best_silicon(candidates: &[Value]) -> Value {
    if candidates.is_empty() {
        return json!({
            "algo": "vt_best_silicon_v1",
            "selected": null,
            "reason": "no_candidates",
        });
    }
    let score = |v: &Value| -> i64 {
        let tier = v
            .get("device_tier")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let digest = v
            .get("digest_path")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let provisional = v
            .get("provisional_gateway")
            .and_then(|x| x.as_bool())
            .unwrap_or(false)
            || digest.contains("empty_anchor")
            || digest.contains("gateway_only");
        let silicon = v
            .get("mint_silicon_ok")
            .and_then(|x| x.as_bool())
            .unwrap_or(false)
            || digest.contains("real_curves");
        let batches = v
            .get("batches_n")
            .and_then(|x| x.as_i64())
            .unwrap_or(0);
        let mut s = 0i64;
        if tier == "dh" {
            s += 1_000_000;
        } else if tier == "dv" {
            s += 100_000;
        } else if tier == "dg" {
            s += 1_000;
        }
        if silicon {
            s += 500_000;
        }
        if provisional {
            s -= 400_000;
        }
        if digest.contains("empty_anchor") {
            s -= 200_000;
        }
        s += batches * 100;
        let ts = v
            .get("created_ms")
            .or_else(|| v.get("updated_ms"))
            .and_then(|x| x.as_i64())
            .unwrap_or(0);
        // slight recency among equals
        s += (ts / 1_000_000).min(999);
        s
    };
    let best = candidates
        .iter()
        .max_by_key(|v| score(v))
        .cloned()
        .unwrap();
    json!({
        "algo": "vt_best_silicon_v1",
        "selected": best,
        "reason": "prefer_dh_silicon_over_latest_b8",
        "policy": "ops_display_not_mint",
    })
}

/// Schedule hint when host agrees but commercial WG floors disagree.
pub fn b10x_schedule_hint(fields_a: &Value, fields_b: Option<&Value>) -> Value {
    let ma = GroupMaterials::from_fields(fields_a);
    let host_ok = ma.has_host_sep();
    let mut wg_disagree = false;
    let mut host_agree = host_ok;
    if let Some(fb) = fields_b {
        let mb = GroupMaterials::from_fields(fb);
        match (&ma.hw_webgl_stable, &mb.hw_webgl_stable) {
            (Some(a), Some(b)) if a != b && !a.is_empty() && !b.is_empty() => {
                wg_disagree = true;
            }
            _ => {}
        }
        host_agree = match (&ma.webrtc, &mb.webrtc) {
            (Some(a), Some(b)) => a == b,
            _ => ma.has_host_sep() && mb.has_host_sep(),
        };
    } else {
        // self: near_host style — residual present but single engine; still schedule deepen
        wg_disagree = ma.webgl_core_ok() && ma.residual_ok() && !ma.dual_hw_digests();
    }
    let schedule = host_agree && wg_disagree;
    json!({
        "algo": "b10x_schedule_hint_v1",
        "schedule_b10x": schedule,
        "reason": if schedule {
            "wg_disagree_host_agree_deepen_for_floor_align"
        } else {
            "no_b10x_hint"
        },
        "host_agree": host_agree,
        "wg_disagree": wg_disagree,
        "packs": if schedule {
            json!(["B10x_silicon_ulp", "B10x_silicon_rint", "B10x_angle_crosscheck"])
        } else {
            json!([])
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dual_fields() -> Value {
        json!({
            "residual_mean": 0.26,
            "residual_std": 0.09,
            "residual_entropy_ok": true,
            "hw_webgl_stable": "wg_aaa111",
            "hw_audio_stable": "au_bbb222",
            "hw_curve_webgl": [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8],
            "hw_curve_audio": [0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0],
            "webrtc_host_ip_hash": "hostdeadbeef",
            "form_class": "desktop",
            "hardware_concurrency": 8,
            "platform": "Linux x86_64",
            "engine_family": "blink",
        })
    }

    fn webgl_only_same_host(webgl: &str, host: &str) -> Value {
        json!({
            "residual_mean": 0.26,
            "residual_std": 0.09,
            "residual_entropy_ok": true,
            "hw_webgl_stable": webgl,
            "hw_curve_webgl": [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9],
            "webrtc_host_ip_hash": host,
            "form_class": "desktop",
            "hardware_concurrency": 8,
            "platform": "Linux x86_64",
            "engine_family": "gecko",
        })
    }

    #[test]
    fn dual_residual_group_wins() {
        let s = select_identity_group(&dual_fields());
        assert_eq!(s.group_id, "G_DH_DUAL_RESIDUAL");
        assert_eq!(s.tier, CompletenessTier::Dh);
        assert!(!s.provisional);
    }

    #[test]
    fn single_webgl_core_same_bucket() {
        let a = webgl_only_same_host("wg_samefloor", "host1");
        let b = webgl_only_same_host("wg_samefloor", "host1");
        let r = same_commercial_bucket(&a, &b);
        assert_eq!(r["a"]["group_id"], "G_DH_CORE_WEBGL");
        assert_eq!(r["same_bucket"], true);
        assert_eq!(r["a"]["device_id"], r["b"]["device_id"]);
    }

    #[test]
    fn webgl_floor_disagree_forks() {
        let a = webgl_only_same_host("wg_floor_a", "host1");
        let b = webgl_only_same_host("wg_floor_b", "host1");
        let r = same_commercial_bucket(&a, &b);
        assert_eq!(r["same_bucket"], false);
        assert_ne!(r["a"]["device_id"], r["b"]["device_id"]);
    }

    #[test]
    fn gateway_only_provisional_dg() {
        let f = json!({
            "early_kick": true,
            "ja4": "t13d1516h2_8daaf6152771",
            "user_agent": "Mozilla/5.0",
            "server_client_ip": "1.2.3.4",
        });
        let s = select_identity_group(&f);
        assert!(s.provisional);
        assert_eq!(s.tier, CompletenessTier::Dg);
        assert!(s.group_id == "G_DG_GATEWAY" || s.group_id == "G_DG_EMPTY");
    }

    #[test]
    fn vt_best_prefers_dh_over_b8() {
        let cands = vec![
            json!({
                "session_id": "b8only",
                "device_tier": "dg",
                "digest_path": "empty_anchor_v1",
                "provisional_gateway": true,
                "batches_n": 1,
                "created_ms": 9_000_000_000i64,
            }),
            json!({
                "session_id": "rich",
                "device_tier": "dh",
                "digest_path": "real_curves_v1",
                "mint_silicon_ok": true,
                "batches_n": 60,
                "created_ms": 8_000_000_000i64,
            }),
        ];
        let r = select_vt_best_silicon(&cands);
        assert_eq!(r["selected"]["session_id"], "rich");
    }

    #[test]
    fn score_boost_uses_probe_fields() {
        let thin = json!({"platform": "Linux"});
        let rich = json!({
            "platform": "Linux",
            "timezone": "Asia/Shanghai",
            "hardware_concurrency": 8,
            "ja4": "t13d",
            "engine_family": "blink",
            "webgl_unmasked_renderer": "GPU",
            "pointer_samples": [1,2,3],
            "webdriver": false,
        });
        let bt = score_materials_boost(&thin);
        let br = score_materials_boost(&rich);
        assert!(br["os"]["boost"].as_f64().unwrap() >= bt["os"]["boost"].as_f64().unwrap());
        assert!(br["br"]["boost"].as_f64().unwrap() > 0.0);
    }

    #[test]
    fn field_registry_nonempty() {
        let j = field_registry_json();
        assert!(j["fields"].as_array().unwrap().len() >= 10);
        assert!(j["identity_groups"].as_array().unwrap().len() >= 5);
    }
}
