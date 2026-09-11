//! Physical contradiction + plausibility assertions (iss/60 L2 + E1 + E8).
//!
//! All assertions lower confidence / force deepen — **never hard ban**.
//! Algo: `physical_contradiction_v1` · `physical_plausibility_v1` · `emulation_timing_v1`

use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Mutex;

pub const PHYSICAL_CONTRADICTION_ALGO: &str = "physical_contradiction_v1";
pub const PHYSICAL_PLAUSIBILITY_ALGO: &str = "physical_plausibility_v1";
pub const EMULATION_TIMING_ALGO: &str = "emulation_timing_v1";

// ─── Cohort envelopes for plausibility (process-local + optional shared later) ─

#[derive(Clone, Debug)]
struct Envelope {
    n: u64,
    sum: f64,
    sum_sq: f64,
    min: f64,
    max: f64,
}

impl Envelope {
    fn new() -> Self {
        Self {
            n: 0,
            sum: 0.0,
            sum_sq: 0.0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
        }
    }
    fn observe(&mut self, x: f64) {
        if !x.is_finite() {
            return;
        }
        self.n += 1;
        self.sum += x;
        self.sum_sq += x * x;
        self.min = self.min.min(x);
        self.max = self.max.max(x);
    }
    fn mean(&self) -> f64 {
        if self.n == 0 {
            0.0
        } else {
            self.sum / self.n as f64
        }
    }
    fn std(&self) -> f64 {
        if self.n < 2 {
            return 1.0;
        }
        let m = self.mean();
        ((self.sum_sq / self.n as f64) - m * m).max(0.0).sqrt().max(1e-6)
    }
    /// Outside mean ± k*std or beyond historical max*10 for timing
    fn outside(&self, x: f64, k: f64) -> bool {
        if self.n < 5 || !x.is_finite() {
            return false;
        }
        let m = self.mean();
        let s = self.std();
        (x - m).abs() > k * s
    }
}

impl Envelope {
    fn merged(&self, o: &Envelope) -> Envelope {
        if o.n == 0 {
            return self.clone();
        }
        if self.n == 0 {
            return o.clone();
        }
        Envelope {
            n: self.n + o.n,
            sum: self.sum + o.sum,
            sum_sq: self.sum_sq + o.sum_sq,
            min: self.min.min(o.min),
            max: self.max.max(o.max),
        }
    }
    fn to_arr(&self) -> [f64; 5] {
        [self.n as f64, self.sum, self.sum_sq, self.min, self.max]
    }
    fn from_arr(a: &[f64]) -> Option<Envelope> {
        if a.len() < 5 {
            return None;
        }
        Some(Envelope {
            n: a[0] as u64,
            sum: a[1],
            sum_sq: a[2],
            min: a[3],
            max: a[4],
        })
    }
}

// ─── iss/61 F3: multi-worker envelope sharing via shared_governance ─────────
// Shape: {"writers": {wid: {"ts": ms, "m": {metric: [n,sum,sq,min,max]}, "c": {cohort|metric: [...]}}}}
// Each process rewrites only its own writer entry (no double-count on reload).

const ENV_SHARED_NAME: &str = "physical_env";
const ENV_WRITER_TTL_MS: u64 = 24 * 3600 * 1000;
const ENV_SYNC_INTERVAL_MS: u64 = 2_000;

fn env_now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn env_writer_id() -> String {
    use std::sync::OnceLock;
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| format!("{}-{}", std::process::id(), env_now_ms()))
        .clone()
}

struct EnvState {
    /// metric → envelope (this process only)
    by_metric: HashMap<String, Envelope>,
    /// cohort|metric → envelope (this process only)
    by_cohort: HashMap<String, Envelope>,
    /// merged envelopes from *other* workers (shared store)
    peer_metric: HashMap<String, Envelope>,
    peer_cohort: HashMap<String, Envelope>,
    last_load_ms: u64,
    last_flush_ms: u64,
    dirty: bool,
}

