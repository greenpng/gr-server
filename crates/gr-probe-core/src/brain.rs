//! Session brain v2: static wave + gaps → frontier route_plan (dynamic deepen).
//!
//! Static packs kick without prior analyze (short-visit SLA).
//! Dynamic packs (mid4/deep amplify) only via route_plan after evidence/gaps + Soft gate.

use crate::catalog::{load_catalog, PackDef};
use crate::contracts::{
    batch_ids_from_evidence, coverage_for_batches, present_packages, validate_route_plan,
    ContractError,
};
use crate::product_matrix::load_task_gap_map;
use crate::session_ticket::should_skip_session_probe;
use crate::xsrc::{evaluate_xsrc, TruthResult};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct Gap {
    pub code: String,
    pub severity: String,
    pub detail: String,
}

impl Gap {
    pub fn to_value(&self) -> Value {
        json!({
            "code": self.code,
            "severity": self.severity,
            "detail": self.detail,
        })
    }
}

#[derive(Debug, Clone)]
pub struct FrontierPlan {
    pub session_id: String,
    pub gaps: Vec<Gap>,
    pub packs: Vec<Value>,
    pub parallel_groups: Vec<Vec<String>>,
    pub stop_probe: bool,
    pub soft_v2_ready: bool,
    pub amplify_allowed: bool,
    pub notes: Vec<String>,
    pub plan_version: u64,
    pub static_pack_ids: Vec<String>,
    pub dynamic_pack_ids: Vec<String>,
    /// Plan checklist: static + B8 + (mid when soft). NOT real_band terminal.
    pub coverage: Value,
    /// B3 conf-sufficiency: per-axis convergence ∧ xsrc consistency ∧ budget.
    pub conf_sufficiency: Value,
    /// B4 numeric-verify closed loop: divergent A3 slots → value-compare
    /// spotcheck target + spoof uplift hint. Empty object when nothing diverged.
    pub numeric_verify: Value,
    /// B1 axis budgets: per-axis {budget, used} snapshot after trimming.
    pub axis_budget: Value,
}

impl FrontierPlan {
    pub fn to_route_plan(&self) -> Result<Value, ContractError> {
        let plan = json!({
            "version": "v5.2-directions",
            "session_id": self.session_id,
            "strategy_id": "strategy_directional_ucb",
            "dag_version": 2,
            "plan_version": self.plan_version,
            // iss/72: plan_epoch aliases plan_version for FE stale rejection.
            "plan_epoch": self.plan_version,
            "packs": self.packs,
            "parallel_groups": self.parallel_groups,
            "stop_probe": self.stop_probe,
            "static_pack_ids": self.static_pack_ids,
            "dynamic_pack_ids": self.dynamic_pack_ids,
            "notes": self.notes.join("; "),
            "coverage_complete": self.coverage.get("coverage_complete").and_then(|v| v.as_bool()).unwrap_or(false),
            "sandbox_plan": self.coverage.get("sandbox_plan").cloned().unwrap_or(json!(null)),
            "direction_ranking": self.coverage.get("direction_ranking").cloned().unwrap_or(json!([])),
            "conf_sufficiency": self.conf_sufficiency,
            "numeric_verify": self.numeric_verify,
            "axis_budget": self.axis_budget,
        });
        validate_route_plan(&plan)
    }

    pub fn to_value(&self) -> Result<Value, ContractError> {
        let route = self.to_route_plan()?;
        Ok(json!({
            "session_id": self.session_id,
            "gaps": self.gaps.iter().map(|g| g.to_value()).collect::<Vec<_>>(),
            "packs": self.packs,
            "parallel_groups": self.parallel_groups,
            "stop_probe": self.stop_probe,
            "soft_v2_ready": self.soft_v2_ready,
            "amplify_allowed": self.amplify_allowed,
            "plan_version": self.plan_version,
            "plan_epoch": self.plan_version,
            "static_pack_ids": self.static_pack_ids,
            "dynamic_pack_ids": self.dynamic_pack_ids,
            "notes": self.notes,
            "coverage": self.coverage,
            "conf_sufficiency": self.conf_sufficiency,
            "numeric_verify": self.numeric_verify,
            "axis_budget": self.axis_budget,
            "route_plan": route,
        }))
    }
}

fn sources_from(evidence: &Value) -> HashSet<String> {
    let mut sources = HashSet::new();
    if let Some(arr) = evidence.get("sources").and_then(|v| v.as_array()) {
        for s in arr {
            if let Some(st) = s.as_str() {
                sources.insert(st.split(':').next().unwrap_or(st).to_string());
            }
        }
    }
    if let Some(batches) = evidence.get("batches").and_then(|v| v.as_array()) {
        for b in batches {
            if let Some(st) = b.get("source").and_then(|v| v.as_str()) {
                sources.insert(st.split(':').next().unwrap_or(st).to_string());
            }
        }
    }
    if evidence
        .get("has_gateway")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        sources.insert("gateway".into());
    }
    // Cloudflare only if verified edge fields exist — never client has_cloudflare flag alone.
    let cf_verified = evidence
        .get("cf_fields")
        .and_then(|v| v.as_object())
        .map(|cf| {
            ["bot_score", "country", "cf_ray", "colo", "asn"]
                .iter()
                .any(|k| cf.get(*k).is_some_and(|v| !v.is_null() && v.as_str() != Some("")))
        })
        .unwrap_or(false);
    if cf_verified {
        sources.insert("cloudflare".into());
    } else {
        // Drop untrusted client-asserted cloudflare from source set used for gaps.
        sources.remove("cloudflare");
    }
    sources
}

/// Present batch ids including catalog alias normalization (B8_gateway vs B8_gateway_early).
fn present_batches(evidence: &Value) -> HashSet<String> {
    let catalog = load_catalog().ok();
    let mut present = HashSet::new();
    for id in batch_ids_from_evidence(evidence) {
        present.insert(id.clone());
        if let Some(cat) = catalog.as_ref() {
            if let Some(def) = cat.resolve(&id) {
                present.insert(def.batch_id.clone());
                present.insert(def.pack_id.clone());
            }
        }
        // Common aliases
        if id == "B8_gateway" {
            present.insert("B8_gateway_early".into());
        }
        if id == "B8_gateway_early" {
            present.insert("B8_gateway".into());
        }
    }
    present
}

/// Planned probe checklist — **maximize schedule** (v5.8.54+).
///
/// Product terminal ≠ real_band and ≠ dh_ alone. Final only when brain schedule floors land:
/// - static: catalog static packs (except CF-only when not CF)
/// - gateway: B8
/// - identity_core: B0/B1/B2/B3/B10/B11/B12 (commercial + interaction + anti-camouflage)
/// - mid_enrich: soft-on mid/deepen floor (B7 sandbox, curves, webgpu, hedge densify…)
/// - dense_open: high-sev dense gap codes must land their packs
/// - r_spotcheck: ≥1 R00–R99 authenticity pack received
/// - multi_source: main + ≥1 of gateway/worker/iframe/sandbox (when soft on)
///
/// Missing fields still score demotions — they do not authorize early stop.
pub fn coverage_checklist(
    evidence: &Value,
    present: &HashSet<String>,
    soft_v2_ready: bool,
    allow_amplify: bool,
) -> Result<Value, ContractError> {
    let cat = load_catalog()?;
    let sources = sources_from(evidence);
    let has_present = |id: &str| present.contains(id);

    let mut static_required: Vec<String> = Vec::new();
    let mut static_missing: Vec<String> = Vec::new();
    for p in cat.static_packs() {
        // edge.cf only when CF inject path
        if p.pack_id == "edge.cf" || p.source == "cloudflare" {
            let inject = evidence
                .pointer("/meta/inject_path")
                .or_else(|| evidence.pointer("/fields/inject_path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if inject != "cf_worker" && !sources.contains("cloudflare") {
                continue;
            }
        }
        static_required.push(p.batch_id.clone());
        if !present.contains(&p.batch_id) && !present.contains(&p.pack_id) {
            static_missing.push(p.batch_id.clone());
        }
    }
    // dedupe required list
    static_required.sort();
    static_required.dedup();
    static_missing.sort();
    static_missing.dedup();

    let gateway_present = has_present("B8_gateway")
        || has_present("B8_gateway_early")
        || sources.iter().any(|s| s == "gateway" || s.starts_with("gateway"));
    let gateway_required = true;

    // Identity core (always): hard anchors + interaction / anti-camouflage.
    let identity_core: &[&str] = &[
        "B0_bootstrap",
        "B1_conflict",
        "B2_hardware",
        "B3_system",
        "B10_hw_curves",
        "B11_interaction",
        "B12_anti_camouflage",
    ];
    let mut identity_missing: Vec<String> = Vec::new();
    for id in identity_core {
        if !has_present(id) {
            identity_missing.push((*id).into());
        }
    }
    let identity_complete = identity_missing.is_empty();

    // Soft-on mid/enrich: sandbox must + multi-direction mid/dense enrich floor (count-based
    // so capability-missing packs demote scores without pinning forever; brain still schedules).
    let mid_must: &[&str] = &["B7_sandbox"];
    // Maximize schedule: silicon + media + nest + worker + deepen packs for full analysis.
    // Priority order is applied when planning packs (B10 first, then B19 webcodecs, B10x…).
    let mid_enrich_ids: &[&str] = &[
        "B13_authorized",
        "B15_cross_curves",
        "B16_fast_signals",
        "B18_webgpu",
        "B19_eme_media",
        "B20_pohw_challenge",
        "B21_font_canvas",
        "B22_gpu_timer",
        "B23_native_canvas_hedge",
        "B27_storage_privacy",
        "B30_gpu_bandwidth",
        "B31_shader_ulp",
        "B34_cache_ladder",
        "B36_raster_edge",
        "B44_speech_deep",
        "B46_audio_deep",
        "B50_font_matrix_detail",
        "B65_client_hints_full",
        "B77_worker_env_deep",
        "B80_r100_high_value",
    ];
    /// Soft-on: require more mid_enrich packs — maximize probe for comprehensive analysis.
    /// Not "enough to analyze early"; analysis still runs on schedule arms with partial data.
    const MID_ENRICH_MIN: usize = 10;
    let mid_planned = allow_amplify && soft_v2_ready;
    let mut mid_missing: Vec<String> = Vec::new();
    let mut mid_enrich_hit = 0usize;
    if mid_planned {
        for id in mid_must {
            if !has_present(id) {
                mid_missing.push((*id).into());
            }
        }
        for id in mid_enrich_ids {
            if has_present(id) {
                mid_enrich_hit += 1;
            }
        }
        if mid_enrich_hit < MID_ENRICH_MIN {
            mid_missing.push(format!(
                "mid_enrich_count<{MID_ENRICH_MIN}(have={mid_enrich_hit})"
            ));
        }
    }
    let mid_complete = !mid_planned || mid_missing.is_empty();

    // Dense high-sev obligations from open gap codes (same map as dense_digest_slots).
    let dense_gap_map: &[(&str, &str)] = &[
        ("need_api_flags_detail", "B47_api_flags_detail"),
        ("need_font_matrix_detail", "B50_font_matrix_detail"),
        ("need_webrtc_ice_deep", "B55_webrtc_ice_deep"),
        ("need_webgl_params_full", "B59_webgl_params_full"),
        ("need_navigator_deep", "B52_navigator_deep"),
        ("need_client_hints_full", "B65_client_hints_full"),
        ("need_ua_ch_high_entropy", "B65_client_hints_full"),
        ("need_storage_estimate_async", "B27_storage_privacy"),
        ("need_webgpu_adapter_async", "B18_webgpu"),
        ("need_speech_voices_async", "B44_speech_deep"),
        ("need_media_capabilities_async", "B19_eme_media"),
        ("need_worker_iframe_deep", "B77_worker_env_deep"),
        ("need_r100_high_value", "B80_r100_high_value"),
        ("need_gpu_ns_dense", "B59_webgl_params_full"),
    ];
    let mut gap_codes: HashSet<String> = evidence
        .pointer("/meta/open_gap_codes")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(|s| s.to_string()))
        .chain(
            evidence
                .get("gaps")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .filter_map(|g| {
                    g.get("code")
                        .and_then(|c| c.as_str())
                        .map(|s| s.to_string())
                }),
        )
        .collect();
    // Derive open dense gaps from live scan so coverage matches what brain will schedule.
    if let Ok(scanned) = scan_gaps(evidence, None) {
        for g in scanned {
            gap_codes.insert(g.code);
        }
    }
    let mut dense_required: Vec<String> = Vec::new();
    let mut dense_missing: Vec<String> = Vec::new();
    for (code, pid) in dense_gap_map {
        if gap_codes.contains(*code) {
            // Research-gated packs (B18/B47 families) are not default-schedule
            // obligations; their gaps stay observability notes, not coverage debt.
            if let Some(def) = cat.resolve(pid) {
                if def.gate == "research" {
                    continue;
                }
            }
            if !dense_required.iter().any(|x| x == pid) {
                dense_required.push((*pid).into());
            }
            if !has_present(pid) && !dense_missing.iter().any(|x| x == pid) {
                dense_missing.push((*pid).into());
            }
        }
    }
    // Soft-on: require at least one dense pack landed when amplify (proves dense path ran).
    let dense_any_present = present.iter().any(|id| crate::brain_control::is_dense_pack_id(id));
    let dense_floor_ok = if mid_planned {
        dense_any_present || dense_required.is_empty()
    } else {
        true
    };
    // Dense complete only when open dense obligations landed (product: get more data).
    // Do not soft-stop on silicon alone — that left dense B packs unscheduled.
    let dense_complete = dense_missing.is_empty() && dense_floor_ok;

    // R100 authenticity lane: ≥1 Rxx_spotcheck received (not commercial must, but schedule-final must).
    let r_received: Vec<String> = present
        .iter()
        .filter(|id| crate::brain_control::is_verify_rand_pack(id))
        .cloned()
        .collect();
    let r_spotcheck_complete = !r_received.is_empty();

    // Multi-source: main + at least one of gateway / worker / iframe / sandbox nest.
    let source_kinds: HashSet<String> = sources
        .iter()
        .map(|s| {
            let base = s.split(':').next().unwrap_or(s);
            base.to_string()
        })
        .collect();
    let has_main = source_kinds.contains("main") || source_kinds.is_empty();
    let has_aux = ["gateway", "worker", "iframe", "sandbox", "nest"]
        .iter()
        .any(|k| source_kinds.iter().any(|s| s == *k || s.starts_with(k)));
    // Soft-on maximize: require multi-source corroboration (gateway and/or nest).
    let multi_source_planned = mid_planned || gateway_required;
    let multi_source_complete = if multi_source_planned {
        (has_main || has_present("B0_bootstrap")) && (has_aux || gateway_present)
    } else {
        true
    };
    // Soft-on: prefer nest after B7 (worker/iframe). If B7 landed, require nest source.
    let nest_required = mid_planned && has_present("B7_sandbox");
    let nest_complete = if nest_required {
        source_kinds.iter().any(|s| {
            s == "worker"
                || s == "iframe"
                || s == "sandbox"
                || s.starts_with("worker")
                || s.starts_with("iframe")
        })
    } else {
        true
    };

    let static_complete = static_missing.is_empty();
    let gateway_complete = !gateway_required || gateway_present;
    let coverage_complete = static_complete
        && gateway_complete
        && identity_complete
        && mid_complete
        && dense_complete
        && r_spotcheck_complete
        && multi_source_complete
        && nest_complete;

    // DAG-style explain for missing batches (FE probe_dag.json aligned).
    // Soft depends: who must land first / why this gap blocks commercial path.
    let dag_depends: &[(&str, &[&str], &str)] = &[
        ("B10_hw_curves", &["B0_bootstrap"], "commercial_primary_silicon"),
        ("B10x_silicon_noderiv", &["B10_hw_curves"], "deepen_after_primary"),
        ("B10x_silicon_rint", &["B10_hw_curves"], "deepen_after_primary"),
        ("B10x_silicon_ulp", &["B10_hw_curves"], "deepen_after_primary"),
        ("B18_webgpu", &["B10_hw_curves"], "secondary_gpu_after_primary"),
        ("B46_audio_deep", &["B10_hw_curves"], "secondary_audio_after_primary"),
        ("B7_sandbox", &["B0_bootstrap"], "nest_corroboration"),
        ("B11_interaction", &["B0_bootstrap"], "behavior_risk_not_device_id"),
        ("B12_anti_camouflage", &["B2_hardware"], "lie_consistency"),
        ("B2_hardware", &["B0_bootstrap"], "l1_identity"),
        ("B3_system", &["B0_bootstrap"], "l1_identity"),
        ("B8_gateway", &[], "edge_network_truth"),
        ("B4_mobile", &["B0_bootstrap"], "device_form_display"),
        ("B29_sensors_battery", &["B0_bootstrap"], "device_power_sensors"),
    ];
    let mut all_missing: Vec<String> = Vec::new();
    all_missing.extend(identity_missing.iter().cloned());
    all_missing.extend(static_missing.iter().cloned());
    all_missing.extend(mid_missing.iter().cloned());
    all_missing.extend(dense_missing.iter().cloned());
    if !gateway_complete {
        all_missing.push("B8_gateway".into());
    }
    all_missing.sort();
    all_missing.dedup();
    let mut dag_explain: Vec<Value> = Vec::new();
    for mid in &all_missing {
        // mid_enrich_count tokens are not batch ids
        if mid.contains('<') || mid.contains('(') {
            dag_explain.push(json!({
                "missing": mid,
                "kind": "count_floor",
                "depends_on": [],
                "note": "schedule_more_mid_enrich_packs",
            }));
            continue;
        }
        let mut matched = false;
        for (bid, deps, note) in dag_depends {
            if mid == *bid || mid.starts_with(&format!("{bid}")) {
                let blocked_by: Vec<String> = deps
                    .iter()
                    .filter(|d| !has_present(d))
                    .map(|d| (*d).to_string())
                    .collect();
                dag_explain.push(json!({
                    "missing": mid,
                    "kind": "batch",
                    "depends_on": deps,
                    "blocked_by_unmet_deps": blocked_by,
                    "note": note,
                    "hard_primary": *bid == "B10_hw_curves",
                }));
                matched = true;
                break;
            }
        }
        if !matched {
            dag_explain.push(json!({
                "missing": mid,
                "kind": "batch",
                "depends_on": ["B0_bootstrap"],
                "note": "catalog_or_dense_obligation",
            }));
        }
    }

    // ─── State interpretability (§4.2): planes_cover + per-gap missing_state ───
    // Planes: S0=edge truth, S1=identity hardware, S2=multi-source/nest, S3=behavior.
    let s0_present = gateway_present || source_kinds.contains("cloudflare");
    let s1_present = ["B0_bootstrap", "B1_conflict", "B2_hardware", "B3_system", "B10_hw_curves"]
        .iter()
        .any(|id| has_present(id));
    let s2_present = has_present("B7_sandbox")
        || source_kinds.iter().any(|k| {
            k == "worker"
                || k == "iframe"
                || k == "sandbox"
                || k.starts_with("worker")
                || k.starts_with("iframe")
        });
    let s3_present = has_present("B11_interaction") || has_present("B12_anti_camouflage");
    let planes_cover = json!({
        "S0_edge": if s0_present { "observed" } else { "not_observed" },
        "S1_identity": if s1_present { "observed" } else { "not_observed" },
        "S2_multisource": if s2_present { "observed" } else { "not_observed" },
        "S3_behavior": if s3_present { "observed" } else { "not_observed" },
    });
    // Short-visit minimum: at least S0 (edge) or S1 (identity) observed. A session
    // with neither is not_observed at the floor level, however deep other planes are.
    let short_visit_min_cover = s0_present || s1_present;

    // Map each missing batch id to its catalog missing_state (degraded/unsupported/
    // blocked/timeout/not_observed) so coverage edges explain *why* not observed.
    let mut missing_states: Map<String, Value> = Map::new();
    for mid in &all_missing {
        if mid.contains('<') || mid.contains('(') {
            missing_states.insert(
                mid.clone(),
                json!({"missing_state": "not_observed", "kind": "count_floor"}),
            );
            continue;
        }
        let state = cat
            .resolve(mid)
            .map(|d| {
                let dag = d.probe_dag_v2();
                (
                    dag.get("missing_state")
                        .and_then(|v| v.as_str())
                        .unwrap_or("degraded")
                        .to_string(),
                    d.gate.clone(),
                )
            })
            .unwrap_or_else(|| ("not_observed".into(), "default".into()));
        missing_states.insert(
            mid.clone(),
            json!({
                "missing_state": state.0,
                "gate": state.1,
            }),
        );
    }

    Ok(json!({
        "static_complete": static_complete,
        "static_required": static_required,
        "static_missing": static_missing,
        "planes_cover": planes_cover,
        "short_visit_min_cover": short_visit_min_cover,
        "missing_states": Value::Object(missing_states),
        "gateway_complete": gateway_complete,
        "gateway_required": gateway_required,
        "identity_complete": identity_complete,
        "identity_missing": identity_missing,
        "mid_planned": mid_planned,
        "mid_complete": mid_complete,
        "mid_missing": mid_missing,
        "mid_must": mid_must,
        "mid_enrich_ids": mid_enrich_ids,
        "mid_enrich_hit": mid_enrich_hit,
        "mid_enrich_min": MID_ENRICH_MIN,
        "dense_planned": mid_planned || !dense_required.is_empty(),
        "dense_complete": dense_complete,
        "dense_required": dense_required,
        "dense_missing": dense_missing,
        "dense_any_present": dense_any_present,
        "r_spotcheck_complete": r_spotcheck_complete,
        "r_received": r_received,
        "multi_source_planned": multi_source_planned,
        "multi_source_complete": multi_source_complete,
        "nest_required": nest_required,
        "nest_complete": nest_complete,
        "source_kinds": source_kinds.into_iter().collect::<Vec<_>>(),
        "coverage_complete": coverage_complete,
        "dag_explain": dag_explain,
        "dag_algo": "gr_probe_dag_explain_v1",
        "terminal_means": "full_brain_schedule_static_mid_dense_r_multisource",
        "policy": "maximize_probe_v5854",
    }))
}

