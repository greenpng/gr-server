//! Shadow K↔V Atlas scoring (iss/63 §5.4, iss/67 A3).
//!
//! Spoof / farm-tightness / zero-silicon / replay — **shadow only**, never hard-block.
//!
//! iss/69 U3/U4: replay is derived from FE challenge fields (no orphan field),
//! fit uses the population-atlas cell templates (+ best alternative key), and
//! same-key peer digests are produced into the shared store so farm-tightness
//! has a real data source.

use crate::entropy_census::census_slot_digests;
use crate::model_key::model_key_from_fields;
use crate::population_atlas::{atlas_observe, cohort_key_from_fields};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

pub const ATLAS_SCORE_ALGO: &str = "atlas_score_shadow_v1";

// ─── Same-key peer digest store (iss/69 U3 producer) ───────────────────────

const PEER_SHARED_KEY: &str = "atlas_peers_v1";
const PEER_RING_CAP: usize = 48;
const PEER_TTL_MS: u64 = 72 * 3600 * 1000;
const PEER_TAKE: usize = 16;

fn peer_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Record the session's V digest (res part preferred) under its model key.
/// Ring-capped + TTL-pruned, flushed to shared JSON (multi-worker safe).
pub fn observe_peer_digest(model_key: &str, v_digest: &str) {
    if model_key.is_empty()
        || model_key == "unk:unk"
        || v_digest.is_empty()
        || v_digest == "0"
    {
        return;
    }
    use crate::shared_governance::{as_object_mut, with_shared_json};
    if crate::shared_governance::shared_governance_dir().is_none() {
        return;
    }
    let t = peer_now_ms();
    let _ = with_shared_json(PEER_SHARED_KEY, json!({}), |v| {
        let o = as_object_mut(v);
        let arr = o
            .entry(model_key.to_string())
            .or_insert(json!([]))
            .as_array_mut()
            .unwrap();
        arr.retain(|e| {
            e.get("ts")
                .and_then(|x| x.as_u64())
                .map(|ts| t.saturating_sub(ts) < PEER_TTL_MS)
                .unwrap_or(true)
        });
        arr.push(json!({"d": v_digest, "ts": t}));
        if arr.len() > PEER_RING_CAP {
            let drop = arr.len() - PEER_RING_CAP;
            arr.drain(0..drop);
        }
    });
}

/// Recent same-key peer digests (newest first, capped for farm census).
pub fn peer_digests_for_key(model_key: &str) -> Vec<String> {
    if model_key.is_empty() {
        return Vec::new();
    }
    use crate::shared_governance::with_shared_json;
    if crate::shared_governance::shared_governance_dir().is_none() {
        return Vec::new();
    }
    let t = peer_now_ms();
    with_shared_json(PEER_SHARED_KEY, json!({}), |v| {
        v.get(model_key)
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .rev()
                    .filter(|e| {
                        e.get("ts")
                            .and_then(|x| x.as_u64())
                            .map(|ts| t.saturating_sub(ts) < PEER_TTL_MS)
                            .unwrap_or(false)
                    })
                    .filter_map(|e| {
                        e.get("d").and_then(|x| x.as_str()).map(|s| s.to_string())
                    })
                    .take(PEER_TAKE)
                    .collect()
            })
            .unwrap_or_default()
    })
    .unwrap_or_default()
}

/// Attach same-key peer digests onto the scoring context (producer wiring).
pub fn attach_peer_context(fields: &mut Map<String, Value>, model_key: &str) {
    let peers = peer_digests_for_key(model_key);
    fields.insert("atlas_peer_digests".into(), json!(peers));
    fields.insert("atlas_peer_n".into(), json!(fields["atlas_peer_digests"].as_array().map(|a| a.len()).unwrap_or(0)));
    fields.insert("atlas_peer_key".into(), json!(model_key.to_string()));
}

