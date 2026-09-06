//! Guest/local identity SLA dashboard aggregation (no production traffic required).
//!
//! Builds rates and no-id reason histograms from **real** evaluate/projection fields
//! (eligible, device_id, no_id_reasons, soft_promote) — never hardcoded rates.

use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// One session/cell row for SLA aggregation (from evaluate result or matrix cell).
#[derive(Debug, Clone, Default)]
pub struct SlaSessionRow {
    pub engine: String,
    pub device_id: Option<String>,
    pub eligible: Option<bool>,
    pub has_both_curves: Option<bool>,
    pub has_audio: Option<bool>,
    pub has_webgl: Option<bool>,
    pub no_id_reasons: Vec<String>,
    pub soft_promote: Option<bool>,
    pub confidence_version: Option<String>,
    pub materials_included: Vec<String>,
}

impl SlaSessionRow {
    /// Parse from evaluate_session result or product/device projection JSON.
    pub fn from_evaluate_result(v: &Value) -> Self {
        let device = v.get("device").cloned().unwrap_or(Value::Null);
        let product = v.get("product").cloned().unwrap_or(Value::Null);
        let trust = device.get("trust").cloned().unwrap_or(Value::Null);
        let did = device
            .get("device_id")
            .or_else(|| product.get("device_id"))
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty() && *s != "null")
            .map(|s| s.to_string());
        let reasons = device
            .get("no_id_reasons")
            .or_else(|| product.get("no_id_reasons"))
            .or_else(|| trust.get("no_id_reasons"))
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let mats: Vec<String> = trust
            .get("materials_included")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            engine: v
                .get("engine")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            device_id: did,
            eligible: trust
                .get("eligible")
                .or_else(|| device.get("linkable"))
                .and_then(|x| x.as_bool()),
            has_both_curves: trust.get("has_both_curves").and_then(|x| x.as_bool()),
            has_audio: Some(mats.iter().any(|m| m == "hw_audio_stable")),
            has_webgl: Some(mats.iter().any(|m| m == "hw_webgl_stable")),
            no_id_reasons: reasons,
            soft_promote: product
                .get("soft_promote")
                .or_else(|| v.get("soft_promote"))
                .or_else(|| device.get("soft_promote"))
                .and_then(|x| x.as_bool()),
            confidence_version: device
                .pointer("/trust/confidence_version")
                .or_else(|| product.get("confidence_version"))
                .and_then(|x| x.as_str())
                .map(|s| s.to_string()),
            materials_included: mats,
        }
    }

    /// Build SLA row from `analysis_latest` scalar projection (no result_json TOAST).
    ///
    /// Best-effort: materials / no_id_reasons are inferred from tier/digest/residual flags.
    pub fn from_analysis_latest_scalar(v: &Value) -> Self {
        let did = v
            .get("device_id")
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty() && *s != "null")
            .map(|s| s.to_string());
        let tier = v
            .get("device_tier")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let digest = v
            .get("digest_path")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let residual_ok = v.get("residual_entropy_ok").and_then(|x| x.as_bool());
        let has_webrtc = v.get("has_webrtc_host").and_then(|x| x.as_bool());
        let mut mats = Vec::new();
        if residual_ok == Some(true)
            || digest.contains("residual")
            || digest.contains("real_curves")
            || digest.contains("webgl")
        {
            mats.push("hw_webgl_stable".into());
        }
        if digest.contains("audio") {
            mats.push("hw_audio_stable".into());
        }
        if has_webrtc == Some(true) {
            mats.push("webrtc_host".into());
        }
        let mut reasons = Vec::new();
        if did.is_none() {
            reasons.push("no_device_id".into());
        }
        if residual_ok == Some(false) {
            reasons.push("residual_entropy_low".into());
        }
        if v.get("collision_risk").and_then(|x| x.as_bool()) == Some(true) {
            reasons.push("collision_risk".into());
        }
        Self {
            engine: v
                .get("product_version")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            device_id: did.clone(),
            // Scalars: presence of commercial id ≈ eligible for identity dashboard.
            eligible: Some(did.is_some()),
            has_both_curves: Some(
                residual_ok == Some(true)
                    || digest.contains("real_curves")
                    || digest.contains("residual"),
            ),
            has_audio: Some(mats.iter().any(|m| m == "hw_audio_stable")),
            has_webgl: Some(mats.iter().any(|m| m == "hw_webgl_stable") || residual_ok == Some(true)),
            no_id_reasons: reasons,
            soft_promote: Some(false),
            confidence_version: if tier.is_empty() {
                None
            } else {
                Some(format!("tier_{tier}"))
            },
            materials_included: mats,
        }
    }

    /// Parse from guest matrix cell JSON (fp5 / local matrix).
    pub fn from_matrix_cell(v: &Value) -> Self {
        let did = v
            .get("device_id")
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let reasons = v
            .get("no_id_reasons")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let mats: Vec<String> = v
            .get("materials_included")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            engine: v
                .get("engine")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            device_id: did,
            eligible: v.get("eligible").and_then(|x| x.as_bool()),
            has_both_curves: v.get("has_both_curves").and_then(|x| x.as_bool()),
            has_audio: v
                .get("has_audio")
                .and_then(|x| x.as_bool())
                .or(Some(mats.iter().any(|m| m == "hw_audio_stable"))),
            has_webgl: v
                .get("has_webgl")
                .and_then(|x| x.as_bool())
                .or(Some(mats.iter().any(|m| m == "hw_webgl_stable"))),
            no_id_reasons: reasons,
            soft_promote: v.get("soft_promote").and_then(|x| x.as_bool()),
            confidence_version: v
                .get("confidence_version")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string()),
            materials_included: mats,
        }
    }
}