pub fn scan_gaps(evidence: &Value, truth: Option<&crate::xsrc::TruthResult>) -> Result<Vec<Gap>, ContractError> {
    let fields = evidence
        .get("fields")
        .cloned()
        .unwrap_or(Value::Object(Map::new()));
    let sources = sources_from(evidence);
    let batch_ids = batch_ids_from_evidence(evidence);
    let pkgs = present_packages(&fields, &sources)?;
    let cov = coverage_for_batches(&batch_ids)?;

    let owned;
    let t = if let Some(tr) = truth {
        tr
    } else {
        owned = evaluate_xsrc(evidence, None)?;
        &owned
    };

    let mut gaps = Vec::new();
    if t.xsrc_status == "conflict" {
        gaps.push(Gap {
            code: "xsrc_conflict".into(),
            severity: "high".into(),
            detail: "claim/obs or S0×FE conflict".into(),
        });
    }
    // Demo→v5: residual / stack fusion missing while GPU label present → request B10 curves
    // (static pack already scheduled; gap documents authenticity incompleteness).
    {
        let fo = fields.as_object().cloned().unwrap_or_default();
        let has_gpu = fo
            .get("webgl_unmasked_renderer")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty());
        let has_webgl_curve = fo
            .get("hw_curve_webgl")
            .and_then(|v| v.as_array())
            .is_some_and(|a| a.len() >= 4);
        // Material digests (post-slim) also count as residual/silicon presence.
        let has_material_silicon = fo
            .get("hw_webgl_stable")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty())
            || fo
                .get("hw_audio_stable")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty())
            || fo
                .get("hw_canvas_stable")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty());
        let has_residual = fo.get("residual_mean").and_then(|v| v.as_f64()).is_some()
            || fo
                .get("stack_class")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty() && s != "unknown")
            || fo.get("residual_soft_like").and_then(|v| v.as_bool()).is_some()
            || has_webgl_curve
            || has_material_silicon
            || fo
                .get("webgl_residual_entropy_ok")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
        // Commercial already mint with real_curves → do not reopen residual storm.
        let commercial_done = evidence
            .get("commercial_identity_final")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || fo
                .get("device_id")
                .and_then(|v| v.as_str())
                .is_some_and(crate::device_tier::is_commercial_device_id)
            || evidence
                .pointer("/device/device_id")
                .and_then(|v| v.as_str())
                .is_some_and(crate::device_tier::is_commercial_device_id)
            || evidence
                .pointer("/device/multi_segment")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            || evidence
                .pointer("/device/device_tier")
                .and_then(|v| v.as_str())
                == Some("multi");
        let real_curves = evidence
            .pointer("/device/digest_path")
            .or_else(|| evidence.get("digest_path"))
            .and_then(|v| v.as_str())
            .is_some_and(|s| s.contains("real_curves"));
        // iss/43 P1 · phase B: webgl_unmasked without hw_curve → highest priority B10;
        // commercial finalize must wait for curves (not labels alone).
        if has_gpu && !has_webgl_curve && !has_material_silicon && !(commercial_done && real_curves)
        {
            gaps.push(Gap {
                code: "need_b10_curves_before_finalize".into(),
                severity: "high".into(),
                detail: "webgl_unmasked present without hw_curve_webgl — force B10, defer commercial finalize"
                    .into(),
            });
        }
        // Class-K texture caps missing while silicon already landed — short dwell / B2↔B10
        // race. Keep probing until gl_max_texture_size (or alias) is present.
        let has_tex_cap = fo.get("gl_max_texture_size").is_some()
            || fo.get("webgl_max_texture").is_some()
            || fo.get("max_texture_size").is_some()
            || fo.get("claimed_max_tex").is_some();
        if (has_material_silicon || has_webgl_curve || has_residual) && !has_tex_cap {
            gaps.push(Gap {
                code: "need_gl_texture_caps_for_class_k".into(),
                severity: "high".into(),
                detail: "silicon present without gl_max_texture_size — schedule B2/B10 caps before CIF"
                    .into(),
            });
        }
        if has_gpu && !has_residual && !(commercial_done && real_curves) {
            gaps.push(Gap {
                code: "stack_residual_missing".into(),
                severity: "high".into(),
                detail: "GPU label without residual/stack_class — schedule B2 residual + B10 hw curves"
                    .into(),
            });
        }
        // Host separator missing while residual/curves present → re-probe ICE / OS instance.
        let has_host_sep = fo
            .get("webrtc_host_ip_hash")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty())
            || fo
                .get("os_instance_hash")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty());
        if has_residual && !has_host_sep {
            gaps.push(Gap {
                code: "need_host_sep_for_dh".into(),
                severity: "high".into(),
                detail: "residual/curves without webrtc/os_instance host sep — schedule B3/B9/B10 host materials before dh"
                    .into(),
            });
        }
        // Probe-DAG integrity (A4): FE B0 ships dag metadata about pack scheduling.
        // Skipped steps mean the probe stream itself was truncated — a browser
        // integrity proxy that must surface as a re-probe gap.
        let dag_skips = fo
            .get("dag_v2_skipped_n")
            .and_then(|v| v.as_u64().or_else(|| v.as_i64().map(|i| i as u64)))
            .unwrap_or(0);
        if dag_skips > 0 {
            let dag_ver = fo
                .get("dag_v2_version")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let dag_engine = fo
                .get("dag_v2_engine")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            gaps.push(Gap {
                code: "probe_dag_skipped".into(),
                severity: if dag_skips >= 3 { "high" } else { "medium" }.into(),
                detail: format!(
                    "FE probe DAG skipped {dag_skips} steps (dag_v2={dag_ver} engine={dag_engine}) — probe stream incomplete"
                ),
            });
        }
        // Pack-health (all-fields-utilized layer): FE packs carrying explicit
        // failure/incomplete marks surfaced diagnostics but no material —
        // schedule targeted re-probe rather than treating the session as thin.
        {
            let (pack_failed, pack_incomplete) = crate::utilization::pack_health_marks(&fo);
            if pack_failed > 0 || pack_incomplete > 0 {
                gaps.push(Gap {
                    code: "probe_pack_incomplete".into(),
                    severity: if pack_failed > 0 { "high" } else { "medium" }.into(),
                    detail: format!(
                        "FE probe packs incomplete: failed={pack_failed} incomplete={pack_incomplete} — schedule targeted re-probe"
                    ),
                });
            }
        }
        // Never gap for host-silicon recovery (not a product goal).
    }
    if t.xsrc_status == "missing_server"
        || sources
            .intersection(&HashSet::from([
                "gateway".to_string(),
                "cloudflare".to_string(),
            ]))
            .next()
            .is_none()
    {
        gaps.push(Gap {
            code: "missing_gateway_or_cf".into(),
            severity: "high".into(),
            detail: "need B8/gateway or CF edge".into(),
        });
    }
    if !cov
        .get("lite5")
        .and_then(|v| v.get("full"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let missing = cov
            .get("lite5")
            .and_then(|v| v.get("missing"))
            .cloned()
            .unwrap_or(json!([]));
        gaps.push(Gap {
            code: "lite5_incomplete".into(),
            severity: "high".into(),
            detail: format!("missing {missing}"),
        });
    }
    if !pkgs
        .get("PKG_AUTO")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        gaps.push(Gap {
            code: "missing_automation".into(),
            severity: "high".into(),
            detail: "need B1 automation pack".into(),
        });
    }
    if !pkgs.get("PKG_GPU").and_then(|v| v.as_bool()).unwrap_or(false)
        && !pkgs
            .get("PKG_GEOCPU")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    {
        gaps.push(Gap {
            code: "missing_link_anchors".into(),
            severity: "mid".into(),
            detail: "need GPU or screen+tz+cores".into(),
        });
    }
    if !pkgs.get("PKG_GPU").and_then(|v| v.as_bool()).unwrap_or(false) {
        gaps.push(Gap {
            code: "missing_gpu".into(),
            severity: "mid".into(),
            detail: "B2 webgl unmasked".into(),
        });
    }
    if !cov
        .get("mid4")
        .and_then(|v| v.get("any"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        gaps.push(Gap {
            code: "mid_absent".into(),
            severity: "low".into(),
            detail: "optional mid4 for longer dwell".into(),
        });
    }
    // Multi-source sandbox absent
    let has_sandbox = sources.iter().any(|s| {
        s.starts_with("iframe")
            || s.starts_with("worker")
            || s.starts_with("sandbox")
            || s.contains("sandbox")
    }) || batch_ids.iter().any(|b| b == "B7_sandbox");
    if !has_sandbox {
        gaps.push(Gap {
            code: "missing_sandbox".into(),
            severity: "low".into(),
            detail: "sandbox iframe/worker multi-source".into(),
        });
    }
    if !t.has_main_core {
        gaps.push(Gap {
            code: "thin_main".into(),
            severity: "high".into(),
            detail: "no main_core — insufficient for confirm".into(),
        });
    }

    // --- Product-task gaps (norm/11 · J17): shared evidence, not per-result probe suites ---
    {
        let fo = fields.as_object().cloned().unwrap_or_default();
        let has_form = fo
            .get("form_class")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty())
            || evidence
                .pointer("/device/trust/materials/form_class")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty());
        let has_webgl = fo
            .get("hw_curve_webgl")
            .and_then(|v| v.as_array())
            .is_some_and(|a| a.len() >= 4)
            || fo
                .get("hw_webgl_stable")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty())
            || evidence
                .pointer("/device/trust/materials/hw_webgl_stable")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty());
        let has_audio = fo
            .get("hw_curve_audio")
            .and_then(|v| v.as_array())
            .is_some_and(|a| a.len() >= 4)
            || fo
                .get("hw_audio_stable")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty())
            || evidence
                .pointer("/device/trust/materials/hw_audio_stable")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty());
        let has_curves = has_webgl || has_audio;
        let commercial_minted = evidence
            .get("commercial_identity_final")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || evidence
                .pointer("/device/device_id")
                .and_then(|v| v.as_str())
                .is_some_and(crate::device_tier::is_commercial_device_id)
            || evidence
                .pointer("/device/multi_segment")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            || evidence
                .pointer("/device/device_tier")
                .and_then(|v| v.as_str())
                == Some("multi");
        if (!has_form || !has_curves) && !commercial_minted {
            gaps.push(Gap {
                code: "need_device_hard_anchor".into(),
                severity: "high".into(),
                detail: "task.device_id: missing form and/or hw curves (shared hard materials)"
                    .into(),
            });
        }
        // Cross-browser commercial prefers webgl residual; audio-only sessions (ungoogled
        // thin FE) diverge until B10 webgl arrives — high urgency for device_id task.
        if has_audio && !has_webgl {
            gaps.push(Gap {
                code: "need_device_webgl_residual".into(),
                severity: "high".into(),
                detail: "task.device_id: audio present but hw_curve_webgl missing — schedule B10/B2 for cross-browser machine id"
                    .into(),
            });
        }
        let has_env = fo.get("vm_score").is_some()
            || fo
                .get("stack_class")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty() && s != "unknown")
            || fo.get("residual_mean").is_some()
            || has_curves;
        if !has_env {
            gaps.push(Gap {
                code: "need_os_env".into(),
                severity: "mid".into(),
                detail: "task.os: insufficient env/vm/physical materials".into(),
            });
        }
        let has_auto_surface = fo.get("webdriver").is_some()
            || fo.get("automation").is_some()
            || batch_ids.iter().any(|b| b == "B1_conflict" || b == "B12_anti_camouflage");
        if !has_auto_surface {
            gaps.push(Gap {
                code: "need_br_automation".into(),
                severity: "high".into(),
                detail: "task.br: missing automation / anti-camouflage surface".into(),
            });
        }
        let rpa_bound = fo
            .get("behavior_early_bound")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let rpa_events = fo
            .get("behavior_events")
            .map(|v| !v.is_null())
            .unwrap_or(false)
            || fo
                .get("behavior_count")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
                > 0.0;
        if !rpa_bound {
            gaps.push(Gap {
                code: "need_rpa_bind".into(),
                severity: "high".into(),
                detail: "task.rpa: no early behavior bind".into(),
            });
        } else if !rpa_events {
            gaps.push(Gap {
                code: "need_rpa_events".into(),
                severity: "mid".into(),
                detail: "task.rpa: bound but sparse/no events".into(),
            });
        }
        // iss/21 T-BRAIN-1: kinematics / CDP deepen gaps
        let has_kin = fo.get("input_mouse_entropy").is_some()
            || fo.get("pre_action_move_count").is_some()
            || fo.get("ttfi_ms").is_some()
            || fo.get("integer_coord_ratio").is_some();
        if rpa_bound && rpa_events && !has_kin {
            gaps.push(Gap {
                code: "need_rpa_kinematics".into(),
                severity: "high".into(),
                detail: "task.rpa: events present but bio kinematics vectors missing".into(),
            });
        }
        let cdp_known = fo.get("cdp_runtime_hint").is_some();
        if !cdp_known {
            gaps.push(Gap {
                code: "need_cdp_probe".into(),
                severity: "mid".into(),
                detail: "task.br: cdp_runtime_hint absent — schedule anti-camouflage deepen".into(),
            });
        }
        if t.xsrc_status == "missing_server"
            || sources
                .intersection(&HashSet::from([
                    "gateway".to_string(),
                    "cloudflare".to_string(),
                ]))
                .next()
                .is_none()
        {
            gaps.push(Gap {
                code: "need_gateway_or_cf".into(),
                severity: "high".into(),
                detail: "task.xsrc/os/br conf: need verified server observation".into(),
            });
        }

        // Sandbox multi-source health (product: ≥2 kinds, ≥1 nest with data)
        let sandbox_blocked = fo
            .get("sandbox_blocked")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let sandbox_ok = fo
            .get("sandbox_ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let sandbox_all_empty = fo
            .get("sandbox_all_empty")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || fo
                .get("js_ok_sandbox_dead")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
        let sandbox_under_two = fo
            .get("sandbox_under_two_kinds")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let cap = fo
            .get("sandbox_capability_score")
            .and_then(|v| v.as_f64())
            .unwrap_or(-1.0);
        let payload_n = fo
            .get("sandbox_payload_source_n")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if sandbox_blocked
            || sandbox_all_empty
            || (!sandbox_ok
                && fo.get("sandbox_sources_received").is_none()
                && !batch_ids.iter().any(|b| b == "B7_sandbox"))
        {
            gaps.push(Gap {
                code: "need_br_kernel_consistency".into(),
                severity: "mid".into(),
                detail: "task.br: schedule B7_sandbox multi-source consistency".into(),
            });
        }
        if sandbox_all_empty || (batch_ids.iter().any(|b| b == "B7_sandbox") && payload_n == 0) {
            gaps.push(Gap {
                code: "sandbox_all_empty".into(),
                severity: "high".into(),
                detail: "all nest sandboxes empty/blocked — re-schedule B7 dual-nest".into(),
            });
        }
        if sandbox_under_two || (payload_n > 0 && payload_n < 2) || (cap >= 0.0 && cap < 0.55) {
            gaps.push(Gap {
                code: "sandbox_under_two_kinds".into(),
                severity: "mid".into(),
                detail: "need ≥2 nest kinds with data — deepen sandbox_plan".into(),
            });
        }
        // Expanded system/fonts materials
        if fo.get("font_count").is_none() && fo.get("math_digest").is_none() {
            gaps.push(Gap {
                code: "need_os_env".into(),
                severity: "mid".into(),
                detail: "task.os: schedule B3_system fonts/math media materials".into(),
            });
        }

        // Registered physical/deep gap packs (H06/H10/H16/D37) — brain can schedule.
        let has_webgpu = fo.get("webgpu_adapter").and_then(|v| v.as_bool()).unwrap_or(false)
            || fo
                .get("webgpu_limits")
                .map(|v| !v.is_null())
                .unwrap_or(false);
        if !has_webgpu && !batch_ids.iter().any(|b| b == "B18_webgpu") {
            // Soft/SwiftShader often lacks WebGPU — mid→low under soft residual class.
            let softish = fo
                .get("soft_stack")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                || fo
                    .get("residual_soft_like")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
            gaps.push(Gap {
                code: "need_webgpu_secure_context".into(),
                severity: if softish { "low".into() } else { "mid".into() },
                detail: "iss2 H06: schedule B18_webgpu".into(),
            });
        }
        let has_eme = fo
            .get("eme_systems")
            .map(|v| !v.is_null())
            .unwrap_or(false)
            || fo
                .get("request_media_key_system")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
        if !has_eme && !batch_ids.iter().any(|b| b == "B19_eme_media") {
            gaps.push(Gap {
                code: "need_media_eme".into(),
                severity: "mid".into(),
                detail: "iss2 H10: schedule B19_eme_media".into(),
            });
        }
        let has_challenge = fo
            .get("challenge_residual_mean")
            .and_then(|v| v.as_f64())
            .is_some()
            || fo.get("pohw_triad").map(|v| !v.is_null()).unwrap_or(false);
        if !has_challenge {
            gaps.push(Gap {
                code: "need_challenge_seed".into(),
                severity: "high".into(),
                detail: "D37/H16: schedule B20_challenge_seed (seed-mixed residual)".into(),
            });
            gaps.push(Gap {
                code: "need_pohw_challenge".into(),
                severity: "high".into(),
                detail: "iss2 H16 PoHW multi-surface via B20 (residual+audio+cpu)".into(),
            });
        }
        // Soft residual stack: do not waste budget on host-silicon GPU deep digs.
        // Prefer unit multi-seed / challenge verify / rpa control-plane (demo fleet).
        let soft_stack_obs = fo
            .get("soft_stack")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || fo
                .get("residual_soft_like")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            || fo
                .get("stack_class")
                .and_then(|v| v.as_str())
                .is_some_and(|s| matches!(s, "soft_render" | "software" | "virt"));
        let gpu_label_untrusted = fo
            .get("gpu_label_untrusted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || fo
                .get("spoof_score")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
                >= 0.4;
        if soft_stack_obs {
            gaps.push(Gap {
                code: "soft_stack_budget_rpa_os".into(),
                severity: "mid".into(),
                detail: "task.rpa/os/device: soft residual class — targeted rpa/os/unit packs, not host-silicon GPU dig"
                    .into(),
            });
            // Soft without OS/instance separator → ServerMint collision_risk; target separator.
            let has_os_sep = fo
                .get("os_instance_hash")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty())
                || fo
                    .get("webrtc_host_ip_hash")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| !s.is_empty());
            if !has_os_sep {
                gaps.push(Gap {
                    code: "need_os_instance_separator".into(),
                    severity: "mid".into(),
                    detail: "task.device_id: soft class needs OS/instance or webrtc host for ServerMint fork"
                        .into(),
                });
            }
        }
        if (gpu_label_untrusted || soft_stack_obs) && fo.get("unit_surface_id").is_none() {
            gaps.push(Gap {
                code: "need_unit_surface".into(),
                severity: "high".into(),
                detail: "task.device_id/os/br: multi-seed unit_surface missing — schedule B10 targeted"
                    .into(),
            });
        }
        if gpu_label_untrusted {
            gaps.push(Gap {
                code: "need_claim_obs_verify".into(),
                severity: "high".into(),
                detail: "task.br/os: gpu_label_untrusted — challenge/unit claim-obs verify, not GPU host dig"
                    .into(),
            });
        }
        // br capability surface for kernel integrity (API/caps), not leaf expansion.
        let has_caps = fo.get("webgl2_support").is_some()
            || fo.get("webgl_max_texture").is_some()
            || fo.get("webgl_extensions_hash").is_some();
        if !has_caps
            && (gpu_label_untrusted
                || fo
                    .get("webgl_unmasked_renderer")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| !s.is_empty()))
        {
            gaps.push(Gap {
                code: "need_br_capability_probe".into(),
                severity: "high".into(),
                detail: "task.br: WebGL2/caps/API surface for claim_capability_incoherence".into(),
            });
        }
        // H01 GPU staircase gap
        let has_gpu_stair = fo
            .get("gpu_wall_staircase")
            .map(|v| v.as_array().map(|a| !a.is_empty()).unwrap_or(!v.is_null()))
            .unwrap_or(false)
            || fo.get("gpu_slope_wall").and_then(|v| v.as_f64()).is_some();
        if !has_gpu_stair && !batch_ids.iter().any(|b| b == "B22_gpu_timer") {
            // Soft class: demote — host silicon recovery is not a product goal.
            let sev = if soft_stack_obs { "low" } else { "high" };
            gaps.push(Gap {
                code: "need_gpu_staircase".into(),
                severity: sev.into(),
                detail: "H01: schedule B22_gpu_timer wall/timer staircase".into(),
            });
        }
        // Deep census volume gap (v57-class digests)
        let has_census_vol = fo.get("font_bitmap_hash").is_some()
            || fo.get("api_flags_hash").is_some()
            || fo
                .get("census_leaf_estimate")
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x as i64)))
                .unwrap_or(0)
                > 0;
        if !has_census_vol && !batch_ids.iter().any(|b| b == "B21_census_volume") {
            gaps.push(Gap {
                code: "need_census_volume".into(),
                severity: "mid".into(),
                detail: "v57-class census digests: schedule B21_census_volume".into(),
            });
            gaps.push(Gap {
                code: "need_census_density".into(),
                severity: "mid".into(),
                detail: "census density via B21/B5".into(),
            });
        }
        // Multi-material hedge gaps (cannot trust single field/source)
        let has_native_hedge = fo.get("native_integrity_ratio").is_some()
            || fo.get("canvas_geometry_hash").is_some();
        if !has_native_hedge && !batch_ids.iter().any(|b| b == "B23_native_canvas_hedge") {
            gaps.push(Gap {
                code: "need_native_canvas_hedge".into(),
                severity: "mid".into(),
                detail: "D26/D02 multi-material hedge: schedule B23_native_canvas_hedge".into(),
            });
        }
        let has_cross_vote = fo.get("material_vote_digest").is_some()
            || fo.get("material_count").is_some();
        if !has_cross_vote && !batch_ids.iter().any(|b| b == "B24_material_crosscheck") {
            gaps.push(Gap {
                code: "need_material_crosscheck".into(),
                severity: "mid".into(),
                detail: "multi-material cross-check vote: schedule B24_material_crosscheck".into(),
            });
        }
        // Thin material families → need hedge materials (analysis will refuse single-family real)
        let thin_device = !(fo.get("hw_curve_webgl").is_some()
            && (fo.get("gpu_wall_staircase").is_some()
                || fo.get("pohw_triad").is_some()
                || fo.get("cpu_timing_curve").is_some()));
        if thin_device {
            gaps.push(Gap {
                code: "need_hedge_materials".into(),
                severity: "high".into(),
                detail: "device axis needs ≥2 material families (curves+gpu/pohw/cpu)".into(),
            });
        }
        // H03 shader numeric
        if fo.get("shader_ulp_max").is_none()
            && fo.get("precision_digest").is_none()
            && !batch_ids.iter().any(|b| b == "B31_shader_numeric")
        {
            gaps.push(Gap {
                code: "need_shader_numeric".into(),
                severity: "high".into(),
                detail: "H03: schedule B31_shader_numeric".into(),
            });
        }
        // H05 caps pressure
        if fo.get("actual_max_tex").is_none()
            && !batch_ids.iter().any(|b| b == "B33_caps_pressure")
        {
            gaps.push(Gap {
                code: "need_caps_pressure".into(),
                severity: "high".into(),
                detail: "H05: schedule B33_caps_pressure".into(),
            });
        }
        // H08 clock/rAF
        if fo.get("perf_now_resolution_ms").is_none()
            && fo.get("raf_jitter_cv").is_none()
            && !batch_ids.iter().any(|b| b == "B25_clock_raf")
        {
            gaps.push(Gap {
                code: "need_clock_raf".into(),
                severity: "mid".into(),
                detail: "H08: schedule B25_clock_raf".into(),
            });
        }
        // H10 codec matrix deepen
        let eme_thin = fo.get("codec_matrix").is_none()
            && fo
                .get("eme_systems")
                .and_then(|v| v.as_object())
                .map(|o| o.len() < 2)
                .unwrap_or(true);
        if eme_thin && !batch_ids.iter().any(|b| b == "B19_eme_media") {
            gaps.push(Gap {
                code: "need_eme_codec_matrix".into(),
                severity: "mid".into(),
                detail: "H10: schedule B19_eme_media codec matrix".into(),
            });
        }
        // H01 GPU-ns path
        if fo.get("gpu_ns_staircase").is_none()
            && fo.get("gpu_ns_median").is_none()
            && !batch_ids.iter().any(|b| b == "B22_gpu_timer")
        {
            let sev = if soft_stack_obs { "low" } else { "high" };
            gaps.push(Gap {
                code: "need_gpu_ns".into(),
                severity: sev.into(),
                detail: "H01: schedule B22_gpu_timer GPU-ns path".into(),
            });
        }
        // H16 signed challenge
        if fo.get("challenge_seed_sig").is_none()
            && !batch_ids.iter().any(|b| b == "B20_challenge_seed")
        {
            gaps.push(Gap {
                code: "need_signed_challenge".into(),
                severity: "high".into(),
                detail: "H16: signed challenge via B20".into(),
            });
        }
        // Peripheral
        if fo.get("permissions_matrix").is_none()
            && !batch_ids.iter().any(|b| b == "B28_permissions_media")
        {
            gaps.push(Gap {
                code: "need_permissions_media".into(),
                severity: "mid".into(),
                detail: "B28 permissions/mediaDevices".into(),
            });
        }
        if fo.get("battery_level").is_none()
            && fo.get("sensor_accel_present").is_none()
            && !batch_ids.iter().any(|b| b == "B29_sensors_battery")
        {
            gaps.push(Gap {
                code: "need_sensors_battery".into(),
                severity: "mid".into(),
                detail: "B29 sensors/battery".into(),
            });
        }
        if fo.get("gpu_readback_ladder").is_none()
            && fo.get("roundtrip_intercept_ms").is_none()
            && !batch_ids.iter().any(|b| b == "B30_gpu_bandwidth")
        {
            gaps.push(Gap {
                code: "need_gpu_bandwidth_h02".into(),
                severity: "mid".into(),
                detail: "H02: B30_gpu_bandwidth".into(),
            });
        }
        if fo.get("cpu_cache_ladder").is_none()
            && !batch_ids.iter().any(|b| b == "B34_cpu_cache_ladder")
        {
            gaps.push(Gap {
                code: "need_cpu_cache_ladder".into(),
                severity: "mid".into(),
                detail: "H07: B34_cpu_cache_ladder".into(),
            });
        }
        // iss/74 G2 os/gecko surface cross-check (B3_system corroboration)
        if fo.get("os_kernel_hint").is_none()
            && !batch_ids.iter().any(|b| b == "B86_os_gecko_surface")
        {
            gaps.push(Gap {
                code: "need_os_surface_h20".into(),
                severity: "mid".into(),
                detail: "H20: B86_os_gecko_surface".into(),
            });
        }
        // iss/74 R1 engine behavior diff (realm/engine discriminator)
        if fo.get("engine_error_9set_digest").is_none()
            && !batch_ids.iter().any(|b| b == "B87_engine_behavior_diff")
        {
            gaps.push(Gap {
                code: "need_engine_behavior".into(),
                severity: "mid".into(),
                detail: "R1: B87_engine_behavior_diff".into(),
            });
        }
        // iss/74 H8 wasm instruction-level throughput (CPU microarch C-key)
        if fo.get("wasm_bi_f32_mul_ns").is_none()
            && !batch_ids.iter().any(|b| b == "B88_wasm_instruction_throughput")
        {
            gaps.push(Gap {
                code: "need_wasm_throughput".into(),
                severity: "mid".into(),
                detail: "H8: B88_wasm_instruction_throughput".into(),
            });
        }
        // iss/74 D06 audio known-lock (fake-DSP detection fodder)
        if fo.get("audio_known_lock_sum").is_none()
            && !batch_ids.iter().any(|b| b == "B89_audio_known_lock")
        {
            gaps.push(Gap {
                code: "need_audio_known_lock".into(),
                severity: "mid".into(),
                detail: "D06: B89_audio_known_lock".into(),
            });
        }
        // iss/74 G3 os emoji raster + fallback chain
        if fo.get("emoji_raster_hash").is_none()
            && !batch_ids.iter().any(|b| b == "B90_os_emoji_raster")
        {
            gaps.push(Gap {
                code: "need_emoji_raster".into(),
                severity: "mid".into(),
                detail: "G3: B90_os_emoji_raster".into(),
            });
        }
        // iss/74 K1 storage disk quota (isolation/VM clues)
        if fo.get("storage_quota_grid_class").is_none()
            && !batch_ids.iter().any(|b| b == "B91_storage_disk_quota")
        {
            gaps.push(Gap {
                code: "need_storage_quota".into(),
                severity: "mid".into(),
                detail: "K1: B91_storage_disk_quota".into(),
            });
        }
        // Protocol edge (server-injected); schedule gateway re-kick if thin
        if fo.get("ja4").is_none()
            && fo.get("h2_fingerprint").is_none()
            && fo.get("protocol_engine").is_none()
            && !batch_ids.iter().any(|b| b == "B8_gateway" || b == "B8_gateway_early")
        {
            gaps.push(Gap {
                code: "need_protocol_edge".into(),
                severity: "high".into(),
                detail: "H13: protocol fingerprint via gateway/Pingora TLS fields".into(),
            });
        }
        // Side-channel depth: have TLS JA4 but no QUIC/SYN corroboration yet (not a FE pack gap)
        if fo.get("ja4").is_some()
            && fo.get("quic_tls_ja4").is_none()
            && fo.get("tcp_syn_present").is_none()
        {
            gaps.push(Gap {
                code: "need_protocol_side_depth".into(),
                severity: "low".into(),
                detail: "Pingora side-channel: QUIC/SYN not joined yet (listen may be off)".into(),
            });
        }
        // Full H3 application path: QUIC Initial seen but no H3 request yet
        if (fo.get("quic_listen_present").is_some() || fo.get("quic_tls_ja4").is_some())
            && fo.get("h3_app_present").is_none()
            && !batch_ids.iter().any(|b| b == "B8_gateway" || b == "B8_gateway_early")
        {
            gaps.push(Gap {
                code: "need_h3_app".into(),
                severity: "mid".into(),
                detail: "HTTP/3 app layer: enable --h3-listen and Alt-Svc so client completes H3 request".into(),
            });
        }
        // TCP JA4 vs QUIC JA4 disagree → deepen browser kernel packs
        if let (Some(tj), Some(qj)) = (
            fo.get("ja4").and_then(|v| v.as_str()),
            fo.get("quic_tls_ja4").and_then(|v| v.as_str()),
        ) {
            if !tj.is_empty() && !qj.is_empty() && tj != qj {
                gaps.push(Gap {
                    code: "need_protocol_tcp_quic_reconcile".into(),
                    severity: "mid".into(),
                    detail: "TCP JA4 != QUIC JA4 — schedule B1/B40/B8 re-observe".into(),
                });
            }
        }
        if fo.get("agent_parity_hash").is_none()
            && !batch_ids.iter().any(|b| b == "B26_agent_parity")
        {
            gaps.push(Gap {
                code: "need_agent_parity".into(),
                severity: "mid".into(),
                detail: "v57 agent parity: B26_agent_parity".into(),
            });
        }
        if fo.get("privacy_storage_score").is_none()
            && fo.get("storage_quota").is_none()
            && !batch_ids.iter().any(|b| b == "B27_storage_privacy")
        {
            gaps.push(Gap {
                code: "need_storage_privacy".into(),
                severity: "mid".into(),
                detail: "storage/privacy deep: B27_storage_privacy".into(),
            });
        }
        if fo.get("dom_rect_hash").is_none()
            && fo.get("perf_timeline_hash").is_none()
            && !batch_ids.iter().any(|b| b == "B35_dom_perf")
        {
            gaps.push(Gap {
                code: "need_dom_perf".into(),
                severity: "mid".into(),
                detail: "dom_rect/perf timeline: B35_dom_perf".into(),
            });
        }
        if fo.get("layer_divergence_score").is_none()
            && !batch_ids.iter().any(|b| b == "B15_cross_curves")
        {
            gaps.push(Gap {
                code: "need_layer_divergence".into(),
                severity: "mid".into(),
                detail: "H14 multi-field divergence: B15_cross_curves".into(),
            });
        }
        if fo.get("webgpu_limits_hash").is_none()
            && fo.get("webgpu_dual_adapter_diff").is_none()
            && !batch_ids.iter().any(|b| b == "B18_webgpu")
        {
            gaps.push(Gap {
                code: "need_webgpu_deep".into(),
                severity: "mid".into(),
                detail: "H06 WebGPU deepen: B18_webgpu".into(),
            });
        }
        // iss/74 Phase 2 research pack gaps. Research-gated packs stay
        // observability notes in the default schedule; B93 is default mid.
        if fo.get("atomic_contention_hist16").is_none()
            && !batch_ids.iter().any(|b| b == "B92_webgpu_atomic_contention")
        {
            gaps.push(Gap {
                code: "need_webgpu_atomic_contention".into(),
                severity: "low".into(),
                detail: "iss/74 R2 WebGPU atomic contention (research): B92_webgpu_atomic_contention".into(),
            });
        }
        if fo.get("blink_fork_guess").is_none()
            && !batch_ids.iter().any(|b| b == "B93_blink_fork_matrix")
        {
            gaps.push(Gap {
                code: "need_blink_fork_matrix".into(),
                severity: "mid".into(),
                detail: "iss/74 R3 blink fork matrix: B93_blink_fork_matrix".into(),
            });
        }
        if fo.get("sab_dual_clock_ok").is_none()
            && !batch_ids.iter().any(|b| b == "B94_sab_dual_clock_differential")
        {
            gaps.push(Gap {
                code: "need_sab_dual_clock".into(),
                severity: "low".into(),
                detail: "iss/74 R4 SAB dual clock (research): B94_sab_dual_clock_differential".into(),
            });
        }
        if fo.get("gpu_eu_curve").is_none()
            && !batch_ids.iter().any(|b| b == "B95_gpu_eu_timing")
        {
            gaps.push(Gap {
                code: "need_gpu_eu_timing".into(),
                severity: "low".into(),
                detail: "iss/74 R1 GPU EU timing (research): B95_gpu_eu_timing".into(),
            });
        }
        if fo.get("raster_edge_hash").is_none() && !batch_ids.iter().any(|b| b == "B36_raster_msaa")
        {
            gaps.push(Gap {
                code: "need_raster_msaa".into(),
                severity: "mid".into(),
                detail: "H04 raster/MSAA: B36_raster_msaa".into(),
            });
        }
        if fo.get("thermal_cpu_slope").is_none()
            && !batch_ids.iter().any(|b| b == "B37_thermal_drift_lite")
        {
            gaps.push(Gap {
                code: "need_thermal_drift".into(),
                severity: "low".into(),
                detail: "H09 thermal lite: B37_thermal_drift_lite".into(),
            });
        }
        if fo.get("neg_dict_hits").is_none()
            && fo.get("neg_dict_hit_n").is_none()
            && !batch_ids.iter().any(|b| b == "B38_neg_dict")
        {
            gaps.push(Gap {
                code: "need_neg_dict".into(),
                severity: "mid".into(),
                detail: "H15 neg dictionary: B38_neg_dict".into(),
            });
        }
        if fo.get("mem_alloc_ladder").is_none()
            && !batch_ids.iter().any(|b| b == "B39_mem_pressure")
        {
            gaps.push(Gap {
                code: "need_mem_pressure".into(),
                severity: "mid".into(),
                detail: "D08 mem pressure: B39_mem_pressure".into(),
            });
        }
        if fo.get("ws_handshake_ms").is_none()
            && fo.get("websocket_present").is_none()
            && !batch_ids.iter().any(|b| b == "B40_websocket_fp")
        {
            gaps.push(Gap {
                code: "need_websocket_fp".into(),
                severity: "low".into(),
                detail: "D21 websocket fp: B40_websocket_fp".into(),
            });
        }
        if fo.get("hid_surface_score").is_none()
            && fo.get("gamepad_count").is_none()
            && !batch_ids.iter().any(|b| b == "B41_hid_gamepad")
        {
            gaps.push(Gap {
                code: "need_hid_gamepad".into(),
                severity: "low".into(),
                detail: "D35 HID/gamepad: B41_hid_gamepad".into(),
            });
        }
        // H09 full progressive after lite present or long dwell
        let has_lite_thermal = fo.get("thermal_cpu_slope").is_some();
        let long_dwell = fo
            .get("behavior_count")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
            > 5.0
            || fo.get("page_dwell_ms").and_then(|v| v.as_f64()).unwrap_or(0.0) > 15000.0;
        if fo.get("thermal_cpu_slope_full").is_none()
            && (has_lite_thermal || long_dwell)
            && !batch_ids.iter().any(|b| b == "B42_thermal_drift_full")
        {
            gaps.push(Gap {
                code: "need_thermal_full".into(),
                severity: "mid".into(),
                detail: "H09 full progressive thermal: B42_thermal_drift_full".into(),
            });
        }
        if fo.get("errors_engine_hash").is_none()
            && !batch_ids.iter().any(|b| b == "B43_errors_engine")
        {
            gaps.push(Gap {
                code: "need_errors_engine".into(),
                severity: "mid".into(),
                detail: "v57 errors engine: B43_errors_engine".into(),
            });
        }
        if fo.get("speech_voices_hash").is_none()
            && !batch_ids.iter().any(|b| b == "B44_speech_deep")
        {
            gaps.push(Gap {
                code: "need_speech_deep".into(),
                severity: "low".into(),
                detail: "D14 speech voices: B44_speech_deep".into(),
            });
        }
        if fo.get("display_mq_hash").is_none()
            && !batch_ids.iter().any(|b| b == "B45_display_hdr")
        {
            gaps.push(Gap {
                code: "need_display_hdr".into(),
                severity: "mid".into(),
                detail: "H11 display/HDR: B45_display_hdr".into(),
            });
        }
        // Prefer GPU-ns dense staircase when only wall present (real GPU 1050 Ti path)
        if fo.get("gpu_wall_staircase").is_some()
            && fo.get("gpu_ns_staircase").is_none()
            && !batch_ids.iter().any(|b| b == "B22_gpu_timer")
        {
            gaps.push(Gap {
                code: "need_gpu_ns_dense".into(),
                severity: "high".into(),
                detail: "H01 re-run B22 for GPU-ns on real GPU".into(),
            });
        }
        // timer available but empty ns → force recollect with async poll path
        if fo.get("timer_query_available") == Some(&Value::Bool(true))
            && fo.get("gpu_ns_staircase").is_none()
            && fo.get("gpu_ns_points")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
                < 1.0
        {
            gaps.push(Gap {
                code: "need_gpu_ns_dense".into(),
                severity: "high".into(),
                detail: "H01 timer_query available but gpu_ns empty — force B22".into(),
            });
        }
        if fo.get("audio_deep_hash").is_none()
            && fo.get("audio_deep_moments").is_none()
            && !batch_ids.iter().any(|b| b == "B46_audio_deep")
        {
            gaps.push(Gap {
                code: "need_audio_deep".into(),
                severity: "mid".into(),
                detail: "D03 audio deep digests: B46_audio_deep".into(),
            });
        }

    // --- Dense packs B47–B80: open task_gap_map codes so brain schedules them ---
        let has_batch = |id: &str| batch_ids.iter().any(|b| b == id);
        let missing = |keys: &[&str]| keys.iter().all(|k| fo.get(*k).is_none());

        // R100→B high-value promote (nav protocol / intl locale / rtc surface)
        if missing(&[
            "nav_next_hop_protocol",
            "r100_promote_algo",
            "intl_locale_max",
            "rtc_can_trickle",
        ]) && !has_batch("B80_r100_high_value")
        {
            gaps.push(Gap {
                code: "need_r100_high_value".into(),
                severity: "high".into(),
                detail: "material_hedge: B80_r100_high_value R100-promoted commercial fields"
                    .into(),
            });
        }

        if missing(&["api_flags_hit", "api_flags_hash", "api_probe_hit"])
            && !has_batch("B47_api_flags_detail")
        {
            gaps.push(Gap {
                code: "need_api_flags_detail".into(),
                severity: "high".into(),
                detail: "census: B47_api_flags_detail API claim-obs".into(),
            });
        }
        if missing(&["css_supports_ok_n", "css_supports_hash"])
            && !has_batch("B48_css_supports_detail")
        {
            gaps.push(Gap {
                code: "need_css_supports_detail".into(),
                severity: "mid".into(),
                detail: "census: B48_css_supports_detail".into(),
            });
        }
        if missing(&["css_props_resolved_n", "css_props_hash"])
            && !has_batch("B49_css_props_detail")
        {
            gaps.push(Gap {
                code: "need_css_props_detail".into(),
                severity: "mid".into(),
                detail: "census: B49_css_props_detail".into(),
            });
        }
        if missing(&["font_hit_n", "font_matrix_hash"]) && !has_batch("B50_font_matrix_detail")
        {
            gaps.push(Gap {
                code: "need_font_matrix_detail".into(),
                severity: "high".into(),
                detail: "census: B50_font_matrix_detail OS font family".into(),
            });
        }
        if missing(&["mq_true_n", "mq_hash"]) && !has_batch("B51_mq_matrix_detail") {
            gaps.push(Gap {
                code: "need_mq_matrix_detail".into(),
                severity: "mid".into(),
                detail: "census: B51_mq_matrix_detail".into(),
            });
        }
        if missing(&["nav_user_agent_len", "nav_plugins_n", "nav_gpu"])
            && !has_batch("B52_navigator_deep")
        {
            gaps.push(Gap {
                code: "need_navigator_deep".into(),
                severity: "high".into(),
                detail: "browser_kernel: B52_navigator_deep".into(),
            });
        }
        if missing(&["window_own_n", "window_keys_hash"]) && !has_batch("B53_window_keys_deep")
        {
            gaps.push(Gap {
                code: "need_window_keys_deep".into(),
                severity: "mid".into(),
                detail: "browser_kernel: B53_window_keys_deep".into(),
            });
        }
        if missing(&["plugins_list_n", "plugins_hash", "mimes_n"])
            && !has_batch("B54_plugin_mime_deep")
        {
            gaps.push(Gap {
                code: "need_plugin_mime_deep".into(),
                severity: "mid".into(),
                detail: "browser_kernel: B54_plugin_mime_deep".into(),
            });
        }
        if missing(&["ice_cand_n", "ice_hash", "rtc_peer"])
            && !has_batch("B55_webrtc_ice_deep")
        {
            gaps.push(Gap {
                code: "need_webrtc_ice_deep".into(),
                severity: "high".into(),
                detail: "network: B55_webrtc_ice_deep".into(),
            });
        }
        if missing(&["webrtc_stats_n", "webrtc_stats_hash", "webrtc_audio_packets"])
            && !has_batch("B56_webrtc_stats")
        {
            gaps.push(Gap {
                code: "need_webrtc_stats".into(),
                severity: "high".into(),
                detail: "network: B56_webrtc_stats getStats materials".into(),
            });
        }
        if missing(&["canvas_emoji_hash", "path2d_ok", "canvas_data_hash"])
            && !has_batch("B57_canvas_emoji_path")
        {
            gaps.push(Gap {
                code: "need_canvas_emoji_path".into(),
                severity: "mid".into(),
                detail: "material_hedge: B57_canvas_emoji_path".into(),
            });
        }
        if missing(&["text_width", "text_actual_bbox"]) && !has_batch("B58_canvas_text_metrics")
        {
            gaps.push(Gap {
                code: "need_canvas_text_metrics".into(),
                severity: "mid".into(),
                detail: "material_hedge: B58_canvas_text_metrics".into(),
            });
        }
        if missing(&["glp_MAX_TEX", "gl_ext_full_n", "glp_VENDOR"])
            && !has_batch("B59_webgl_params_full")
        {
            gaps.push(Gap {
                code: "need_webgl_params_full".into(),
                severity: "high".into(),
                detail: "gpu_physical: B59_webgl_params_full".into(),
            });
        }
        if missing(&["gl_ext_full_hash"]) && !has_batch("B60_webgl_extensions_full") {
            // still open when full params not yet collected either
            if !has_batch("B59_webgl_params_full") {
                gaps.push(Gap {
                    code: "need_webgl_params_full".into(),
                    severity: "high".into(),
                    detail: "gpu_physical: B59/B60 webgl extensions".into(),
                });
            }
        }
        if missing(&["audio_worklet", "offline_sr", "audio_ctx_ctor"])
            && !has_batch("B61_audio_worklet")
            && !has_batch("B62_offline_audio_moments")
        {
            gaps.push(Gap {
                code: "need_audio_worklet_offline".into(),
                severity: "mid".into(),
                detail: "material_hedge: B61/B62 audio worklet+offline".into(),
            });
        }
        if missing(&["intl_locale", "intl_tz", "tz_dst"])
            && !has_batch("B63_intl_full")
            && !has_batch("B64_timezone_deep")
        {
            gaps.push(Gap {
                code: "need_intl_timezone_deep".into(),
                severity: "mid".into(),
                detail: "os: B63_intl_full / B64_timezone_deep".into(),
            });
        }
        if missing(&["ua_ch_architecture", "ua_ch_bitness", "ua_ch_form_factors"])
            && !has_batch("B65_client_hints_full")
            && !has_batch("B66_sec_ch_headers")
        {
            gaps.push(Gap {
                code: "need_client_hints_full".into(),
                severity: "high".into(),
                detail: "browser_kernel: B65/B66 client hints full".into(),
            });
        }
        if missing(&["sw_reg_n", "caches_api"])
            && !has_batch("B67_service_worker_deep")
            && !has_batch("B68_cache_storage_deep")
        {
            gaps.push(Gap {
                code: "need_sw_cache_deep".into(),
                severity: "mid".into(),
                detail: "browser_kernel: B67/B68 SW+cache".into(),
            });
        }
        if missing(&["bluetooth", "usb", "hid", "payment_request"])
            && !has_batch("B69_bluetooth_usb")
            && !has_batch("B70_payment_credential")
        {
            gaps.push(Gap {
                code: "need_peripheral_payment".into(),
                severity: "low".into(),
                detail: "peripheral: B69/B70 bluetooth/usb/payment".into(),
            });
        }
        if missing(&["wake_lock", "idle_detector", "keyboard_layout_n"])
            && !has_batch("B71_idle_wake_lock")
            && !has_batch("B72_keyboard_layout")
        {
            gaps.push(Gap {
                code: "need_idle_keyboard".into(),
                severity: "low".into(),
                detail: "mobile/browser: B71/B72 idle+keyboard".into(),
            });
        }
        if missing(&["pointer_fine", "hover_hover", "vv_width"])
            && !has_batch("B73_pointer_capabilities")
            && !has_batch("B74_visual_viewport")
        {
            gaps.push(Gap {
                code: "need_pointer_viewport".into(),
                severity: "mid".into(),
                detail: "rpa/os: B73/B74 pointer+visualViewport".into(),
            });
        }
        if missing(&["perf_entries_n", "perf_entry_types", "js_heap_used"])
            && !has_batch("B75_performance_entries")
        {
            gaps.push(Gap {
                code: "need_performance_entries".into(),
                severity: "mid".into(),
                detail: "browser_kernel: B75_performance_entries".into(),
            });
        }
        if missing(&["math_hash", "wasm_present", "wasm_validate_empty"])
            && !has_batch("B76_math_wasm_deep")
        {
            gaps.push(Gap {
                code: "need_math_wasm_deep".into(),
                severity: "mid".into(),
                detail: "material_hedge: B76_math_wasm_deep".into(),
            });
        }
        if missing(&["is_top", "worker_ctor", "cross_origin_isolated", "frame_element"])
            && !has_batch("B77_worker_env_deep")
            && !has_batch("B78_iframe_env_deep")
            && !has_batch("B79_cross_origin_isolation")
        {
            gaps.push(Gap {
                code: "need_worker_iframe_deep".into(),
                severity: "high".into(),
                detail: "sandbox_xsrc: B77/B78/B79 cross-context".into(),
            });
        }
        // --- Async competitive follow-ups (must complete for normal-path authenticity) ---
        if missing(&[
            "ua_ch_architecture",
            "ua_ch_platform_version",
            "ua_ch_high_entropy_ok",
        ]) && !has_batch("B65_client_hints_full")
        {
            gaps.push(Gap {
                code: "need_ua_ch_high_entropy".into(),
                severity: "high".into(),
                detail: "async UA-CH getHighEntropyValues: B65_client_hints_full".into(),
            });
        }
        if missing(&["storage_quota_bytes", "storage_estimate_ok", "storage_usage_bytes"])
            && !has_batch("B27_storage_privacy")
        {
            gaps.push(Gap {
                code: "need_storage_estimate_async".into(),
                severity: "high".into(),
                detail: "async storage.estimate: B27_storage_privacy".into(),
            });
        }
        if missing(&[
            "webgpu_adapter_ok",
            "webgpu_features_hash",
            "webgpu_adapter_vendor",
        ]) && !has_batch("B18_webgpu")
        {
            gaps.push(Gap {
                code: "need_webgpu_adapter_async".into(),
                severity: "high".into(),
                detail: "async WebGPU requestAdapter: B18_webgpu".into(),
            });
        }
        if missing(&["speech_voices_hash", "speech_voices_n", "speech_voices_async_ok"])
            && !has_batch("B44_speech_deep")
        {
            gaps.push(Gap {
                code: "need_speech_voices_async".into(),
                severity: "mid".into(),
                detail: "async speech voices second tick: B44_speech_deep".into(),
            });
        }
        if missing(&[
            "media_capabilities_hash",
            "media_capabilities_supported_n",
            "codec_support_hash",
        ]) && !has_batch("B19_eme_media")
        {
            gaps.push(Gap {
                code: "need_media_capabilities_async".into(),
                severity: "mid".into(),
                detail: "async MediaCapabilities.decodingInfo: B19_eme_media".into(),
            });
        }

        // Gateway S0 density: thin gateway materials
        let gw_n = fo
            .get("gateway_material_count")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        if gw_n < 50.0
            && fo.get("ja4").is_none()
            && !has_batch("B8_gateway")
            && !has_batch("B8_gateway_early")
        {
            gaps.push(Gap {
                code: "need_gateway_s0_dense".into(),
                severity: "high".into(),
                detail: "gateway S0 dense materials for FE claim-obs".into(),
            });
        }
    }

    
    let sev_rank = |s: &str| match s {
        "high" => 0,
        "mid" | "medium" => 1,
        "low" => 2,
        _ => 9,
    };
    let mut best: HashMap<String, Gap> = HashMap::new();
    for g in gaps {
        match best.get(&g.code) {
            Some(old) if sev_rank(&old.severity) <= sev_rank(&g.severity) => {}
            _ => {
                best.insert(g.code.clone(), g);
            }
        }
    }
    let mut out: Vec<Gap> = best.into_values().collect();
    // Deterministic: severity then gap code (HashMap iteration is unordered).
    out.sort_by(|a, b| {
        sev_rank(&a.severity)
            .cmp(&sev_rank(&b.severity))
            .then_with(|| a.code.cmp(&b.code))
    });
    Ok(out)
}

