//! End-to-end pure evaluation: contracts → xsrc/band → bot → link → soft_v2 → brain.
//! Product surface: device_id + os + br + rpa (norm/10); diagnostics separate (bot/real_band/…).
//!
//! # Identity single entry
//!
//! `product.device_id` is produced **only** via [`crate::device_tier::select_device_tier`]
//! (ServerMint / residual_led + trust projection). See
//! `v5-docs/architecture/identity-single-entry.md`. Soft / product_scores / FE must not rewrite it.

use crate::bot::{score_bot, BotScore};
use crate::brain::build_frontier;
use crate::brain_control::{
    apply_band_pack_budget, build_battle_log, build_belief, classify_stop_reason,
    policy_band_from_scores, scan_capability_envelope, select_missions, sign_route_plan,
    unknown_bucket_from_belief, update_direction_priors,
};
use crate::client_exec::{apply_client_exec_demotion, assess_client_execution};
use crate::contracts::{batch_ids_from_evidence, coverage_for_batches, validate_route_plan};
use crate::decision::{decide_from_evaluate_parts, decide_from_evaluate_parts_scored};
use crate::experiment::{apply_strategy, default_stable_policy};
use crate::link::{
    associate, browser_surface_id_from_fields, commercial_device_id_from_fields, gpu_key,
};
use crate::association_ladder::association_ladder;
use crate::link_or_mint::{apply_server_mint, link_or_mint_pair};
use crate::product_scores::build_product_surface_with_evidence;
use crate::session_ticket::{
    b10_sla_violation, evidence_has_b10, issue_session_ticket_versioned, should_skip_session_probe,
};
use crate::soft_v2::{
    member_from_evidence, soft_pair_decision, PROMOTE_TO_COMMERCIAL_ID,
};
use crate::stack_auth::{
    commercial_id_blocked_by_stack, gpu_label_commercial_ok, stack_auth_from_fields,
};
use crate::device_ensemble::{fuse_pair_ensemble, fuse_single_ensemble};
use crate::device_tier::{
    authentic_fields_for_device_id, commercial_family, device_id_body, format_algo_device_id,
    is_commercial_device_id, is_dg_id, is_dh_id, is_dv_id, parse_algo_group, select_device_tier,
};
use crate::trust::{commercial_projection, COMMERCIAL_ALGO};
use crate::protocol_edge::derive_engine_claim_obs;
use crate::xsrc::evaluate_xsrc;
use serde_json::{json, Map, Value};

/// Diagnostic inventory of machine-visible fields (not commercial digest keys).
/// **F-11 commercial SSOT** is `field_product_matrix.commercial_device_id.digest_order`
/// (consumed via `commercial_projection`). This list only drives hard_materials_*
/// presence diagnostics on the device projection.
const DEVICE_MATERIALS: &[&str] = &[
    // SSOT commercial digest keys + source fields (hw_curve_* → hw_*_stable)
    "form_class",
    "hw_curve_audio",
    "hw_curve_webgl",
    "hardware_concurrency",
    "architecture",
    "os_family",
    "webrtc_host_ip_hash",
    // Diag surface (never commercial digest — listed for ops completeness)
    "platform",
    "timezone",
    "device_memory",
    "webgl_unmasked_renderer",
    "webgl_unmasked_vendor",
    "screen_width",
    "screen_height",
];

/// Derive os_family generically from platform / UA when FE omitted it.
/// Platform first — Playwright WebKit UA may claim Macintosh while platform is Linux.
fn derive_os_family(fields: &Map<String, Value>) -> Option<String> {
    if let Some(s) = fields.get("os_family").and_then(|v| v.as_str()) {
        if !s.is_empty() {
            return Some(s.to_ascii_lowercase());
        }
    }
    let platform = fields
        .get("platform")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let ua = fields
        .get("user_agent")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if platform.contains("android") {
        return Some("android".into());
    }
    if platform.contains("iphone") || platform.contains("ipad") {
        return Some("ios".into());
    }
    if platform.contains("win") {
        return Some("windows".into());
    }
    if platform.contains("linux") || platform.contains("x11") {
        return Some("linux".into());
    }
    if platform.contains("mac") {
        return Some("macos".into());
    }
    if ua.contains("android") {
        return Some("android".into());
    }
    if ua.contains("iphone") || ua.contains("ipad") || ua.contains("ios") {
        return Some("ios".into());
    }
    if ua.contains("windows") || ua.contains("win32") || ua.contains("win64") {
        return Some("windows".into());
    }
    if ua.contains("linux") || ua.contains("x11") {
        return Some("linux".into());
    }
    if ua.contains("mac os") || ua.contains("macintosh") || ua.contains("macintel") {
        return Some("macos".into());
    }
    None
}

/// Normalize materials for peer vectors / diagnostics (GPU uses single gpu_key path).
fn normalize_material_value(key: &str, v: &Value) -> Value {
    match key {
        "webgl_unmasked_renderer" => {
            if let Some(s) = v.as_str() {
                json!(gpu_key(s))
            } else {
                v.clone()
            }
        }
        "webgl_unmasked_vendor" => Value::Null,
        "os_family" | "form_class" | "timezone" => {
            if let Some(s) = v.as_str() {
                json!(s.trim().to_ascii_lowercase())
            } else {
                v.clone()
            }
        }
        "platform" => {
            if let Some(s) = v.as_str() {
                let t = s.to_ascii_lowercase();
                let fam = if t.contains("win") {
                    "windows"
                } else if t.contains("mac") || t.contains("iphone") || t.contains("ipad") {
                    "apple"
                } else if t.contains("linux") || t.contains("android") || t.contains("x11") {
                    "linuxoid"
                } else {
                    "other"
                };
                json!(fam)
            } else {
                v.clone()
            }
        }
        "hardware_concurrency" | "device_memory" | "screen_width" | "screen_height" => {
            if let Some(n) = v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)) {
                json!(n)
            } else {
                v.clone()
            }
        }
        _ => v.clone(),
    }
}

