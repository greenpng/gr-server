//! iss/58 B3: **Multi-layer HNSW** over (optionally supervised) contrastive embeddings.
//!
//! Implements hierarchical NSW (Malkov & Yashunin style):
//! - random level with `P(level ≥ l) ≈ M^{−l}`
//! - search top-down, expand with `ef` on the target layer
//! - bidirectional neighbor links, layer-0 denser (`M_MAX0`)
//!
//! Embeddings: unsupervised base (mean-center + fixed hyperplanes) then optional
//! learned linear projection from `contrastive_sup` (weak labels / synthetic fleet).
//!
//! Process-local graph + optional file share via `shared_governance`.

use crate::shared_governance::{
    as_object_mut, clear_shared_governance_files, shared_governance_dir, with_shared_json,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::cmp::Ordering as CmpOrdering;
use std::sync::Mutex;

pub const HNSW_LITE_ALGO: &str = "hnsw_multilayer_v2";
pub const EMB_DIM: usize = 32;
/// Max neighbors per layer (layers > 0).
const M: usize = 16;
/// Layer-0 max neighbors (denser bottom layer).
const M_MAX0: usize = 32;
const EF_CONSTRUCTION: usize = 64;
const EF_SEARCH: usize = 32;
const MAX_LEVEL: usize = 12;
const MAX_NODES: usize = 8192;
/// level multiplier: level = floor(-ln(U) * ml), ml = 1/ln(M)
fn level_ml() -> f64 {
    1.0 / (M as f64).ln()
}

#[derive(Clone)]
struct Node {
    id: String,
    emb: Vec<f32>,
    lsh: String,
    /// Highest layer this node participates in.
    level: usize,
    /// neighbors[layer] → node indices
    neighbors: Vec<Vec<usize>>,
}

struct Graph {
    nodes: Vec<Node>,
    by_id: HashMap<String, usize>,
    /// Entry point (highest-level node).
    entry: Option<usize>,
    dirty: bool,
}

impl Graph {
    fn new() -> Self {
        Self {
            nodes: Vec::new(),
            by_id: HashMap::new(),
            entry: None,
            dirty: false,
        }
    }
}

static GRAPH: Mutex<Option<Graph>> = Mutex::new(None);
/// iss/74 §3.1.2: Atlas alt-key seeding uses an isolated graph so warm-cell
/// inserts cannot pollute identity/device HNSW (or `hnsw_lite` unit tests).
static ATLAS_GRAPH: Mutex<Option<Graph>> = Mutex::new(None);
/// Deterministic level counter for tests (None = hash-based).
static LEVEL_SEED: Mutex<u64> = Mutex::new(1);

fn with_graph<R>(f: impl FnOnce(&mut Graph) -> R) -> R {
    let mut g = GRAPH.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(Graph::new());
        if let Some(ref mut gr) = *g {
            load_graph_from_shared(gr);
        }
    }
    f(g.as_mut().unwrap())
}

fn with_atlas_graph<R>(f: impl FnOnce(&mut Graph) -> R) -> R {
    let mut g = ATLAS_GRAPH.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(Graph::new());
    }
    f(g.as_mut().unwrap())
}

pub fn reset_hnsw_for_tests() {
    let mut g = GRAPH.lock().unwrap_or_else(|e| e.into_inner());
    *g = Some(Graph::new());
    let mut ag = ATLAS_GRAPH.lock().unwrap_or_else(|e| e.into_inner());
    *ag = Some(Graph::new());
    let mut s = LEVEL_SEED.lock().unwrap_or_else(|e| e.into_inner());
    *s = 1;
}

// ─── Embeddings ────────────────────────────────────────────────────────────

