//! Analysis quality layer: field utilization, mutual verification, confidence posture,
//! and open re-probe gaps.
//!
//! Architecture principles (product + multi-role review):
//! 1. Every probe field has an axis role — never globally discarded because it is
//!    `never_digest` for commercial device_id digest only (still reference_aux /
//!    low-weight claim-obs / contradict on br/os/rpa and device_id diag).
//! 2. Product outcomes come from multi-algorithm confirm **and** contradict signals,
//!    not a single threshold or two-field hardwire.
//! 3. Confidence = completeness × role weights; thin data → low conf + explicit posture.
//! 4. Insufficient/conflicting materials → task-targeted re-probe gaps (self-analysis loop).

use crate::brain::{scan_gaps, Gap};
use crate::link_or_mint::field_algorithm_weights;
use crate::product_matrix::load_field_product_matrix;
use crate::product_scores::{role_importance, field_axis_contributions};
use crate::stack_auth::StackAuth;
use crate::utilization::{
    channel_presence_summary, merge_policy_into_utilization, run_channel_checks,
};
use crate::xsrc::TruthResult;
use serde_json::{json, Map, Value};

/// Build machine-readable field → axes/roles/weights utilization map for present fields.
/// Includes missing critical materials so ops can see blackhole risk.
pub fn build_field_utilization_map(fields: &Value) -> Value {
    let Ok(m) = load_field_product_matrix() else {
        return json!({"error": "matrix_unavailable"});
    };
    let fo = fields.as_object().cloned().unwrap_or_default();
    let weights = field_algorithm_weights();
    let algs = weights
        .get("algorithms")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    let mut present_rows = Vec::new();
    let mut missing_critical = Vec::new();
    let mut by_axis: Map<String, Value> = Map::new();
    for ax in ["device_id", "os", "br", "rpa"] {
        by_axis.insert(ax.into(), json!({"present": [], "missing_material": []}));
    }

    for row in &m.fields {
        let present = field_present_util(&fo, &row.field);
        let is_critical = row.axes.values().any(|r| {
            matches!(r.as_str(), "material" | "veto" | "conf")
                && matches!(row.trust_tier.as_str(), "T0" | "T1" | "T2")
        });
        if !present && !is_critical {
            continue;
        }
        let mut axis_roles = Map::new();
        for (ax, role) in &row.axes {
            axis_roles.insert(ax.clone(), json!(role));
            // Annotate utilization role class
            let util_role = match role.as_str() {
                "material" => "material",
                "veto" => "veto_or_counter",
                "conf" => "conf_or_hedge",
                "diag" => "diag_low_weight",
                "exclude" => "exclude_this_axis_only",
                _ => "other",
            };
            if let Some(bucket) = by_axis.get_mut(ax).and_then(|v| v.as_object_mut()) {
                let key = if present { "present" } else { "missing_material" };
                if let Some(arr) = bucket.get_mut(key).and_then(|v| v.as_array_mut()) {
                    if role == "material" || role == "veto" || (role == "conf" && is_critical) {
                        arr.push(json!({
                            "field": row.field,
                            "role": role,
                            "util": util_role,
                            "trust_tier": row.trust_tier,
                        }));
                    }
                }
            }
        }
        // Per-algorithm weight hints when field named in weight table
        let mut algo_w = Map::new();
        for (alg, table) in &algs {
            if let Some(obj) = table.as_object() {
                // exact or prefix match keys containing field stem
                for (k, v) in obj {
                    if k == &row.field
                        || k.contains(&row.field)
                        || row.field.contains(k.trim_end_matches("_alone"))
                    {
                        algo_w.insert(format!("{alg}:{k}"), v.clone());
                    }
                }
            }
        }
        let never_digest = m
            .commercial_device_id
            .never_digest
            .iter()
            .any(|x| x == &row.field);
        let entry = json!({
            "field": row.field,
            "present": present,
            "trust_tier": row.trust_tier,
            "primary_packs": row.primary_packs,
            "axes": axis_roles,
            "commercial_never_digest": never_digest,
            "note": if never_digest {
                "commercial digest excluded; reference_aux OK (low-weight / claim-obs / contradict / diag)"
            } else {
                "eligible for axis material/conf per matrix"
            },
            "algorithm_weight_hits": algo_w,
        });
        if present {
            present_rows.push(entry);
        } else if is_critical {
            missing_critical.push(entry);
        }
    }

    // All-fields-utilized layer: policy-registered fields (no product-matrix
    // row) still get a visible consumption role — nothing from the FE stream
    // is dropped as "unused".
    let policy_registered = merge_policy_into_utilization(&fo, &mut present_rows, &mut by_axis);

    let contrib = field_axis_contributions(fields);
    json!({
        "algo": "field_utilization_v1",
        "present_count": present_rows.len(),
        "policy_registered_present": policy_registered,
        "missing_critical_count": missing_critical.len(),
        "present_sample": present_rows.into_iter().take(40).collect::<Vec<_>>(),
        "missing_critical_sample": missing_critical.into_iter().take(24).collect::<Vec<_>>(),
        "by_axis": by_axis,
        "field_axis_contributions": contrib,
        "never_digest_policy": "commercial-digest-only — reference_aux allowed; never global product ban",
        "algorithm_weights_ref": "link_or_mint::field_algorithm_weights",
        "utilization_policy": channel_presence_summary(&fo),
    })
}

fn field_present_util(fo: &Map<String, Value>, field: &str) -> bool {
    if field.contains('.') {
        let parts: Vec<&str> = field.splitn(2, '.').collect();
        if fo.get(parts[0])
            .and_then(|v| v.get(parts[1]))
            .map(|v| !v.is_null())
            .unwrap_or(false)
        {
            return true;
        }
    }
    match fo.get(field) {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(_) => true,
    }
}