fn project_device(
    fields: &Value,
    truth_band: &str,
    has_server: bool,
    evidence: Option<&Value>,
) -> Value {
    let mut fo = fields.as_object().cloned().unwrap_or_default();
    // Backfill os_family when missing (common pre-phase FE gap).
    if !fo
        .get("os_family")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty())
    {
        if let Some(os) = derive_os_family(&fo) {
            fo.insert("os_family".into(), json!(os));
        }
    }
    // Demo→v5: stack/spoof authenticity (residual vs GPU label)
    let stack_auth = stack_auth_from_fields(&Value::Object(fo.clone()));
    // Untrusted spoofed GPU labels must not strengthen commercial hard materials.
    if stack_auth.gpu_label_untrusted {
        fo.remove("webgl_unmasked_renderer");
        fo.remove("webgl_unmasked_vendor");
    }
    let mut materials = Map::new();
    let mut present = Vec::new();
    let mut missing = Vec::new();
    for k in DEVICE_MATERIALS {
        if *k == "webgl_unmasked_vendor" {
            if let Some(v) = fo.get(*k) {
                if !v.is_null() && v.as_str() != Some("") {
                    present.push((*k).to_string());
                    continue;
                }
            }
            missing.push((*k).to_string());
            continue;
        }
        if *k == "webgl_unmasked_renderer" && stack_auth.gpu_label_untrusted {
            missing.push((*k).to_string());
            continue;
        }
        if let Some(v) = fo.get(*k) {
            if !v.is_null() && v.as_str() != Some("") {
                materials.insert((*k).into(), normalize_material_value(k, v));
                present.push((*k).to_string());
                continue;
            }
        }
        missing.push((*k).to_string());
    }
    let has_gpu = materials
        .get("webgl_unmasked_renderer")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());
    let has_machine_core = fo.get("hardware_concurrency").is_some()
        && (fo.get("timezone").is_some()
            || fo
                .get("os_family")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty()));
    // Commercial id = trust-gated machine-stable (cross-browser / cross-proxy).
    // Prefer main authentic materials; never mint from nest-overwritten identity.
    let fields_merged = Value::Object(fo.clone());
    let mut fields_v = authentic_fields_for_device_id(&fields_merged, evidence);
    let mut proj = commercial_projection(&fields_v);
    // Soft/untrusted: commercial id only when projection eligible (host separator on soft).
    // Annotate warnings; do not force-emit low-entropy soft dv_*.
    if commercial_id_blocked_by_stack(&stack_auth) {
        if let Some(obj) = proj.as_object_mut() {
            obj.insert("gpu_label_untrusted".into(), json!(stack_auth.gpu_label_untrusted));
            obj.insert("soft_stack".into(), json!(stack_auth.soft_stack));
            obj.insert("stack_class".into(), json!(stack_auth.stack_class.clone()));
            obj.insert("spoof_score".into(), json!(stack_auth.spoof_score));
            obj.insert("renderer_class".into(), json!(stack_auth.renderer_class.clone()));
        }
    }
    // iss/67: attach model_key BEFORE atlas_observe so cohort cells use K layer
    // (otherwise production atlas stays coarse gpu class only).
    let mut fields_v = fields_v;
    if let Some(fo) = fields_v.as_object_mut() {
        crate::model_key::attach_model_key_extras(fo);
    }
    // iss/58: observe population atlas + clock skew estimate into working fields.
    crate::population_atlas::atlas_observe(&fields_v);
    let atlas_enc = crate::population_atlas::differential_encode(&fields_v);
    let skew_est = crate::hw_clock_skew::estimate_from_fields(&fields_v);
    if let Some(fo) = fields_v.as_object_mut() {
        if let Some(ex) = atlas_enc.get("extras").and_then(|v| v.as_object()) {
            for (k, v) in ex {
                fo.insert(k.clone(), v.clone());
            }
        }
        if skew_est.get("ok").and_then(|v| v.as_bool()) == Some(true) {
            if let Some(b) = skew_est.get("clock_skew_ppm_bucket") {
                fo.insert("clock_skew_ppm_bucket".into(), b.clone());
            }
            if let Some(c) = skew_est.get("clock_skew_class") {
                fo.insert("clock_skew_class".into(), c.clone());
            }
            if let Some(x) = skew_est.get("tz_extra").and_then(|v| v.as_str()) {
                fo.insert("clock_skew_tz_extra".into(), json!(x));
            }
        }
    }
    // Multi-segment product ids: dv0|dv4|dv5|dv6 (SSOT via select_device_segments).
    // Uses enriched fields_v (atlas/skew) so segments see extras.
    let tier = select_device_tier(&fields_v, evidence);
    let device_id = tier
        .get("device_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let device_tier = tier
        .get("device_tier")
        .and_then(|v| v.as_str())
        .unwrap_or("multi")
        .to_string();
    let device_model_id = proj
        .get("device_model_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let browser_surface_id = browser_surface_id_from_fields(&fields_v).unwrap_or_default();
    // ServerMint: system-internal uniqueness beyond pure class residual hash.
    // Soft + OS/instance forks multi-VM collisions; versioned unit refines real path.
    // Empty-anchor: mint_eligible=false → never publish constant pit body (iss/43 R1).
    // Multi-source mint gate uses fields_by_source + source_conflicts from evidence.
    let mint = crate::link_or_mint::apply_server_mint_with_evidence(&fields_v, evidence);
    let mint_id = mint
        .get("device_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let mint_eligible = mint
        .get("mint_eligible")
        .and_then(|v| v.as_bool())
        .unwrap_or(!mint_id.is_empty());
    let mint_empty_anchor = mint
        .get("empty_anchor")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // Host separator: webrtc OR os_instance (must match trust + ServerMint semantics).
    let host_sep = mint
        .get("has_host_separator")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || proj
            .get("soft_has_host_separator")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        || fields_v
            .get("webrtc_host_ip_hash")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty())
        || fields_v
            .get("os_instance_hash")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty());
    let soft_host_sep = host_sep;
    // Public commercial id selection (iss/43 R2/R6/R7 · phases A/D):
    // - dg: ALWAYS use independent gateway id from tier (never ServerMint body).
    // - empty anchor: fall back to tier id (usually dg) — no stable dv_/dh_ constant.
    // - dh: require host sep; else demote mint body to dv_ (do not claim machine UV).
    // - dv: prefer ServerMint body when eligible.
    // Silicon residual/curves present even when tier selection fell to dg (merge lag,
    // missing form_class, thin soft dims). Do not discard ServerMint body — that was
    // producing gateway_only + dg_* while B10 residual+webrtc already sat on the session.
    let mint_has_silicon = mint_eligible
        && !mint_empty_anchor
        && !mint_id.is_empty()
        && (fields_v
            .get("residual_mean")
            .and_then(|v| v.as_f64())
            .is_some()
            || fields_v
                .get("residual_std")
                .and_then(|v| v.as_f64())
                .is_some_and(|s| s > 0.0)
            || fields_v
                .get("hw_curve_webgl")
                .and_then(|v| v.as_array())
                .is_some_and(|a| !a.is_empty())
            || fields_v
                .get("hw_curve_audio")
                .and_then(|v| v.as_array())
                .is_some_and(|a| !a.is_empty())
            || mint
                .get("residual_class")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty() && s != "rm_none"));
    // Public commercial multi-segment id: SSOT is select_device_segments (via select_device_tier).
    // ServerMint remains diagnostic uniqueness — does not rewrite multi-segment bodies.
    let public_device_id = device_id.clone();
    let device_id_segments = tier
        .get("device_id_segments")
        .cloned()
        .unwrap_or(json!({}));
    let composite_self =
        crate::composite_association::self_association_readiness(&fields_v);
    let composite_demoted = composite_self
        .get("demote_dh_recommended")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let _ = (mint_has_silicon, mint_empty_anchor, mint_eligible); // diagnostics only
    let device_tier = if device_tier.is_empty() {
        "multi".to_string()
    } else {
        device_tier
    };
    let device_algo_group = tier
        .get("device_algo_group")
        .and_then(|v| v.as_str())
        .unwrap_or("dv0-4-5-6")
        .to_string();
    let device_cluster_id = {
        // Cluster key from dv0 residual+curve parts (stable across precision lanes).
        let body = public_device_id
            .strip_prefix("dv0-")
            .unwrap_or(public_device_id.as_str());
        if body.is_empty() {
            String::new()
        } else {
            let h = {
                use sha2::{Digest, Sha256};
                let mut x = Sha256::new();
                x.update(body.as_bytes());
                format!("{:x}", x.finalize())
            };
            format!("dc_{}", &h[..16])
        }
    };
    // Multi-segment surface is always linkable for association assist when non-placeholder silicon exists.
    let linkable = tier
        .get("parts_present")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        >= 2
        && !public_device_id.is_empty();
    let collision_risk = if stack_auth.soft_stack
        || mint.get("soft_class").and_then(|v| v.as_bool()).unwrap_or(false)
    {
        // Soft path: collision only when no host/instance separator (not OR of stale flags).
        !soft_host_sep
            && mint
                .get("collision_risk")
                .and_then(|v| v.as_bool())
                .unwrap_or(true)
    } else {
        proj.get("collision_risk")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || mint
                .get("collision_risk")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            || (device_tier != "dg" && !host_sep)
    };
    let trust_sum = proj.get("trust_sum").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let mut confidence = tier
        .get("device_confidence")
        .or_else(|| tier.get("confidence"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    if confidence <= 0.0 {
        confidence = if !linkable {
            0.0
        } else if trust_sum >= 2.5 {
            0.92
        } else if trust_sum >= 2.0 {
            0.85
        } else if has_server && has_machine_core && has_gpu {
            0.80
        } else if has_machine_core {
            0.70
        } else {
            0.55
        };
    }
    // Soft / untrusted / collision: demote confidence but **keep** commercial id.
    if stack_auth.gpu_label_untrusted && device_tier != "dg" {
        confidence = (confidence * 0.75_f64).min(0.65_f64);
    } else if stack_auth.soft_stack && device_tier == "dv" {
        confidence = (confidence * 0.85_f64).min(0.75_f64);
    }
    if composite_demoted {
        confidence = (confidence * 0.88_f64).min(0.78_f64);
    } else if let Some(ascore) = composite_self.get("assoc_score").and_then(|v| v.as_f64()) {
        if ascore < 0.45 && device_tier == "dh" {
            confidence = (confidence * 0.90_f64).min(0.80_f64);
        }
    }
    if collision_risk {
        confidence = (confidence * 0.82_f64).min(0.72_f64);
    }
    // iss/67 C2: stability gate from family_observations / channel drift.
    // iss/69 U3: attach same-VT digest history (producer in
    // device_segments::select_device_segments) so the gate has a real input.
    // When GR_CHANNEL_DRIFT_GATE=1 and unstable/zero-replay → demote conf only (no hard-block).
    if !public_device_id.is_empty() {
        crate::hw_channel_drift::attach_vt_family_digests(&mut fields_v, &public_device_id);
    }
    let stability_gate = crate::hw_channel_drift::stability_gate_from_fields(&fields_v);
    if crate::hw_channel_drift::drift_gate_enabled()
        && stability_gate
            .get("ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        && !stability_gate
            .get("stable")
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
    {
        confidence = (confidence * 0.80_f64).min(0.70_f64);
    }
    // iss/63 commercial K/V: soft demote from Atlas adjudication (never hard-block).
    // Prefer precomputed atlas_shadow_score on fields (device_segments); recompute if absent.
    {
        let ash = fields_v
            .get("atlas_shadow_score")
            .cloned()
            .unwrap_or_else(|| crate::atlas_score::atlas_shadow_score(&fields_v));
        if ash
            .get("enforce")
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
        {
            let demote = ash
                .get("demote_weight")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
                .clamp(0.0, 0.48);
            if demote >= 0.12 {
                let factor = (1.0 - demote).clamp(0.55, 1.0);
                confidence = (confidence * factor).min(0.78);
            }
        }
        // Keep on fields for product/device surface
        if let Some(obj) = fields_v.as_object_mut() {
            obj.insert("atlas_shadow_score".into(), ash);
        }
    }
    // soft_promote redline always false
    let _ = PROMOTE_TO_COMMERCIAL_ID;
    let _ = commercial_device_id_from_fields; // retained for link re-exports / tests
    let mut analysis_posture: Vec<Value> = proj
        .get("analysis_posture")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if let Some(arr) = mint.get("analysis_posture").and_then(|v| v.as_array()) {
        for x in arr {
            if !analysis_posture.iter().any(|p| p == x) {
                analysis_posture.push(x.clone());
            }
        }
    }
    // Prefer honest mint digest_path; for dg/empty override real_curves lie (iss/43 R4).
    // Exception: silicon materials recovered after false dg → use mint/proj real path.
    let digest_path = {
        let mint_dp = mint.get("digest_path").and_then(|v| v.as_str());
        let proj_dp = proj.get("digest_path").and_then(|v| v.as_str());
        let chosen = if mint_empty_anchor {
            "empty_anchor_v1"
        } else if device_tier == "dg" && !mint_has_silicon {
            // True gateway / empty FE — never claim real_curves_v1.
            if mint_dp == Some("empty_anchor_v1") {
                "empty_anchor_v1"
            } else {
                "gateway_only_v1"
            }
        } else {
            mint_dp
                .or(proj_dp)
                .unwrap_or("thin_surface_v1")
        };
        json!(chosen)
    };
    // Honesty: gateway/dg or empty-anchor must not inherit confirmed_real authenticity.
    let authenticity_band = if device_tier == "dg"
        || mint_empty_anchor
        || digest_path.as_str() == Some("gateway_only_v1")
        || digest_path.as_str() == Some("empty_anchor_v1")
    {
        if matches!(truth_band, "confirmed_real" | "likely_real") {
            "watch"
        } else {
            truth_band
        }
    } else {
        truth_band
    };
    // iss/73: always materialize slot_roles (json! cannot host complex blocks)
    let slot_roles_surface = {
        let sr = tier.get("slot_roles").cloned().unwrap_or(Value::Null);
        if sr
            .as_object()
            .map(|o| o.get("policy").is_some())
            .unwrap_or(false)
        {
            sr
        } else {
            let ar_class = tier
                .get("curve_selection_notes")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter().any(|n| {
                        n.as_str()
                            .map(|s| {
                                s.contains("ar_class_slot_k_not_v")
                                    || s.contains("ar_fixed_residual_class_not_v")
                            })
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(true);
            crate::device_segments::slot_roles_json(ar_class)
        }
    };
    let empty_arr = Value::Array(vec![]);
    let id_warnings_v = proj
        .get("id_warnings")
        .cloned()
        .unwrap_or_else(|| empty_arr.clone());
    let no_id_reasons_v = proj
        .get("no_id_reasons")
        .cloned()
        .unwrap_or_else(|| empty_arr.clone());
    let provisional_gateway_v = mint.get("provisional_gateway").cloned().unwrap_or_else(|| {
        json!(
            device_tier == "dg"
                && (mint_empty_anchor
                    || digest_path.as_str() == Some("gateway_only_v1")
                    || digest_path.as_str() == Some("empty_anchor_v1"))
        )
    });
    let mut device_out = json!({
        "device_id": if public_device_id.is_empty() { Value::Null } else { json!(public_device_id) },
        "device_id_match": tier.get("device_id_match").cloned().unwrap_or(Value::Null),
        "device_id_reserved_compare": tier.get("device_id_reserved_compare").cloned().unwrap_or(Value::Null),
        "device_id_segments": device_id_segments,
        "device_id_segment_list": tier.get("device_id_segment_list").cloned().unwrap_or(json!([])),
        "device_segment_part_order": tier.get("device_segment_part_order").cloned().unwrap_or(json!([])),
        "slot_scheme": tier.get("slot_scheme").cloned().unwrap_or(json!("extended_curve_v2")),
        "slot_aliases": tier.get("slot_aliases").cloned().unwrap_or(json!({})),
        "device_tier": device_tier,
        "device_algo_group": device_algo_group,
        "multi_segment": true,
        "precision_lanes": tier.get("precision_lanes").cloned().unwrap_or(json!({})),
        "precision_policy": tier.get("precision_policy").cloned().unwrap_or(json!({})),
        "hw_curve_families": tier.get("hw_curve_families").cloned().unwrap_or(json!(0)),
        "device_segments": tier.get("device_segments").cloned().unwrap_or(json!({})),
        "curve_selection_notes": tier.get("curve_selection_notes").cloned().unwrap_or(json!([])),
        "curve_descriptors": tier.get("curve_descriptors").cloned().unwrap_or(json!({})),
        "conf_ceiling": tier.get("conf_ceiling").cloned().unwrap_or(json!(null)),
        "conf_reserved_compare": tier.get("conf_reserved_compare").cloned().unwrap_or(json!(null)),
        "material_slots_addressable": tier.get("material_slots_addressable").cloned().unwrap_or(json!(10)),
        "slot_quality": tier.get("slot_quality").cloned().unwrap_or(json!({})),
        // iss/67 K↔V diagnostics — must reach evaluate/analyze (never commercial body digests)
        "slot_quality_conf": tier.get("slot_quality_conf").cloned().unwrap_or(json!({})),
        "slot_roles": slot_roles_surface,
        "lane_c_sig": tier.get("lane_c_sig").cloned().unwrap_or(Value::Null),
        "lane_s_sig": tier.get("lane_s_sig").cloned().unwrap_or(Value::Null),
        "lane_c_materials_ready": tier.get("lane_c_materials_ready").cloned().unwrap_or(json!(false)),
        "hw_model_key": tier.get("hw_model_key").cloned().unwrap_or(Value::Null),
        "hw_model_key_info": tier.get("hw_model_key_info").cloned().unwrap_or(json!({})),
        "hw_model_key_segments": tier.get("hw_model_key_segments").cloned().unwrap_or(json!({})),
        "atlas_shadow_score": tier.get("atlas_shadow_score").cloned().unwrap_or(json!({})),
        "stability_gate": stability_gate.clone(),
        "os_source_policy": tier
            .get("os_source_policy")
            .cloned()
            .unwrap_or(json!("authentic_script_stable_os_class; gateway_ip_ua_excluded")),
        "device_model_id": if device_model_id.is_empty() { Value::Null } else { json!(device_model_id) },
        "device_id_candidate": proj.get("device_id_candidate").cloned().unwrap_or(Value::Null),
        "class_device_id": mint.get("class_device_id").cloned().unwrap_or(Value::Null),
        "device_cluster_id": if device_cluster_id.is_empty() { Value::Null } else { json!(device_cluster_id) },
        "browser_surface_id": if browser_surface_id.is_empty() { Value::Null } else { json!(browser_surface_id) },
        "confidence": confidence,
        "device_confidence": confidence,
        "collision_risk": collision_risk,
        "webgl_residual_entropy_ok": proj
            .get("webgl_residual_entropy_ok")
            .cloned()
            .unwrap_or(json!(true)),
        "analysis_posture": analysis_posture,
        "tier_reasons": tier.get("tier_reasons").cloned().unwrap_or(json!([])),
        "dimensions": tier.get("dimensions").cloned().unwrap_or(json!({})),
        "linkable": linkable,
        "hard_materials_present": present,
        "hard_materials_missing": missing,
        "materials_normalized": true,
        "algo": COMMERCIAL_ALGO,
        "server_mint": mint.get("server_mint").cloned().unwrap_or(json!(false)),
        "mint_eligible": mint_eligible,
        "empty_anchor": mint_empty_anchor,
        "has_host_separator": host_sep,
        "server_mint_algo": mint.get("server_mint_algo").cloned().unwrap_or(json!("server_mint_v1")),
        "link_or_mint_algo": mint.get("link_or_mint_algo").cloned().unwrap_or(json!("link_or_mint_v1")),
        "uniqueness_marker": mint.get("uniqueness_marker").cloned().unwrap_or(Value::Null),
        "link_or_mint": mint,
        "multi_source_mint_gate": mint
            .get("multi_source_mint_gate")
            .cloned()
            .unwrap_or(Value::Null),
        "stack_auth": stack_auth.to_value(),
        "digest_path": digest_path,
        "unit_surface_versioned": proj.get("unit_surface_versioned").cloned().unwrap_or(json!(false)),
        "unit_surface_algo": proj.get("unit_surface_algo").cloned().unwrap_or(Value::Null),
        "id_warnings": id_warnings_v.clone(),
        "soft_has_host_separator": proj.get("soft_has_host_separator").cloned().unwrap_or(json!(false)),
        "trust": {
            "sum": trust_sum,
            "floor": proj.get("trust_floor"),
            "sum_min": proj.get("trust_sum_min"),
            "eligible": proj.get("eligible"),
            "has_hardware_anchor": proj.get("has_hardware_anchor"),
            "has_both_curves": proj.get("has_both_curves"),
            "materials_included": proj.get("materials_included"),
            "materials": proj.get("materials"),
            "soft_extras_included": proj.get("soft_extras_included"),
            "corroboration": proj.get("corroboration"),
            "trust_scores": proj.get("trust_scores"),
            "server_client_ip_in_digest": false,
            "webrtc_in_digest": proj.get("webrtc_in_digest").cloned().unwrap_or(json!(false)),
            "soft_has_host_separator": proj.get("soft_has_host_separator").cloned().unwrap_or(json!(false)),
            "webgl_residual_entropy_ok": proj
                .get("webgl_residual_entropy_ok")
                .cloned()
                .unwrap_or(json!(true)),
            "device_model_id": proj.get("device_model_id").cloned().unwrap_or(Value::Null),
            "confidence_version": crate::conf_cal::active_confidence_version(),
            "no_id_reasons": proj.get("no_id_reasons"),
            "eligibility_reasons": proj.get("eligibility_reasons"),
            "gpu_label_commercial_ok": gpu_label_commercial_ok(&stack_auth),
            "renderer_class": stack_auth.renderer_class.clone(),
            "digest_path": digest_path.clone(),
            "id_warnings": id_warnings_v,
            "soft_stack": stack_auth.soft_stack,
            "stack_class": stack_auth.stack_class.clone(),
        },
        "no_id_reasons": no_id_reasons_v,
        "soft_promote": false,
        "authenticity_band": authenticity_band,
        "requires_server_for_confirmed": true,
        "composite_association": composite_self,
        "composite_dh_demoted": composite_demoted,
        "algo_group": mint.get("algo_group").cloned().unwrap_or(Value::Null),
        "algo_group_id": mint.get("algo_group_id").cloned().unwrap_or(Value::Null),
        "algo_group_tier": mint.get("algo_group_tier").cloned().unwrap_or(Value::Null),
        "provisional_gateway": provisional_gateway_v,
        "multi_source_resolution": mint
            .get("multi_source_resolution")
            .cloned()
            .unwrap_or(Value::Null),
        "multi_source_mint_gate": mint
            .get("multi_source_mint_gate")
            .cloned()
            .unwrap_or(Value::Null),
    });
    // iss/32: association ladder annotates commercial id (does not replace dh_/dv_/dg_).
    let ladder = association_ladder(&fields_v, Some(&device_out));
    let b10x_hint = crate::algo_groups::b10x_schedule_hint(&fields_v, None);
    if let Some(obj) = device_out.as_object_mut() {
        obj.insert(
            "association_level".into(),
            ladder
                .get("association_level")
                .cloned()
                .unwrap_or(json!("gateway")),
        );
        obj.insert(
            "association_basis".into(),
            ladder
                .get("association_basis")
                .cloned()
                .unwrap_or(json!([])),
        );
        obj.insert(
            "continuity_band".into(),
            ladder
                .get("continuity_band")
                .cloned()
                .unwrap_or_else(|| {
                    composite_self
                        .get("continuity_band")
                        .cloned()
                        .unwrap_or(json!("weak"))
                }),
        );
        obj.insert("association_ladder".into(), ladder);
        obj.insert("b10x_schedule_hint".into(), b10x_hint);
        // Soft host cluster + score material boosts (never rewrite device_id).
        let soft_host = crate::engine_surface::soft_host_cluster_from_fields(&fields_v);
        obj.insert("soft_host_cluster".into(), soft_host);
        let score_boost = crate::algo_groups::score_materials_boost(&fields_v);
        obj.insert("score_materials_boost".into(), score_boost);
    }
    device_out
}

/// Compare two field vectors with sparse_safe associate (multi-session / multi-browser).
/// Callers supply projected or raw hard-material maps — no engine identifiers.
pub fn multi_session_link(
    fields_a: &Value,
    fields_b: &Value,
    link_algo: &str,
) -> Result<Value, String> {
    let r = associate(fields_a, fields_b, link_algo)?;
    let mut out = r.to_value();
    // Compose with link_or_mint / ServerMint decision (soft OS fork, unit link).
    let lom = link_or_mint_pair(fields_a, fields_b);
    if let Some(obj) = out.as_object_mut() {
        obj.insert("link_or_mint".into(), lom);
    }
    Ok(out)
}

/// Multi-algorithm pair fusion (all digest strategies × all associate algos).
/// Prefer this over any single `link_algo` for cross-browser machine decisions.
pub fn multi_session_ensemble(fields_a: &Value, fields_b: &Value) -> Value {
    let mut ens = fuse_pair_ensemble(fields_a, fields_b);
    let lom = link_or_mint_pair(fields_a, fields_b);
    if let Some(obj) = ens.as_object_mut() {
        obj.insert("link_or_mint".into(), lom);
    }
    ens
}

/// Evaluate one session evidence snapshot. Pure: no I/O beyond already-loaded specs.
pub fn evaluate_session(
    evidence: &Value,
    strategy_mutations: Option<&Value>,
    peer_vector: Option<&Value>,
    peer_evidence: Option<&Value>,
    soft_v2_ready: bool,
) -> Result<Value, String> {
    if !evidence.is_object() {
        return Err("evidence must be object".into());
    }

    let strat = apply_strategy(strategy_mutations, Some(default_stable_policy().map_err(|e| e.to_string())?), true)
        .map_err(|e| e.to_string())?;
    let policy = &strat.base;
    let bot_algo = policy
        .get("bot_algo")
        .and_then(|v| v.as_str())
        .unwrap_or("balanced");
    let link_algo = policy
        .get("link_algo")
        .and_then(|v| v.as_str())
        .unwrap_or("sparse_safe");
    let bot_algo = if matches!(bot_algo, "balanced" | "baseline") {
        bot_algo
    } else {
        "balanced"
    };

    let mut fields = evidence
        .get("fields")
        .cloned()
        .unwrap_or(Value::Object(Map::new()));
    // Backfill os_family + promote server_* into fields for machine-stable device_id.
    if let Some(obj) = fields.as_object_mut() {
        if !obj
            .get("os_family")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty())
        {
            if let Some(os) = derive_os_family(obj) {
                obj.insert("os_family".into(), json!(os));
            }
        }
        for k in ["server_client_ip", "server_asn", "server_country"] {
            if !obj.contains_key(k) {
                if let Some(v) = evidence.get(k).cloned() {
                    obj.insert(k.into(), v);
                }
            }
            if let Some(v) = evidence.pointer(&format!("/gateway_fields/{k}")).cloned() {
                obj.entry(k.to_string()).or_insert(v);
            }
        }
        // protocol_engine from gateway for engine family claim-obs
        if !obj.contains_key("protocol_engine") {
            if let Some(v) = evidence
                .pointer("/gateway_fields/protocol_engine")
                .cloned()
            {
                obj.insert("protocol_engine".into(), v);
            }
        }
        // iss/21 T-ENG-1: engine_claim / engine_obs (families, not brands)
        let (claim, obs) = derive_engine_claim_obs(obj);
        obj.entry("engine_claim".to_string())
            .or_insert_with(|| json!(claim));
        obj.entry("engine_obs".to_string())
            .or_insert_with(|| json!(obs));
    }
    let bot = score_bot(&fields, bot_algo)?;
    let truth = evaluate_xsrc(evidence, Some(&bot)).map_err(|e| e.to_string())?;

    let link_result = if let Some(peer) = peer_vector {
        let keys = [
            "os_family",
            "hardware_concurrency",
            "screen_width",
            "screen_height",
            "timezone",
            "webgl_unmasked_renderer",
            "form_class",
            "device_memory",
            "platform",
            "residual_mean",
            "stack_class",
            "spoof_score",
            "residual_soft_like",
            "renderer_class",
        ];
        let mut self_vec = Map::new();
        if let Some(fo) = fields.as_object() {
            for k in keys {
                if let Some(v) = fo.get(k) {
                    self_vec.insert(k.into(), normalize_material_value(k, v));
                }
            }
        }
        // Strip untrusted GPU labels before peer associate (same as link prepare_link_fields)
        let auth_self = stack_auth_from_fields(&Value::Object(self_vec.clone()));
        if auth_self.gpu_label_untrusted {
            self_vec.remove("webgl_unmasked_renderer");
            self_vec.insert("stack_class".into(), json!(auth_self.stack_class.clone()));
            self_vec.insert("spoof_score".into(), json!(auth_self.spoof_score));
        }
        // Normalize peer similarly when possible
        let mut peer_norm = peer.clone();
        if let Some(po) = peer_norm.as_object_mut() {
            let keys_owned: Vec<String> = po.keys().cloned().collect();
            for k in keys_owned {
                if let Some(v) = po.get(&k).cloned() {
                    po.insert(k.clone(), normalize_material_value(&k, &v));
                }
            }
            if !po
                .get("os_family")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty())
            {
                if let Some(os) = derive_os_family(po) {
                    po.insert("os_family".into(), json!(os));
                }
            }
            let auth_peer = stack_auth_from_fields(&Value::Object(po.clone()));
            if auth_peer.gpu_label_untrusted {
                po.remove("webgl_unmasked_renderer");
                po.insert("stack_class".into(), json!(auth_peer.stack_class));
                po.insert("spoof_score".into(), json!(auth_peer.spoof_score));
            }
        }
        // Inject prior commercial device_id when present (link association needs it).
        if let Some(did) = fields
            .get("device_id")
            .or_else(|| evidence.pointer("/device/device_id"))
            .and_then(|v| v.as_str())
        {
            if is_dh_id(did) || is_dv_id(did) {
                self_vec.insert("device_id".into(), json!(did));
            }
        }
        Some(associate(&Value::Object(self_vec), &peer_norm, link_algo)?)
    } else {
        None
    };

    let soft = if let Some(peer_ev) = peer_evidence {
        let mut soft = soft_pair_decision(evidence, peer_ev, None);
        if let Some(edge) = soft.get_mut("edge") {
            if edge.is_object() {
                if let Some(obj) = edge.as_object_mut() {
                    obj.insert(
                        "promote_to_commercial_id".into(),
                        json!(PROMOTE_TO_COMMERCIAL_ID),
                    );
                }
            }
        }
        soft
    } else {
        let m = member_from_evidence(evidence, None);
        json!({
            "member": m.to_value(),
            "promote_to_commercial_id": PROMOTE_TO_COMMERCIAL_ID,
            "edge": null,
            "soft_v2": true,
        })
    };

    // Schedule remaining coverage packs. Session cool-down can skip session materials
    // only when silicon materials exist (ticket v2). Force / missing silicon → full path.
    let mut skip_session = should_skip_session_probe(evidence);
    let b10_sla_code = b10_sla_violation(evidence, None);
    // SLA: need B10 but missing → never skip session; force commercial pack path.
    if b10_sla_code.is_some() {
        skip_session = false;
    }
    let mut evidence_for_brain = evidence.clone();
    if skip_session {
        if let Some(obj) = evidence_for_brain.as_object_mut() {
            obj.insert("skip_session_probe".into(), json!(true));
            if let Some(meta) = obj.get_mut("meta").and_then(|m| m.as_object_mut()) {
                meta.insert("skip_session_probe".into(), json!(true));
            } else {
                obj.insert(
                    "meta".into(),
                    json!({"skip_session_probe": true}),
                );
            }
        }
    } else if let Some(obj) = evidence_for_brain.as_object_mut() {
        // Explicitly clear stale skip flags when SLA / force demands full session probe.
        obj.insert("skip_session_probe".into(), json!(false));
        if let Some(meta) = obj.get_mut("meta").and_then(|m| m.as_object_mut()) {
            meta.insert("skip_session_probe".into(), json!(false));
        }
    }
    // Adaptive pack budget (band + short-visit) so FE can finish parallel race faster.
    let band_hint = evidence
        .get("real_band")
        .or_else(|| evidence.get("policy_band"))
        .and_then(|v| v.as_str())
        .unwrap_or("watch");
    let mut max_packs = crate::brain_control::max_packs_for_band(band_hint).max(6);
    // No FE main → skip mid/deep amplify budget (nojs/robots lightweight path).
    let has_main_fe = evidence
        .get("batches")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter().any(|b| {
                let src = b.get("source").and_then(|v| v.as_str()).unwrap_or("");
                src == "main" || src.starts_with("iframe") || src.starts_with("sandbox")
            })
        })
        .unwrap_or(false);
    if !has_main_fe {
        max_packs = 4; // B8 + lite residuals only
    }
    let frontier = build_frontier(&evidence_for_brain, soft_v2_ready, None, max_packs)
        .map_err(|e| e.to_string())?;
    let mut derived_route = frontier.to_route_plan().map_err(|e| e.to_string())?;
    let static_wave = crate::brain::static_kick_plan().map_err(|e| e.to_string())?;
    let mut coverage = frontier.coverage.clone();
    // Soft terminal from frontier (coverage_complete). Commercial final may override later.
    let mut analysis_terminal = frontier.stop_probe
        && coverage
            .get("coverage_complete")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

    let (route_ok, route_err) = if let Some(rp) = evidence.get("route_plan") {
        match validate_route_plan(rp) {
            Ok(_) => (Some(true), None),
            Err(e) => (Some(false), Some(e.to_string())),
        }
    } else {
        (None, None)
    };

    // G-ARCH-22: client-proposed packs cannot expand server allowlist.
    let route_authority = crate::brain::filter_client_proposed_packs(
        frontier.packs.as_slice(),
        evidence.get("route_plan"),
    );

    // G-ARCH-21: T0 capabilities from fields (brain may filter packs).
    let capabilities = crate::capabilities::project_capabilities(&fields);
    let _caps_filtered =
        crate::capabilities::filter_packs_by_capabilities(frontier.packs.as_slice(), &capabilities);

    let batches = batch_ids_from_evidence(evidence);
    let cov = coverage_for_batches(&batches).map_err(|e| e.to_string())?;

    let session_id = evidence
        .get("session_id")
        .cloned()
        .or_else(|| fields.get("session_id").cloned())
        .unwrap_or(Value::Null);

    // Coarse short-visit utility: with PKG_UA or B8, bot algo still scores (not empty).
    let has_ua = fields
        .get("user_agent")
        .or_else(|| fields.get("ua"))
        .map(|v| !v.is_null() && v.as_str() != Some(""))
        .unwrap_or(false);

    // iss/61 F2: gated fuzzy-ECC mint material (default OFF; GR_FUZZY_MINT=1).
    // When on, helper-stabilized digests fold into wg/au slot extras so same-machine
    // micro-drift near quantization boundaries does not wrong-split the mint.
    let fields_mint_holder;
    let fields_for_device: &Value = if crate::device_segments::fuzzy_mint_enabled() {
        let mut fm = fields.clone();
        let fz = crate::fuzzy_ecc::fuzzy_ecc_from_fields(&fields);
        if let Some(ex) = fz.get("extras").and_then(|v| v.as_object()) {
            if let Some(fo) = fm.as_object_mut() {
                for k in ["fuzzy_ecc_wg_digest", "fuzzy_ecc_au_digest"] {
                    if let Some(v) = ex.get(k) {
                        fo.insert(k.into(), v.clone());
                    }
                }
            }
        }
        fields_mint_holder = fm;
        &fields_mint_holder
    } else {
        &fields
    };
    let mut device = project_device(
        fields_for_device,
        &truth.real_band,
        truth.has_server_side,
        Some(evidence),
    );
    // iss/58 identity governance: hot-bucket · effective_bits · confidence · mint_posture
    let sid_for_gov = evidence
        .get("session_id")
        .and_then(|v| v.as_str())
        .or_else(|| fields.get("session_id").and_then(|v| v.as_str()))
        .unwrap_or("");
    let gov_base_conf = device
        .get("confidence")
        .or_else(|| device.get("device_confidence"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.55);
    let gov_collision_in = device
        .get("collision_risk")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let gov_did = device
        .get("device_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let gov_segs = device
        .get("device_id_segments")
        .cloned()
        .unwrap_or(json!({}));
    let governance = crate::identity_governance::apply_identity_governance(
        &fields,
        &gov_did,
        &gov_segs,
        sid_for_gov,
        gov_base_conf,
        gov_collision_in,
    );
    if let Some(obj) = device.as_object_mut() {
        obj.insert(
            "confidence_version".into(),
            json!(crate::conf_cal::active_confidence_version()),
        );
        obj.insert("identity_governance".into(), governance.clone());
        if let Some(id) = governance.get("identity") {
            obj.insert("identity".into(), id.clone());
            if let Some(c) = id.get("confidence").and_then(|v| v.as_f64()) {
                obj.insert("confidence".into(), json!(c));
                obj.insert("device_confidence".into(), json!(c));
            }
        }
        if let Some(cr) = governance.get("collision_risk").and_then(|v| v.as_bool()) {
            obj.insert("collision_risk".into(), json!(cr));
        }
        obj.insert(
            "mint_posture".into(),
            governance
                .get("mint_posture")
                .cloned()
                .unwrap_or(json!("stable")),
        );
        obj.insert(
            "ephemeral".into(),
            governance
                .get("ephemeral")
                .cloned()
                .unwrap_or(json!(false)),
        );
        obj.insert(
            "ephemeral_ttl_ms".into(),
            governance
                .get("ephemeral_ttl_ms")
                .cloned()
                .unwrap_or(json!(0)),
        );
        // Govern public device_id: withhold / ephemeral when hot (merge_averse default)
        if let Some(gdid) = governance.get("device_id_governed") {
            if gdid.is_null() || gdid.as_str() == Some("") {
                if governance
                    .get("commercial_blocked")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    obj.insert("device_id".into(), Value::Null);
                    obj.insert(
                        "device_id_stable_candidate".into(),
                        governance
                            .get("stable_device_id_candidate")
                            .cloned()
                            .unwrap_or(Value::Null),
                    );
                }
            } else {
                obj.insert("device_id".into(), gdid.clone());
                if governance
                    .get("ephemeral")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    obj.insert(
                        "device_id_stable_candidate".into(),
                        governance
                            .get("stable_device_id_candidate")
                            .cloned()
                            .unwrap_or(Value::Null),
                    );
                }
            }
        }
        obj.insert(
            "effective_bits".into(),
            governance
                .get("effective_bits")
                .cloned()
                .unwrap_or(json!({})),
        );
        obj.insert(
            "bucket_heat".into(),
            governance
                .get("bucket_heat")
                .cloned()
                .unwrap_or(json!({})),
        );
        obj.insert(
            "force_deepen".into(),
            governance
                .get("force_deepen")
                .cloned()
                .unwrap_or(json!(false)),
        );
        // B7 closed loop: collapsed slots auto-downweight commercial confidence
        if governance
            .pointer("/effective_bits/alert_collapse")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            let c = obj
                .get("confidence")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.5);
            let c2 = (c * 0.85).min(0.72);
            obj.insert("confidence".into(), json!(c2));
            obj.insert("device_confidence".into(), json!(c2));
            obj.insert("effective_bits_downweight".into(), json!(true));
        }

        // iss/60: fuzzy ECC + physical assert + antidetect signals (no hard ban)
        let fuzzy = crate::fuzzy_ecc::fuzzy_ecc_from_fields(&fields);
        obj.insert("fuzzy_ecc".into(), fuzzy.clone());
        if let Some(ex) = fuzzy.get("extras").and_then(|v| v.as_object()) {
            for (k, v) in ex {
                if k.ends_with("_digest") {
                    obj.insert(k.clone(), v.clone());
                }
            }
        }
        let phys = crate::physical_assert::physical_assert_surface(&fields);
        obj.insert("physical_assert".into(), phys.clone());
        let mu = crate::identity_governance::machine_heat_unit(
            Some(&fields),
            &gov_did,
            sid_for_gov,
        );
        let anti = crate::antidetect_signals::antidetect_signals_surface(
            &fields,
            mu,
            sid_for_gov,
        );
        obj.insert("antidetect".into(), anti.clone());
        let mut conf = obj
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.55);
        let mut force_deepen = governance
            .get("force_deepen")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if phys
            .get("should_downweight")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            conf = (conf * 0.82).min(0.7);
            obj.insert("physical_assert_downweight".into(), json!(true));
        }
        if phys
            .get("should_deepen")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            force_deepen = true;
        }
        if anti
            .get("should_downweight")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            conf = (conf * 0.85).min(0.72);
            obj.insert("antidetect_downweight".into(), json!(true));
        }
        if anti
            .get("should_deepen")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            force_deepen = true;
            // profile farm → ephemeral preference when multi-profile hot
            if anti
                .pointer("/profile_farm/alert")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                obj.insert("profile_farm_alert".into(), json!(true));
                if !obj
                    .get("ephemeral")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    // soft signal only — do not rewrite stable id here
                    obj.insert("mint_posture_hint".into(), json!("deepen_required"));
                }
            }
        }
        obj.insert("confidence".into(), json!(conf));
        obj.insert("device_confidence".into(), json!(conf));
        obj.insert("force_deepen".into(), json!(force_deepen));
        // iss/73: re-assert slot_roles after all device mutations (some revs dropped it).
        if !obj
            .get("slot_roles")
            .and_then(|v| v.get("policy"))
            .and_then(|v| v.as_str())
            .map(|s| s.contains("iss67"))
            .unwrap_or(false)
        {
            let ar_class = obj
                .get("curve_selection_notes")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter().any(|n| {
                        n.as_str()
                            .map(|s| {
                                s.contains("ar_class_slot_k_not_v")
                                    || s.contains("ar_fixed_residual_class_not_v")
                            })
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(true);
            obj.insert(
                "slot_roles".into(),
                crate::device_segments::slot_roles_json(ar_class),
            );
        }
        // iss/73 P0-1: Atlas tail-cell explore urgency → deepen silicon packs
        let explore = crate::population_atlas::atlas_explore_urgency(&fields);
        obj.insert("atlas_explore_urgency".into(), explore.clone());
        if explore
            .get("should_deepen")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            force_deepen = true;
        }
        if force_deepen {
            let mut packs: Vec<String> = vec![
                "B10x_silicon_deep".into(),
                "B30_gpu_bandwidth".into(),
                "B84_gpu_bandwidth_ladder".into(),
                "B46_audio_deep".into(),
                "B20_challenge_seed".into(),
            ];
            if let Some(arr) = explore.get("deepen_packs").and_then(|v| v.as_array()) {
                for p in arr {
                    if let Some(s) = p.as_str() {
                        if !packs.iter().any(|x| x == s) {
                            packs.push(s.to_string());
                        }
                    }
                }
            }
            obj.insert("deepen_packs_extra".into(), json!(packs));
        }
        // iss/73: expose cell coverage snapshot (ops / admin)
        obj.insert(
            "atlas_cell_coverage".into(),
            crate::population_atlas::cell_coverage_report(),
        );
    }

    // Client JS execution quality (no-JS / early pagehide / thin FE).
    let client_execution = assess_client_execution(evidence, &truth);
    let client_exec_status = client_execution
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("js_ok");
    // Usable for product axes when FE contributed main_core or partial FE (not server-only / early abort).
    let client_js_usable = matches!(client_exec_status, "js_ok" | "js_thin");

    let mut short_visit = json!({
        "static_wave_available": true,
        "static_pack_count": static_wave.get("packs").and_then(|p| p.as_array()).map(|a| a.len()).unwrap_or(0),
        "has_ua_signal": has_ua,
        "usable_coarse": has_ua || truth.has_server_side,
        "confirmed_requires_multi_source": true,
        "fe_only_never_confirmed": true,
        "client_execution_status": client_exec_status,
        "client_js_usable": client_js_usable,
    });
    if let Some(hint) = client_execution.get("product_hint") {
        if let Some(obj) = short_visit.as_object_mut() {
            obj.insert("client_execution_hint".into(), hint.clone());
        }
    }

    let soft_edge = soft
        .get("edge")
        .map(|e| !e.is_null())
        .unwrap_or(false)
        || soft
            .get("edges")
            .and_then(|e| e.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false)
        || soft
            .get("has_edge")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let conf = device
        .get("confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let dv = device.get("device_id").and_then(|v| v.as_str());
    // Placeholder decisions; overwritten after product scores (G-ARCH-15).
    let decision_nonsensitive =
        decide_from_evaluate_parts(&bot.verdict, dv, soft_edge, conf, "nonsensitive");
    let decision_sensitive =
        decide_from_evaluate_parts(&bot.verdict, dv, soft_edge, conf, "sensitive");

    let stack = stack_auth_from_fields(&fields);
    let page_id = evidence
        .get("page_id")
        .and_then(|v| v.as_str())
        .or_else(|| fields.get("page_id").and_then(|v| v.as_str()));

    let source_conflicts: Vec<String> = evidence
        .get("source_conflicts")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // Structured realm conflict graph (L1–L4 diff + outlier, §4.2). Verdict-based
    // pressure feeds the product surface; hard conflicts demote real bands below.
    let realm_conflict_graph = evidence
        .get("realm_conflict_graph")
        .cloned()
        .unwrap_or(Value::Null);
    let realm_conflict_verdict = realm_conflict_graph
        .get("verdict")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let realm_conflict_pressure = match realm_conflict_verdict {
        "hard_conflict" => 0.8,
        "soft_mismatch" => 0.4,
        _ => 0.0,
    };
    let realm_outlier = realm_conflict_graph
        .get("outlier_realm")
        .and_then(|v| v.get("realm"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Stamp multi_source mint pressures onto fields for OS/BR demotion axes.
    let mut fields_for_product = fields.clone();
    if let Some(gate) = device.get("multi_source_mint_gate") {
        if let Some(fo) = fields_for_product.as_object_mut() {
            if let Some(sp) = gate.get("single_source_pressure").and_then(|v| v.as_f64()) {
                fo.insert("mint_single_source_pressure".into(), json!(sp));
            }
            if let Some(cp) = gate.get("conflict_pressure").and_then(|v| v.as_f64()) {
                fo.insert("mint_conflict_pressure".into(), json!(cp));
            }
            fo.insert("multi_source_mint_gate".into(), gate.clone());
        }
    }
    // Realm conflict pressure (structured graph) onto product surface like mint pressure.
    // Folded into mint_conflict_pressure so product_scores applies the demotion —
    // the realm graph is the structured/weighted superset of flat source conflicts.
    // M1: realm coherence score (numeric-level cross-realm agreement) rides along
    // for the spoof_risk main channel.
    if let Some(fo) = fields_for_product.as_object_mut() {
        fo.insert("realm_conflict_pressure".into(), json!(realm_conflict_pressure));
        fo.insert("realm_conflict_verdict".into(), json!(realm_conflict_verdict));
        if let Some(rcs) = realm_conflict_graph.get("realm_coherence_score") {
            fo.insert("realm_coherence_score".into(), rcs.clone());
        }
        if let Some(rcv) = realm_conflict_graph.get("realm_coherence_verdict") {
            fo.insert("realm_coherence_verdict".into(), rcv.clone());
        }
        if realm_conflict_pressure > 0.0 {
            fo.insert(
                "realm_conflict_keys".into(),
                realm_conflict_graph
                    .get("conflicted_keys")
                    .cloned()
                    .unwrap_or_else(|| json!([])),
            );
            let existing = fo
                .get("mint_conflict_pressure")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            fo.insert(
                "mint_conflict_pressure".into(),
                json!((existing * 100.0).max(realm_conflict_pressure * 100.0) / 100.0),
            );
        }
    }
    // Populate legacy entitlement metadata; the current product is ungated.
    if let Some(fo) = fields_for_product.as_object_mut() {
        if fo.get("rpa_analysis_enabled").is_none() {
            let site = evidence
                .get("site_id")
                .and_then(|v| v.as_str())
                .or_else(|| fields.get("site_id").and_then(|v| v.as_str()));
            let host = fields
                .get("page_host")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    fields.get("page_url").and_then(|v| v.as_str()).and_then(|u| {
                        let s = u.trim();
                        let rest = s
                            .strip_prefix("https://")
                            .or_else(|| s.strip_prefix("http://"))
                            .unwrap_or(s);
                        let host = rest.split('/').next().unwrap_or(rest);
                        let host = host.split(':').next().unwrap_or(host);
                        if host.is_empty() {
                            None
                        } else {
                            Some(host)
                        }
                    })
                });
            let ent = crate::plan_entitlement::resolve_site_entitlement(site, host, None);
            fo.insert("rpa_analysis_enabled".into(), json!(ent.rpa_enabled));
            fo.insert("plan".into(), json!(ent.plan));
        }
    }

    // Unified product surface (norm/10) — shared evidence projections, not separate probe suites.
    // Pass full evidence so open re-probe gaps / field utilization use session batches+sources.
    let (mut product, diagnostics, mut page) = build_product_surface_with_evidence(
        &device,
        &fields_for_product,
        &stack,
        &bot,
        &truth,
        page_id,
        &decision_nonsensitive,
        &decision_sensitive,
        bot.algo.as_str(),
        link_algo,
        coverage
            .get("coverage_complete")
            .and_then(|v| v.as_bool()),
        analysis_terminal,
        &source_conflicts,
        Some(evidence),
    );

    // Cap / annotate product when client JS never ran or exited early (G-PROD client_exec).
    apply_client_exec_demotion(&mut product, &mut page, &mut device, &client_execution);

    // Re-read conf after demotion (device confidence may be capped).
    let conf = device
        .get("confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let dv = device.get("device_id").and_then(|v| v.as_str());

    let os_s = product
        .pointer("/os/score")
        .and_then(|v| v.as_f64());
    let br_s = product
        .pointer("/br/score")
        .and_then(|v| v.as_f64());
    let rpa_s = page
        .as_ref()
        .and_then(|p| p.pointer("/rpa/score"))
        .and_then(|v| v.as_f64())
        .or_else(|| product.pointer("/rpa/score").and_then(|v| v.as_f64()));
    let decision_nonsensitive = decide_from_evaluate_parts_scored(
        &bot.verdict,
        dv,
        soft_edge,
        conf,
        "nonsensitive",
        os_s,
        br_s,
        rpa_s,
    );
    let decision_sensitive = decide_from_evaluate_parts_scored(
        &bot.verdict,
        dv,
        soft_edge,
        conf,
        "sensitive",
        os_s,
        br_s,
        rpa_s,
    );

    // Product-aware real_band demotion: multi-signal os/br/bot must not leave
    // confirmed_real when product surface already says abnormal (G-ARCH/lab integrity).
    let (mut real_band, mut band_demote_reasons) =
        demote_real_band_with_product(&truth.real_band, &bot, &product, &source_conflicts);
    // Realm conflict graph: hard cross-realm conflicts (incl. weighted outlier)
    // demote remaining real-ish bands — structured superset of source_conflicts.
    if matches!(real_band.as_str(), "confirmed_real" | "likely_real")
        && (realm_conflict_verdict == "hard_conflict" || realm_conflict_pressure >= 0.8)
    {
        real_band = "watch".into();
        let reason = if !realm_outlier.is_empty() {
            format!(
                "demote_realm_conflict_graph:verdict={realm_conflict_verdict},outlier={realm_outlier}"
            )
        } else {
            format!("demote_realm_conflict_graph:verdict={realm_conflict_verdict}")
        };
        band_demote_reasons.push(reason);
    }
    let mut truth_out = truth.to_value();
    if let Some(obj) = truth_out.as_object_mut() {
        obj.insert("real_band".into(), json!(real_band));
        if !band_demote_reasons.is_empty() {
            let mut rs = truth
                .reasons
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            rs.extend(band_demote_reasons.iter().cloned());
            obj.insert("reasons".into(), json!(rs));
            obj.insert("band_demoted_from".into(), json!(truth.real_band));
        }
    }
    let mut diagnostics_out = diagnostics;
    if let Some(obj) = diagnostics_out.as_object_mut() {
        obj.insert("real_band".into(), json!(real_band));
        if !band_demote_reasons.is_empty() {
            obj.insert("band_demote_reasons".into(), json!(band_demote_reasons));
        }
        // Shared-algorithm: fingerprint for analyze reuse (skip re-evaluate when unchanged).
        let sfp = crate::session_ticket::silicon_materials_fingerprint(&fields, None);
        obj.insert("silicon_materials_fp".into(), json!(sfp));
        // Surface client-exec summary for ops/diagnostics without replacing root field.
        obj.insert(
            "client_execution".into(),
            json!({
                "status": client_exec_status,
                "implication": client_execution.get("implication"),
                "product_hint": client_execution.get("product_hint"),
                "evidence_quality": client_execution.pointer("/probe_summary/evidence_quality"),
            }),
        );
        // Structured cross-realm diff + conflict graph for ops/diagnostics (§4.2).
        if realm_conflict_pressure > 0.0 {
            obj.insert("realm_conflict_graph".into(), realm_conflict_graph.clone());
            obj.insert(
                "realm_conflict_summary".into(),
                json!({
                    "verdict": realm_conflict_verdict,
                    "severity": realm_conflict_graph.get("severity"),
                    "pressure": realm_conflict_pressure,
                    "outlier_realm": if realm_outlier.is_empty() {
                        Value::Null
                    } else {
                        json!(realm_outlier)
                    },
                }),
            );
        }
    }

    // Compat aliases on product for older clients reading real_band inside product
    // (diagnostics is authoritative for those fields going forward).
    let mut product_out = product;
    if let Some(obj) = product_out.as_object_mut() {
        // Mirror commercial multi-segment mint onto product (SDK / lab matrix often read product only).
        if obj.get("device_id").is_none() || obj.get("device_id").map(|v| v.is_null()).unwrap_or(true)
        {
            if let Some(did) = device.get("device_id") {
                obj.insert("device_id".into(), did.clone());
            }
        }
        if obj.get("device_id_segments").is_none() {
            if let Some(segs) = device.get("device_id_segments") {
                obj.insert("device_id_segments".into(), segs.clone());
            }
        }
        if obj.get("multi_segment").is_none() {
            obj.insert(
                "multi_segment".into(),
                device
                    .get("multi_segment")
                    .cloned()
                    .unwrap_or(json!(true)),
            );
        }
        if obj.get("device_tier").is_none() {
            if let Some(t) = device.get("device_tier") {
                obj.insert("device_tier".into(), t.clone());
            }
        }
        if obj.get("device_confidence").is_none() {
            if let Some(c) = device
                .get("device_confidence")
                .or_else(|| device.get("confidence"))
            {
                obj.insert("device_confidence".into(), c.clone());
            }
        }
        obj.insert("_compat_real_band".into(), json!(real_band));
        obj.insert(
            "decision".into(),
            json!({
                "nonsensitive": decision_nonsensitive,
                "sensitive": decision_sensitive,
                "order": ["bot_veto", "has_dv", "product_scores_l4", "soft_corroboration", "sensitivity"],
                "policy_id": crate::policy::load_product_policy()
                    .get("policy_id")
                    .cloned()
                    .unwrap_or(json!("site_default_v1")),
            }),
        );
        // L4 conf: default heuristic_v0; explicit adopt may switch to calibrated_v0_guest.
        let conf_adopt = crate::conf_cal::adopt_decision_report();
        let conf_ver = crate::conf_cal::active_confidence_version();
        obj.insert(
            "confidence_calibration_ref".into(),
            crate::conf_cal::confidence_calibration_ref(),
        );
        obj.insert("confidence_adopt".into(), conf_adopt);
        obj.insert("confidence_version".into(), json!(conf_ver));
    }

    // Pagehide / min-evidence for short-visit identity emit (before return gate).
    let fo = fields.as_object().cloned().unwrap_or_default();
    let pagehide_final = fo
        .get("pagehide_flush")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || fo
            .get("behavior_pagehide")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        || fo
            .get("rpa_flush_reason")
            .and_then(|v| v.as_str())
            .map(|s| s == "pagehide" || s == "hidden" || s == "flush")
            .unwrap_or(false)
        || evidence
            .get("pagehide_flush")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    // Min evidence: any of B0/B1/B8 present OR ≥3 probe batches OR FE main core.
    let has_min_evidence = {
        let batches = evidence
            .get("batches")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut has_b0 = false;
        let mut has_b1 = false;
        let mut has_b8 = false;
        let mut n = 0usize;
        for b in &batches {
            n += 1;
            let id = b
                .get("batch_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if id == "B0_bootstrap" {
                has_b0 = true;
            }
            if id == "B1_conflict" {
                has_b1 = true;
            }
            if id == "B8_gateway" || id == "B8_gateway_early" {
                has_b8 = true;
            }
        }
        has_b0 || has_b1 || has_b8 || n >= 3
            || client_execution
                .get("has_main_core")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            || matches!(client_exec_status, "js_ok" | "js_thin")
    };

    // SDK return gate: complete+idle OR pagehide+min_evidence OR idle+min_evidence.
    let probe_complete = analysis_terminal
        || coverage
            .get("coverage_complete")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        || derived_route
            .get("stop_probe")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let last_upload_ms = evidence
        .get("last_upload_ms")
        .or_else(|| evidence.get("updated_ms"))
        .and_then(|v| v.as_i64());
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let sdk_return = crate::return_gate::should_return_identity_to_sdk_ex(
        probe_complete,
        last_upload_ms,
        now_ms,
        pagehide_final,
        has_min_evidence,
    );
    product_out = crate::return_gate::apply_identity_return_gate(&product_out, &sdk_return);

    // Page RPA analyze trigger decision (pagehide final OR ≥30s idle).
    let rpa_idle_flush = fo
        .get("rpa_idle_flush")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let last_rpa = fo
        .get("collected_at")
        .and_then(|v| v.as_i64())
        .or(last_upload_ms);
    let has_rpa = fo
        .get("behavior_early_bound")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || fo
            .get("behavior_events")
            .and_then(|v| v.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false)
        || fo
            .get("behavior_count")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
            > 0.0;
    let rpa_gate = crate::return_gate::should_analyze_page_rpa(
        pagehide_final || rpa_idle_flush,
        last_rpa,
        now_ms,
        has_rpa,
    );
    if let Some(ref mut p) = page {
        if let Some(obj) = p.as_object_mut() {
            obj.insert("rpa_analyze".into(), rpa_gate.clone());
            // Ensure page_url always present when available
            if obj.get("page_url").map(|v| v.is_null()).unwrap_or(true) {
                if let Some(u) = fo.get("page_url").or_else(|| fo.get("href")) {
                    obj.insert("page_url".into(), u.clone());
                }
            }
        }
    }

    let sid_str = session_id.as_str().unwrap_or("").to_string();
    let product_version_for_ticket = evidence
        .get("product_version")
        .or_else(|| evidence.pointer("/meta/product_version"))
        .or_else(|| evidence.pointer("/meta/version"))
        .and_then(|v| v.as_str());
    // Cool ticket only after primary B10_hw_curves + silicon materials (not B10x-only).
    let session_ticket = if sid_str.is_empty() || !evidence_has_b10(evidence) {
        None
    } else {
        issue_session_ticket_versioned(
            &sid_str,
            &fields,
            &product_out,
            None,
            product_version_for_ticket,
        )
    };

    // iss/22 control plane: Belief → Envelope → Mission → BattleLog → stop_reason
    let fp_channel_scores =
        crate::fp_channel_scores::build_fp_channel_scores(&fields, &stack);
    if let Some(obj) = product_out.as_object_mut() {
        obj.insert("fp_channel_scores".into(), fp_channel_scores.clone());
    }
    let mut belief = build_belief(&fields, &product_out, &stack, &truth, &device);
    crate::fp_channel_scores::apply_fp_channel_adversary_tag(&mut belief, &fp_channel_scores);
    let envelope = scan_capability_envelope(&fields, &belief);
    let missions = select_missions(&belief, &frontier.gaps, &envelope);
    let policy_band = policy_band_from_scores(os_s, br_s, rpa_s);

    // P1d: band-driven pack budget on route packs (must floors kept)
    let mut route_packs: Vec<Value> = derived_route
        .get("packs")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    // iss/58 B4/C: hot-bucket / low-entropy → force deepen packs
    if device
        .get("force_deepen")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let deepen: Vec<String> = device
            .pointer("/identity_governance/deepen_packs")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_else(|| {
                vec![
                    "B10x_silicon_deep".into(),
                    "B46_audio_deep".into(),
                    "B10x_silicon_ulp".into(),
                ]
            });
        crate::identity_governance::inject_deepen_packs(&mut route_packs, &deepen);
    }
    let packs_before_band = route_packs.len();
    apply_band_pack_budget(&mut route_packs, &policy_band);
    let band_trimmed = packs_before_band.saturating_sub(route_packs.len());

    // B10 SLA: force-schedule commercial curves + sandbox when missing and SLA fires.
    let mut ops_codes: Vec<Value> = Vec::new();
    if let Some(code) = b10_sla_code {
        // Severity: crawler/gateway_only short visits are expected misses → warn.
        // Human desktop with form still missing B10 after grace → error (force schedule).
        let bot = evidence
            .pointer("/analysis/bot_verdict")
            .or_else(|| evidence.get("bot_verdict"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let dig = evidence
            .pointer("/device/digest_path")
            .or_else(|| evidence.get("digest_path"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let form = evidence
            .pointer("/fields/form_class")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let sev = if bot == "crawler" || bot == "bot" || dig.contains("gateway_only") || form.is_empty()
        {
            "warn"
        } else {
            "error"
        };
        ops_codes.push(json!({
            "code": code,
            "severity": sev,
            "stage": "analyze",
            "detail": "need_b10_without_batch",
            "bot_verdict": bot,
            "digest_path": dig,
        }));
        let has_b10_pack = route_packs.iter().any(|p| {
            p.get("pack_id")
                .or_else(|| p.get("batch_id"))
                .and_then(|v| v.as_str())
                == Some("B10_hw_curves")
        });
        if !has_b10_pack && !evidence_has_b10(evidence) {
            route_packs.insert(
                0,
                json!({
                    "pack_id": "B10_hw_curves",
                    "batch_id": "B10_hw_curves",
                    "priority": 1000,
                    "reason": "b10_sla_force",
                    "schedule": "static",
                    "layer": "hard",
                }),
            );
            // Also nudge sandbox for multi-source residual.
            let has_b7 = route_packs.iter().any(|p| {
                p.get("pack_id")
                    .or_else(|| p.get("batch_id"))
                    .and_then(|v| v.as_str())
                    == Some("B7_sandbox")
            });
            if !has_b7 {
                route_packs.insert(
                    1.min(route_packs.len()),
                    json!({
                        "pack_id": "B7_sandbox",
                        "batch_id": "B7_sandbox",
                        "priority": 900,
                        "reason": "b10_sla_force_sandbox",
                        "schedule": "static",
                        "layer": "hard",
                    }),
                );
            }
            ops_codes.push(json!({
                "code": "b10_sla_force_schedule",
                "severity": "warn",
                "stage": "analyze",
                "detail": "B10_hw_curves+B7_sandbox injected",
            }));
        }
        // Reflect into derived_route packs for FE clients that read route_plan only.
        if let Some(obj) = derived_route.as_object_mut() {
            obj.insert("packs".into(), json!(route_packs.clone()));
            obj.insert("b10_sla".into(), json!(true));
        }
    }
    // EDH research gate: after B10, force silicon B10x packs until they land.
    // Never mark schedule final while required B10x silicon is missing.
    let b10x_missing = crate::edh::missing_b10x_silicon(evidence);
    let has_b10_now = evidence_has_b10(evidence);
    if has_b10_now && !b10x_missing.is_empty() {
        for (i, pid) in b10x_missing.iter().enumerate() {
            let already = route_packs.iter().any(|p| {
                p.get("pack_id")
                    .or_else(|| p.get("batch_id"))
                    .and_then(|v| v.as_str())
                    == Some(pid.as_str())
            });
            if already {
                continue;
            }
            route_packs.insert(
                i.min(route_packs.len()),
                json!({
                    "pack_id": pid,
                    "batch_id": pid,
                    "priority": 1090 - i as i64,
                    "reason": "edh_b10x_silicon_must_land",
                    "schedule": "dynamic",
                    "layer": "hard",
                    "pack_lane": "deepen",
                    "hard_eligible": true,
                    "force_recollect": false,
                }),
            );
        }
        if let Some(obj) = derived_route.as_object_mut() {
            obj.insert("packs".into(), json!(route_packs.clone()));
            obj.insert("b10x_must_land".into(), json!(true));
            obj.insert("b10x_missing".into(), json!(b10x_missing.clone()));
        }
        // Ops severity: mid-flight schedule is info (FE still uploading B10x).
        // Warn only when sticky — high rev, pagehide/final, or long after B10.
        // Prod v145: 89/91 warn sessions later landed B10x → warn flood hid real misses.
        let ev_rev = evidence
            .get("analysis_rev")
            .or_else(|| evidence.pointer("/meta/analysis_rev"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let pagehide_ops = pagehide_final
            || evidence
                .pointer("/fields/pagehide_final")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
        let created_ops = evidence
            .get("created_ms")
            .or_else(|| evidence.pointer("/meta/created_ms"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let age_ops = if created_ops > 0 {
            now_ms.saturating_sub(created_ops)
        } else {
            0
        };
        // Sticky warn when long-lived / multi-rev still missing B10x.
        // Pagehide with *partial* B10x (e.g. deep landed, rint/noderiv missing) is
        // expected on short visits — keep as info so crawler/bounce noise ≠ quality fail.
        // Full miss on pagehide (no B10x at all) stays sticky warn.
        // Required trio only (ulp/noderiv/rint); deep is preferred deepen, not in missing[].
        let b10x_full_set = crate::edh::b10x_silicon_required_packs().len();
        let partial_b10x =
            !b10x_missing.is_empty() && b10x_missing.len() < b10x_full_set;
        let sticky_miss = if pagehide_ops && partial_b10x {
            false
        } else {
            pagehide_ops || ev_rev >= 3 || age_ops >= 20_000
        };
        if sticky_miss {
            ops_codes.push(json!({
                "code": "b10x_must_land",
                "severity": "warn",
                "stage": "analyze",
                "detail": format!(
                    "missing={}; sticky=1; rev={}; age_ms={}; pagehide={}",
                    b10x_missing.join(","),
                    ev_rev,
                    age_ops,
                    pagehide_ops as u8
                ),
            }));
        } else {
            ops_codes.push(json!({
                "code": "b10x_must_land_scheduled",
                "severity": "info",
                "stage": "analyze",
                "detail": format!(
                    "missing={}; packs_injected=1; rev={}; age_ms={}; pagehide={}; partial={}",
                    b10x_missing.join(","),
                    ev_rev,
                    age_ops,
                    pagehide_ops as u8,
                    partial_b10x as u8
                ),
            }));
        }
    }
    // Secondary silicon / infra deepen (iss/54 P5/P7/P8): schedule after B10 when materials missing.
    // Concurrency: B18=gpu, B46=audio, B47=cpu — may run in parallel across classes.
    // Not terminal-blocking (honest skip ok for no adapter / no COOP).
    if has_b10_now {
        let fo = evidence
            .get("fields")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        let mut present_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        if let Some(arr) = evidence.get("batches").and_then(|v| v.as_array()) {
            for b in arr {
                if let Some(id) = b
                    .get("batch_id")
                    .or_else(|| b.get("pack_id"))
                    .and_then(|v| v.as_str())
                {
                    present_ids.insert(id.to_string());
                }
            }
        }
        for p in &route_packs {
            if let Some(id) = p
                .get("pack_id")
                .or_else(|| p.get("batch_id"))
                .and_then(|v| v.as_str())
            {
                present_ids.insert(id.to_string());
            }
        }
        // Secondary candidates: (pack_id, priority, reason, force_recollect)
        // force_recollect when dense stub already claimed batch_id without real materials.
        let mut secondary: Vec<(&str, i64, &str, bool)> = Vec::new();
        // B47 first (cpu ∥) — timing foundation; always land ok or honest skip
        // iss/65: never force_recollect secondary every analyze (wire storms).
        let has_sab = fo.get("sab_clock_ok").is_some() || fo.get("sab_clock_skip").is_some();
        if !has_sab && !present_ids.contains("B47_sab_clock") {
            secondary.push(("B47_sab_clock", 980, "infra_sab_clock_p8", false));
        }
        // B18 WebGPU f32+f16 when no compute residual yet.
        // Dense densify may have emitted B18_webgpu with deferred_to_b18 / adapter-only —
        // treat that as incomplete and force_recollect real mid compute.
        // Terminal mid B18 (compute_ok/skip/f16_*) must not re-loop forever.
        let has_webgpu_silicon = fo.get("hw_curve_webgpu").is_some()
            || fo.get("webgpu_compute_ok").and_then(|v| v.as_bool()) == Some(true)
            || fo.get("webgpu_f16_ok").and_then(|v| v.as_bool()) == Some(true);
        let webgpu_mid_terminal = fo.get("webgpu_compute_ok").is_some()
            || fo.get("webgpu_compute_skip").is_some()
            || fo.get("webgpu_f16_ok").is_some()
            || fo.get("webgpu_f16_skip").is_some()
            || fo.get("webgpu_algo").and_then(|v| v.as_str()).map(|s| s.contains("f32") || s.contains("compute")).unwrap_or(false);
        let webgpu_permanently_gone = fo
            .get("webgpu_adapter_skip")
            .and_then(|v| v.as_str())
            .map(|s| s == "no_gpu" || s == "cached_none")
            .unwrap_or(false)
            || fo.get("webgpu_skip").is_some();
        let dense_webgpu_stub = present_ids.contains("B18_webgpu")
            && !has_webgpu_silicon
            && !webgpu_mid_terminal
            && (fo
                .get("webgpu_adapter_skip")
                .and_then(|v| v.as_str())
                .map(|s| s == "deferred_to_b18" || s == "null" || s == "cached")
                .unwrap_or(false)
                || fo.get("dense_pack").and_then(|v| v.as_str()) == Some("B18_webgpu")
                || (fo.get("webgpu_available").and_then(|v| v.as_bool()) == Some(true)
                    && fo.get("webgpu_compute_ok").is_none()
                    && fo.get("hw_curve_webgpu").is_none()));
        if !has_webgpu_silicon && !webgpu_permanently_gone && !webgpu_mid_terminal {
            // Dense stub only: one force is allowed via FE force budget; brain schedules once.
            if !present_ids.contains("B18_webgpu") {
                secondary.push(("B18_webgpu", 970, "secondary_webgpu_p5", false));
            } else if dense_webgpu_stub {
                // incomplete dense stub — schedule recollect flag but FE caps to 1×
                secondary.push((
                    "B18_webgpu",
                    970,
                    "secondary_webgpu_p5_force_after_dense",
                    true,
                ));
            }
        }
        // B46 audio deep + convolver when shallow/missing audio
        let has_audio_deep = fo.get("audio_deep_curve").is_some()
            || fo.get("audio_convolver_ok").and_then(|v| v.as_bool()) == Some(true)
            || fo.get("audio_deep_skip").is_some();
        if !has_audio_deep && !present_ids.contains("B46_audio_deep") {
            secondary.push(("B46_audio_deep", 960, "secondary_audio_deep_p7", false));
        }
        // Advanced silicon deep after core B10x trio (preferred, not hard-required)
        let has_deep = present_ids.contains("B10x_silicon_deep")
            || fo
                .get("b10x_pack")
                .and_then(|v| v.as_str())
                .map(|s| s.contains("silicon_deep"))
                .unwrap_or(false)
            || fo
                .get("multipath_profile")
                .and_then(|v| v.as_str())
                .map(|s| s == "silicon_deep")
                .unwrap_or(false);
        if !has_deep && b10x_missing.is_empty() && !present_ids.contains("B10x_silicon_deep") {
            secondary.push((
                "B10x_silicon_deep",
                1050,
                "preferred_silicon_deep_p1_p4",
                false,
            ));
        }
        for (i, (pid, prio, reason, force_rec)) in secondary.iter().enumerate() {
            if present_ids.contains(*pid) && !*force_rec {
                continue;
            }
            present_ids.insert((*pid).to_string());
            route_packs.insert(
                (b10x_missing.len() + i).min(route_packs.len()),
                json!({
                    "pack_id": pid,
                    "batch_id": pid,
                    "priority": prio,
                    "reason": reason,
                    "schedule": "dynamic",
                    "layer": if pid.starts_with("B10x_") { "hard" } else { "mid" },
                    "pack_lane": "deepen",
                    "hard_eligible": pid.starts_with("B10x_") || *pid == "B18_webgpu",
                    "force_recollect": force_rec,
                }),
            );
        }
        if !secondary.is_empty() {
            if let Some(obj) = derived_route.as_object_mut() {
                obj.insert("packs".into(), json!(route_packs.clone()));
                obj.insert(
                    "secondary_silicon_infra".into(),
                    json!(secondary.iter().map(|(p, _, _, _)| *p).collect::<Vec<_>>()),
                );
            }
            ops_codes.push(json!({
                "code": "secondary_silicon_infra_scheduled",
                "severity": "info",
                "stage": "analyze",
                "detail": secondary.iter().map(|(p, _, r, f)| format!("{p}:{r}{}", if *f { ":force" } else { "" })).collect::<Vec<_>>().join(","),
            }));
        }
    }

    // Stale skip_session_probe on thin sessions: should_skip already returns false, but
    // ops noise was flooding warn (~30–40/100 sess). Emit info once-style (analyze path)
    // and never treat as error — FE already force-reprobes cool_without_silicon.
    if session_ticket.is_none()
        && evidence
            .get("fields")
            .map(|f| !crate::session_ticket::has_silicon_materials(f))
            .unwrap_or(true)
        && !skip_session
    {
        let meta_skip = evidence
            .pointer("/meta/skip_session_probe")
            .or_else(|| evidence.get("skip_session_probe"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if meta_skip {
            ops_codes.push(json!({
                "code": "ticket_early_cool_blocked",
                "severity": "info",
                "stage": "analyze",
                "detail": "stale_skip_cleared_no_silicon; full probe path",
            }));
        }
    }

    // Prefer stop is reserved for true terminal coverage — never for "risk high" alone.
    // Missing fields demote OS/BR/RPA/device scores; other batches must keep running.
    let prefer_stop = policy_band
        .get("prefer_stop")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mut coverage_complete = coverage
        .get("coverage_complete")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let open_gap_n = product_out
        .pointer("/open_verification_gaps/open_count")
        .or_else(|| product_out.pointer("/analysis_quality/open_gap_count"))
        .and_then(|v| v.as_u64())
        .or_else(|| {
            product_out
                .pointer("/open_verification_gaps/open_gaps")
                .and_then(|v| v.as_array())
                .map(|a| a.len() as u64)
        })
        .unwrap_or(0);
    let mut stop_probe = frontier.stop_probe;
    // Maximize-probe policy: never stop early on "enough for device_id".
    // Only force-stop when band prefers stop AND coverage complete AND no open verify gaps
    // and route has nothing left to schedule. High-risk band must not abort mid/deep.
    if prefer_stop && coverage_complete && open_gap_n == 0 && route_packs.is_empty() {
        stop_probe = true;
    } else if prefer_stop && open_gap_n > 0 {
        // Keep probing: gaps still need materials (missing field impact is in scores only).
        stop_probe = false;
    }
    // Incomplete coverage → always keep probing (re-arm packs for FE + cold→hot reopen).
    if !coverage_complete {
        stop_probe = false;
    }
    if !route_packs.is_empty() {
        stop_probe = false;
    }
    // EDH: never terminal while required silicon B10x packs are still missing after B10.
    let b10x_incomplete = evidence_has_b10(evidence)
        && !crate::edh::missing_b10x_silicon(evidence).is_empty();
    if b10x_incomplete {
        stop_probe = false;
        analysis_terminal = false;
    }
    // Commercial mint is a milestone only — force continue packs for comprehensive data.
    // (stop only after full brain schedule floors.)

    // Commercial identity *milestone* (dh_/dv_ + silicon): product surface may show
    // a device_id early, but this must NOT stop brain packs or soft/mid (maximize probe).
    // Cycle final is only brain schedule complete — see analysis_completes_cycle.
    let (commercial_identity_final, cif_block_reason) = {
        // iss/58: ephemeral / deepen_required / commercial_blocked never final
        let posture = device
            .get("mint_posture")
            .and_then(|v| v.as_str())
            .unwrap_or("stable");
        let blocked = device
            .pointer("/identity_governance/commercial_blocked")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || device
                .get("ephemeral")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            || posture == "ephemeral_cohort"
            || posture == "deepen_required"
            || posture == "empty_anchor";
        if blocked {
            (false, Some("governance_or_ephemeral"))
        } else {
            let did = device
                .get("device_id")
                .and_then(|v| v.as_str())
                .or_else(|| product_out.get("device_id").and_then(|v| v.as_str()))
                .unwrap_or("");
            // Ephemeral / empty / zero Lane-C head is never commercial final (iss/75 178).
            if did.is_empty() || did.starts_with("dve-") {
                (false, Some("empty_or_ephemeral_id"))
            } else if !crate::device_segments::lane_c_commercial_head_ok(did) {
                (false, Some("zero_lane_c_head"))
            } else if real_band == "crawler" || bot.verdict == "crawler" {
                // Crawler / gateway noise must not mint CIF (178 Bingbot false CIF).
                (false, Some("crawler"))
            } else {
                let dig = device
                    .get("digest_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                // Require a real commercial id body — tier=multi alone must not finalize
                // when device_id is missing/thin (178 INCOMPLETE_DEVICE_ID + CIF false-positive).
                let commercial = is_commercial_device_id(did)
                    || is_dh_id(did)
                    || is_dv_id(did);
                // Lab/178 bug: residual fields or thin digests alone set CIF without
                // primary B10_hw_curves → FE cool / coverage_complete while silicon
                // head still empty (ar/res/wg zeros). Primary B10 batch is mandatory.
                let has_primary_b10 = evidence_has_b10(evidence);
                let residual_ok = fo
                    .get("hw_curve_webgl")
                    .map(|v| !v.is_null())
                    .unwrap_or(false)
                    || fo.get("residual_mean").and_then(|v| v.as_f64()).is_some()
                    || fo.get("residual_std").and_then(|v| v.as_f64()).is_some()
                    || dig.contains("real_curves");
                let silicon = has_primary_b10 && residual_ok;
                // Commercial Model-ID must be texture-class / named / soft — never cores-only.
                let mk_info = device
                    .get("hw_model_key_info")
                    .cloned()
                    .or_else(|| fo.get("hw_model_key_info").cloned())
                    .unwrap_or_else(|| {
                        crate::model_key::model_key_from_fields(&Value::Object(fo.clone()))
                    });
                let k_ready = crate::model_key::commercial_model_key_ready(&mk_info);
                // Same-browser reliability: require Lane-C path+curve materials before CIF
                // so incomplete B10x packs cannot freeze a half-mint (force-reprobe lab).
                let lane_c_ready = device
                    .get("lane_c_materials_ready")
                    .and_then(|v| v.as_bool())
                    .unwrap_or_else(|| {
                        crate::device_segments::lane_c_materials_ready(&fo)
                    });
                if !commercial {
                    (false, Some("non_commercial_id"))
                } else if !has_primary_b10 {
                    (false, Some("primary_b10_missing"))
                } else if !silicon {
                    (false, Some("silicon_incomplete"))
                } else if !k_ready {
                    (false, Some("k_not_ready"))
                } else if !lane_c_ready {
                    (false, Some("lane_c_materials_not_ready"))
                } else {
                    (true, None)
                }
            }
        }
    };
    // Hard final for cycle cool (178): commercial silicon ready + primary B10 + B10x satisfied.
    // Optional mid packs must not block cool (production had 0% complete).
    // Still never terminal on thin mint (commercial_identity_final requires silicon).
    let hard_identity_ready = commercial_identity_final
        && evidence_has_b10(evidence)
        && !b10x_incomplete
        && crate::complete_on_commercial_silicon();
    if hard_identity_ready {
        analysis_terminal = true;
        stop_probe = true;
        coverage_complete = true;
        if let Some(o) = coverage.as_object_mut() {
            o.insert("coverage_complete".into(), json!(true));
            o.insert("hard_identity_ready".into(), json!(true));
            o.insert("has_primary_b10".into(), json!(true));
        }
    } else if !evidence_has_b10(evidence) {
        // Never advertise coverage/terminal complete without primary residual pack.
        analysis_terminal = false;
        stop_probe = false;
        if coverage_complete {
            coverage_complete = false;
            if let Some(o) = coverage.as_object_mut() {
                o.insert("coverage_complete".into(), json!(false));
                o.insert("blocked_reason".into(), json!("primary_b10_missing"));
            }
        }
    }
    // v5.8.53: never clear route_packs / force stop solely because dh_ exists.

    let packs_empty = route_packs.is_empty();
    let mut stop_reason = classify_stop_reason(
        stop_probe,
        coverage_complete,
        packs_empty,
        &envelope,
        &belief,
    );
    if prefer_stop && stop_probe {
        stop_reason = "policy_band";
    }

    let mut battle_log = build_battle_log(
        &sid_str,
        frontier.plan_version,
        &belief,
        &envelope,
        &missions,
        &frontier.gaps,
        &route_packs,
        stop_reason,
        &policy_band,
    );
    if let Some(obj) = battle_log.as_object_mut() {
        obj.insert("fp_channel_scores".into(), fp_channel_scores.clone());
        let ca = device
            .get("composite_association")
            .cloned()
            .unwrap_or_else(|| {
                crate::composite_association::self_association_readiness(&fields)
            });
        obj.insert("composite_association".into(), ca);
    }
    let unknown_bucket = unknown_bucket_from_belief(&belief, &envelope);

    // Direction learning update (session-local prior for next tick)
    let scheduled_dirs: Vec<String> = route_packs
        .iter()
        .filter_map(|p| {
            p.get("direction_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let prev_priors = evidence
        .pointer("/meta/direction_priors")
        .or_else(|| evidence.pointer("/session_meta/direction_priors"));
    let yield_ok = envelope
        .get("in_range")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let direction_priors = update_direction_priors(prev_priors, &scheduled_dirs, yield_ok);

    let evidence_rev = evidence
        .get("evidence_rev")
        .or_else(|| evidence.pointer("/meta/evidence_rev"))
        .and_then(|v| v.as_i64())
        .unwrap_or(frontier.plan_version as i64);

    // Enrich route_plan
    if let Some(obj) = derived_route.as_object_mut() {
        // Inline-injected packs (b10_sla / EDH deepen) carry dag_v2 too, so the
        // FE engine gate and the sealed pack-set decision see every scheduled pack.
        for p in route_packs.iter_mut() {
            crate::brain::ensure_pack_dag_v2(p);
        }
        obj.insert("packs".into(), json!(route_packs));
        obj.insert(
            "mission_id".into(),
            missions
                .get("primary")
                .cloned()
                .unwrap_or(json!("hold")),
        );
        obj.insert("stop_probe".into(), json!(stop_probe));
        obj.insert("stop_reason".into(), json!(if hard_identity_ready {
            "hard_identity_ready"
        } else {
            stop_reason
        }));
        obj.insert(
            "policy_band".into(),
            policy_band.get("band").cloned().unwrap_or(json!("mid")),
        );
        obj.insert(
            "envelope_in_range".into(),
            envelope.get("in_range").cloned().unwrap_or(json!(true)),
        );
        obj.insert("evidence_rev".into(), json!(evidence_rev));
        obj.insert("band_packs_trimmed".into(), json!(band_trimmed));
        obj.insert("hard_identity_ready".into(), json!(hard_identity_ready));
        obj.insert("coverage_complete".into(), json!(coverage_complete));
        // iss/72: plan_epoch for FE SessionScheduler stale rejection (monotone with plan_version).
        obj.insert("plan_epoch".into(), json!(frontier.plan_version));
        obj.insert("plan_version".into(), json!(frontier.plan_version));
        // Re-assert EDH must-land flags after any pack list mutation (cool-safe path).
        let b10x_missing_final = crate::edh::missing_b10x_silicon(evidence);
        if evidence_has_b10(evidence) && !b10x_missing_final.is_empty() {
            obj.insert("b10x_must_land".into(), json!(true));
            obj.insert("b10x_missing".into(), json!(b10x_missing_final));
        } else if evidence_has_b10(evidence) {
            obj.insert("b10x_must_land".into(), json!(false));
            obj.insert("b10x_missing".into(), json!([]));
        }
        // iss/61 F1: ship fuzzy Helper Data in route_plan so FE can persist it
        // (localStorage) and echo it next session for cross-tick stabilization.
        let fh = crate::fuzzy_ecc::helper_route_payload(&fields);
        if fh.as_object().map(|o| !o.is_empty()).unwrap_or(false) {
            obj.insert("fuzzy_helper".into(), fh);
        }
        // iss/opus5 03-P0-1: planned-vs-actual evidence ledger. The issued pack
        // set is the server-side expectation; the seal below binds it, so the
        // FE engine gate and the sealed pack-set decision stay answerable.
        let expected_ids = crate::evidence_ledger::expected_pack_ids(&Value::Object(obj.clone()));
        let received_ids = crate::evidence_ledger::received_batch_ids(evidence);
        obj.insert(
            "evidence_missing".into(),
            crate::evidence_ledger::missing_ledger(&expected_ids, &received_ids),
        );
        let seal = sign_route_plan(&Value::Object(obj.clone()), &sid_str, evidence_rev);
        obj.insert("route_seal".into(), seal);
    }
    let mut brain_out = frontier.to_value().map_err(|e| e.to_string())?;
    if let Some(obj) = brain_out.as_object_mut() {
        obj.insert("missions".into(), missions.clone());
        obj.insert("stop_reason".into(), json!(stop_reason));
        obj.insert("stop_probe".into(), json!(stop_probe));
        obj.insert("packs".into(), json!(route_packs.clone()));
    }

    // iss/36 D-2 · atlas E-13/E-16: finalize recommended_action with stop/envelope; battle_log gate.
    let env_in_range = envelope
        .get("in_range")
        .and_then(|v| v.as_bool());
    let recommended_action = crate::decision::derive_recommended_action(
        &product_out,
        &device,
        bot.verdict.as_str(),
        Some(stop_reason),
        env_in_range,
    );
    if let Some(obj) = product_out.as_object_mut() {
        obj.insert("recommended_action".into(), recommended_action.clone());
        // Mirror root bot onto product for thin/SDK readers (FP-style path).
        obj.insert("bot".into(), bot.to_value());
        // iss/58: surface identity confidence + mint posture for SDK/ops (not UV SLA)
        if let Some(id) = device.get("identity") {
            obj.insert("identity".into(), id.clone());
        }
        if let Some(mp) = device.get("mint_posture") {
            obj.insert("mint_posture".into(), mp.clone());
        }
        if let Some(e) = device.get("ephemeral") {
            obj.insert("ephemeral".into(), e.clone());
        }
        if let Some(eb) = device.get("effective_bits") {
            obj.insert("effective_bits".into(), eb.clone());
        }
        if let Some(bh) = device.get("bucket_heat") {
            obj.insert("bucket_heat".into(), bh.clone());
        }
        if let Some(ig) = device.get("identity_governance") {
            obj.insert("identity_governance".into(), ig.clone());
        }
        // E-16: battle_log readability on product surface (stop_reason never empty string).
        let bl_stop = battle_log
            .get("stop_reason")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(stop_reason);
        obj.insert(
            "degradation_gate".into(),
            json!({
                "battle_log_schema": battle_log.get("schema").cloned().unwrap_or(json!("gr_battle_log_v1")),
                "stop_reason": bl_stop,
                "stop_reason_nonempty": !bl_stop.is_empty(),
                "unknown_bucket_present": unknown_bucket.as_ref().map(|v| v.is_object()).unwrap_or(false),
                "envelope_in_range": env_in_range,
            }),
        );
        // Re-assert multi-segment mint after return_gate slim (SDK product surface).
        if obj.get("multi_segment").map(|v| v.is_null()).unwrap_or(true)
            || obj.get("multi_segment").is_none()
        {
            let multi = device
                .get("multi_segment")
                .and_then(|v| v.as_bool())
                .unwrap_or_else(|| {
                    device
                        .get("device_tier")
                        .and_then(|v| v.as_str())
                        .map(|t| t == "multi")
                        .unwrap_or(false)
                });
            obj.insert("multi_segment".into(), json!(multi));
        }
        if obj.get("device_id").map(|v| v.is_null() || v.as_str() == Some("")).unwrap_or(true) {
            if let Some(did) = device.get("device_id") {
                if !did.is_null() {
                    obj.insert("device_id".into(), did.clone());
                }
            }
        }
        if obj.get("device_id_segments").map(|v| v.is_null()).unwrap_or(true) {
            if let Some(segs) = device.get("device_id_segments") {
                obj.insert("device_id_segments".into(), segs.clone());
            }
        }
        if obj.get("device_tier").map(|v| v.is_null()).unwrap_or(true) {
            if let Some(t) = device.get("device_tier") {
                obj.insert("device_tier".into(), t.clone());
            }
        }
    }

    // Soft probe_complete follows coverage/brain terminal, not dh_ alone.
    let probe_complete = probe_complete || analysis_terminal;

    // Enrich link with minted device_id + machine bind promotion (webkit "未关联" fix).
    let mut link_out = link_result.as_ref().map(|l| l.to_value());
    let minted_did = device
        .get("device_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let mut device_link_bound = false;
    if let Some(ref mut lv) = link_out {
        let hard = lv
            .get("hard_matches")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let mut dec = lv
            .get("decision")
            .and_then(|v| v.as_str())
            .unwrap_or("POSSIBLE")
            .to_string();
        let has_core = lv
            .pointer("/details/has_machine_core")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if dec == "POSSIBLE" && hard >= 5 && has_core {
            dec = "MACHINE_BOUND".into();
            if let Some(o) = lv.as_object_mut() {
                o.insert("decision".into(), json!("MACHINE_BOUND"));
            }
        }
        if let Some(det) = lv.get_mut("details").and_then(|d| d.as_object_mut()) {
            if det
                .get("device_id_a")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .is_empty()
                && (is_dh_id(&minted_did) || is_dv_id(&minted_did))
            {
                det.insert("device_id_a".into(), json!(minted_did.clone()));
            }
            if dec == "MACHINE_BOUND" {
                det.insert("promoted_from".into(), json!("POSSIBLE"));
            }
            let bound = dec == "LINK" || dec == "MACHINE_BOUND";
            det.insert("device_link_bound".into(), json!(bound));
            device_link_bound = bound;
        } else {
            device_link_bound = dec == "LINK" || dec == "MACHINE_BOUND";
        }
    }
    let brain_schedule_final = !b10x_incomplete
        && (analysis_terminal
            || hard_identity_ready
            || (stop_probe && coverage_complete && route_packs.is_empty()));
    // SDK consumption (iss/48): never treat thin multi-segment mint as auto-linkable.
    let identity_state = {
        let did = device
            .get("device_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let coll = device
            .get("collision_risk")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let conf = device
            .get("device_confidence")
            .or_else(|| device.get("confidence"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        if did.is_empty() {
            "not_linkable"
        } else if coll || conf < 0.35 {
            "provisional"
        } else if commercial_identity_final || conf >= 0.55 {
            "linkable"
        } else {
            "provisional"
        }
    };
    // Three terminal surfaces (product must not collapse these).
    let terminal_states = json!({
        "commercial_identity_final": commercial_identity_final,
        "cif_block_reason": cif_block_reason,
        "brain_schedule_final": brain_schedule_final,
        "device_link_bound": device_link_bound,
        "identity_state": identity_state,
        "note": "commercial mint ≠ schedule cool ≠ cross-session link bound",
    });
    if let Some(obj) = device.as_object_mut() {
        obj.insert(
            "cif_block_reason".into(),
            match cif_block_reason {
                Some(r) => json!(r),
                None => Value::Null,
            },
        );
    }

    // Persist-ready control snapshot (handlers write to session meta)
    let control_persist = json!({
        "belief": belief,
        "missions": missions,
        "capability_envelope": envelope,
        "battle_log": battle_log,
        "policy_band": policy_band,
        "stop_reason": stop_reason,
        "direction_priors": direction_priors,
        "unknown_bucket": unknown_bucket,
        "plan_version": frontier.plan_version,
        "plan_epoch": frontier.plan_version,
        "evidence_rev": evidence_rev,
        "recommended_action": recommended_action,
        "commercial_identity_final": commercial_identity_final,
        "brain_schedule_final": brain_schedule_final,
        "device_link_bound": device_link_bound,
        "terminal_states": terminal_states,
    });

    let edh_out = crate::edh::build_edh(evidence, &device);
    // Compact secondary HW / infra surface for lab matrix & SDK (fields often not
    // persisted on slim analysis). Honest skip is success for B47 COOP/COEP, B18 no GPU.
    let secondary_hw = json!({
        "algo": "secondary_hw_surface_v1",
        "b47_sab": {
            "ok": fo.get("sab_clock_ok").cloned().unwrap_or(Value::Null),
            "skip": fo.get("sab_clock_skip").cloned().unwrap_or(Value::Null),
            "digest": fo.get("sab_clock_digest").cloned().unwrap_or(Value::Null),
            "landed": fo.get("sab_clock_ok").is_some() || fo.get("sab_clock_skip").is_some(),
        },
        "b18_webgpu": {
            "compute_ok": fo.get("webgpu_compute_ok").cloned().unwrap_or(Value::Null),
            "compute_skip": fo.get("webgpu_compute_skip").cloned().unwrap_or(Value::Null),
            "f16_ok": fo.get("webgpu_f16_ok").cloned().unwrap_or(Value::Null),
            "f16_skip": fo.get("webgpu_f16_skip").cloned().unwrap_or(Value::Null),
            "skip": fo.get("webgpu_skip").cloned().unwrap_or(Value::Null),
            "available": fo.get("webgpu_available").cloned().unwrap_or(Value::Null),
            "landed": fo.get("webgpu_compute_ok").is_some()
                || fo.get("webgpu_compute_skip").is_some()
                || fo.get("webgpu_skip").is_some()
                || fo.get("hw_curve_webgpu").is_some(),
        },
        "b46_audio": {
            "convolver_ok": fo.get("audio_convolver_ok").cloned().unwrap_or(Value::Null),
            "deep_skip": fo.get("audio_deep_skip").cloned().unwrap_or(Value::Null),
            "deep_hash": fo.get("audio_deep_hash").cloned().unwrap_or(Value::Null),
            "landed": fo.get("audio_deep_curve").is_some()
                || fo.get("audio_convolver_ok").is_some()
                || fo.get("audio_deep_skip").is_some(),
        },
        "b10x_deep": {
            "pack": fo.get("b10x_pack").cloned().unwrap_or(Value::Null),
            "profile": fo.get("multipath_profile").cloned().unwrap_or(Value::Null),
            "ok": fo.get("b10x_ok").cloned().unwrap_or(Value::Null),
            "landed": fo.get("b10x_pack").is_some()
                || fo.get("multipath_profile").is_some(),
        },
    });
    if let Some(obj) = product_out.as_object_mut() {
        obj.insert("secondary_hw".into(), secondary_hw.clone());
        obj.insert("commercial_identity_final".into(), json!(commercial_identity_final));
        if let Some(r) = cif_block_reason {
            obj.insert("cif_block_reason".into(), json!(r));
        }
    }
    Ok(json!({
        "session_id": session_id,
        "product_version": crate::GR_PRODUCT_VERSION,
        "fe_version": crate::GR_FE_VERSION,
        "strategy": strat.to_value(),
        "layer_coverage": cov,
        "coverage": coverage,
        // Terminal only when brain coverage/schedule says so (not dh_ milestone).
        "analysis_terminal": analysis_terminal,
        // Milestone: commercial materials present; does NOT imply halt/cool.
        "commercial_identity_final": commercial_identity_final,
        "cif_block_reason": cif_block_reason,
        "secondary_hw": secondary_hw,
        "identity_state": identity_state,
        "brain_schedule_final": brain_schedule_final,
        "device_link_bound": device_link_bound,
        "terminal_states": terminal_states,
        // Soft: coverage_complete or terminal. Does NOT alone mean halt_uploads.
        "probe_complete": probe_complete,
        "stop_probe": stop_probe,
        "sdk_return": sdk_return,
        "rpa_analyze": rpa_gate,
        "bot": bot.to_value(),
        "truth": truth_out,
        "real_band": real_band,
        "xsrc_status": truth.xsrc_status,
        "credibility": truth.credibility,
        "device": device,
        "device_hypothesis": edh_out.clone(),
        "edh": edh_out,
        "product": product_out,
        "diagnostics": diagnostics_out,
        "page": page,
        "session_ticket": session_ticket,
        "skip_session_probe": skip_session,
        "ops_events": ops_codes,
        "b10_present": evidence_has_b10(evidence),
        "b10_sla": b10_sla_code,
        "link": link_out,
        // Multi-algo ensemble (never single digest / single associate as sole truth).
        // Uses session fields (not project_device-local fields_v); pair needs peer_vector.
        "device_ensemble": {
            "single": fuse_single_ensemble(&fields),
            "pair": peer_vector.map(|p| fuse_pair_ensemble(&fields, p)),
        },
        "soft": soft,
        "brain": brain_out,
        "route_plan": derived_route,
        "route_authority": route_authority,
        "capabilities": capabilities,
        "static_wave": static_wave,
        "plan_version": frontier.plan_version,
        "plan_epoch": frontier.plan_version,
        "short_visit": short_visit,
        "client_execution": client_execution,
        // iss/22 control plane
        "belief": belief,
        "capability_envelope": envelope,
        "missions": missions,
        "policy_band": policy_band,
        "battle_log": battle_log,
        "stop_reason": stop_reason,
        "unknown_bucket": unknown_bucket,
        "direction_priors": direction_priors,
        "control_persist": control_persist,
        "defaults": {
            "bot_algo": bot.algo,
            "link_algo": link_algo,
        },
        "route_plan_valid": route_ok,
        "route_plan_error": route_err,
        "soft_promote": PROMOTE_TO_COMMERCIAL_ID,
        "task_gaps": frontier.gaps.iter().map(|g| g.to_value()).collect::<Vec<_>>(),
    }))
}

/// True when product leg reasons cite *abnormal env/automation* (not mere coverage_cap).
fn product_leg_has_abnormal_evidence(product: &Value, leg: &str) -> bool {
    let Some(arr) = product
        .pointer(&format!("/{leg}/reasons"))
        .and_then(|v| v.as_array())
    else {
        return false;
    };
    // Substring match carefully: "automation" alone must NOT match mat:automation family hits.
    const MARKERS: &[&str] = &[
        "vm_score",
        "soft_stack",
        "residual_soft_like",
        "software_renderer",
        "stack_class=vm",
        "stack_class=soft",
        "webdriver",
        "bot_verdict=bot",
        "bot_verdict=automation",
        "bot_verdict=crawler",
        "bot_verdict=suspect",
        "spoof_score",
        "spoof=",
        "headless",
        "chrome_ua_without_runtime",
        "plugins_empty",
        "no_server_side_cap",
        "automation.",
        "automation_globals",
        "automation_suspect",
    ];
    for r in arr {
        let s = r.as_str().unwrap_or("").to_ascii_lowercase();
        // coverage_cap alone is thin-evidence, not abnormal env
        if s.starts_with("coverage_cap") || s.starts_with("mat:") || s.starts_with("families=") {
            continue;
        }
        if s.contains("material_families") || s.contains("hedge_materials") {
            continue;
        }
        if MARKERS.iter().any(|m| s.contains(m)) {
            return true;
        }
    }
    false
}

/// Align diagnostic real_band with multi-signal product surface.
/// Prevents confirmed_real when os/br/bot already mark abnormal env/browser/automation.
/// Does **not** demote solely on coverage-cap "suspect" status (fixtures & thin-but-real paths).
/// Also honors `product.client_execution_status` after client_exec demotion.
fn demote_real_band_with_product(
    band: &str,
    bot: &BotScore,
    product: &Value,
    source_conflicts: &[String],
) -> (String, Vec<String>) {
    let mut reasons = Vec::new();
    let mut out = band.to_string();
    if !matches!(out.as_str(), "confirmed_real" | "likely_real") {
        return (out, reasons);
    }
    let ce = product
        .get("client_execution_status")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    // No usable FE / server-edge only: never leave real-ish bands.
    if matches!(ce, "js_unavailable" | "server_only") {
        out = "insufficient".into();
        reasons.push(format!("demote_client_exec:{ce}"));
        return (out, reasons);
    }
    // Early exit / thin FE: demote confirmed → likely_real (soft), keep likely_real.
    if ce == "js_early_exit" && out == "confirmed_real" {
        out = "likely_real".into();
        reasons.push("demote_client_exec:js_early_exit".into());
        // continue other product checks may further demote
    } else if ce == "js_thin" && out == "confirmed_real" {
        out = "likely_real".into();
        reasons.push("demote_client_exec:js_thin".into());
    }

    let os_status = product
        .pointer("/os/status")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let br_status = product
        .pointer("/br/status")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let os_score = product
        .pointer("/os/score")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0);
    let br_score = product
        .pointer("/br/score")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0);
    let os_abn = product_leg_has_abnormal_evidence(product, "os");
    let br_abn = product_leg_has_abnormal_evidence(product, "br");

    if matches!(bot.verdict.as_str(), "crawler") {
        out = "crawler".into();
        reasons.push("demote_product_bot:crawler".into());
    } else if matches!(bot.verdict.as_str(), "bot" | "automation")
        || br_status == "fake"
        || br_score < 0.35
        || br_abn && br_score < 0.55
    {
        out = "likely_bot".into();
        reasons.push(format!(
            "demote_product_br:status={br_status},score={br_score:.2},bot={},abn={br_abn}",
            bot.verdict
        ));
    } else if os_status == "fake" || os_score < 0.45 || (os_abn && os_score < 0.55) {
        out = "watch".into();
        reasons.push(format!(
            "demote_product_os:status={os_status},score={os_score:.2},abn={os_abn}"
        ));
    } else if !source_conflicts.is_empty() {
        out = "watch".into();
        reasons.push("demote_source_conflict".into());
    } else if os_abn && br_abn && (os_status != "real" || br_status != "real") {
        // Dual abnormal *evidence* (not mere coverage caps) → watch
        out = "watch".into();
        reasons.push(format!(
            "demote_dual_abnormal_evidence:br={br_score:.2},os={os_score:.2}"
        ));
    } else if out == "confirmed_real" && (os_abn || br_abn) {
        // Soft demote: any abnormal product evidence (e.g. chrome_ua_without_runtime)
        // disqualifies confirmed_real even when scores are mid-high.
        out = "likely_real".into();
        reasons.push(format!(
            "demote_abnormal_evidence_soft:os_abn={os_abn},br_abn={br_abn},os={os_score:.2},br={br_score:.2}"
        ));
    }
    (out, reasons)
}

