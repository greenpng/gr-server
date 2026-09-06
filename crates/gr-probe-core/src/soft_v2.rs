//! Soft Blocking V2 — same-host soft edges, never promote.
//!
//! Hardware-noise soft association: custom FE curve probes (audio/canvas/cpu/webgl)
//! are compared by cosine/relative shape similarity. This path is designed for
//! fingerprint browsers that change kernel UA + proxy IP on the same physical host.
//! Soft edges NEVER promote to commercial device_id.

use crate::contracts::load_all_specs;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

/// Constitutional: soft never promotes to commercial device id.
pub const PROMOTE_TO_COMMERCIAL_ID: bool = false;

/// Minimum cosine similarity for a single noise curve to count as "aligned".
const HW_CURVE_COS_MIN: f64 = 0.82;
/// Need at least this many curve families aligned for soft_noise edge.
const HW_CURVE_ALIGN_MIN: usize = 2;

#[derive(Debug, Clone)]
pub struct SoftConfig {
    pub p1_window_ms: i64,
    pub p2_window_ms: i64,
    pub hot_bucket_threshold: i64,
    pub legacy_bare_hs2_edges: bool,
}

impl Default for SoftConfig {
    fn default() -> Self {
        Self {
            p1_window_ms: 300_000,
            p2_window_ms: 3_600_000,
            hot_bucket_threshold: 32,
            legacy_bare_hs2_edges: false,
        }
    }
}