/// Unsupervised base embed (no learned W). Public for contrastive_sup training.
pub fn contrastive_embed_base(curve: &[f64]) -> Vec<f32> {
    if curve.is_empty() {
        return vec![0.0; EMB_DIM];
    }
    let n = curve.len() as f64;
    let mean = curve.iter().sum::<f64>() / n;
    let mut centered: Vec<f64> = curve.iter().map(|x| x - mean).collect();
    let var = centered.iter().map(|x| x * x).sum::<f64>() / n;
    let std = var.sqrt().max(1e-9);
    for x in &mut centered {
        *x /= std;
    }
    let mut out = vec![0.0f32; EMB_DIM];
    for d in 0..EMB_DIM {
        let mut acc = 0.0f64;
        for (i, &v) in centered.iter().enumerate() {
            let mut h = Sha256::new();
            h.update(b"contrast_emb_v1");
            h.update((d as u32).to_le_bytes());
            h.update((i as u32).to_le_bytes());
            let dig = h.finalize();
            let sign = if dig[0] & 1 == 0 { 1.0 } else { -1.0 };
            let w = 1.0 + (dig[1] as f64) / 255.0;
            acc += v * sign * w;
        }
        out[d] = acc as f32;
    }
    l2_normalize_slice(&mut out);
    out
}

/// Alias kept for callers / tests.
pub fn contrastive_embed(curve: &[f64]) -> Vec<f32> {
    let base = contrastive_embed_base(curve);
    crate::contrastive_sup::project_embed(&base)
}

fn l2_normalize_slice(v: &mut [f32]) {
    let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n < 1e-9 {
        return;
    }
    for x in v.iter_mut() {
        *x /= n;
    }
}

