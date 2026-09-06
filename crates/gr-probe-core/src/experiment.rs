//! Experiment / canary strategy apply with constitutional red-lines.

use crate::contracts::{load_all_specs, ContractError};
use serde_json::{json, Map, Value};

#[derive(Debug, Clone)]
pub struct StrategyApplyResult {
    pub ok: bool,
    pub strategy_id: String,
    pub applied: Map<String, Value>,
    pub stripped: Vec<String>,
    pub denied: Vec<String>,
    pub base: Map<String, Value>,
    pub message: String,
}

impl StrategyApplyResult {
    pub fn to_value(&self) -> Value {
        json!({
            "ok": self.ok,
            "strategy_id": self.strategy_id,
            "applied": self.applied,
            "stripped": self.stripped,
            "denied": self.denied,
            "base": self.base,
            "message": self.message,
        })
    }
}

pub fn default_stable_policy() -> Result<Map<String, Value>, ContractError> {
    let specs = load_all_specs()?;
    let stable_id = specs.experiment_redlines["stable_strategy_id"]
        .as_str()
        .unwrap_or("strategy_stable")
        .to_string();
    let m = json!({
        "soft_promote": false,
        "promote_soft_to_hard": false,
        "d2_hard_material_whitelist": [
            "webgl_unmasked_renderer",
            "hardware_concurrency",
            "os_family",
            "screen_width",
            "screen_height",
            "timezone"
        ],
        "hard_material_whitelist": [
            "webgl_unmasked_renderer",
            "hardware_concurrency",
            "os_family"
        ],
        "confirmed_real_gate": {
            "require_server_side": true,
            "require_main_core": true,
            "require_xsrc_not_conflict": true,
            "require_no_strong_bot": true,
            "require_not_fe_only": true
        },
        "fe_only_to_confirmed": false,
        "allow_fe_only_confirmed": false,
        "skip_xsrc_for_confirmed": false,
        "bot_algo": "balanced",
        "link_algo": "sparse_safe",
        "pack_order": ["lite", "gateway", "mid", "deep"],
        "pack_priorities": {"lite": 100, "gateway": 95, "mid": 70, "deep": 40},
        "bot_soft_weights": {"locale": 2, "swiftshader_alone": 10},
        "locale_weight": 2,
        "swiftshader_soft_weight": 10,
        "canary_percent": 0,
        "strategy_id": stable_id,
    });
    Ok(m.as_object().cloned().unwrap())
}

pub fn apply_strategy(
    mutations: Option<&Value>,
    base: Option<Map<String, Value>>,
    strict: bool,
) -> Result<StrategyApplyResult, ContractError> {
    let specs = load_all_specs()?;
    let cfg = &specs.experiment_redlines;
    let denied_keys: Vec<String> = cfg["denied_mutation_keys"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    let allowed_keys: Vec<String> = cfg["allowed_mutation_keys"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    let denied_set: std::collections::HashSet<_> = denied_keys.iter().cloned().collect();
    let allowed_set: std::collections::HashSet<_> = allowed_keys.iter().cloned().collect();

    let mut policy = base.unwrap_or(default_stable_policy()?);
    let mut stripped = Vec::new();
    let mut denied = Vec::new();
    let mut applied_mut = Map::new();

    if let Some(Value::Object(muts)) = mutations {
        for (key, val) in muts {
            if denied_set.contains(key) {
                denied.push(key.clone());
                continue;
            }
            if !allowed_set.contains(key) {
                stripped.push(key.clone());
                continue;
            }
            policy.insert(key.clone(), val.clone());
            applied_mut.insert(key.clone(), val.clone());
        }
    }

    let stable = default_stable_policy()?;
    for key in &denied_keys {
        if let Some(v) = stable.get(key) {
            policy.insert(key.clone(), v.clone());
        }
    }
    policy.insert("bot_algo".into(), json!("balanced"));
    policy.insert("link_algo".into(), json!("sparse_safe"));
    policy.insert("soft_promote".into(), json!(false));
    policy.insert("fe_only_to_confirmed".into(), json!(false));
    policy.insert("allow_fe_only_confirmed".into(), json!(false));
    policy.insert("skip_xsrc_for_confirmed".into(), json!(false));

    let ok = if strict { denied.is_empty() } else { true };
    let sid = policy
        .get("strategy_id")
        .and_then(|v| v.as_str())
        .unwrap_or("strategy_stable")
        .to_string();
    let msg = if ok {
        "ok".into()
    } else {
        format!("denied mutations: {denied:?}")
    };
    Ok(StrategyApplyResult {
        ok,
        strategy_id: sid,
        applied: applied_mut,
        stripped,
        denied,
        base: policy,
        message: msg,
    })
}