impl EnvState {
    /// own + peer workers combined view for one metric.
    fn combined_metric(&self, name: &str) -> Option<Envelope> {
        let own = self.by_metric.get(name);
        let peer = self.peer_metric.get(name);
        match (own, peer) {
            (None, None) => None,
            (Some(a), None) => Some(a.clone()),
            (None, Some(b)) => Some(b.clone()),
            (Some(a), Some(b)) => Some(a.merged(b)),
        }
    }
}

/// Load peer writer envelopes from shared store into `st` (excluding self).
fn env_load_peers(st: &mut EnvState) {
    st.last_load_ms = env_now_ms();
    let Some(v) = crate::shared_governance::read_shared_json(ENV_SHARED_NAME) else {
        return;
    };
    let Some(writers) = v.get("writers").and_then(|w| w.as_object()) else {
        return;
    };
    let now = env_now_ms();
    let own = env_writer_id();
    st.peer_metric.clear();
    st.peer_cohort.clear();
    for (wid, wv) in writers {
        if *wid == own {
            continue;
        }
        let ts = wv.get("ts").and_then(|x| x.as_u64()).unwrap_or(0);
        if now.saturating_sub(ts) > ENV_WRITER_TTL_MS {
            continue;
        }
        for (section, dst) in [
            ("m", &mut st.peer_metric),
            ("c", &mut st.peer_cohort),
        ] {
            if let Some(m) = wv.get(section).and_then(|x| x.as_object()) {
                for (k, arr) in m {
                    if let Some(a) = arr.as_array() {
                        let nums: Vec<f64> = a.iter().filter_map(|x| x.as_f64()).collect();
                        if let Some(env) = Envelope::from_arr(&nums) {
                            dst.entry(k.clone())
                                .and_modify(|e| *e = e.merged(&env))
                                .or_insert(env);
                        }
                    }
                }
            }
        }
    }
}

/// Persist own envelopes under own writer id (prune stale writers).
fn env_flush_own(st: &mut EnvState) {
    st.last_flush_ms = env_now_ms();
    st.dirty = false;
    let own = env_writer_id();
    let now = env_now_ms();
    let m: Map<String, Value> = st
        .by_metric
        .iter()
        .map(|(k, e)| (k.clone(), json!(e.to_arr())))
        .collect();
    let c: Map<String, Value> = st
        .by_cohort
        .iter()
        .map(|(k, e)| (k.clone(), json!(e.to_arr())))
        .collect();
    let _ = crate::shared_governance::with_shared_json(
        ENV_SHARED_NAME,
        json!({"writers": {}}),
        |v| {
            let root = crate::shared_governance::as_object_mut(v);
            let writers = root
                .entry("writers")
                .or_insert_with(|| json!({}));
            let wobj = crate::shared_governance::as_object_mut(writers);
            // prune stale writers
            let stale: Vec<String> = wobj
                .iter()
                .filter(|(_, wv)| {
                    let ts = wv.get("ts").and_then(|x| x.as_u64()).unwrap_or(0);
                    now.saturating_sub(ts) > ENV_WRITER_TTL_MS
                })
                .map(|(k, _)| k.clone())
                .collect();
            for k in stale {
                wobj.remove(&k);
            }
            wobj.insert(
                own.clone(),
                json!({"ts": now, "m": m, "c": c}),
            );
        },
    );
}

/// Throttled sync: reload peers + flush own (called on observe path).
fn env_sync_shared(st: &mut EnvState) {
    let now = env_now_ms();
    if now.saturating_sub(st.last_load_ms) >= ENV_SYNC_INTERVAL_MS {
        env_load_peers(st);
    }
    if st.dirty && now.saturating_sub(st.last_flush_ms) >= ENV_SYNC_INTERVAL_MS {
        env_flush_own(st);
    }
}

static ENV: Mutex<Option<EnvState>> = Mutex::new(None);