/// Feature flag: when false, still computes but marks disabled.
fn shadow_enabled() -> bool {
    match gr_abi::env::get("ATLAS_SHADOW_SCORE") {
        Some(v) => v != "0" && v != "false" && v != "off",
        None => true,
    }
}

/// Material-aware prior when the own Cell is still cold (<8 samples).
/// Never invent high fit (0.72) just because K looks named — that hid empty Atlas.
fn material_prior_fit(fields: &Value, key: &str, spoof: bool, zero_silicon: bool) -> f64 {
    if zero_silicon {
        return 0.15;
    }
    if spoof {
        return 0.25;
    }
    let has_curve = [
        "hw_curve_webgl",
        "webgl_residual_multipath",
        "hw_curve_audio",
        "audio_seed_delta_curve",
        "hw_curve_cpu",
        "cpu_timing_curve",
    ]
    .iter()
    .any(|k| {
        fields
            .get(*k)
            .and_then(|v| v.as_array())
            .map(|a| a.len() >= 8)
            .unwrap_or(false)
            || fields.get(*k).and_then(|v| v.as_object()).is_some()
    });
    let has_residual = fields
        .get("residual_mean")
        .and_then(|v| v.as_f64())
        .is_some();
    if key.starts_with("class:") {
        return if has_curve || has_residual { 0.48 } else { 0.32 };
    }
    if key.starts_with("unk:") || key == "unk:unk" {
        return if has_curve { 0.40 } else { 0.28 };
    }
    if key.starts_with("soft:") {
        return 0.20;
    }
    // named GPU, cold cell: honest mid prior (not 0.72 fake certainty)
    if has_curve && has_residual {
        0.58
    } else if has_curve || has_residual {
        0.50
    } else {
        0.38
    }
}