/// Hard contradict score reasons only (not low-weight claim context / diag).
fn score_reason_is_hard_contradict(reason: &str) -> bool {
    // Explicit non-contradict / conf-only context (must not demote fusion).
    const CONTEXT_ONLY: &[&str] = &[
        "gpu_label_claim_context_low_weight",
        "chrome_ua_without_runtime_claim",
        "unit_surface_soft_class_context",
        "os_instance_hash_present",
        "ja4_protocol_present",
        "ja3_protocol_present",
    ];
    if CONTEXT_ONLY.iter().any(|c| reason == *c || reason.starts_with(c)) {
        return false;
    }
    // Hard claim-obs / integrity / collusion / capability conflicts from product_scores.
    const HARD_PREFIXES: &[&str] = &[
        "gpu_label_vs_residual_soft_claim_obs",
        "gpu_label_untrusted_claim_obs",
        "gpu_label_vs_residual_soft_integrity",
        "gpu_label_untrusted",
        "claim_capability_incoherence",
        "gpu_claim_vs_max_texture_incoherent",
        "gpu_claim_residual_unavailable",
        "claim_collusion_",
        "engine_claim_obs_mismatch",
        "claim_obs",
    ];
    if HARD_PREFIXES
        .iter()
        .any(|p| reason == *p || reason.starts_with(p))
    {
        return true;
    }
    // Capability incoherence alias
    if reason.contains("capability_incoherence") || reason.contains("_incoherent") {
        return true;
    }
    false
}

