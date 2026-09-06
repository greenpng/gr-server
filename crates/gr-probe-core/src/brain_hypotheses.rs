//! iss/38 P1-1: adversary / env hypotheses → pack elevation (alongside re_probe_priority).
//!
//! Hypotheses are relation-driven (tags + field presence), not brand catalogs.

use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Hypothesis codes used in battle notes / coverage.
pub const H_VM: &str = "H_vm";
pub const H_EMU: &str = "H_emu";
pub const H_AD: &str = "H_ad";
pub const H_CDP: &str = "H_cdp";
pub const H_REAL: &str = "H_real";

/// Packs to elevate when a hypothesis is active.
pub fn packs_for_hypothesis(h: &str) -> &'static [&'static str] {
    match h {
        H_VM => &[
            "B22_gpu_timer",
            "B30_gpu_ns",
            "B18_gpu_bandwidth",
            "B2_hardware",
            "B10_hw_curves",
            "B3_system",
        ],
        H_EMU => &[
            "B4_mobile",
            "B29_sensors_battery",
            "B2_hardware",
            "B12_anti_camouflage",
            "B3_system",
        ],
        H_AD => &[
            "B12_anti_camouflage",
            "B24_cross_check",
            "B33_caps_pressure",
            "B10_hw_curves",
            "B26_agent_parity",
        ],
        H_CDP => &[
            "B12_anti_camouflage",
            "B26_agent_parity",
            "B6_risk",
            "B11_interaction",
        ],
        H_REAL => &[
            "B10_hw_curves",
            "B15_cross_curves",
            "B17_hw_physical",
            "B2_hardware",
        ],
        _ => &[],
    }
}

fn flag_true(fo: &serde_json::Map<String, Value>, k: &str) -> bool {
    fo.get(k)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || fo
            .get(k)
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty() && s != "false" && s != "0")
}

fn nonempty(fo: &serde_json::Map<String, Value>, k: &str) -> bool {
    fo.get(k).is_some_and(|v| match v {
        Value::Null => false,
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        _ => true,
    })
}

/// Activate hypotheses from fields + optional belief tags.
pub fn activate_hypotheses(fields: &Value, belief_tags: &[String]) -> Vec<String> {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let mut out: Vec<String> = Vec::new();

    let soft = flag_true(&fo, "soft_stack")
        || flag_true(&fo, "residual_soft_like")
        || matches!(
            fo.get("stack_class").and_then(|v| v.as_str()).unwrap_or(""),
            "virt" | "vm" | "cloud" | "soft_render" | "software"
        );
    if soft {
        out.push(H_VM.into());
    }

    let emu = flag_true(&fo, "emulator_hint")
        || belief_tags.iter().any(|t| t.contains("emulator") || t.contains("H_emu"));
    let mobile_ua = fo
        .get("user_agent")
        .and_then(|v| v.as_str())
        .map(|u| {
            let l = u.to_ascii_lowercase();
            l.contains("mobile") || l.contains("android") || l.contains("iphone")
        })
        .unwrap_or(false)
        || flag_true(&fo, "mobile_ua_claim");
    if emu || (mobile_ua && soft) {
        out.push(H_EMU.into());
    }

    if flag_true(&fo, "antidetect_vendor_hint")
        || flag_true(&fo, "fingerprint_vendor_lie")
        || flag_true(&fo, "prototype_chain_tamper")
        || belief_tags
            .iter()
            .any(|t| t.contains("antidetect") || t.contains("H_ad"))
    {
        out.push(H_AD.into());
    }

    let cdp_n = fo
        .get("cdp_runtime_hint")
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
        .unwrap_or(0.0);
    let cdp_hit = cdp_n >= 1.0 || flag_true(&fo, "cdp_runtime_hint");
    let wd = flag_true(&fo, "webdriver");
    if cdp_hit || wd || !fo.contains_key("cdp_runtime_hint") {
        out.push(H_CDP.into());
    }

    let has_curves = nonempty(&fo, "hw_curve_webgl") || nonempty(&fo, "hw_curve_audio");
    if has_curves && !soft && !emu {
        out.push(H_REAL.into());
    }

    // Dedup preserve order
    let mut seen = HashSet::new();
    out.retain(|h| seen.insert(h.clone()));
    out
}

/// Union of packs for active hypotheses.
pub fn elevated_packs_for_fields(fields: &Value, belief_tags: &[String]) -> (Vec<String>, Vec<String>) {
    let hyps = activate_hypotheses(fields, belief_tags);
    let mut packs = HashSet::new();
    for h in &hyps {
        for p in packs_for_hypothesis(h) {
            packs.insert((*p).to_string());
        }
    }
    let mut pack_list: Vec<String> = packs.into_iter().collect();
    pack_list.sort();
    (hyps, pack_list)
}

/// Coverage snippet for frontier notes.
pub fn hypothesis_coverage_json(fields: &Value, belief_tags: &[String]) -> Value {
    let (hyps, packs) = elevated_packs_for_fields(fields, belief_tags);
    let mut by: HashMap<String, Vec<String>> = HashMap::new();
    for h in &hyps {
        by.insert(
            h.clone(),
            packs_for_hypothesis(h)
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        );
    }
    serde_json::json!({
        "algo": "brain_hypotheses_v1",
        "active": hyps,
        "elevated_packs": packs,
        "by_hypothesis": by,
    })
}