/// Static kick list for FE — no analyze required (short-visit SLA).
pub fn static_kick_plan() -> Result<Value, ContractError> {
    let cat = load_catalog()?;
    let packs: Vec<Value> = cat
        .static_packs()
        .iter()
        .map(|p| pack_json(p, None))
        .collect();
    let ids: Vec<String> = packs
        .iter()
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    // Hardware-safe stages: light may race; B2 / B10 / B7 each alone (never concurrent HW thrash).
    let groups = crate::brain_control::build_hw_safe_parallel_groups(&ids);
    Ok(json!({
        "version": "v5.2-complete-probe",
        "schedule": "static",
        "policy": "complete_probe_upload_analyze",
        "schedule_gate_forgeable_os_br": false,
        "packs": packs,
        "parallel_groups": groups,
        "static_pack_ids": ids,
        "kick_without_analyze": true,
        "notes": "static wave complete-probe: light race → B2 → B10 → B7 (hw-safe); never skip for forgeable OS/br; kick≠finish-gate",
        "doc": "v5-docs/architecture/b-batch-complete-probe-schedule.md",
    }))
}

/// Attach `dag_v2` metadata to an inline-injected route pack (b10_sla force,
/// EDH deepen). Packs from the catalog already carry it via `pack_json`.
pub fn ensure_pack_dag_v2(p: &mut Value) {
    if p.get("dag_v2").is_some() {
        return;
    }
    let Some(id) = p.get("pack_id").and_then(|v| v.as_str()) else {
        return;
    };
    let Ok(cat) = load_catalog() else {
        return;
    };
    let Some(def) = cat.resolve(id) else {
        return;
    };
    if let Some(o) = p.as_object_mut() {
        o.insert("dag_v2".into(), def.probe_dag_v2());
    }
}

