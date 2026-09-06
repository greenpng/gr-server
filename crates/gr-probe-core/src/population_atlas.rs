//! Population atlas: cohort differential encoding + equal-mass binning (iss/58 B2/B5).
//!
//! Process-local atlas updated from observations, **merged to shared JSON** for
//! multi-worker (`data/shared_governance/population_atlas.json`). Versioned
//! `binmap_version` / `atlas_version` enter digest material so rebalance does
//! not silently false-split.
//!
//! Cohort key uses **configuration-class** signals only (engine × GPU class × OS class)
//! — never IP/UA; never raw renderer model strings in digest body.

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub const POPULATION_ATLAS_ALGO: &str = "population_atlas_v1";
pub const BINMAP_VERSION: &str = "eqmass64_v1";
pub const ATLAS_VERSION: &str = "atlas_v1";

#[derive(Clone, Default)]
struct CurveTemplate {
    /// Running sum per bin (fixed length after first)
    sum: Vec<f64>,
    n: usize,
}

impl CurveTemplate {
    fn observe(&mut self, curve: &[f64]) {
        if curve.is_empty() {
            return;
        }
        if self.sum.is_empty() {
            self.sum = curve.to_vec();
            self.n = 1;
            return;
        }
        let n = self.sum.len().min(curve.len());
        for i in 0..n {
            self.sum[i] += curve[i];
        }
        // grow if longer curve arrives
        if curve.len() > self.sum.len() {
            for v in curve.iter().skip(self.sum.len()) {
                self.sum.push(*v);
            }
        }
        self.n += 1;
    }

    /// Merge another template (shared multi-worker). Prefer max n / proportional sum.
    fn merge_from(&mut self, other: &CurveTemplate) {
        if other.n == 0 || other.sum.is_empty() {
            return;
        }
        if self.n == 0 || self.sum.is_empty() {
            self.sum = other.sum.clone();
            self.n = other.n;
            return;
        }
        // If peer has strictly more samples, adopt peer (avoid double-count on reload).
        if other.n > self.n {
            // Blend: keep weighted average of means * new n
            let n_a = self.n as f64;
            let n_b = other.n as f64;
            let len = self.sum.len().max(other.sum.len());
            let mut sum = vec![0.0; len];
            for i in 0..len {
                let a = self.sum.get(i).copied().unwrap_or(0.0) / n_a;
                let b = other.sum.get(i).copied().unwrap_or(0.0) / n_b;
                // Take peer mean when peer is fresher (higher n); mild blend
                let w = 0.35;
                sum[i] = (a * w + b * (1.0 - w)) * n_b;
            }
            self.sum = sum;
            self.n = other.n;
        }
    }

    fn median_proxy(&self) -> Vec<f64> {
        // mean as proxy for median with limited samples
        if self.n == 0 || self.sum.is_empty() {
            return Vec::new();
        }
        let n = self.n as f64;
        self.sum.iter().map(|s| s / n).collect()
    }
}

struct AtlasState {
    /// cohort_key → channel → template
    by_cohort: HashMap<String, HashMap<String, CurveTemplate>>,
    last_flush_ms: u64,
    loaded: bool,
}

impl AtlasState {
    fn new() -> Self {
        Self {
            by_cohort: HashMap::new(),
            last_flush_ms: 0,
            loaded: false,
        }
    }
}

static ATLAS: Mutex<Option<AtlasState>> = Mutex::new(None);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn with_atlas<R>(f: impl FnOnce(&mut AtlasState) -> R) -> R {
    let mut g = ATLAS.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(AtlasState::new());
    }
    let st = g.as_mut().unwrap();
    if !st.loaded {
        load_atlas_from_shared(st);
        st.loaded = true;
    }
    f(st)
}

/// Serialize atlas/HNSW-touching tests (global process state). Nested-safe.
pub fn with_atlas_test_lock<R>(f: impl FnOnce() -> R) -> R {
    use std::cell::Cell;
    thread_local! {
        static HELD: Cell<bool> = const { Cell::new(false) };
    }
    static LOCK: Mutex<()> = Mutex::new(());
    if HELD.with(|h| h.get()) {
        return f();
    }
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    HELD.with(|h| h.set(true));
    let out = f();
    HELD.with(|h| h.set(false));
    out
}

pub fn reset_atlas_for_tests() {
    // Always hold test lock so parallel tests cannot interleave observe/reset.
    with_atlas_test_lock(|| {
        // Clear shared store first so load_atlas_from_shared cannot re-pollute.
        use crate::shared_governance::{as_object_mut, with_shared_json};
        if crate::shared_governance::shared_governance_dir().is_some() {
            let _ = with_shared_json("population_atlas", json!({"by_cohort":{}}), |v| {
                as_object_mut(v).insert("by_cohort".into(), json!({}));
            });
        }
        crate::hnsw_lite::reset_hnsw_for_tests();
        with_atlas(|a| {
            *a = AtlasState::new();
            a.loaded = true; // skip auto-load after reset unless shared re-seeded
            a.last_flush_ms = now_ms(); // avoid immediate shared reload of stale peers
        });
    });
}

fn load_atlas_from_shared(st: &mut AtlasState) {
    use crate::shared_governance::with_shared_json;
    if crate::shared_governance::shared_governance_dir().is_none() {
        return;
    }
    let pulled = with_shared_json("population_atlas", json!({"by_cohort":{}}), |v| {
        v.get("by_cohort")
            .and_then(|x| x.as_object())
            .cloned()
            .unwrap_or_default()
    });
    let Some(by) = pulled else {
        return;
    };
    for (ck, chans) in by {
        let m = st.by_cohort.entry(ck).or_default();
        if let Some(obj) = chans.as_object() {
            for (ch, tv) in obj {
                let n = tv.get("n").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
                let sum: Vec<f64> = tv
                    .get("sum")
                    .and_then(|x| x.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_f64()).collect())
                    .unwrap_or_default();
                if n == 0 || sum.is_empty() {
                    continue;
                }
                let peer = CurveTemplate { sum, n };
                m.entry(ch.clone()).or_default().merge_from(&peer);
            }
        }
    }
}