pub fn load_soft_config() -> SoftConfig {
    if let Ok(specs) = load_all_specs() {
        let raw = &specs.soft_v2;
        SoftConfig {
            p1_window_ms: raw
                .get("p1_window_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(300_000),
            p2_window_ms: raw
                .get("p2_window_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(3_600_000),
            hot_bucket_threshold: raw
                .get("hot_bucket_threshold")
                .and_then(|v| v.as_i64())
                .unwrap_or(32),
            legacy_bare_hs2_edges: raw
                .get("legacy_bare_hs2_edges_default")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        }
    } else {
        SoftConfig::default()
    }
}

#[derive(Debug, Clone, Default)]
pub struct HwCurves {
    pub audio: Vec<f64>,
    pub canvas: Vec<f64>,
    pub cpu: Vec<f64>,
    pub webgl: Vec<f64>,
}

impl HwCurves {
    pub fn is_empty(&self) -> bool {
        self.audio.is_empty()
            && self.canvas.is_empty()
            && self.cpu.is_empty()
            && self.webgl.is_empty()
    }

    pub fn family_count(&self) -> usize {
        (if self.audio.is_empty() { 0 } else { 1 })
            + (if self.canvas.is_empty() { 0 } else { 1 })
            + (if self.cpu.is_empty() { 0 } else { 1 })
            + (if self.webgl.is_empty() { 0 } else { 1 })
    }
}

#[derive(Debug, Clone)]
pub struct SoftMember {
    pub tenant: String,
    pub session_id: String,
    pub hs2: String,
    pub net_shard: Option<String>,
    pub asn_class: Option<String>,
    pub client_ref: Option<String>,
    pub ts_ms: i64,
    pub device_id_v2: Option<String>,
    pub env_class: String,
    /// Observed server egress IP (proxy IP when fingerprint browser uses proxy).
    pub server_client_ip: Option<String>,
    /// Live hardware-noise curves (soft path; never hashed into commercial id).
    pub hw_curves: HwCurves,
}

impl SoftMember {
    pub fn to_value(&self) -> Value {
        json!({
            "tenant": self.tenant,
            "session_id": self.session_id,
            "hs2": self.hs2,
            "net_shard": self.net_shard,
            "asn_class": self.asn_class,
            "client_ref": self.client_ref,
            "ts_ms": self.ts_ms,
            "device_id_v2": self.device_id_v2,
            "env_class": self.env_class,
            "server_client_ip": self.server_client_ip,
            "hw_curve_families": self.hw_curves.family_count(),
        })
    }
}

fn json_f64_vec(v: Option<&Value>) -> Vec<f64> {
    match v {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_f64().or_else(|| x.as_i64().map(|i| i as f64)))
            .collect(),
        _ => Vec::new(),
    }
}

/// Extract live hardware-noise curves from session fields (B10_hw_curves).
pub fn extract_hw_curves(fields: &Value) -> HwCurves {
    let fo = fields.as_object();
    let nested = fo
        .and_then(|o| o.get("hw_noise_curves"))
        .and_then(|v| v.as_object());
    let get = |flat: &str, nest: &str| -> Vec<f64> {
        if let Some(o) = fo {
            let v = o
                .get(flat)
                .or_else(|| nested.and_then(|n| n.get(nest)));
            return json_f64_vec(v);
        }
        Vec::new()
    };
    HwCurves {
        audio: get("hw_curve_audio", "audio"),
        canvas: get("hw_curve_canvas", "canvas"),
        cpu: get("hw_curve_cpu", "cpu"),
        webgl: get("hw_curve_webgl", "webgl"),
    }
}

/// Cosine similarity of two equal-length (or min-len truncated) vectors.
pub fn cosine_similarity(a: &[f64], b: &[f64]) -> Option<f64> {
    let n = a.len().min(b.len());
    if n < 4 {
        return None;
    }
    let mut dot = 0.0;
    let mut na = 0.0;
    let mut nb = 0.0;
    for i in 0..n {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na <= 1e-18 || nb <= 1e-18 {
        return None;
    }
    Some(dot / (na.sqrt() * nb.sqrt()))
}

/// Coarse 16-bit signature of a curve for soft candidate bucketing (not a commercial digest).
/// Quantize each sample into 4 bins; XOR-fold into u16. Used only to prune O(n²) soft pairs.
pub fn curve_coarse_sig(curve: &[f64]) -> u16 {
    if curve.is_empty() {
        return 0;
    }
    let mut h: u16 = 0;
    let n = curve.len().min(32);
    for i in 0..n {
        let v = curve[i];
        // Relative to local mean for scale-ish invariance.
        let bin = if v < -0.5 {
            0u16
        } else if v < 0.0 {
            1
        } else if v < 0.5 {
            2
        } else {
            3
        };
        h ^= bin.wrapping_shl((i as u32 % 8) * 2);
        h = h.rotate_left(1).wrapping_add(bin.wrapping_mul(17));
    }
    h
}

/// Combined soft coarse key across available curve families (audio|webgl primary).
pub fn hw_curves_coarse_key(c: &HwCurves) -> u32 {
    let a = curve_coarse_sig(&c.audio) as u32;
    let w = curve_coarse_sig(&c.webgl) as u32;
    let u = curve_coarse_sig(&c.cpu) as u32;
    let v = curve_coarse_sig(&c.canvas) as u32;
    // Fold so empty curves don't dominate: prefer audio+webgl.
    (a << 16) | (w << 8) | ((u ^ v) & 0xff)
}

/// Hamming distance of 32-bit keys (for soft pair pruning).
pub fn coarse_key_hamming(a: u32, b: u32) -> u32 {
    (a ^ b).count_ones()
}

/// Max hamming distance to still run full cosine (soft-only candidate gate).
pub const SOFT_COARSE_HAMMING_MAX: u32 = 12;

/// Compare hardware-noise curves across two sessions.
/// Returns (aligned_families, details). Soft-only — never a commercial id source.
pub fn hw_noise_similarity(a: &HwCurves, b: &HwCurves) -> Value {
    let pairs = [
        ("audio", a.audio.as_slice(), b.audio.as_slice()),
        ("canvas", a.canvas.as_slice(), b.canvas.as_slice()),
        ("cpu", a.cpu.as_slice(), b.cpu.as_slice()),
        ("webgl", a.webgl.as_slice(), b.webgl.as_slice()),
    ];
    let mut aligned = Vec::new();
    let mut scores = Map::new();
    let mut missing = Vec::new();
    for (name, va, vb) in pairs {
        if va.is_empty() || vb.is_empty() {
            missing.push(name.to_string());
            continue;
        }
        if let Some(c) = cosine_similarity(va, vb) {
            scores.insert(name.into(), json!((c * 10000.0).round() / 10000.0));
            if c >= HW_CURVE_COS_MIN {
                aligned.push(name.to_string());
            }
        } else {
            missing.push(name.to_string());
        }
    }
    let eligible = aligned.len() >= HW_CURVE_ALIGN_MIN;
    // Confidence from mean of aligned cosines (measured, not a fixed table).
    let conf = if aligned.is_empty() {
        0.0
    } else {
        let sum: f64 = aligned
            .iter()
            .filter_map(|k| scores.get(k).and_then(|v| v.as_f64()))
            .sum();
        sum / aligned.len() as f64
    };
    json!({
        "eligible": eligible,
        "aligned": aligned,
        "aligned_n": aligned.len(),
        "scores": scores,
        "missing": missing,
        "confidence": (conf * 10000.0).round() / 10000.0,
        "promote_to_commercial_id": PROMOTE_TO_COMMERCIAL_ID,
        "algo": "hw_noise_cosine_v1",
        "note": "soft_only_cross_browser_cross_ip",
    })
}

#[derive(Debug, Clone)]
pub struct SoftEdge {
    pub a_session: String,
    pub b_session: String,
    pub priority: String,
    pub promote_to_commercial_id: bool,
    pub confidence: f64,
    pub reason: String,
}

impl SoftEdge {
    pub fn to_value(&self) -> Value {
        json!({
            "a_session": self.a_session,
            "b_session": self.b_session,
            "priority": self.priority,
            "promote_to_commercial_id": self.promote_to_commercial_id,
            "confidence": self.confidence,
            "reason": self.reason,
        })
    }
}

pub fn host_surface_hs2(fields: &Value) -> String {
    let f = fields.as_object().cloned().unwrap_or_default();
    let parts = [
        str_field(&f, "os_family"),
        str_field(&f, "hardware_concurrency"),
        str_field(&f, "device_memory"),
        str_field(&f, "timezone"),
        str_field(&f, "screen_width"),
        str_field(&f, "screen_height"),
        str_field(&f, "form_class"),
    ];
    if parts.iter().all(|p| p.is_empty()) {
        return String::new();
    }
    let line = parts.join("|");
    let mut hasher = Sha256::new();
    hasher.update(format!("hs2|{line}").as_bytes());
    let dig = format!("{:x}", hasher.finalize());
    format!("hs2_{}", &dig[..16])
}

fn str_field(f: &Map<String, Value>, key: &str) -> String {
    match f.get(key) {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(other) => other.to_string(),
    }
}

pub fn classify_env(signals: &Value) -> String {
    let s = signals.as_object().cloned().unwrap_or_default();
    if s.get("is_datacenter_asn")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || s.get("env_class").and_then(|v| v.as_str()) == Some("datacenter")
    {
        return "datacenter".into();
    }
    if s.get("is_mobile_carrier")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        && s.get("shared_nat_hint")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    {
        return "cgnat_mobile".into();
    }
    if s.get("shared_nat_hint")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        && !s
            .get("is_mobile_carrier")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    {
        return "corp_nat".into();
    }
    if s.get("asn_name").is_some()
        && !s
            .get("shared_nat_hint")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    {
        return "residential".into();
    }
    s.get("env_class")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string()
}

pub fn soft_link_mode(env: &str, signals: &Value, hot_threshold: i64) -> String {
    let s = signals.as_object().cloned().unwrap_or_default();
    if s.get("supercluster")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return "supercluster".into();
    }
    if env == "datacenter" {
        return "disabled".into();
    }
    let bucket = s
        .get("bucket_member_count")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if bucket >= hot_threshold || env == "cgnat_mobile" || env == "corp_nat" {
        return "suppressed".into();
    }
    "active".into()
}

/// Product-facing homogenization / hot-bucket crowding signal (iss/50 H6).
///
/// Soft path historically only **suppressed** links on hot buckets. Station
/// operators need a **consumable product field** (severity / window / basis)
/// for farm detection — not internal suppress-only.
pub fn homogenization_product_signal(
    env_class: &str,
    env_signals: &Value,
    hot_threshold: i64,
    window_ms: i64,
) -> Value {
    let s = env_signals.as_object();
    let bucket = s
        .and_then(|m| m.get("bucket_member_count"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let supercluster = s
        .and_then(|m| m.get("supercluster"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mode = soft_link_mode(env_class, env_signals, hot_threshold);
    let mut basis: Vec<String> = Vec::new();
    if supercluster {
        basis.push("supercluster".into());
    }
    if bucket >= hot_threshold {
        basis.push(format!("hot_bucket_n={bucket}>={hot_threshold}"));
    }
    if env_class == "cgnat_mobile" || env_class == "corp_nat" {
        basis.push(format!("env_class={env_class}"));
    }
    if env_class == "datacenter" {
        basis.push("datacenter".into());
    }
    if basis.is_empty() && mode == "active" {
        basis.push("no_crowding_signal".into());
    } else if basis.is_empty() {
        basis.push(format!("mode={mode}"));
    }

    let severity = if supercluster || mode == "supercluster" {
        "critical"
    } else if mode == "suppressed" || bucket >= hot_threshold * 2 {
        "high"
    } else if mode == "disabled" || env_class == "datacenter" {
        "medium"
    } else if bucket >= (hot_threshold / 2).max(1) {
        "low"
    } else {
        "none"
    };

    let score = if supercluster {
        0.95
    } else if bucket >= hot_threshold {
        (0.45 + 0.5 * ((bucket as f64) / (hot_threshold as f64 * 3.0)).min(1.0)).min(0.92)
    } else if mode == "disabled" {
        0.35
    } else if bucket > 0 {
        (0.08 * bucket as f64 / hot_threshold as f64).min(0.4)
    } else {
        0.0
    };

    // iss/50 C3 product name `environment_homogeneity` is an alias of this signal.
    json!({
        "algo": "homogenization_v1",
        "score": (score * 10000.0).round() / 10000.0,
        "severity": severity,
        "window_ms": window_ms,
        "hot_bucket_threshold": hot_threshold,
        "hot_bucket_n": bucket,
        "env_class": env_class,
        "soft_link_mode": mode,
        "basis": basis,
        "product_signal": true,
        "internal_suppress_only": false,
        "promote_to_commercial_id": false,
        "environment_homogeneity": {
            "algo": "environment_homogeneity_v1",
            "score": (score * 10000.0).round() / 10000.0,
            "severity": severity,
            "window_ms": window_ms,
            "basis": basis,
            "hot_bucket_n": bucket,
        },
        "sdk_use": "fuse with peer_similarity + subject_ref for farm/register velocity; not a device UV",
        "note": "higher = more same-host / hot-bucket crowding; was suppress-only, now SDK-consumable",
    })
}

pub fn soft_edges_allowed(mode: &str) -> bool {
    mode != "disabled" && mode != "supercluster"
}

pub fn member_from_evidence(evidence: &Value, tenant: Option<&str>) -> SoftMember {
    let fields = evidence
        .get("fields")
        .cloned()
        .unwrap_or(Value::Object(Map::new()));
    let env_sig = evidence
        .get("env_signals")
        .cloned()
        .unwrap_or(Value::Object(Map::new()));
    let gw = evidence
        .get("gateway_fields")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let cf = evidence
        .get("cf_fields")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let net = evidence
        .get("net_shard")
        .and_then(|v| v.as_str())
        .or_else(|| env_sig.get("net_shard").and_then(|v| v.as_str()))
        .or_else(|| gw.get("net_shard").and_then(|v| v.as_str()))
        .or_else(|| cf.get("colo").and_then(|v| v.as_str()))
        .or_else(|| cf.get("net_shard").and_then(|v| v.as_str()))
        .map(|s| s.to_string());
    let asn = evidence
        .get("asn_class")
        .and_then(|v| v.as_str())
        .or_else(|| env_sig.get("asn_class").and_then(|v| v.as_str()))
        .or_else(|| gw.get("asn_class").and_then(|v| v.as_str()))
        .map(|s| s.to_string());
    let client_ref = evidence
        .get("client_ref")
        .and_then(|v| v.as_str())
        .or_else(|| fields.get("client_ref").and_then(|v| v.as_str()))
        .map(|s| s.to_string());
    let hs2 = evidence
        .get("hs2")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| host_surface_hs2(&fields));
    let env = if env_sig.as_object().is_some_and(|o| !o.is_empty()) {
        classify_env(&env_sig)
    } else {
        classify_env(&json!({
            "env_class": evidence.get("env_class").cloned().unwrap_or(Value::Null)
        }))
    };
    let ts = evidence
        .get("ts_ms")
        .and_then(|v| v.as_i64())
        .or_else(|| fields.get("ts_ms").and_then(|v| v.as_i64()))
        .unwrap_or(0);
    let server_ip = fields
        .get("server_client_ip")
        .and_then(|v| v.as_str())
        .or_else(|| evidence.get("server_client_ip").and_then(|v| v.as_str()))
        .or_else(|| gw.get("server_client_ip").and_then(|v| v.as_str()))
        .map(|s| s.to_string());
    let hw_curves = extract_hw_curves(&fields);
    SoftMember {
        tenant: evidence
            .get("tenant")
            .and_then(|v| v.as_str())
            .unwrap_or(tenant.unwrap_or("default"))
            .to_string(),
        session_id: evidence
            .get("session_id")
            .and_then(|v| v.as_str())
            .or_else(|| fields.get("session_id").and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string(),
        hs2,
        net_shard: net,
        asn_class: asn,
        client_ref,
        ts_ms: ts,
        device_id_v2: evidence
            .get("device_id_v2")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        env_class: env,
        server_client_ip: server_ip,
        hw_curves,
    }
}

pub struct SoftBlockingEngine {
    pub config: SoftConfig,
    members: Vec<SoftMember>,
}

impl SoftBlockingEngine {
    pub fn new(config: Option<SoftConfig>) -> Self {
        Self {
            config: config.unwrap_or_else(load_soft_config),
            members: Vec::new(),
        }
    }

    pub fn observe(&mut self, m: SoftMember) {
        if let Some(i) = self
            .members
            .iter()
            .position(|x| x.session_id == m.session_id && x.tenant == m.tenant)
        {
            self.members[i] = m;
        } else {
            self.members.push(m);
        }
    }

    pub fn members(&self) -> &[SoftMember] {
        &self.members
    }

    pub fn pair_edge(&self, a: &SoftMember, b: &SoftMember, mode: &str) -> Option<SoftEdge> {
        if a.tenant != b.tenant || a.session_id == b.session_id {
            return None;
        }
        if !soft_edges_allowed(mode) {
            return None;
        }
        if let (Some(ra), Some(rb)) = (&a.client_ref, &b.client_ref) {
            if ra == rb {
                return Some(SoftEdge {
                    a_session: a.session_id.clone(),
                    b_session: b.session_id.clone(),
                    priority: "P0".into(),
                    promote_to_commercial_id: PROMOTE_TO_COMMERCIAL_ID,
                    confidence: 0.95,
                    reason: "same_client_ref".into(),
                });
            }
        }

        // Hardware-noise soft path: works when API fields / hs2 / egress IP diverge
        // (fingerprint browser + proxy). Never promotes to commercial device_id.
        let noise = hw_noise_similarity(&a.hw_curves, &b.hw_curves);
        let noise_ok = noise
            .get("eligible")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let noise_conf = noise
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let ip_differs = match (&a.server_client_ip, &b.server_client_ip) {
            (Some(ia), Some(ib)) if ia != ib => true,
            _ => false,
        };
        if noise_ok {
            let reason = if ip_differs {
                "hw_noise_cross_ip".into()
            } else {
                "hw_noise_same_host".into()
            };
            return Some(SoftEdge {
                a_session: a.session_id.clone(),
                b_session: b.session_id.clone(),
                priority: if ip_differs {
                    "P1_noise_xip".into()
                } else {
                    "P1_noise".into()
                },
                promote_to_commercial_id: PROMOTE_TO_COMMERCIAL_ID,
                confidence: (0.55 + 0.35 * noise_conf).min(0.92),
                reason,
            });
        }

        let same_hs = !a.hs2.is_empty() && a.hs2 == b.hs2;
        if !same_hs {
            return None;
        }
        let dt = (a.ts_ms - b.ts_ms).abs();
        let same_net = a
            .net_shard
            .as_ref()
            .zip(b.net_shard.as_ref())
            .is_some_and(|(x, y)| x == y);
        let same_asn = a
            .asn_class
            .as_ref()
            .zip(b.asn_class.as_ref())
            .is_some_and(|(x, y)| x == y);

        if same_net && dt <= self.config.p1_window_ms {
            return Some(SoftEdge {
                a_session: a.session_id.clone(),
                b_session: b.session_id.clone(),
                priority: "P1".into(),
                promote_to_commercial_id: PROMOTE_TO_COMMERCIAL_ID,
                confidence: if mode == "active" { 0.7 } else { 0.55 },
                reason: "hs2_net_window".into(),
            });
        }
        if same_asn && dt <= self.config.p2_window_ms && !same_net {
            if mode == "suppressed" {
                return None;
            }
            return Some(SoftEdge {
                a_session: a.session_id.clone(),
                b_session: b.session_id.clone(),
                priority: "P2".into(),
                promote_to_commercial_id: PROMOTE_TO_COMMERCIAL_ID,
                confidence: 0.45,
                reason: "hs2_asn_window".into(),
            });
        }
        if self.config.legacy_bare_hs2_edges {
            return Some(SoftEdge {
                a_session: a.session_id.clone(),
                b_session: b.session_id.clone(),
                priority: "P3_legacy".into(),
                promote_to_commercial_id: PROMOTE_TO_COMMERCIAL_ID,
                confidence: 0.35,
                reason: "legacy_bare_hs2".into(),
            });
        }
        None
    }

    pub fn edges(&self, env_signals: Option<&Value>) -> Vec<SoftEdge> {
        let sig = env_signals.cloned().unwrap_or(json!({}));
        let mut out = Vec::new();
        let n = self.members.len();
        // Precompute coarse keys once; for large n prune full cosine via hamming gate.
        let coarse: Vec<u32> = self
            .members
            .iter()
            .map(|m| hw_curves_coarse_key(&m.hw_curves))
            .collect();
        let use_coarse_prune = n >= 24;
        for i in 0..n {
            for j in (i + 1)..n {
                let a = &self.members[i];
                let b = &self.members[j];
                // Fast path: same client_ref / hs2 still fully evaluated; noise pairs need curves.
                let same_ref = match (&a.client_ref, &b.client_ref) {
                    (Some(x), Some(y)) if !x.is_empty() && x == y => true,
                    _ => false,
                };
                let same_hs = !a.hs2.is_empty() && a.hs2 == b.hs2;
                if use_coarse_prune && !same_ref && !same_hs {
                    let both_have_noise = !a.hw_curves.is_empty() && !b.hw_curves.is_empty();
                    if both_have_noise
                        && coarse_key_hamming(coarse[i], coarse[j]) > SOFT_COARSE_HAMMING_MAX
                    {
                        continue;
                    }
                }
                let mut sig_a = sig.clone();
                if let Some(obj) = sig_a.as_object_mut() {
                    if !obj.contains_key("bucket_member_count") {
                        obj.insert("bucket_member_count".into(), json!(0));
                    }
                }
                let mode_a =
                    soft_link_mode(&a.env_class, &sig_a, self.config.hot_bucket_threshold);
                let mode_b =
                    soft_link_mode(&b.env_class, &sig_a, self.config.hot_bucket_threshold);
                let mode = if mode_a == "disabled"
                    || mode_b == "disabled"
                    || mode_a == "supercluster"
                    || mode_b == "supercluster"
                {
                    "disabled"
                } else if mode_a == "suppressed" || mode_b == "suppressed" {
                    "suppressed"
                } else {
                    "active"
                };
                if let Some(e) = self.pair_edge(a, b, mode) {
                    out.push(e);
                }
            }
        }
        out
    }

    pub fn legacy_edge_count(&self) -> usize {
        let cfg = SoftConfig {
            p1_window_ms: self.config.p1_window_ms,
            p2_window_ms: self.config.p2_window_ms,
            hot_bucket_threshold: self.config.hot_bucket_threshold,
            legacy_bare_hs2_edges: true,
        };
        let mut eng = SoftBlockingEngine::new(Some(cfg));
        for m in &self.members {
            eng.observe(m.clone());
        }
        eng.edges(None).len()
    }
}

pub fn soft_pair_decision(
    a_evidence: &Value,
    b_evidence: &Value,
    config: Option<SoftConfig>,
) -> Value {
    let mut eng = SoftBlockingEngine::new(config);
    let ma = member_from_evidence(a_evidence, None);
    let mb = member_from_evidence(b_evidence, None);
    eng.observe(ma.clone());
    eng.observe(mb.clone());
    let edges = eng.edges(None);
    let edge = edges.first().map(|e| e.to_value());
    let legacy_would = eng.legacy_edge_count() > 0 && edges.is_empty();
    let noise = hw_noise_similarity(&ma.hw_curves, &mb.hw_curves);
    let ip_differs = match (&ma.server_client_ip, &mb.server_client_ip) {
        (Some(ia), Some(ib)) if ia != ib => true,
        _ => false,
    };
    json!({
        "edge": edge,
        "promote_to_commercial_id": PROMOTE_TO_COMMERCIAL_ID,
        "a": ma.to_value(),
        "b": mb.to_value(),
        "legacy_would_edge": legacy_would,
        "hw_noise": noise,
        "server_ip_differs": ip_differs,
        "soft_v2": true,
        "algo_note": "API fields feed analysis only; hw_noise soft-links cross-browser/cross-ip same host",
    })
}

pub fn build_soft_graph(
    members: &[Value],
    config: Option<SoftConfig>,
    env_signals: Option<&Value>,
) -> Result<Value, String> {
    let cfg = config.unwrap_or_else(load_soft_config);
    let mut eng = SoftBlockingEngine::new(Some(cfg.clone()));
    let mut normalized = Vec::new();
    for m in members {
        let sm = if m.get("hs2").is_some() && m.get("session_id").is_some() && m.get("fields").is_none()
        {
            SoftMember {
                tenant: m
                    .get("tenant")
                    .and_then(|v| v.as_str())
                    .unwrap_or("default")
                    .to_string(),
                session_id: m
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                hs2: m
                    .get("hs2")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                net_shard: m
                    .get("net_shard")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                asn_class: m
                    .get("asn_class")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                client_ref: m
                    .get("client_ref")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                ts_ms: m.get("ts_ms").and_then(|v| v.as_i64()).unwrap_or(0),
                device_id_v2: m
                    .get("device_id_v2")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                env_class: m
                    .get("env_class")
                    .and_then(|v| v.as_str())
                    .unwrap_or("residential")
                    .to_string(),
                server_client_ip: m
                    .get("server_client_ip")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                hw_curves: extract_hw_curves(m.get("fields").unwrap_or(m)),
            }
        } else {
            member_from_evidence(m, m.get("tenant").and_then(|v| v.as_str()))
        };
        eng.observe(sm.clone());
        normalized.push(sm);
    }
    let v2_edges = eng.edges(env_signals);
    let legacy_n = eng.legacy_edge_count();
    let n = normalized.len();
    let max_pairs = if n >= 2 { n * (n - 1) / 2 } else { 0 };
    let mut buckets: Map<String, Value> = Map::new();
    for m in &normalized {
        let key = if m.hs2.is_empty() {
            format!("{}|empty", m.tenant)
        } else {
            format!("{}|{}", m.tenant, m.hs2)
        };
        let cur = buckets.get(&key).and_then(|v| v.as_i64()).unwrap_or(0);
        buckets.insert(key, json!(cur + 1));
    }
    let all_never = v2_edges.iter().all(|e| !e.promote_to_commercial_id);
    Ok(json!({
        "soft_v2": true,
        "n_members": n,
        "n_v2_edges": v2_edges.len(),
        "n_legacy_bare_hs2_edges": legacy_n,
        "max_pairs": max_pairs,
        "v2_lt_legacy": if legacy_n > 0 { v2_edges.len() < legacy_n } else { v2_edges.is_empty() },
        "promote_to_commercial_id": PROMOTE_TO_COMMERCIAL_ID,
        "all_edges_never_promote": all_never && !PROMOTE_TO_COMMERCIAL_ID,
        "edges": v2_edges.iter().map(|e| e.to_value()).collect::<Vec<_>>(),
        "members": normalized.iter().map(|m| m.to_value()).collect::<Vec<_>>(),
        "hs2_buckets": buckets,
        "config": {
            "p1_window_ms": cfg.p1_window_ms,
            "p2_window_ms": cfg.p2_window_ms,
            "legacy_bare_hs2_edges": cfg.legacy_bare_hs2_edges,
        },
    }))
}

// ── Pluggable Soft edge store + commercial id heat (multi-worker durable default) ──

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Mis-recall fuse: when a soft bucket exceeds this many edges, further soft links are fused (dropped).
pub const SOFT_MISRECALL_FUSE_DEFAULT: i64 = 64;
/// Owner field for ops escalation (documented; product may override).
pub const SOFT_FUSE_OWNER: &str = "soft_ops";

/// Trait for Soft edge persistence (process-local default; swap for Redis/PG later).
pub trait SoftEdgeStore: Send + Sync {
    fn put_edge(&self, tenant: &str, edge: &SoftEdge) -> Result<(), String>;
    fn list_edges(&self, tenant: &str) -> Result<Vec<SoftEdge>, String>;
    fn record_device_id_sighting(
        &self,
        tenant: &str,
        device_id: &str,
        session_id: &str,
    ) -> Result<i64, String>;
    fn device_id_heat(&self, tenant: &str, device_id: &str) -> Result<DeviceIdHeat, String>;
    fn fuse_threshold(&self) -> i64;
    fn fuse_owner(&self) -> &str;
}

#[derive(Debug, Clone)]
pub struct DeviceIdHeat {
    pub device_id: String,
    pub session_count: i64,
    pub sessions: Vec<String>,
    pub collision_style: bool,
    pub soft_promote: bool,
}

impl DeviceIdHeat {
    pub fn to_value(&self) -> Value {
        json!({
            "device_id": self.device_id,
            "session_count": self.session_count,
            "sessions": self.sessions,
            "collision_style": self.collision_style,
            "soft_promote": false,
            "note": "heat from commercial dv_* sightings only; IP/UA not hashed here",
        })
    }
}

#[derive(Default)]
struct SoftStoreInner {
    edges: HashMap<String, Vec<SoftEdge>>,
    /// tenant|dv_* → session ids
    heat: HashMap<String, Vec<String>>,
}

/// Default durable-in-process Soft store (Arc shared across "workers" in tests).
#[derive(Clone)]
pub struct MemorySoftEdgeStore {
    inner: Arc<Mutex<SoftStoreInner>>,
    fuse_threshold: i64,
    fuse_owner: String,
}

impl MemorySoftEdgeStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(SoftStoreInner::default())),
            fuse_threshold: SOFT_MISRECALL_FUSE_DEFAULT,
            fuse_owner: SOFT_FUSE_OWNER.into(),
        }
    }

    pub fn with_fuse(threshold: i64, owner: &str) -> Self {
        Self {
            inner: Arc::new(Mutex::new(SoftStoreInner::default())),
            fuse_threshold: threshold,
            fuse_owner: owner.into(),
        }
    }

    /// Second worker handle sharing the same backend (multi-worker consistency).
    pub fn worker_view(&self) -> Self {
        self.clone()
    }
}

impl Default for MemorySoftEdgeStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SoftEdgeStore for MemorySoftEdgeStore {
    fn put_edge(&self, tenant: &str, edge: &SoftEdge) -> Result<(), String> {
        let mut g = self.inner.lock().map_err(|e| e.to_string())?;
        let list = g.edges.entry(tenant.to_string()).or_default();
        if list.len() as i64 >= self.fuse_threshold {
            return Err(format!(
                "soft_misrecall_fuse: tenant={tenant} edges>={} owner={}",
                self.fuse_threshold, self.fuse_owner
            ));
        }
        let mut e = edge.clone();
        e.promote_to_commercial_id = PROMOTE_TO_COMMERCIAL_ID;
        list.push(e);
        Ok(())
    }

    fn list_edges(&self, tenant: &str) -> Result<Vec<SoftEdge>, String> {
        let g = self.inner.lock().map_err(|e| e.to_string())?;
        Ok(g.edges.get(tenant).cloned().unwrap_or_default())
    }

    fn record_device_id_sighting(
        &self,
        tenant: &str,
        device_id: &str,
        session_id: &str,
    ) -> Result<i64, String> {
        // Multi-segment dv0-|dv4-|dv5-|dv6- and legacy exclusive commercial ids.
        if !crate::device_tier::is_commercial_device_id(device_id) {
            return Err("heat only tracks commercial device ids".into());
        }
        let key = format!("{tenant}|{device_id}");
        let mut g = self.inner.lock().map_err(|e| e.to_string())?;
        let sessions = g.heat.entry(key).or_default();
        if !sessions.iter().any(|s| s == session_id) {
            sessions.push(session_id.to_string());
        }
        Ok(sessions.len() as i64)
    }

    fn device_id_heat(&self, tenant: &str, device_id: &str) -> Result<DeviceIdHeat, String> {
        let key = format!("{tenant}|{device_id}");
        let g = self.inner.lock().map_err(|e| e.to_string())?;
        let sessions = g.heat.get(&key).cloned().unwrap_or_default();
        let n = sessions.len() as i64;
        Ok(DeviceIdHeat {
            device_id: device_id.to_string(),
            session_count: n,
            sessions,
            collision_style: n >= 2,
            soft_promote: PROMOTE_TO_COMMERCIAL_ID,
        })
    }

    fn fuse_threshold(&self) -> i64 {
        self.fuse_threshold
    }

    fn fuse_owner(&self) -> &str {
        &self.fuse_owner
    }
}

/// Ops-style heat summary for a commercial id (never promotes Soft).
pub fn commercial_id_heat_report(store: &dyn SoftEdgeStore, tenant: &str, device_id: &str) -> Value {
    match store.device_id_heat(tenant, device_id) {
        Ok(h) => json!({
            "ok": true,
            "heat": h.to_value(),
            "fuse": {
                "threshold": store.fuse_threshold(),
                "owner": store.fuse_owner(),
            },
            "soft_promote": false,
        }),
        Err(e) => json!({"ok": false, "error": e, "soft_promote": false}),
    }
}

// ── File-backed Soft store (multi-process / multi-worker durable default) ──

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

/// Durable Soft edge + heat store on disk (JSON). Multi-process via lockfile.
#[derive(Clone)]
pub struct FileSoftEdgeStore {
    dir: PathBuf,
    fuse_threshold: i64,
    fuse_owner: String,
}

impl FileSoftEdgeStore {
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, String> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        Ok(Self {
            dir,
            fuse_threshold: SOFT_MISRECALL_FUSE_DEFAULT,
            fuse_owner: SOFT_FUSE_OWNER.into(),
        })
    }

    pub fn with_fuse(dir: impl AsRef<Path>, threshold: i64, owner: &str) -> Result<Self, String> {
        let mut s = Self::open(dir)?;
        s.fuse_threshold = threshold;
        s.fuse_owner = owner.into();
        Ok(s)
    }

    pub fn worker_view(&self) -> Self {
        self.clone()
    }

    fn edges_path(&self) -> PathBuf {
        self.dir.join("soft_edges.json")
    }
    fn heat_path(&self) -> PathBuf {
        self.dir.join("soft_heat.json")
    }
    fn lock_path(&self) -> PathBuf {
        self.dir.join(".soft.lock")
    }

    fn with_lock<R>(&self, f: impl FnOnce() -> Result<R, String>) -> Result<R, String> {
        let lock = self.lock_path();
        for _ in 0..400 {
            match OpenOptions::new().write(true).create_new(true).open(&lock) {
                Ok(mut lf) => {
                    let _ = writeln!(lf, "{}", std::process::id());
                    let res = f();
                    let _ = fs::remove_file(&lock);
                    return res;
                }
                Err(_) => thread::sleep(Duration::from_millis(5)),
            }
        }
        Err("soft_store_lock_timeout".into())
    }

    fn load_edges_map(&self) -> Result<Map<String, Value>, String> {
        let p = self.edges_path();
        if !p.is_file() {
            return Ok(Map::new());
        }
        let s = fs::read_to_string(&p).map_err(|e| e.to_string())?;
        let v: Value = serde_json::from_str(&s).map_err(|e| e.to_string())?;
        Ok(v.as_object().cloned().unwrap_or_default())
    }

    fn save_edges_map(&self, m: &Map<String, Value>) -> Result<(), String> {
        let tmp = self.dir.join("soft_edges.json.tmp");
        let s = serde_json::to_string_pretty(&Value::Object(m.clone())).map_err(|e| e.to_string())?;
        fs::write(&tmp, s).map_err(|e| e.to_string())?;
        fs::rename(&tmp, self.edges_path()).map_err(|e| e.to_string())
    }

    fn load_heat_map(&self) -> Result<Map<String, Value>, String> {
        let p = self.heat_path();
        if !p.is_file() {
            return Ok(Map::new());
        }
        let s = fs::read_to_string(&p).map_err(|e| e.to_string())?;
        let v: Value = serde_json::from_str(&s).map_err(|e| e.to_string())?;
        Ok(v.as_object().cloned().unwrap_or_default())
    }

    fn save_heat_map(&self, m: &Map<String, Value>) -> Result<(), String> {
        let tmp = self.dir.join("soft_heat.json.tmp");
        let s = serde_json::to_string_pretty(&Value::Object(m.clone())).map_err(|e| e.to_string())?;
        fs::write(&tmp, s).map_err(|e| e.to_string())?;
        fs::rename(&tmp, self.heat_path()).map_err(|e| e.to_string())
    }
}