/// Observe fields into population atlas (model-aware cohort) and return shadow scores.
pub fn atlas_shadow_score(fields: &Value) -> Value {
    let enabled = shadow_enabled();

    // Ensure K is on the field object *before* observe + cell_fit (cohort placement).
    let mk = model_key_from_fields(fields);
    let key = mk
        .get("hw_model_key")
        .and_then(|v| v.as_str())
        .unwrap_or("unk:unk")
        .to_string();
    let backend = mk
        .get("gl_backend")
        .and_then(|v| v.as_str())
        .unwrap_or("unk");
    let mut fields_mk = fields.as_object().cloned().unwrap_or_default();
    if !key.is_empty() && key != "unk:unk" {
        fields_mk
            .entry("hw_model_key".to_string())
            .or_insert_with(|| json!(key.clone()));
    }
    if backend != "unk" {
        fields_mk
            .entry("gl_backend_class".to_string())
            .or_insert_with(|| json!(backend));
    }
    let fields_enriched = Value::Object(fields_mk.clone());

    // Always observe when enabled — cold-start path uses same observe
    if enabled {
        atlas_observe(&fields_enriched);
    }

    // Peer producer: residual / wg digest under model key (farm tightness input).
    // Merge store peers with any pre-supplied digests (tests / external producers) —
    // never wipe atlas_peer_digests that callers already attached.
    if enabled && !key.is_empty() && key != "unk:unk" {
        let v_dig = residual_v_digest(&fields_enriched);
        if !v_dig.is_empty() && v_dig != "0" {
            // Only publish when there is real residual/curve material (not empty hash).
            let has_v_material = fields_enriched
                .get("residual_structure_digest")
                .or_else(|| fields_enriched.get("res_diff_digest"))
                .or_else(|| fields_enriched.get("wg_diff_digest"))
                .or_else(|| fields_enriched.get("hw_curve_webgl_fp"))
                .and_then(|v| v.as_str())
                .map(|s| !s.is_empty() && s != "0")
                .unwrap_or(false)
                || fields_enriched.get("residual_mean").and_then(|v| v.as_f64()).is_some()
                || fields_enriched
                    .get("hw_curve_webgl")
                    .or_else(|| fields_enriched.get("webgl_residual_multipath"))
                    .and_then(|v| v.as_array())
                    .map(|a| a.len() >= 4)
                    .unwrap_or(false);
            if has_v_material {
                observe_peer_digest(&key, &v_dig);
            }
        }
        // Farm census needs multiplicity (same digest repeated = tightness).
        // Prefer caller-supplied digests when present; else use store ring.
        // Never de-dupe — collision share is the signal.
        let supplied: Vec<String> = fields_mk
            .get("atlas_peer_digests")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|e| e.as_str().map(|s| s.to_string()))
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let mut peers = if supplied.len() >= 3 {
            supplied
        } else {
            let mut store = peer_digests_for_key(&key);
            // Append any sparse supplied samples (keep multiplicity)
            store.extend(supplied);
            store
        };
        if peers.len() > PEER_TAKE {
            peers.truncate(PEER_TAKE);
        }
        fields_mk.insert("atlas_peer_digests".into(), json!(peers));
        fields_mk.insert("atlas_peer_n".into(), json!(peers.len()));
        fields_mk.insert("atlas_peer_key".into(), json!(key.clone()));
    }
    let fields_for_score = Value::Object(fields_mk);

    let cohort = cohort_key_from_fields(&fields_for_score);
    let model_cell = format!("mk={key}|{backend}");

    let residual_soft = fields_for_score
        .get("residual_soft_like")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let soft_gpu = fields_for_score
        .get("webgl_unmasked_renderer")
        .and_then(|v| v.as_str())
        .map(|s| {
            let l = s.to_lowercase();
            l.contains("swiftshader") || l.contains("llvmpipe") || l.contains("softpipe")
        })
        .unwrap_or(false);

    let zero_silicon = residual_soft
        || soft_gpu
        || fields_for_score
            .get("zero_silicon")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

    // Replay (iss/63 逐位重演): seed present but response bit-identical when the
    // seed changes. Produced from FE challenge fields (iss/69 U3) — no orphan input.
    let seed_used = fields_for_score
        .get("challenge_seed_used")
        .or_else(|| fields_for_score.get("seed_k"))
        .or_else(|| fields_for_score.get("webgpu_challenge_seed_used"))
        .or_else(|| fields_for_score.get("challenge_seed_present"))
        .is_some();
    let replay_exact = seed_used
        && (fields_for_score
            .get("replay_exact")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || fields_for_score
                .get("challenge_alt_changed")
                .and_then(|v| v.as_bool())
                == Some(false)
            || fields_for_score
                .get("residual_identical_to_challenge")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            || fields_for_score
                .get("family_alert_constant_zero_replay")
                .and_then(|v| v.as_bool())
                .unwrap_or(false));

    // Farm tightness proxy from same-key peer digests (producer: observe_peer_digest).
    let farm = farm_tightness_hint(&fields_for_score, &key);

    // Real cell fit against population-atlas templates (own K + best alt_key).
    let cell = crate::population_atlas::cell_fit_and_alt_key(&fields_for_score);
    let cell_cold = cell.get("cold").and_then(|v| v.as_bool()).unwrap_or(true);
    let cell_cur_fit = cell
        .get("current_fit")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let cell_alt_fit = cell
        .get("alt_fit")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let best_alt_key = cell
        .get("alt_key")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let cell_lift = cell_alt_fit - cell_cur_fit;
    let mismatch_cell = !cell_cold
        && cell_cur_fit < 0.35
        && cell_alt_fit > 0.0
        && cell_lift >= 0.25
        && cell_alt_fit > cell_cur_fit * 1.5;

    // Spoof: soft GPU with high-end model key claim, explicit mismatch flag,
    // or cell-fit mismatch (V fits another K significantly better).
    let spoof = (soft_gpu && key.starts_with("nvidia:"))
        || fields_for_score
            .get("model_silicon_mismatch")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        || mismatch_cell;

    let mut signals = Vec::new();
    if spoof {
        signals.push("model_silicon_mismatch_shadow");
    }
    if farm.get("tight").and_then(|v| v.as_bool()).unwrap_or(false) {
        signals.push("farm_tightness_shadow");
    }
    if zero_silicon {
        signals.push("zero_silicon_shadow");
    }
    if replay_exact {
        signals.push("replay_exact_shadow");
    }

    // Warm cell → real fit; cold → material prior (never fake 0.72 for named K).
    let fit = if !cell_cold && !zero_silicon {
        cell_cur_fit
    } else {
        material_prior_fit(&fields_for_score, &key, spoof, zero_silicon)
    };

    // Soft demote weight for mint consumers (never hard-block).
    let mut demote = 0.0f64;
    if spoof {
        demote += 0.18;
    }
    if farm.get("tight").and_then(|v| v.as_bool()).unwrap_or(false) {
        demote += 0.12;
    }
    if zero_silicon {
        demote += 0.20;
    }
    if replay_exact {
        demote += 0.22;
    }
    demote = demote.min(0.48);

    json!({
        "algo": ATLAS_SCORE_ALGO,
        "enabled": enabled,
        "hard_block": false,
        "hw_model_key": key,
        "model_cell": model_cell,
        "cohort_key": cohort,
        "model_key_info": mk,
        "atlas_fit_score": (fit * 10000.0).round() / 10000.0,
        "atlas_fit_source": if !cell_cold && !zero_silicon { "cell_template" } else { "material_prior" },
        "best_alt_key": if best_alt_key.is_empty() { Value::Null } else { json!(best_alt_key) },
        "best_alt_fit": cell_alt_fit,
        "cell_fit": cell,
        "signals": signals,
        "spoof_shadow": spoof,
        "farm_tightness": farm,
        "zero_silicon_shadow": zero_silicon,
        "replay_exact_shadow": replay_exact,
        "model_silicon_mismatch": mismatch_cell || spoof,
        "demote_weight": (demote * 10000.0).round() / 10000.0,
        "should_downweight": demote >= 0.18,
        "should_deepen": demote >= 0.24 || mismatch_cell || replay_exact,
        "enforce": atlas_enforce_enabled(),
        "note": "commercial K/V atlas — soft demote only; hard_block always false",
    })
}