/// Base (unsupervised) embed from fields — for training labels.
pub fn embed_base_from_fields(fields: &Value) -> Vec<f32> {
    for keys in [
        &["hw_curve_webgl", "webgl_residual_multipath"][..],
        &["audio_seed_delta_curve", "audio_deep_curve", "hw_curve_audio"][..],
        &["cpu_timing_curve", "hw_curve_cpu"][..],
    ] {
        for k in keys {
            if let Some(a) = fields.get(*k).and_then(|v| v.as_array()) {
                let xs: Vec<f64> = a.iter().filter_map(|x| x.as_f64()).collect();
                if xs.len() >= 8 {
                    return contrastive_embed_base(&xs);
                }
            }
        }
    }
    let lsh = fields
        .get("wg_whiten_lsh")
        .or_else(|| fields.get("curve_lsh"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !lsh.is_empty() {
        let mut h = Sha256::new();
        h.update(lsh.as_bytes());
        let dig = h.finalize();
        let mut emb = vec![0.0f32; EMB_DIM];
        for i in 0..EMB_DIM {
            emb[i] = (dig[i % dig.len()] as f32 / 127.5) - 1.0;
        }
        l2_normalize_slice(&mut emb);
        return emb;
    }
    vec![0.0; EMB_DIM]
}

/// Runtime embed: base → supervised projection.
pub fn embed_from_fields(fields: &Value) -> Vec<f32> {
    crate::contrastive_sup::embed_supervised_from_fields(fields)
}

fn cosine_dist(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 2.0;
    }
    let mut dot = 0.0f32;
    for i in 0..n {
        dot += a[i] * b[i];
    }
    (1.0 - dot).max(0.0)
}

fn lsh_of(fields: &Value) -> String {
    fields
        .get("wg_whiten_lsh")
        .or_else(|| fields.get("au_whiten_lsh"))
        .or_else(|| fields.get("curve_lsh"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .chars()
        .take(16)
        .collect()
}

// ─── HNSW primitives ───────────────────────────────────────────────────────

#[derive(Clone)]
struct Cand {
    dist: f32,
    idx: usize,
}
impl PartialEq for Cand {
    fn eq(&self, o: &Self) -> bool {
        self.idx == o.idx
    }
}
impl Eq for Cand {}
impl PartialOrd for Cand {
    fn partial_cmp(&self, o: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(o))
    }
}
impl Ord for Cand {
    fn cmp(&self, o: &Self) -> CmpOrdering {
        self.dist.partial_cmp(&o.dist).unwrap_or(CmpOrdering::Equal)
    }
}

#[derive(Copy, Clone)]
struct ordered_f32(f32);
impl PartialEq for ordered_f32 {
    fn eq(&self, o: &Self) -> bool {
        self.0 == o.0
    }
}
impl Eq for ordered_f32 {}
impl PartialOrd for ordered_f32 {
    fn partial_cmp(&self, o: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(o))
    }
}
impl Ord for ordered_f32 {
    fn cmp(&self, o: &Self) -> CmpOrdering {
        self.0.partial_cmp(&o.0).unwrap_or(CmpOrdering::Equal)
    }
}

/// Hash-based level for stable multi-process (no global RNG drift).
/// Geometric: while hash_byte % M == 0, climb one layer (classic HNSW).
fn assign_level(device_id: &str) -> usize {
    let mut h = Sha256::new();
    h.update(b"hnsw_level_v2|");
    h.update(device_id.as_bytes());
    let dig = h.finalize();
    let m = (M as u8).max(2);
    let mut level = 0usize;
    for b in dig.iter() {
        if level >= MAX_LEVEL {
            break;
        }
        if b % m != 0 {
            break;
        }
        level += 1;
    }
    // Also accept continuous formula as lower bound so very high layers still possible
    let mut u_bytes = [0u8; 8];
    u_bytes.copy_from_slice(&dig[0..8]);
    let u_int = u64::from_le_bytes(u_bytes);
    let u = ((u_int as f64) / (u64::MAX as f64)).clamp(1e-12, 1.0 - 1e-12);
    let cont = (-u.ln() * level_ml()).floor() as usize;
    level.max(cont).min(MAX_LEVEL)
}

fn m_for_layer(layer: usize) -> usize {
    if layer == 0 {
        M_MAX0
    } else {
        M
    }
}

/// Search one layer: ef closest among graph neighborhood of `ep`.
fn search_layer(
    g: &Graph,
    query: &[f32],
    ep: &[usize],
    ef: usize,
    layer: usize,
) -> Vec<(usize, f32)> {
    if g.nodes.is_empty() || ep.is_empty() {
        return Vec::new();
    }
    let ef = ef.max(1);
    let mut visited: HashSet<usize> = HashSet::new();
    // min-heap candidates (best first)
    let mut candidates: BinaryHeap<std::cmp::Reverse<(ordered_f32, usize)>> = BinaryHeap::new();
    // max-heap result (worst on top)
    let mut w: BinaryHeap<Cand> = BinaryHeap::new();

    for &e in ep {
        if e >= g.nodes.len() || visited.contains(&e) {
            continue;
        }
        // only nodes that exist at this layer
        if g.nodes[e].level < layer {
            continue;
        }
        let d = cosine_dist(query, &g.nodes[e].emb);
        visited.insert(e);
        candidates.push(std::cmp::Reverse((ordered_f32(d), e)));
        w.push(Cand { dist: d, idx: e });
    }
    if w.is_empty() {
        // fallback: any node at layer
        for (i, n) in g.nodes.iter().enumerate() {
            if n.level >= layer {
                let d = cosine_dist(query, &n.emb);
                visited.insert(i);
                candidates.push(std::cmp::Reverse((ordered_f32(d), i)));
                w.push(Cand { dist: d, idx: i });
                break;
            }
        }
    }
    if w.is_empty() {
        return Vec::new();
    }

    while let Some(std::cmp::Reverse((ordered_f32(d_c), c))) = candidates.pop() {
        let f_worst = w.peek().map(|x| x.dist).unwrap_or(f32::MAX);
        if d_c > f_worst && w.len() >= ef {
            break;
        }
        let nbs = g.nodes[c]
            .neighbors
            .get(layer)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        for &nb in nbs {
            if nb >= g.nodes.len() || !visited.insert(nb) {
                continue;
            }
            if g.nodes[nb].level < layer {
                continue;
            }
            let d = cosine_dist(query, &g.nodes[nb].emb);
            let f_w = w.peek().map(|x| x.dist).unwrap_or(f32::MAX);
            if d < f_w || w.len() < ef {
                candidates.push(std::cmp::Reverse((ordered_f32(d), nb)));
                w.push(Cand { dist: d, idx: nb });
                if w.len() > ef {
                    w.pop();
                }
            }
        }
    }

    // Small graph: brute force for correctness
    if g.nodes.len() <= 48 {
        let mut all: Vec<(usize, f32)> = g
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.level >= layer)
            .map(|(i, n)| (i, cosine_dist(query, &n.emb)))
            .collect();
        all.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(CmpOrdering::Equal));
        return all.into_iter().take(ef).collect();
    }

    let mut out: Vec<(usize, f32)> = w.into_iter().map(|c| (c.idx, c.dist)).collect();
    out.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(CmpOrdering::Equal));
    out
}