/// Mutual verification fusion: confirm vs contradict channels across sub-algorithms.
/// Residual-led identity is not redefined by a single weak L0 claim.
pub fn build_mutual_verification(
    fields: &Value,
    stack: &StackAuth,
    os: &Value,
    br: &Value,
    rpa: &Value,
    device: &Value,
    truth: &TruthResult,
    source_conflicts: &[String],
) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let mut confirms: Vec<Value> = Vec::new();
    let mut contradicts: Vec<Value> = Vec::new();

    // --- Residual / unit class (device primary) ---
    if fo.get("residual_mean").is_some()
        || fo
            .get("hw_curve_webgl")
            .and_then(|v| v.as_array())
            .is_some_and(|a| a.len() >= 4)
    {
        confirms.push(json!({
            "channel": "residual_class",
            "axes": ["device_id", "os"],
            "signal": "residual_or_webgl_curve_present",
            "stance": "confirm",
        }));
    } else {
        contradicts.push(json!({
            "channel": "residual_class",
            "axes": ["device_id"],
            "signal": "residual_materials_thin",
            "stance": "contradict_thin",
        }));
    }
    if fo
        .get("unit_surface_id")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty())
    {
        let ver = fo
            .get("unit_multiround_stable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            && fo
                .get("unit_surface_algo")
                .and_then(|v| v.as_str())
                .is_some_and(|a| a == "gr_unit_v1" || a.starts_with("gr_unit_v"));
        confirms.push(json!({
            "channel": "unit_surface",
            "axes": ["device_id", "os", "br"],
            "signal": if ver { "unit_versioned_stable" } else { "unit_present_conf_only" },
            "stance": "confirm",
            "versioned": ver,
        }));
    }

    // --- Claim-obs L0 vs residual (os collusion + br integrity) ---
    let ren = fo
        .get("webgl_unmasked_renderer")
        .or_else(|| fo.get("webgl_renderer"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let exotic = ren.contains("geforce")
        || ren.contains("radeon")
        || ren.contains("apple m")
        || ren.contains("rtx")
        || ren.contains("adreno");
    let soft_obs = stack.soft_stack
        || stack.residual_soft_like == Some(true)
        || fo
            .get("residual_soft_like")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    if exotic && soft_obs {
        contradicts.push(json!({
            "channel": "claim_obs_gpu",
            "axes": ["os", "br"],
            "signal": "exotic_gpu_label_vs_soft_residual",
            "stance": "contradict",
            "note": "does_not_redefine_residual_led_device_id",
        }));
    } else if exotic && !soft_obs {
        confirms.push(json!({
            "channel": "claim_obs_gpu",
            "axes": ["os", "br"],
            "signal": "gpu_label_consistent_with_residual_class",
            "stance": "confirm_weak",
        }));
    }
    if stack.gpu_label_untrusted {
        contradicts.push(json!({
            "channel": "stack_auth",
            "axes": ["os", "br", "device_id"],
            "signal": "gpu_label_untrusted",
            "stance": "contradict",
            "device_role": "conf_penalty_not_digest_primary",
        }));
    }

    // --- Capability integrity (br) ---
    if exotic {
        if fo.get("webgl2_support").and_then(|v| v.as_bool()) == Some(false) {
            contradicts.push(json!({
                "channel": "capability",
                "axes": ["br"],
                "signal": "high_end_claim_webgl2_false",
                "stance": "contradict",
            }));
        }
        if let Some(mt) = fo
            .get("webgl_max_texture")
            .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
        {
            if mt > 0.0 && mt < 8192.0 {
                contradicts.push(json!({
                    "channel": "capability",
                    "axes": ["br"],
                    "signal": "high_end_claim_low_max_texture",
                    "stance": "contradict",
                }));
            }
        }
    }

    // --- Control plane (rpa primary) ---
    let wd = fo
        .get("webdriver")
        .and_then(|v| v.as_bool())
        .or_else(|| {
            fo.get("automation")
                .and_then(|v| v.get("webdriver"))
                .and_then(|v| v.as_bool())
        })
        .unwrap_or(false);
    let cdp = fo
        .get("cdp_runtime_hint")
        .map(|v| v.as_bool().unwrap_or(v.as_f64().unwrap_or(0.0) >= 1.0))
        .unwrap_or(false);
    let headless = fo
        .get("headless_likely")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || fo
            .get("user_agent")
            .and_then(|v| v.as_str())
            .is_some_and(|u| u.contains("HeadlessChrome"));
    if wd || cdp || headless {
        contradicts.push(json!({
            "channel": "control_plane",
            "axes": ["rpa", "br"],
            "signal": if wd { "webdriver" } else if cdp { "cdp" } else { "headless" },
            "stance": "contradict_automation",
            "rpa_primary": true,
        }));
    } else if fo.get("behavior_events").is_some()
        || fo
            .get("behavior_early_bound")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    {
        confirms.push(json!({
            "channel": "control_plane",
            "axes": ["rpa"],
            "signal": "behavior_surface_without_control_flags",
            "stance": "confirm_weak",
        }));
    }

    // --- Protocol (br only) ---
    if fo.get("ja4").is_some() || fo.get("ja3").is_some() {
        confirms.push(json!({
            "channel": "protocol_edge",
            "axes": ["br"],
            "signal": "tls_fp_present",
            "stance": "confirm",
            "device_digest": false,
        }));
    }
    // --- Channel integrity (A2): B8 convergence vs server-observed edge ---
    if fo
        .get("b8_converge_without_edge")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        contradicts.push(json!({
            "channel": "channel_integrity",
            "axes": ["br", "os"],
            "signal": "b8_converge_without_edge",
            "stance": "contradict_weak",
            "device_digest": false,
            "note": "FE claims B8 dual-path convergence but no gateway edge observation exists",
        }));
    } else if fo
        .get("b8_edge_seen")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        confirms.push(json!({
            "channel": "channel_integrity",
            "axes": ["br"],
            "signal": "b8_edge_observed",
            "stance": "confirm_weak",
        }));
    }
    // --- Sandbox residual algorithm consistency (A3) ---
    match fo
        .get("sandbox_residual_algo_match")
        .and_then(|v| v.as_bool())
    {
        Some(true) => {
            confirms.push(json!({
                "channel": "sandbox_residual",
                "axes": ["device_id", "os"],
                "signal": "nest_algo_matches_main",
                "stance": "confirm",
                "note": "sandbox observed the same residual algorithm as main",
            }));
        }
        Some(false) => {
            contradicts.push(json!({
                "channel": "sandbox_residual",
                "axes": ["os"],
                "signal": "nest_algo_diverges_main",
                "stance": "contradict_weak",
                "device_digest": false,
                "note": "sandbox residual algorithm diverges from main — soft demotion only",
            }));
        }
        None => {}
    }
    // --- FE build/version integrity (A5): mid-session drift or build stamp lie ---
    if fo
        .get("fe_version_changed")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        contradicts.push(json!({
            "channel": "fe_integrity",
            "axes": ["br", "os"],
            "signal": "fe_code_version_changed_mid_session",
            "stance": "contradict_weak",
            "device_digest": false,
            "note": "FE bundle code version changed inside one session — page modified or mixed bundles",
        }));
    }
    if fo
        .get("fe_build_mismatch")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        contradicts.push(json!({
            "channel": "fe_integrity",
            "axes": ["br", "os"],
            "signal": "fe_build_impl_epoch_mismatch",
            "stance": "contradict_weak",
            "device_digest": false,
            "note": "FE build stamp disagrees with the manifest FE epoch",
        }));
    }

    // --- Field-utilization policy channels (all-fields-utilized) ---
    // Group-level consistency checkers for FE fields without a product-matrix
    // row: every registered field participates through a soft confirm/
    // contradict_weak signal (never hard device-digest).
    {
        let (u_confirms, u_contradicts) = run_channel_checks(&fo);
        confirms.extend(u_confirms);
        contradicts.extend(u_contradicts);
    }

    // --- XSRC / multi-source ---
    if truth.has_server_side && truth.has_main_core {
        confirms.push(json!({
            "channel": "xsrc_multi_source",
            "axes": ["os", "br", "device_id"],
            "signal": "server_plus_main",
            "stance": "confirm",
        }));
    } else if !truth.has_server_side {
        contradicts.push(json!({
            "channel": "xsrc_multi_source",
            "axes": ["os", "br"],
            "signal": "fe_only_or_no_server",
            "stance": "contradict_thin",
        }));
    }
    if !source_conflicts.is_empty() || truth.xsrc_status == "conflict" {
        contradicts.push(json!({
            "channel": "xsrc_conflict",
            "axes": ["os", "br"],
            "signal": "source_claim_obs_conflict",
            "stance": "contradict",
            "conflicts": source_conflicts,
        }));
    }

    // --- Soft env ---
    if soft_obs {
        confirms.push(json!({
            "channel": "soft_env",
            "axes": ["os", "device_id"],
            "signal": "soft_residual_class",
            "stance": "confirm_env_class",
            "device_posture": "soft_tier_collision_risk_possible",
        }));
    }

    // --- Antidetect / emulator (X-8/X-9): os/br/rpa only — never commercial digest ---
    let antidetect = fo
        .get("antidetect_vendor_hint")
        .map(|v| {
            v.as_bool().unwrap_or(false)
                || v.as_str().is_some_and(|s| !s.is_empty() && s != "false")
        })
        .unwrap_or(false)
        || fo
            .get("fingerprint_vendor_lie")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    if antidetect {
        contradicts.push(json!({
            "channel": "antidetect",
            "axes": ["os", "br"],
            "signal": "antidetect_or_vendor_lie",
            "stance": "contradict",
            "device_digest": false,
            "note": "never_commercial_digest_residual_led_id",
        }));
    }
    let emu = fo
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
    if emu {
        contradicts.push(json!({
            "channel": "emulator",
            "axes": ["os", "rpa"],
            "signal": "emulator_or_virt_stack",
            "stance": "contradict",
            "association_cap": "env",
            "device_digest": false,
        }));
    }
    if fo
        .get("prototype_chain_tamper")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        contradicts.push(json!({
            "channel": "antidetect",
            "axes": ["br"],
            "signal": "prototype_chain_tamper",
            "stance": "contradict",
            "device_digest": false,
        }));
    }
    // --- Screen RFP mask (surface_identity): FE screen_fp_protection_suspect ---
    // RFP window masking alone is an observation (legit privacy tooling). It
    // only becomes a contradict when paired with a high-end GPU claim that the
    // residual class does not support — RFP-style masks and soft/stable stacks
    // co-occur, so a "clean" residual + RFP mask + exotic label is inconsistent.
    let rfp_screen = fo
        .get("screen_fp_protection_suspect")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if rfp_screen {
        // Only fire when stack_auth found no independent spoof evidence —
        // otherwise that channel already carries the contradict.
        let exotic_vs_clean_residual = exotic
            && !soft_obs
            && stack.gpu_label_untrusted == false
            && stack.residual_soft_like == Some(false);
        if exotic_vs_clean_residual {
            contradicts.push(json!({
                "channel": "screen_fp",
                "axes": ["os", "br"],
                "signal": "rfp_mask_vs_clean_high_end_gpu",
                "stance": "contradict_weak",
                "device_digest": false,
                "note": "rfp-style masking on a claimed high-end silicon while residual reads clean",
            }));
        } else if stack.soft_stack || soft_obs {
            confirms.push(json!({
                "channel": "screen_fp",
                "axes": ["os", "br"],
                "signal": "rfp_mask_soft_stack_consistent",
                "stance": "confirm_weak",
            }));
        }
    }

    // Axis score reasons as secondary channels — only **hard** claim-obs / collusion /
    // capability conflicts. Low-weight context reasons (e.g. gpu_label_claim_context_low_weight)
    // must NOT become false contradicts that demote residual-rich sessions.
    for (axis, block) in [("os", os), ("br", br), ("rpa", rpa)] {
        if let Some(arr) = block.get("reasons").and_then(|v| v.as_array()) {
            for r in arr {
                let s = r.as_str().unwrap_or("");
                if score_reason_is_hard_contradict(s) {
                    contradicts.push(json!({
                        "channel": format!("score_{axis}"),
                        "axes": [axis],
                        "signal": s,
                        "stance": "contradict",
                    }));
                }
            }
        }
    }

    let n_confirm = confirms.len();
    let n_contradict = contradicts
        .iter()
        .filter(|c| {
            c.get("stance")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s == "contradict" || s == "contradict_automation")
        })
        .count();
    let n_thin = contradicts
        .iter()
        .filter(|c| {
            c.get("stance")
                .and_then(|v| v.as_str())
                == Some("contradict_thin")
        })
        .count();

    // Fusion summary: multi-channel required for high certainty.
    let fusion_stance = if n_contradict >= 2 {
        "multi_contradict"
    } else if n_contradict == 1 && n_confirm >= 2 {
        "mixed_verify"
    } else if n_thin > 0 && n_confirm < 2 {
        "thin_insufficient"
    } else if n_confirm >= 3 && n_contradict == 0 {
        "multi_confirm"
    } else if n_confirm >= 1 {
        "partial_confirm"
    } else {
        "unknown"
    };

    let collision_risk = device
        .get("collision_risk")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    json!({
        "algo": "mutual_verification_v1",
        "fusion_stance": fusion_stance,
        "confirm_count": n_confirm,
        "contradict_count": n_contradict,
        "thin_count": n_thin,
        "confirms": confirms,
        "contradicts": contradicts,
        "collision_risk": collision_risk,
        "residual_led_device": true,
        "rules": [
            "L0_gpu_label_never_sole_device_identity",
            "JA_protocol_br_only_never_device_digest",
            "control_plane_rpa_primary",
            "soft_residual_alone_not_rpa",
            "thin_materials_lower_confidence_not_fake_certainty",
            "contradict_channels_require_multi_algorithm_not_single_threshold",
        ],
    })
}