fn with_env<R>(f: impl FnOnce(&mut EnvState) -> R) -> R {
    let mut g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(EnvState {
            by_metric: HashMap::new(),
            by_cohort: HashMap::new(),
            peer_metric: HashMap::new(),
            peer_cohort: HashMap::new(),
            last_load_ms: 0,
            last_flush_ms: 0,
            dirty: false,
        });
    }
    f(g.as_mut().unwrap())
}

pub fn reset_physical_env_for_tests() {
    with_env(|e| {
        e.by_metric.clear();
        e.by_cohort.clear();
        e.peer_metric.clear();
        e.peer_cohort.clear();
        e.last_load_ms = 0;
        e.last_flush_ms = 0;
        e.dirty = false;
    });
}

fn f64_field(fields: &Value, keys: &[&str]) -> Option<f64> {
    for k in keys {
        if let Some(v) = fields.get(*k).and_then(|x| x.as_f64()) {
            if v.is_finite() {
                return Some(v);
            }
        }
    }
    None
}

fn str_field(fields: &Value, keys: &[&str]) -> String {
    for k in keys {
        if let Some(s) = fields.get(*k).and_then(|x| x.as_str()) {
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    String::new()
}

fn i64_field(fields: &Value, keys: &[&str]) -> Option<i64> {
    for k in keys {
        if let Some(v) = fields.get(*k) {
            if let Some(i) = v.as_i64() {
                return Some(i);
            }
            if let Some(f) = v.as_f64() {
                return Some(f as i64);
            }
        }
    }
    None
}

fn staircase_slope(fields: &Value) -> Option<f64> {
    // Prefer explicit field, else estimate from ladder arrays
    if let Some(s) = f64_field(fields, &["gpu_wall_staircase_slope", "staircase_slope"]) {
        return Some(s);
    }
    for k in ["gpu_bandwidth_ladder", "gl_bandwidth_lite", "bw_ladder_ms"] {
        if let Some(a) = fields.get(k).and_then(|v| v.as_array()) {
            let xs: Vec<f64> = a.iter().filter_map(|x| x.as_f64()).collect();
            if xs.len() >= 2 {
                return Some((xs[xs.len() - 1] - xs[0]) / (xs.len() as f64 - 1.0));
            }
        }
    }
    None
}

/// Physical contradiction rules (claim vs measured soft/hard).
pub fn physical_contradiction(fields: &Value) -> Value {
    let mut asserts = Vec::new();
    let mut score = 0.0; // higher = more contradiction

    let cores = i64_field(
        fields,
        &["hardware_concurrency", "nav_hw_concurrency", "cores_claim"],
    );
    let mem = f64_field(fields, &["device_memory", "nav_device_memory"]);
    let touch = i64_field(fields, &["max_touch_points", "nav_max_touch"]);
    let ua = str_field(fields, &["user_agent", "ua"]);
    let platform = str_field(fields, &["platform", "nav_platform"]);
    let renderer = str_field(
        fields,
        &["webgl_unmasked_renderer", "webgl_governed_renderer", "gpu_model"],
    )
    .to_ascii_lowercase();
    let residual_std = f64_field(fields, &["residual_std", "webgl_residual_std"]);
    let sab_n = fields
        .get("sab_clock_curve")
        .or_else(|| fields.get("hw_curve_sab"))
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let vc_ok = fields.get("vc_ok").and_then(|v| v.as_bool());

    // Cores vs SAB richness
    if let Some(c) = cores {
        if c >= 16 && sab_n > 0 && sab_n < 8 {
            asserts.push(json!({
                "id": "cores_vs_sab_thin",
                "severity": "medium",
                "detail": format!("claim_cores={c} sab_curve_n={sab_n}"),
            }));
            score += 0.25;
        }
        if c >= 32 {
            if let Some(std) = residual_std {
                if std < 1e-6 {
                    asserts.push(json!({
                        "id": "high_cores_flat_residual",
                        "severity": "high",
                        "detail": "claimed many cores but residual nearly dead",
                    }));
                    score += 0.35;
                }
            }
        }
    }

    // Mem vs cores class
    if let (Some(c), Some(m)) = (cores, mem) {
        if c >= 16 && m > 0.0 && m <= 2.0 {
            asserts.push(json!({
                "id": "mem_cores_env_conflict",
                "severity": "medium",
                "detail": format!("cores={c} device_memory={m}"),
            }));
            score += 0.2;
        }
    }

    // High-end GPU claim vs weak silicon
    let claims_dgpu = renderer.contains("rtx")
        || renderer.contains("radeon rx")
        || renderer.contains("geforce")
        || renderer.contains("nvidia");
    let _claims_igpu = renderer.contains("intel")
        || renderer.contains("uhd")
        || renderer.contains("iris")
        || renderer.contains("apple m")
        || renderer.contains("adreno")
        || renderer.contains("mali");
    if claims_dgpu {
        if let Some(std) = residual_std {
            if std < 1e-5 {
                asserts.push(json!({
                    "id": "dgpu_claim_flat_residual",
                    "severity": "high",
                    "detail": "dGPU label but residual std ~0",
                }));
                score += 0.4;
            }
        }
        if vc_ok == Some(false) {
            asserts.push(json!({
                "id": "dgpu_claim_no_hw_encode",
                "severity": "medium",
                "detail": "prefer-hardware encode failed",
            }));
            score += 0.2;
        }
        if let Some(slope) = staircase_slope(fields) {
            if slope >= 0.0 && slope < 0.05 {
                asserts.push(json!({
                    "id": "dgpu_claim_flat_bandwidth",
                    "severity": "high",
                    "detail": format!("staircase_slope={slope}"),
                }));
                score += 0.35;
            }
        }
    }

    // Mobile UA vs desktop form
    let ua_l = ua.to_ascii_lowercase();
    let mobile_ua = ua_l.contains("mobile") || ua_l.contains("android") || ua_l.contains("iphone");
    let desktop_plat = platform.contains("Win") || platform.contains("Linux") || platform.contains("Mac");
    if mobile_ua && desktop_plat && touch.unwrap_or(0) == 0 {
        asserts.push(json!({
            "id": "mobile_ua_desktop_platform",
            "severity": "high",
            "detail": format!("ua mobile-ish platform={platform}"),
        }));
        score += 0.3;
    }
    if !mobile_ua && touch.unwrap_or(0) >= 5 && desktop_plat {
        // touch desktop can be real; only soft
        asserts.push(json!({
            "id": "desktop_high_touch",
            "severity": "low",
            "detail": format!("touch={}", touch.unwrap_or(0)),
        }));
        score += 0.05;
    }

    // Software GL vs hardware claims
    if renderer.contains("swiftshader")
        || renderer.contains("llvmpipe")
        || renderer.contains("softpipe")
    {
        if claims_dgpu || cores.unwrap_or(0) >= 8 {
            asserts.push(json!({
                "id": "soft_gl_vs_hw_claim",
                "severity": "high",
                "detail": "software GL with high hardware claim",
            }));
            score += 0.45;
        }
    }

    let severity = if score >= 0.7 {
        "high"
    } else if score >= 0.35 {
        "medium"
    } else if score > 0.0 {
        "low"
    } else {
        "none"
    };

    json!({
        "algo": PHYSICAL_CONTRADICTION_ALGO,
        "assertions": asserts,
        "contradiction_score": ((score as f64) * 1000.0).round() / 1000.0,
        "severity": severity,
        "action": if score >= 0.35 { "downweight_conf_force_deepen" } else { "none" },
        "hard_ban": false,
    })
}

/// Observe metrics into cohort envelopes (call on analyze).
pub fn plausibility_observe(fields: &Value) {
    let cohort = crate::population_atlas::cohort_key_from_fields(fields);
    let mut metrics: Vec<(&str, f64)> = Vec::new();
    if let Some(s) = staircase_slope(fields) {
        metrics.push(("staircase_slope", s));
    }
    if let Some(s) = f64_field(fields, &["residual_std", "webgl_residual_std"]) {
        metrics.push(("residual_std", s));
    }
    if let Some(t) = f64_field(fields, &["probe_wall_ms", "b10_wall_ms", "collect_wall_ms"]) {
        metrics.push(("probe_wall_ms", t));
    }
    if let Some(a) = fields.get("sab_clock_curve").and_then(|v| v.as_array()) {
        if a.len() >= 4 {
            let xs: Vec<f64> = a.iter().filter_map(|x| x.as_f64()).collect();
            if xs.len() >= 4 {
                let mean = xs.iter().sum::<f64>() / xs.len() as f64;
                metrics.push(("sab_mean", mean));
            }
        }
    }
    with_env(|st| {
        for (name, x) in metrics {
            st.by_metric.entry(name.into()).or_insert_with(Envelope::new).observe(x);
            let key = format!("{cohort}|{name}");
            st.by_cohort.entry(key).or_insert_with(Envelope::new).observe(x);
        }
        st.dirty = true;
        env_sync_shared(st);
    });
}

/// Claimed hardware class vs measured envelope (E1).
pub fn physical_plausibility(fields: &Value) -> Value {
    plausibility_observe(fields);
    let mut lies = Vec::new();
    let mut score = 0.0;
    let renderer = str_field(
        fields,
        &["webgl_unmasked_renderer", "webgl_governed_renderer", "gpu_model"],
    )
    .to_ascii_lowercase();
    let claims_dgpu = renderer.contains("rtx")
        || renderer.contains("geforce")
        || renderer.contains("radeon rx");

    with_env(|st| {
        if let Some(s) = staircase_slope(fields) {
            if let Some(env) = st.combined_metric("staircase_slope") {
                if claims_dgpu && env.n >= 5 {
                    let m = env.mean();
                    if s < m * 0.3 && m > 0.1 {
                        lies.push(json!({
                            "id": "hardware_claim_lie_bandwidth",
                            "claim": "dgpu",
                            "metric": "staircase_slope",
                            "observed": s,
                            "cohort_mean": m,
                        }));
                        score += 0.4;
                    }
                }
                if env.outside(s, 4.0) {
                    lies.push(json!({
                        "id": "staircase_out_of_envelope",
                        "observed": s,
                        "mean": env.mean(),
                        "std": env.std(),
                    }));
                    score += 0.15;
                }
            }
        }
        if let Some(std) = f64_field(fields, &["residual_std", "webgl_residual_std"]) {
            if claims_dgpu && std < 1e-6 {
                lies.push(json!({
                    "id": "hardware_claim_lie_residual",
                    "claim": "dgpu",
                    "observed_std": std,
                }));
                score += 0.35;
            }
        }
    });

    json!({
        "algo": PHYSICAL_PLAUSIBILITY_ALGO,
        "hardware_claim_lie": !lies.is_empty() && score >= 0.3,
        "assertions": lies,
        "score": ((score as f64) * 1000.0).round() / 1000.0,
        "action": if score >= 0.3 { "downweight_conf_force_deepen" } else { "none" },
        "hard_ban": false,
    })
}

/// Probe wall-time envelope (E8): 10× cohort max → emulation_timing_suspect.
pub fn emulation_timing(fields: &Value) -> Value {
    let mut suspects = Vec::new();
    let wall = f64_field(
        fields,
        &["probe_wall_ms", "b10_wall_ms", "collect_wall_ms", "pack_wall_ms"],
    );
    if let Some(w) = wall {
        with_env(|st| {
            st.by_metric
                .entry("probe_wall_ms".into())
                .or_insert_with(Envelope::new)
                .observe(w);
            st.dirty = true;
            env_sync_shared(st);
            if let Some(env) = st.combined_metric("probe_wall_ms") {
                if env.n >= 8 && env.max.is_finite() && env.max > 0.0 && w > env.max * 10.0 {
                    suspects.push(json!({
                        "id": "emulation_timing_suspect",
                        "wall_ms": w,
                        "cohort_max": env.max,
                        "ratio": w / env.max,
                    }));
                }
            }
        });
    }
    json!({
        "algo": EMULATION_TIMING_ALGO,
        "suspect": !suspects.is_empty(),
        "assertions": suspects,
        "action": if !suspects.is_empty() { "downweight_conf" } else { "none" },
        "hard_ban": false,
    })
}

/// iss/67 C3: four K↔V adjudications as shadow signals (never hard-ban).
/// Sources: atlas_shadow_score already computed on fields, or recompute lightly.
pub fn atlas_kv_adjudication(fields: &Value) -> Value {
    let score = fields
        .get("atlas_shadow_score")
        .cloned()
        .unwrap_or_else(|| crate::atlas_score::atlas_shadow_score(fields));
    let signals = score
        .get("signals")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let spoof = score
        .get("spoof_shadow")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || signals.iter().any(|s| {
            s.as_str()
                .map(|t| t.contains("spoof") || t.contains("mismatch"))
                .unwrap_or(false)
        });
    let farm = score
        .get("farm_tightness")
        .and_then(|v| {
            // farm_tightness may be object {tight:bool} or numeric proxy
            if let Some(b) = v.get("tight").and_then(|t| t.as_bool()) {
                Some(b)
            } else {
                v.as_f64().map(|x| x > 0.7)
            }
        })
        .unwrap_or(false)
        || signals
            .iter()
            .any(|s| s.as_str().map(|t| t.contains("farm")).unwrap_or(false));
    let zero_si = score
        .get("zero_silicon_shadow")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || signals
            .iter()
            .any(|s| s.as_str().map(|t| t.contains("zero_silicon")).unwrap_or(false));
    let replay = score
        .get("replay_exact_shadow")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || signals
            .iter()
            .any(|s| s.as_str().map(|t| t.contains("replay")).unwrap_or(false));
    let mut flags = Vec::new();
    if spoof {
        flags.push("key_value_mismatch");
    }
    if farm {
        flags.push("farm_tightness");
    }
    if zero_si {
        flags.push("zero_silicon_structure");
    }
    if replay {
        flags.push("challenge_replay");
    }
    // Soft demote weight (shadow): each flag +0.12, capped 0.48 — never hard ban
    let demote = (flags.len() as f64 * 0.12).min(0.48);
    json!({
        "algo": "atlas_kv_adjudication_v1",
        "shadow": true,
        "hard_block": false,
        "flags": flags,
        "spoof": spoof,
        "farm_tight": farm,
        "zero_silicon": zero_si,
        "replay_exact": replay,
        "demote_weight": demote,
        "should_downweight": demote >= 0.24,
        "should_deepen": demote >= 0.24,
        "atlas_shadow_score": score,
        "note": "iss/67 C3 — four adjudications shadow only; no hard-block",
    })
}

/// Combined physical assert surface for evaluate.
pub fn physical_assert_surface(fields: &Value) -> Value {
    let c = physical_contradiction(fields);
    let p = physical_plausibility(fields);
    let t = emulation_timing(fields);
    let atlas_kv = atlas_kv_adjudication(fields);
    let atlas_demote = atlas_kv
        .get("demote_weight")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let score = c.get("contradiction_score").and_then(|v| v.as_f64()).unwrap_or(0.0)
        + p.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0)
        + if t.get("suspect").and_then(|v| v.as_bool()).unwrap_or(false) {
            0.3
        } else {
            0.0
        }
        + atlas_demote;
    json!({
        "algo": "physical_assert_bundle_v1",
        "contradiction": c,
        "plausibility": p,
        "timing": t,
        "atlas_kv": atlas_kv,
        "bundle_score": (score * 1000.0).round() / 1000.0,
        "should_downweight": score >= 0.35,
        "should_deepen": score >= 0.35,
        "hard_ban": false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// iss/61 F3: peer worker envelopes merge into the combined view via shared store.
    #[test]
    fn peer_envelope_merges_via_shared_store() {
        let _guard = crate::shared_governance::ISS58_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _dir = crate::shared_governance::test_isolation_dir("physenv");
        reset_physical_env_for_tests();
        // Fake peer writer: staircase_slope n=10, sum=50 (mean 5)
        crate::shared_governance::with_shared_json(
            ENV_SHARED_NAME,
            json!({"writers": {}}),
            |v| {
                let root = crate::shared_governance::as_object_mut(v);
                root.insert(
                    "writers".into(),
                    json!({
                        "peer-writer-1": {
                            "ts": env_now_ms(),
                            "m": {"staircase_slope": [10.0, 50.0, 300.0, 3.0, 8.0]},
                            "c": {}
                        }
                    }),
                );
            },
        );
        // Own observation: one sample at 5.0
        plausibility_observe(&json!({"gpu_wall_staircase_slope": 5.0}));
        with_env(|st| {
            env_load_peers(st);
            let c = st.combined_metric("staircase_slope").expect("combined");
            assert_eq!(c.n, 11, "own 1 + peer 10, got {c:?}");
            assert!((c.mean() - (55.0 / 11.0)).abs() < 1e-9, "mean={}", c.mean());
            // Own map must stay own-only (no double count on flush)
            assert_eq!(st.by_metric.get("staircase_slope").map(|e| e.n), Some(1));
        });
        reset_physical_env_for_tests();
    }

    #[test]
    fn soft_gl_with_dgpu_claim_flags() {
        reset_physical_env_for_tests();
        let f = json!({
            "webgl_unmasked_renderer": "Google SwiftShader",
            "hardware_concurrency": 16,
            "residual_std": 0.0,
        });
        let r = physical_contradiction(&f);
        assert!(r["contradiction_score"].as_f64().unwrap() > 0.3, "{r}");
        assert_ne!(r["severity"], "none");
    }

    #[test]
    fn real_ish_fields_low_score() {
        reset_physical_env_for_tests();
        let f = json!({
            "webgl_unmasked_renderer": "ANGLE (Intel, Mesa)",
            "hardware_concurrency": 8,
            "device_memory": 8,
            "residual_std": 0.02,
            "platform": "Linux x86_64",
            "user_agent": "Mozilla/5.0 Chrome/140.0.0.0",
        });
        let r = physical_contradiction(&f);
        assert!(r["contradiction_score"].as_f64().unwrap() < 0.35, "{r}");
    }

    #[test]
    fn atlas_kv_adjudication_shadow_no_hard_block() {
        reset_physical_env_for_tests();
        let f = json!({
            "webgl_unmasked_renderer": "Google SwiftShader",
            "atlas_shadow_score": {
                "spoof_shadow": true,
                "zero_silicon_shadow": true,
                "replay_exact_shadow": false,
                "farm_tightness": {"tight": true},
                "signals": ["model_silicon_mismatch_shadow", "farm_tightness_shadow", "zero_silicon_shadow"]
            }
        });
        let a = atlas_kv_adjudication(&f);
        assert_eq!(a["hard_block"], false);
        assert_eq!(a["shadow"], true);
        assert!(a["demote_weight"].as_f64().unwrap() >= 0.24, "{a}");
        let flags = a["flags"].as_array().unwrap();
        assert!(flags.iter().any(|x| x.as_str() == Some("key_value_mismatch")));
        let surface = physical_assert_surface(&f);
        assert!(surface.get("atlas_kv").is_some());
        assert_eq!(surface["hard_ban"], false);
    }
}