fn pack_json(def: &PackDef, priority_override: Option<i64>) -> Value {
    // iss/design §9.3: attach the full DAG v2 metadata per pack so the FE
    // schedules by planes/depends_on/cost_class/engine_profiles/deadline_ms and
    // the sealed pack-set decision can explain "why was this field not executed".
    json!({
        "pack_id": def.pack_id,
        "priority": priority_override.unwrap_or(def.priority),
        "source": def.source,
        "layer": def.layer,
        "schedule": def.schedule,
        "batch_id": def.batch_id,
        "executable": def.executable,
        "hard_eligible": def.hard_eligible,
        "dag_v2": def.probe_dag_v2(),
    })
}

/// Build frontier route_plan from evidence gaps. Only emits missing packs.
// ── B2 info-gain pricing helpers (iss/74 strategy §5-B2) ────────────────
// effective_priority = base + boost + ig(axis), ig = axis confidence gap ×
// expected gap reduction ÷ cost (budget proxy). High-entropy curve packs
// (gpu/audio/cpu) outrank flat census/enumeration at equal base priority.

/// Pack → probe axis. B1 axis vocabulary: cpu/gpu/audio/census/network/peripheral
/// (general bucket for unclassified packs; they get no ig edge).
pub fn pack_axis(pid: &str) -> &'static str {
    let p = pid.to_ascii_lowercase();
    if p.starts_with("b10")
        || p.contains("gpu")
        || p.contains("webgl")
        || p.contains("webgpu")
        || p.contains("gl_")
        || p.contains("canvas")
        || p.contains("curve")
    {
        "gpu"
    } else if p.contains("audio") || p.contains("music") {
        "audio"
    } else if p.contains("cpu") || p.contains("wasm") || p.contains("fma") || p.contains("thermal") {
        "cpu"
    } else if p.contains("ja") || p.contains("webrtc") || p.contains("ws_")
        || p.contains("edge") || p.contains("gateway") || p.contains("ice")
    {
        "network"
    } else if p.contains("touch") || p.contains("pointer") || p.contains("wheel")
        || p.contains("peripheral") || p.contains("storage")
    {
        "peripheral"
    } else if p.contains("census") || p.contains("dense") || p.contains("env")
        || p.contains("system") || p.contains("hardware") || p.contains("boot")
    {
        "census"
    } else {
        "general"
    }
}

/// Material keys considered present-covered per axis (deterministic proxy;
/// richer than gap codes alone because coverage floors have no codes).
fn axis_material_keys(axis: &str) -> &'static [&'static str] {
    match axis {
        "gpu" => &[
            "hw_curve_webgl",
            "webgl_unmasked_renderer",
            "webgl2_support",
            "webgl_max_texture",
            "webgpu_compute_ok",
            "webgpu_f16_ok",
            "engine_canvas_farbling_delta",
            "atomic_contention_ok",
            "gpu_eu_ok",
        ],
        "audio" => &[
            "hw_curve_audio",
            "audio_deep_curve",
            "audio_known_lock_sum",
            "audio_channel_vs_copy_delta",
            "audio_silent_osc_unique_bins",
        ],
        "cpu" => &[
            "hw_curve_cpu",
            "hardware_concurrency",
            "wasm_bi_f32_mul_ns",
            "wasm_simd_f32x4_ns",
            "wasm_throughput_digest",
            "ws_fma_delta_curve",
            "ws_simd_timing_curve",
            "sab_dual_clock_ok",
        ],
        "network" => &["ja4", "webrtc_host_ip_hash", "webrtc_cand_types", "webrtc_rtt"],
        "peripheral" => &[
            "max_touch_points",
            "pointer_fine",
            "touch_event",
            "wheel_event",
            "storage_quota_grid_class",
        ],
        "census" => &[
            "fonts_present",
            "plugins_len",
            "mime_types_len",
            "canvas_w_base",
            "os_kernel_hint",
            "oscpu_raw",
            "window_keys_hash",
        ],
        _ => &[],
    }
}

/// Axis confidence gap ∈ [0,1]: 1 - present/total of that axis's material
/// keys. Empty axis list → 0.0 (no signal; never blocks).
pub fn axis_confidence_gap(evidence: &Value, axis: &str) -> f64 {
    let keys = axis_material_keys(axis);
    if keys.is_empty() {
        return 0.0;
    }
    let fo = evidence
        .get("fields")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let present = keys
        .iter()
        .filter(|k| {
            fo.get(**k).is_some_and(|v| match v {
                Value::Null => false,
                Value::Array(a) => !a.is_empty(),
                Value::String(s) => !s.is_empty(),
                _ => true,
            })
        })
        .count();
    let total = keys.len();
    let gap = 1.0 - present as f64 / total as f64;
    (gap * 1e4).round() / 1e4
}

/// B5 (iss/74 §5-B5): contradiction-triggered research unlock condition.
/// True when A5 claim-obs conflict edges, M1 realm coherence hard conflict,
/// or B4 xsrc-numeric divergence show cross-source / claim-obs incoherence —
/// the brain then actively unlocks B18/B92/B95 deepening instead of a
/// constant research-gate defer.
fn research_unlock_triggered(evidence: &Value) -> bool {
    let fo = evidence
        .get("fields")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    // A5 claim-obs rules are pure fields → graph; any conflict edge triggers.
    let graph = crate::claim_obs_graph::build_claim_obs_graph(&Value::Object(fo));
    if graph
        .get("n_conflict")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        > 0
    {
        return true;
    }
    // M1 realm coherence hard conflict.
    if let Some(v) = evidence.pointer(
        "/multi_source_consistency/structured_realm_diff/realm_coherence_verdict",
    ) {
        if v.as_str() == Some("conflict") {
            return true;
        }
    }
    // B4 xsrc numeric divergent slots (object map of {slot, verdict}).
    if let Some(slots) = evidence.pointer("/multi_source_consistency/xsrc_numeric/slots") {
        if let Some(m) = slots.as_object() {
            if m.values().any(|d| {
                d.get("verdict").and_then(|v| v.as_str()) == Some("divergent")
            }) {
                return true;
            }
        }
    }
    false
}

/// Budget proxy: relative cost tiers per pack (heavy silicon > mid > light).
fn pack_cost_factor(pid: &str, layer: &str) -> f64 {
    let heavy = matches!(pid, "B10_hw_curves" | "B17_hw_physical" | "B18_webgpu")
        || pid.starts_with("B10x_")
        || pid == "B42_thermal";
    let light = matches!(pid, "B0_bootstrap" | "B34_fonts" | "B35_plugins")
        || layer == "lite5"
        || layer == "b8";
    if heavy {
        3.0
    } else if light {
        0.8
    } else if layer == "mid4" || layer == "deep" {
        2.0
    } else {
        1.2
    }
}

/// Expected gap reduction from running the pack: curve/hardware packs carry
/// most of their axis's residual uncertainty; enumeration packs less.
fn expected_gap_reduction(pid: &str) -> f64 {
    let p = pid.to_ascii_lowercase();
    if p.starts_with("b10")
        || p.contains("curve")
        || p.contains("gpu")
        || p.contains("audio")
        || p.contains("wasm")
        || p.contains("thermal")
    {
        0.8
    } else if p.contains("census") || p.contains("dense") || p.starts_with("b34")
        || p.starts_with("b35") || p.starts_with("b90")
    {
        0.35
    } else {
        0.5
    }
}

/// B2 information gain for a pack: gap × reduction ÷ cost, scaled into the
/// priority domain (≤ ~3_000) — an ig edge, never a veto.
pub fn axis_info_gain(evidence: &Value, pid: &str, layer: &str) -> (i64, f64) {
    let axis = pack_axis(pid);
    let gap = axis_confidence_gap(evidence, axis);
    if gap <= 0.0 {
        return (0, gap);
    }
    let ig = gap * expected_gap_reduction(pid) / pack_cost_factor(pid, layer) * 2_000.0;
    (ig.round().clamp(0.0, 3_000.0) as i64, gap)
}

// ── B1 axis budgets (iss/74 §5-B1) ──────────────────────────────────────
// Per-axis dynamic budgets by device class replace the fixed band cap:
//   desktop {gpu 4, audio 3, cpu 3, network 3, peripheral 3, census 6}
//   mobile  {gpu 2, audio 2, cpu 2, network 2, peripheral 2, census 4}
//   headless{gpu 1, audio 1, cpu 2, network 2, peripheral 1, census 3}
// Census gets headroom for dense waves (2–3 parallel allowed); mutually
// exclusive resource axes (gpu) stay tight.

/// Device class from evidence fields: mobile < headless < desktop.
pub fn device_class(fields: &Map<String, Value>) -> &'static str {
    let form = fields
        .get("form_class")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let headless = fields
        .get("headless_likely")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || fields
            .get("webdriver")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    if headless {
        "headless"
    } else if form.contains("mobile") || form.contains("phone") || form.contains("tablet") {
        "mobile"
    } else {
        "desktop"
    }
}

/// Per-axis pack budget for a device class. `general` is unbounded.
pub fn axis_budget(class: &str) -> &'static [(&'static str, usize)] {
    match class {
        "mobile" => &[
            ("census", 4),
            ("gpu", 2),
            ("audio", 2),
            ("cpu", 2),
            ("network", 2),
            ("peripheral", 2),
        ],
        "headless" => &[
            ("census", 3),
            ("gpu", 1),
            ("audio", 1),
            ("cpu", 2),
            ("network", 2),
            ("peripheral", 1),
        ],
        _ => &[
            ("census", 6),
            ("gpu", 4),
            ("audio", 3),
            ("cpu", 3),
            ("network", 3),
            ("peripheral", 3),
        ],
    }
}

/// Parse the B1 axis-budget calibration payload
/// (`spec/axis_budget_calibration.json` → `{classes: {class: {axis: cap}}}`).
/// Returns per-class budgets keyed by device class; malformed input yields
/// `None` so callers fall back to the built-in defaults.
fn parse_axis_budget_calibration(raw: &Value) -> Option<Vec<(String, Vec<(String, usize)>)>> {
    let classes = raw.get("classes")?.as_object()?;
    let mut out = Vec::new();
    for (class, caps) in classes {
        let caps = caps.as_object()?;
        let mut row = Vec::new();
        for (axis, cap) in caps {
            row.push((axis.clone(), usize::try_from(cap.as_u64()?).ok()?));
        }
        out.push((class.clone(), row));
    }
    out.sort();
    Some(out)
}

/// B1 axis budgets resolved from the lab calibration SSOT
/// (`spec/axis_budget_calibration.json`). Lab recalibration only edits the
/// spec file — no code change needed. Missing file/class or malformed payload
/// falls back to the built-in `axis_budget` table. The OnceLock guarantees a
/// single load; per-class rows are leaked once (tiny, process-lifetime).
pub fn axis_budget_resolved(class: &str) -> &'static [(&'static str, usize)] {
    static CAL: OnceLock<Option<HashMap<&'static str, &'static [(&'static str, usize)]>>> =
        OnceLock::new();
    let table = CAL.get_or_init(|| {
        let spec_dir = crate::contracts::find_spec_dir();
        let path = spec_dir.join("axis_budget_calibration.json");
        let raw: serde_json::Value = match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(_) => return None,
            },
            Err(_) => return None,
        };
        let rows = parse_axis_budget_calibration(&raw)?;
        let mut map = HashMap::new();
        for (cls, budgets) in rows {
            let key: &'static str = Box::leak(cls.into_boxed_str());
            let row: &'static [(&'static str, usize)] = Box::leak(
                budgets
                    .into_iter()
                    .map(|(axis, cap)| {
                        (Box::leak(axis.into_boxed_str()) as &'static str, cap)
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            );
            map.insert(key, row);
        }
        Some(map)
    });
    if let Some(map) = table {
        if let Some(row) = map.get(class) {
            return row;
        }
    }
    axis_budget(class)
}

/// Trim over-budget axes (B1): drop lowest effective_priority packs on the
/// axis first; only the mandatory core anchors (B10/B0/B2/B3/B8 family,
/// verify_rand R-lane) are never trimmed. Hard-eligible deepen packs are
/// still subject to the per-axis bound — they re-enter on the next tick.
/// Records per-axis budget usage.
fn trim_to_axis_budget(
    packs: &mut Vec<Value>,
    class: &str,
    notes: &mut Vec<String>,
) -> Value {
    let budget = axis_budget_resolved(class);
    let mut counts: HashMap<&'static str, usize> = HashMap::new();
    for p in packs.iter() {
        let pid = p.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
        *counts.entry(pack_axis(pid)).or_insert(0) += 1;
    }
    let mut trimmed: Vec<String> = Vec::new();
    for (axis, cap) in budget {
        let mut n = *counts.get(*axis).unwrap_or(&0);
        if n <= *cap {
            continue;
        }
        // Drop lowest-effective_priority, non-anchor packs of this axis.
        // Collect (index, ep); keep lowest-ep first, then remove the chosen
        // indices in DESCENDING order so earlier indices stay valid.
        let mut cands: Vec<(usize, i64, String)> = packs
            .iter()
            .enumerate()
            .filter(|(_, p)| pack_axis(p.get("pack_id").and_then(|v| v.as_str()).unwrap_or("")) == *axis)
            .filter(|(_, p)| {
                let pid = p.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
                !matches!(
                    pid,
                    "B10_hw_curves"
                        | "B0_bootstrap"
                        | "B2_hardware"
                        | "B3_system"
                        | "B8_gateway_early"
                ) && !crate::brain_control::is_verify_rand_pack(pid)
            })
            .map(|(i, p)| {
                let pid = p.get("pack_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let ep = p
                    .get("effective_priority")
                    .or_else(|| p.get("priority"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                (i, ep, pid)
            })
            .collect();
        cands.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)));
        let to_drop = n.saturating_sub(*cap).min(cands.len());
        let mut drop_idxs: Vec<usize> = cands
            .iter()
            .take(to_drop)
            .map(|(i, _, pid)| {
                trimmed.push(pid.clone());
                *i
            })
            .collect();
        drop_idxs.sort_unstable_by(|a, b| b.cmp(a)); // descending keeps indices valid
        for i in drop_idxs {
            packs.remove(i);
            n -= 1;
        }
    }
    // Usage snapshot after trimming.
    let mut usage = Map::new();
    let mut per_axis: HashMap<&'static str, usize> = HashMap::new();
    let mut general = 0usize;
    for p in packs.iter() {
        let pid = p.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
        let ax = pack_axis(pid);
        if ax == "general" {
            general += 1;
        } else {
            *per_axis.entry(ax).or_insert(0) += 1;
        }
    }
    for (axis, cap) in budget {
        usage.insert(
            (*axis).to_string(),
            json!({
                "budget": cap,
                "used": per_axis.get(*axis).copied().unwrap_or(0),
            }),
        );
    }
    usage.insert("general".into(), json!({"budget": null, "used": general}));
    if !trimmed.is_empty() {
        notes.push(format!(
            "axis_budget: trimmed over-budget {class} packs (lowest priority first) {:?}",
            trimmed
        ));
    }
    json!(usage)
}

/// Map a gap code to its probe axis (for B3 open-gap convergence gating).
fn gap_code_axis(code: &str) -> &'static str {
    let c = code.to_ascii_lowercase();
    if c.contains("gpu") || c.contains("webgl") {
        "gpu"
    } else if c.contains("audio") {
        "audio"
    } else if c.contains("wasm") || c.contains("cpu") || c.contains("fma") {
        "cpu"
    } else if c.contains("ja") || c.contains("webrtc") || c.contains("edge") || c.contains("gateway")
    {
        "network"
    } else if c.contains("touch") || c.contains("pointer") || c.contains("storage") {
        "peripheral"
    } else if c.contains("census") || c.contains("env") || c.contains("os_") || c.contains("system")
    {
        "census"
    } else {
        "general"
    }
}