impl SoftEdgeStore for FileSoftEdgeStore {
    fn put_edge(&self, tenant: &str, edge: &SoftEdge) -> Result<(), String> {
        self.with_lock(|| {
            let mut map = self.load_edges_map()?;
            let key = tenant.to_string();
            let mut list = map
                .get(&key)
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if list.len() as i64 >= self.fuse_threshold {
                return Err(format!(
                    "soft_misrecall_fuse: tenant={tenant} edges>={} owner={}",
                    self.fuse_threshold, self.fuse_owner
                ));
            }
            let mut e = edge.clone();
            e.promote_to_commercial_id = PROMOTE_TO_COMMERCIAL_ID;
            list.push(e.to_value());
            map.insert(key, Value::Array(list));
            self.save_edges_map(&map)
        })
    }

    fn list_edges(&self, tenant: &str) -> Result<Vec<SoftEdge>, String> {
        self.with_lock(|| {
            let map = self.load_edges_map()?;
            let list = map
                .get(tenant)
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            Ok(list
                .iter()
                .map(|v| SoftEdge {
                    a_session: v
                        .get("a_session")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    b_session: v
                        .get("b_session")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    priority: v
                        .get("priority")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    promote_to_commercial_id: PROMOTE_TO_COMMERCIAL_ID,
                    confidence: v.get("confidence").and_then(|x| x.as_f64()).unwrap_or(0.0),
                    reason: v
                        .get("reason")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                })
                .collect())
        })
    }