/// Task-critical materials per product axis (iss/31 G-31-2).
/// Small SSOT subset so confidence is not crushed by 500+ matrix rows.
pub fn task_critical_fields(axis: &str) -> &'static [&'static str] {
    match axis {
        "device_id" => &[
            "form_class",
            "hw_curve_webgl",
            "hw_curve_audio",
            "residual_mean",
            "unit_surface_id",
            "unit_multiround_stable",
            "webrtc_host_ip_hash",
            "os_instance_hash",
            "os_family",
            "hardware_concurrency",
            "architecture",
        ],
        "os" => &[
            "residual_soft_like",
            "soft_stack",
            "vm_score",
            "stack_class",
            "residual_mean",
            "hw_curve_webgl",
            "webgl_unmasked_renderer",
            "os_family",
            "platform",
            "os_instance_hash",
            "unit_surface_id",
        ],
        "br" => &[
            "webgl2_support",
            "webgl_max_texture",
            "webgl_extensions_hash",
            "webgl_unmasked_renderer",
            "residual_mean",
            "residual_soft_like",
            "chrome_runtime",
            "ja4",
            "ja3",
            "webdriver",
            "cdp_runtime_hint",
            "unit_multiround_stable",
            "sandbox_ok",
        ],
        "rpa" => &[
            "webdriver",
            "cdp_runtime_hint",
            "behavior_events",
            "behavior_count",
            "behavior_early_bound",
            "headless_likely",
            "automation.playwright",
            "automation.selenium",
            "outer_zero",
            "input_mouse_entropy",
            "pre_action_move_count",
        ],
        _ => &[],
    }
}