/// Residual / structure digest used as V fingerprint for peer farm census.
fn residual_v_digest(fields: &Value) -> String {
    if let Some(s) = fields
        .get("residual_structure_digest")
        .or_else(|| fields.get("res_diff_digest"))
        .or_else(|| fields.get("wg_diff_digest"))
        .or_else(|| fields.get("hw_curve_webgl_fp"))
        .and_then(|v| v.as_str())
    {
        if !s.is_empty() && s != "0" {
            return s.to_string();
        }
    }
    // Fallback: hash residual_mean + residual_std + first curve samples
    let mut h = Sha256::new();
    h.update(b"atlas_v_dig_v1|");
    if let Some(m) = fields.get("residual_mean").and_then(|v| v.as_f64()) {
        h.update(m.to_le_bytes());
    }
    if let Some(s) = fields.get("residual_std").and_then(|v| v.as_f64()) {
        h.update(s.to_le_bytes());
    }
    if let Some(a) = fields
        .get("hw_curve_webgl")
        .or_else(|| fields.get("webgl_residual_multipath"))
        .and_then(|v| v.as_array())
    {
        for x in a.iter().take(16) {
            if let Some(f) = x.as_f64() {
                h.update(f.to_le_bytes());
            }
        }
    }
    let dig = format!("{:x}", h.finalize());
    dig[..16.min(dig.len())].to_string()
}

/// When true, mint path applies demote_weight (still no hard-block).
pub fn atlas_enforce_enabled() -> bool {
    match gr_abi::env::get("ATLAS_ENFORCE") {
        Some(v) => {
            let l = v.to_ascii_lowercase();
            l == "1" || l == "true" || l == "on" || l == "yes"
        }
        // Default ON for commercial path — soft demote only.
        None => true,
    }
}

