//! Association ladder for commercial device_id (iss/32).
//!
//! Product semantics:
//! - `os` / `br` / `rpa` are **current browser/page** scores — never the device key.
//! - `device_id` binds to an **observable environment** rung:
//!   hardware → env (VM/soft host) → profile (config cluster) → gateway-only.
//! Soft/virtual stacks must not claim `hardware`.

use crate::stack_auth::stack_auth_from_fields;
use serde_json::{json, Map, Value};

pub const ASSOCIATION_LADDER_ALGO: &str = "association_ladder_v1";

fn has_nonempty(fo: &Map<String, Value>, key: &str) -> bool {
    match fo.get(key) {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(_) => true,
    }
}

fn curve_ok(fo: &Map<String, Value>) -> bool {
    fo.get("hw_curve_webgl")
        .and_then(|v| v.as_array())
        .is_some_and(|a| a.len() >= 4)
        || fo.get("hw_curve_audio")
            .and_then(|v| v.as_array())
            .is_some_and(|a| a.len() >= 8)
        || fo.get("residual_mean").and_then(|v| v.as_f64()).is_some()
}

/// Derive association ladder level + basis from fields and optional tier/projection context.
pub fn association_ladder(fields: &Value, device: Option<&Value>) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let stack = stack_auth_from_fields(fields);
    let soft = stack.soft_stack
        || stack.residual_soft_like == Some(true)
        || fo
            .get("residual_soft_like")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        || fo
            .get("soft_stack")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

    let has_residual = curve_ok(&fo);
    let has_unit = has_nonempty(&fo, "unit_surface_id");
    let unit_versioned = has_unit
        && fo
            .get("unit_multiround_stable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        && fo
            .get("unit_surface_algo")
            .and_then(|v| v.as_str())
            .is_some_and(|a| a == "gr_unit_v1" || a.starts_with("gr_unit_v"));
    let has_webrtc = has_nonempty(&fo, "webrtc_host_ip_hash");
    let has_os_instance = has_nonempty(&fo, "os_instance_hash");
    let has_form = has_nonempty(&fo, "form_class");
    let has_gateway = has_nonempty(&fo, "server_client_ip")
        || has_nonempty(&fo, "server_asn")
        || device
            .and_then(|d| d.get("device_tier"))
            .and_then(|v| v.as_str())
            == Some("dg");
    let tier = device
        .and_then(|d| d.get("device_tier"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let collision = device
        .and_then(|d| d.get("collision_risk"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut basis: Vec<String> = Vec::new();
    if has_residual {
        basis.push("residual_webgl_or_curves".into());
    }
    if unit_versioned {
        basis.push("unit_surface_versioned".into());
    } else if has_unit {
        basis.push("unit_surface_conf".into());
    }
    if has_webrtc {
        basis.push("webrtc_host_separator".into());
    }
    if has_os_instance {
        basis.push("os_instance_hash".into());
    }
    if soft {
        basis.push("soft_stack".into());
    }
    if has_form {
        basis.push("form_class".into());
    }
    if has_gateway {
        basis.push("gateway_network".into());
    }
    if collision {
        basis.push("collision_risk".into());
    }

    let emulator = fo
        .get("emulator_hint")
        .map(|v| {
            v.as_bool().unwrap_or(false)
                || v.as_str().is_some_and(|s| !s.is_empty() && s != "false")
        })
        .unwrap_or(false)
        || matches!(
            fo.get("stack_class").and_then(|v| v.as_str()).unwrap_or(""),
            "emulator" | "virt" | "vm"
        );

    // Cross-family ladder (phase C): residual + host sep enables cross-browser env bind
    // even when UA family differs (gecko↔blink). Collision without host sep never claims hardware UV.
    let cross_family_env_ok =
        has_residual && (has_webrtc || has_os_instance) && !soft && !emulator;

    // Ladder decision (soft / emulator cannot claim hardware).
    let (level, reason) = if soft || emulator {
        if has_webrtc || has_os_instance {
            (
                "env",
                if emulator {
                    "emulator_or_virt_stack — env association only, never silicon"
                } else {
                    "soft_or_virtual_stack_with_host_separator — same execution env, not silicon"
                },
            )
        } else if has_residual || has_unit {
            (
                "profile",
                "soft/emulator residual/unit without host separator — config/cluster bind",
            )
        } else if has_gateway {
            ("gateway", "soft/emulator path thin materials — gateway-class only")
        } else {
            ("gateway", "soft/emulator path insufficient materials")
        }
    } else if has_residual && (has_webrtc || has_os_instance) && (tier == "dh" || has_form) {
        (
            "hardware",
            "non-soft residual + host separator — cross-browser machine/env bind (incl. cross-family)",
        )
    } else if has_residual && (tier == "dh" || (has_form && has_residual)) {
        if collision {
            (
                "env",
                "residual present but collision_risk / no host sep — env class, not machine UV",
            )
        } else if has_gateway || tier == "dh" {
            (
                "env",
                "non-soft residual without host sep — env-class residual bind (dh demoted path)",
            )
        } else {
            (
                "env",
                "non-soft residual present without host sep — not hardware UV claim",
            )
        }
    } else if has_residual {
        ("env", "residual present but not full hardware tier context")
    } else if has_gateway || tier == "dg" {
        ("gateway", "gateway/network observation only — no FE residual class")
    } else if has_unit {
        ("profile", "unit surface without residual hardware path")
    } else {
        ("gateway", "insufficient association materials")
    };

    // Map composite continuity band (when present on device) onto ladder surface.
    let continuity_band = device
        .and_then(|d| d.pointer("/composite_association/continuity_band"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            if level == "hardware" && !soft {
                "confirmed".into()
            } else if level == "env" {
                "likely".into()
            } else if level == "profile" {
                "weak".into()
            } else {
                "distinct".into()
            }
        });
    let likely_same_machine = device
        .and_then(|d| d.pointer("/composite_association/likely_same_machine"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let assoc_score = device
        .and_then(|d| d.pointer("/composite_association/assoc_score"))
        .and_then(|v| v.as_f64());

    // Product redlines embedded for consumers.
    json!({
        "algo": ASSOCIATION_LADDER_ALGO,
        "association_level": level,
        "association_basis": basis,
        "association_reason": reason,
        "soft_stack": soft,
        "claims_host_silicon": level == "hardware",
        "cross_browser_target": level == "hardware" || level == "env",
        "cross_family_env_ok": cross_family_env_ok,
        "host_separator": has_webrtc || has_os_instance,
        "continuity_band": continuity_band,
        "assoc_score": assoc_score,
        "likely_same_machine": likely_same_machine,
        "composite_algo": crate::composite_association::COMPOSITE_ASSOCIATION_ALGO,
        "redlines": {
            "os_br_rpa_not_device_key": true,
            "os_br_rpa_scope": "current_browser_or_page_only",
            "device_id_scope": "cross_browser_env_bind_best_effort",
            "no_pure_fe_global_uv_promise": true,
            "soft_never_hardware_level": soft,
            "ip_ua_ja_never_commercial_digest": true,
            "dh_requires_host_separator": true,
            "no_silent_force_merge_different_wg": true,
        },
        "browser_surface_note": "use browser_surface_id for this browser; device_id for env/machine ladder",
    })
}