/// Confidence completeness report for one axis (0..1 + posture).
pub fn axis_confidence_report(
    axis: &str,
    fields: &Value,
    score: f64,
    field_hits: &[String],
    verification: Option<&Value>,
) -> Value {
    let cov = load_field_product_matrix()
        .map(|m| m.axis_coverage(axis, fields))
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let fo = fields.as_object().cloned().unwrap_or_default();
    // Primary: task-critical subset (G-31-2). Secondary: matrix imp for residual signal.
    let critical = task_critical_fields(axis);
    let mut crit_have: f64 = 0.0;
    let mut crit_need: f64 = 0.0;
    let mut miss = Vec::new();
    for f in critical {
        crit_need += 1.0;
        if field_present_util(&fo, f) {
            crit_have += 1.0;
        } else {
            miss.push((*f).to_string());
        }
    }
    let crit_cov: f64 = if crit_need > 0.0 {
        (crit_have / crit_need).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (imp_have, imp_need, mut miss_matrix) = load_field_product_matrix()
        .map(|m| {
            let fo = fields.as_object().cloned().unwrap_or_default();
            let mut need = 0.0;
            let mut have = 0.0;
            let mut miss = Vec::new();
            for row in &m.fields {
                let Some(role) = row.axes.get(axis) else {
                    continue;
                };
                if role == "exclude" || role == "diag" {
                    continue;
                }
                let tier = row.trust_tier.as_str();
                let ok = match (role.as_str(), tier) {
                    ("material", "T0" | "T1" | "T2") => true,
                    ("conf" | "veto", "T0" | "T1") => true,
                    _ => false,
                };
                if !ok {
                    continue;
                }
                // Prefer critical fields when listed; still count T0–T1 material for residual
                let in_crit = critical.iter().any(|c| *c == row.field.as_str());
                if !in_crit && !(role == "material" && matches!(tier, "T0" | "T1")) {
                    continue;
                }
                let w = role_importance(role, &row.trust_tier) * if in_crit { 1.4 } else { 0.35 };
                if w <= 0.0 {
                    continue;
                }
                need += w;
                if field_present_util(&fo, &row.field) {
                    have += w;
                } else if in_crit {
                    miss.push(row.field.clone());
                }
            }
            (have, need, miss)
        })
        .unwrap_or((0.0, 0.0, Vec::new()));
    for m in miss_matrix.drain(..) {
        if !miss.contains(&m) {
            miss.push(m);
        }
    }
    let imp_cov_raw = if imp_need > 0.0 {
        (imp_have / imp_need).clamp(0.0, 1.0)
    } else {
        cov
    };
    // Blend: 70% task-critical + 30% weighted matrix critical/T0-T1
    let imp_cov = (0.70 * crit_cov + 0.30 * imp_cov_raw).clamp(0.0, 1.0);
    let hit_boost = (field_hits.len() as f64 / 12.0).clamp(0.0, 0.15);
    let score = score.clamp(0.0, 1.0);
    let mut conf =
        (0.12 + 0.52 * imp_cov + 0.18 * cov + 0.15 * score + hit_boost).clamp(0.0, 1.0);

    // Mutual verification demotion / boost
    let mut fusion_note = "none";
    if let Some(v) = verification {
        let stance = v.get("fusion_stance").and_then(|x| x.as_str()).unwrap_or("");
        match stance {
            "multi_contradict" => {
                conf = (conf * 0.72).min(0.55);
                fusion_note = "multi_contradict_demote";
            }
            "mixed_verify" => {
                conf = (conf * 0.88).min(0.75);
                fusion_note = "mixed_verify_demote";
            }
            "thin_insufficient" => {
                conf = conf.min(0.42);
                fusion_note = "thin_cap";
            }
            "multi_confirm" => {
                conf = (conf + 0.06).min(0.98);
                fusion_note = "multi_confirm_boost";
            }
            _ => {}
        }
    }

    // Posture uses conf + importance coverage + hit breadth.
    // Matrix is large: do not require high imp_cov alone for "partial" when scorer hits are rich.
    let hit_n = field_hits.len();
    let posture = if conf < 0.36 || (imp_cov < 0.15 && hit_n < 4) {
        "thin"
    } else if conf < 0.50 || (imp_cov < 0.35 && hit_n < 8) {
        "partial"
    } else if conf >= 0.62 && (imp_cov >= 0.40 || hit_n >= 10) {
        "sufficient"
    } else {
        "adequate"
    };

    json!({
        "confidence": (conf * 10000.0).round() / 10000.0,
        "confidence_algo": "task_critical_completeness_v1",
        "posture": posture,
        "importance_coverage": (imp_cov * 10000.0).round() / 10000.0,
        "task_critical_coverage": (crit_cov * 10000.0).round() / 10000.0,
        "task_critical_fields": critical,
        "axis_coverage": (cov * 10000.0).round() / 10000.0,
        "score_component": score,
        "field_hit_count": field_hits.len(),
        "missing_critical_sample": miss.into_iter().take(8).collect::<Vec<_>>(),
        "fusion_adjust": fusion_note,
        "data_sufficient": posture == "sufficient" || posture == "adequate",
    })
}

/// Task-oriented open verification gaps for re-probe (visitor may continue probing).
pub fn open_verification_gaps(evidence: &Value) -> Value {
    let gaps = scan_gaps(evidence, None).unwrap_or_default();
    classify_open_gaps(&gaps)
}

pub fn classify_open_gaps(gaps: &[Gap]) -> Value {
    let task_keywords = [
        ("device", "task.device_id"),
        ("unit", "task.device_id"),
        ("claim_obs", "task.br"),
        ("capability", "task.br"),
        ("br_", "task.br"),
        ("os_", "task.os"),
        ("rpa", "task.rpa"),
        ("soft_stack", "task.os"),
        ("control", "task.rpa"),
        ("cdp", "task.rpa"),
        ("webgpu", "task.device_id"),
        ("gpu_", "task.device_id"),
        ("challenge", "task.device_id"),
        ("gateway", "task.xsrc"),
        ("probe_dag", "task.general"),
        ("b8_", "task.xsrc"),
        ("sandbox_residual", "task.device_id"),
        ("probe_pack_incomplete", "task.general"),
        ("pack_health", "task.xsrc"),
    ];
    let mut by_task: Map<String, Value> = Map::new();
    let mut open = Vec::new();
    let mut soft_gpu_demoted = Vec::new();
    for g in gaps {
        let code = g.code.as_str();
        let mut tasks = Vec::new();
        for (kw, task) in &task_keywords {
            if code.contains(kw) {
                if !tasks.contains(&task.to_string()) {
                    tasks.push((*task).to_string());
                }
            }
        }
        if tasks.is_empty() {
            tasks.push("task.general".into());
        }
        let entry = json!({
            "code": g.code,
            "severity": g.severity,
            "detail": g.detail,
            "tasks": tasks,
        });
        // Soft-class GPU digs demoted to low — still listed but marked.
        if (code.contains("gpu_staircase") || code.contains("gpu_ns") || code.contains("webgpu"))
            && g.severity == "low"
        {
            soft_gpu_demoted.push(entry.clone());
        }
        open.push(entry.clone());
        for t in tasks {
            let arr = by_task
                .entry(t)
                .or_insert_with(|| json!([]));
            if let Some(a) = arr.as_array_mut() {
                a.push(entry.clone());
            }
        }
    }
    // High-priority re-probe first (exclude low GPU digs from top).
    // Phase B: B10 / host-sep / residual gaps always surface first for re-mint readiness.
    let mut re_probe_priority: Vec<Value> = open
        .iter()
        .filter(|g| {
            let sev = g.get("severity").and_then(|v| v.as_str()).unwrap_or("");
            let code = g.get("code").and_then(|v| v.as_str()).unwrap_or("");
            sev == "high"
                || code.contains("unit")
                || code.contains("claim_obs")
                || code.contains("capability")
                || code.contains("rpa")
                || code.contains("os_instance")
                || code.contains("soft_stack_budget")
                || code.contains("b10_curves")
                || code.contains("host_sep")
                || code.contains("webgl_residual")
                || code.contains("need_device")
        })
        .cloned()
        .collect();
    // Stable sort: finalize-blocking gaps first.
    re_probe_priority.sort_by(|a, b| {
        let rank = |g: &Value| -> i32 {
            let code = g.get("code").and_then(|v| v.as_str()).unwrap_or("");
            if code.contains("b10_curves") || code.contains("finalize") {
                0
            } else if code.contains("host_sep") {
                1
            } else if code.contains("device") || code.contains("residual") {
                2
            } else {
                3
            }
        };
        rank(a).cmp(&rank(b))
    });
    re_probe_priority.truncate(16);

    json!({
        "algo": "open_verification_gaps_v1",
        "open_count": open.len(),
        "open_gaps": open,
        "by_task": by_task,
        "re_probe_priority": re_probe_priority,
        "soft_gpu_host_digs_demoted": soft_gpu_demoted,
        "self_analysis_loop": "when visitor stays: brain schedules re_probe_priority packs; short-visit uses confidence_posture+open_gaps without blocking",
    })
}

/// iss/38 P2-1 / D-10: honest thin-platform classification (WebView/TV/thin — never refuse probe).
pub fn classify_platform_honesty(fields: &Value) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let ua = fo
        .get("user_agent")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let form = fo
        .get("form_class")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let webview = ua.contains("; wv)")
        || (ua.contains("version/") && ua.contains("mobile") && !ua.contains("chrome/"));
    let tv = ua.contains("smart-tv")
        || ua.contains("smarttv")
        || ua.contains("tizen")
        || ua.contains("webos")
        || ua.contains("bravia")
        || form == "tv";
    let has_curves = fo.get("hw_curve_webgl").is_some() || fo.get("hw_curve_audio").is_some();
    let cores = fo
        .get("hardware_concurrency")
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
        .unwrap_or(0.0);
    let thin = !has_curves && cores > 0.0 && cores <= 2.0;

    let kind = if webview {
        "webview"
    } else if tv {
        "tv"
    } else if thin {
        "thin"
    } else {
        "standard"
    };
    let skip_sensor_expectation = matches!(kind, "webview" | "tv" | "thin");
    let prefer_tier = match kind {
        "webview" | "tv" | "thin" => "dg_or_dv",
        _ => "ladder",
    };
    json!({
        "algo": "platform_honesty_v1",
        "kind": kind,
        "skip_sensor_expectation": skip_sensor_expectation,
        "prefer_device_tier_path": prefer_tier,
        "refuse_probe": false,
        "note": "thin/webview/tv → honest posture; never catalog-refuse",
    })
}