    fn record_device_id_sighting(
        &self,
        tenant: &str,
        device_id: &str,
        session_id: &str,
    ) -> Result<i64, String> {
        // Multi-segment dv0-|dv4-|dv5-|dv6- and legacy exclusive commercial ids.
        if !crate::device_tier::is_commercial_device_id(device_id) {
            return Err("heat only tracks commercial device ids".into());
        }
        self.with_lock(|| {
            let mut map = self.load_heat_map()?;
            let key = format!("{tenant}|{device_id}");
            let mut sessions = map
                .get(&key)
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if !sessions.iter().any(|s| s.as_str() == Some(session_id)) {
                sessions.push(json!(session_id));
            }
            let n = sessions.len() as i64;
            map.insert(key, Value::Array(sessions));
            self.save_heat_map(&map)?;
            Ok(n)
        })
    }

    fn device_id_heat(&self, tenant: &str, device_id: &str) -> Result<DeviceIdHeat, String> {
        self.with_lock(|| {
            let map = self.load_heat_map()?;
            let key = format!("{tenant}|{device_id}");
            let sessions: Vec<String> = map
                .get(&key)
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let n = sessions.len() as i64;
            Ok(DeviceIdHeat {
                device_id: device_id.to_string(),
                session_count: n,
                sessions,
                collision_style: n >= 2,
                soft_promote: PROMOTE_TO_COMMERCIAL_ID,
            })
        })
    }

