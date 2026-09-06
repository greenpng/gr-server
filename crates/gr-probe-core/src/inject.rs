//! Inject plane single-flight coordination (WP1).

use crate::contracts::{load_all_specs, ContractError};
use serde_json::{json, Value};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum InjectError {
    #[error("{0}")]
    Msg(String),
}

impl InjectError {
    fn new(msg: impl Into<String>) -> Self {
        Self::Msg(msg.into())
    }
}

#[derive(Debug, Clone)]
pub struct InjectConfig {
    pub primary: String,
    pub next_mode: String,
    pub boot_started: bool,
    pub notes: Vec<String>,
}

impl InjectConfig {
    pub fn to_value(&self) -> Value {
        json!({
            "primary": self.primary,
            "next_mode": self.next_mode,
            "boot_started": self.boot_started,
            "notes": self.notes,
        })
    }
}

#[derive(Debug, Clone)]
pub struct InjectPlan {
    pub ok: bool,
    pub primary: String,
    pub actors: Vec<Value>,
    pub errors: Vec<String>,
    pub single_flight_flag: String,
    pub boot_script_loaders: Vec<String>,
    pub cfg_mergers: Vec<String>,
}

impl InjectPlan {
    pub fn to_value(&self) -> Value {
        json!({
            "ok": self.ok,
            "primary": self.primary,
            "actors": self.actors,
            "errors": self.errors,
            "single_flight_flag": self.single_flight_flag,
            "boot_script_loaders": self.boot_script_loaders,
            "cfg_mergers": self.cfg_mergers,
        })
    }
}

fn load_inject_matrix() -> Result<Value, ContractError> {
    Ok(load_all_specs()?.inject_matrix.clone())
}