fn select_neighbors(candidates: &[(usize, f32)], m: usize) -> Vec<usize> {
    candidates.iter().take(m).map(|(i, _)| *i).collect()
}

fn connect(g: &mut Graph, a: usize, b: usize, layer: usize) {
    if a == b || a >= g.nodes.len() || b >= g.nodes.len() {
        return;
    }
    let m = m_for_layer(layer);
    // ensure neighbor vecs long enough
    while g.nodes[a].neighbors.len() <= layer {
        g.nodes[a].neighbors.push(Vec::new());
    }
    while g.nodes[b].neighbors.len() <= layer {
        g.nodes[b].neighbors.push(Vec::new());
    }
    // Snapshot embeddings to avoid borrow conflicts while trimming.
    let emb_a = g.nodes[a].emb.clone();
    let emb_b = g.nodes[b].emb.clone();
    let emb_all: Vec<Vec<f32>> = g.nodes.iter().map(|n| n.emb.clone()).collect();

    for (src, dst, emb_src) in [(a, b, &emb_a), (b, a, &emb_b)] {
        let nbs = &mut g.nodes[src].neighbors[layer];
        if !nbs.contains(&dst) {
            nbs.push(dst);
        }
        if nbs.len() > m {
            let mut scored: Vec<(usize, f32)> = nbs
                .iter()
                .map(|&j| {
                    let d = emb_all
                        .get(j)
                        .map(|e| cosine_dist(emb_src, e))
                        .unwrap_or(2.0);
                    (j, d)
                })
                .collect();
            scored.sort_by(|x, y| x.1.partial_cmp(&y.1).unwrap_or(CmpOrdering::Equal));
            *nbs = scored.into_iter().take(m).map(|(j, _)| j).collect();
        }
    }
}

fn hnsw_insert_graph(
    g: &mut Graph,
    device_id: &str,
    emb: Vec<f32>,
    lsh: String,
    persist: bool,
) {
    if let Some(&idx) = g.by_id.get(device_id) {
        g.nodes[idx].emb = emb;
        g.nodes[idx].lsh = lsh;
        g.dirty = true;
        if persist {
            maybe_persist(g);
        }
        return;
    }
    if g.nodes.len() >= MAX_NODES {
        // drop lowest index (oldest-ish)
        remove_idx(g, 0);
    }
    let level = assign_level(device_id);
    let idx = g.nodes.len();

    // Empty graph
    if g.nodes.is_empty() {
        let mut neighbors = Vec::new();
        for _ in 0..=level {
            neighbors.push(Vec::new());
        }
        g.nodes.push(Node {
            id: device_id.to_string(),
            emb,
            lsh,
            level,
            neighbors,
        });
        g.by_id.insert(device_id.to_string(), idx);
        g.entry = Some(idx);
        g.dirty = true;
        if persist {
            maybe_persist(g);
        }
        return;
    }

    let entry = g.entry.unwrap_or(0);
    let entry_level = g.nodes.get(entry).map(|n| n.level).unwrap_or(0);

    // Top-down greedy to target level+1
    let mut curr = vec![entry];
    let top = entry_level.max(level);
    for lc in (level + 1..=top).rev() {
        let nearest = search_layer(g, &emb, &curr, 1, lc);
        if let Some(&(i, _)) = nearest.first() {
            curr = vec![i];
        }
    }

    // For each layer from min(level, entry_level) down to 0
    let mut neighbors_per_layer: Vec<Vec<usize>> = Vec::new();
    for _ in 0..=level {
        neighbors_per_layer.push(Vec::new());
    }

    for lc in (0..=level.min(entry_level)).rev() {
        let candidates = search_layer(g, &emb, &curr, EF_CONSTRUCTION, lc);
        let selected = select_neighbors(&candidates, m_for_layer(lc));
        neighbors_per_layer[lc] = selected.clone();
        if let Some(&(best, _)) = candidates.first() {
            curr = vec![best];
        }
    }

    g.nodes.push(Node {
        id: device_id.to_string(),
        emb,
        lsh,
        level,
        neighbors: neighbors_per_layer.clone(),
    });
    g.by_id.insert(device_id.to_string(), idx);

    // Bidirectional connect
    for lc in 0..=level {
        for &nb in &neighbors_per_layer[lc] {
            connect(g, idx, nb, lc);
        }
    }

    // Update entry if new node is higher
    if level > entry_level {
        g.entry = Some(idx);
    }
    g.dirty = true;
    if persist {
        maybe_persist(g);
    }
}