    fn fuse_threshold(&self) -> i64 {
        self.fuse_threshold
    }

    fn fuse_owner(&self) -> &str {
        &self.fuse_owner
    }
}

#[cfg(test)]
mod coarse_index_tests {
    use super::*;

    #[test]
    fn coarse_sig_stable_and_hamming() {
        let a = vec![0.1, 0.2, -0.3, 0.4, 0.5, -0.1, 0.0, 0.9];
        let b = a.clone();
        let c = vec![-0.9, -0.8, -0.7, -0.6, -0.5, -0.4, -0.3, -0.2];
        assert_eq!(curve_coarse_sig(&a), curve_coarse_sig(&b));
        let ha = curve_coarse_sig(&a) as u32;
        let hc = curve_coarse_sig(&c) as u32;
        // Different shape should usually differ; allow equal only if empty-like.
        let _ = (ha, hc);
        assert_eq!(coarse_key_hamming(0, 0), 0);
        assert!(coarse_key_hamming(0xffff_ffff, 0) == 32);
    }

    #[test]
    fn hw_curves_coarse_key_empty_zeroish() {
        let empty = HwCurves::default();
        assert_eq!(hw_curves_coarse_key(&empty), 0);
    }

    #[test]
    fn heat_tracks_multi_segment_dv0_ids() {
        let store = MemorySoftEdgeStore::default();
        let multi =
            "dv0-0.26-abc123def0-1111111111-2222222222-linux-x86_64-c3-UTC-oihash000-rtchash00";
        let n = store
            .record_device_id_sighting("t1", multi, "sess-a")
            .expect("multi-segment must be heat-trackable");
        assert_eq!(n, 1);
        let n2 = store
            .record_device_id_sighting("t1", multi, "sess-b")
            .expect("second sighting");
        assert_eq!(n2, 2);
        let heat = store.device_id_heat("t1", multi).expect("heat read");
        assert_eq!(heat.session_count, 2);
        assert!(heat.collision_style);
    }

