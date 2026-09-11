//! Weakly-supervised contrastive embedding (iss/58 B3 residual).
//!
//! Trains a linear projection `W ∈ R^{d×d}` on top of the unsupervised base
//! embed so that same-host pairs get high cosine, different-host pairs low.
//!
//! Label sources (no GPU / multi-SKU required for engineering MVP):
//! 1. Offline `[{same_host|same_device, fields_a, fields_b}]` (LabeledPair-compatible)
//! 2. Online weak: same `device_id` re-observation = positive; class-floor collide
//!    with different host seps = hard negative
//!
//! Weights persist via `shared_governance` (`contrastive_w.json`) and process memory.

use crate::hnsw_lite::{embed_base_from_fields, EMB_DIM};
use crate::shared_governance::{
    as_object_mut, shared_governance_dir, with_shared_json,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;

/// Deep 2-layer MLP by default (iss/60 R3); linear fallback via GR_CONTRASTIVE_DEEP=0.
pub const CONTRASTIVE_SUP_ALGO: &str = "contrastive_sup_deep_v2";
const HIDDEN: usize = 64;
const MARGIN: f32 = 0.35;
const LR: f32 = 0.05;
const LR_OFFLINE: f32 = 0.12;
const MAX_ONLINE_BUF: usize = 256;

fn deep_enabled() -> bool {
    !matches!(
        gr_abi::env::get("CONTRASTIVE_DEEP")
            .unwrap_or_else(|| "1".into())
            .to_ascii_lowercase()
            .as_str(),
        "0" | "false" | "off" | "no" | "linear"
    )
}

/// Deep encoder: x → ReLU(W1 x + b1) → W2 h + b2 → L2 normalize.
/// Also keeps linear W for backward-compatible load.
struct Proj {
    /// linear residual path (d×d) — always applied as residual add after deep
    w_lin: Vec<f32>,
    w1: Vec<f32>, // HIDDEN * d
    b1: Vec<f32>, // HIDDEN
    w2: Vec<f32>, // d * HIDDEN
    b2: Vec<f32>, // d
    trained_steps: u64,
    n_pos: u64,
    n_neg: u64,
    last_loss: f32,
    loaded: bool,
    deep: bool,
}

impl Proj {
    fn identity() -> Self {
        let d = EMB_DIM;
        let h = HIDDEN;
        let mut w_lin = vec![0.0f32; d * d];
        for i in 0..d {
            w_lin[i * d + i] = 1.0;
        }
        // Xavier-ish init for deep layers
        let mut w1 = vec![0.0f32; h * d];
        let mut w2 = vec![0.0f32; d * h];
        let scale1 = (2.0f32 / d as f32).sqrt() * 0.5;
        let scale2 = (2.0f32 / h as f32).sqrt() * 0.5;
        for i in 0..h * d {
            let t = ((i * 1103515245 + 12345) & 0xffff) as f32 / 65535.0;
            w1[i] = (t - 0.5) * 2.0 * scale1;
        }
        for i in 0..d * h {
            let t = ((i * 1664525 + 1013904223) & 0xffff) as f32 / 65535.0;
            w2[i] = (t - 0.5) * 2.0 * scale2;
        }
        // Start deep near-zero residual so identity linear dominates until trained
        for x in w2.iter_mut() {
            *x *= 0.01;
        }
        Self {
            w_lin,
            w1,
            b1: vec![0.0; h],
            w2,
            b2: vec![0.0; d],
            trained_steps: 0,
            n_pos: 0,
            n_neg: 0,
            last_loss: 0.0,
            loaded: false,
            deep: deep_enabled(),
        }
    }

    fn apply(&self, x: &[f32]) -> Vec<f32> {
        let d = EMB_DIM;
        // linear path
        let mut y_lin = vec![0.0f32; d];
        for i in 0..d {
            let mut acc = 0.0f32;
            for j in 0..d.min(x.len()) {
                acc += self.w_lin[i * d + j] * x[j];
            }
            y_lin[i] = acc;
        }
        if !self.deep {
            l2_normalize(&mut y_lin);
            return y_lin;
        }
        let h = HIDDEN;
        let mut hid = vec![0.0f32; h];
        for i in 0..h {
            let mut acc = self.b1[i];
            for j in 0..d.min(x.len()) {
                acc += self.w1[i * d + j] * x[j];
            }
            hid[i] = acc.max(0.0); // ReLU
        }
        let mut y = vec![0.0f32; d];
        for i in 0..d {
            let mut acc = self.b2[i];
            for j in 0..h {
                acc += self.w2[i * h + j] * hid[j];
            }
            // residual: deep + linear
            y[i] = acc + y_lin[i];
        }
        l2_normalize(&mut y);
        y
    }
}

struct OnlineBuf {
    /// device_id → last base embed
    last: HashMap<String, Vec<f32>>,
    /// class body key (slots 0-7) → device ids (for hard negs)
    by_class: HashMap<String, Vec<String>>,
}

impl OnlineBuf {
    fn new() -> Self {
        Self {
            last: HashMap::new(),
            by_class: HashMap::new(),
        }
    }
}

static PROJ: Mutex<Option<Proj>> = Mutex::new(None);
static BUF: Mutex<Option<OnlineBuf>> = Mutex::new(None);

fn with_proj<R>(f: impl FnOnce(&mut Proj) -> R) -> R {
    let mut g = PROJ.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(Proj::identity());
        if let Some(ref mut p) = *g {
            load_proj(p);
            // Local warm-start: synthetic same-SKU fleet (no multi-machine needed).
            // On when GR_CONTRASTIVE_WARM_FLEET=1, or shared governance lab dir is set
            // and env is not explicitly 0.
            let warm_env = gr_abi::env::get("CONTRASTIVE_WARM_FLEET")
                .unwrap_or_default()
                .to_ascii_lowercase();
            let warm = match warm_env.as_str() {
                "1" | "true" | "on" | "yes" => true,
                "0" | "false" | "off" | "no" => false,
                _ => shared_governance_dir().is_some(),
            };
            if p.trained_steps == 0 && warm {
                // Inline warm fit without re-entering with_proj
                let pairs = synthetic_fleet_pairs(16);
                let epochs = 4;
                for _ in 0..epochs {
                    for pair in &pairs {
                        let same = pair
                            .get("same_host")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        let a = pair.get("fields_a").cloned().unwrap_or(json!({}));
                        let b = pair.get("fields_b").cloned().unwrap_or(json!({}));
                        let xa = embed_base_from_fields(&a);
                        let xb = embed_base_from_fields(&b);
                        if xa.iter().all(|x| *x == 0.0) || xb.iter().all(|x| *x == 0.0) {
                            continue;
                        }
                        let _ = sgd_pair(p, &xa, &xb, same);
                    }
                }
                persist_proj(p);
            }
            p.loaded = true;
        }
    }
    f(g.as_mut().unwrap())
}