/// Fold present T1/T2 matrix materials into analysis as low-weight aux / contradict.
/// Does **not** expand commercial digest_order — only confidence, posture, diagnostics.
pub fn build_material_participation(fields: &Value, device: &Value) -> Value {
    let Ok(m) = load_field_product_matrix() else {
        return json!({"error": "matrix_unavailable"});
    };
    let fo = fields.as_object().cloned().unwrap_or_default();
    let mut aux_hits = Vec::new();
    let mut contradict = Vec::new();
    let mut conf_boost = 0.0f64;
    let mut conf_penalty = 0.0f64;
    let mut used = 0u32;
    let mut present_material = 0u32;
    for row in &m.fields {
        let role = row.axes.get("device_id").map(|s| s.as_str()).unwrap_or("");
        if !matches!(role, "material" | "conf" | "veto") {
            continue;
        }
        if !matches!(row.trust_tier.as_str(), "T0" | "T1" | "T2") {
            continue;
        }
        if !field_present_util(&fo, &row.field) {
            continue;
        }
        present_material += 1;
        let never_digest = m
            .commercial_device_id
            .never_digest
            .iter()
            .any(|x| x == &row.field);
        // Participation class
        let class = if never_digest {
            "reference_aux"
        } else if role == "veto" {
            "veto_or_counter"
        } else if role == "material" {
            "analysis_material"
        } else {
            "conf_or_hedge"
        };
        used += 1;
        match class {
            "reference_aux" => {
                conf_boost += 0.008;
                aux_hits.push(format!("{}:aux", row.field));
            }
            "veto_or_counter" => {
                conf_penalty += 0.04;
                contradict.push(format!("{}:veto", row.field));
            }
            "analysis_material" => {
                conf_boost += 0.012;
                aux_hits.push(format!("{}:mat", row.field));
            }
            _ => {
                conf_boost += 0.006;
                aux_hits.push(format!("{}:conf", row.field));
            }
        }
    }
    // Soft stack / webdriver contradict device silicon confidence
    if device
        .pointer("/trust/soft_stack")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || fo.get("webdriver").and_then(|v| v.as_bool()).unwrap_or(false)
    {
        conf_penalty += 0.08;
        contradict.push("soft_or_webdriver_device_penalty".into());
    }
    // Dual-HW inventory for analysis (not commercial expansion).
    let has_audio_curve = fo
        .get("hw_curve_audio")
        .and_then(|v| v.as_array())
        .map(|a| a.len() >= 8)
        .unwrap_or(false)
        || fo
            .get("hw_audio_stable")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty());
    let has_webgl_curve = fo
        .get("hw_curve_webgl")
        .and_then(|v| v.as_array())
        .map(|a| a.len() >= 4)
        .unwrap_or(false)
        || fo
            .get("hw_webgl_stable")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty());
    let has_webrtc = fo
        .get("webrtc_host_ip_hash")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty())
        || fo
            .get("webrtc_host_ip_hash_v2")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty());
    let has_ja4 = fo
        .get("ja4")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());
    let dual_hw = has_audio_curve && has_webgl_curve;
    if !dual_hw {
        conf_penalty += 0.03;
        contradict.push("dual_hw_incomplete_for_dh".into());
    } else {
        conf_boost += 0.02;
        aux_hits.push("dual_hw_audio_webgl:mat".into());
    }
    // Soft / protocol signals: analysis-only corroboration (never commercial body alone).
    if has_webrtc {
        conf_boost += 0.01;
        aux_hits.push("webrtc_host:host_sep_conf".into());
    }
    if has_ja4 {
        conf_boost += 0.006;
        aux_hits.push("ja4:reference_aux".into());
    }
    // Multi-path residual measurement quality (conf only — not commercial digest).
    let residual_paths_n = fo
        .get("residual_paths")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let residual_ok_n = fo
        .get("residual_paths")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter(|p| {
                    p.get("ok").and_then(|v| v.as_bool()).unwrap_or(false)
                        && p.get("entropy_ok").and_then(|v| v.as_bool()).unwrap_or(true)
                })
                .count()
        })
        .unwrap_or(0);
    // A2 curve-morph consumption: same-surface (audio deep vs shallow) shape
    // bucket agreement replaces fixed ±0.02–0.04 pointwise deltas as the
    // coherence signal. Soft confirmation only — never a commercial gate.
    let morph = crate::curve_morph::curve_morph_of(&fo);
    let audio_morph_agree = morph
        .get("coherence")
        .and_then(|c| c.get("audio_deep_vs_hw"))
        .and_then(|v| v.as_str())
        == Some("agree");
    if audio_morph_agree {
        conf_boost += 0.01;
        aux_hits.push("audio_morph_agree:mat".into());
    }
    // PATH_CAP≈2–4 under GL governor (product multipath). Single ok path is full residual material.
    // Multipath (≥2/3 ok) remains a conf bonus only — never a recollect/completeness gate.
    if residual_ok_n >= 1 || residual_paths_n >= 1 {
        conf_boost += 0.01;
        aux_hits.push(format!("residual_path_ok:n={residual_paths_n}/ok={residual_ok_n}"));
    }
    if residual_paths_n >= 3 {
        conf_boost += 0.01;
        aux_hits.push(format!("residual_multipath:n={residual_paths_n}/ok={residual_ok_n}"));
    }
    if residual_ok_n >= 2 {
        conf_boost += 0.01;
        aux_hits.push("residual_path_agreement_eligible".into());
    }
    let net = (conf_boost - conf_penalty).clamp(-0.25, 0.20);
    json!({
        "algo": "material_participation_v3_multipath",
        "policy": "expand analysis consumption; never expand commercial digest_order; dual HW required for dh; multi-path residual conf",
        "present_t0t2_device_axis": present_material,
        "participating_count": used,
        "confidence_delta": (net * 10000.0).round() / 10000.0,
        "dual_hw_ok": dual_hw,
        "has_audio_hw": has_audio_curve,
        "has_webgl_hw": has_webgl_curve,
        "has_webrtc_host_sep": has_webrtc,
        "has_ja4_aux": has_ja4,
        "audio_morph_coherent": audio_morph_agree,
        "curve_morph": morph,
        "residual_paths_n": residual_paths_n,
        "residual_paths_ok_n": residual_ok_n,
        "aux_hits_sample": aux_hits.into_iter().take(40).collect::<Vec<_>>(),
        "contradict_sample": contradict.into_iter().take(20).collect::<Vec<_>>(),
        "note": "All present fields participate as aux/contradict/low-weight; mint body stays dual-HW + residual_led; residual_paths inform conf only",
    })
}