fn flush_atlas_to_shared(st: &mut AtlasState) {
    use crate::shared_governance::{as_object_mut, with_shared_json};
    if crate::shared_governance::shared_governance_dir().is_none() {
        return;
    }
    let t = now_ms();
    if st.last_flush_ms > 0 && t.saturating_sub(st.last_flush_ms) < 300 {
        return;
    }
    // Cap cohorts written
    let mut snap: Vec<(String, Vec<(String, Vec<f64>, usize)>)> = Vec::new();
    for (ck, chans) in st.by_cohort.iter().take(256) {
        let mut chv = Vec::new();
        for (ch, t) in chans {
            chv.push((ch.clone(), t.sum.clone(), t.n));
        }
        snap.push((ck.clone(), chv));
    }
    let _ = with_shared_json("population_atlas", json!({"by_cohort":{}}), |v| {
        let o = as_object_mut(v);
        o.insert("algo".into(), json!(POPULATION_ATLAS_ALGO));
        o.insert("atlas_version".into(), json!(ATLAS_VERSION));
        o.insert("binmap_version".into(), json!(BINMAP_VERSION));
        let by = o
            .entry("by_cohort")
            .or_insert(json!({}))
            .as_object_mut()
            .unwrap();
        for (ck, chans) in snap {
            let entry = by.entry(ck).or_insert(json!({})).as_object_mut().unwrap();
            for (ch, sum, n) in chans {
                let peer_n = entry
                    .get(&ch)
                    .and_then(|x| x.get("n"))
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0) as usize;
                if n >= peer_n {
                    entry.insert(ch, json!({"sum": sum, "n": n}));
                }
            }
        }
    });
    st.last_flush_ms = t;
}

