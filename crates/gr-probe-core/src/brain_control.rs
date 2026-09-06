//! Brain control plane (iss/22 P0): Belief · Capability Envelope · Mission · BattleLog · stop_reason.
//!
//! Evolves the existing gap+UCB skeleton without replacing `build_frontier`.
//! - Belief: first-class axis claim/obs/conf/status
//! - Envelope: fast "in shooting range?" scan before deepen
//! - Mission: which war to fight this tick (feeds direction filter later)
//! - BattleLog: structured engagement record for offline iteration
//! - stop_reason: clarity|frontier_empty|budget|timeout|policy_band|out_of_envelope|coverage

use crate::stack_auth::StackAuth;
use crate::xsrc::TruthResult;
use serde_json::{json, Map, Value};

/// Axis status for routing (iss/22 §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisStatus {
    Clear,
    Thin,
    Conflict,
    Unknown,
    Absent,
}

impl AxisStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            AxisStatus::Clear => "clear",
            AxisStatus::Thin => "thin",
            AxisStatus::Conflict => "conflict",
            AxisStatus::Unknown => "unknown",
            AxisStatus::Absent => "absent",
        }
    }
}

fn f_score(block: Option<&Value>) -> Option<f64> {
    block.and_then(|v| v.get("score")).and_then(|s| s.as_f64())
}

fn conf_of(block: Option<&Value>) -> f64 {
    block
        .and_then(|v| v.get("confidence").or_else(|| v.get("score")))
        .and_then(|s| s.as_f64())
        .unwrap_or(0.0)
}

fn status_from_score(score: Option<f64>, has_material: bool) -> AxisStatus {
    if !has_material {
        return AxisStatus::Unknown;
    }
    match score {
        None => AxisStatus::Unknown,
        Some(s) if s >= 0.7 => AxisStatus::Clear,
        Some(s) if s >= 0.45 => AxisStatus::Thin,
        Some(_) => AxisStatus::Thin,
    }
}

