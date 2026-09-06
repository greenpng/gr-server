//! Loadable contracts + validators (WP0 / Phase A).

use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ContractError {
    #[error("{0}")]
    Msg(String),
}

impl ContractError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self::Msg(msg.into())
    }
}

/// Resolve `spec/` relative to workspace root (green-v5/), walking up from CWD and crate dirs.
pub fn find_spec_dir() -> PathBuf {
    if let Some(p) = gr_abi::env::get("SPEC_DIR") {
        let pb = PathBuf::from(p);
        if pb.is_dir() {
            return pb;
        }
    }
    // Order: CWD-relative deploy paths first, then build-tree path (dev).
    let candidates = [
        PathBuf::from("spec"),
        PathBuf::from("/opt/green-v5/spec"),
        PathBuf::from("../spec"),
        PathBuf::from("../../spec"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spec"),
    ];
    for c in candidates {
        if c.is_dir() {
            return c
                .canonicalize()
                .unwrap_or_else(|_| c);
        }
    }
    // Last resort: CWD `spec` (error path will mention this)
    PathBuf::from("spec")
}

fn load_json_file(path: &Path) -> Result<Value, ContractError> {
    let text = fs::read_to_string(path)
        .map_err(|e| ContractError::new(format!("missing or unreadable spec {}: {e}", path.display())))?;
    serde_json::from_str(&text)
        .map_err(|e| ContractError::new(format!("invalid json {}: {e}", path.display())))
}

#[derive(Debug, Clone)]
pub struct Specs {
    pub sources: Value,
    pub layers: Value,
    pub real_bands: Value,
    pub packages: Value,
    pub route_plan_schema: Value,
    pub experiment_redlines: Value,
    pub soft_v2: Value,
    pub inject_matrix: Value,
}

impl Specs {
    pub fn load_from(spec_dir: &Path) -> Result<Self, ContractError> {
        Ok(Self {
            sources: load_json_file(&spec_dir.join("sources.json"))?,
            layers: load_json_file(&spec_dir.join("layers_v11.json"))?,
            real_bands: load_json_file(&spec_dir.join("real_bands.json"))?,
            packages: load_json_file(&spec_dir.join("packages.json"))?,
            route_plan_schema: load_json_file(&spec_dir.join("route_plan.schema.json"))?,
            experiment_redlines: load_json_file(&spec_dir.join("experiment_redlines.json"))?,
            soft_v2: load_json_file(&spec_dir.join("soft_v2.json")).unwrap_or_else(|_| {
                json!({
                    "promote_to_commercial_id": false,
                    "p1_window_ms": 300000,
                    "p2_window_ms": 3600000,
                    "hot_bucket_threshold": 32,
                    "legacy_bare_hs2_edges_default": false
                })
            }),
            inject_matrix: load_json_file(&spec_dir.join("inject_matrix.json"))?,
        })
    }

    pub fn server_side(&self) -> HashSet<String> {
        self.sources["server_side"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect()
    }

    pub fn browser_side(&self) -> HashSet<String> {
        self.sources["browser_side"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect()
    }

    pub fn allowed_sources(&self) -> HashSet<String> {
        self.sources["sources"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect()
    }

    pub fn layers_map(&self) -> HashMap<String, Vec<String>> {
        let mut out = HashMap::new();
        if let Some(obj) = self.layers["layers"].as_object() {
            for (k, v) in obj {
                let members: Vec<String> = v
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect();
                out.insert(k.clone(), members);
            }
        }
        out
    }

    pub fn main_core_batches(&self) -> HashSet<String> {
        self.layers["main_core_batches"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect()
    }

    pub fn real_band_list(&self) -> Vec<String> {
        self.real_bands["bands"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect()
    }
}

static SPECS: OnceLock<Result<Specs, String>> = OnceLock::new();

pub fn load_all_specs() -> Result<&'static Specs, ContractError> {
    let r = SPECS.get_or_init(|| {
        let dir = find_spec_dir();
        Specs::load_from(&dir).map_err(|e| e.to_string())
    });
    match r {
        Ok(s) => Ok(s),
        Err(e) => Err(ContractError::new(e.clone())),
    }
}

/// For tests that need a fresh load (not typically needed).
#[cfg(test)]
pub fn load_specs_from(path: &Path) -> Result<Specs, ContractError> {
    Specs::load_from(path)
}

pub fn validate_source(source: &str) -> Result<String, ContractError> {
    let specs = load_all_specs()?;
    let allowed = specs.allowed_sources();
    let base = source.split(':').next().unwrap_or(source);
    if allowed.contains(base) || allowed.contains(source) {
        Ok(source.to_string())
    } else {
        let mut sorted: Vec<_> = allowed.into_iter().collect();
        sorted.sort();
        Err(ContractError::new(format!(
            "invalid source: {source:?}; allowed={sorted:?}"
        )))
    }
}

pub fn coverage_for_batches(batch_ids: &[String]) -> Result<Value, ContractError> {
    let specs = load_all_specs()?;
    let present: HashSet<&str> = batch_ids.iter().map(|s| s.as_str()).collect();
    let layers = specs.layers_map();
    let mut out = Map::new();
    for (name, members) in layers {
        let mset: HashSet<&str> = members.iter().map(|s| s.as_str()).collect();
        let any = present.intersection(&mset).next().is_some();
        let full = !mset.is_empty() && mset.is_subset(&present);
        let mut present_list: Vec<&str> = present.intersection(&mset).copied().collect();
        present_list.sort();
        let mut missing: Vec<&str> = mset.difference(&present).copied().collect();
        missing.sort();
        // State-interpretability (§4.2): each layer reports an observation state —
        // observed (every member), partial (some members), not_observed (no member).
        let state = if full {
            "observed"
        } else if any {
            "partial"
        } else {
            "not_observed"
        };
        out.insert(
            name,
            json!({
                "members": members,
                "any": any,
                "full": full,
                "state": state,
                "present": present_list,
                "missing": missing,
            }),
        );
    }
    Ok(Value::Object(out))
}

pub fn validate_real_band(band: &str) -> Result<String, ContractError> {
    let specs = load_all_specs()?;
    let bands = specs.real_band_list();
    if bands.iter().any(|b| b == band) {
        Ok(band.to_string())
    } else {
        Err(ContractError::new(format!(
            "invalid real_band: {band:?}; allowed={bands:?}"
        )))
    }
}

pub fn validate_route_plan(plan: &Value) -> Result<Value, ContractError> {
    let obj = plan
        .as_object()
        .ok_or_else(|| ContractError::new("route_plan must be object"))?;
    for key in ["version", "session_id", "packs", "parallel_groups"] {
        if !obj.contains_key(key) {
            return Err(ContractError::new(format!(
                "route_plan missing required field: {key}"
            )));
        }
    }
    let sid = obj["session_id"]
        .as_str()
        .ok_or_else(|| ContractError::new("route_plan.session_id must be non-empty string"))?;
    if sid.trim().is_empty() {
        return Err(ContractError::new(
            "route_plan.session_id must be non-empty string",
        ));
    }
    let packs = obj["packs"]
        .as_array()
        .ok_or_else(|| ContractError::new("route_plan.packs must be array"))?;
    for (i, pack) in packs.iter().enumerate() {
        let p = pack
            .as_object()
            .ok_or_else(|| ContractError::new(format!("packs[{i}] must be object")))?;
        let pid = p
            .get("pack_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if pid.is_empty() {
            return Err(ContractError::new(format!("packs[{i}].pack_id required")));
        }
        match p.get("priority") {
            Some(Value::Number(n)) if n.is_i64() || n.is_u64() => {}
            _ => {
                return Err(ContractError::new(format!(
                    "packs[{i}].priority must be int"
                )));
            }
        }
        if let Some(src) = p.get("source").and_then(|v| v.as_str()) {
            validate_source(src)?;
        }
    }
    let groups = obj["parallel_groups"]
        .as_array()
        .ok_or_else(|| ContractError::new("route_plan.parallel_groups must be array"))?;
    for (gi, group) in groups.iter().enumerate() {
        let arr = group.as_array().ok_or_else(|| {
            ContractError::new(format!("parallel_groups[{gi}] must be string array"))
        })?;
        if !arr.iter().all(|x| x.is_string()) {
            return Err(ContractError::new(format!(
                "parallel_groups[{gi}] must be string array"
            )));
        }
    }
    Ok(plan.clone())
}

fn has_any(fields: &Map<String, Value>, keys: &[&str]) -> bool {
    for k in keys {
        if let Some(v) = fields.get(*k) {
            match v {
                Value::Null => continue,
                Value::String(s) if s.is_empty() => continue,
                Value::Object(o) if o.is_empty() => continue,
                Value::Array(a) if a.is_empty() => continue,
                _ => return true,
            }
        }
    }
    false
}

pub fn present_packages(fields: &Value, sources: &HashSet<String>) -> Result<Value, ContractError> {
    let specs = load_all_specs()?;
    let f = fields.as_object().cloned().unwrap_or_default();
    let auto = f
        .get("automation")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    let pkg_auto = has_any(&f, &["webdriver", "playwright", "selenium", "cdc"])
        || ["webdriver", "playwright", "selenium", "cdc"]
            .iter()
            .any(|k| auto.get(*k).is_some_and(|v| !v.is_null()));

    let pkg_gpu = has_any(&f, &["webgl_unmasked_renderer", "webgl_unmasked_vendor"]);

    let geocpu_full = ["screen_width", "screen_height", "timezone", "hardware_concurrency"]
        .iter()
        .all(|k| {
            f.get(*k)
                .is_some_and(|v| !v.is_null() && v.as_str() != Some(""))
        });
    let geocpu_alt = f.get("screen_width").is_some_and(|v| !v.is_null())
        && f.get("timezone").is_some_and(|v| !v.is_null())
        && f.get("hardware_concurrency").is_some_and(|v| !v.is_null());
    let pkg_geocpu = geocpu_full || geocpu_alt;

    let pkg_ua = has_any(&f, &["user_agent", "ua"]);
    let server = sources.intersection(&specs.server_side()).next().is_some();
    let browser = sources.intersection(&specs.browser_side()).next().is_some()
        || sources.contains("main");

    Ok(json!({
        "PKG_UA": pkg_ua,
        "PKG_AUTO": pkg_auto,
        "PKG_GPU": pkg_gpu,
        "PKG_GEOCPU": pkg_geocpu,
        "PKG_LINK": pkg_gpu || pkg_geocpu,
        "PKG_XSRC": server && browser,
    }))
}

/// Collect batch_id strings from evidence.
pub fn batch_ids_from_evidence(evidence: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    if let Some(batches) = evidence.get("batches").and_then(|b| b.as_array()) {
        for b in batches {
            if let Some(id) = b.get("batch_id").and_then(|v| v.as_str()) {
                ids.push(id.to_string());
            } else if let Some(s) = b.as_str() {
                ids.push(s.to_string());
            }
        }
    }
    if let Some(extra) = evidence
        .get("fields")
        .and_then(|f| f.get("_batch_ids"))
        .and_then(|a| a.as_array())
    {
        for x in extra {
            if let Some(s) = x.as_str() {
                ids.push(s.to_string());
            }
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_local_specs() {
        let s = load_all_specs().expect("specs");
        assert!(s.allowed_sources().contains("main"));
        assert!(s.main_core_batches().contains("B0_bootstrap"));
    }
}