/// Attach quality layer onto product axes and build diagnostics block.
pub fn attach_analysis_quality(
    product: &mut Value,
    fields: &Value,
    stack: &StackAuth,
    device: &Value,
    truth: &TruthResult,
    source_conflicts: &[String],
    evidence: Option<&Value>,
) -> Value {
    let os = product.get("os").cloned().unwrap_or(json!({}));
    let br = product.get("br").cloned().unwrap_or(json!({}));
    let rpa = product.get("rpa").cloned().unwrap_or(json!({}));

    let verification = build_mutual_verification(
        fields,
        stack,
        &os,
        &br,
        &rpa,
        device,
        truth,
        source_conflicts,
    );
    let utilization = build_field_utilization_map(fields);
    let material_part = build_material_participation(fields, device);

    // Open gaps from evidence or synthetic evidence from fields
    let ev = evidence.cloned().unwrap_or_else(|| {
        json!({
            "sources": ["main"],
            "batches": [{"batch_id": "B10_hw_curves", "source": "main"}],
            "fields": fields,
        })
    });
    let open_gaps = open_verification_gaps(&ev);

    // Per-axis confidence reports with fusion
    for axis in ["os", "br", "rpa"] {
        if let Some(block) = product.get_mut(axis) {
            let score = block.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let hits: Vec<String> = block
                .get("field_hits")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let report = axis_confidence_report(axis, fields, score, &hits, Some(&verification));
            if let Some(obj) = block.as_object_mut() {
                if let Some(c) = report.get("confidence") {
                    obj.insert("confidence".into(), c.clone());
                }
                obj.insert("confidence_report".into(), report.clone());
                obj.insert(
                    "confidence_posture".into(),
                    report.get("posture").cloned().unwrap_or(json!("unknown")),
                );
                obj.insert(
                    "confidence_algo".into(),
                    json!("completeness_x_importance_v2"),
                );
            }
        }
    }

    // Device confidence posture from completeness of device materials
    let dev_hits: Vec<String> = device
        .get("hard_materials_present")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let dev_score = device
        .get("device_confidence")
        .or_else(|| device.get("confidence"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.5);
    let device_conf = axis_confidence_report(
        "device_id",
        fields,
        dev_score,
        &dev_hits,
        Some(&verification),
    );

    if let Some(obj) = product.as_object_mut() {
        obj.insert("mutual_verification".into(), verification.clone());
        obj.insert("field_utilization".into(), utilization.clone());
        obj.insert("material_participation".into(), material_part.clone());
        obj.insert("open_verification_gaps".into(), open_gaps.clone());
        obj.insert("device_confidence_report".into(), device_conf.clone());
        if let Some(c) = device_conf.get("confidence").and_then(|v| v.as_f64()) {
            // Blend with existing device_confidence + material participation delta
            let prev = obj
                .get("device_confidence")
                .and_then(|v| v.as_f64())
                .unwrap_or(c);
            let part_delta = material_part
                .get("confidence_delta")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let blended = (0.40 * prev + 0.50 * c + 0.10 * (0.5 + part_delta)).clamp(0.0, 1.0);
            obj.insert(
                "device_confidence".into(),
                json!((blended * 10000.0).round() / 10000.0),
            );
            obj.insert(
                "device_confidence_posture".into(),
                device_conf
                    .get("posture")
                    .cloned()
                    .unwrap_or(json!("unknown")),
            );
        }
        // Low-weight os/br boosts from material participation (not digest).
        let part_delta = material_part
            .get("confidence_delta")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        for axis in ["os", "br"] {
            if let Some(block) = obj.get_mut(axis).and_then(|v| v.as_object_mut()) {
                if let Some(score) = block.get("score").and_then(|v| v.as_f64()) {
                    // Tiny auxiliary shift only — never dominate primary scorers.
                    let adj = (score + part_delta * 0.15).clamp(0.0, 1.0);
                    block.insert("score".into(), json!((adj * 10000.0).round() / 10000.0));
                    block.insert("material_participation_delta".into(), json!(part_delta));
                }
            }
        }
        obj.insert(
            "analysis_quality".into(),
            json!({
                "fusion_stance": verification.get("fusion_stance"),
                "confirm_count": verification.get("confirm_count"),
                "contradict_count": verification.get("contradict_count"),
                "open_gap_count": open_gaps.get("open_count"),
                "re_probe_ready": open_gaps
                    .get("re_probe_priority")
                    .and_then(|v| v.as_array())
                    .map(|a| !a.is_empty())
                    .unwrap_or(false),
                "field_present": utilization.get("present_count"),
                "missing_critical": utilization.get("missing_critical_count"),
                "material_participating": material_part.get("participating_count"),
                "material_participation_delta": material_part.get("confidence_delta"),
                "utilization_levels": crate::utilization::utilization_level_presence(
                    &fields.as_object().cloned().unwrap_or_default(),
                ),
                "quality_algo": "analysis_quality_v2_material_participation",
            }),
        );
    }

    json!({
        "mutual_verification": verification,
        "field_utilization": utilization,
        "material_participation": material_part,
        "open_verification_gaps": open_gaps,
        "device_confidence_report": device_conf,
    })
}