pub fn validate_inject_config(
    primary: &str,
    next_mode: &str,
    cf_enabled: Option<bool>,
    nginx_enabled: Option<bool>,
) -> Result<InjectConfig, InjectError> {
    let matrix = load_inject_matrix().map_err(|e| InjectError::new(e.to_string()))?;
    let mut allowed: Vec<String> = matrix["primary_options"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    allowed.push("none".into());
    if !allowed.iter().any(|a| a == primary) {
        let mut sorted = allowed;
        sorted.sort();
        return Err(InjectError::new(format!(
            "invalid primary {primary:?}; allowed={sorted:?}"
        )));
    }
    if !matches!(next_mode, "cfg_only" | "full" | "off") {
        return Err(InjectError::new(format!(
            "invalid next_mode {next_mode:?}"
        )));
    }
    let mut notes = Vec::new();
    let cf_enabled = cf_enabled.unwrap_or(primary == "cf_worker");
    let nginx_enabled = nginx_enabled.unwrap_or(primary == "nginx");
    if cf_enabled && nginx_enabled && primary != "none" {
        return Err(InjectError::new(
            "cf_worker and nginx both enabled as injectors — violates at_most_one_primary",
        ));
    }
    if primary == "cf_worker" && !cf_enabled {
        return Err(InjectError::new(
            "primary=cf_worker but cf_enabled=false",
        ));
    }
    if primary == "nginx" && !nginx_enabled {
        return Err(InjectError::new("primary=nginx but nginx_enabled=false"));
    }
    if primary == "none" && next_mode != "full" {
        notes.push("no primary: recommend next_mode=full as fallback".into());
    }
    if next_mode == "full" && matches!(primary, "cf_worker" | "nginx") {
        notes.push(
            "next_mode=full with primary present relies on runtime single-flight; prefer cfg_only"
                .into(),
        );
    }
    Ok(InjectConfig {
        primary: primary.into(),
        next_mode: next_mode.into(),
        boot_started: false,
        notes,
    })
}

pub fn plan_inject(primary: &str, next_mode: &str, boot_started: bool) -> InjectPlan {
    let matrix = match load_inject_matrix() {
        Ok(m) => m,
        Err(e) => {
            return InjectPlan {
                ok: false,
                primary: primary.into(),
                actors: vec![],
                errors: vec![e.to_string()],
                single_flight_flag: "__GR_BOOT_STARTED__".into(),
                boot_script_loaders: vec![],
                cfg_mergers: vec![],
            };
        }
    };
    let flag = matrix["single_flight_flag"]
        .as_str()
        .unwrap_or("__GR_BOOT_STARTED__")
        .to_string();
    let mut errors = Vec::new();
    let mut actors = Vec::new();
    let mut boot_loaders = Vec::new();
    let mut cfg_mergers = Vec::new();

    let cfg = match validate_inject_config(primary, next_mode, None, None) {
        Ok(c) => c,
        Err(e) => {
            return InjectPlan {
                ok: false,
                primary: primary.into(),
                actors: vec![],
                errors: vec![e.to_string()],
                single_flight_flag: flag,
                boot_script_loaders: vec![],
                cfg_mergers: vec![],
            };
        }
    };

    if primary == "cf_worker" {
        actors.push(json!({"role": "cf_worker", "action": "insert_boot", "primary": true}));
        boot_loaders.push("cf_worker".into());
    } else if primary == "nginx" {
        actors.push(json!({"role": "nginx", "action": "insert_boot", "primary": true}));
        boot_loaders.push("nginx".into());
    }

    if next_mode == "cfg_only" {
        actors.push(json!({"role": "next", "action": "merge_cfg_only", "primary": false}));
        cfg_mergers.push("next".into());
    } else if next_mode == "full" {
        if boot_started {
            actors.push(json!({
                "role": "next",
                "action": "skip_boot_already_started",
                "primary": false
            }));
            cfg_mergers.push("next".into());
        } else {
            actors.push(json!({"role": "next", "action": "load_boot_fallback", "primary": false}));
            boot_loaders.push("next".into());
        }
    }

    if boot_loaders.len() > 1 {
        errors.push(format!("multiple boot loaders planned: {boot_loaders:?}"));
    }
    if boot_started && boot_loaders.iter().any(|b| b == "next") {
        errors.push("next would load boot but flag already started".into());
    }

    InjectPlan {
        ok: errors.is_empty(),
        primary: cfg.primary,
        actors,
        errors,
        single_flight_flag: flag,
        boot_script_loaders: boot_loaders,
        cfg_mergers,
    }
}

pub fn simulate_document_boot(primary: &str, next_mode: &str) -> Value {
    let mut boot_runs = 0i32;
    let mut flag = false;
    let mut events = Vec::new();

    if matches!(primary, "cf_worker" | "nginx") {
        if !flag {
            boot_runs += 1;
            flag = true;
            events.push(format!("{primary}:boot_start"));
        } else {
            events.push(format!("{primary}:skip"));
        }
    }
    if next_mode == "cfg_only" {
        events.push("next:merge_cfg".into());
    } else if next_mode == "full" {
        if !flag {
            boot_runs += 1;
            flag = true;
            events.push("next:boot_fallback".into());
        } else {
            events.push("next:skip_boot".into());
        }
    }
    json!({
        "boot_runs": boot_runs,
        "boot_started": flag,
        "events": events,
        "ok": boot_runs <= 1,
        "single_flight": boot_runs <= 1,
    })
}

pub fn inject_deploy_gate(
    primary: &str,
    next_mode: &str,
    cf_enabled: Option<bool>,
    nginx_enabled: Option<bool>,
) -> Value {
    let mut errors = Vec::new();
    let mut cfg_dict = None;
    match validate_inject_config(primary, next_mode, cf_enabled, nginx_enabled) {
        Ok(cfg) => cfg_dict = Some(cfg.to_value()),
        Err(e) => errors.push(e.to_string()),
    }
    let plan = plan_inject(primary, next_mode, false);
    if !plan.ok {
        errors.extend(plan.errors.clone());
    }
    if cf_enabled == Some(true) && nginx_enabled == Some(true) {
        let msg = "dual PRIMARY: cf_worker+nginx both enabled";
        if !errors.iter().any(|e| e.contains("both enabled")) {
            errors.push(msg.into());
        }
    }
    let sim_primary = if primary == "none" { "none" } else { primary };
    let sim = simulate_document_boot(sim_primary, next_mode);
    if !sim
        .get("single_flight")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        errors.push(format!(
            "single_flight violated: boot_runs={}",
            sim.get("boot_runs").and_then(|v| v.as_i64()).unwrap_or(-1)
        ));
    }
    let matrix = load_inject_matrix().unwrap_or(json!({}));
    json!({
        "ok": errors.is_empty(),
        "errors": errors,
        "config": cfg_dict,
        "plan": plan.to_value(),
        "simulate": sim,
        "rules": matrix.get("rules"),
        "single_flight_flag": matrix.get("single_flight_flag"),
        "cf_worker_skeleton": "edge/cf-worker/worker.js",
        "note": "CF Worker PRIMARY is mutually exclusive with nginx PRIMARY",
    })
}