/// B3 conf-sufficiency signal: per-axis confidence convergence ∧ cross-source
/// consistency (or qualified missing evidence) ∧ budget headroom.
/// `xsrc_ok` — multi-source consistency verdicts: "ok" | "qualified" (no
/// cross-source material to judge) | "conflict".
pub fn conf_sufficiency(
    evidence: &Value,
    gaps: &[Gap],
    xsrc: &TruthResult,
) -> Value {
    let mut per_axis = Map::new();
    let mut all_converged = true;
    let mut open_axes: Vec<String> = Vec::new();
    for axis in ["gpu", "audio", "cpu", "network", "peripheral", "census"] {
        let gap = axis_confidence_gap(evidence, axis);
        let open_gap = gaps
            .iter()
            .any(|g| gap_code_axis(&g.code) == axis);
        // Convergence is gap-driven: residual open-gap codes on an already
        // low-gap axis stay diagnostic (notes) but do not veto convergence —
        // otherwise partial fixtures can never reach sufficiency.
        let converged = gap <= 0.5;
        if !converged {
            all_converged = false;
            if open_gap {
                open_axes.push(axis.to_string());
            }
        }
        per_axis.insert(
            axis.to_string(),
            json!({
                "confidence_gap": gap,
                "open_gap": open_gap,
                "converged": converged,
            }),
        );
    }
    // Cross-source consistency: ok, or qualified when nothing to compare.
    let xsrc_state = if xsrc.xsrc_status == "ok" {
        "ok"
    } else if has_cross_source_material(evidence) {
        "conflict"
    } else {
        "qualified"
    };
    let xsrc_consistent = xsrc_state == "ok" || xsrc_state == "qualified";
    // Budget headroom (qualified when not reported).
    let budget_ok = match (
        evidence.get("budget_ms").and_then(|v| v.as_f64()),
        evidence.get("spent_ms").and_then(|v| v.as_f64()),
    ) {
        (Some(b), Some(s)) => s < b,
        _ => true,
    };
    let sufficient = all_converged && xsrc_consistent && budget_ok;
    json!({
        "algo": "conf_sufficiency_v1",
        "verdict": if sufficient { "sufficient" } else { "insufficient" },
        "per_axis": per_axis,
        "xsrc": xsrc_state,
        "xsrc_consistent": xsrc_consistent,
        "budget_ok": budget_ok,
        "open_gap_axes": open_axes,
    })
}