fn with_buf<R>(f: impl FnOnce(&mut OnlineBuf) -> R) -> R {
    let mut g = BUF.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(OnlineBuf::new());
    }
    f(g.as_mut().unwrap())
}

pub fn reset_contrastive_for_tests() {
    // Skip warm-fleet during unit isolation
    std::env::set_var("GR_CONTRASTIVE_WARM_FLEET", "0");
    let mut g = PROJ.lock().unwrap_or_else(|e| e.into_inner());
    *g = Some(Proj::identity());
    if let Some(ref mut p) = *g {
        p.loaded = true; // prevent auto warm on next with_proj
    }
    let mut b = BUF.lock().unwrap_or_else(|e| e.into_inner());
    *b = Some(OnlineBuf::new());
}

fn l2_normalize(v: &mut [f32]) {
    let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n < 1e-9 {
        return;
    }
    for x in v.iter_mut() {
        *x /= n;
    }
}

/// Project base embedding through learned W (identity if untrained).
pub fn project_embed(base: &[f32]) -> Vec<f32> {
    with_proj(|p| p.apply(base))
}

/// Full embed from fields: base unsupervised → supervised projection.
pub fn embed_supervised_from_fields(fields: &Value) -> Vec<f32> {
    let base = embed_base_from_fields(fields);
    if base.iter().all(|x| *x == 0.0) {
        return base;
    }
    project_embed(&base)
}

fn class_body_key(device_id: &str) -> String {
    // first 8 slot digests of dv0-... body
    let body = device_id
        .strip_prefix("dv0-")
        .or_else(|| device_id.strip_prefix("dve-"))
        .unwrap_or(device_id);
    let parts: Vec<&str> = body.split('-').collect();
    if parts.len() >= 8 {
        parts[..8].join("-")
    } else {
        body.to_string()
    }
}