/// Cohort key from configuration-class fields (not digest body materials).
///
/// iss/67: when `hw_model_key` is present, it becomes the primary GPU cell token
/// (still a class key — never raw model strings in commercial body).
pub fn cohort_key_from_fields(fields: &Value) -> String {
    let eng = fields
        .get("engine_family")
        .or_else(|| fields.get("residual_probe_engine"))
        .and_then(|v| v.as_str())
        .unwrap_or("unk");
    let os = fields
        .get("os_family")
        .or_else(|| fields.get("ua_ch_platform"))
        .and_then(|v| v.as_str())
        .unwrap_or("unk");
    // Model-layer key (iss/63 K) preferred over coarse stack class
    let gpu = fields
        .get("hw_model_key")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty() && *s != "unk:unk")
        .or_else(|| {
            fields
                .get("gl_stack_class")
                .or_else(|| fields.get("renderer_class"))
                .or_else(|| fields.get("stack_class"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or("gpu_unk");
    // A7/A8 cohort signals if present
    let gamut = fields
        .get("cohort_color_gamut")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let dpr_b = fields
        .get("cohort_dpr_bucket")
        .or_else(|| fields.get("dpr"))
        .and_then(|v| v.as_f64())
        .map(|x| format!("{x:.2}"))
        .unwrap_or_default();
    // A7/A8 cohort materials (iss/60 L6) — configuration class only
    let storage_cls = fields
        .get("cohort_storage_class")
        .or_else(|| fields.get("storage_quota_class"))
        .or_else(|| fields.get("storage_estimate_class"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let icu = fields
        .get("cohort_icu_locale")
        .or_else(|| fields.get("intl_locale"))
        .or_else(|| fields.get("language"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let hdr = fields
        .get("cohort_hdr")
        .or_else(|| fields.get("display_hdr"))
        .or_else(|| fields.get("color_gamut"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    // probe_version pins cell against digest-schema drift (iss/68 Terra)
    let probe_ver = fields
        .get("product_version")
        .or_else(|| fields.get("probe_version"))
        .or_else(|| fields.get("sdk_v"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    format!("{eng}|{os}|{gpu}|g={gamut}|d={dpr_b}|s={storage_cls}|i={icu}|h={hdr}|pv={probe_ver}")
}

/// Offline / history cold-start: observe many field objects into the atlas.
pub fn bootstrap_from_field_batch(batch: &[Value]) -> Value {
    let mut n = 0usize;
    for f in batch {
        if f.is_object() {
            // ensure model key is attached for cell placement when possible
            let mut m = f.as_object().cloned().unwrap_or_default();
            if !m.contains_key("hw_model_key") {
                crate::model_key::attach_model_key_extras(&mut m);
            }
            atlas_observe(&Value::Object(m));
            n += 1;
        }
    }
    serde_json::json!({
        "algo": "population_atlas_bootstrap_v1",
        "ok": true,
        "n_observed": n,
    })
}

fn curve(fields: &Value, keys: &[&str]) -> Vec<f64> {
    for k in keys {
        if let Some(a) = fields.get(*k).and_then(|v| v.as_array()) {
            let xs: Vec<f64> = a.iter().filter_map(|x| x.as_f64()).collect();
            // fit accepts ≥4; observe prefers ≥8 when available
            if xs.len() >= 4 {
                return xs;
            }
        }
    }
    // residual_path_means: tile to ≥8 for template_similarity length gate
    if keys.iter().any(|k| *k == "residual_path_means") {
        if let Some(a) = fields.get("residual_path_means").and_then(|v| v.as_array()) {
            let xs: Vec<f64> = a.iter().filter_map(|x| x.as_f64()).collect();
            if xs.len() >= 2 {
                let mut out = xs.clone();
                while out.len() < 8 {
                    out.extend_from_slice(&xs);
                }
                return out;
            }
        }
    }
    Vec::new()
}

/// Observe fields into atlas (call on analyze). Multi-worker shared via file.
pub fn atlas_observe(fields: &Value) {
    let ck = cohort_key_from_fields(fields);
    let channels = [
        (
            "res",
            curve(
                fields,
                &[
                    "webgl_residual_multipath",
                    "residual_path_means",
                    "hw_curve_webgl",
                ],
            ),
        ),
        ("wg", curve(fields, &["hw_curve_webgl", "webgl_residual_multipath"])),
        (
            "au",
            curve(
                fields,
                &[
                    "audio_seed_delta_curve",
                    "audio_deep_curve",
                    "hw_curve_audio",
                ],
            ),
        ),
        ("cp", curve(fields, &["cpu_timing_curve", "hw_curve_cpu"])),
    ];
    with_atlas(|a| {
        // Refresh peer templates periodically
        if a.last_flush_ms == 0 || now_ms().saturating_sub(a.last_flush_ms) > 2000 {
            load_atlas_from_shared(a);
        }
        let m = a.by_cohort.entry(ck).or_default();
        for (ch, c) in channels {
            if c.is_empty() {
                continue;
            }
            m.entry(ch.into()).or_default().observe(&c);
        }
        flush_atlas_to_shared(a);
    });
}

fn robust_z(curve: &[f64], template: &[f64]) -> Vec<f64> {
    let n = curve.len().min(template.len());
    if n == 0 {
        return Vec::new();
    }
    let mut deltas: Vec<f64> = (0..n).map(|i| curve[i] - template[i]).collect();
    // MAD of deltas
    let mut sorted = deltas.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let med = sorted[sorted.len() / 2];
    let mut abs_dev: Vec<f64> = deltas.iter().map(|d| (d - med).abs()).collect();
    abs_dev.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mad = abs_dev[abs_dev.len() / 2].max(1e-9);
    for d in &mut deltas {
        *d = (*d - med) / (1.4826 * mad);
    }
    deltas
}

/// Equal-mass binning: map each z to rank/N * n_bins (session-local ranks + version).
pub fn equal_mass_bins(z: &[f64], n_bins: usize) -> Vec<u16> {
    if z.is_empty() {
        return Vec::new();
    }
    let n_bins = n_bins.max(2);
    let mut order: Vec<usize> = (0..z.len()).collect();
    order.sort_by(|&i, &j| z[i].partial_cmp(&z[j]).unwrap_or(std::cmp::Ordering::Equal));
    let mut out = vec![0u16; z.len()];
    let n = z.len() as f64;
    for (rank, &i) in order.iter().enumerate() {
        let bin = ((rank as f64 / n) * n_bins as f64).floor() as usize;
        out[i] = bin.min(n_bins - 1) as u16;
    }
    out
}

fn digest_bins(channel: &str, bins: &[u16]) -> String {
    let mut h = Sha256::new();
    h.update(POPULATION_ATLAS_ALGO.as_bytes());
    h.update(b"|");
    h.update(BINMAP_VERSION.as_bytes());
    h.update(b"|");
    h.update(ATLAS_VERSION.as_bytes());
    h.update(b"|");
    h.update(channel.as_bytes());
    h.update(b"|");
    for b in bins {
        h.update(b.to_le_bytes());
    }
    format!("{:x}", h.finalize())[..12].to_string()
}

/// Encode res/au differential materials for mint extras.
pub fn differential_encode(fields: &Value) -> Value {
    let ck = cohort_key_from_fields(fields);
    atlas_observe(fields);
    let templates = with_atlas(|a| {
        a.by_cohort
            .get(&ck)
            .map(|m| {
                let mut out = Map::new();
                for (k, t) in m {
                    out.insert(k.clone(), json!(t.median_proxy()));
                }
                out
            })
            .unwrap_or_default()
    });

    let mut extras = Map::new();
    extras.insert("atlas_version".into(), json!(ATLAS_VERSION));
    extras.insert("binmap_version".into(), json!(BINMAP_VERSION));
    extras.insert("cohort_key_hash".into(), {
        let mut h = Sha256::new();
        h.update(ck.as_bytes());
        json!(format!("{:x}", h.finalize())[..12].to_string())
    });

    for (ch, keys) in [
        ("wg", &["hw_curve_webgl", "webgl_residual_multipath"][..]),
        (
            "au",
            &[
                "audio_seed_delta_curve",
                "audio_deep_curve",
                "hw_curve_audio",
            ][..],
        ),
    ] {
        let c = curve(fields, keys);
        if c.len() < 8 {
            continue;
        }
        let tmpl: Vec<f64> = templates
            .get(ch)
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_f64()).collect())
            .unwrap_or_default();
        let z = if tmpl.len() >= 8 {
            robust_z(&c, &tmpl)
        } else {
            // cold start: equal-mass on raw curve (still versioned)
            c.clone()
        };
        let bins = equal_mass_bins(&z, 64);
        let dig = digest_bins(ch, &bins);
        extras.insert(
            format!("{ch}_diff_digest"),
            json!(dig),
        );
        extras.insert(
            format!("{ch}_diff_n"),
            json!(bins.len()),
        );
        extras.insert(
            format!("{ch}_diff_mode"),
            json!(if tmpl.len() >= 8 {
                "cohort_robust_z"
            } else {
                "cold_equal_mass"
            }),
        );
        // B3 phase1: whitening LSH on robust-z / raw for ANN recall
        attach_whitening_lsh(&mut extras, ch, &z);
    }

    json!({
        "algo": POPULATION_ATLAS_ALGO,
        "extras": extras,
        "cohort_key": ck,
    })
}

// ─── iss/67 C1 shadow fit: K cell fit + best alternative key (iss/69 U4) ───

/// Channels used for shadow fit (iss/73 audit P0-2): **res is V primary**.
/// Weights rebalanced; res/wg multipath overlap is handled by fit_weight_adjusted.
const FIT_CHANNELS: &[(&str, &[&str], f64)] = &[
    (
        "res",
        &[
            "webgl_residual_multipath",
            "residual_path_means",
            "hw_curve_webgl_fused_s",
            "hw_curve_webgl",
        ],
        0.40,
    ),
    ("wg", &["hw_curve_webgl", "webgl_residual_multipath"], 0.25),
    (
        "au",
        &["audio_seed_delta_curve", "audio_deep_curve", "hw_curve_audio"],
        0.20,
    ),
    ("cp", &["cpu_timing_curve", "hw_curve_cpu"], 0.15),
];

fn fit_weight(ch: &str) -> f64 {
    FIT_CHANNELS
        .iter()
        .find(|(c, _, _)| *c == ch)
        .map(|(_, _, w)| *w)
        .unwrap_or(0.0)
}

/// When both res and wg are observed, downweight wg to avoid multipath double-count.
fn fit_weight_adjusted(ch: &str, has_res: bool, has_wg: bool) -> f64 {
    let w = fit_weight(ch);
    if ch == "wg" && has_res && has_wg {
        w * 0.70
    } else {
        w
    }
}

// ─── iss/73 P1-2: per-(kernel,backend) canonical T_k ───────────────────────

/// Affine map: out[i] = scale * in[i] + bias (broadcast), optional length trim.
#[derive(Clone, Debug)]
struct TkAffine {
    scale: f64,
    bias: f64,
}

static TK_TABLE: RwLock<Option<HashMap<String, TkAffine>>> = RwLock::new(None);

fn tk_key(engine: &str, backend: &str) -> String {
    format!(
        "{}|{}",
        engine.to_ascii_lowercase(),
        backend.to_ascii_lowercase()
    )
}

fn load_tk_map_from_disk() -> HashMap<String, TkAffine> {
    let path = gr_abi::env::get("CANONICAL_TK_FILE").unwrap_or_else(|| {
        "data/canonical_tk.json".into()
    });
    let mut map = HashMap::new();
    if let Ok(txt) = std::fs::read_to_string(&path) {
        if let Ok(v) = serde_json::from_str::<Value>(&txt) {
            if let Some(o) = v.as_object() {
                for (k, val) in o {
                    // skip meta keys
                    if k.starts_with('_') {
                        continue;
                    }
                    let scale = val.get("scale").and_then(|x| x.as_f64()).unwrap_or(1.0);
                    let bias = val.get("bias").and_then(|x| x.as_f64()).unwrap_or(0.0);
                    map.insert(k.clone(), TkAffine { scale, bias });
                }
            }
        }
    }
    // Built-in mild defaults for known lab engines (identity if unknown)
    for (k, scale, bias) in [
        ("blink|gl", 1.0, 0.0),
        ("chrome|gl", 1.0, 0.0),
        ("gecko|gl", 1.0, 0.0),
        ("firefox|gl", 1.0, 0.0),
        // WebKit residual often slightly lower mean on lab hosts — mild uplift toward Blink
        ("webkit|gl", 1.02, 0.0005),
        ("webkit|metal", 1.02, 0.0005),
    ] {
        map.entry(k.into()).or_insert(TkAffine { scale, bias });
    }
    map
}

/// Load / hot-reload T_k table from `GR_CANONICAL_TK_FILE` or default path.
/// JSON: `{ "chrome|gl": {"scale":1.0,"bias":0.0}, "webkit|gl": {...} }`
pub fn reload_canonical_tk() -> usize {
    let map = load_tk_map_from_disk();
    let n = map.len();
    if let Ok(mut g) = TK_TABLE.write() {
        *g = Some(map);
    }
    n
}

fn tk_affine(engine: &str, backend: &str) -> TkAffine {
    // Ensure loaded
    {
        let need = TK_TABLE
            .read()
            .ok()
            .map(|g| g.is_none())
            .unwrap_or(true);
        if need {
            let _ = reload_canonical_tk();
        }
    }
    let k = tk_key(engine, backend);
    let k_gl = tk_key(engine, "gl");
    let k_unk = tk_key(engine, "unk");
    if let Ok(g) = TK_TABLE.read() {
        if let Some(map) = g.as_ref() {
            if let Some(a) = map
                .get(&k)
                .or_else(|| map.get(&k_gl))
                .or_else(|| map.get(&k_unk))
            {
                return a.clone();
            }
        }
    }
    TkAffine {
        scale: 1.0,
        bias: 0.0,
    }
}

/// Apply per-kernel×backend affine map before generic canonical features (iss/63 §5.2 / iss/73 P1-2).
pub fn apply_t_k(engine: &str, backend: &str, curve: &[f64]) -> Vec<f64> {
    if curve.is_empty() {
        return Vec::new();
    }
    let aff = tk_affine(engine, backend);
    curve
        .iter()
        .map(|x| x * aff.scale + aff.bias)
        .collect()
}

/// Engine/backend from fields for T_k lookup.
pub fn engine_backend_from_fields(fields: &Value) -> (String, String) {
    let eng = fields
        .get("engine_family")
        .or_else(|| fields.get("residual_probe_engine"))
        .and_then(|v| v.as_str())
        .unwrap_or("unk")
        .to_ascii_lowercase();
    let be = fields
        .get("gl_backend")
        .or_else(|| fields.get("gl_backend_class"))
        .and_then(|v| v.as_str())
        .unwrap_or("gl")
        .to_ascii_lowercase();
    (eng, be)
}

/// iss/67 C1: canonical V features for cross-engine compare.
/// L0 absolute values stay cell-local; L1 mean-center + L2 unit-scale + ratio modes.
/// iss/73: optional T_k affine applied when engine/backend provided via `canonical_v_features_for_fields`.
pub fn canonical_v_features(curve: &[f64]) -> Vec<f64> {
    canonical_v_features_inner(curve)
}

/// Canonical features after T_k for this session's engine×backend.
pub fn canonical_v_features_for_fields(fields: &Value, curve: &[f64]) -> Vec<f64> {
    let (eng, be) = engine_backend_from_fields(fields);
    let mapped = apply_t_k(&eng, &be, curve);
    canonical_v_features_inner(&mapped)
}

fn canonical_v_features_inner(curve: &[f64]) -> Vec<f64> {
    if curve.len() < 4 {
        return Vec::new();
    }
    let n = curve.len();
    let mean = curve.iter().sum::<f64>() / n as f64;
    // L1: mean-center
    let centered: Vec<f64> = curve.iter().map(|x| x - mean).collect();
    // L2: unit scale by L2 norm
    let l2 = centered.iter().map(|x| x * x).sum::<f64>().sqrt().max(1e-12);
    let mut out: Vec<f64> = centered.iter().map(|x| x / l2).collect();
    // Ratio / modal structure (engine-robust): p25/p50/p75 ratios, lag-1, energy odd/even
    let mut sorted = curve.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let q = |p: f64| {
        let i = ((n as f64 - 1.0) * p).round() as usize;
        sorted[i.min(n - 1)]
    };
    let p25 = q(0.25);
    let p50 = q(0.50);
    let p75 = q(0.75);
    let denom = (p75 - p25).abs().max(1e-12);
    out.push((p50 - p25) / denom);
    out.push((p75 - p50) / denom);
    out.push(if mean.abs() > 1e-12 { p50 / mean } else { 0.0 });
    let mut lag1 = 0.0;
    let std = (curve.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n as f64)
        .sqrt()
        .max(1e-12);
    if n >= 4 {
        let mut num = 0.0;
        for i in 1..n {
            num += (curve[i] - mean) * (curve[i - 1] - mean);
        }
        lag1 = num / ((n - 1) as f64 * std * std);
    }
    out.push(lag1.clamp(-1.0, 1.0));
    let mut odd = 0.0;
    let mut even = 0.0;
    for (i, v) in curve.iter().enumerate() {
        if i % 2 == 0 {
            even += v.abs();
        } else {
            odd += v.abs();
        }
    }
    out.push((odd / (even + 1e-12)).clamp(0.0, 10.0));
    out
}

/// Relative-L2 similarity of a curve to a template mean (1.0 = identical shape).
/// iss/67 C1: compare in canonical feature space when both have enough samples.
/// iss/73: uses T_k-aware features when `fields` provided.
fn template_similarity_fields(fields: Option<&Value>, curve: &[f64], t: &[f64]) -> f64 {
    let n = curve.len().min(t.len());
    if n < 8 || t.is_empty() {
        return 0.0;
    }
    if curve.len() >= 8 && t.len() >= 8 {
        let a = match fields {
            Some(f) => canonical_v_features_for_fields(f, curve),
            None => canonical_v_features(curve),
        };
        let b = match fields {
            Some(f) => canonical_v_features_for_fields(f, t),
            None => canonical_v_features(t),
        };
        let m = a.len().min(b.len());
        if m >= 8 {
            let mut d2 = 0.0;
            let mut b2 = 0.0;
            for i in 0..m {
                let d = a[i] - b[i];
                d2 += d * d;
                b2 += b[i] * b[i];
            }
            let rel = (d2 / (b2 + 1e-12)).sqrt().min(1.0);
            return (1.0 - rel).max(0.0);
        }
    }
    let mut d2 = 0.0;
    let mut t2 = 0.0;
    for i in 0..n {
        let d = curve[i] - t[i];
        d2 += d * d;
        t2 += t[i] * t[i];
    }
    let rel = (d2 / (t2 + 1e-12)).sqrt().min(1.0);
    (1.0 - rel).max(0.0)
}

/// iss/73 P0-1: cell coverage report for Atlas tail explore.
pub fn cell_coverage_report() -> Value {
    with_atlas(|a| {
        if a.last_flush_ms == 0 || now_ms().saturating_sub(a.last_flush_ms) > 2000 {
            load_atlas_from_shared(a);
        }
        let mut cells = Vec::new();
        let mut n_cold = 0usize;
        let mut n_warm = 0usize;
        let mut n_total_samples = 0usize;
        for (ck, chans) in &a.by_cohort {
            let mut n_max = 0usize;
            let mut channels = Map::new();
            for (ch, t) in chans {
                n_max = n_max.max(t.n);
                n_total_samples += t.n;
                channels.insert(ch.clone(), json!({"n": t.n}));
            }
            let cold = n_max < 8;
            if cold {
                n_cold += 1;
            } else {
                n_warm += 1;
            }
            cells.push(json!({
                "key": ck,
                "n": n_max,
                "cold": cold,
                "channels": channels,
            }));
        }
        cells.sort_by(|a, b| {
            let na = a.get("n").and_then(|v| v.as_u64()).unwrap_or(0);
            let nb = b.get("n").and_then(|v| v.as_u64()).unwrap_or(0);
            na.cmp(&nb)
        });
        json!({
            "algo": "cell_coverage_report_v1",
            "n_cells": cells.len(),
            "n_cold": n_cold,
            "n_warm": n_warm,
            "n_total_channel_samples": n_total_samples,
            "cold_threshold": 8,
            "cells": cells,
            "tail": cells.iter().filter(|c| c.get("cold").and_then(|v| v.as_bool()).unwrap_or(false)).take(32).cloned().collect::<Vec<_>>(),
        })
    })
}

/// iss/73 P0-1: explore urgency for a session's K cell — drives deepen_packs_extra.
pub fn atlas_explore_urgency(fields: &Value) -> Value {
    let ck = cohort_key_from_fields(fields);
    let enabled = match gr_abi::env::get("ATLAS_EXPLORE") {
        Some(v) => v != "0" && v != "false" && v != "off",
        None => true, // default on
    };
    let (n, cold) = with_atlas(|a| {
        if a.last_flush_ms == 0 || now_ms().saturating_sub(a.last_flush_ms) > 2000 {
            load_atlas_from_shared(a);
        }
        let n = a
            .by_cohort
            .get(&ck)
            .map(|m| m.values().map(|t| t.n).max().unwrap_or(0))
            .unwrap_or(0);
        (n, n < 8)
    });
    let urgency = if !enabled {
        0.0
    } else if n == 0 {
        1.0
    } else if n < 4 {
        0.9
    } else if n < 8 {
        0.7
    } else if n < 16 {
        0.35
    } else {
        0.0
    };
    let should_deepen = urgency >= 0.7;
    let packs: Vec<&str> = if should_deepen {
        vec![
            "B10x_silicon_deep",
            "B10x_silicon_noderiv",
            "B10x_silicon_ulp",
            "B20_challenge_seed",
        ]
    } else {
        vec![]
    };
    json!({
        "algo": "atlas_explore_urgency_v1",
        "enabled": enabled,
        "cohort_key": ck,
        "cell_n": n,
        "cold": cold,
        "urgency": urgency,
        "should_deepen": should_deepen,
        "deepen_packs": packs,
        "note": "iss/73 P0-1: tail cell (n<8) → force silicon deepen packs",
    })
}

/// Shadow fit of the session V-channels to its own K cell, plus the best
/// alternative cell's fit. iss/73: res in FIT_CHANNELS; alt-key via HNSW when available.
pub fn cell_fit_and_alt_key(fields: &Value) -> Value {
    let ck = cohort_key_from_fields(fields);
    let obs: Vec<(&str, Vec<f64>)> = FIT_CHANNELS
        .iter()
        .filter_map(|(ch, keys, _)| {
            let c = curve(fields, keys);
            if c.len() >= 8 {
                Some((*ch, c))
            } else {
                None
            }
        })
        .collect();
    let has_res = obs.iter().any(|(ch, _)| *ch == "res");
    let has_wg = obs.iter().any(|(ch, _)| *ch == "wg");

    with_atlas(|a| {
        if a.last_flush_ms == 0 || now_ms().saturating_sub(a.last_flush_ms) > 2000 {
            load_atlas_from_shared(a);
        }
    });

    // Seed HNSW with warm cells for alt-key (iss/73 P1-1)
    with_atlas(|a| {
        for (oc, m) in a.by_cohort.iter() {
            let n = m.values().map(|t| t.n).max().unwrap_or(0);
            if n >= 8 {
                // synthetic fields from template for embedding
                if let Some(t) = m.get("wg").or_else(|| m.get("res")) {
                    let med = t.median_proxy();
                    if med.len() >= 8 {
                        let mut synth = Map::new();
                        synth.insert("hw_curve_webgl".into(), json!(med));
                        crate::hnsw_lite::atlas_hnsw_insert(oc, &Value::Object(synth));
                    }
                }
            }
        }
    });

    let (current_fit, current_n, current_n_max, channels) = with_atlas(|a| {
        let cur = a.by_cohort.get(&ck);
        let mut per = Map::new();
        let mut wacc = 0.0;
        let mut wsum = 0.0;
        let mut n_total = 0usize;
        let mut n_max = 0usize;
        for (ch, c) in &obs {
            let (f, tn) = cur
                .and_then(|m| m.get(*ch))
                .map(|t| {
                    (
                        template_similarity_fields(Some(fields), c, &t.median_proxy()),
                        t.n,
                    )
                })
                .unwrap_or((0.0, 0));
            let w = fit_weight_adjusted(ch, has_res, has_wg);
            wacc += w * f;
            wsum += w;
            n_total += tn;
            n_max = n_max.max(tn);
            per.insert(
                (*ch).into(),
                json!({"fit": (f * 10000.0).round() / 10000.0, "template_n": tn, "weight": w}),
            );
        }
        let fit = if wsum > 0.0 { wacc / wsum } else { 0.0 };
        (
            (fit * 10000.0).round() / 10000.0,
            n_total,
            n_max,
            per,
        )
    });

    // Alt-key: HNSW nearest cohorts first, then crude fill
    let hnsw_hits = crate::hnsw_lite::atlas_hnsw_search(fields, 16);
    let use_hnsw = !hnsw_hits.is_empty()
        && match gr_abi::env::get("ATLAS_HNSW_ALT") {
            Some(v) => v != "0" && v != "false" && v != "off",
            None => true,
        };

    let (alt_key, alt_fit, alt_n, alt_method) = with_atlas(|a| {
        let mut best_key = String::new();
        let mut best_fit = 0.0f64;
        let mut best_n = 0usize;

        let mut candidates: Vec<String> = Vec::new();
        if use_hnsw {
            for (id, _dist) in &hnsw_hits {
                if id != &ck {
                    candidates.push(id.clone());
                }
            }
        }
        // Always include crude scan of up to 256 for recall safety
        for (oc, _) in a.by_cohort.iter().take(256) {
            if oc != &ck && !candidates.iter().any(|c| c == oc) {
                candidates.push(oc.clone());
            }
        }

        for oc in candidates.iter().take(320) {
            let Some(m) = a.by_cohort.get(oc) else {
                continue;
            };
            let mut wacc = 0.0;
            let mut wsum = 0.0;
            let mut nacc = 0usize;
            for (ch, c) in &obs {
                if let Some(t) = m.get(*ch) {
                    if t.n < 8 {
                        continue;
                    }
                    let f = template_similarity_fields(Some(fields), c, &t.median_proxy());
                    let w = fit_weight_adjusted(ch, has_res, has_wg);
                    wacc += w * f;
                    wsum += w;
                    nacc += t.n;
                }
            }
            if wsum > 0.0 {
                let f = wacc / wsum;
                if f > best_fit {
                    best_fit = f;
                    best_key = oc.clone();
                    best_n = nacc;
                }
            }
        }
        let method = if best_key.is_empty() {
            "none"
        } else if use_hnsw && hnsw_hits.iter().any(|(id, _)| id == &best_key) {
            "hnsw_lite"
        } else if use_hnsw {
            "hnsw+crude"
        } else {
            "crude"
        };
        (
            best_key,
            (best_fit * 10000.0).round() / 10000.0,
            best_n,
            method,
        )
    });

    let lift = if current_fit > 0.0 {
        alt_fit - current_fit
    } else {
        0.0
    };
    let explore = atlas_explore_urgency(fields);
    // Cold when no channel has ≥8 samples (max-n, not sum — multi-channel must not
    // inflate warmth from channel count alone).
    let cold = current_n_max < 8;
    json!({
        "algo": "cell_fit_shadow_v2",
        "current_key": ck,
        "current_fit": current_fit,
        "current_template_n": current_n,
        "current_template_n_max": current_n_max,
        "alt_key": alt_key,
        "alt_fit": alt_fit,
        "alt_template_n": alt_n,
        "alt_method": alt_method,
        "lift": (lift * 10000.0).round() / 10000.0,
        "channels": channels,
        "cold": cold,
        "hard_block": false,
        "fit_channels": ["res", "wg", "au", "cp"],
        "explore": explore,
        "note": "iss/73: res in fit + HNSW alt-key + explore urgency; shadow only",
    })
}

/// iss/58 B3 Phase1: simple whitening LSH of differential z (no ML).
/// Projects z onto fixed random hyperplanes (seeded) → bit digest for recall.
pub fn whitening_lsh_bits(z: &[f64], bits: usize) -> String {
    if z.is_empty() {
        return String::new();
    }
    let bits = bits.clamp(8, 64);
    let mut h = Sha256::new();
    h.update(b"whiten_lsh_v1|");
    // Fixed projection: hash-derived signs per dim
    for b in 0..bits {
        let mut acc = 0.0f64;
        for (i, &v) in z.iter().enumerate() {
            let mut hh = Sha256::new();
            hh.update(b.to_le_bytes());
            hh.update(i.to_le_bytes());
            let dig = hh.finalize();
            let sign = if dig[0] & 1 == 0 { 1.0 } else { -1.0 };
            acc += v * sign;
        }
        h.update([if acc >= 0.0 { 1u8 } else { 0u8 }]);
    }
    format!("{:x}", h.finalize())[..16].to_string()
}

/// Attach whitening LSH digests into extras map when z available.
/// Also attaches contrastive embedding digest for B3 HNSW-lite.
pub fn attach_whitening_lsh(extras: &mut Map<String, Value>, ch: &str, z: &[f64]) {
    if z.len() < 8 {
        return;
    }
    let dig = whitening_lsh_bits(z, 32);
    if !dig.is_empty() {
        extras.insert(format!("{ch}_whiten_lsh"), json!(dig));
    }
    // Contrastive embedding fingerprint (first 8 dims hex) for debug / ops
    let emb = crate::hnsw_lite::contrastive_embed(z);
    if !emb.is_empty() {
        let mut h = Sha256::new();
        h.update(b"contrast_v1|");
        for x in &emb {
            h.update(x.to_le_bytes());
        }
        extras.insert(
            format!("{ch}_contrast_emb"),
            json!(format!("{:x}", h.finalize())[..16].to_string()),
        );
        extras.insert("b3_ann".into(), json!("hnsw_lite_v1"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn differential_splits_same_template_different_delta() {
        reset_atlas_for_tests();
        // Seed template with class-similar base
        let base: Vec<f64> = (0..32).map(|i| 0.2 + i as f64 * 0.01).collect();
        for _ in 0..5 {
            let f = json!({
                "engine_family": "blink",
                "os_family": "Linux",
                "gl_stack_class": "angle",
                "hw_curve_webgl": base,
                "audio_seed_delta_curve": base,
            });
            atlas_observe(&f);
        }
        let mut a = base.clone();
        let mut b = base.clone();
        for i in 0..8 {
            a[i] += 0.05;
            b[i] -= 0.05;
        }
        let fa = json!({
            "engine_family": "blink",
            "os_family": "Linux",
            "gl_stack_class": "angle",
            "hw_curve_webgl": a,
            "audio_seed_delta_curve": a,
        });
        let fb = json!({
            "engine_family": "blink",
            "os_family": "Linux",
            "gl_stack_class": "angle",
            "hw_curve_webgl": b,
            "audio_seed_delta_curve": b,
        });
        let da = differential_encode(&fa);
        let db = differential_encode(&fb);
        let ha = da["extras"]["wg_diff_digest"].as_str().unwrap_or("");
        let hb = db["extras"]["wg_diff_digest"].as_str().unwrap_or("");
        assert!(!ha.is_empty());
        assert_ne!(ha, hb, "different deltas must fork diff digests");
    }



    #[test]
    fn cell_fit_prefers_own_key_over_alt_key() {
        with_atlas_test_lock(|| {
            reset_atlas_for_tests();
            std::env::set_var("GR_ATLAS_HNSW_ALT", "0"); // crude only for deterministic alt
            let base: Vec<f64> = (0..32).map(|i| 0.2 + i as f64 * 0.008).collect();
            let shifted: Vec<f64> = base.iter().map(|x| x + 0.35).collect();
            for _ in 0..10 {
                atlas_observe(&json!({
                    "engine_family": "blink",
                    "os_family": "Windows",
                    "hw_model_key": "nvidia:rtx_3060",
                    "gl_backend_class": "d3d11",
                    "hw_curve_webgl": base.clone(),
                    "audio_seed_delta_curve": base.clone(),
                }));
            }
            for _ in 0..10 {
                atlas_observe(&json!({
                    "engine_family": "blink",
                    "os_family": "Windows",
                    "hw_model_key": "nvidia:gtx_1050",
                    "gl_backend_class": "d3d11",
                    "hw_curve_webgl": shifted.clone(),
                    "audio_seed_delta_curve": shifted.clone(),
                }));
            }
            let q = json!({
                "engine_family": "blink",
                "os_family": "Windows",
                "hw_model_key": "nvidia:rtx_3060",
                "gl_backend_class": "d3d11",
                "hw_curve_webgl": base,
                "audio_seed_delta_curve": base,
            });
            let r = cell_fit_and_alt_key(&q);
            let cur = r["current_fit"].as_f64().unwrap();
            let alt = r["alt_fit"].as_f64().unwrap();
            assert!(cur > 0.5, "own-cell fit should be high: {r}");
            assert!(
                r["alt_key"].as_str().unwrap_or("").contains("nvidia:gtx_1050"),
                "alt key should be the shifted cell: {r}"
            );
            assert!(cur > alt, "own fit must beat alt fit: {r}");
            assert_eq!(r["hard_block"], false);
            std::env::remove_var("GR_ATLAS_HNSW_ALT");
        });
    }

    #[test]
    fn res_channel_in_fit_and_shifted_res_lowers_fit() {
        with_atlas_test_lock(|| {
            reset_atlas_for_tests();
            let base: Vec<f64> = (0..32).map(|i| 0.2 + i as f64 * 0.008).collect();
            let shifted: Vec<f64> = base.iter().map(|x| x + 0.4).collect();
            for _ in 0..12 {
                atlas_observe(&json!({
                    "engine_family": "blink",
                    "os_family": "Linux",
                    "hw_model_key": "nvidia:rtx_4090",
                    "hw_curve_webgl": base.clone(),
                    "webgl_residual_multipath": base.clone(),
                    "audio_seed_delta_curve": base.clone(),
                }));
            }
            let good = cell_fit_and_alt_key(&json!({
                "engine_family": "blink",
                "os_family": "Linux",
                "hw_model_key": "nvidia:rtx_4090",
                "hw_curve_webgl": base.clone(),
                "webgl_residual_multipath": base.clone(),
                "audio_seed_delta_curve": base.clone(),
            }));
            let bad_res = cell_fit_and_alt_key(&json!({
                "engine_family": "blink",
                "os_family": "Linux",
                "hw_model_key": "nvidia:rtx_4090",
                "hw_curve_webgl": base.clone(),
                "webgl_residual_multipath": shifted,
                "audio_seed_delta_curve": base,
            }));
            assert!(
                good["channels"].get("res").is_some(),
                "res must be a fit channel: {good}"
            );
            let gf = good["current_fit"].as_f64().unwrap();
            let bf = bad_res["current_fit"].as_f64().unwrap();
            assert!(
                gf > bf,
                "shifted res must lower fit: good={gf} bad={bf} g={good} b={bad_res}"
            );
        });
    }

    #[test]
    fn explore_urgency_high_for_cold_cell() {
        with_atlas_test_lock(|| {
            reset_atlas_for_tests();
            let u = atlas_explore_urgency(&json!({
                "engine_family": "blink",
                "os_family": "Linux",
                "hw_model_key": "nvidia:rare_gpu_xyz",
            }));
            assert_eq!(u["cold"], true);
            assert!(u["urgency"].as_f64().unwrap() >= 0.7);
            assert_eq!(u["should_deepen"], true);
            let cov = cell_coverage_report();
            assert!(cov["n_cells"].as_u64().is_some());
        });
    }

    #[test]
    fn apply_t_k_webkit_differs_from_identity() {
        let c: Vec<f64> = (0..16).map(|i| 0.1 * i as f64).collect();
        let a = apply_t_k("webkit", "gl", &c);
        let b = apply_t_k("blink", "gl", &c);
        assert_ne!(a[1], b[1]);
        let fa = canonical_v_features_for_fields(
            &json!({"engine_family":"webkit","gl_backend":"gl"}),
            &c,
        );
        assert!(!fa.is_empty());
    }
}