/// True when evidence carries nest/cross-source material to compare
/// (worker/iframe/sandbox sources with any field).
fn has_cross_source_material(evidence: &Value) -> bool {
    evidence
        .get("sources")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .any(|s| matches!(s.as_str(), Some("worker" | "iframe" | "sandbox")))
        })
        .unwrap_or(false)
        || evidence
            .get("fields")
            .and_then(|f| f.get("sandbox_sources_received"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            > 0
}

pub fn build_frontier(
    evidence: &Value,
    soft_v2_ready: bool,
    allow_amplify: Option<bool>,
    max_packs: usize,
) -> Result<FrontierPlan, ContractError> {
    let cat = load_catalog()?;
    let sid = evidence
        .get("session_id")
        .and_then(|v| v.as_str())
        .or_else(|| {
            evidence
                .get("fields")
                .and_then(|f| f.get("session_id"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or("unknown")
        .to_string();
    let truth = evaluate_xsrc(evidence, None)?;
    let gaps = scan_gaps(evidence, Some(&truth))?;
    let allow_amplify = allow_amplify.unwrap_or(soft_v2_ready);
    let present = present_batches(evidence);

    let plan_version = evidence
        .get("plan_version")
        .and_then(|v| v.as_u64())
        .or_else(|| evidence.get("analysis_rev").and_then(|v| v.as_u64()))
        .unwrap_or(0)
        + 1;

    let mut packs: Vec<Value> = Vec::new();
    let mut notes: Vec<String> = Vec::new();

    fn try_add(
        packs: &mut Vec<Value>,
        notes: &mut Vec<String>,
        present: &HashSet<String>,
        max_packs: usize,
        def: &PackDef,
        priority: Option<i64>,
    ) {
        try_add_force(packs, notes, present, max_packs, def, priority, false);
    }

    /// force=true re-schedules pack even if batch already present (incomplete residual fields).
    fn try_add_force(
        packs: &mut Vec<Value>,
        notes: &mut Vec<String>,
        present: &HashSet<String>,
        max_packs: usize,
        def: &PackDef,
        priority: Option<i64>,
        force: bool,
    ) {
        if packs
            .iter()
            .any(|p| p.get("pack_id").and_then(|v| v.as_str()) == Some(def.pack_id.as_str()))
        {
            return;
        }
        if !force && (present.contains(&def.batch_id) || present.contains(&def.pack_id)) {
            return;
        }
        if packs.len() >= max_packs {
            return;
        }
        if !def.executable {
            notes.push(format!("skip non-executable {}", def.pack_id));
            return;
        }
        let mut pj = pack_json(def, priority);
        if force {
            if let Some(obj) = pj.as_object_mut() {
                obj.insert("force_recollect".into(), json!(true));
                obj.insert(
                    "reason".into(),
                    json!("stack_residual_missing_recollect"),
                );
            }
        }
        packs.push(pj);
    }

    let resolve = |id: &str| -> Option<&PackDef> { cat.resolve(id) };
    let codes: HashSet<String> = gaps.iter().map(|g| g.code.clone()).collect();

    let skip_session = should_skip_session_probe(evidence);
    // B5 deep-vs-broad research unlock (iss/74 §5-B5): contradiction-triggered.
    // Claim-obs conflict edges, M1 realm coherence conflict, or B4 xsrc numeric
    // divergence unlock B18/B92/B95 deepening (no constant defer). Runs BEFORE
    // the primary task_gap_map fill so high-entropy deepening preempts
    // breadth; B94 stays behind the dual-KPI gate unless separately justified.
    let b5_research_unlock = |packs: &mut Vec<Value>, notes: &mut Vec<String>| {
        if !research_unlock_triggered(evidence) {
            for (code, pid) in [
                ("need_webgpu_atomic_contention", "B92_webgpu_atomic_contention"),
                ("need_sab_dual_clock", "B94_sab_dual_clock_differential"),
                ("need_gpu_eu_timing", "B95_gpu_eu_timing"),
            ] {
                if codes.contains(code) {
                    notes.push(format!(
                        "research_gate: {pid} deferred pending dual-KPI or contradiction trigger"
                    ));
                }
            }
            return;
        }
        let mut unlocked_n = 0usize;
        for (code, pid, prio) in [
            ("need_webgpu_atomic_contention", "B92_webgpu_atomic_contention", 96),
            ("need_gpu_eu_timing", "B95_gpu_eu_timing", 95),
            ("need_webgpu_deep", "B18_webgpu", 91),
        ] {
            if !codes.contains(code) {
                continue;
            }
            if let Some(d) = resolve(pid) {
                let before = packs.len();
                try_add(packs, notes, &present, max_packs, d, Some(prio));
                if packs.len() > before {
                    if let Some(last) = packs.last_mut() {
                        if let Some(obj) = last.as_object_mut() {
                            obj.insert("reason".into(), json!("research_unlock:contradiction"));
                            obj.insert("pack_lane".into(), json!("deepen"));
                            obj.insert("hard_eligible".into(), json!(true));
                        }
                    }
                    unlocked_n += 1;
                }
            }
        }
        if unlocked_n > 0 {
            notes.push(format!(
                "b5_research_unlock: contradiction-triggered unlock ×{unlocked_n} (B18/B92/B95)"
            ));
        }
    };
    if skip_session {
        notes.push(
            "session cool-down: skip_session_probe — only page/rpa packs (B11); session materials cooled"
                .into(),
        );
        // Page rpa must re-kick even when B11 batch already present (new page / no behavior).
        let fields = evidence
            .get("fields")
            .cloned()
            .unwrap_or(Value::Object(Map::new()));
        let fo = fields.as_object().cloned().unwrap_or_default();
        let has_behavior = fo
            .get("behavior_early_bound")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || fo
                .get("behavior_count")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
                > 0.0
            || fo
                .get("behavior_events")
                .and_then(|v| v.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false);
        // Force recollect B11 only when behavior still missing / explicit rpa gap.
        // iss/65: page_id.is_some() alone always-true → infinite B11 force storm.
        let force_b11 = !has_behavior
            || codes.contains("need_rpa_bind")
            || codes.contains("need_rpa_events");
        if let Some(d) = resolve("B11_interaction") {
            if force_b11 {
                try_add_force(
                    &mut packs,
                    &mut notes,
                    &present,
                    max_packs,
                    d,
                    Some(100),
                    // force only when batch present but still no behavior
                    present.contains("B11_interaction") && !has_behavior,
                );
                notes.push(
                    "cool-down force B11_interaction for page rpa (new page / missing behavior)"
                        .into(),
                );
            } else {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(100));
            }
        }
        // Still allow conflict if explicit xsrc conflict
        if codes.contains("xsrc_conflict") {
            if let Some(d) = resolve("B1_conflict") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(99));
            }
        }
    } else if let Ok(tmap) = load_task_gap_map() {
        // ACTIVE: task_gap_map is **primary** dynamic routing (spec status ACTIVE).
        // Soft-gated dynamic mid/deep never scheduled when soft_v2_ready=false (J3).
        // B5: contradiction-unlocked research deepens preempt the breadth fill.
        b5_research_unlock(&mut packs, &mut notes);
        let mut code_refs: Vec<&str> = codes.iter().map(|s| s.as_str()).collect();
        code_refs.sort(); // deterministic gap → pack expansion
        let mapped = tmap.packs_for_gap_codes(code_refs, soft_v2_ready);
        for pid in mapped {
            if let Some(d) = resolve(pid) {
                if d.schedule == "dynamic"
                    && (d.requires_soft_v2 || d.layer == "mid4" || d.layer == "deep")
                    && !soft_v2_ready
                {
                    notes.push(format!(
                        "task_gap_map skip soft-gated dynamic {} (soft_v2 not ready)",
                        d.pack_id
                    ));
                    continue;
                }
                // Boost priority so task-mapped packs win over opportunistic dense.
                let prio = Some(d.priority.max(70) + 15);
                try_add(&mut packs, &mut notes, &present, max_packs, d, prio);
                notes.push(format!("task_gap_map primary: {}", d.pack_id));
            }
        }
        if !codes.is_empty() {
            notes.push(format!(
                "task_gap_map ACTIVE: {} gap codes → primary pack candidates",
                codes.len()
            ));
        }
    }

    // Legacy gap-driven pack scheduling (session materials) — skipped on cool-down.
    if !skip_session {
        // GPU label without residual/stack: force B2 + B10 re-collect for authenticity.
        // Brake: any landed residual_mean/curve OR commercial final → never infinite recollect
        // (GL context storms previously retriggered residual_missing forever).
        // FE multipath PATH_CAP≈2–4 under GL governor; single ok path is enough residual material.
        let has_residual_mean = evidence
            .pointer("/fields/residual_mean")
            .and_then(|v| v.as_f64())
            .is_some()
            || evidence
                .pointer("/fields/hw_curve_webgl")
                .and_then(|v| v.as_array())
                .is_some_and(|a| a.len() >= 4)
            || evidence
                .pointer("/fields/residual_paths")
                .and_then(|v| v.as_array())
                .is_some_and(|a| {
                    a.iter().any(|p| {
                        p.get("ok").and_then(|v| v.as_bool()).unwrap_or(false)
                            && p.get("curve")
                                .and_then(|v| v.as_array())
                                .is_some_and(|c| c.len() >= 4)
                    })
                });
        // Honest residual already on session → stop force-recollect storms (probe GL thrash).
        let residual_brake = has_residual_mean
            || (evidence
                .get("commercial_identity_final")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                && (evidence
                    .pointer("/device/digest_path")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| s.contains("real_curves"))
                    || present.contains("B10_hw_curves")
                    || present.contains("mid.curves")))
            || (present.contains("B10_hw_curves") && has_residual_mean)
            || present.iter().any(|id| id.starts_with("B10x_") && has_residual_mean);
        if codes.contains("stack_residual_missing") && !residual_brake {
            let force = present.contains("B2_hardware")
                || present.contains("B10_hw_curves")
                || present.contains("lite.gpu")
                || present.contains("mid.curves");
            // Allow slight budget overshoot so residual recollect is not starved by H0x packs
            let room = max_packs.saturating_add(4);
            if let Some(d) = resolve("B2_hardware") {
                try_add_force(
                    &mut packs,
                    &mut notes,
                    &present,
                    room,
                    d,
                    Some(96),
                    force,
                );
            }
            if let Some(d) = resolve("B10_hw_curves") {
                try_add_force(
                    &mut packs,
                    &mut notes,
                    &present,
                    room,
                    d,
                    Some(95),
                    force,
                );
            }
            notes.push(
                "stack_residual_missing: schedule B2 residual + B10 curves (force recollect if batch present without residual)"
                    .into(),
            );
        } else if residual_brake && codes.contains("stack_residual_missing") {
            notes.push(
                "stack_residual_missing brake: residual_mean/curve already landed — skip force recollect storm (GL governor)"
                    .into(),
            );
        }

        // Class-K texture caps gap: silicon landed without gl_max_texture_size.
        // Prefer B10 (caps co-land with curves) + B2 fallback; do not mint CIF on cores-only.
        if codes.contains("need_gl_texture_caps_for_class_k") {
            let room = max_packs.saturating_add(2);
            let force_caps = present.contains("B10_hw_curves")
                || present.contains("mid.curves")
                || present.contains("B2_hardware")
                || present.contains("lite.gpu");
            if let Some(d) = resolve("B10_hw_curves") {
                try_add_force(
                    &mut packs,
                    &mut notes,
                    &present,
                    room,
                    d,
                    Some(97),
                    force_caps,
                );
            }
            if let Some(d) = resolve("B2_hardware") {
                try_add_force(
                    &mut packs,
                    &mut notes,
                    &present,
                    room,
                    d,
                    Some(96),
                    force_caps,
                );
            }
            notes.push(
                "need_gl_texture_caps_for_class_k: force B10/B2 until texture caps land"
                    .into(),
            );
        }

        if codes.contains("xsrc_conflict") {
            if let Some(d) = resolve("B1_conflict") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(100));
                notes.push("priority: conflict reconcile via B1_conflict".into());
            }
            if let Some(d) = resolve("B12_anti_camouflage") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(99));
            }
        }

        if codes.contains("missing_gateway_or_cf") {
            if let Some(d) = resolve("B8_gateway_early") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(98));
            }
            if let Some(d) = resolve("edge.cf") {
                if !present.contains("cloudflare") {
                    try_add(&mut packs, &mut notes, &present, max_packs, d, Some(97));
                }
            }
        }

        if codes.contains("thin_main") || codes.contains("lite5_incomplete") {
            for id in [
                "B0_bootstrap",
                "B12_anti_camouflage",
                "B1_conflict",
                "B2_hardware",
                "B3_system",
            ] {
                if let Some(d) = resolve(id) {
                    try_add(&mut packs, &mut notes, &present, max_packs, d, None);
                }
            }
        }
        if codes.contains("missing_automation") {
            if let Some(d) = resolve("B1_conflict") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, None);
            }
        }
        if codes.contains("missing_gpu") || codes.contains("missing_link_anchors") {
            if let Some(d) = resolve("B2_hardware") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, None);
            }
            if let Some(d) = resolve("B3_system") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, None);
            }
        }
        if codes.contains("missing_sandbox")
            || codes.contains("sandbox_all_empty")
            || codes.contains("sandbox_under_two_kinds")
            || codes.contains("need_br_kernel_consistency")
        {
            if let Some(d) = resolve("B7_sandbox") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, None);
            }
        }

        if allow_amplify && soft_v2_ready {
            if codes.contains("mid_absent") {
                for id in ["B16_fast_signals", "B13_authorized", "B15_cross_curves"] {
                    if let Some(d) = resolve(id) {
                        if d.schedule == "dynamic" {
                            try_add(&mut packs, &mut notes, &present, max_packs, d, None);
                        }
                    }
                }
                if packs
                    .iter()
                    .any(|p| p.get("layer").and_then(|v| v.as_str()) == Some("mid4"))
                {
                    notes.push("amplify mid allowed (soft_v2_ready)".into());
                }
            }
        } else if codes.contains("mid_absent") {
            notes.push("mid withheld: soft_v2 not ready or amplify disabled".into());
        }
    }

    // Coverage fill-pass — skipped when session cool-down (only rpa path).
    if !skip_session {
        let inject = evidence
            .pointer("/meta/inject_path")
            .or_else(|| evidence.pointer("/fields/inject_path"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let sources = sources_from(evidence);
        for p in cat.static_packs() {
            if p.pack_id == "edge.cf" || p.source == "cloudflare" {
                if inject != "cf_worker" && !sources.contains("cloudflare") {
                    continue;
                }
            }
            try_add(&mut packs, &mut notes, &present, max_packs, p, None);
        }
        if !present.contains("B8_gateway") && !present.contains("B8_gateway_early") {
            if let Some(d) = resolve("B8_gateway_early") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(98));
            }
        }
        // Soft mid dynamic — schedule early so residual hard H0x cannot starve budget.
        if allow_amplify && soft_v2_ready {
            for id in ["B16_fast_signals", "B13_authorized", "B15_cross_curves"] {
                if present.contains(id) {
                    continue;
                }
                if let Some(d) = resolve(id) {
                    if d.schedule == "dynamic" {
                        // Prefer mid over residual hard when soft ready (short-visit mid path).
                        try_add(
                            &mut packs,
                            &mut notes,
                            &present,
                            max_packs.saturating_add(4),
                            d,
                            Some(d.priority),
                        );
                        notes.push(format!("soft_mid_amplify: {id}"));
                    }
                }
            }
        }

        // Complete-probe static anchors: never skip because forgeable OS/br already present.
        // Order mirrors catalog static_kick_order / b-batch-complete-probe-schedule.md.
        {
            const COMPLETE_STATIC: &[&str] = &[
                "B11_interaction",
                "B0_bootstrap",
                "B12_anti_camouflage",
                "B1_conflict",
                "B3_system",
                "B2_hardware",
                "B10_hw_curves",
                "B7_sandbox",
                "B8_gateway_early",
            ];
            let room = max_packs.saturating_add(8);
            for id in COMPLETE_STATIC {
                if present.contains(*id) {
                    continue;
                }
                if packs
                    .iter()
                    .any(|p| p.get("pack_id").and_then(|v| v.as_str()) == Some(*id))
                {
                    continue;
                }
                if let Some(d) = resolve(id) {
                    let pri = match *id {
                        "B10_hw_curves" => Some(91),
                        "B2_hardware" => Some(93),
                        "B8_gateway_early" => Some(98),
                        "B11_interaction" => Some(90),
                        "B0_bootstrap" => Some(96),
                        "B12_anti_camouflage" => Some(94),
                        "B1_conflict" => Some(95),
                        "B3_system" => Some(92),
                        "B7_sandbox" => Some(88),
                        _ => None,
                    };
                    try_add(&mut packs, &mut notes, &present, room, d, pri);
                }
            }
            notes.push(
                "complete_probe_static: schedule missing static anchors (no OS/br skip)"
                    .into(),
            );
        }

        // Always schedule B10 when missing — commercial eligibility depends on curve anchors.
        if !present.contains("B10_hw_curves")
            && !packs
                .iter()
                .any(|p| p.get("pack_id").and_then(|v| v.as_str()) == Some("B10_hw_curves"))
        {
            if let Some(d) = resolve("B10_hw_curves") {
                packs.push(pack_json(d, Some(91)));
                notes.push("force_schedule: B10_hw_curves hard commercial anchor".into());
            }
        }

        // Fields map for brain floors (bio/CDP).
        let fo_brain = evidence
            .get("fields")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();

        // Residual deepen (B10x): complete-probe — contract incomplete only.
        // Forgeable OS/br/engine NEVER skip or force residual packs.
        // See b-batch-complete-probe-schedule.md
        {
            let present_vec: Vec<String> = present.iter().cloned().collect();
            let band_hint = evidence
                .pointer("/policy_band/band")
                .or_else(|| evidence.get("policy_band"))
                .and_then(|v| v.as_str());
            let meas = crate::engine_surface::residual_measurement_status(
                evidence.get("fields").unwrap_or(&Value::Null),
                &present_vec,
                band_hint,
            );
            let mp = meas.get("match_progress");
            notes.push(format!(
                "residual_measurement: complete={} score={} matched={}/{} missing=[{}] worth_deepen={} reason={} ticks={}/{} cluster={}/{} policy=complete_probe",
                meas.get("measurement_complete").and_then(|v| v.as_bool()).unwrap_or(false),
                meas.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0),
                mp.and_then(|v| v.get("matched_n")).and_then(|v| v.as_u64()).unwrap_or(0),
                mp.and_then(|v| v.get("contract_n")).and_then(|v| v.as_u64()).unwrap_or(5),
                meas.get("missing").and_then(|v| v.as_array()).map(|a| {
                    a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(",")
                }).unwrap_or_default(),
                meas.get("worth_deepen").and_then(|v| v.as_bool()).unwrap_or(false),
                meas.get("deepen_reason").and_then(|v| v.as_str()).unwrap_or("-"),
                meas.get("b10x_ticks").and_then(|v| v.as_u64()).unwrap_or(0),
                meas.get("max_deepen_ticks").and_then(|v| v.as_u64()).unwrap_or(1),
                meas.get("cluster_votes").and_then(|v| v.as_u64()).unwrap_or(0),
                meas.get("cluster_need_votes").and_then(|v| v.as_u64()).unwrap_or(0),
            ));

            if meas.get("need_b10").and_then(|v| v.as_bool()).unwrap_or(false) {
                notes.push(
                    "residual_schedule: need_b10 first — B10x deferred until B10 lands (complete_probe)"
                        .into(),
                );
            } else if meas
                .get("worth_deepen")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                // Full B10x set eventually; **1 pack per tick** so multipath serial
                // + GL release never floods "Too many active WebGL contexts".
                // Priority order comes from deepen_components (silicon first).
                let reason = meas
                    .get("deepen_reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("always_open_b10x_after_b10");
                let components = meas
                    .get("deepen_components")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                // Lifetime: enough ticks to drain full specialized list (~9).
                let max_t = meas
                    .get("max_deepen_ticks")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(12) as usize;
                let already = meas
                    .get("b10x_ticks")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                let mut scheduled_n = 0usize;
                const B10X_PER_TICK: usize = 1;
                let room_left = max_t
                    .saturating_sub(already)
                    .min(B10X_PER_TICK);
                for pid in components {
                    if scheduled_n >= room_left {
                        break;
                    }
                    if present.contains(pid) {
                        continue;
                    }
                    if packs
                        .iter()
                        .any(|p| p.get("pack_id").and_then(|v| v.as_str()) == Some(pid))
                    {
                        continue;
                    }
                    if let Some(d) = resolve(pid) {
                        try_add(
                            &mut packs,
                            &mut notes,
                            &present,
                            max_packs.saturating_add(8),
                            d,
                            Some(d.priority.max(88)),
                        );
                        notes.push(format!(
                            "residual_schedule: deepen {} reason={} (B10x per_tick≤{B10X_PER_TICK})",
                            pid, reason
                        ));
                        scheduled_n += 1;
                    } else {
                        notes.push(format!(
                            "residual_schedule: catalog missing {} (probe_coverage_gap)",
                            pid
                        ));
                    }
                }
                if scheduled_n == 0 {
                    notes.push(
                        "residual_schedule: worth_deepen but no B10x pack available/executable"
                            .into(),
                    );
                } else {
                    notes.push(format!(
                        "residual_schedule: scheduled {scheduled_n} B10x pack(s) this frontier (cap={B10X_PER_TICK})"
                    ));
                }
            } else if meas
                .get("measurement_complete")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                notes.push(
                    "residual_schedule: measurement_complete — residual lane done (materials full; not OS trust)"
                        .into(),
                );
            } else {
                let ticks = meas.get("b10x_ticks").and_then(|v| v.as_u64()).unwrap_or(0);
                let max_t = meas
                    .get("max_deepen_ticks")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1);
                if ticks >= max_t {
                    notes.push(format!(
                        "residual_schedule: deepen_exhausted ticks={ticks}/{max_t} — mint with best measurement"
                    ));
                } else {
                    notes.push(
                        "residual_schedule: no deepen (soft_residual or budget)"
                            .into(),
                    );
                }
            }

            // Ops gap report (not a schedule skip).
            if meas
                .get("worth_deepen")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                || meas
                    .get("unconventional_kernel")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            {
                let gap = crate::engine_surface::detect_probe_coverage_gap(
                    evidence.get("fields").unwrap_or(&Value::Null),
                    &present_vec,
                );
                notes.push(format!(
                    "probe_coverage_gap: {}",
                    gap.get("code").and_then(|v| v.as_str()).unwrap_or("?")
                ));
            }
        }

        // iss/21 T-BRAIN-1: bio / CDP budget floor — schedule B11/B12 when kinematics or CDP thin.
        // Priority must not fall below soft-mid amplify path under tight budget.
        let bio_thin = !fo_brain.contains_key("input_mouse_entropy")
            && !fo_brain.contains_key("pre_action_move_count")
            && !fo_brain.contains_key("ttfi_ms");
        let cdp_thin = !fo_brain.contains_key("cdp_runtime_hint");
        if bio_thin || codes.contains("need_rpa_kinematics") || codes.contains("need_rpa_bind") {
            if let Some(d) = resolve("B11_interaction") {
                try_add_force(
                    &mut packs,
                    &mut notes,
                    &present,
                    max_packs.saturating_add(2),
                    d,
                    Some(95),
                    true,
                );
                notes.push("bio_budget_floor: force B11_interaction (kinematics/rpa)".into());
            }
        }
        if cdp_thin || codes.contains("need_cdp_probe") {
            if let Some(d) = resolve("B12_anti_camouflage") {
                try_add_force(
                    &mut packs,
                    &mut notes,
                    &present,
                    max_packs.saturating_add(2),
                    d,
                    Some(94),
                    true,
                );
                notes.push("cdp_budget_floor: force B12_anti_camouflage".into());
            }
        }

        // iss/22 P1a: early mission select from field-thin belief → filter UCB directions
        let fields_for_mis = evidence
            .get("fields")
            .cloned()
            .unwrap_or(Value::Object(Map::new()));
        let belief_thin = crate::brain_control::belief_from_fields_thin(&fields_for_mis);
        let env_thin = crate::brain_control::scan_capability_envelope(&fields_for_mis, &belief_thin);
        let missions_early =
            crate::brain_control::select_missions(&belief_thin, &gaps, &env_thin);
        notes.push(format!(
            "mission_primary={}",
            missions_early
                .get("primary")
                .and_then(|v| v.as_str())
                .unwrap_or("hold")
        ));

        // Directional brain (UCB-lite): rank within mission allowlist, then budget packs.
        // Reserve up to DENSE_PACKS_PER_TICK_CAP slots for high-sev dense digests so
        // UCB residual hard packs cannot starve claim-obs digests — and dense cannot
        // flood the whole tick (reserve only, not full budget).
        let dense_gap_open = [
            "need_api_flags_detail",
            "need_font_matrix_detail",
            "need_webrtc_ice_deep",
            "need_webgl_params_full",
            "need_navigator_deep",
            "need_client_hints_full",
            "need_worker_iframe_deep",
        ]
        .iter()
        .any(|c| codes.contains(*c));
        let room_after_floors = max_packs.saturating_sub(packs.len());
        let dense_reserve = if dense_gap_open {
            crate::brain_control::DENSE_PACKS_PER_TICK_CAP.min(room_after_floors)
        } else {
            0
        };
        // Directions fill non-reserved room only (do not steal dense reserve).
        let dir_room = room_after_floors.saturating_sub(dense_reserve);
        let remain = dir_room;
        if dense_reserve > 0 {
            notes.push(format!(
                "dense_digest_reserve={dense_reserve} dir_room={dir_room} (latency-safe)"
            ));
        }
        let mut deferred_dense_ucb: Vec<Value> = Vec::new();
        match crate::brain_directions::schedule_packs_from_directions_missions(
            evidence,
            remain,
            soft_v2_ready,
            Some(&missions_early),
        ) {
            Ok((dir_packs, dir_notes, dir_plan)) => {
                // Non-dense UCB first; dense deferred until after high-sev digest slots.
                for p in dir_packs {
                    let pid = p.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
                    if present.contains(pid) {
                        continue;
                    }
                    if packs
                        .iter()
                        .any(|x| x.get("pack_id").and_then(|v| v.as_str()) == Some(pid))
                    {
                        continue;
                    }
                    if packs.len() >= max_packs {
                        break;
                    }
                    if dense_gap_open && crate::brain_control::is_dense_pack_id(pid) {
                        deferred_dense_ucb.push(p);
                        continue;
                    }
                    packs.push(p);
                }
                if !deferred_dense_ucb.is_empty() {
                    notes.push(format!(
                        "dense_ucb_deferred={}",
                        deferred_dense_ucb.len()
                    ));
                }
                notes.extend(dir_notes);
                notes.push(format!(
                    "brain_directions: {}",
                    dir_plan
                        .get("scheduled_directions")
                        .map(|v| v.to_string())
                        .unwrap_or_default()
                ));
                notes.push(format!(
                    "direction_plan_algo={}",
                    dir_plan
                        .get("algo")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                ));
            }
            Err(e) => notes.push(format!("brain_directions error: {e}")),
        }

        // Force re-schedule challenge when gap code present and pack already empty-seeded.
        if codes.contains("need_challenge_seed") || codes.contains("need_pohw_challenge") {
            if let Some(d) = resolve("B20_challenge_seed") {
                try_add_force(
                    &mut packs,
                    &mut notes,
                    &present,
                    max_packs,
                    d,
                    Some(97),
                    true,
                );
                notes.push("gap need_challenge_seed/need_pohw → force B20_challenge_seed".into());
            }
        }
        if codes.contains("need_gpu_staircase")
            || codes.contains("need_challenge_seed")
            || codes.contains("need_pohw_challenge")
        {
            if let Some(d) = resolve("B22_gpu_timer") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(96));
                notes.push("gap need_gpu_staircase/H01 → B22_gpu_timer".into());
            }
        }
        if codes.contains("need_census_volume") || codes.contains("need_census_density") {
            if let Some(d) = resolve("B21_census_volume") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(88));
                notes.push("gap need_census_volume → B21_census_volume".into());
            }
            if let Some(d) = resolve("B5_census") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(54));
            }
        }
        if codes.contains("need_native_canvas_hedge") || codes.contains("need_hedge_materials") {
            if let Some(d) = resolve("B23_native_canvas_hedge") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(89));
                notes.push("gap hedge → B23_native_canvas_hedge".into());
            }
        }
        if codes.contains("need_material_crosscheck") || codes.contains("need_hedge_materials") {
            if let Some(d) = resolve("B24_material_crosscheck") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(87));
                notes.push("gap hedge → B24_material_crosscheck".into());
            }
        }
        if codes.contains("need_shader_numeric") || codes.contains("need_hedge_materials") {
            if let Some(d) = resolve("B31_shader_numeric") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(95));
                notes.push("gap H03 → B31_shader_numeric".into());
            }
        }
        if codes.contains("need_caps_pressure") || codes.contains("need_hedge_materials") {
            if let Some(d) = resolve("B33_caps_pressure") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(94));
                notes.push("gap H05 → B33_caps_pressure".into());
            }
        }
        if codes.contains("need_clock_raf") {
            if let Some(d) = resolve("B25_clock_raf") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(86));
                notes.push("gap H08 → B25_clock_raf".into());
            }
        }
        if codes.contains("need_eme_codec_matrix") || codes.contains("need_media_eme") {
            if let Some(d) = resolve("B19_eme_media") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(85));
                notes.push("gap H10 → B19_eme_media".into());
            }
        }
        if codes.contains("need_gpu_ns") {
            if let Some(d) = resolve("B22_gpu_timer") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(96));
                notes.push("gap H01 gpu-ns → B22_gpu_timer".into());
            }
        }
        if codes.contains("need_signed_challenge") {
            if let Some(d) = resolve("B20_challenge_seed") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(97));
                notes.push("gap H16 signed → B20_challenge_seed".into());
            }
        }
        if codes.contains("need_permissions_media") {
            if let Some(d) = resolve("B28_permissions_media") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(80));
            }
        }
        if codes.contains("need_sensors_battery") {
            if let Some(d) = resolve("B29_sensors_battery") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(79));
            }
        }
        if codes.contains("need_gpu_bandwidth_h02") {
            if let Some(d) = resolve("B30_gpu_bandwidth") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(93));
            }
        }
        if codes.contains("need_cpu_cache_ladder") {
            if let Some(d) = resolve("B34_cpu_cache_ladder") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(84));
            }
        }
        if codes.contains("need_wasm_throughput") {
            if let Some(d) = resolve("B88_wasm_instruction_throughput") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(94));
            }
        }
        if codes.contains("need_engine_behavior") {
            if let Some(d) = resolve("B87_engine_behavior_diff") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(86));
            }
        }
        if codes.contains("need_os_surface_h20") {
            if let Some(d) = resolve("B86_os_gecko_surface") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(83));
            }
        }
        if codes.contains("need_audio_known_lock") {
            if let Some(d) = resolve("B89_audio_known_lock") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(78));
            }
        }
        if codes.contains("need_emoji_raster") {
            if let Some(d) = resolve("B90_os_emoji_raster") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(73));
            }
        }
        if codes.contains("need_storage_quota") {
            if let Some(d) = resolve("B91_storage_disk_quota") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(71));
            }
        }
        if codes.contains("need_protocol_edge") {
            if let Some(d) = resolve("B8_gateway_early") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(98));
            }
        }
        if codes.contains("need_h3_app") {
            if let Some(d) = resolve("B8_gateway_early") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(96));
            }
            if let Some(d) = resolve("B9_network") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(70));
            }
        }
        if codes.contains("need_protocol_tcp_quic_reconcile") {
            if let Some(d) = resolve("B1_conflict") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(88));
            }
            if let Some(d) = resolve("B40_websocket_fp") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(75));
            }
            if let Some(d) = resolve("B8_gateway_early") {
                try_add_force(
                    &mut packs,
                    &mut notes,
                    &present,
                    max_packs,
                    d,
                    Some(90),
                    true,
                );
            }
        }
        if codes.contains("need_agent_parity") {
            if let Some(d) = resolve("B26_agent_parity") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(81));
            }
        }
        if codes.contains("need_storage_privacy") {
            if let Some(d) = resolve("B27_storage_privacy") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(78));
            }
        }
        if codes.contains("need_dom_perf") {
            if let Some(d) = resolve("B35_dom_perf") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(77));
            }
        }
        // B15 is soft-gated mid4 — never schedule when soft_v2 not ready (J3 / brain_gap_schedule).
        if codes.contains("need_layer_divergence") && soft_v2_ready && allow_amplify {
            if let Some(d) = resolve("B15_cross_curves") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(82));
            }
        }
        if codes.contains("need_webgpu_deep") {
            if let Some(d) = resolve("B18_webgpu") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(91));
            }
        }
        // B93 static fork matrix: default mid, no new permissions (B65-style).
        if codes.contains("need_blink_fork_matrix") {
            if let Some(d) = resolve("B93_blink_fork_matrix") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(44));
            }
        }
        // B5 research unlock fallback: reached only when task_gap_map failed to
        // load (the primary path already ran it before the fill).
        b5_research_unlock(&mut packs, &mut notes);
        if codes.contains("need_raster_msaa") {
            if let Some(d) = resolve("B36_raster_msaa") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(88));
            }
        }
        if codes.contains("need_thermal_drift") {
            if let Some(d) = resolve("B37_thermal_drift_lite") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(72));
            }
        }
        if codes.contains("need_neg_dict") {
            if let Some(d) = resolve("B38_neg_dict") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(83));
            }
        }
        if codes.contains("need_mem_pressure") {
            if let Some(d) = resolve("B39_mem_pressure") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(76));
            }
        }
        if codes.contains("need_websocket_fp") {
            if let Some(d) = resolve("B40_websocket_fp") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(70));
            }
        }
        if codes.contains("need_hid_gamepad") {
            if let Some(d) = resolve("B41_hid_gamepad") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(69));
            }
        }
        if codes.contains("need_thermal_full") {
            if let Some(d) = resolve("B42_thermal_drift_full") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(74));
            }
        }
        if codes.contains("need_errors_engine") {
            if let Some(d) = resolve("B43_errors_engine") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(73));
            }
        }
        if codes.contains("need_speech_deep") {
            if let Some(d) = resolve("B44_speech_deep") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(68));
            }
        }
        if codes.contains("need_display_hdr") {
            if let Some(d) = resolve("B45_display_hdr") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(71));
            }
        }
        if codes.contains("need_gpu_ns_dense") {
            if let Some(d) = resolve("B22_gpu_timer") {
                try_add_force(
                    &mut packs,
                    &mut notes,
                    &present,
                    max_packs,
                    d,
                    Some(96),
                    true,
                );
            }
        }
        if codes.contains("need_audio_deep") {
            if let Some(d) = resolve("B46_audio_deep") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, Some(72));
            }
        }
        if codes.contains("need_webgpu_secure_context") {
            if let Some(d) = resolve("B18_webgpu") {
                try_add_force(
                    &mut packs,
                    &mut notes,
                    &present,
                    max_packs,
                    d,
                    Some(90),
                    true,
                );
            }
        }
        if codes.contains("need_media_eme") {
            if let Some(d) = resolve("B19_eme_media") {
                try_add_force(
                    &mut packs,
                    &mut notes,
                    &present,
                    max_packs,
                    d,
                    Some(85),
                    true,
                );
            }
        }

        // Latency-safe dense digests (B47–B79 high-sev only, capped per tick).
        // Commercial floors already consumed budget; only residual slots get digests.
        // Low/mid dense gaps wait multi-tick — avoids 33-pack flood on first tick.
        {
            use crate::brain_control::{is_dense_pack_id, DENSE_PACKS_PER_TICK_CAP};
            let dense_high: &[(&str, &str, i64)] = &[
                ("need_api_flags_detail", "B47_api_flags_detail", 62),
                ("need_font_matrix_detail", "B50_font_matrix_detail", 61),
                ("need_webrtc_ice_deep", "B55_webrtc_ice_deep", 60),
                ("need_webgl_params_full", "B59_webgl_params_full", 59),
                ("need_navigator_deep", "B52_navigator_deep", 58),
                ("need_client_hints_full", "B65_client_hints_full", 55),
                ("need_ua_ch_high_entropy", "B65_client_hints_full", 56),
                ("need_storage_estimate_async", "B27_storage_privacy", 54),
                ("need_webgpu_adapter_async", "B18_webgpu", 70),
                ("need_speech_voices_async", "B44_speech_deep", 52),
                ("need_media_capabilities_async", "B19_eme_media", 58),
                ("need_worker_iframe_deep", "B77_worker_env_deep", 54),
            ];
            let mut dense_n = packs
                .iter()
                .filter(|p| {
                    p.get("pack_id")
                        .and_then(|v| v.as_str())
                        .map(is_dense_pack_id)
                        .unwrap_or(false)
                })
                .count();
            let dense_before = dense_n;
            for (code, pid, prio) in dense_high {
                if dense_n >= DENSE_PACKS_PER_TICK_CAP {
                    break;
                }
                if packs.len() >= max_packs {
                    break;
                }
                if !codes.contains(*code) {
                    continue;
                }
                if let Some(d) = resolve(pid) {
                    // Research-gated dense lanes (B18/B47) stay out of the default
                    // schedule until their intra/inter-device KPI gate clears.
                    if d.gate == "research" {
                        continue;
                    }
                    let before = packs.len();
                    try_add(&mut packs, &mut notes, &present, max_packs, d, Some(*prio));
                    if packs.len() > before {
                        dense_n += 1;
                    }
                }
            }
            if dense_n > dense_before {
                notes.push(format!(
                    "dense_digest_slots: added={} cap={} (high-sev only)",
                    dense_n - dense_before,
                    DENSE_PACKS_PER_TICK_CAP
                ));
            }
            // Residual UCB dense (hard canvas/audio etc.) only if digest cap not full.
            for p in deferred_dense_ucb.drain(..) {
                if dense_n >= DENSE_PACKS_PER_TICK_CAP || packs.len() >= max_packs {
                    break;
                }
                let pid = p.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
                if present.contains(pid)
                    || packs
                        .iter()
                        .any(|x| x.get("pack_id").and_then(|v| v.as_str()) == Some(pid))
                {
                    continue;
                }
                packs.push(p);
                dense_n += 1;
            }
            if dense_n > 0 {
                notes.push(format!(
                    "dense_total_this_tick={dense_n} cap={}",
                    DENSE_PACKS_PER_TICK_CAP
                ));
            }
        }

        // B15 cross curves after soft ready (mid4) or when amplify allowed.
        if allow_amplify && soft_v2_ready {
            if let Some(d) = resolve("B15_cross_curves") {
                try_add(&mut packs, &mut notes, &present, max_packs, d, None);
            }
        }
    }

    // Staged multi-source sandbox plan always attached for FE (even on cool-down for rpa surfaces).
    let sandbox_plan = crate::brain_directions::sandbox_stage_plan(evidence);
    notes.push(format!(
        "sandbox_stage wave={} kinds={}",
        sandbox_plan
            .get("wave")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        sandbox_plan
            .get("kinds")
            .map(|v| v.to_string())
            .unwrap_or_default()
    ));

    // Hard-anchor boost: commercial curve packs always outrank soft mid/deep under budget.
    // Effective sort key = priority + hard_boost (B10 / hard_eligible static).
    // iss/21 T-BRAIN-1: bio/CDP packs get floor ≥ soft-mid amplify (boost 2_500).
    // iss/31 G-31-1: re_probe_priority from analysis_quality elevates verification packs.
    let fo_prio = evidence
        .get("fields")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let bio_thin_prio = !fo_prio.contains_key("input_mouse_entropy")
        && !fo_prio.contains_key("pre_action_move_count")
        && !fo_prio.contains_key("ttfi_ms");
    let cdp_thin_prio = !fo_prio.contains_key("cdp_runtime_hint");
    let soft_stack_prio = fo_prio
        .get("soft_stack")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || fo_prio
            .get("residual_soft_like")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        || codes.contains("soft_stack_budget_rpa_os");

    // Open verification gaps → candidate packs (quality re_probe_priority).
    let open_gaps_val = crate::analysis_quality::open_verification_gaps(evidence);
    let mut re_probe_packs: HashSet<String> = HashSet::new();
    let mut re_probe_codes: Vec<String> = Vec::new();
    if let Some(arr) = open_gaps_val
        .get("re_probe_priority")
        .and_then(|v| v.as_array())
    {
        for g in arr {
            if let Some(code) = g.get("code").and_then(|v| v.as_str()) {
                re_probe_codes.push(code.to_string());
            }
        }
    }
    // Also map open high gaps via task_gap_map
    if let Ok(tmap) = load_task_gap_map() {
        let refs: Vec<&str> = re_probe_codes.iter().map(|s| s.as_str()).collect();
        for pid in tmap.packs_for_gap_codes(refs, soft_v2_ready) {
            re_probe_packs.insert(pid.to_string());
        }
    }
    // Hard-code high-value verification packs for known gap codes (even if map thin)
    for code in &re_probe_codes {
        if code.contains("unit")
            || code.contains("device")
            || code.contains("claim_obs")
            || code.contains("b10_curves")
            || code.contains("host_sep")
            || code.contains("residual")
        {
            re_probe_packs.insert("B10_hw_curves".into());
            re_probe_packs.insert("B2_hardware".into());
            re_probe_packs.insert("B20_challenge_seed".into());
        }
        if code.contains("capability") || code.contains("br_") {
            re_probe_packs.insert("B2_hardware".into());
            re_probe_packs.insert("B17_hw_physical".into());
            re_probe_packs.insert("B12_anti_camouflage".into());
        }
        if code.contains("rpa") || code.contains("cdp") {
            re_probe_packs.insert("B11_interaction".into());
            re_probe_packs.insert("B12_anti_camouflage".into());
        }
        if code.contains("os_instance") || code.contains("os_") {
            re_probe_packs.insert("B3_system".into());
            re_probe_packs.insert("B16_fast_signals".into());
        }
        // host_sep re-probe: system + network only — never soft-gated mid (B15/B16) here.
        if code.contains("host_sep") {
            re_probe_packs.insert("B3_system".into());
            re_probe_packs.insert("B9_network".into());
        }
        if code.contains("soft_stack") {
            re_probe_packs.insert("B11_interaction".into());
            re_probe_packs.insert("B3_system".into());
            re_probe_packs.insert("B10_hw_curves".into());
        }
    }
    // iss/38 P1-1: hypothesis table elevates packs (stacked with re_probe).
    let fields_for_hyp = evidence
        .get("fields")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let belief_tags: Vec<String> = evidence
        .pointer("/belief/adversary_tags")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(|s| s.to_string()))
        .collect();
    let (active_hyps, hyp_packs) =
        crate::brain_hypotheses::elevated_packs_for_fields(&fields_for_hyp, &belief_tags);
    for pid in &hyp_packs {
        re_probe_packs.insert(pid.clone());
    }
    if !active_hyps.is_empty() {
        notes.push(format!(
            "brain_hypotheses_active: {}",
            active_hyps.join(",")
        ));
    }

    // Schedule missing re_probe packs (verification first).
    // iss/65: NEVER force_recollect on every multi-tick analyze when batch already
    // present — that caused B2/B12/B11/B3 wire storms (5–30× sealed_ok).
    // force=false → try_add_force skips present batches; only schedules gaps.
    if !skip_session {
        for pid in ["B10_hw_curves", "B2_hardware", "B11_interaction", "B12_anti_camouflage", "B3_system", "B20_challenge_seed", "B17_hw_physical"] {
            if re_probe_packs.contains(pid)
                && !packs
                    .iter()
                    .any(|p| p.get("pack_id").and_then(|v| v.as_str()) == Some(pid))
            {
                if let Some(d) = resolve(pid) {
                    try_add_force(
                        &mut packs,
                        &mut notes,
                        &present,
                        max_packs.saturating_add(6),
                        d,
                        Some(d.priority.max(90)),
                        false,
                    );
                    notes.push(format!("re_probe_priority schedule_if_missing: {pid}"));
                }
            }
        }
    }
    if !re_probe_codes.is_empty() {
        notes.push(format!(
            "re_probe_priority: {} gap codes → {} verification packs elevated",
            re_probe_codes.len(),
            re_probe_packs.len()
        ));
    }
    // Force-schedule hypothesis packs similarly to re_probe.
    // Soft-gated mid4/deep must still honor soft_v2_ready (J3).
    if !skip_session {
        for pid in &hyp_packs {
            if !packs.iter().any(|p| p.get("pack_id").and_then(|v| v.as_str()) == Some(pid.as_str()))
            {
                if let Some(d) = resolve(pid) {
                    let soft_gated = d.requires_soft_v2
                        || d.layer == "mid4"
                        || d.layer == "deep"
                        || pid == "B15_cross_curves"
                        || pid == "B13_authorized"
                        || pid == "B16_fast_signals";
                    if soft_gated && !(soft_v2_ready && allow_amplify) {
                        notes.push(format!(
                            "brain_hypothesis skip soft-gated {pid} (soft_v2 not ready)"
                        ));
                        continue;
                    }
                    // iss/65: schedule if missing only — do not force_recollect every tick.
                    try_add_force(
                        &mut packs,
                        &mut notes,
                        &present,
                        max_packs.saturating_add(6),
                        d,
                        Some(d.priority.max(88)),
                        false,
                    );
                    notes.push(format!("brain_hypothesis schedule_if_missing: {pid}"));
                }
            }
        }
    }

    for p in packs.iter_mut() {
        let pid = p
            .get("pack_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let hard = p
            .get("hard_eligible")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || pid == "B10_hw_curves"
            || pid == "B0_bootstrap"
            || pid == "B2_hardware"
            || pid == "B3_system";
        let layer = p.get("layer").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let soft_mid = layer == "mid4" || layer == "deep";
        let base = p.get("priority").and_then(|v| v.as_i64()).unwrap_or(0);
        let mut boost = if pid == "B10_hw_curves" || pid == "B17_hw_physical" {
            10_000
        } else if hard && !soft_mid {
            5_000
        } else if soft_mid {
            0
        } else {
            1_000
        };
        // Bio/CDP deepen: never below soft-mid floor when materials thin
        if (pid == "B11_interaction" && bio_thin_prio)
            || (pid == "B12_anti_camouflage" && cdp_thin_prio)
        {
            boost = boost.max(2_500);
        }
        // Quality re_probe elevation (device unit / claim-obs / control / env)
        let re_elev = re_probe_packs.contains(&pid);
        if re_elev {
            boost = boost.max(7_500);
        }
        let hyp_elev = hyp_packs.iter().any(|h| h == &pid);
        if hyp_elev {
            boost = boost.max(7_200);
        }
        // Soft class: demote pure GPU host digs relative to verification
        let soft_gpu_demote = soft_stack_prio
            && (pid == "B22_gpu_timer" || pid == "B18_webgpu" || pid == "B30_gpu_bandwidth");
        if soft_gpu_demote {
            boost = 0;
        }
        // B2 info-gain pricing: effective_priority = base + boost + ig(axis)
        let (ig, ig_gap) = axis_info_gain(evidence, &pid, &layer);
        if let Some(obj) = p.as_object_mut() {
            obj.insert("effective_priority".into(), json!(base + boost + ig));
            obj.insert("hard_anchor_boost".into(), json!(boost));
            obj.insert("ig_gain".into(), json!(ig));
            obj.insert("ig_axis".into(), json!(pack_axis(&pid)));
            obj.insert("ig_axis_gap".into(), json!(ig_gap));
            if re_elev {
                obj.insert("re_probe_elevated".into(), json!(true));
            }
            if hyp_elev {
                obj.insert("hypothesis_elevated".into(), json!(true));
            }
            if soft_gpu_demote {
                obj.insert("soft_gpu_demoted".into(), json!(true));
            }
        }
    }
    notes.push(
        "hard_anchor_priority: B10/hard_eligible outrank soft mid/deep under budget; +ig(axis) info-gain pricing".into(),
    );
    if bio_thin_prio || cdp_thin_prio {
        notes.push(
            "bio_cdp_budget_floor: B11/B12 effective_priority ≥ soft-mid amplify path".into(),
        );
    }
    if soft_stack_prio {
        notes.push(
            "soft_stack_budget: GPU host digs demoted; re_probe unit/rpa/os elevated".into(),
        );
    }

    // Sort by effective_priority desc, then pack_id
    packs.sort_by(|a, b| {
        let pa = a
            .get("effective_priority")
            .or_else(|| a.get("priority"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let pb = b
            .get("effective_priority")
            .or_else(|| b.get("priority"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        pb.cmp(&pa).then_with(|| {
            let ida = a.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
            let idb = b.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
            ida.cmp(idb)
        })
    });
    // If budget tight and B10 still missing from present, drop soft mid/deep first.
    let hard_missing = !present.contains("B10_hw_curves");
    if hard_missing && packs.len() > max_packs {
        packs.retain(|p| {
            let layer = p.get("layer").and_then(|v| v.as_str()).unwrap_or("");
            let pid = p.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
            layer != "mid4" && layer != "deep"
                || pid == "B10_hw_curves"
                || pid == "B11_interaction"
                || pid == "B12_anti_camouflage"
                || pid == "B7_sandbox"
        });
        notes.push("budget_trim: dropped soft mid/deep while B10 hard curves still missing".into());
    }
    // Budget trim must keep B8 / lite5 static (missing_server path) ahead of residual hard.
    // iss/21 T-BRAIN-1: also protect B11/B12 bio+cdp floors + B7 multi-source.
    if packs.len() > max_packs {
        let must = |p: &Value| {
            let layer = p.get("layer").and_then(|v| v.as_str()).unwrap_or("");
            let pid = p.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
            layer == "b8"
                || layer == "lite5"
                || layer == "mid4"
                || pid == "B8_gateway_early"
                || pid == "edge.cf"
                || pid == "B10_hw_curves"
                || pid == "B11_interaction"
                || pid == "B12_anti_camouflage"
                || pid == "B7_sandbox"
        };
        let mut keep: Vec<Value> = packs.iter().filter(|p| must(p)).cloned().collect();
        let rest: Vec<Value> = packs.into_iter().filter(|p| !must(p)).collect();
        let room = max_packs.saturating_sub(keep.len());
        keep.extend(rest.into_iter().take(room));
        // re-sort
        keep.sort_by(|a, b| {
            let pa = a
                .get("effective_priority")
                .or_else(|| a.get("priority"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let pb = b
                .get("effective_priority")
                .or_else(|| b.get("priority"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            pb.cmp(&pa).then_with(|| {
                let ida = a.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
                let idb = b.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
                ida.cmp(idb)
            })
        });
        packs = keep;
        notes.push(format!(
            "pack budget max_packs={max_packs} (kept b8/lite5/B10; residual next tick)"
        ));
    }

    // B1 axis budgets: per-axis caps by device class (replaces fixed band cap).
    // Runs after the max_packs trim so both constraints compose: axis trim only
    // drops over-budget *lowest-priority* non-anchored packs on the same axis.
    let axis_budget_usage = {
        let fo = evidence
            .get("fields")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        let cls = device_class(&fo);
        trim_to_axis_budget(&mut packs, cls, &mut notes)
    };

    // Thin-batch recollect: present pack but missing contract materials → force again
    // (present ≠ done). Missions tag analysis→next-probe correspondence (≤4).
    if !skip_session {
        let fo = evidence
            .get("fields")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        let thin_contracts: &[(&str, &[&str])] = &[
            (
                "B10_hw_curves",
                &["hw_curve_webgl", "hw_curve_audio", "residual_mean"],
            ),
            (
                "B20_challenge_seed",
                &["pohw_triad", "challenge_residual_mean", "challenge_seed_digest"],
            ),
            (
                "B15_cross_curves",
                &[
                    "layer_divergence_score",
                    "multi_source_match_ratio",
                    "cross_residual_delta",
                ],
            ),
            (
                "B9_network",
                &["ice_has_host", "ice_candidate_types", "webrtc_host_count", "ice_morphology"],
            ),
            (
                "B11_interaction",
                &["behavior_early_bound", "input_mouse_entropy", "behavior_events"],
            ),
            ("B7_sandbox", &["sandbox_sources", "layer_divergence_n", "sandbox_ok"]),
        ];
        for (pid, keys) in thin_contracts {
            if !present.contains(*pid) {
                continue;
            }
            let any_key = keys.iter().any(|k| {
                fo.get(*k)
                    .map(|v| {
                        if v.is_null() {
                            false
                        } else if let Some(s) = v.as_str() {
                            !s.is_empty()
                        } else if let Some(a) = v.as_array() {
                            !a.is_empty()
                        } else {
                            true
                        }
                    })
                    .unwrap_or(false)
            });
            if any_key {
                continue;
            }
            if let Some(d) = resolve(pid) {
                try_add_force(
                    &mut packs,
                    &mut notes,
                    &present,
                    max_packs,
                    d,
                    Some(d.priority + 50),
                    true,
                );
                notes.push(format!(
                    "thin_recollect {pid}: present but missing contract keys {keys:?}"
                ));
            }
        }

        let mut missions: Vec<String> = Vec::new();
        if codes.iter().any(|c| {
            c.contains("device") || c.contains("hard_anchor") || c.contains("hw_curve")
        }) {
            missions.push("secure_device_anchor".into());
        }
        if codes.iter().any(|c| {
            c.contains("soft") || c.contains("gpu") || c.contains("residual") || c.contains("stack")
        }) {
            missions.push("disprove_soft_os".into());
        }
        if codes.iter().any(|c| {
            c.contains("br")
                || c.contains("kernel")
                || c.contains("sandbox")
                || c.contains("agent")
                || c.contains("layer")
        }) {
            missions.push("resolve_br_incoherence".into());
        }
        if codes
            .iter()
            .any(|c| c.contains("rpa") || c.contains("behavior") || c.contains("interaction"))
        {
            missions.push("raise_rpa_clarity".into());
        }
        if codes.iter().any(|c| {
            c.contains("gateway")
                || c.contains("claim")
                || c.contains("protocol")
                || c.contains("webrtc")
                || c.contains("ice")
        }) {
            missions.push("claim_obs_verify".into());
        }
        if missions.is_empty() && !codes.is_empty() {
            missions.push("probe_unknown".into());
        }
        missions.truncate(4);
        if !missions.is_empty() {
            notes.push(format!("missions={}", missions.join(",")));
            for p in packs.iter_mut() {
                if let Some(obj) = p.as_object_mut() {
                    obj.insert("missions".into(), json!(missions.clone()));
                }
            }
        }
    }

    // Cool-safe silicon B10x (EDH research gate) — mirror R spotcheck:
    // session cool-down must NOT skip B10x; otherwise b10x_ticks stays 0 forever.
    // Dedupes against always_open deepen scheduled inside !skip_session via present/packs.
    {
        let has_b10 = crate::session_ticket::evidence_has_b10(evidence)
            || present.contains("B10_hw_curves")
            || present.contains("mid.curves");
        let missing = crate::edh::missing_b10x_silicon(evidence);
        if has_b10 && !missing.is_empty() {
            let room = if skip_session {
                packs.len().saturating_add(missing.len()).max(missing.len() + 2)
            } else {
                max_packs.saturating_add(missing.len())
            };
            let mut scheduled_n = 0usize;
            for (i, pid) in missing.iter().enumerate() {
                if present.contains(pid.as_str())
                    || packs.iter().any(|p| {
                        p.get("pack_id").and_then(|v| v.as_str()) == Some(pid.as_str())
                    })
                {
                    continue;
                }
                if let Some(d) = resolve(pid) {
                    let before = packs.len();
                    let prio = Some(d.priority.max(88).max(1090 - i as i64));
                    try_add(&mut packs, &mut notes, &present, room, d, prio);
                    if packs.len() > before {
                        if let Some(last) = packs.last_mut() {
                            if let Some(obj) = last.as_object_mut() {
                                obj.insert("reason".into(), json!("cool_safe_b10x_silicon_must_land"));
                                obj.insert("pack_lane".into(), json!("deepen"));
                                obj.insert("hard_eligible".into(), json!(true));
                            }
                        }
                        scheduled_n += 1;
                    }
                } else {
                    notes.push(format!(
                        "cool_safe_b10x: catalog missing {} (probe_coverage_gap)",
                        pid
                    ));
                }
            }
            if scheduled_n > 0 {
                notes.push(format!(
                    "cool_safe_b10x: scheduled {scheduled_n} silicon pack(s) missing=[{}] cool={}",
                    missing.join(","),
                    skip_session
                ));
            } else if skip_session {
                notes.push(format!(
                    "cool_safe_b10x: worth schedule but none added missing=[{}] cool=true",
                    missing.join(",")
                ));
            }
        } else if skip_session && has_b10 {
            notes.push("cool_safe_b10x: silicon complete or none missing".into());
        }
    }

    // R random spotcheck: accompany **each B wave** while device-id B work remains.
    // Product: R is not "once and forever stop" — schedule 1 new R with each frontier
    // that still has non-R B packs to run. When no B packs left this tick and B floors
    // complete, stop adding R (completion = brain has no remaining tasks).
    {
        use crate::brain_control::{verify_spotcheck_n_for_session, VERIFY_SPOTCHECK_MAX};
        let non_r_packs_this_tick = packs.iter().any(|p| {
            let pid = p.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
            !crate::brain_control::is_verify_rand_pack(pid)
        });
        let cov_snap = coverage_checklist(
            evidence,
            &present,
            soft_v2_ready,
            allow_amplify && soft_v2_ready,
        )
        .ok();
        let b_floors_done = cov_snap
            .as_ref()
            .map(|c| {
                c.get("static_complete")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                    && c.get("identity_complete")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    && c.get("mid_complete")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    && c.get("gateway_complete")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    && c.get("dense_complete")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
            })
            .unwrap_or(false);
        let r_floor = present
            .iter()
            .any(|id| crate::brain_control::is_verify_rand_pack(id));
        // Schedule R when: (wave has B work) OR (need first R for floor).
        // Skip only when B floors done AND ≥1 R already landed AND no B this tick.
        let want_r = non_r_packs_this_tick || !r_floor || !b_floors_done;
        let want_r = want_r && !(b_floors_done && r_floor && !non_r_packs_this_tick);
        if !want_r {
            notes.push(
                "verify_spotcheck: B floors complete + R floor met + empty B wave — no R"
                    .into(),
            );
        } else {
            let mut h: u64 = 0xcbf29ce484222325;
            for b in sid.as_bytes() {
                h ^= u64::from(*b);
                h = h.wrapping_mul(0x100000001b3);
            }
            h ^= (plan_version as u64).wrapping_mul(0x9e3779b97f4a7c15);
            let n_pick = verify_spotcheck_n_for_session(h).max(1).min(1); // 1 R per wave
            let mut picked = 0usize;
            let mut attempts = 0usize;
            while picked < n_pick && attempts < 200 {
                attempts += 1;
                let idx = ((h.wrapping_add(attempts as u64 * 17)) % 100) as usize;
                let pid = format!("R{idx:02}_spotcheck");
                if present.contains(&pid)
                    || packs
                        .iter()
                        .any(|p| p.get("pack_id").and_then(|v| v.as_str()) == Some(pid.as_str()))
                {
                    continue;
                }
                if let Some(d) = resolve(&pid) {
                    let room = if skip_session {
                        packs
                            .len()
                            .saturating_add(VERIFY_SPOTCHECK_MAX)
                            .max(VERIFY_SPOTCHECK_MAX + 2)
                    } else {
                        max_packs.saturating_add(VERIFY_SPOTCHECK_MAX)
                    };
                    let before = packs.len();
                    try_add(
                        &mut packs,
                        &mut notes,
                        &present,
                        room,
                        d,
                        Some(22),
                    );
                    if packs.len() > before {
                        if let Some(last) = packs.last_mut() {
                            if let Some(obj) = last.as_object_mut() {
                                obj.insert("verify_spotcheck".into(), json!(true));
                                obj.insert("score_weight".into(), json!(0.0));
                                obj.insert("role".into(), json!("authenticity_spotcheck"));
                            }
                        }
                        picked += 1;
                    }
                }
            }
            if picked > 0 {
                notes.push(format!(
                    "verify_spotcheck: wave R scheduled {picked} (with B wave) cool={}",
                    skip_session
                ));
            } else {
                notes.push(format!(
                    "verify_spotcheck: wave R none added cool={}",
                    skip_session
                ));
            }
        }
    }

    // ── B4 numeric verify closed loop (iss/74 §5-B4) ────────────────────────
    // A3 divergence = divergent → brain targets a numeric value-compare
    // spotcheck (R-lane upgraded: same-name packs compare values, not
    // existence). Persistent divergence across analysis revs escalates the
    // spoof uplift hint and lowers verify bandwidth.
    let mut numeric_verify = json!({});
    {
        let xsrc = evidence
            .get("multi_source_consistency")
            .and_then(|m| m.get("xsrc_numeric"))
            .cloned()
            .unwrap_or(Value::Null);
        let divergent_slots: Vec<String> = xsrc
            .get("slots")
            .and_then(|s| s.as_object())
            .map(|slots| {
                slots
                    .values()
                    .filter(|d| {
                        d.get("verdict").and_then(|v| v.as_str()) == Some("divergent")
                    })
                    .filter_map(|d| d.get("slot").and_then(|v| v.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let evidence_rev = evidence
            .get("evidence_rev")
            .or_else(|| evidence.get("meta").and_then(|m| m.get("evidence_rev")))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if !divergent_slots.is_empty() {
            // Persistent: the same divergence was already observable in a prior
            // analysis pass (evidence_rev ≥ 2 ⇒ this plan is not the first).
            let persistent = evidence_rev >= 2;
            let uplift: f64 = if persistent { 0.10 } else { 0.05 };
            // Target a deterministic R pack so repeat plans converge on the
            // same candidate (value-compare semantics attached as metadata).
            let mut h: u64 = 0xcbf29ce484222325;
            for b in format!("{sid}|{}", divergent_slots[0]).as_bytes() {
                h ^= u64::from(*b);
                h = h.wrapping_mul(0x100000001b3);
            }
            let idx = (h % 100) as usize;
            let pid = format!("R{idx:02}_spotcheck");
            if !present.contains(&pid)
                && !packs
                    .iter()
                    .any(|p| p.get("pack_id").and_then(|v| v.as_str()) == Some(pid.as_str()))
            {
                if let Some(d) = resolve(&pid) {
                    let before = packs.len();
                    try_add(&mut packs, &mut notes, &present, max_packs.saturating_add(2), d, Some(30));
                    if packs.len() > before {
                        if let Some(last) = packs.last_mut() {
                            if let Some(obj) = last.as_object_mut() {
                                obj.insert("verify_numeric".into(), json!(true));
                                obj.insert("verify_numeric_slots".into(), json!(divergent_slots));
                                obj.insert("verify_numeric_mode".into(), json!("value_compare"));
                                obj.insert("verify_numeric_persistent".into(), json!(persistent));
                                obj.insert(
                                    "verify_numeric_bandwidth_hint".into(),
                                    json!(if persistent { "lowest" } else { "low" }),
                                );
                            }
                        }
                    }
                }
            } else if let Some(p) = packs.iter_mut().find(|p| {
                p.get("pack_id").and_then(|v| v.as_str()) == Some(pid.as_str())
            }) {
                // Targeted R already present this wave — upgrade semantics.
                if let Some(obj) = p.as_object_mut() {
                    obj.insert("verify_numeric".into(), json!(true));
                    obj.insert("verify_numeric_slots".into(), json!(divergent_slots));
                    obj.insert("verify_numeric_mode".into(), json!("value_compare"));
                    obj.insert("verify_numeric_persistent".into(), json!(persistent));
                    obj.insert(
                        "verify_numeric_bandwidth_hint".into(),
                        json!(if persistent { "lowest" } else { "low" }),
                    );
                }
            }
            let uplift_rounded = (uplift * 100.0).round() / 100.0;
            numeric_verify = json!({
                "slots": divergent_slots,
                "mode": "value_compare",
                "persistent": persistent,
                "spoof_risk_uplift": uplift_rounded,
            });
            notes.push(format!(
                "numeric_verify: divergent xsrc slots {:?} → {} value-compare spotcheck (persistent={persistent}, uplift={uplift_rounded})",
                divergent_slots, pid
            ));
        }
    }

    // Validate every emitted pack resolves in catalog
    for p in &packs {
        let pid = p.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
        if cat.resolve(pid).is_none() {
            return Err(ContractError::new(format!(
                "brain emitted unknown pack_id {pid:?}"
            )));
        }
    }

    let static_pack_ids: Vec<String> = packs
        .iter()
        .filter(|p| p.get("schedule").and_then(|v| v.as_str()) == Some("static"))
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    let dynamic_pack_ids: Vec<String> = packs
        .iter()
        .filter(|p| p.get("schedule").and_then(|v| v.as_str()) == Some("dynamic"))
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    let re_probe_elevated_ids: Vec<String> = packs
        .iter()
        .filter(|p| p.get("re_probe_elevated").and_then(|v| v.as_bool()) == Some(true))
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();

    // Wave membership: dense B47–B79 never sit in core (even if mis-layered).
    // verify_rand always last wave (after commercial material).
    let is_dense = |pid: &str| crate::brain_control::is_dense_pack_id(pid);
    let is_verify = |pid: &str| crate::brain_control::is_verify_rand_pack(pid);
    let core_ids: Vec<String> = packs
        .iter()
        .filter(|p| {
            let pid = p.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
            if is_dense(pid) || is_verify(pid) {
                return false;
            }
            matches!(
                p.get("layer").and_then(|v| v.as_str()),
                Some("lite5") | Some("b8")
            )
        })
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    let mid_ids: Vec<String> = packs
        .iter()
        .filter(|p| {
            let pid = p.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
            if is_dense(pid) || is_verify(pid) {
                return false;
            }
            p.get("layer").and_then(|v| v.as_str()) == Some("mid4")
        })
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    // Hard material (incl. dense hard like B55/B59) — before pure deep census.
    let hard_ids: Vec<String> = packs
        .iter()
        .filter(|p| {
            let pid = p.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
            let layer = p.get("layer").and_then(|v| v.as_str()).unwrap_or("");
            if is_verify(pid) || layer == "verify_rand" {
                return false;
            }
            layer == "hard" || (is_dense(pid) && layer == "hard")
        })
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    let deep_ids: Vec<String> = packs
        .iter()
        .filter(|p| {
            let pid = p.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
            let layer = p.get("layer").and_then(|v| v.as_str()).unwrap_or("");
            if is_verify(pid) || layer == "verify_rand" {
                return false;
            }
            // deep + dense mid/deep residual not already in core/mid/hard
            if hard_ids.iter().any(|h| h == pid) {
                return false;
            }
            if core_ids.iter().any(|c| c == pid) || mid_ids.iter().any(|m| m == pid) {
                return false;
            }
            layer == "deep" || is_dense(pid) || (layer != "lite5" && layer != "b8" && layer != "mid4" && layer != "hard")
        })
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    let verify_ids: Vec<String> = packs
        .iter()
        .filter(|p| {
            let pid = p.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
            let layer = p.get("layer").and_then(|v| v.as_str()).unwrap_or("");
            is_verify(pid) || layer == "verify_rand"
        })
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();

    // Hardware-safe pipeline (product): never schedule 2+ GPU/audio/RTC/sandbox HW packs
    // in the same parallel_group — browsers freeze when B2/B10/B7/R race the main thread.
    // Light packs (lite5/b8/identity) may still race; each hardware pack is a singleton stage.
    // Preserve commercial-before-verify order via build_hw_safe_parallel_groups.
    let all_sched: Vec<String> = packs
        .iter()
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    // Prefer catalog wave order within light: core → mid → hard-light → deep-light.
    // Exclusive resource classes re-staged as class-safe parallel stages (gpu∥audio…).
    let mut ordered: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for bucket in [&core_ids, &mid_ids, &hard_ids, &deep_ids, &verify_ids] {
        for id in bucket {
            if seen.insert(id.clone()) {
                ordered.push(id.clone());
            }
        }
    }
    for id in &all_sched {
        if seen.insert(id.clone()) {
            ordered.push(id.clone());
        }
    }
    let groups = crate::brain_control::build_hw_safe_parallel_groups(&ordered);
    let hw_n = ordered
        .iter()
        .filter(|id| crate::brain_control::is_hardware_probe_pack(id))
        .count();
    let light_n = ordered.len().saturating_sub(hw_n);
    notes.push(format!(
        "parallel_groups: class-safe stages (light_race={} exclusive_slots={} groups={}) — gpu∥audio∥rtc∥cpu; same class serial; multi-source ResourceBus",
        light_n,
        hw_n,
        groups.len()
    ));

    let mut coverage = coverage_checklist(
        evidence,
        &present,
        soft_v2_ready,
        allow_amplify && soft_v2_ready,
    )?;
    if let Some(obj) = coverage.as_object_mut() {
        obj.insert("sandbox_plan".into(), sandbox_plan.clone());
        obj.insert(
            "direction_ranking".into(),
            json!(crate::brain_directions::rank_directions(evidence)),
        );
        obj.insert("re_probe_priority_codes".into(), json!(re_probe_codes));
        obj.insert("re_probe_elevated_packs".into(), json!(re_probe_elevated_ids));
        obj.insert("brain_hypotheses_active".into(), json!(active_hyps));
        obj.insert("hypothesis_elevated_packs".into(), json!(hyp_packs));
        obj.insert("open_verification_gaps".into(), open_gaps_val.clone());
        obj.insert("soft_stack_budget".into(), json!(soft_stack_prio));
    }
    let coverage_complete = coverage
        .get("coverage_complete")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // Completion: brain has no remaining **required** tasks.
    // Product: nearly all B packs must land — B10x / dense obligations / identity
    // are never residual-optional. R is optional only after B floors + ≥1 R.
    // real_band / dh_ alone must NEVER stop probing.
    let r_floor = coverage
        .get("r_spotcheck_complete")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let b_floors = coverage
        .get("static_complete")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        && coverage
            .get("identity_complete")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        && coverage
            .get("mid_complete")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        && coverage
            .get("gateway_complete")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        && coverage
            .get("dense_complete")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let residual_optional = !packs.is_empty()
        && packs.iter().all(|p| {
            let layer = p.get("layer").and_then(|v| v.as_str()).unwrap_or("");
            let schedule = p.get("schedule").and_then(|v| v.as_str()).unwrap_or("");
            let pid = p.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
            // R: optional only when B floors done and ≥1 R already present.
            if crate::brain_control::is_verify_rand_pack(pid) || layer == "verify_rand" {
                return b_floors && r_floor;
            }
            // Dense / B10x / identity cores: never optional while still scheduled.
            if crate::brain_control::is_dense_pack_id(pid) || pid.starts_with("B10x_") {
                return false;
            }
            if pid == "B11_interaction"
                || pid == "B12_anti_camouflage"
                || pid == "B7_sandbox"
                || pid == "B10_hw_curves"
                || pid == "B2_hardware"
                || pid == "B0_bootstrap"
                || pid == "B1_conflict"
                || pid == "B3_system"
                || pid == "B8_gateway"
                || pid == "B8_gateway_early"
                || pid == "B17_hw_physical"
                || pid == "B15_cross_curves"
            {
                return false;
            }
            // Optional: pure residual hard/deep dynamic after required B schedule empty.
            (layer == "hard" || layer == "deep")
                && schedule == "dynamic"
                && !crate::brain_control::is_dense_pack_id(pid)
                && !pid.starts_with("B10x_")
        });
    // Phase B: never finalize while GPU label exists without curves.
    let fields_for_final = evidence
        .get("fields")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let fo_final = fields_for_final.as_object().cloned().unwrap_or_default();
    let has_gpu_label = fo_final
        .get("webgl_unmasked_renderer")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());
    let has_webgl_curve_final = fo_final
        .get("hw_curve_webgl")
        .and_then(|v| v.as_array())
        .is_some_and(|a| a.len() >= 4);
    let _has_residual_final = fo_final.get("residual_mean").and_then(|v| v.as_f64()).is_some()
        || has_webgl_curve_final
        || fo_final
            .get("hw_curve_audio")
            .and_then(|v| v.as_array())
            .is_some_and(|a| a.len() >= 8);
    let _has_host_sep_final = fo_final
        .get("webrtc_host_ip_hash")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty())
        || fo_final
            .get("os_instance_hash")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty());
    let defer_commercial_finalize = has_gpu_label && !has_webgl_curve_final;
    // B3: conf-convergence stop — coverage floors must also satisfy per-axis
    // confidence convergence ∧ cross-source consistency (or qualified missing
    // evidence) ∧ budget headroom. Emitted as conf_sufficiency for product/SDK.
    let conf_suf = conf_sufficiency(evidence, &gaps, &truth);
    let conf_blocks = conf_suf
        .get("verdict")
        .and_then(|v| v.as_str())
        == Some("insufficient");
    // Full schedule: coverage floors + no required packs still queued.
    let stop = coverage_complete
        && (packs.is_empty() || residual_optional)
        && !defer_commercial_finalize
        && !conf_blocks;
    if conf_blocks {
        let open_axes = conf_suf
            .get("open_gap_axes")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        notes.push(
            format!(
                "CONF_GATE: conf_sufficiency insufficient (open axes [{open_axes}] / xsrc {}) — coverage complete but not conf-converged; continue probe",
                conf_suf
                    .get("xsrc")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
            ),
        );
    }
    if defer_commercial_finalize {
        notes.push(
            "host_sep_missing: continue multi-source ICE/OS instance re-probe (score demote, not skip schedule)"
                .into(),
        );
    }
    if stop {
        notes.push(
            "STOP_PROBE: full brain schedule (static+identity+mid+dense+R+multisource) — not real_band/dh terminal"
                .into(),
        );
        if residual_optional && !packs.is_empty() {
            notes.push(
                "residual non-dense hard/deep packs remain after required schedule — optional deepen only"
                    .into(),
            );
        }
    } else if !coverage_complete {
        let miss = [
            ("static", coverage.get("static_complete")),
            ("identity", coverage.get("identity_complete")),
            ("mid", coverage.get("mid_complete")),
            ("dense", coverage.get("dense_complete")),
            ("r_spotcheck", coverage.get("r_spotcheck_complete")),
            ("multi_source", coverage.get("multi_source_complete")),
            ("nest", coverage.get("nest_complete")),
        ]
        .iter()
        .filter(|(_, v)| !v.and_then(|x| x.as_bool()).unwrap_or(false))
        .map(|(k, _)| *k)
        .collect::<Vec<_>>()
        .join(",");
        notes.push(format!(
            "coverage incomplete [{miss}] — continue maximize probe+analyze"
        ));
    } else if !packs.is_empty() {
        notes.push(
            "coverage floors met but required packs still queued this tick — continue schedule"
                .into(),
        );
    }

    Ok(FrontierPlan {
        session_id: sid,
        gaps,
        packs,
        parallel_groups: groups,
        stop_probe: stop,
        soft_v2_ready,
        amplify_allowed: allow_amplify && soft_v2_ready,
        notes,
        plan_version,
        static_pack_ids,
        dynamic_pack_ids,
        coverage,
        conf_sufficiency: conf_suf,
        numeric_verify,
        axis_budget: axis_budget_usage,
    })
}

/// G-ARCH-22: server-derived packs are the only allowlist. Client-proposed pack_ids
/// that are not already in `server_packs` are rejected (cannot expand authority).
pub fn filter_client_proposed_packs(
    server_packs: &[Value],
    client_route: Option<&Value>,
) -> Value {
    let allowed: HashSet<String> = server_packs
        .iter()
        .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    let mut rejected = Vec::new();
    let mut accepted_from_client = Vec::new();
    if let Some(route) = client_route {
        if let Some(arr) = route.get("packs").and_then(|v| v.as_array()) {
            for p in arr {
                let pid = p
                    .get("pack_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if pid.is_empty() {
                    continue;
                }
                if allowed.contains(&pid) {
                    accepted_from_client.push(pid);
                } else {
                    rejected.push(pid);
                }
            }
        }
    }
    let mut allowed_ids: Vec<String> = allowed.into_iter().collect();
    allowed_ids.sort();
    accepted_from_client.sort();
    rejected.sort();
    json!({
        "authority": "server",
        "allowed_pack_ids": allowed_ids,
        "accepted_client_pack_ids": accepted_from_client,
        "rejected_client_pack_ids": rejected,
        "client_cannot_expand_allowlist": true,
    })
}

/// Filter route_plan packs to those not yet kicked/collected (FE multi-tick helper, pure).
/// Packs with `force_recollect=true` are always kept (e.g. B2 present without residual).
pub fn filter_unresolved_packs(plan_packs: &[Value], already: &HashSet<String>) -> Vec<Value> {
    let cat = load_catalog().ok();
    plan_packs
        .iter()
        .filter(|p| {
            // Brain residual re-collect: keep even when batch already present.
            if p.get("force_recollect")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                return true;
            }
            let pid = p.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
            if already.contains(pid) {
                return false;
            }
            if let Some(bid) = p.get("batch_id").and_then(|v| v.as_str()) {
                if already.contains(bid) {
                    return false;
                }
            }
            if let Some(c) = cat.as_ref() {
                if let Some(def) = c.resolve(pid) {
                    if already.contains(&def.batch_id) || already.contains(&def.pack_id) {
                        return false;
                    }
                }
            }
            true
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_kick_without_analyze() {
        let plan = static_kick_plan().expect("static");
        assert_eq!(plan["kick_without_analyze"], true);
        let packs = plan["packs"].as_array().unwrap();
        assert!(packs.len() >= 6, "lite5+b8+behavior+sandbox");
        assert!(packs.iter().all(|p| p["schedule"] == "static"));
        assert!(packs.iter().any(|p| p["pack_id"] == "B8_gateway_early"));
        assert!(packs.iter().any(|p| p["pack_id"] == "B0_bootstrap"));
        assert!(packs.iter().any(|p| p["pack_id"] == "B11_interaction"));
        // Hard commercial anchors must be static-kick (short-visit eligible path).
        assert!(
            packs.iter().any(|p| p["pack_id"] == "B10_hw_curves"),
            "B10_hw_curves must be in static kick for commercial device_id materials"
        );
        let b10 = packs
            .iter()
            .find(|p| p["pack_id"] == "B10_hw_curves")
            .unwrap();
        assert_eq!(b10["hard_eligible"], true);
        // DAG v2 travels with every planned pack for FE scheduling.
        assert!(packs.iter().all(|p| p["dag_v2"].is_object()));
        assert_eq!(b10["dag_v2"]["cost_class"], "heavy");
        assert_eq!(b10["dag_v2"]["deadline_ms"], 90_000);
        assert!(b10["dag_v2"]["engine_profiles"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "blink"));
    }

    #[test]
    fn frontier_route_plan_carries_dag_v2() {
        let evidence = json!({
            "session_id": "s1",
            "sources": ["main"],
            "batches": [{"batch_id": "B0_bootstrap", "source": "main"}],
            "fields": {"user_agent": "Mozilla/5.0", "session_id": "s1"}
        });
        let plan = build_frontier(&evidence, true, None, 40).expect("frontier");
        let route = plan.to_route_plan().expect("route");
        assert_eq!(route["dag_version"], 2);
        let packs = route["packs"].as_array().unwrap();
        if !packs.is_empty() {
            assert!(packs.iter().all(|p| p["dag_v2"].is_object()));
            let b10 = packs
                .iter()
                .find(|p| p["pack_id"] == "B10_hw_curves");
            if let Some(b10) = b10 {
                assert!(b10["dag_v2"]["engine_profiles"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v == "blink"));
            }
        }
    }

    #[test]
    fn coverage_checklist_carries_planes_cover_and_missing_states() {
        // B0-only evidence: S1 observed via bootstrap, everything else not_observed.
        let evidence = json!({
            "session_id": "s1",
            "sources": ["main"],
            "batches": [{"batch_id": "B0_bootstrap", "source": "main"}],
            "fields": {"user_agent": "Mozilla/5.0", "session_id": "s1"}
        });
        let present: HashSet<String> = present_batches(&evidence);
        let cov = coverage_checklist(&evidence, &present, true, true).expect("coverage");
        assert_eq!(cov["planes_cover"]["S1_identity"], "observed", "{cov}");
        assert_eq!(cov["planes_cover"]["S0_edge"], "not_observed", "{cov}");
        assert_eq!(cov["short_visit_min_cover"], true);
        let ms = cov["missing_states"].as_object().unwrap();
        // Missing batch ids map to concrete missing_state + gate.
        assert!(ms.contains_key("B11_interaction"), "{ms:?}");
        assert_eq!(ms["B11_interaction"]["missing_state"], "degraded");
        assert_eq!(ms["B11_interaction"]["gate"], "default");
        // Research lane gaps are not default coverage debt.
        assert!(!ms.contains_key("B18_webgpu"), "B18 must not be default debt: {ms:?}");
        assert!(!ms.contains_key("B47_sab_clock"), "B47 must not be default debt: {ms:?}");
        // Empty evidence: no plane observed; short-visit floor false.
        let ev2 = json!({
            "session_id": "s2",
            "sources": [],
            "batches": [],
            "fields": {"session_id": "s2"}
        });
        let cov2 = coverage_checklist(&ev2, &HashSet::new(), true, true).expect("coverage2");
        assert_eq!(cov2["planes_cover"]["S1_identity"], "not_observed");
        assert_eq!(cov2["short_visit_min_cover"], false);
    }

    #[test]
    fn frontier_emits_catalog_ids_only() {
        let evidence = json!({
            "session_id": "s1",
            "sources": ["main"],
            "batches": [{"batch_id": "B0_bootstrap", "source": "main"}],
            "fields": {
                "user_agent": "Mozilla/5.0",
                "session_id": "s1"
            }
        });
        let plan = build_frontier(&evidence, true, None, 12).expect("frontier");
        let cat = load_catalog().unwrap();
        for p in &plan.packs {
            let pid = p["pack_id"].as_str().unwrap();
            assert!(cat.resolve(pid).is_some(), "unresolved {pid}");
            assert!(p["executable"].as_bool().unwrap_or(false) || cat.resolve(pid).unwrap().executable);
        }
        // Should request gateway / lite fills
        assert!(plan.packs.iter().any(|p| {
            matches!(
                p["pack_id"].as_str(),
                Some("B8_gateway_early") | Some("B1_conflict") | Some("B2_hardware")
            )
        }));
        assert!(plan.packs.iter().any(|p| p["schedule"] == "dynamic") || plan.notes.iter().any(|n| n.contains("amplify") || n.contains("mid")));
    }

    #[test]
    fn hard_anchor_outranks_soft_mid_under_tight_budget() {
        // Thin session: missing almost everything; soft ready so mid would want to schedule.
        let evidence = json!({
            "session_id": "hard_prio",
            "sources": ["main", "gateway"],
            "has_gateway": true,
            "batches": [
                {"batch_id": "B0_bootstrap", "source": "main"},
                {"batch_id": "B8_gateway", "source": "gateway"}
            ],
            "fields": {
                "user_agent": "Mozilla/5.0",
                "form_class": "desktop",
                "session_id": "hard_prio"
            }
        });
        // Tight budget: hard anchors must still appear before soft mid fills the slots.
        let plan = build_frontier(&evidence, true, None, 4).expect("frontier");
        assert!(
            plan.notes.iter().any(|n| n.contains("hard_anchor_priority")),
            "notes should mention hard_anchor_priority: {:?}",
            plan.notes
        );
        let ids: Vec<&str> = plan
            .packs
            .iter()
            .filter_map(|p| p.get("pack_id").and_then(|v| v.as_str()))
            .collect();
        // B10 must be planned when missing (hard commercial curves).
        assert!(
            ids.contains(&"B10_hw_curves"),
            "B10 must be in tight-budget plan, got {ids:?}"
        );
        // Soft mid must not crowd out B10: if any mid4 present, B10 effective_priority higher.
        if let Some(b10) = plan.packs.iter().find(|p| p["pack_id"] == "B10_hw_curves") {
            let b10_ep = b10["effective_priority"].as_i64().unwrap_or(0);
            for p in &plan.packs {
                if p.get("layer").and_then(|v| v.as_str()) == Some("mid4") {
                    let ep = p["effective_priority"].as_i64().unwrap_or(0);
                    assert!(
                        b10_ep > ep,
                        "B10 effective_priority {b10_ep} must beat mid4 {ep}"
                    );
                }
            }
        }
        // First pack should be hard-boosted (gateway or B10/lite hard), not soft mid.
        if let Some(first) = plan.packs.first() {
            let layer = first.get("layer").and_then(|v| v.as_str()).unwrap_or("");
            assert_ne!(layer, "mid4", "first pack under tight budget must not be soft mid4");
        }
    }

    #[test]
    fn mid_dynamic_only_when_soft() {
        let evidence = json!({
            "session_id": "s2",
            "sources": ["main", "gateway"],
            "has_gateway": true,
            "batches": [
                {"batch_id": "B0_bootstrap", "source": "main"},
                {"batch_id": "B1_conflict", "source": "main"},
                {"batch_id": "B2_hardware", "source": "main"},
                {"batch_id": "B3_system", "source": "main"},
                {"batch_id": "B12_anti_camouflage", "source": "main"},
                {"batch_id": "B8_gateway", "source": "gateway"}
            ],
            "fields": {
                "user_agent": "Mozilla/5.0",
                "webdriver": false,
                "webgl_unmasked_renderer": "ANGLE",
                "screen_width": 1920,
                "screen_height": 1080,
                "timezone": "Asia/Shanghai",
                "hardware_concurrency": 8
            }
        });
        let no = build_frontier(&evidence, false, None, 8).unwrap();
        // Soft-gated dynamic mid withheld; static hard B10 may still schedule.
        assert!(
            !no.packs.iter().any(|p| {
                p.get("schedule").and_then(|v| v.as_str()) == Some("dynamic")
                    && p.get("layer").and_then(|v| v.as_str()) == Some("mid4")
            }),
            "dynamic mid4 must be withheld when soft_v2_ready=false; packs={:?}",
            no.packs
        );
        let yes = build_frontier(&evidence, true, None, 8).unwrap();
        assert!(
            yes.packs.iter().any(|p| {
                p.get("schedule").and_then(|v| v.as_str()) == Some("dynamic")
                    && p.get("layer").and_then(|v| v.as_str()) == Some("mid4")
            }) || yes.packs.iter().any(|p| p["pack_id"] == "B10_hw_curves"),
            "soft ready should plan mid dynamic and/or static hard curves; packs={:?}",
            yes.packs
        );
        assert!(
            yes.dynamic_pack_ids
                .iter()
                .any(|id| id.starts_with("B1") && id.as_str() != "B10_hw_curves")
                || yes.packs.iter().any(|p| p["pack_id"] == "B10_hw_curves")
        );
    }

    #[test]
    fn client_cannot_expand_route_allowlist() {
        let server = vec![
            json!({"pack_id": "B0_bootstrap", "priority": 1, "source": "main"}),
            json!({"pack_id": "B10_hw_curves", "priority": 2, "source": "main"}),
        ];
        let client = json!({
            "version": "v5.1",
            "session_id": "s-evil",
            "packs": [
                {"pack_id": "B0_bootstrap", "priority": 1},
                {"pack_id": "B99_client_injected", "priority": 99},
                {"pack_id": "B14_shadow", "priority": 50}
            ],
            "parallel_groups": []
        });
        let auth = filter_client_proposed_packs(&server, Some(&client));
        let rejected = auth["rejected_client_pack_ids"].as_array().unwrap();
        assert!(rejected.iter().any(|v| v == "B99_client_injected"));
        assert!(rejected.iter().any(|v| v == "B14_shadow"));
        assert_eq!(auth["authority"], "server");
        assert_eq!(auth["client_cannot_expand_allowlist"], true);
        let accepted = auth["accepted_client_pack_ids"].as_array().unwrap();
        assert!(accepted.iter().any(|v| v == "B0_bootstrap"));
        assert!(!accepted.iter().any(|v| v == "B99_client_injected"));
    }

    #[test]
    fn parse_axis_budget_calibration_accepts_spec_shape() {
        let raw = json!({
            "classes": {
                "desktop": {"census": 6, "gpu": 4},
                "mobile": {"census": 4, "gpu": 2}
            }
        });
        let rows = parse_axis_budget_calibration(&raw).expect("valid payload");
        let map: HashMap<String, Vec<(String, usize)>> = rows.into_iter().collect();
        assert_eq!(map["desktop"][0], ("census".to_string(), 6));
        assert_eq!(map["mobile"][1], ("gpu".to_string(), 2));
    }

    #[test]
    fn parse_axis_budget_calibration_malformed_none() {
        assert!(parse_axis_budget_calibration(&json!({})).is_none());
        assert!(parse_axis_budget_calibration(&json!({"classes": []})).is_none());
        assert!(parse_axis_budget_calibration(&json!({"classes": {"desktop": {"cpu": "many"}}})).is_none());
    }

    #[test]
    fn axis_budget_resolved_unknown_class_falls_back_to_defaults() {
        // Unknown class → built-in desktop table (same as axis_budget default arm).
        let resolved = axis_budget_resolved("tablet");
        let defaults = axis_budget("tablet");
        assert_eq!(resolved.len(), defaults.len());
        for (a, b) in resolved.iter().zip(defaults.iter()) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn axis_budget_calibration_file_mirrors_builtin_defaults() {
        // SSOT sync guard: spec/axis_budget_calibration.json must mirror the
        // built-in table until the lab lands real-machine numbers. When lab
        // recalibration lands, update THIS test with the new expected values.
        for class in ["desktop", "mobile", "headless"] {
            let resolved = axis_budget_resolved(class);
            let defaults = axis_budget(class);
            // Axis order is irrelevant (per-axis caps are independent); compare
            // as (axis, cap) sets.
            let mut resolved: Vec<(&str, usize)> =
                resolved.iter().map(|(a, c)| (*a, *c)).collect();
            let mut defaults: Vec<(&str, usize)> =
                defaults.iter().map(|(a, c)| (*a, *c)).collect();
            resolved.sort();
            defaults.sort();
            assert_eq!(
                resolved, defaults,
                "{class}: spec/axis_budget_calibration.json drifted from builtin defaults"
            );
        }
    }
}