/// SGD step on one pair (base embeddings already unit-ish).
fn sgd_pair_lr(p: &mut Proj, xa: &[f32], xb: &[f32], same: bool, lr: f32) -> f32 {
    let d = EMB_DIM;
    let ya = p.apply(xa);
    let yb = p.apply(xb);
    let mut cos = 0.0f32;
    for i in 0..d {
        cos += ya[i] * yb[i];
    }
    cos = cos.clamp(-1.0, 1.0);

    let (loss, scale) = if same {
        ((1.0 - cos).max(0.0), -1.0f32)
    } else {
        let l = (cos - MARGIN).max(0.0);
        let s = if cos > MARGIN { 1.0f32 } else { 0.0 };
        (l, s)
    };
    if scale == 0.0 && !same {
        return 0.0;
    }

    // Linear path is primary (anti-collapse); deep is residual with lower LR.
    let lr_lin = lr;
    let lr_deep = lr * 0.35;
    for i in 0..d {
        for j in 0..d {
            let g = scale
                * (yb[i] * xa.get(j).copied().unwrap_or(0.0)
                    + ya[i] * xb.get(j).copied().unwrap_or(0.0));
            p.w_lin[i * d + j] -= lr_lin * g;
        }
    }
    // Mild diagonal pull toward identity to avoid collapse
    for i in 0..d {
        p.w_lin[i * d + i] += lr_lin * 0.01 * (1.0 - p.w_lin[i * d + i]);
    }
    if p.deep {
        let h = HIDDEN;
        let mut ha = vec![0.0f32; h];
        let mut hb = vec![0.0f32; h];
        for i in 0..h {
            let mut aa = p.b1[i];
            let mut ab = p.b1[i];
            for j in 0..d.min(xa.len()) {
                aa += p.w1[i * d + j] * xa[j];
            }
            for j in 0..d.min(xb.len()) {
                ab += p.w1[i * d + j] * xb[j];
            }
            ha[i] = aa.max(0.0);
            hb[i] = ab.max(0.0);
        }
        for i in 0..d {
            let gya = scale * yb[i];
            let gyb = scale * ya[i];
            p.b2[i] -= lr_deep * (gya + gyb) * 0.5;
            for j in 0..h {
                p.w2[i * h + j] -= lr_deep * (gya * ha[j] + gyb * hb[j]) * 0.5;
            }
        }
        for j in 0..h {
            let mut gha = 0.0f32;
            let mut ghb = 0.0f32;
            for i in 0..d {
                gha += scale * yb[i] * p.w2[i * h + j];
                ghb += scale * ya[i] * p.w2[i * h + j];
            }
            if ha[j] <= 0.0 {
                gha = 0.0;
            }
            if hb[j] <= 0.0 {
                ghb = 0.0;
            }
            p.b1[j] -= lr_deep * (gha + ghb) * 0.5;
            for k in 0..d {
                let xa_k = xa.get(k).copied().unwrap_or(0.0);
                let xb_k = xb.get(k).copied().unwrap_or(0.0);
                p.w1[j * d + k] -= lr_deep * (gha * xa_k + ghb * xb_k) * 0.5;
            }
        }
        for w in p.w1.iter_mut().chain(p.w2.iter_mut()) {
            *w = w.clamp(-2.0, 2.0);
        }
    }
    for w in p.w_lin.iter_mut() {
        *w = w.clamp(-4.0, 4.0);
    }
    p.trained_steps = p.trained_steps.saturating_add(1);
    if same {
        p.n_pos = p.n_pos.saturating_add(1);
    } else {
        p.n_neg = p.n_neg.saturating_add(1);
    }
    p.last_loss = loss;
    loss
}

fn sgd_pair(p: &mut Proj, xa: &[f32], xb: &[f32], same: bool) -> f32 {
    sgd_pair_lr(p, xa, xb, same, LR)
}

