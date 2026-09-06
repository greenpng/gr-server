//! iss/39 R8: JA*/engine population prior as low-weight br aux (file-backed, not live hourly).

use serde_json::{json, Map, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::contracts::{find_spec_dir, ContractError};
use crate::protocol_edge::{brand_to_engine_family, derive_engine_claim_obs};

static PRIOR: OnceLock<Result<Value, String>> = OnceLock::new();

pub fn load_ja_population_prior() -> Result<&'static Value, ContractError> {
    let r = PRIOR.get_or_init(|| {
        let path = PathBuf::from(find_spec_dir()).join("ja_population_prior_v1.json");
        let raw = fs::read_to_string(&path).map_err(|e| format!("read {path:?}: {e}"))?;
        serde_json::from_str(&raw).map_err(|e| format!("parse ja prior: {e}"))
    });
    r.as_ref().map_err(|e| ContractError::Msg(e.clone()))
}

/// Apply rare-family prior bump on br risk (clamped). Never digest.
pub fn apply_ja_population_prior_br(
    fo: &Map<String, Value>,
    risk: &mut f64,
    reasons: &mut Vec<String>,
    hits: &mut Vec<String>,
) {
    let Ok(prior) = load_ja_population_prior() else {
        return;
    };
    let (claim, _obs) = derive_engine_claim_obs(fo);
    let ja = fo
        .get("ja4")
        .or_else(|| fo.get("tls_ja4"))
        .or_else(|| fo.get("protocol_engine"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let fam = if !ja.is_empty() {
        brand_to_engine_family(ja)
    } else {
        claim
    };
    hits.push("ja_population_prior".into());
    let share = prior
        .pointer(&format!("/engine_family_share/{fam}"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.04);
    let thr = prior
        .get("rare_family_threshold")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.08);
    let cap = prior
        .get("weight_cap")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.04);
    if share < thr {
        let bump = ((thr - share) * 0.5).clamp(0.0, cap);
        *risk = (*risk + bump).min(1.0);
        reasons.push(format!("ja_pop_prior_rare_family={fam}/{share:.2}"));
    } else if fam != "unknown" {
        *risk = (*risk - 0.01).max(0.0);
        reasons.push(format!("ja_pop_prior_common_family={fam}"));
    }
}

pub fn ja_population_prior_json() -> Value {
    load_ja_population_prior()
        .cloned()
        .unwrap_or_else(|_| json!({"loaded": false}))
}