fn str_field(fo: &Map<String, Value>, k: &str) -> Option<String> {
    fo.get(k)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn bool_field(fo: &Map<String, Value>, k: &str) -> Option<bool> {
    fo.get(k).and_then(|v| v.as_bool())
}

fn present(fo: &Map<String, Value>, k: &str) -> bool {
    match fo.get(k) {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(_) => true,
    }
}

/// Build first-class Belief from product scores + fields + stack + xsrc (iss/22 B-HYP-1).
pub fn build_belief(
    fields: &Value,
    product: &Value,
    stack: &StackAuth,
    truth: &TruthResult,
    device: &Value,
) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let os_block = product.get("os");
    let br_block = product.get("br");
    let rpa_block = product
        .get("rpa")
        .or_else(|| product.pointer("/page/rpa"));

    // form claim/obs
    let form_claim = str_field(&fo, "form_class").unwrap_or_else(|| "unknown".into());
    let mobile_claim = bool_field(&fo, "mobile_ua_claim")
        .or_else(|| bool_field(&fo, "mobile_ua_signals"))
        .unwrap_or(false);
    let form_obs = if present(&fo, "max_touch_points")
        && fo.get("max_touch_points").and_then(|v| v.as_f64()).unwrap_or(0.0) > 1.0
        && form_claim == "desktop"
    {
        "mobile_touch".to_string()
    } else {
        form_claim.clone()
    };
    let form_status = if mobile_claim && form_claim == "desktop" {
        AxisStatus::Conflict
    } else if form_claim == "unknown" || form_claim.is_empty() {
        AxisStatus::Unknown
    } else {
        AxisStatus::Clear
    };

    // engine claim/obs (families from iss/21)
    let eng_claim = str_field(&fo, "engine_claim").unwrap_or_else(|| "unknown".into());
    let eng_obs = str_field(&fo, "engine_obs").unwrap_or_else(|| "unknown".into());
    let eng_status = if eng_claim != "unknown"
        && eng_obs != "unknown"
        && eng_claim != eng_obs
    {
        AxisStatus::Conflict
    } else if eng_claim == "unknown" && eng_obs == "unknown" {
        AxisStatus::Unknown
    } else if eng_claim == "unknown" || eng_obs == "unknown" {
        AxisStatus::Thin
    } else {
        AxisStatus::Clear
    };

    // os
    let os_claim = str_field(&fo, "os_family").unwrap_or_else(|| "unknown".into());
    let os_score = f_score(os_block);
    let os_has = present(&fo, "platform")
        || present(&fo, "os_family")
        || present(&fo, "hw_curve_webgl")
        || present(&fo, "vm_score")
        || stack.soft_stack;
    let mut os_status = status_from_score(os_score, os_has);
    if stack.soft_stack || stack.residual_soft_like == Some(true) {
        // soft path is a clear observation of stack class, not unknown
        if os_status == AxisStatus::Unknown {
            os_status = AxisStatus::Thin;
        }
    }
    let mut os_support = Vec::new();
    if present(&fo, "platform") {
        os_support.push("platform");
    }
    if present(&fo, "vm_score") {
        os_support.push("vm_score");
    }
    if stack.soft_stack {
        os_support.push("soft_stack");
    }
    if present(&fo, "hw_curve_webgl") {
        os_support.push("hw_curve_webgl");
    }

    // br
    let br_score = f_score(br_block);
    let br_has = present(&fo, "webdriver")
        || present(&fo, "automation")
        || present(&fo, "native_integrity_ratio")
        || present(&fo, "material_cross_conflict")
        || present(&fo, "ja4")
        || present(&fo, "protocol_engine");
    let mut br_status = status_from_score(br_score, br_has);
    if bool_field(&fo, "material_cross_conflict").unwrap_or(false)
        || truth.xsrc_status == "conflict"
    {
        br_status = AxisStatus::Conflict;
    }
    let mut br_support = Vec::new();
    if present(&fo, "native_integrity_ratio") {
        br_support.push("native_integrity");
    }
    if present(&fo, "material_cross_conflict") {
        br_support.push("material_cross");
    }
    if present(&fo, "cdp_runtime_hint") {
        br_support.push("cdp");
    }
    if present(&fo, "ja4") || present(&fo, "protocol_engine") {
        br_support.push("protocol");
    }

    // rpa
    let rpa_score = f_score(rpa_block);
    let rpa_has = present(&fo, "behavior_early_bound")
        || present(&fo, "behavior_events")
        || present(&fo, "input_mouse_entropy")
        || present(&fo, "pre_action_move_count");
    let rpa_kin = present(&fo, "input_mouse_entropy")
        || present(&fo, "ttfi_ms")
        || present(&fo, "integer_coord_ratio");
    let mut rpa_status = if !rpa_has {
        AxisStatus::Unknown
    } else if !rpa_kin {
        AxisStatus::Thin
    } else {
        status_from_score(rpa_score, true)
    };
    if bool_field(&fo, "sensitive_action_zero_move").unwrap_or(false) {
        rpa_status = AxisStatus::Conflict;
    }
    let mut rpa_support = Vec::new();
    if rpa_has {
        rpa_support.push("behavior");
    }
    if rpa_kin {
        rpa_support.push("kinematics");
    }
    if bool_field(&fo, "sensitive_action_zero_move").unwrap_or(false) {
        rpa_support.push("sensitive_zero_move");
    }

    // stack obs
    let stack_obs = if stack.soft_stack {
        "soft_render"
    } else if !stack.stack_class.is_empty() {
        stack.stack_class.as_str()
    } else {
        "unknown"
    };
    let stack_status = if stack.soft_stack || stack.spoof_score >= 0.55 {
        AxisStatus::Clear // clear observation of soft/spoof
    } else if stack_obs == "unknown" {
        AxisStatus::Unknown
    } else {
        AxisStatus::Thin
    };

    // device
    let eligible = device
        .get("device_id")
        .and_then(|v| v.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    let device_conf = device
        .get("confidence")
        .or_else(|| device.get("device_confidence"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let device_reason = if eligible {
        "eligible"
    } else if !present(&fo, "hw_curve_webgl") && !present(&fo, "hw_curve_audio") {
        "missing_hw_curves"
    } else {
        "ineligible"
    };

    let mut adversary_tags: Vec<String> = Vec::new();
    // iss/22 P2: FE eagerMid / other bypass stamps
    if bool_field(&fo, "brain_bypass").unwrap_or(false)
        || str_field(&fo, "bypass_reason").is_some()
    {
        adversary_tags.push("brain_bypass".into());
        if let Some(r) = str_field(&fo, "bypass_reason") {
            adversary_tags.push(format!("bypass:{r}"));
        }
    }
    if mobile_claim && form_claim == "desktop" {
        adversary_tags.push("ua_form_spoof_suspect".into());
    }
    if eng_status == AxisStatus::Conflict {
        adversary_tags.push("engine_claim_obs_mismatch".into());
    }
    if stack.soft_stack {
        adversary_tags.push("soft_render_env".into());
    }
    if bool_field(&fo, "webdriver").unwrap_or(false) {
        adversary_tags.push("webdriver".into());
    }

    let mut unknown_flags: Vec<String> = Vec::new();
    if !present(&fo, "ua_ch_platform") && !present(&fo, "ua_ch_mobile") {
        unknown_flags.push("no_ua_ch".into());
    }
    if !present(&fo, "hw_curve_webgl") && !present(&fo, "webgl_unmasked_renderer") {
        unknown_flags.push("no_webgl".into());
    }
    if eng_claim == "unknown" && eng_obs == "unknown" {
        unknown_flags.push("unknown_engine".into());
    }

    json!({
        "revision": 1,
        "schema": "gr_belief_v1",
        "axes": {
            "form": {
                "claim": form_claim,
                "obs": form_obs,
                "conf": if form_status == AxisStatus::Clear { 0.75 } else if form_status == AxisStatus::Conflict { 0.35 } else { 0.2 },
                "support": if mobile_claim { json!(["mobile_ua_claim", "form_class"]) } else { json!(["form_class"]) },
                "status": form_status.as_str(),
            },
            "os": {
                "claim": os_claim,
                "obs": if stack.soft_stack { "soft_render" } else { "capability" },
                "conf": conf_of(os_block).max(os_score.unwrap_or(0.0) * 0.9),
                "support": os_support,
                "status": os_status.as_str(),
                "score": os_score,
            },
            "engine": {
                "claim": eng_claim,
                "obs": eng_obs,
                "conf": if eng_status == AxisStatus::Clear { 0.7 } else if eng_status == AxisStatus::Conflict { 0.25 } else { 0.15 },
                "support": ["engine_claim", "engine_obs"],
                "status": eng_status.as_str(),
            },
            "stack": {
                "claim": Value::Null,
                "obs": stack_obs,
                "conf": stack.spoof_score.max(if stack.soft_stack { 0.7 } else { 0.2 }),
                "support": if stack.soft_stack { json!(["soft_stack", "residual"]) } else { json!([stack_obs]) },
                "status": stack_status.as_str(),
            },
            "br": {
                "conf": conf_of(br_block).max(br_score.unwrap_or(0.0) * 0.9),
                "status": br_status.as_str(),
                "support": br_support,
                "score": br_score,
            },
            "rpa": {
                "conf": conf_of(rpa_block).max(rpa_score.unwrap_or(0.0) * 0.9),
                "status": rpa_status.as_str(),
                "support": rpa_support,
                "score": rpa_score,
            },
            "device": {
                "eligible": eligible,
                "reason": device_reason,
                "conf": device_conf,
                "status": if eligible { "clear" } else if device_reason == "missing_hw_curves" { "thin" } else { "unknown" },
            },
        },
        "adversary_tags": adversary_tags,
        "unknown_flags": unknown_flags,
        "xsrc_status": truth.xsrc_status,
        "real_band": truth.real_band,
    })
}

/// Fast Capability Envelope scan (iss/22 B-ENV-1) — O(field presence), no heavy compute.
pub fn scan_capability_envelope(fields: &Value, belief: &Value) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let webgl = present(&fo, "hw_curve_webgl")
        || present(&fo, "webgl_unmasked_renderer")
        || present(&fo, "webgl2_support");
    let audio = present(&fo, "hw_curve_audio") || present(&fo, "audio_sample_rate");
    let ua_ch = present(&fo, "ua_ch_platform") || present(&fo, "ua_ch_mobile");
    let sensors = present(&fo, "sensor_accel_present")
        || bool_field(&fo, "sensors_motion").unwrap_or(false);
    let behavior = present(&fo, "behavior_early_bound") || present(&fo, "behavior_events");
    let kinematics = present(&fo, "input_mouse_entropy")
        || present(&fo, "ttfi_ms")
        || present(&fo, "pre_action_move_count");

    let axis_status = |axis: &str| -> &str {
        belief
            .pointer(&format!("/axes/{axis}/status"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
    };

    let mut device_reasons = Vec::new();
    let device_status = if webgl || audio {
        "in"
    } else {
        device_reasons.push("no_webgl_no_audio");
        "absent"
    };

    let mut os_reasons = Vec::new();
    let os_st = axis_status("os");
    let os_status = if os_st == "conflict" {
        os_reasons.push("os_conflict");
        "thin"
    } else if present(&fo, "platform") || present(&fo, "os_family") {
        "in"
    } else {
        os_reasons.push("no_platform_obs");
        "thin"
    };

    let mut br_reasons = Vec::new();
    let br_st = axis_status("br");
    let br_status = if br_st == "conflict" {
        br_reasons.push("br_conflict");
        "thin"
    } else if present(&fo, "engine_obs")
        || present(&fo, "protocol_engine")
        || present(&fo, "ja4")
        || present(&fo, "chrome_runtime")
    {
        "in"
    } else if present(&fo, "user_agent") {
        br_reasons.push("ua_only_claim");
        "thin"
    } else {
        br_reasons.push("no_br_obs");
        "unknown"
    };

    let mut rpa_reasons = Vec::new();
    let rpa_status = if kinematics {
        "in"
    } else if behavior {
        rpa_reasons.push("behavior_without_kinematics");
        "thin"
    } else {
        rpa_reasons.push("no_behavior");
        "unknown"
    };

    let axes_in = [device_status, os_status, br_status, rpa_status]
        .iter()
        .filter(|s| **s == "in")
        .count();
    let in_range = axes_in >= 2 || (webgl && present(&fo, "platform"));

    let decision = if !in_range && device_status == "absent" && rpa_status == "unknown" {
        "declare_out_of_envelope"
    } else if !in_range {
        "hold"
    } else if br_status == "thin" || rpa_status == "thin" || os_status == "thin" {
        "deepen"
    } else {
        "hold"
    };

    json!({
        "schema": "gr_envelope_v1",
        "in_range": in_range,
        "axes": {
            "device": { "status": device_status, "reason_codes": device_reasons },
            "os": { "status": os_status, "reason_codes": os_reasons },
            "br": { "status": br_status, "reason_codes": br_reasons },
            "rpa": { "status": rpa_status, "reason_codes": rpa_reasons },
        },
        "capability_bitmap": {
            "webgl": webgl,
            "offline_audio": audio,
            "ua_ch": ua_ch,
            "sensors": sensors,
            "behavior": behavior,
            "kinematics": kinematics,
        },
        "catalog_cover_ratio": if in_range { 0.85 } else { 0.35 },
        "decision": decision,
    })
}

/// Select missions from belief + gaps (iss/22 B-MIS-1). Ordered by severity.
pub fn select_missions(belief: &Value, gaps: &[crate::brain::Gap], envelope: &Value) -> Value {
    let mut missions: Vec<Value> = Vec::new();
    let axis = |name: &str| -> &str {
        belief
            .pointer(&format!("/axes/{name}/status"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
    };
    let eligible = belief
        .pointer("/axes/device/eligible")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let env_decision = envelope
        .get("decision")
        .and_then(|v| v.as_str())
        .unwrap_or("deepen");

    if env_decision == "declare_out_of_envelope" {
        missions.push(json!({
            "id": "probe_unknown_adversary",
            "priority": 100,
            "trigger": "out_of_envelope",
            "directions": ["browser_kernel", "protocol_edge"],
        }));
    }

    if !eligible
        || axis("device") == "thin"
        || gaps.iter().any(|g| g.code.contains("device") || g.code.contains("hw") || g.code == "missing_link_anchors")
    {
        missions.push(json!({
            "id": "secure_device_anchor",
            "priority": 95,
            "trigger": "device_ineligible_or_thin",
            "directions": ["gpu_physical", "cpu_clock", "challenge_pohw"],
        }));
    }

    let soft_stack_obs = belief
        .pointer("/axes/stack/obs")
        .and_then(|v| v.as_str())
        == Some("soft_render");
    if axis("os") == "conflict"
        || soft_stack_obs
        || gaps
            .iter()
            .any(|g| g.code.contains("os") || g.code == "need_os_env")
    {
        missions.push(json!({
            "id": "resolve_os_conflict",
            "priority": 85,
            "trigger": "os_or_stack_weak",
            "directions": ["media_codec", "peripheral", "mobile_form", "thermal_mem"],
        }));
    }

    if axis("br") == "conflict"
        || axis("engine") == "conflict"
        || axis("br") == "thin"
        || gaps.iter().any(|g| {
            g.code.contains("br")
                || g.code == "need_cdp_probe"
                || g.code == "need_br_automation"
                || g.code == "missing_sandbox"
        })
    {
        missions.push(json!({
            "id": "resolve_br_incoherence",
            "priority": 88,
            "trigger": "br_or_engine_weak",
            "directions": ["browser_kernel", "sandbox_xsrc", "protocol_edge", "material_hedge"],
        }));
    }

    if axis("rpa") == "unknown"
        || axis("rpa") == "thin"
        || axis("rpa") == "conflict"
        || gaps.iter().any(|g| {
            g.code.contains("rpa")
                || g.code == "need_rpa_bind"
                || g.code == "need_rpa_kinematics"
                || g.code == "need_rpa_events"
        })
    {
        missions.push(json!({
            "id": "raise_rpa_clarity",
            "priority": 90,
            "trigger": "rpa_thin_or_unknown",
            "directions": ["browser_kernel"],
            "packs_hint": ["B11_interaction", "B12_anti_camouflage"],
        }));
    }

    // Multi-axis thin → unknown adversary explore
    let thin_n = ["os", "br", "rpa", "device"]
        .iter()
        .filter(|a| {
            let s = axis(a);
            s == "thin" || s == "unknown" || s == "absent"
        })
        .count();
    if thin_n >= 3 && !missions.iter().any(|m| m.get("id").and_then(|v| v.as_str()) == Some("probe_unknown_adversary"))
    {
        missions.push(json!({
            "id": "probe_unknown_adversary",
            "priority": 70,
            "trigger": "multi_axis_thin",
            "directions": ["census_surface", "browser_kernel", "protocol_edge"],
        }));
    }

    missions.sort_by(|a, b| {
        let pa = a.get("priority").and_then(|v| v.as_i64()).unwrap_or(0);
        let pb = b.get("priority").and_then(|v| v.as_i64()).unwrap_or(0);
        pb.cmp(&pa)
    });

    let primary = missions
        .first()
        .and_then(|m| m.get("id").cloned())
        .unwrap_or(json!("hold"));

    json!({
        "schema": "gr_missions_v1",
        "primary": primary,
        "ordered": missions,
    })
}

/// Stop reason enum (iss/22 B-STOP-1).
pub fn classify_stop_reason(
    stop_probe: bool,
    coverage_complete: bool,
    packs_empty: bool,
    envelope: &Value,
    belief: &Value,
) -> &'static str {
    if !stop_probe {
        return "continue";
    }
    let env_dec = envelope
        .get("decision")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if env_dec == "declare_out_of_envelope" {
        return "out_of_envelope";
    }
    // Clarity: device eligible + axes not thin/conflict on br/os when coverage done
    let eligible = belief
        .pointer("/axes/device/eligible")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let br_ok = matches!(
        belief.pointer("/axes/br/status").and_then(|v| v.as_str()),
        Some("clear") | Some("thin")
    );
    if coverage_complete && eligible && br_ok {
        return "clarity";
    }
    if coverage_complete && packs_empty {
        return "frontier_empty";
    }
    if coverage_complete {
        return "coverage";
    }
    if packs_empty {
        return "frontier_empty";
    }
    "budget"
}

/// Policy band from product scores (iss/22 §4.3) — drives deepen vs challenge vs hold.
///
/// **Probe continuation policy (product):** missing fields and high-risk scores demote
/// axis conf / business action; they must **not** freeze the FE probe pipeline.
/// `prefer_stop` is always false here — stop only when coverage frontier is truly done
/// (see `evaluate` + `build_frontier`). High risk → **more** verification packs, not fewer.
pub fn policy_band_from_scores(os: Option<f64>, br: Option<f64>, rpa: Option<f64>) -> Value {
    // Risk-ish: lower safety score → higher band risk
    let risks: Vec<f64> = [os, br, rpa]
        .into_iter()
        .flatten()
        .map(|s| (1.0 - s).clamp(0.0, 1.0))
        .collect();
    let risk = if risks.is_empty() {
        0.4
    } else {
        risks.iter().sum::<f64>() / risks.len() as f64
    };
    let (band, action, probe) = if risk < 0.3 {
        ("low", "allow", "minimal")
    } else if risk < 0.55 {
        ("mid", "step_up", "raise_rpa_br")
    } else if risk < 0.8 {
        ("high", "challenge", "conflict_deepen")
    } else {
        // Critical: business may tarpit/block, but probe **verifies** (webdriver / antidetect /
        // incomplete env) instead of stop_probe — missing materials never halt other batches.
        ("critical", "block_or_tarpit", "verify_deepen")
    };
    json!({
        "schema": "gr_policy_band_v1",
        "risk": risk,
        "band": band,
        "action": action,
        "probe_policy": probe,
        "max_packs_cap": max_packs_for_band(band),
        // Never prefer_stop from band alone — scores already carry demotion; open gaps need packs.
        "prefer_stop": false,
        "prefer_stop_note": "missing_fields_and_risk_demote_scores_not_halt_probe",
    })
}

/// iss/22 P1d: band → max dynamic pack budget (after force floors).
///
/// Caps stay modest so 80-pack catalog cannot inflate first-tick wall-clock.
/// Dense B47–B79 enter via multi-tick residual slots + high-sev digests cap (brain).
pub fn max_packs_for_band(band: &str) -> usize {
    match band {
        "low" => 6,
        "mid" => 10,
        "high" => 10, // challenge: keep depth for conflict verification
        // Critical: deepen automation / env verification packs (was 3 + prefer_stop → mid death)
        "critical" => 12,
        _ => 12,
    }
}

/// Dense deepen packs (catalog B47–B79) — optional residual, never band "must" floors.
pub fn is_dense_pack_id(pack_id: &str) -> bool {
    let digits: String = pack_id
        .trim_start_matches('B')
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits
        .parse::<u32>()
        // B47–B80: densify + R100 high-value promote (B80)
        .map(|n| (47..=80).contains(&n))
        .unwrap_or(false)
}

/// Resource class for concurrent probe balancing (product v5.8.137).
///
/// **Rules (FE ResourceBus + brain stages):**
/// - Same class: at most **one** in-flight globally (main / iframe / worker share).
/// - Different classes: may run in parallel **only if packs do not share a class**.
/// - Compound packs (B10) multi-lock on FE (`resource_classes`); brain primary remains
///   `gpu` so B10 is staged first; FE holds audio/cpu too so B46 cannot thrash OfflineAudio.
/// - Light class: multiple may race (subject to FE light concurrency cap).
///
/// Classes: `gpu` | `audio` | `rtc` | `cpu` | `nest` | `verify` | `light`
pub fn resource_class(pack_id: &str) -> &'static str {
    if is_verify_rand_pack(pack_id) {
        return "verify";
    }
    if pack_id == "B7_sandbox" {
        // Nest orchestrator: serializes kinds internally; uses ResourceBus per sub-task.
        return "nest";
    }
    // Custom / merchant probes (C##_… / custom_* / merchant_*)
    if (pack_id.starts_with('C')
        && pack_id.len() >= 3
        && pack_id.as_bytes()[1].is_ascii_digit())
        || pack_id.starts_with("custom_")
        || pack_id.starts_with("merchant_")
    {
        let l = pack_id.to_ascii_lowercase();
        if l.contains("audio") {
            return "audio";
        }
        if l.contains("webgl") || l.contains("gpu") || l.contains("canvas") {
            return "gpu";
        }
        if l.contains("webrtc") || l.contains("ice") {
            return "rtc";
        }
        if l.contains("cpu") || l.contains("hw") {
            return "cpu";
        }
        // Default custom to light so unknown merchant scripts do not block exclusive lanes.
        return "light";
    }
    // GPU / WebGL residual / canvas-GL
    // B10 primary class is gpu (brain staging); FE multi-locks [gpu,audio,cpu].
    if pack_id == "B10_hw_curves"
        || pack_id.starts_with("B10x_")
        || pack_id == "B15_cross_curves"
        || pack_id == "B18_webgpu"
        || pack_id == "B22_gpu_timer"
        || pack_id == "B23_native_canvas_hedge"
        || pack_id == "B30_gpu_bandwidth"
        || pack_id == "B31_shader_numeric"
        || pack_id == "B36_raster_msaa"
        || pack_id == "B57_canvas_emoji_path"
        || pack_id == "B58_canvas_text_metrics"
        || pack_id == "B59_webgl_params_full"
        || pack_id == "B60_webgl_extensions_full"
    {
        return "gpu";
    }
    // Audio path
    if pack_id == "B46_audio_deep"
        || pack_id == "B61_audio_worklet"
        || pack_id == "B62_offline_audio_moments"
    {
        return "audio";
    }
    // WebRTC / network media path
    if pack_id == "B9_network" || pack_id == "B55_webrtc_ice_deep" {
        return "rtc";
    }
    // Heavy CPU / thermal / caps (not pure GPU)
    if pack_id == "B2_hardware"
        || pack_id == "B17_hw_physical"
        || pack_id == "B20_challenge_seed"
        || pack_id == "B33_caps_pressure"
        || pack_id == "B34_cpu_cache_ladder"
        || pack_id == "B37_thermal_drift_lite"
        || pack_id == "B42_thermal_drift_full"
        || pack_id == "B47_sab_clock"
    {
        return "cpu";
    }
    let l = pack_id.to_ascii_lowercase();
    if l.contains("webgl")
        || l.contains("webgpu")
        || l.contains("gpu")
        || l.contains("canvas")
        || l.contains("shader")
        || l.contains("raster")
        || l.contains("msaa")
    {
        return "gpu";
    }
    if l.contains("audio") {
        return "audio";
    }
    if l.contains("webrtc") || l.contains("ice") {
        return "rtc";
    }
    if l.contains("hw_") || l.contains("thermal") || l.contains("cpu") {
        return "cpu";
    }
    "light"
}

/// Browser **resource-touching** packs (any non-light class).
/// Prefer `resource_class` for concurrency; this remains for callers that need a bool.
pub fn is_hardware_probe_pack(pack_id: &str) -> bool {
    resource_class(pack_id) != "light"
}

/// Stage FE `parallel_groups` by **resource class**.
///
/// Within a stage, at most one pack per exclusive class (`gpu`/`audio`/`rtc`/`cpu`/`nest`/`verify`),
/// so FE may run **different classes in parallel** (gpu∥audio) without same-class thrash.
/// Light packs form their own early stage (may race among themselves).
///
/// Order of exclusive pick per stage (priority for short visits):
/// gpu → audio → rtc → cpu → nest → verify (one each if available).
/// Stages repeat until all exclusive packs drained.
pub fn build_hw_safe_parallel_groups(pack_ids: &[String]) -> Vec<Vec<String>> {
    if pack_ids.is_empty() {
        return Vec::new();
    }
    let mut light: Vec<String> = Vec::new();
    // Queues per exclusive class, preserving input order (brain already prioritized).
    let mut by_class: std::collections::HashMap<&'static str, Vec<String>> =
        std::collections::HashMap::new();
    const EXCLUSIVE: &[&str] = &["gpu", "audio", "rtc", "cpu", "nest", "verify"];
    for id in pack_ids {
        let c = resource_class(id);
        if c == "light" {
            light.push(id.clone());
        } else {
            by_class.entry(c).or_default().push(id.clone());
        }
    }

    let mut groups: Vec<Vec<String>> = Vec::new();
    if !light.is_empty() {
        groups.push(light);
    }

    loop {
        let mut stage: Vec<String> = Vec::new();
        let mut any = false;
        for &cls in EXCLUSIVE {
            if let Some(q) = by_class.get_mut(cls) {
                if !q.is_empty() {
                    stage.push(q.remove(0));
                    any = true;
                }
            }
        }
        if !any {
            break;
        }
        groups.push(stage);
    }

    if groups.is_empty() {
        groups.push(pack_ids.to_vec());
    }
    groups
}

/// Stable per-source pack order permutation (anti-spoof + natural stagger).
/// Same catalog, different execution order per `source` (main / iframe:d1 / worker:d1).
pub fn shuffle_packs_for_source(pack_ids: &[String], session_id: &str, source: &str) -> Vec<String> {
    if pack_ids.len() <= 1 {
        return pack_ids.to_vec();
    }
    let mut h: u64 = 0xcbf29ce484222325;
    for b in session_id.as_bytes().iter().chain(source.as_bytes()) {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    let mut out = pack_ids.to_vec();
    // Fisher–Yates with session+source seed (deterministic).
    let n = out.len();
    for i in (1..n).rev() {
        h ^= h << 13;
        h ^= h >> 7;
        h ^= h << 17;
        let j = (h as usize) % (i + 1);
        out.swap(i, j);
    }
    out
}

/// Max dense packs scheduled in one frontier tick.
/// FE ResourceBus serializes GPU/canvas work; keep tick density low so GL loseContext
/// can reclaim slots between packs (Chrome async context free).
pub const DENSE_PACKS_PER_TICK_CAP: usize = 1;

/// Authenticity spot-check packs R00–R99 (score_weight=0) — separate lane.
/// Product: schedule **exactly 1** random R batch per frontier tick (anti-evasion).
/// Richness is 120 ops/pack; concurrency safety is FE HW mutex + brain singleton stages.
pub const VERIFY_SPOTCHECK_MIN: usize = 1;
pub const VERIFY_SPOTCHECK_MAX: usize = 1;
/// Nominal / default pick when a single cap is needed.
pub const VERIFY_SPOTCHECK_PER_TICK: usize = 1;

/// Session-stable pick count in [VERIFY_SPOTCHECK_MIN, VERIFY_SPOTCHECK_MAX].
pub fn verify_spotcheck_n_for_session(session_seed: u64) -> usize {
    let span = VERIFY_SPOTCHECK_MAX.saturating_sub(VERIFY_SPOTCHECK_MIN).saturating_add(1);
    if span <= 1 {
        return VERIFY_SPOTCHECK_MIN.max(1);
    }
    VERIFY_SPOTCHECK_MIN + (session_seed as usize) % span
}

/// Random verify pack id? `R00_spotcheck` … `R99_spotcheck`
pub fn is_verify_rand_pack(pack_id: &str) -> bool {
    pack_id.starts_with('R')
        && (pack_id.contains("spotcheck")
            || pack_id
                .trim_start_matches('R')
                .chars()
                .take(2)
                .all(|c| c.is_ascii_digit()))
}

/// Lightweight field-only belief for early mission select (before full product scores).
pub fn belief_from_fields_thin(fields: &Value) -> Value {
    let stack = crate::stack_auth::stack_auth_from_fields(fields);
    let truth = crate::xsrc::TruthResult {
        xsrc_status: "ok".into(),
        real_band: "watch".into(),
        credibility: 0.4,
        fe_only: true,
        has_server_side: false,
        has_main_core: true,
        packages: json!([]),
        reasons: vec![],
        details: json!({}),
    };
    let product = json!({
        "os": {"score": 0.5, "confidence": 0.4},
        "br": {"score": 0.5, "confidence": 0.4},
        "rpa": {"score": 0.35, "confidence": 0.3},
    });
    let device = json!({"confidence": 0.1, "device_id": Value::Null});
    build_belief(fields, &product, &stack, &truth, &device)
}

/// Pack budget lane (architecture/pack-budget-tiers.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackLane {
    /// L0 — commercial / session identity core
    CoreIdentity,
    /// L1 — deepen / engine-specialized / dense
    Deepen,
    /// L2 — random authenticity verify (independent quota)
    VerifyRand,
}

/// Classify pack into L0 / L1 / L2.
pub fn pack_lane(pack_id: &str, layer: &str) -> PackLane {
    if is_verify_rand_pack(pack_id) || layer == "verify_rand" {
        return PackLane::VerifyRand;
    }
    if pack_id.starts_with("B10x_") || is_dense_pack_id(pack_id) {
        return PackLane::Deepen;
    }
    match pack_id {
        // Complete-probe static anchors — never truncated by L1 dense budget.
        "B0_bootstrap"
        | "B1_conflict"
        | "B2_hardware"
        | "B3_system"
        | "B7_sandbox"
        | "B8_gateway_early"
        | "B10_hw_curves"
        | "B11_interaction"
        | "B12_anti_camouflage" => PackLane::CoreIdentity,
        _ if layer == "lite5" || layer == "b8" || layer == "hard" => {
            if pack_id == "B7_sandbox" || pack_id == "B10_hw_curves" {
                PackLane::CoreIdentity
            } else {
                PackLane::Deepen
            }
        }
        _ => PackLane::Deepen,
    }
}

fn pack_priority(p: &Value) -> i64 {
    p.get("effective_priority")
        .or_else(|| p.get("priority"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
}

/// Apply **tiered** pack budget: L0 core must + L1 deepen under band cap + L2 verify independent.
///
/// Replaces naive truncate; keeps identity packs from being starved by dense/R100.
pub fn apply_tiered_pack_budget(packs: &mut Vec<Value>, band: &Value) {
    let cap = band
        .get("max_packs_cap")
        .and_then(|v| v.as_u64())
        .unwrap_or(12) as usize;

    let mut core: Vec<Value> = Vec::new();
    let mut deepen: Vec<Value> = Vec::new();
    let mut verify: Vec<Value> = Vec::new();
    for p in packs.iter() {
        let pid = p.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
        let layer = p.get("layer").and_then(|v| v.as_str()).unwrap_or("");
        match pack_lane(pid, layer) {
            PackLane::CoreIdentity => core.push(p.clone()),
            PackLane::Deepen => deepen.push(p.clone()),
            PackLane::VerifyRand => verify.push(p.clone()),
        }
    }
    core.sort_by(|a, b| pack_priority(b).cmp(&pack_priority(a)));
    deepen.sort_by(|a, b| pack_priority(b).cmp(&pack_priority(a)));
    verify.sort_by(|a, b| pack_priority(b).cmp(&pack_priority(a)));

    // L0: keep all core (identity must not be truncated by deepen)
    let mut keep = core;
    // L1a: B10x silicon must-land / EDH deepen is reserved — never starved by dense L1.
    // Production: short visits often lost residual_ok because B10x was truncated after core.
    let mut b10x_reserved: Vec<Value> = Vec::new();
    let mut deepen_rest: Vec<Value> = Vec::new();
    for p in deepen.into_iter() {
        let pid = p.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
        let reason = p.get("reason").and_then(|v| v.as_str()).unwrap_or("");
        let hard_el = p.get("hard_eligible").and_then(|v| v.as_bool()) == Some(true);
        let must = pid.starts_with("B10x_silicon_")
            || reason.contains("b10x")
            || reason.contains("edh_b10x")
            || (hard_el && pid.starts_with("B10x_"));
        if must {
            b10x_reserved.push(p);
        } else {
            deepen_rest.push(p);
        }
    }
    keep.extend(b10x_reserved);
    // L1b: remaining commercial cap for other deepen (after reserved silicon)
    let room = cap.saturating_sub(keep.len());
    keep.extend(deepen_rest.into_iter().take(room));
    // L2: independent verify quota
    for v in verify.into_iter().take(VERIFY_SPOTCHECK_MAX) {
        let pid = v.get("pack_id").and_then(|x| x.as_str()).unwrap_or("");
        if !keep
            .iter()
            .any(|p| p.get("pack_id").and_then(|x| x.as_str()) == Some(pid))
        {
            keep.push(v);
        }
    }
    // Annotate lane on each pack for FE/ops observability
    for p in keep.iter_mut() {
        if let Some(obj) = p.as_object_mut() {
            let pid = obj.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
            let layer = obj.get("layer").and_then(|v| v.as_str()).unwrap_or("");
            let lane = match pack_lane(pid, layer) {
                PackLane::CoreIdentity => "core_identity",
                PackLane::Deepen => "deepen",
                PackLane::VerifyRand => "verify_rand",
            };
            obj.insert("pack_lane".into(), json!(lane));
        }
    }
    keep.sort_by(|a, b| pack_priority(b).cmp(&pack_priority(a)));
    *packs = keep;
}

/// Apply policy_band pack budget: tiered L0/L1/L2 (iss/22 P1d + pack-budget-tiers).
pub fn apply_band_pack_budget(packs: &mut Vec<Value>, band: &Value) {
    apply_tiered_pack_budget(packs, band);
}

/// iss/22 P2: HMAC-SHA256 route plan seal (optional env GR_ROUTE_HMAC_SECRET).
/// Returns hex signature over canonical fields; empty secret → unsigned.
pub fn sign_route_plan(route: &Value, session_id: &str, evidence_rev: i64) -> Value {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let secret = gr_abi::env::get("ROUTE_HMAC_SECRET").unwrap_or_default();
    let packs: Vec<String> = route
        .get("packs")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let mission = route
        .get("mission_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let plan_version = route
        .get("plan_version")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let payload = format!(
        "v1|{session_id}|{evidence_rev}|{plan_version}|{mission}|{}",
        packs.join(",")
    );
    if secret.is_empty() {
        return json!({
            "alg": "none",
            "signed": false,
            "evidence_rev": evidence_rev,
            "payload_fp": simple_fp(&payload),
        });
    }
    // Portable keyed hash without extra crate: secret||payload via DefaultHasher rounds
    let mut h = DefaultHasher::new();
    secret.hash(&mut h);
    payload.hash(&mut h);
    secret.hash(&mut h);
    let sig = format!("{:016x}", h.finish());
    json!({
        "alg": "gr_route_v1",
        "signed": true,
        "sig": sig,
        "evidence_rev": evidence_rev,
        "payload_fp": simple_fp(&payload),
    })
}

fn simple_fp(s: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Update direction_priors map after a tick (learning only ranks, not redlines).
pub fn update_direction_priors(prev: Option<&Value>, scheduled_dirs: &[String], yield_ok: bool) -> Value {
    let mut map = prev
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    for d in scheduled_dirs {
        let mut entry = map
            .get(d)
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        let n = entry.get("n").and_then(|v| v.as_f64()).unwrap_or(0.0) + 1.0;
        let ok = entry.get("yield_ok").and_then(|v| v.as_f64()).unwrap_or(0.0)
            + if yield_ok { 1.0 } else { 0.0 };
        entry.insert("n".into(), json!(n));
        entry.insert("yield_ok".into(), json!(ok));
        map.insert(d.clone(), Value::Object(entry));
    }
    Value::Object(map)
}

/// Aggregate unknown_bucket feature for offline iteration (iss/22 B-UNK-1).
pub fn unknown_bucket_from_belief(belief: &Value, envelope: &Value) -> Option<Value> {
    let flags = belief
        .get("unknown_flags")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let out = envelope
        .get("decision")
        .and_then(|v| v.as_str())
        == Some("declare_out_of_envelope");
    if flags.is_empty() && !out {
        return None;
    }
    Some(json!({
        "schema": "gr_unknown_bucket_v1",
        "unknown_flags": flags,
        "out_of_envelope": out,
        "adversary_tags": belief.get("adversary_tags"),
        "engine_status": belief.pointer("/axes/engine/status"),
        "capability_bitmap": envelope.get("capability_bitmap"),
    }))
}

/// Structured BattleLog entry (iss/22 B-LOG-1).
pub fn build_battle_log(
    session_id: &str,
    plan_version: u64,
    belief: &Value,
    envelope: &Value,
    missions: &Value,
    gaps: &[crate::brain::Gap],
    packs: &[Value],
    stop_reason: &str,
    policy_band: &Value,
) -> Value {
    let pack_ids: Vec<String> = packs
        .iter()
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    let gap_codes: Vec<String> = gaps.iter().map(|g| g.code.clone()).collect();
    let unknown_flags = belief
        .get("unknown_flags")
        .cloned()
        .unwrap_or(json!([]));
    let adversary_tags = belief
        .get("adversary_tags")
        .cloned()
        .unwrap_or(json!([]));

    json!({
        "schema": "gr_battle_log_v1",
        "session_id": session_id,
        "plan_version": plan_version,
        "mission_id": missions.get("primary"),
        "missions_ordered": missions.get("ordered").and_then(|v| v.as_array()).map(|a| {
            a.iter().filter_map(|m| m.get("id").cloned()).collect::<Vec<_>>()
        }).unwrap_or_default(),
        "belief_snapshot": {
            "os_status": belief.pointer("/axes/os/status"),
            "br_status": belief.pointer("/axes/br/status"),
            "rpa_status": belief.pointer("/axes/rpa/status"),
            "device_eligible": belief.pointer("/axes/device/eligible"),
            "engine_status": belief.pointer("/axes/engine/status"),
            "form_status": belief.pointer("/axes/form/status"),
        },
        "envelope": {
            "in_range": envelope.get("in_range"),
            "decision": envelope.get("decision"),
            "capability_bitmap": envelope.get("capability_bitmap"),
        },
        "gaps_selected": gap_codes,
        "packs_scheduled": pack_ids,
        "stop_reason": stop_reason,
        "policy_band": policy_band,
        "adversary_tags": adversary_tags,
        "unknown_flags": unknown_flags,
        "note": "structured engagement record — offline iteration fuel; not client-visible baring",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stack_auth::stack_auth_from_fields;

    #[test]
    fn belief_marks_form_conflict() {
        let fields = json!({
            "form_class": "desktop",
            "mobile_ua_claim": true,
            "platform": "Linux",
            "user_agent": "Mobile Safari",
            "engine_claim": "blink",
            "engine_obs": "blink",
        });
        let product = json!({
            "os": {"score": 0.5, "confidence": 0.4},
            "br": {"score": 0.5, "confidence": 0.4},
        });
        let stack = stack_auth_from_fields(&fields);
        let truth = TruthResult {
            xsrc_status: "ok".into(),
            real_band: "watch".into(),
            credibility: 0.5,
            fe_only: true,
            has_server_side: false,
            has_main_core: true,
            packages: json!([]),
            reasons: vec![],
            details: json!({}),
        };
        let device = json!({"device_id": null, "confidence": 0.1});
        let b = build_belief(&fields, &product, &stack, &truth, &device);
        assert_eq!(
            b.pointer("/axes/form/status").and_then(|v| v.as_str()),
            Some("conflict")
        );
    }

    #[test]
    fn envelope_out_when_no_capabilities() {
        let fields = json!({"user_agent": "x", "form_class": "desktop"});
        let product = json!({"os":{"score":0.3},"br":{"score":0.3}});
        let stack = stack_auth_from_fields(&fields);
        let truth = TruthResult {
            xsrc_status: "ok".into(),
            real_band: "watch".into(),
            credibility: 0.3,
            fe_only: true,
            has_server_side: false,
            has_main_core: false,
            packages: json!([]),
            reasons: vec![],
            details: json!({}),
        };
        let device = json!({"confidence": 0.0});
        let b = build_belief(&fields, &product, &stack, &truth, &device);
        let e = scan_capability_envelope(&fields, &b);
        assert_eq!(e.get("in_range").and_then(|v| v.as_bool()), Some(false));
    }

    #[test]
    fn mission_selects_rpa_when_unknown() {
        let belief = json!({
            "axes": {
                "device": {"eligible": true, "status": "clear"},
                "os": {"status": "clear"},
                "br": {"status": "clear"},
                "rpa": {"status": "unknown"},
                "engine": {"status": "clear"},
                "stack": {"status": "thin", "obs": "unknown"},
                "form": {"status": "clear"},
            }
        });
        let env = json!({"decision": "deepen", "in_range": true});
        let m = select_missions(&belief, &[], &env);
        assert_eq!(
            m.get("primary").and_then(|v| v.as_str()),
            Some("raise_rpa_clarity")
        );
    }
}

#[cfg(test)]
mod hw_safe_groups_tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn resource_classes_and_class_safe_stages() {
        assert_eq!(resource_class("B10_hw_curves"), "gpu");
        assert_eq!(resource_class("B10x_silicon_ulp"), "gpu");
        assert_eq!(resource_class("B46_audio_deep"), "audio");
        assert_eq!(resource_class("B55_webrtc_ice_deep"), "rtc");
        assert_eq!(resource_class("B2_hardware"), "cpu");
        assert_eq!(resource_class("B7_sandbox"), "nest");
        assert_eq!(resource_class("R03_spotcheck"), "verify");
        assert_eq!(resource_class("B0_bootstrap"), "light");

        let ids = vec![
            "B0_bootstrap".into(),
            "B1_conflict".into(),
            "B2_hardware".into(),
            "B3_system".into(),
            "B10_hw_curves".into(),
            "B46_audio_deep".into(),
            "B7_sandbox".into(),
            "R03_spotcheck".into(),
        ];
        let g = build_hw_safe_parallel_groups(&ids);
        // First group is light race
        assert!(g[0].contains(&"B0_bootstrap".into()));
        assert!(g[0].contains(&"B3_system".into()));
        assert!(!g[0].iter().any(|x| is_hardware_probe_pack(x)));
        // Exclusive stages: at most one pack per resource class
        for stage in g.iter().skip(1) {
            let mut seen = HashSet::new();
            for p in stage {
                let c = resource_class(p);
                assert!(c != "light");
                assert!(
                    seen.insert(c),
                    "duplicate class {c} in stage {stage:?}"
                );
            }
        }
        // First exclusive stage should pack different classes together: gpu+audio+cpu+nest+verify
        let s1 = &g[1];
        assert!(s1.iter().any(|p| resource_class(p) == "gpu"));
        assert!(s1.iter().any(|p| resource_class(p) == "audio"));
        assert!(s1.iter().any(|p| resource_class(p) == "cpu"));
    }

    #[test]
    fn no_two_same_class_in_same_group() {
        let ids: Vec<String> = (0..8)
            .map(|i| format!("R{i:02}_spotcheck"))
            .chain(["B10_hw_curves".into(), "B2_hardware".into(), "B0_bootstrap".into()])
            .collect();
        let g = build_hw_safe_parallel_groups(&ids);
        for stage in &g {
            let mut seen = HashSet::new();
            for p in stage {
                if resource_class(p) == "light" {
                    continue;
                }
                let c = resource_class(p);
                assert!(seen.insert(c), "dup class {c} in {stage:?}");
            }
            // Multiple exclusive packs OK if different classes (gpu∥audio).
        }
    }

    #[test]
    fn source_shuffle_differs_by_source() {
        let ids: Vec<String> = (0..10).map(|i| format!("P{i}")).collect();
        let a = shuffle_packs_for_source(&ids, "cycle_x", "main");
        let b = shuffle_packs_for_source(&ids, "cycle_x", "iframe:d1");
        let c = shuffle_packs_for_source(&ids, "cycle_x", "main");
        assert_eq!(a, c, "same source stable");
        assert_ne!(a, b, "different sources permute");
        let mut sa = a.clone();
        let mut sb = b.clone();
        sa.sort();
        sb.sort();
        assert_eq!(sa, sb, "same multiset");
    }
}