/// Offline fit from fusion-style labeled pairs.
pub fn fit_contrastive_from_pairs(pairs: &[Value]) -> Value {
    if pairs.is_empty() {
        return json!({
            "algo": CONTRASTIVE_SUP_ALGO,
            "ok": false,
            "error": "empty_pairs",
        });
    }
    let mut used = 0usize;
    let mut loss_sum = 0.0f32;
    with_proj(|p| {
        // Offline: more epochs + higher LR for small synthetic fleets
        let epochs = if pairs.len() < 80 { 40 } else { 12 };
        for _ in 0..epochs {
            for pair in pairs {
                let same = pair
                    .get("same_host")
                    .or_else(|| pair.get("same_device"))
                    .and_then(|v| v.as_bool());
                let Some(same) = same else { continue };
                let a = pair
                    .get("fields_a")
                    .or_else(|| pair.get("a"))
                    .cloned()
                    .unwrap_or(json!({}));
                let b = pair
                    .get("fields_b")
                    .or_else(|| pair.get("b"))
                    .cloned()
                    .unwrap_or(json!({}));
                let a = a.get("fields").cloned().unwrap_or(a);
                let b = b.get("fields").cloned().unwrap_or(b);
                let xa = embed_base_from_fields(&a);
                let xb = embed_base_from_fields(&b);
                if xa.iter().all(|x| *x == 0.0) || xb.iter().all(|x| *x == 0.0) {
                    continue;
                }
                let l = sgd_pair_lr(p, &xa, &xb, same, LR_OFFLINE);
                loss_sum += l;
                used += 1;
            }
        }
        persist_proj(p);
    });
    json!({
        "algo": CONTRASTIVE_SUP_ALGO,
        "ok": used > 0,
        "pairs_steps": used,
        "mean_loss": if used > 0 { loss_sum / used as f32 } else { 0.0 },
        "stats": contrastive_stats(),
    })
}

/// Online weak supervision when a device is registered.
pub fn observe_contrastive_online(device_id: &str, fields: &Value) {
    if device_id.is_empty() {
        return;
    }
    let base = embed_base_from_fields(fields);
    if base.iter().all(|x| *x == 0.0) {
        return;
    }
    let ck = class_body_key(device_id);

    // Positive: re-observation of same device
    let prev = with_buf(|b| b.last.get(device_id).cloned());
    if let Some(prev_emb) = prev {
        with_proj(|p| {
            let _ = sgd_pair(p, &prev_emb, &base, true);
        });
    }

    // Hard negative: other devices sharing class floor (slots 0-7)
    let hard_negs: Vec<Vec<f32>> = with_buf(|b| {
        let mut out = Vec::new();
        if let Some(ids) = b.by_class.get(&ck) {
            for id in ids.iter().rev().take(4) {
                if id == device_id {
                    continue;
                }
                if let Some(e) = b.last.get(id) {
                    out.push(e.clone());
                }
            }
        }
        out
    });
    if !hard_negs.is_empty() {
        with_proj(|p| {
            for neg in &hard_negs {
                let _ = sgd_pair(p, &base, neg, false);
            }
            if p.trained_steps % 16 == 0 {
                persist_proj(p);
            }
        });
    }

    with_buf(|b| {
        b.last.insert(device_id.to_string(), base);
        if b.last.len() > MAX_ONLINE_BUF {
            // drop arbitrary oldest-ish: clear half keys
            let drop: Vec<String> = b.last.keys().take(MAX_ONLINE_BUF / 2).cloned().collect();
            for k in drop {
                b.last.remove(&k);
            }
        }
        let v = b.by_class.entry(ck).or_default();
        if !v.iter().any(|x| x == device_id) {
            v.push(device_id.to_string());
        }
        if v.len() > 64 {
            v.drain(0..v.len() - 64);
        }
    });
}