/// Insert or update a device in the multi-layer HNSW graph.
pub fn hnsw_insert(device_id: &str, fields: &Value) {
    if device_id.is_empty() {
        return;
    }
    let emb = embed_from_fields(fields);
    if emb.iter().all(|x| *x == 0.0) {
        return;
    }
    let lsh = lsh_of(fields);
    with_graph(|g| hnsw_insert_graph(g, device_id, emb, lsh, true));
}

/// Atlas alt-key seeding (iss/74): isolated graph, no shared-file persist.
pub fn atlas_hnsw_insert(cohort_key: &str, fields: &Value) {
    if cohort_key.is_empty() {
        return;
    }
    let emb = embed_from_fields(fields);
    if emb.iter().all(|x| *x == 0.0) {
        return;
    }
    let lsh = lsh_of(fields);
    with_atlas_graph(|g| hnsw_insert_graph(g, cohort_key, emb, lsh, false));
}

fn remove_idx(g: &mut Graph, idx: usize) {
    if idx >= g.nodes.len() {
        return;
    }
    let last = g.nodes.len() - 1;
    g.nodes.swap_remove(idx);
    // Remap neighbor indices: removed idx gone; old `last` now lives at idx.
    let nlen = g.nodes.len();
    for n in &mut g.nodes {
        for layer in &mut n.neighbors {
            let mut next = Vec::with_capacity(layer.len());
            for &j in layer.iter() {
                if j == idx {
                    continue; // dropped node
                }
                if j == last {
                    next.push(idx); // swapped into place
                } else if j < nlen {
                    next.push(j);
                }
            }
            *layer = next;
        }
    }
    g.by_id.clear();
    for (i, n) in g.nodes.iter().enumerate() {
        g.by_id.insert(n.id.clone(), i);
    }
    g.entry = g
        .nodes
        .iter()
        .enumerate()
        .max_by_key(|(_, n)| n.level)
        .map(|(i, _)| i);
}

fn hnsw_search_graph(g: &Graph, emb: &[f32], lsh: &str, k: usize) -> Vec<(String, f32)> {
    if g.nodes.is_empty() {
        return Vec::new();
    }
    let entry = g.entry.unwrap_or(0);
    let entry_level = g.nodes.get(entry).map(|n| n.level).unwrap_or(0);
    let mut curr = vec![entry];
    // Top-down ef=1
    for lc in (1..=entry_level).rev() {
        let nearest = search_layer(g, emb, &curr, 1, lc);
        if let Some(&(i, _)) = nearest.first() {
            curr = vec![i];
        }
    }
    // Bottom layer with ef_search
    let mut hits = search_layer(
        g,
        emb,
        &curr,
        EF_SEARCH.max(k).min(g.nodes.len().max(1)),
        0,
    );
    // LSH hybrid
    if lsh.len() >= 4 {
        let pref: String = lsh.chars().take(8).collect();
        for (i, n) in g.nodes.iter().enumerate() {
            if n.lsh.starts_with(&pref) {
                let d = cosine_dist(emb, &n.emb) * 0.85;
                if !hits.iter().any(|(j, _)| *j == i) {
                    hits.push((i, d));
                }
            }
        }
    }
    hits.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(CmpOrdering::Equal));
    hits.dedup_by(|a, b| a.0 == b.0);
    hits.into_iter()
        .take(k)
        .filter_map(|(i, d)| g.nodes.get(i).map(|n| (n.id.clone(), d)))
        .collect()
}

/// Hierarchical search + LSH hybrid boost.
pub fn hnsw_search(fields: &Value, k: usize) -> Vec<(String, f32)> {
    let emb = embed_from_fields(fields);
    let lsh = lsh_of(fields);
    with_graph(|g| hnsw_search_graph(g, &emb, &lsh, k))
}

/// Atlas alt-key search over the isolated warm-cell graph (iss/74).
pub fn atlas_hnsw_search(fields: &Value, k: usize) -> Vec<(String, f32)> {
    let emb = embed_from_fields(fields);
    let lsh = lsh_of(fields);
    with_atlas_graph(|g| hnsw_search_graph(g, &emb, &lsh, k))
}