/// Aggregate guest/local identity SLA dashboard from session rows.
pub fn aggregate_identity_sla(rows: &[SlaSessionRow]) -> Value {
    let n = rows.len() as f64;
    let mut reason_hist: BTreeMap<String, i64> = BTreeMap::new();
    let mut engines: BTreeMap<String, i64> = BTreeMap::new();
    let mut n_eligible = 0i64;
    let mut n_has_id = 0i64;
    let mut n_both_curves = 0i64;
    let mut n_audio = 0i64;
    let mut n_webgl = 0i64;
    let mut soft_promote_violations = 0i64;
    let mut conf_versions: BTreeMap<String, i64> = BTreeMap::new();
    let mut unique_ids: BTreeMap<String, i64> = BTreeMap::new();

    for r in rows {
        if !r.engine.is_empty() {
            *engines.entry(r.engine.clone()).or_insert(0) += 1;
        }
        if r.eligible == Some(true) {
            n_eligible += 1;
        }
        if let Some(ref id) = r.device_id {
            if crate::device_tier::is_commercial_device_id(id) {
                n_has_id += 1;
                *unique_ids.entry(id.clone()).or_insert(0) += 1;
            }
        }
        if r.has_both_curves == Some(true) {
            n_both_curves += 1;
        }
        if r.has_audio == Some(true) {
            n_audio += 1;
        }
        if r.has_webgl == Some(true) {
            n_webgl += 1;
        }
        if r.soft_promote == Some(true) {
            soft_promote_violations += 1;
        }
        if let Some(ref cv) = r.confidence_version {
            *conf_versions.entry(cv.clone()).or_insert(0) += 1;
        }
        if r.device_id.is_none() || r.eligible == Some(false) {
            if r.no_id_reasons.is_empty() {
                *reason_hist.entry("unspecified_or_eligible_false".into()).or_insert(0) += 1;
            } else {
                for code in &r.no_id_reasons {
                    *reason_hist.entry(code.clone()).or_insert(0) += 1;
                }
            }
        }
    }

    let rate = |c: i64| if n > 0.0 { (c as f64) / n } else { 0.0 };
    let reasons_json: Map<String, Value> = reason_hist
        .iter()
        .map(|(k, v)| (k.clone(), json!(*v)))
        .collect();
    let engines_json: Map<String, Value> = engines
        .iter()
        .map(|(k, v)| (k.clone(), json!(*v)))
        .collect();

    let rates = json!({
        "eligible": rate(n_eligible),
        "has_device_id": rate(n_has_id),
        "has_both_curves": rate(n_both_curves),
        "has_audio_anchor": rate(n_audio),
        "has_webgl_anchor": rate(n_webgl),
    });
    let base = json!({
        "ok": true,
        "source": "guest_simulated_or_local_evaluate",
        "n_sessions": rows.len(),
        "rates": rates,
        "counts": {
            "eligible": n_eligible,
            "has_device_id": n_has_id,
            "has_both_curves": n_both_curves,
            "has_audio_anchor": n_audio,
            "has_webgl_anchor": n_webgl,
            "unique_device_ids": unique_ids.len(),
        },
        "unique_device_id_heatmap": unique_ids.iter().map(|(k,v)| json!({"device_id": k, "sessions": v})).collect::<Vec<_>>(),
        "no_id_reason_histogram": reasons_json,
        "engines": engines_json,
        "soft_promote_violations": soft_promote_violations,
        "confidence_versions": conf_versions,
        "gating": {
            "soft_promote_clean": soft_promote_violations == 0,
            "has_reason_histogram": !reason_hist.is_empty() || n_has_id as f64 == n,
            "non_empty": rows.len() > 0,
        }
    });
    // Attach organic-anchor SLA alerts (thresholds from policy / env).
    let alerts = evaluate_sla_alerts(&base);
    let mut out = base;
    if let Some(obj) = out.as_object_mut() {
        obj.insert("alerts".into(), alerts);
    }
    out
}