/// Build synthetic fleet pairs (same-SKU class floor) for local fit.
pub fn synthetic_fleet_pairs(n: usize) -> Vec<Value> {
    let n = n.clamp(4, 64);
    let class = "deadclass01";
    let mut fields = Vec::new();
    let mut ids = Vec::new();
    for i in 0..n {
        let oi = format!("oi{:08x}", 0x1000u32 + i as u32);
        let rtc = format!("rt{:08x}", 0x2000u32 + i as u32 * 3);
        let id = format!(
            "dv0-{c}-{c}-{c}-{c}-{c}-{c}-{c}-{c}-{oi}-{rtc}",
            c = class,
            oi = oi,
            rtc = rtc
        );
        // Shared class floor + per-device residual (separable but same SKU class)
        let mut curve: Vec<f64> = (0..32)
            .map(|j| {
                let class = 0.26 + (j as f64) * 0.001;
                let dev = ((i + 1) as f64) * 0.015 * ((j as f64 * 0.7).sin());
                class + dev
            })
            .collect();
        curve[i % 32] += 0.05 * (i as f64 + 1.0);
        let audio: Vec<f64> = (0..32)
            .map(|j| 0.1 + j as f64 * 0.0005 + ((i + 1) as f64) * 0.012 * ((j as f64).cos()))
            .collect();
        let f = json!({
            "hw_curve_webgl": curve,
            "hw_curve_audio": audio,
            "os_instance_hash": oi,
            "webrtc_host_ip_hash": rtc,
            "wg_whiten_lsh": format!("{:016x}", (i as u64).wrapping_mul(0x9e3779b97f4a7c15)),
        });
        ids.push(id);
        fields.push(f);
    }
    let mut pairs = Vec::new();
    // Same-device re-obs: small noise = positive
    for i in 0..n {
        let mut f2 = fields[i].clone();
        if let Some(arr) = f2.get_mut("hw_curve_webgl").and_then(|v| v.as_array_mut()) {
            for (k, v) in arr.iter_mut().enumerate() {
                if let Some(x) = v.as_f64() {
                    *v = json!(x + 0.0005 * ((k % 3) as f64 - 1.0));
                }
            }
        }
        pairs.push(json!({
            "same_host": true,
            "same_device": true,
            "fields_a": fields[i],
            "fields_b": f2,
            "label_source": "synthetic_reobs",
        }));
    }
    // Cross-device same SKU class = negative
    for i in 0..n {
        for j in (i + 1)..n.min(i + 4) {
            pairs.push(json!({
                "same_host": false,
                "same_device": false,
                "fields_a": fields[i],
                "fields_b": fields[j],
                "label_source": "synthetic_sku_hardneg",
            }));
        }
    }
    pairs
}

/// Fit on synthetic fleet (local machine, no multi-SKU hardware).
pub fn fit_contrastive_synthetic_fleet(n: usize) -> Value {
    let pairs = synthetic_fleet_pairs(n);
    let mut out = fit_contrastive_from_pairs(&pairs);
    if let Some(o) = out.as_object_mut() {
        o.insert("synthetic_fleet_n".into(), json!(n));
        o.insert("pairs_n".into(), json!(pairs.len()));
    }
    out
}

pub fn contrastive_stats() -> Value {
    with_proj(|p| {
        json!({
            "algo": CONTRASTIVE_SUP_ALGO,
            "emb_dim": EMB_DIM,
            "hidden": HIDDEN,
            "deep": p.deep,
            "trained_steps": p.trained_steps,
            "n_pos": p.n_pos,
            "n_neg": p.n_neg,
            "last_loss": p.last_loss,
            "is_identity": p.trained_steps == 0,
            "shared": shared_governance_dir().is_some(),
            "l2": crate::shared_l2::l2_enabled(),
        })
    })
}

fn persist_proj(p: &Proj) {
    if shared_governance_dir().is_none()
        && !crate::shared_l2::l2_enabled()
        && p.trained_steps == 0
    {
        return;
    }
    if shared_governance_dir().is_none() && !crate::shared_l2::l2_enabled() {
        return;
    }
    let _ = with_shared_json("contrastive_w", json!({}), |v| {
        let o = as_object_mut(v);
        o.insert("algo".into(), json!(CONTRASTIVE_SUP_ALGO));
        o.insert("emb_dim".into(), json!(EMB_DIM));
        o.insert("hidden".into(), json!(HIDDEN));
        o.insert("deep".into(), json!(p.deep));
        o.insert("w".into(), json!(p.w_lin)); // legacy key
        o.insert("w_lin".into(), json!(p.w_lin));
        o.insert("w1".into(), json!(p.w1));
        o.insert("b1".into(), json!(p.b1));
        o.insert("w2".into(), json!(p.w2));
        o.insert("b2".into(), json!(p.b2));
        o.insert("trained_steps".into(), json!(p.trained_steps));
        o.insert("n_pos".into(), json!(p.n_pos));
        o.insert("n_neg".into(), json!(p.n_neg));
    });
}

fn load_vec(v: &Value, key: &str) -> Vec<f32> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_f64().map(|f| f as f32))
                .collect()
        })
        .unwrap_or_default()
}