pub fn hnsw_stats() -> Value {
    with_graph(|g| {
        let mut max_level = 0usize;
        let mut layer_hist = vec![0u64; MAX_LEVEL + 1];
        for n in &g.nodes {
            max_level = max_level.max(n.level);
            if n.level <= MAX_LEVEL {
                layer_hist[n.level] += 1;
            }
        }
        json!({
            "algo": HNSW_LITE_ALGO,
            "nodes": g.nodes.len(),
            "entry": g.entry,
            "max_level": max_level,
            "layer_hist": layer_hist,
            "emb_dim": EMB_DIM,
            "m": M,
            "m_max0": M_MAX0,
            "ef_construction": EF_CONSTRUCTION,
            "ef_search": EF_SEARCH,
            "multilayer": true,
            "shared": shared_governance_dir().is_some(),
            "contrastive": crate::contrastive_sup::contrastive_stats(),
        })
    })
}

fn maybe_persist(g: &mut Graph) {
    if !g.dirty {
        return;
    }
    if g.nodes.len() % 8 != 0 && g.nodes.len() > 1 {
        return;
    }
    persist_graph(g);
    g.dirty = false;
}

fn persist_graph(g: &Graph) {
    if shared_governance_dir().is_none() {
        return;
    }
    let _ = with_shared_json("hnsw_ann", json!({"nodes":[]}), |v| {
        let o = as_object_mut(v);
        o.insert("algo".into(), json!(HNSW_LITE_ALGO));
        o.insert("emb_dim".into(), json!(EMB_DIM));
        o.insert("multilayer".into(), json!(true));
        let mut arr = Vec::new();
        for n in &g.nodes {
            arr.push(json!({
                "id": n.id,
                "emb": n.emb,
                "lsh": n.lsh,
                "level": n.level,
                "neighbors": n.neighbors,
            }));
        }
        o.insert("nodes".into(), json!(arr));
        o.insert("entry".into(), json!(g.entry));
    });
}

fn load_graph_from_shared(g: &mut Graph) {
    if shared_governance_dir().is_none() {
        return;
    }
    let loaded = with_shared_json("hnsw_ann", json!({"nodes":[]}), |v| {
        let nodes = v
            .get("nodes")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default();
        let entry = v.get("entry").and_then(|x| x.as_u64()).map(|x| x as usize);
        (nodes, entry)
    });
    let Some((nodes, entry)) = loaded else {
        return;
    };
    g.nodes.clear();
    g.by_id.clear();
    for n in nodes {
        let id = n.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
        if id.is_empty() {
            continue;
        }
        let emb: Vec<f32> = n
            .get("emb")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_f64().map(|f| f as f32))
                    .collect()
            })
            .unwrap_or_default();
        if emb.len() != EMB_DIM {
            continue;
        }
        let lsh = n
            .get("lsh")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let level = n.get("level").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
        // Support both multilayer neighbors[][] and legacy flat neighbors[]
        let neighbors: Vec<Vec<usize>> = if let Some(arr) = n.get("neighbors").and_then(|x| x.as_array())
        {
            if arr.first().map(|x| x.is_array()).unwrap_or(false) {
                arr.iter()
                    .map(|layer| {
                        layer
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(|x| x.as_u64().map(|u| u as usize))
                                    .collect()
                            })
                            .unwrap_or_default()
                    })
                    .collect()
            } else {
                // legacy single-layer
                let flat: Vec<usize> = arr
                    .iter()
                    .filter_map(|x| x.as_u64().map(|u| u as usize))
                    .collect();
                let mut layers = vec![Vec::new(); level + 1];
                layers[0] = flat;
                layers
            }
        } else {
            vec![Vec::new(); level + 1]
        };
        let idx = g.nodes.len();
        g.by_id.insert(id.clone(), idx);
        g.nodes.push(Node {
            id,
            emb,
            lsh,
            level,
            neighbors,
        });
    }
    g.entry = entry
        .filter(|&e| e < g.nodes.len())
        .or_else(|| {
            g.nodes
                .iter()
                .enumerate()
                .max_by_key(|(_, n)| n.level)
                .map(|(i, _)| i)
        });
}