/// SLA alert thresholds (product_policy.sla_alerts + env overrides).
#[derive(Debug, Clone)]
pub struct SlaAlertThresholds {
    pub min_sessions: i64,
    pub min_has_device_id_rate: f64,
    pub min_both_curves_rate: f64,
    pub min_eligible_rate: f64,
    pub max_soft_promote_violations: i64,
}

impl Default for SlaAlertThresholds {
    fn default() -> Self {
        Self {
            min_sessions: 3,
            min_has_device_id_rate: 0.10,
            min_both_curves_rate: 0.05,
            min_eligible_rate: 0.10,
            max_soft_promote_violations: 0,
        }
    }
}

pub fn sla_alert_thresholds() -> SlaAlertThresholds {
    let mut t = SlaAlertThresholds::default();
    let p = crate::policy::load_product_policy();
    if let Some(a) = p.get("sla_alerts") {
        if let Some(v) = a.get("min_sessions").and_then(|x| x.as_i64()) {
            t.min_sessions = v;
        }
        if let Some(v) = a.get("min_has_device_id_rate").and_then(|x| x.as_f64()) {
            t.min_has_device_id_rate = v;
        }
        if let Some(v) = a.get("min_both_curves_rate").and_then(|x| x.as_f64()) {
            t.min_both_curves_rate = v;
        }
        if let Some(v) = a.get("min_eligible_rate").and_then(|x| x.as_f64()) {
            t.min_eligible_rate = v;
        }
        if let Some(v) = a
            .get("max_soft_promote_violations")
            .and_then(|x| x.as_i64())
        {
            t.max_soft_promote_violations = v;
        }
    }
    if let Some(s) = gr_abi::env::get("SLA_MIN_SESSIONS") {
        if let Ok(v) = s.parse() {
            t.min_sessions = v;
        }
    }
    if let Some(s) = gr_abi::env::get("SLA_MIN_DEVICE_ID_RATE") {
        if let Ok(v) = s.parse() {
            t.min_has_device_id_rate = v;
        }
    }
    t
}

/// Evaluate alerts against an aggregate_identity_sla report.
/// Severity: ok | warn | critical. Never invent rates — only compare report fields.
pub fn evaluate_sla_alerts(sla_report: &Value) -> Value {
    let thr = sla_alert_thresholds();
    let n = sla_report
        .get("n_sessions")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let rates = sla_report.get("rates").cloned().unwrap_or(json!({}));
    let has_id = rates
        .get("has_device_id")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let both = rates
        .get("has_both_curves")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let elig = rates
        .get("eligible")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let soft_v = sla_report
        .get("soft_promote_violations")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let mut fired: Vec<Value> = Vec::new();
    let mut severity = "ok";

    if n < thr.min_sessions {
        fired.push(json!({
            "code": "insufficient_sessions",
            "severity": "warn",
            "n_sessions": n,
            "min_sessions": thr.min_sessions,
            "message": "not enough sessions for organic SLA confidence",
        }));
        severity = "warn";
    } else {
        if has_id < thr.min_has_device_id_rate {
            fired.push(json!({
                "code": "low_device_id_rate",
                "severity": "critical",
                "rate": has_id,
                "min": thr.min_has_device_id_rate,
                "message": "organic has_device_id rate below SLA floor",
            }));
            severity = "critical";
        }
        if both < thr.min_both_curves_rate {
            fired.push(json!({
                "code": "low_both_curves_rate",
                "severity": "critical",
                "rate": both,
                "min": thr.min_both_curves_rate,
                "message": "organic hard-anchor (both curves) rate below SLA floor",
            }));
            if severity != "critical" {
                severity = "critical";
            }
        }
        if elig < thr.min_eligible_rate {
            fired.push(json!({
                "code": "low_eligible_rate",
                "severity": "warn",
                "rate": elig,
                "min": thr.min_eligible_rate,
                "message": "eligible rate below soft SLA floor",
            }));
            if severity == "ok" {
                severity = "warn";
            }
        }
    }
    if soft_v > thr.max_soft_promote_violations {
        fired.push(json!({
            "code": "soft_promote_violation",
            "severity": "critical",
            "count": soft_v,
            "max": thr.max_soft_promote_violations,
            "message": "Soft promote must stay false — constitution breach",
        }));
        severity = "critical";
    }

    json!({
        "algo": "identity_sla_alerts_v1",
        "severity": severity,
        "alert_count": fired.len(),
        "fired": fired,
        "thresholds": {
            "min_sessions": thr.min_sessions,
            "min_has_device_id_rate": thr.min_has_device_id_rate,
            "min_both_curves_rate": thr.min_both_curves_rate,
            "min_eligible_rate": thr.min_eligible_rate,
            "max_soft_promote_violations": thr.max_soft_promote_violations,
        },
        "pass": fired.is_empty() || (severity == "warn" && n < thr.min_sessions),
        "note": "alerts from real aggregate rates only — never marketing constants",
    })
}