fn load_proj(p: &mut Proj) {
    if shared_governance_dir().is_none() && !crate::shared_l2::l2_enabled() {
        return;
    }
    let loaded = with_shared_json("contrastive_w", json!({}), |v| v.clone());
    let Some(v) = loaded else {
        return;
    };
    let w_lin = {
        let w = load_vec(&v, "w_lin");
        if w.len() == EMB_DIM * EMB_DIM {
            w
        } else {
            load_vec(&v, "w")
        }
    };
    if w_lin.len() == EMB_DIM * EMB_DIM {
        p.w_lin = w_lin;
    }
    let w1 = load_vec(&v, "w1");
    let w2 = load_vec(&v, "w2");
    let b1 = load_vec(&v, "b1");
    let b2 = load_vec(&v, "b2");
    if w1.len() == HIDDEN * EMB_DIM {
        p.w1 = w1;
    }
    if w2.len() == EMB_DIM * HIDDEN {
        p.w2 = w2;
    }
    if b1.len() == HIDDEN {
        p.b1 = b1;
    }
    if b2.len() == EMB_DIM {
        p.b2 = b2;
    }
    p.trained_steps = v.get("trained_steps").and_then(|x| x.as_u64()).unwrap_or(0);
    p.n_pos = v.get("n_pos").and_then(|x| x.as_u64()).unwrap_or(0);
    p.n_neg = v.get("n_neg").and_then(|x| x.as_u64()).unwrap_or(0);
    if let Some(d) = v.get("deep").and_then(|x| x.as_bool()) {
        p.deep = d && deep_enabled();
    }
}

/// Force flush weights.
pub fn contrastive_flush() {
    with_proj(|p| persist_proj(p));
}

/// Cosine between two fields after supervised projection (eval helper).
pub fn pair_cosine_supervised(fields_a: &Value, fields_b: &Value) -> f32 {
    let a = embed_supervised_from_fields(fields_a);
    let b = embed_supervised_from_fields(fields_b);
    let mut c = 0.0f32;
    for i in 0..EMB_DIM.min(a.len()).min(b.len()) {
        c += a[i] * b[i];
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared_governance::set_shared_governance_dir_for_tests;
    #[test]
    fn synthetic_fit_separates_hard_negs() {
        let _g = crate::shared_governance::ISS58_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set_shared_governance_dir_for_tests(None);
        reset_contrastive_for_tests();
        let r = fit_contrastive_synthetic_fleet(12);
        assert_eq!(r["ok"], true, "{r}");
        assert!(r["pairs_steps"].as_u64().unwrap_or(0) > 10);

        // After fit: same-device reobs cosine > cross-device cosine (on average)
        let pairs = synthetic_fleet_pairs(8);
        let mut same_c = 0.0f32;
        let mut diff_c = 0.0f32;
        let mut ns = 0usize;
        let mut nd = 0usize;
        for p in &pairs {
            let same = p["same_host"].as_bool().unwrap_or(false);
            let c = pair_cosine_supervised(&p["fields_a"], &p["fields_b"]);
            if same {
                same_c += c;
                ns += 1;
            } else {
                diff_c += c;
                nd += 1;
            }
        }
        let same_m = same_c / ns.max(1) as f32;
        let diff_m = diff_c / nd.max(1) as f32;
        assert!(
            same_m > diff_m + 0.05,
            "expected same_host cos {same_m} > diff {diff_m} by ≥0.05"
        );
    }

    #[test]
    fn online_reobs_trains() {
        let _g = crate::shared_governance::ISS58_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set_shared_governance_dir_for_tests(None);
        reset_contrastive_for_tests();
        // Force empty proj (reset already did; re-assert steps=0)
        {
            let mut g = PROJ.lock().unwrap_or_else(|e| e.into_inner());
            *g = Some(Proj::identity());
            if let Some(ref mut p) = *g {
                p.loaded = true;
            }
        }
        let id = "dv0-aaaaaaaaaa-bbbbbbbbbb-cccccccccc-dddddddddd-eeeeeeeeee-ffffffffff-gggggggggg-hhhhhhhhhh-iiiiiiiiii-jjjjjjjjjj";
        let f1 = json!({"hw_curve_webgl": (0..32).map(|i| 0.2 + i as f64 * 0.01).collect::<Vec<_>>()});
        let mut f2 = f1.clone();
        if let Some(a) = f2["hw_curve_webgl"].as_array_mut() {
            a[0] = json!(0.201);
        }
        observe_contrastive_online(id, &f1);
        observe_contrastive_online(id, &f2);
        let st = contrastive_stats();
        assert!(
            st["trained_steps"].as_u64().unwrap_or(0) >= 1,
            "st={st}"
        );
        assert!(st["n_pos"].as_u64().unwrap_or(0) >= 1, "st={st}");
    }
}
