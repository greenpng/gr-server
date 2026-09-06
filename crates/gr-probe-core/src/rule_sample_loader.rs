//! iss/38 P2-3: relationship rule samples loadable without baking into score_* bodies.
//! Shadow mode: emit reason tags; never touch commercial digest.

use serde::Deserialize;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::contracts::{find_spec_dir, ContractError};

#[derive(Debug, Clone, Deserialize)]
pub struct RuleSample {
    pub id: String,
    #[serde(default)]
    pub when: Vec<RulePred>,
    #[serde(default)]
    pub then_reasons: Vec<String>,
    #[serde(default)]
    pub axis: String,
    #[serde(default = "default_weight")]
    pub weight: f64,
}

fn default_weight() -> f64 {
    0.05
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum RulePred {
    FieldTrue { field: String },
    FieldFalse { field: String },
    FieldEq { field: String, value: String },
    FieldPresent { field: String },
    FieldMissing { field: String },
    And { preds: Vec<RulePred> },
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuleSamplePack {
    pub version: String,
    #[serde(default)]
    pub mode: String,
    pub rules: Vec<RuleSample>,
}

static PACK: OnceLock<Result<RuleSamplePack, String>> = OnceLock::new();

pub fn load_rule_sample_pack() -> Result<&'static RuleSamplePack, ContractError> {
    let r = PACK.get_or_init(|| {
        let dir = find_spec_dir();
        let path = PathBuf::from(&dir).join("rule_samples/v1/core_relations.json");
        let raw = fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        serde_json::from_str(&raw).map_err(|e| format!("parse rule samples: {e}"))
    });
    r.as_ref().map_err(|e| ContractError::Msg(e.clone()))
}

fn pred_ok(fo: &serde_json::Map<String, Value>, p: &RulePred) -> bool {
    match p {
        RulePred::FieldTrue { field } => fo.get(field).and_then(|v| v.as_bool()) == Some(true),
        RulePred::FieldFalse { field } => fo.get(field).and_then(|v| v.as_bool()) == Some(false),
        RulePred::FieldEq { field, value } => {
            if let Some(v) = fo.get(field) {
                if let Some(s) = v.as_str() {
                    return s == value;
                }
                if let Some(n) = v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)) {
                    if let Ok(expect) = value.parse::<f64>() {
                        return (n - expect).abs() < 1e-9;
                    }
                }
                if let Some(b) = v.as_bool() {
                    return (b && value == "true") || (!b && value == "false");
                }
            }
            false
        }
        RulePred::FieldPresent { field } => fo.get(field).is_some_and(|v| !v.is_null()),
        RulePred::FieldMissing { field } => !fo.contains_key(field) || fo.get(field).map(|v| v.is_null()).unwrap_or(true),
        RulePred::And { preds } => preds.iter().all(|x| pred_ok(fo, x)),
    }
}

/// Evaluate pack in shadow mode: returns matched rule ids + reasons (no score mutation here).
pub fn evaluate_rule_samples_shadow(fields: &Value) -> Value {
    let Ok(pack) = load_rule_sample_pack() else {
        return json!({
            "algo": "rule_sample_shadow_v1",
            "loaded": false,
            "matched": [],
            "reasons": [],
        });
    };
    let fo = fields.as_object().cloned().unwrap_or_default();
    let mut matched = Vec::new();
    let mut reasons = Vec::new();
    for rule in &pack.rules {
        let ok = if rule.when.is_empty() {
            false
        } else {
            rule.when.iter().all(|p| pred_ok(&fo, p))
        };
        if ok {
            matched.push(json!({
                "id": rule.id,
                "axis": rule.axis,
                "weight": rule.weight,
            }));
            for r in &rule.then_reasons {
                if !reasons.iter().any(|x: &String| x == r) {
                    reasons.push(r.clone());
                }
            }
        }
    }
    json!({
        "algo": "rule_sample_shadow_v1",
        "loaded": true,
        "mode": pack.mode,
        "version": pack.version,
        "matched": matched,
        "reasons": reasons,
        "note": "shadow only — does not rewrite commercial digest",
    })
}

/// iss/38 N3 / D-13: apply matched rule samples as **low-weight** risk on one axis.
/// Never touches commercial digest materials. Clamped total influence.
pub fn apply_rule_samples_axis(
    fields: &Value,
    axis: &str,
    risk: &mut f64,
    reasons: &mut Vec<String>,
    hits: &mut Vec<String>,
) {
    let Ok(pack) = load_rule_sample_pack() else {
        return;
    };
    let fo = fields.as_object().cloned().unwrap_or_default();
    let mut delta = 0.0_f64;
    let mut matched_n = 0u32;
    for rule in &pack.rules {
        if !rule.axis.is_empty() && rule.axis != axis && rule.axis != "*" {
            continue;
        }
        let ok = !rule.when.is_empty() && rule.when.iter().all(|p| pred_ok(&fo, p));
        if !ok {
            continue;
        }
        matched_n += 1;
        hits.push(format!("rule_sample:{}", rule.id));
        for r in &rule.then_reasons {
            if !reasons.iter().any(|x| x == r) {
                reasons.push(r.clone());
            }
        }
        // Weight is risk bump (higher = worse env/br/rpa); clamp per-rule
        delta += rule.weight.clamp(0.0, 0.12);
    }
    if matched_n > 0 {
        delta = delta.clamp(0.0, 0.18);
        *risk = (*risk + delta).min(1.0);
        reasons.push(format!("rule_sample_active_delta={delta:.2}"));
        reasons.push(format!("rule_sample_matched_n={matched_n}"));
    }
}