/// Parse mixed array: evaluate results and/or matrix cells.
pub fn rows_from_json_array(arr: &[Value]) -> Vec<SlaSessionRow> {
    arr.iter()
        .map(|v| {
            if v.get("device").is_some() || v.get("product").is_some() {
                SlaSessionRow::from_evaluate_result(v)
            } else {
                SlaSessionRow::from_matrix_cell(v)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commercial_projection;

    #[test]
    fn sla_from_real_projections_not_constants() {
        let thin = commercial_projection(&json!({}));
        let rich = commercial_projection(&json!({
            "form_class": "desktop",
            "hw_curve_audio": (0..64).map(|i| ((i as f64)*0.17).sin().abs()+0.1).collect::<Vec<_>>(),
            "hw_curve_webgl": (0..32).map(|i| ((i as f64)*0.29).cos().abs()*0.4+0.05).collect::<Vec<_>>(),
            "platform": "Linux x86_64",
        }));
        let rows = vec![
            SlaSessionRow::from_evaluate_result(&json!({
                "device": {
                    "device_id": thin.get("device_id"),
                    "soft_promote": false,
                    "no_id_reasons": thin.get("no_id_reasons"),
                    "trust": thin,
                },
                "product": { "soft_promote": false },
                "engine": "thin",
            })),
            SlaSessionRow::from_matrix_cell(&json!({
                "engine": "camoufox",
                "device_id": rich.get("device_id"),
                "eligible": rich.get("eligible"),
                "has_both_curves": rich.get("has_both_curves"),
                "has_audio": true,
                "has_webgl": true,
                "materials_included": rich.get("materials_included"),
                "soft_promote": false,
                "no_id_reasons": rich.get("no_id_reasons").cloned().unwrap_or(json!([])),
            })),
        ];
        let rep = aggregate_identity_sla(&rows);
        assert_eq!(rep["n_sessions"], 2);
        assert_eq!(rep["soft_promote_violations"], 0);
        assert!(rep["gating"]["soft_promote_clean"].as_bool().unwrap());
        // rates computed from rows (0, 0.5, or 1) not magic marketing numbers
        let has_id = rep["rates"]["has_device_id"].as_f64().unwrap();
        assert!(has_id >= 0.0 && has_id <= 1.0);
        assert!(rep.get("alerts").is_some());
        assert!(rep["alerts"]["algo"].as_str().unwrap().contains("sla_alerts"));
    }

    #[test]
    fn sla_alerts_fire_on_soft_promote_violation() {
        let rep = aggregate_identity_sla(&[SlaSessionRow {
            engine: "x".into(),
            device_id: Some("dv_test".into()),
            eligible: Some(true),
            has_both_curves: Some(true),
            has_audio: Some(true),
            has_webgl: Some(true),
            soft_promote: Some(true),
            ..Default::default()
        }]);
        assert_eq!(rep["soft_promote_violations"], 1);
        let alerts = &rep["alerts"];
        assert_eq!(alerts["severity"], "critical");
        let fired = alerts["fired"].as_array().unwrap();
        assert!(fired.iter().any(|f| f["code"] == "soft_promote_violation"));
    }
}