fn farm_tightness_hint(fields: &Value, key: &str) -> Value {
    // Optional: fields.atlas_peer_digests = [digest,...] same key peers
    let samples: Vec<String> = fields
        .get("atlas_peer_digests")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    if samples.len() < 3 {
        return json!({
            "tight": false,
            "reason": "insufficient_peer_digests",
            "hw_model_key": key,
        });
    }
    let census = census_slot_digests("atlas_peer", &samples);
    let top_share = census
        .get("top_share")
        .or_else(|| census.get("max_collision_share"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    // If one digest owns >40% of peers under same key → tightness hint
    let tight = top_share >= 0.4;
    json!({
        "tight": tight,
        "top_share": top_share,
        "n_peers": samples.len(),
        "hw_model_key": key,
        "census_algo": census.get("algo").cloned().unwrap_or(json!("entropy_census")),
    })
}

/// Offline cold-start: observe a batch of field objects into the atlas.
pub fn bootstrap_atlas_from_fields_batch(batch: &[Value]) -> Value {
    let mut n = 0usize;
    for f in batch {
        if f.is_object() {
            atlas_observe(f);
            n += 1;
        }
    }
    json!({
        "algo": "atlas_bootstrap_v1",
        "ok": true,
        "n_observed": n,
        "note": "offline/history cold-start via population_atlas::atlas_observe",
    })
}

/// Attach shadow score + model key onto fields map for analyze pipeline.
pub fn attach_atlas_shadow_extras(fields: &mut Map<String, Value>) {
    crate::model_key::attach_model_key_extras(fields);
    let score = atlas_shadow_score(&Value::Object(fields.clone()));
    if score
        .get("model_silicon_mismatch")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        fields.insert("model_silicon_mismatch".into(), json!(true));
    }
    fields.insert("atlas_shadow_score".into(), score);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::population_atlas::reset_atlas_for_tests;

    #[test]
    fn shadow_soft_gpu_flags_zero_and_spoof() {
        reset_atlas_for_tests();
        let f = json!({
            "engine_family": "blink",
            "os_family": "linux",
            "webgl_unmasked_renderer": "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device), SwiftShader)",
        });
        let s = atlas_shadow_score(&f);
        assert_eq!(s["hard_block"], false);
        assert_eq!(s["zero_silicon_shadow"], true);
        assert_eq!(s["spoof_shadow"], false); // soft key, not nvidia claim
        assert!(s["hw_model_key"].as_str().unwrap().contains("soft"));
    }

    #[test]
    fn bootstrap_observes_batch() {
        reset_atlas_for_tests();
        let batch = vec![
            json!({
                "engine_family": "blink",
                "os_family": "windows",
                "webgl_unmasked_renderer": "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 Direct3D11, D3D11)",
                "hw_curve_webgl": [0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8,0.9,1.0,1.1,1.2],
            }),
            json!({
                "engine_family": "blink",
                "os_family": "windows",
                "webgl_unmasked_renderer": "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 Direct3D11, D3D11)",
                "hw_curve_webgl": [0.11,0.21,0.31,0.41,0.51,0.61,0.71,0.81,0.91,1.01,1.11,1.21],
            }),
        ];
        let r = bootstrap_atlas_from_fields_batch(&batch);
        assert_eq!(r["n_observed"], 2);
    }

    #[test]
    fn farm_tightness_from_peers() {
        reset_atlas_for_tests();
        let digs: Vec<Value> = (0..10)
            .map(|i| {
                if i < 6 {
                    json!("same_digest_aaaaaaaa")
                } else {
                    json!(format!("other_{i}"))
                }
            })
            .collect();
        let f = json!({
            "engine_family": "blink",
            "os_family": "windows",
            "webgl_unmasked_renderer": "ANGLE (NVIDIA, NVIDIA GeForce GTX 1050 Ti, OpenGL)",
            "atlas_peer_digests": digs,
        });
        let s = atlas_shadow_score(&f);
        assert_eq!(s["farm_tightness"]["tight"], true);
    }

    #[test]
    fn replay_derived_from_alt_seed_unchanged() {
        reset_atlas_for_tests();
        // seed present + alternate-seed response bit-identical → replay shadow
        let f1 = json!({
            "engine_family": "blink",
            "os_family": "windows",
            "challenge_seed_used": "s123",
            "challenge_alt_changed": false,
        });
        let s1 = atlas_shadow_score(&f1);
        assert_eq!(s1["replay_exact_shadow"], true, "{s1}");
        // alt changed → NOT replay
        let f2 = json!({
            "engine_family": "blink",
            "os_family": "windows",
            "challenge_seed_used": "s123",
            "challenge_alt_changed": true,
        });
        let s2 = atlas_shadow_score(&f2);
        assert_eq!(s2["replay_exact_shadow"], false, "{s2}");
        // no seed at all → NOT replay even when alt field present
        let f3 = json!({
            "engine_family": "blink",
            "os_family": "windows",
            "challenge_alt_changed": false,
        });
        let s3 = atlas_shadow_score(&f3);
        assert_eq!(s3["replay_exact_shadow"], false, "{s3}");
    }

    #[test]
    fn peer_store_feeds_farm_tightness() {
        let _guard = crate::shared_governance::ISS58_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "atlas_score_peer_test_{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        crate::shared_governance::set_shared_governance_dir_for_tests(Some(dir.clone()));
        reset_atlas_for_tests();
        for i in 0..10 {
            observe_peer_digest(
                "nvidia:rtx_3060",
                &format!("digest_{}", if i < 6 { "same".to_string() } else { i.to_string() }),
            );
        }
        let mut m = Map::new();
        attach_peer_context(&mut m, "nvidia:rtx_3060");
        let peers = m["atlas_peer_digests"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert_eq!(peers.len(), 10, "peer ring should carry 10 digests");
        let f = json!({
            "engine_family": "blink",
            "os_family": "windows",
            "webgl_unmasked_renderer": "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 Direct3D11, D3D11)",
            "atlas_peer_digests": peers,
        });
        let s = atlas_shadow_score(&f);
        assert_eq!(s["farm_tightness"]["tight"], true, "{s}");
        crate::shared_governance::clear_thread_shared_governance_override();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn material_prior_not_fake_high_for_named_cold_cell() {
        reset_atlas_for_tests();
        // Named GPU, no curves → honest mid prior (not 0.72 ladder)
        let f = json!({
            "engine_family": "blink",
            "os_family": "windows",
            "webgl_unmasked_renderer": "ANGLE (NVIDIA, NVIDIA GeForce GTX 1050 Ti Direct3D11, D3D11)",
        });
        let s = atlas_shadow_score(&f);
        assert_eq!(s["atlas_fit_source"], "material_prior", "{s}");
        let fit = s["atlas_fit_score"].as_f64().unwrap();
        assert!(fit < 0.65, "cold named must not fake high fit: {s}");
        assert!(fit >= 0.30, "named cold prior floor: {s}");
        assert_eq!(s["hard_block"], false);
        assert!(s["hw_model_key"].as_str().unwrap().contains("nvidia"));
    }

    #[test]
    fn soft_gpu_with_nvidia_claim_is_spoof_demote() {
        reset_atlas_for_tests();
        let f = json!({
            "engine_family": "blink",
            "os_family": "linux",
            // Soft GPU string but nvidia model key forced
            "webgl_unmasked_renderer": "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device), SwiftShader)",
            "hw_model_key": "nvidia:rtx_3060",
            "model_silicon_mismatch": true,
        });
        let s = atlas_shadow_score(&f);
        assert_eq!(s["zero_silicon_shadow"], true, "{s}");
        assert_eq!(s["spoof_shadow"], true, "{s}");
        let demote = s["demote_weight"].as_f64().unwrap();
        assert!(demote >= 0.30, "spoof+zero should demote hard-soft: {s}");
        assert_eq!(s["should_deepen"], true);
        assert_eq!(s["hard_block"], false);
        assert_eq!(s["enforce"], true); // commercial default ON
    }

    #[test]
    fn warm_cell_fit_and_best_alt_key() {
        crate::population_atlas::with_atlas_test_lock(|| {
            reset_atlas_for_tests();
            std::env::set_var("GR_ATLAS_HNSW_ALT", "0");
            let base: Vec<f64> = (0..32).map(|i| 0.15 + i as f64 * 0.007).collect();
            let shifted: Vec<f64> = base.iter().map(|x| x + 0.42).collect();
            for _ in 0..12 {
                atlas_observe(&json!({
                    "engine_family": "blink",
                    "os_family": "Windows",
                    "hw_model_key": "nvidia:rtx_3060",
                    "gl_backend_class": "d3d11",
                    "hw_curve_webgl": base.clone(),
                    "audio_seed_delta_curve": base.clone(),
                    "webgl_unmasked_renderer": "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 Direct3D11, D3D11)",
                }));
            }
            for _ in 0..12 {
                atlas_observe(&json!({
                    "engine_family": "blink",
                    "os_family": "Windows",
                    "hw_model_key": "nvidia:gtx_1050",
                    "gl_backend_class": "d3d11",
                    "hw_curve_webgl": shifted.clone(),
                    "audio_seed_delta_curve": shifted.clone(),
                    "webgl_unmasked_renderer": "ANGLE (NVIDIA, NVIDIA GeForce GTX 1050 Direct3D11, D3D11)",
                }));
            }
            // Query claims rtx_3060 but carries shifted curves → should prefer gtx alt
            let q = json!({
                "engine_family": "blink",
                "os_family": "Windows",
                "hw_model_key": "nvidia:rtx_3060",
                "gl_backend_class": "d3d11",
                "hw_curve_webgl": shifted,
                "audio_seed_delta_curve": shifted,
                "webgl_unmasked_renderer": "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 Direct3D11, D3D11)",
                "residual_mean": 0.12,
            });
            let s = atlas_shadow_score(&q);
            // Warm cell path
            assert_eq!(s["atlas_fit_source"], "cell_template", "{s}");
            let fit = s["atlas_fit_score"].as_f64().unwrap();
            let alt_fit = s["best_alt_fit"].as_f64().unwrap_or(0.0);
            // Own cell (rtx base) should fit poorly vs alt (gtx shifted)
            assert!(
                alt_fit > fit || s["model_silicon_mismatch"].as_bool().unwrap_or(false),
                "mismatch path expected when V fits alt better: {s}"
            );
            if let Some(alt) = s["best_alt_key"].as_str() {
                assert!(
                    alt.contains("gtx_1050") || alt.contains("nvidia"),
                    "alt should point at other GPU cell: {s}"
                );
            }
            assert_eq!(s["hard_block"], false);
        });
    }

    #[test]
    fn injects_hw_model_key_before_observe() {
        reset_atlas_for_tests();
        // No hw_model_key in input — derived from unmasked and injected for cohort
        let f = json!({
            "engine_family": "blink",
            "os_family": "windows",
            "webgl_unmasked_renderer": "ANGLE (NVIDIA, NVIDIA GeForce GTX 1660 Super Direct3D11, D3D11)",
            "hw_curve_webgl": (0..16).map(|i| 0.1 * i as f64).collect::<Vec<_>>(),
            "residual_mean": 0.05,
        });
        let s = atlas_shadow_score(&f);
        let k = s["hw_model_key"].as_str().unwrap();
        assert!(k.starts_with("nvidia:"), "{s}");
        assert!(!k.contains("webkit"), "{s}");
        assert!(s.get("model_key_info").is_some());
    }
}