pub fn hnsw_flush() {
    with_graph(|g| {
        persist_graph(g);
        g.dirty = false;
    });
}

/// Max level among nodes (test/ops).
pub fn hnsw_max_level() -> usize {
    with_graph(|g| g.nodes.iter().map(|n| n.level).max().unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared_governance::set_shared_governance_dir_for_tests;
    fn iso_hnsw(label: &str) -> (std::sync::MutexGuard<'static, ()>, std::path::PathBuf) {
        let guard = crate::shared_governance::ISS58_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::set_var("GR_CONTRASTIVE_WARM_FLEET", "0");
        let dir = crate::shared_governance::test_isolation_dir(label);
        crate::contrastive_sup::reset_contrastive_for_tests();
        reset_hnsw_for_tests();
        {
            let mut g = GRAPH.lock().unwrap();
            *g = Some(Graph::new());
        }
        (guard, dir)
    }

    #[test]
    fn contrastive_embed_unit_norm() {
        let c: Vec<f64> = (0..32).map(|i| i as f64 * 0.01).collect();
        let e = contrastive_embed_base(&c);
        assert_eq!(e.len(), EMB_DIM);
        let n: f32 = e.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((n - 1.0).abs() < 1e-3, "norm={n}");
    }

    #[test]
    fn hnsw_recalls_near_neighbor() {
        let (_g, dir) = iso_hnsw("recall");
        let base: Vec<f64> = (0..32).map(|i| 0.2 + i as f64 * 0.01).collect();
        for i in 0..40 {
            let mut c = base.clone();
            c[0] += i as f64 * 0.001;
            let f = json!({
                "hw_curve_webgl": c,
                "wg_whiten_lsh": format!("{:016x}", i as u64 * 0x1111),
            });
            hnsw_insert(&format!("dev{i}"), &f);
        }
        let mut q = base.clone();
        q[0] += 0.001;
        let qf = json!({"hw_curve_webgl": q, "wg_whiten_lsh": format!("{:016x}", 1u64 * 0x1111)});
        let hits = hnsw_search(&qf, 5);
        assert!(!hits.is_empty());
        let ids: Vec<_> = hits.iter().map(|(id, _)| id.as_str()).collect();
        assert!(ids.iter().any(|id| id.starts_with("dev")), "hits={ids:?}");
        let st = hnsw_stats();
        assert_eq!(st["nodes"].as_u64().unwrap_or(0), 40);
        assert_eq!(st["multilayer"], true);
        clear_shared_governance_files();
        set_shared_governance_dir_for_tests(None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn multilayer_has_upper_layers_with_enough_nodes() {
        let (_g, dir) = iso_hnsw("mlayers");
        let base: Vec<f64> = (0..32).map(|i| i as f64 * 0.02).collect();
        for i in 0..200 {
            let mut c = base.clone();
            c[i % 32] += 0.01 * (i as f64);
            let f = json!({
                "hw_curve_webgl": c,
                "wg_whiten_lsh": format!("{:016x}", i as u64 * 0x9e37),
            });
            hnsw_insert(&format!("n{i}"), &f);
        }
        let max_l = hnsw_max_level();
        assert!(
            max_l >= 1,
            "expected multilayer levels with 200 nodes, max_level={max_l}"
        );
        let st = hnsw_stats();
        assert_eq!(st["algo"], HNSW_LITE_ALGO);
        clear_shared_governance_files();
        set_shared_governance_dir_for_tests(None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn shared_hnsw_roundtrip() {
        let (_g, dir) = iso_hnsw("hnsw_rt");
        let f = json!({"hw_curve_webgl": (0..32).map(|i| i as f64 * 0.02).collect::<Vec<_>>()});
        hnsw_insert("dA_unique_rt", &f);
        hnsw_flush();
        {
            let mut g = GRAPH.lock().unwrap();
            *g = Some(Graph::new());
            load_graph_from_shared(g.as_mut().unwrap());
        }
        let hits = hnsw_search(&f, 5);
        assert!(
            hits.iter().any(|(id, _)| id == "dA_unique_rt"),
            "hits={hits:?}"
        );
        clear_shared_governance_files();
        set_shared_governance_dir_for_tests(None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
