//! F6 — Lane-S engine-family normalization (iss/54).
//!
//! ULP / advanced silicon curves are engine-sensitive. Before digest/LSH on Lane-S,
//! apply affine scale/bias per engine family (from `spec/lane_s_engine_norm_v1.json`
//! or `GR_LANE_S_ENGINE_NORM_PATH`).
//!
//! - Does not invent materials; does not merge hosts.
//! - Offline `calibrate_engine_norm_from_batch` estimates scale/bias so each engine's
//!   Lane-S curve mean magnitude matches a blink reference (or global mean).
//! - Explicit adopt writes JSON; set env + restart for process OnceLock.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

pub const LANE_S_ENGINE_NORM_ALGO: &str = "lane_s_engine_norm_v1";
pub const LANE_S_ENGINE_NORM_CAL_ALGO: &str = "lane_s_engine_norm_calibrate_v1";

#[derive(Debug, Clone)]
pub struct EngineNorm {
    pub scale: f64,
    pub bias: f64,
    pub lsh_salt: String,
    pub mag_quanta_scale: f64,
    pub drop_cold_first_seed: bool,
}

impl Default for EngineNorm {
    fn default() -> Self {
        Self {
            scale: 1.0,
            bias: 0.0,
            lsh_salt: "eng_unknown_v1".into(),
            mag_quanta_scale: 1.0,
            drop_cold_first_seed: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EngineNormTable {
    pub engines: HashMap<String, EngineNorm>,
    pub learned_from_fleet: bool,
    pub source: String,
}

impl Default for EngineNormTable {
    fn default() -> Self {
        let mut engines = HashMap::new();
        engines.insert(
            "blink".into(),
            EngineNorm {
                lsh_salt: "eng_blink_v1".into(),
                ..Default::default()
            },
        );
        engines.insert(
            "gecko".into(),
            EngineNorm {
                scale: 1.02,
                mag_quanta_scale: 1.05,
                lsh_salt: "eng_gecko_v1".into(),
                ..Default::default()
            },
        );
        engines.insert(
            "webkit".into(),
            EngineNorm {
                scale: 1.08,
                bias: -0.002,
                mag_quanta_scale: 1.12,
                lsh_salt: "eng_webkit_v1".into(),
                drop_cold_first_seed: true,
            },
        );
        engines.insert("unknown".into(), EngineNorm::default());
        Self {
            engines,
            learned_from_fleet: false,
            source: "builtin_default".into(),
        }
    }
}

fn parse_table(v: &Value, source: &str) -> EngineNormTable {
    let mut t = EngineNormTable::default();
    t.source = source.into();
    t.learned_from_fleet = v
        .get("learned_from_fleet")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    if let Some(obj) = v.get("engines").and_then(|x| x.as_object()) {
        for (k, val) in obj {
            let mut n = EngineNorm::default();
            if let Some(s) = val.get("scale").and_then(|x| x.as_f64()) {
                n.scale = s;
            }
            if let Some(b) = val.get("bias").and_then(|x| x.as_f64()) {
                n.bias = b;
            }
            if let Some(s) = val.get("lsh_salt").and_then(|x| x.as_str()) {
                n.lsh_salt = s.into();
            }
            if let Some(s) = val.get("mag_quanta_scale").and_then(|x| x.as_f64()) {
                n.mag_quanta_scale = s;
            }
            n.drop_cold_first_seed = val
                .get("drop_cold_first_seed")
                .and_then(|x| x.as_bool())
                .unwrap_or(false);
            t.engines.insert(k.to_ascii_lowercase(), n);
        }
    }
    t
}

pub fn load_engine_norm_table() -> EngineNormTable {
    static T: OnceLock<EngineNormTable> = OnceLock::new();
    T.get_or_init(load_engine_norm_table_fresh).clone()
}

pub fn load_engine_norm_table_fresh() -> EngineNormTable {
    let paths = [
        gr_abi::env::get("LANE_S_ENGINE_NORM_PATH").unwrap_or_default(),
        format!(
            "{}/../../spec/lane_s_engine_norm_v1.json",
            env!("CARGO_MANIFEST_DIR")
        ),
        "spec/lane_s_engine_norm_v1.json".into(),
    ];
    for p in paths {
        if p.trim().is_empty() {
            continue;
        }
        if let Ok(t) = std::fs::read_to_string(&p) {
            if let Ok(v) = serde_json::from_str::<Value>(&t) {
                return parse_table(&v, &p);
            }
        }
    }
    EngineNormTable::default()
}

/// Normalize free-form engine / UA / brand strings → blink|gecko|webkit|unknown.
pub fn normalize_engine_key(eng: &str) -> String {
    let e = eng.to_ascii_lowercase();
    if e.is_empty() || e == "unknown" {
        return "unknown".into();
    }
    // Edge / Chrome / Chromium / Opera / Brave → blink
    if e.contains("blink")
        || e.contains("chrome")
        || e.contains("chromium")
        || e.contains("edg/")
        || e.contains("edg ")
        || e.contains("edge")
        || e.contains("opera")
        || e.contains("brave")
        || e.contains("vivaldi")
        || e == "chrome"
        || e == "edge"
    {
        return "blink".into();
    }
    if e.contains("gecko") || e.contains("firefox") || e.contains("fxios") {
        return "gecko".into();
    }
    // Safari / iOS WebKit (careful: "chrome mobile" already caught as blink)
    if e.contains("webkit") || e.contains("safari") || e.contains("applewebkit") || e == "ios" {
        return "webkit".into();
    }
    // brand tokens sometimes passed alone
    match e.as_str() {
        "ff" | "fx" => "gecko".into(),
        "sf" | "ios_safari" => "webkit".into(),
        _ => {
            if e.len() <= 12 && !e.contains(' ') {
                // unknown short token — keep as unknown rather than invent family
                "unknown".into()
            } else {
                "unknown".into()
            }
        }
    }
}

/// Robust engine detection from a fields bag.
pub fn detect_engine_from_fields(fields: &Value) -> String {
    let fo = match fields.as_object() {
        Some(m) => m,
        None => return "unknown".into(),
    };
    // Prefer structured trust helper when available
    if let Some(s) = crate::trust::engine_family_from_fields(fo) {
        return normalize_engine_key(&s);
    }
    for k in [
        "engine_family",
        "engine_obs",
        "engine_claim",
        "protocol_engine",
        "browser_engine",
        "js_engine_family",
    ] {
        if let Some(s) = fo.get(k).and_then(|v| v.as_str()) {
            let n = normalize_engine_key(s);
            if n != "unknown" {
                return n;
            }
        }
    }
    // UA / brands
    for k in ["user_agent", "ua", "navigator_user_agent", "sec_ch_ua"] {
        if let Some(s) = fo.get(k).and_then(|v| v.as_str()) {
            let n = normalize_engine_key(s);
            if n != "unknown" {
                return n;
            }
        }
    }
    if let Some(arr) = fo.get("brands").or_else(|| fo.get("ua_ch_brands")).and_then(|v| v.as_array()) {
        for b in arr {
            let s = b
                .get("brand")
                .or_else(|| b.as_str().map(|_| b))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let n = normalize_engine_key(s);
            if n != "unknown" {
                return n;
            }
        }
    }
    "unknown".into()
}

/// Apply engine affine + optional cold-seed drop (WebKit) to a Lane-S curve.
pub fn normalize_lane_s_curve(curve: &[f64], engine_family: Option<&str>) -> (Vec<f64>, Value) {
    let table = load_engine_norm_table();
    let key = normalize_engine_key(engine_family.unwrap_or(""));
    let norm = table.engines.get(&key).cloned().unwrap_or_default();
    let mut c = if norm.drop_cold_first_seed {
        crate::trust::residual_curve_for_engine(curve, Some("webkit"))
    } else {
        curve.to_vec()
    };
    for x in &mut c {
        *x = *x * norm.scale + norm.bias;
    }
    let meta = json!({
        "algo": LANE_S_ENGINE_NORM_ALGO,
        "engine": key,
        "scale": norm.scale,
        "bias": norm.bias,
        "lsh_salt": norm.lsh_salt,
        "mag_quanta_scale": norm.mag_quanta_scale,
        "drop_cold_first_seed": norm.drop_cold_first_seed,
        "learned_from_fleet": table.learned_from_fleet,
        "source": table.source,
        "note": if table.learned_from_fleet {
            "calibrated / fleet-filled table"
        } else {
            "design default or uncalibrated file — not yet learned_from_fleet"
        },
    });
    (c, meta)
}

pub fn engine_norm_table_json() -> Value {
    let t = load_engine_norm_table_fresh();
    let mut engines = serde_json::Map::new();
    let mut keys: Vec<_> = t.engines.keys().cloned().collect();
    keys.sort();
    for k in keys {
        if let Some(n) = t.engines.get(&k) {
            engines.insert(
                k,
                json!({
                    "scale": n.scale,
                    "bias": n.bias,
                    "lsh_salt": n.lsh_salt,
                    "mag_quanta_scale": n.mag_quanta_scale,
                    "drop_cold_first_seed": n.drop_cold_first_seed,
                }),
            );
        }
    }
    json!({
        "algo": LANE_S_ENGINE_NORM_ALGO,
        "learned_from_fleet": t.learned_from_fleet,
        "source": t.source,
        "engines": engines,
        "silent_adopt": false,
    })
}

pub fn engine_norm_status() -> Value {
    let t = engine_norm_table_json();
    json!({
        "algo": LANE_S_ENGINE_NORM_ALGO,
        "learned_from_fleet": t.get("learned_from_fleet"),
        "source": t.get("source"),
        "env_override": !gr_abi::env::get("LANE_S_ENGINE_NORM_PATH")
            .unwrap_or_default()
            .trim()
            .is_empty(),
        "engines": t.get("engines"),
        "note": "Lane-S pre-digest affine only; never invents materials",
    })
}

fn f64_vec(v: &Value) -> Vec<f64> {
    match v {
        Value::Array(a) => a
            .iter()
            .filter_map(|x| x.as_f64().or_else(|| x.as_i64().map(|i| i as f64)))
            .filter(|x| x.is_finite())
            .collect(),
        _ => Vec::new(),
    }
}

fn mean_abs(curve: &[f64]) -> f64 {
    if curve.is_empty() {
        return 0.0;
    }
    curve.iter().map(|x| x.abs()).sum::<f64>() / curve.len() as f64
}

fn lane_s_raw_from_fields(fields: &Value) -> Vec<f64> {
    // Prefer residual_paths ulp/fma/denorm/interp curves; fallback hw_curve_webgl
    let fo = fields.as_object().cloned().unwrap_or_default();
    let mut parts: Vec<Vec<f64>> = Vec::new();
    if let Some(arr) = fo.get("residual_paths").and_then(|v| v.as_array()) {
        for e in arr {
            let mode = e
                .get("shader_mode")
                .or_else(|| e.get("path_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let silicon = mode.contains("ulp")
                || mode.contains("fma")
                || mode.contains("denorm")
                || mode.contains("interp")
                || mode.contains("tex_lerp");
            if !silicon {
                continue;
            }
            let c = e
                .get("curve")
                .or_else(|| e.get("values"))
                .map(f64_vec)
                .unwrap_or_default();
            if c.len() >= 8 {
                parts.push(c);
            }
        }
    }
    if parts.is_empty() {
        let c = fo
            .get("hw_curve_webgl")
            .or_else(|| fo.get("webgl_residual_curve"))
            .map(f64_vec)
            .unwrap_or_default();
        if c.len() >= 8 {
            parts.push(c);
        }
    }
    if parts.is_empty() {
        return Vec::new();
    }
    // average
    let n = parts.iter().map(|p| p.len()).min().unwrap_or(0);
    let mut out = vec![0.0; n];
    for p in &parts {
        for i in 0..n {
            out[i] += p[i];
        }
    }
    let d = parts.len() as f64;
    for x in &mut out {
        *x /= d;
    }
    out
}

/// Offline calibrate: align each engine's mean|Lane-S| to blink (or global) reference.
/// Input batch: array of fields / `{fields, engine_family}` / `{observations:[...]}`.
pub fn calibrate_engine_norm_from_batch(batch: &Value) -> Value {
    let items: Vec<Value> = batch
        .get("observations")
        .or_else(|| batch.get("batch"))
        .or_else(|| batch.get("items"))
        .and_then(|v| v.as_array())
        .cloned()
        .or_else(|| batch.as_array().cloned())
        .unwrap_or_default();
    if items.is_empty() {
        return json!({
            "algo": LANE_S_ENGINE_NORM_CAL_ALGO,
            "ok": false,
            "error": "empty_batch",
            "adopt": false,
        });
    }
    let mut per_engine: HashMap<String, Vec<f64>> = HashMap::new();
    for it in &items {
        let fields = it.get("fields").cloned().unwrap_or_else(|| it.clone());
        let eng = it
            .get("engine_family")
            .or_else(|| it.get("engine"))
            .and_then(|v| v.as_str())
            .map(|s| normalize_engine_key(s))
            .unwrap_or_else(|| detect_engine_from_fields(&fields));
        let curve = lane_s_raw_from_fields(&fields);
        if curve.len() < 8 {
            continue;
        }
        per_engine
            .entry(eng)
            .or_default()
            .push(mean_abs(&curve));
    }
    if per_engine.is_empty() {
        return json!({
            "algo": LANE_S_ENGINE_NORM_CAL_ALGO,
            "ok": false,
            "error": "no_curves",
            "adopt": false,
        });
    }
    let mut means: HashMap<String, f64> = HashMap::new();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for (k, vs) in &per_engine {
        let m = vs.iter().sum::<f64>() / vs.len() as f64;
        means.insert(k.clone(), m);
        counts.insert(k.clone(), vs.len());
    }
    let ref_mean = means
        .get("blink")
        .copied()
        .or_else(|| {
            let all: Vec<f64> = means.values().copied().collect();
            if all.is_empty() {
                None
            } else {
                Some(all.iter().sum::<f64>() / all.len() as f64)
            }
        })
        .unwrap_or(1.0)
        .max(1e-6);

    let base = load_engine_norm_table_fresh();
    let mut engines = serde_json::Map::new();
    for fam in ["blink", "gecko", "webkit", "unknown"] {
        let mut n = base.engines.get(fam).cloned().unwrap_or_default();
        if let Some(&m) = means.get(fam) {
            if m > 1e-9 {
                // scale so mean_abs * scale ≈ ref_mean
                n.scale = (ref_mean / m).clamp(0.5, 2.0);
                n.scale = (n.scale * 10000.0).round() / 10000.0;
                n.mag_quanta_scale = (n.scale * 1.0).clamp(0.8, 1.5);
                n.mag_quanta_scale = (n.mag_quanta_scale * 10000.0).round() / 10000.0;
            }
        }
        if n.lsh_salt.is_empty() {
            n.lsh_salt = format!("eng_{fam}_v1");
        }
        if fam == "webkit" {
            n.drop_cold_first_seed = true;
        }
        engines.insert(
            fam.into(),
            json!({
                "scale": n.scale,
                "bias": n.bias,
                "lsh_salt": n.lsh_salt,
                "mag_quanta_scale": n.mag_quanta_scale,
                "drop_cold_first_seed": n.drop_cold_first_seed,
                "n_samples": counts.get(fam).copied().unwrap_or(0),
                "mean_abs": means.get(fam).copied().unwrap_or(0.0),
            }),
        );
    }
    let multi_engine = means.keys().filter(|k| *k != "unknown").count() >= 2;
    let table = json!({
        "algo": LANE_S_ENGINE_NORM_ALGO,
        "version": 1,
        "learned_from_fleet": multi_engine,
        "source": "offline_calibrate_v1",
        "reference_engine": if means.contains_key("blink") { "blink" } else { "global_mean" },
        "reference_mean_abs": ref_mean,
        "engines": engines,
        "note": if multi_engine {
            "calibrated from multi-engine batch; review then adopt"
        } else {
            "single-engine batch — scales near identity; need multi-browser fleet for real calibrate"
        },
    });
    json!({
        "algo": LANE_S_ENGINE_NORM_CAL_ALGO,
        "ok": true,
        "n_items": items.len(),
        "engines_seen": means.keys().cloned().collect::<Vec<_>>(),
        "per_engine_n": counts,
        "table": table,
        "adopt": false,
        "silent_adopt": false,
        "note": "candidate table — write_engine_norm_candidate / adopt_engine_norm; never silent",
    })
}

pub fn write_engine_norm_candidate(table: &Value, path: &Path) -> Value {
    let body = if table.get("engines").is_some() {
        table.clone()
    } else if let Some(t) = table.get("table") {
        t.clone()
    } else {
        return json!({"ok": false, "error": "no_table"});
    };
    let mut out = body;
    if let Some(o) = out.as_object_mut() {
        o.insert("algo".into(), json!(LANE_S_ENGINE_NORM_ALGO));
        o.insert(
            "written_at_ms".into(),
            json!(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0)),
        );
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(
        path,
        serde_json::to_string_pretty(&out).unwrap_or_else(|_| "{}".into()),
    ) {
        Ok(()) => json!({
            "ok": true,
            "path": path.display().to_string(),
            "algo": "engine_norm_write_v1",
            "table": out,
            "adopted_into_process": false,
        }),
        Err(e) => json!({"ok": false, "error": e.to_string()}),
    }
}

pub fn adopt_engine_norm(table: &Value, path: &Path) -> Value {
    let wr = write_engine_norm_candidate(table, path);
    if !wr.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        return wr;
    }
    std::env::set_var("GR_LANE_S_ENGINE_NORM_PATH", path.display().to_string());
    let _ = crate::config_snapshot::push_config_overlay(
        &json!({
            "lane_s_engine_norm_path": path.display().to_string(),
            "lane_s_engine_norm_learned": table
                .get("learned_from_fleet")
                .or_else(|| table.pointer("/table/learned_from_fleet"))
                .cloned()
                .unwrap_or(json!(true)),
        }),
        None,
    );
    json!({
        "ok": true,
        "algo": "engine_norm_adopt_v1",
        "path": path.display().to_string(),
        "env_set": true,
        "fresh": engine_norm_table_json(),
        "silent_adopt": false,
        "process_once_lock_note": "restart long-lived workers to refresh OnceLock",
    })
}

pub fn engine_norm_ops(batch: &Value, mode: &str, out_path: Option<&Path>) -> Value {
    let cal = calibrate_engine_norm_from_batch(batch);
    if !cal.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        return cal;
    }
    match mode {
        "calibrate" | "fit" | "" => cal,
        "write" => {
            let path = out_path
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| Path::new("data/lane_s_engine_norm_candidate.json").to_path_buf());
            let t = cal.get("table").cloned().unwrap_or(json!({}));
            let mut out = write_engine_norm_candidate(&t, &path);
            if let Some(o) = out.as_object_mut() {
                o.insert("calibrate".into(), cal);
            }
            out
        }
        "adopt" => {
            let path = out_path
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| Path::new("data/lane_s_engine_norm_adopted.json").to_path_buf());
            let t = cal.get("table").cloned().unwrap_or(json!({}));
            let mut out = adopt_engine_norm(&t, &path);
            if let Some(o) = out.as_object_mut() {
                o.insert("calibrate".into(), cal);
            }
            out
        }
        other => json!({"ok": false, "error": format!("unknown_mode:{other}"), "calibrate": cal}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn webkit_scale_applied() {
        let c = vec![0.1_f64; 32];
        let (out, meta) = normalize_lane_s_curve(&c, Some("webkit"));
        assert!((out[0] - (0.1 * 1.08 - 0.002)).abs() < 1e-9);
        assert_eq!(meta["engine"], "webkit");
    }

    #[test]
    fn blink_identity_scale() {
        let c = vec![0.2_f64; 32];
        let (out, _) = normalize_lane_s_curve(&c, Some("blink"));
        assert!((out[0] - 0.2).abs() < 1e-12);
    }

    #[test]
    fn detect_engine_from_ua() {
        assert_eq!(
            detect_engine_from_fields(&json!({"user_agent": "Mozilla/5.0 Firefox/120.0"})),
            "gecko"
        );
        assert_eq!(
            detect_engine_from_fields(&json!({"engine_family": "chrome"})),
            "blink"
        );
        assert_eq!(
            detect_engine_from_fields(&json!({"sec_ch_ua": "\"Google Chrome\";v=\"120\""})),
            "blink"
        );
    }

    #[test]
    fn calibrate_multi_engine() {
        fn item(eng: &str, scale: f64) -> Value {
            let c: Vec<f64> = (0..32).map(|i| (i as f64 * 0.1 + 0.2) * scale).collect();
            json!({
                "engine_family": eng,
                "residual_paths": [{"path_id":"ulp","shader_mode":"ulp","ok":true,"curve": c}],
            })
        }
        let batch = json!([
            item("blink", 1.0),
            item("blink", 1.0),
            item("gecko", 1.2),
            item("gecko", 1.25),
            item("webkit", 0.8),
            item("webkit", 0.85),
        ]);
        let r = calibrate_engine_norm_from_batch(&batch);
        assert_eq!(r["ok"], true);
        assert_eq!(r["table"]["learned_from_fleet"], true);
        let gs = r["table"]["engines"]["gecko"]["scale"].as_f64().unwrap();
        // gecko mean higher → scale < 1 to match blink
        assert!(gs < 1.0);
    }
}