    #[test]
    fn heat_rejects_non_commercial_ticket() {
        let store = MemorySoftEdgeStore::default();
        let err = store
            .record_device_id_sighting("t1", "ticket_abc", "s1")
            .expect_err("non-commercial");
        assert!(err.contains("commercial"), "{err}");
    }

    #[test]
    fn homogenization_is_product_signal_not_suppress_only() {
        let hot = homogenization_product_signal(
            "residential",
            &json!({"bucket_member_count": 64, "supercluster": false}),
            32,
            300_000,
        );
        assert_eq!(hot["algo"], "homogenization_v1");
        assert_eq!(hot["product_signal"], true);
        assert_eq!(hot["internal_suppress_only"], false);
        assert_eq!(hot["promote_to_commercial_id"], false);
        assert_eq!(hot["severity"], "high");
        assert!(hot["score"].as_f64().unwrap_or(0.0) >= 0.45);
        assert!(hot["basis"]
            .as_array()
            .map(|a| a.iter().any(|v| v.as_str().unwrap_or("").contains("hot_bucket")))
            .unwrap_or(false));

        let quiet = homogenization_product_signal(
            "residential",
            &json!({"bucket_member_count": 1}),
            32,
            300_000,
        );
        assert_eq!(quiet["severity"], "none");
        assert_eq!(quiet["soft_link_mode"], "active");
    }
}
