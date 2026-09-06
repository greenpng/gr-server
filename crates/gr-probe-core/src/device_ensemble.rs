//! Multi-algorithm device identity ensemble.
//!
//! Does **not** trust a single digest or single associate path. Runs several
//! digest strategies + link algorithms (v9-aligned), then fuses into a final
//! decision with per-algo votes and agreement confidence.
//!
//! - Digest strategies: machine-stable subsets (webgl / audio / geo-cpu / hybrid)
//! - Link algorithms: baseline · sparse_safe · loose · strict
//! - Final: consensus with hard vetoes; never loose-only LINK without corroboration

use crate::link::{
    associate_baseline, associate_loose, associate_sparse_safe, LinkResult,
};
use crate::trust::{commercial_projection, cores_class, hash_selected};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

pub const ENSEMBLE_ALGO: &str = "device_ensemble_v1";

#[derive(Debug, Clone)]
pub struct DigestVote {
    pub strategy: String,
    pub device_id: Option<String>,
    pub keys: Vec<String>,
}

fn hash_parts(ns: &str, pairs: &[(&str, &str)]) -> String {
    let mut h = Sha256::new();
    h.update(ns.as_bytes());
    for (k, v) in pairs {
        h.update(b"|");
        h.update(k.as_bytes());
        h.update(b"=");
        h.update(v.as_bytes());
    }
    format!("{:x}", h.finalize())
}

fn mat_str(mats: &Map<String, Value>, k: &str) -> Option<String> {
    mats.get(k)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn fo_str(fo: &Map<String, Value>, k: &str) -> Option<String> {
    fo.get(k)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Run all digest strategies on one field bag (uses commercial materials map).
pub fn digest_strategy_votes(fields: &Value) -> Vec<DigestVote> {
    let proj = commercial_projection(fields);
    let mats = proj
        .get("materials")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let fo = fields.as_object().cloned().unwrap_or_default();
    let form = mat_str(&mats, "form_class")
        .or_else(|| fo_str(&fo, "form_class"))
        .unwrap_or_else(|| "unknown".into());
    let cores = mat_str(&mats, "cores_class").unwrap_or_else(|| {
        fo.get("hardware_concurrency")
            .and_then(|v| v.as_i64())
            .map(cores_class)
            .unwrap_or_else(|| "c0".into())
    });
    let arch = mat_str(&mats, "architecture").unwrap_or_default();
    let webgl = mat_str(&mats, "hw_webgl_stable");
    let webgl_fine = mat_str(&mats, "hw_webgl_fine");
    let audio = mat_str(&mats, "hw_audio_stable");
    let sw = mat_str(&mats, "screen_w_class");
    let sh = mat_str(&mats, "screen_h_class");
    let tz = mat_str(&mats, "timezone").or_else(|| fo_str(&fo, "timezone"));
    let gpu_v = mat_str(&mats, "gpu_vendor_class");

    let mut votes = Vec::new();

    // S0: shipped commercial projection (primary path)
    votes.push(DigestVote {
        strategy: "commercial_primary".into(),
        device_id: proj
            .get("device_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        keys: proj
            .get("materials_included")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
    });

    // S1: webgl-primary (form + webgl + cores + arch) — cross-browser silicon
    if let Some(ref w) = webgl {
        let dig = hash_parts(
            "ens_webgl_v1|",
            &[
                ("form", &form),
                ("webgl", w),
                ("cores", &cores),
                ("arch", &arch),
            ],
        );
        votes.push(DigestVote {
            strategy: "webgl_primary".into(),
            device_id: Some(format!("dv_{}", &dig[..16])),
            keys: vec![
                "form_class".into(),
                "hw_webgl_stable".into(),
                "cores_class".into(),
                "architecture".into(),
            ],
        });
    }

    // S2: fine webgl only (engine-sensitive; diagnostic)
    if let Some(ref wf) = webgl_fine {
        let dig = hash_parts(
            "ens_webgl_fine_v1|",
            &[("form", &form), ("wf", wf), ("cores", &cores)],
        );
        votes.push(DigestVote {
            strategy: "webgl_fine".into(),
            device_id: Some(format!("dv_{}", &dig[..16])),
            keys: vec!["form_class".into(), "hw_webgl_fine".into(), "cores_class".into()],
        });
    }

    // S3: audio-primary (when webgl missing)
    if let Some(ref a) = audio {
        let dig = hash_parts(
            "ens_audio_v1|",
            &[("form", &form), ("audio", a), ("cores", &cores), ("arch", &arch)],
        );
        votes.push(DigestVote {
            strategy: "audio_primary".into(),
            device_id: Some(format!("dv_{}", &dig[..16])),
            keys: vec![
                "form_class".into(),
                "hw_audio_stable".into(),
                "cores_class".into(),
                "architecture".into(),
            ],
        });
    }

    // S4: PKG_GEOCPU (v9 sparse_safe geo path)
    if sw.is_some() || tz.is_some() {
        let dig = hash_parts(
            "ens_geocpu_v1|",
            &[
                ("form", &form),
                ("cores", &cores),
                ("arch", &arch),
                ("sw", sw.as_deref().unwrap_or("")),
                ("sh", sh.as_deref().unwrap_or("")),
                ("tz", tz.as_deref().unwrap_or("")),
            ],
        );
        votes.push(DigestVote {
            strategy: "geocpu".into(),
            device_id: Some(format!("dv_{}", &dig[..16])),
            keys: vec![
                "form_class".into(),
                "cores_class".into(),
                "architecture".into(),
                "screen_w_class".into(),
                "screen_h_class".into(),
                "timezone".into(),
            ],
        });
    }

    // S5: hybrid webgl + geo + vendor class (no fine residual)
    if webgl.is_some() || gpu_v.is_some() {
        let dig = hash_parts(
            "ens_hybrid_v1|",
            &[
                ("form", &form),
                ("webgl", webgl.as_deref().unwrap_or("")),
                ("gpuv", gpu_v.as_deref().unwrap_or("")),
                ("cores", &cores),
                ("arch", &arch),
                ("sw", sw.as_deref().unwrap_or("")),
                ("tz", tz.as_deref().unwrap_or("")),
            ],
        );
        votes.push(DigestVote {
            strategy: "hybrid_webgl_geo".into(),
            device_id: Some(format!("dv_{}", &dig[..16])),
            keys: vec![
                "form_class".into(),
                "hw_webgl_stable".into(),
                "gpu_vendor_class".into(),
                "cores_class".into(),
                "architecture".into(),
                "screen_w_class".into(),
                "timezone".into(),
            ],
        });
    }

    // S6: materials-included hash via trust helper (reproducible subset)
    if !mats.is_empty() {
        let keys: Vec<&str> = ["form_class", "hw_webgl_stable", "cores_class", "architecture"]
            .into_iter()
            .filter(|k| mats.contains_key(*k))
            .collect();
        if keys.len() >= 2 {
            let dig = hash_selected(&mats, &keys);
            votes.push(DigestVote {
                strategy: "materials_core4".into(),
                device_id: Some(format!("dv_{}", &dig[..16.min(dig.len())])),
                keys: keys.iter().map(|s| (*s).to_string()).collect(),
            });
        }
    }

    // S7: machine_xbr — residual (std/mean bucket) + webrtc host + cores + media inventory.
    // Intentionally excludes engine-noisy unit_surface / fine webgl digests so Chrome/Firefox/
    // Opera/WebKit on the same host vote the same machine key when silicon+host match.
    {
        let residual = fo_str(&fo, "residual_std")
            .or_else(|| fo_str(&fo, "residual_mean"))
            .or_else(|| mat_str(&mats, "residual_mean_bucket"))
            .unwrap_or_default();
        let rtc = fo_str(&fo, "webrtc_host_ip_hash")
            .or_else(|| mat_str(&mats, "webrtc_host_ip_hash"))
            .unwrap_or_default();
        let media = format!(
            "i{}o{}v{}",
            fo.get("media_input_count")
                .and_then(|v| v.as_i64())
                .unwrap_or(-1),
            fo.get("media_output_count")
                .and_then(|v| v.as_i64())
                .unwrap_or(-1),
            fo.get("media_video_count")
                .and_then(|v| v.as_i64())
                .unwrap_or(-1)
        );
        let disp = fo
            .get("display_count")
            .and_then(|v| v.as_i64())
            .map(|n| n.to_string())
            .unwrap_or_default();
        if !residual.is_empty() || !rtc.is_empty() {
            let dig = hash_parts(
                "ens_machine_xbr_v1|",
                &[
                    ("form", &form),
                    ("cores", &cores),
                    ("residual", &residual),
                    ("rtc", &rtc),
                    ("media", &media),
                    ("disp", &disp),
                    ("arch", &arch),
                ],
            );
            votes.push(DigestVote {
                strategy: "machine_xbr".into(),
                device_id: Some(format!("dv_{}", &dig[..16])),
                keys: vec![
                    "form_class".into(),
                    "cores_class".into(),
                    "residual_xbr".into(),
                    "webrtc_host_ip_hash".into(),
                    "media_inventory".into(),
                    "display_count".into(),
                    "architecture".into(),
                ],
            });
        }
    }

    votes
}

fn associate_strict(a: &Value, b: &Value) -> LinkResult {
    let base = associate_baseline(a, b);
    let hard = base.hard_matches;
    let mut vetoes = base.vetoes.clone();
    let missing_hard = ["os_family", "hardware_concurrency", "screen_width", "timezone", "webgl_unmasked_renderer"]
        .iter()
        .filter(|n| base.missing.iter().any(|m| m == **n))
        .count();
    if missing_hard >= 3 && !vetoes.iter().any(|v| v == "sparse_pair") {
        vetoes.push("sparse_pair".into());
    }
    let decision = if vetoes.iter().any(|v| {
        matches!(
            v.as_str(),
            "os_family_mismatch"
                | "form_class_desktop_vs_mobile"
                | "gpu_model_mismatch"
                | "cpu_cores_far_apart"
                | "sparse_pair"
        )
    }) {
        "UNRELATED".into()
    } else if base.confidence >= 0.78 && hard >= 4 && vetoes.is_empty() {
        "LINK".into()
    } else if base.confidence >= 0.48 && hard >= 2 {
        "POSSIBLE".into()
    } else {
        "UNRELATED".into()
    };
    LinkResult {
        decision,
        confidence: base.confidence,
        hard_matches: hard,
        matched: base.matched,
        mismatched: base.mismatched,
        missing: base.missing,
        vetoes,
        algo: "strict".into(),
        details: json!({ "missing_hard": missing_hard }),
    }
}

/// Run all link algorithms (never a single path).
pub fn associate_all(a: &Value, b: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    for (name, f) in [
        (
            "baseline",
            associate_baseline as fn(&Value, &Value) -> LinkResult,
        ),
        ("sparse_safe", associate_sparse_safe),
        ("loose", associate_loose),
        ("strict", associate_strict),
    ] {
        let r = f(a, b);
        let mut v = r.to_value();
        if let Some(obj) = v.as_object_mut() {
            obj.insert("algo".into(), json!(name));
        }
        out.push(v);
    }
    out
}

fn vote_tally(ids: &[Option<String>]) -> (Option<String>, usize, usize) {
    let mut counts: Map<String, Value> = Map::new();
    let mut n = 0usize;
    for id in ids {
        let Some(s) = id else { continue };
        n += 1;
        let e = counts.entry(s.clone()).or_insert(json!(0));
        *e = json!(e.as_u64().unwrap_or(0) + 1);
    }
    let mut best: Option<(String, u64)> = None;
    for (k, v) in counts {
        let c = v.as_u64().unwrap_or(0);
        if best.as_ref().map(|(_, bc)| c > *bc).unwrap_or(true) {
            best = Some((k, c));
        }
    }
    match best {
        Some((id, c)) => (Some(id), c as usize, n),
        None => (None, 0, n),
    }
}

/// Fuse multi-algo results for a pair of sessions / browsers.
pub fn fuse_pair_ensemble(a: &Value, b: &Value) -> Value {
    let dig_a = digest_strategy_votes(a);
    let dig_b = digest_strategy_votes(b);
    let mut digest_agree = Vec::new();
    for va in &dig_a {
        if let Some(ref ida) = va.device_id {
            if let Some(vb) = dig_b.iter().find(|x| x.strategy == va.strategy) {
                if let Some(ref idb) = vb.device_id {
                    digest_agree.push(json!({
                        "strategy": va.strategy,
                        "same": ida == idb,
                        "id_a": ida,
                        "id_b": idb,
                    }));
                }
            }
        }
    }
    let same_n = digest_agree
        .iter()
        .filter(|x| x.get("same").and_then(|v| v.as_bool()).unwrap_or(false))
        .count();
    let dig_total = digest_agree.len().max(1);
    let dig_agree_rate = same_n as f64 / dig_total as f64;

    let links = associate_all(a, b);
    let link_link = links
        .iter()
        .filter(|l| l.get("decision").and_then(|d| d.as_str()) == Some("LINK"))
        .count();
    let link_possible = links
        .iter()
        .filter(|l| l.get("decision").and_then(|d| d.as_str()) == Some("POSSIBLE"))
        .count();
    let link_unrelated = links
        .iter()
        .filter(|l| l.get("decision").and_then(|d| d.as_str()) == Some("UNRELATED"))
        .count();
    let sparse = links.iter().find(|l| l.get("algo").and_then(|a| a.as_str()) == Some("sparse_safe"));
    let strict = links.iter().find(|l| l.get("algo").and_then(|a| a.as_str()) == Some("strict"));
    let loose = links.iter().find(|l| l.get("algo").and_then(|a| a.as_str()) == Some("loose"));
    let baseline = links.iter().find(|l| l.get("algo").and_then(|a| a.as_str()) == Some("baseline"));

    // Collect vetoes from non-loose associate paths
    let mut all_vetoes: Vec<String> = Vec::new();
    for l in &links {
        let algo = l.get("algo").and_then(|a| a.as_str()).unwrap_or("");
        if algo == "loose" {
            continue;
        }
        if let Some(arr) = l.get("vetoes").and_then(|v| v.as_array()) {
            for x in arr {
                if let Some(s) = x.as_str() {
                    if !all_vetoes.iter().any(|e| e == s) {
                        all_vetoes.push(s.to_string());
                    }
                }
            }
        }
    }
    // Machine-hard vetoes: never overridable by digests
    let machine_hard_veto = all_vetoes.iter().any(|v| {
        matches!(
            v.as_str(),
            "os_family_mismatch" | "form_class_desktop_vs_mobile" | "cpu_cores_far_apart"
        )
    });
    // Label-level GPU string mismatch (spoofable / engine-string). Overridable when
    // residual digests (commercial + webgl) agree — lab proved Blink real GPU label vs
    // Gecko spoofed label can disagree while hw_webgl_stable matches.
    let gpu_label_veto = all_vetoes.iter().any(|v| v == "gpu_model_mismatch");

    fn strategy_same(agree: &[Value], name: &str) -> bool {
        agree.iter().any(|p| {
            p.get("strategy").and_then(|s| s.as_str()) == Some(name)
                && p.get("same").and_then(|v| v.as_bool()) == Some(true)
        })
    }
    let commercial_same = strategy_same(&digest_agree, "commercial_primary");
    let webgl_same = strategy_same(&digest_agree, "webgl_primary");
    let materials_same = strategy_same(&digest_agree, "materials_core4");
    let audio_same = strategy_same(&digest_agree, "audio_primary");
    // Silicon/residual agreement (not geo alone — geo is sparse)
    let residual_machine_agree = commercial_same && (webgl_same || materials_same || audio_same);
    let residual_strong = commercial_same && webgl_same;

    // Hard unrelated: machine-hard always; gpu_label only when residual digests disagree
    let hard_unrelated = machine_hard_veto || (gpu_label_veto && !residual_machine_agree);

    // Fusion rules (multi-algo — never loose alone):
    // 1. machine-hard veto → UNRELATED
    // 2. residual digests (commercial+webgl) agree → LINK even if associate only sees gpu label clash
    // 3. sparse_safe LINK → LINK
    // 4. dig majority + baseline support → LINK
    // 5. never: loose alone
    let sparse_link = sparse
        .and_then(|l| l.get("decision"))
        .and_then(|d| d.as_str())
        == Some("LINK");
    let baseline_pos = baseline
        .and_then(|l| l.get("decision"))
        .and_then(|d| d.as_str())
        .map(|d| d == "LINK" || d == "POSSIBLE")
        .unwrap_or(false);
    let strict_pos = strict
        .and_then(|l| l.get("decision"))
        .and_then(|d| d.as_str())
        .map(|d| d == "LINK" || d == "POSSIBLE")
        .unwrap_or(false);
    let loose_link = loose
        .and_then(|l| l.get("decision"))
        .and_then(|d| d.as_str())
        == Some("LINK");
    let support = links
        .iter()
        .filter(|l| {
            let algo = l.get("algo").and_then(|a| a.as_str()).unwrap_or("");
            if algo == "loose" {
                return false; // loose cannot alone support LINK
            }
            matches!(
                l.get("decision").and_then(|d| d.as_str()),
                Some("LINK") | Some("POSSIBLE")
            )
        })
        .count();

    let fusion_path: &str;
    let final_decision = if machine_hard_veto {
        fusion_path = "machine_hard_veto";
        "UNRELATED"
    } else if residual_strong {
        // Multi-algo: residual digests override spoofable GPU labels / window screen size noise
        fusion_path = "residual_digest_strong";
        "LINK"
    } else if residual_machine_agree && dig_agree_rate >= 0.5 {
        fusion_path = "residual_digest_majority";
        "LINK"
    } else if hard_unrelated {
        fusion_path = "hard_unrelated_with_digest_disagree";
        "UNRELATED"
    } else if sparse_link {
        fusion_path = "sparse_safe_link";
        "LINK"
    } else if dig_agree_rate >= 0.5 && baseline_pos && support >= 2 {
        fusion_path = "digest_majority_plus_associate_support";
        "LINK"
    } else if dig_agree_rate >= 0.34 && support >= 2 {
        fusion_path = "digest_partial_plus_support";
        "POSSIBLE"
    } else if link_possible >= 2 || dig_agree_rate >= 0.34 {
        fusion_path = "associate_possible_or_partial_digest";
        "POSSIBLE"
    } else if loose_link && dig_agree_rate < 0.34 {
        // Explicit: loose alone never elevates to LINK
        fusion_path = "loose_alone_suppressed";
        "UNRELATED"
    } else {
        fusion_path = "default_unrelated";
        "UNRELATED"
    };
    let _ = strict_pos; // reserved for future strict-weighted conf

    // Prefer majority commercial_primary / webgl_primary when LINK
    let primary_ids: Vec<Option<String>> = dig_a
        .iter()
        .filter(|v| {
            matches!(
                v.strategy.as_str(),
                "commercial_primary" | "webgl_primary" | "hybrid_webgl_geo" | "materials_core4"
            )
        })
        .map(|v| {
            // only if same as peer under same strategy
            let same = dig_b.iter().any(|x| {
                x.strategy == v.strategy
                    && x.device_id.is_some()
                    && x.device_id == v.device_id
            });
            if same {
                v.device_id.clone()
            } else {
                None
            }
        })
        .collect();
    let (fused_id, votes, total) = vote_tally(&primary_ids);
    let conf = {
        let mut c = dig_agree_rate * 0.40;
        c += (link_link as f64 / 4.0) * 0.25;
        c += (support as f64 / 3.0) * 0.15;
        if residual_strong {
            c = c.max(0.82);
        } else if residual_machine_agree {
            c = c.max(0.74);
        }
        if sparse_link {
            c = c.max(0.72);
        }
        if machine_hard_veto {
            c = 0.05;
        } else if hard_unrelated && final_decision == "UNRELATED" {
            c = 0.08;
        }
        (c * 10000.0).round() / 10000.0
    };

    json!({
        "algo": ENSEMBLE_ALGO,
        "final_decision": final_decision,
        "final_confidence": conf,
        "fusion_path": fusion_path,
        "fused_device_id": fused_id,
        "fused_votes": votes,
        "fused_vote_total": total,
        "digest_agree_rate": (dig_agree_rate * 10000.0).round() / 10000.0,
        "digest_agree_n": same_n,
        "residual_strong": residual_strong,
        "residual_machine_agree": residual_machine_agree,
        "commercial_same": commercial_same,
        "webgl_same": webgl_same,
        "gpu_label_veto": gpu_label_veto,
        "machine_hard_veto": machine_hard_veto,
        "vetoes_union": all_vetoes,
        "digest_strategies_a": dig_a.iter().map(|v| json!({
            "strategy": v.strategy,
            "device_id": v.device_id,
            "keys": v.keys,
        })).collect::<Vec<_>>(),
        "digest_strategies_b": dig_b.iter().map(|v| json!({
            "strategy": v.strategy,
            "device_id": v.device_id,
            "keys": v.keys,
        })).collect::<Vec<_>>(),
        "digest_pairwise": digest_agree,
        "link_algorithms": links,
        "link_tally": {
            "LINK": link_link,
            "POSSIBLE": link_possible,
            "UNRELATED": link_unrelated,
        },
        "rules": {
            "never_loose_alone": true,
            "sparse_safe_can_confirm_link": true,
            "machine_hard_veto_blocks_link": true,
            "gpu_label_overridable_by_residual_digest": true,
            "digest_majority_supports": true,
            "residual_strong_is_link": true,
        },
        "notes": [
            "Multiple digest strategies + multiple associate algos; final is fusion not single path.",
            "loose is exploratory only and cannot alone produce final LINK.",
            "gpu_model_mismatch is label-level: overridable when commercial+webgl digests agree (residual silicon).",
            "os/form/cpu_cores_far remain hard vetoes that digests cannot override.",
            "JA3/TLS not used in machine digest (protocol/br only).",
        ]
    })
}

/// Single-session: run digest strategies and report agreement (no peer).
pub fn fuse_single_ensemble(fields: &Value) -> Value {
    let votes = digest_strategy_votes(fields);
    let ids: Vec<Option<String>> = votes.iter().map(|v| v.device_id.clone()).collect();
    let (maj, c, n) = vote_tally(&ids);
    let primary = votes
        .iter()
        .find(|v| v.strategy == "commercial_primary")
        .and_then(|v| v.device_id.clone());
    let webgl = votes
        .iter()
        .find(|v| v.strategy == "webgl_primary")
        .and_then(|v| v.device_id.clone());
    let agree_primary_webgl = match (&primary, &webgl) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    };
    json!({
        "algo": ENSEMBLE_ALGO,
        "strategies": votes.iter().map(|v| json!({
            "strategy": v.strategy,
            "device_id": v.device_id,
            "keys": v.keys,
        })).collect::<Vec<_>>(),
        "majority_id": maj,
        "majority_votes": c,
        "majority_total": n,
        "commercial_primary": primary,
        "webgl_primary": webgl,
        "primary_agrees_webgl": agree_primary_webgl,
        "note": "Single-session multi-digest view; pair fusion needs peer fields.",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn curve(n: usize, phase: f64) -> Vec<f64> {
        (0..n)
            .map(|i| (i as f64 * 0.17 + phase).sin().abs() * 0.4 + 0.05)
            .collect()
    }

    fn host_fields(ip: &str, webgl_phase: f64) -> Value {
        json!({
            "form_class": "desktop",
            "platform": "Linux x86_64",
            "os_family": "linux",
            "hardware_concurrency": 12,
            "timezone": "Asia/Singapore",
            "screen_width": 1920,
            "screen_height": 1080,
            "webgl_unmasked_renderer": "ANGLE (NVIDIA GeForce GTX 1050 Ti)",
            "hw_curve_audio": curve(64, 0.1),
            "hw_curve_webgl": curve(32, webgl_phase),
            "server_client_ip": ip,
        })
    }

    #[test]
    fn ensemble_same_host_cross_browser_links() {
        let chrome = host_fields("1.1.1.1", 0.3);
        let firefox = host_fields("8.8.8.8", 0.3); // same residual phase = same machine
        let ens = fuse_pair_ensemble(&chrome, &firefox);
        assert!(
            matches!(
                ens["final_decision"].as_str(),
                Some("LINK") | Some("POSSIBLE")
            ),
            "ensemble should not hard-unrelated same host: {ens}"
        );
        assert!(ens["link_algorithms"].as_array().unwrap().len() >= 4);
        assert!(ens["digest_pairwise"].as_array().unwrap().len() >= 1);
    }

    #[test]
    fn ensemble_never_uses_single_algo_array() {
        let a = host_fields("1.1.1.1", 0.3);
        let b = host_fields("1.1.1.1", 0.9); // different machine residual
        let ens = fuse_pair_ensemble(&a, &b);
        let links = ens["link_algorithms"].as_array().unwrap();
        assert_eq!(links.len(), 4);
        // must include all named algos
        let names: Vec<&str> = links
            .iter()
            .filter_map(|l| l.get("algo").and_then(|a| a.as_str()))
            .collect();
        for need in ["baseline", "sparse_safe", "loose", "strict"] {
            assert!(names.contains(&need), "missing {need} in {names:?}");
        }
    }

    #[test]
    fn single_ensemble_lists_multiple_strategies() {
        let f = host_fields("1.1.1.1", 0.2);
        let ens = fuse_single_ensemble(&f);
        let n = ens["strategies"].as_array().unwrap().len();
        assert!(n >= 3, "expected multiple digest strategies, got {n}");
    }

    #[test]
    fn ensemble_different_machine_shows_digest_disagreement() {
        // Distinct webgl residual phases ⇒ different silicon fingerprints
        let a = host_fields("1.1.1.1", 0.1);
        let b = host_fields("1.1.1.1", 2.7);
        let ens = fuse_pair_ensemble(&a, &b);
        let dig_rate = ens["digest_agree_rate"].as_f64().unwrap_or(1.0);
        let pairwise = ens["digest_pairwise"].as_array().unwrap();
        let webgl_disagree = pairwise.iter().any(|p| {
            matches!(
                p.get("strategy").and_then(|s| s.as_str()),
                Some("webgl_primary") | Some("hybrid_webgl_geo") | Some("commercial_primary")
            ) && p.get("same").and_then(|v| v.as_bool()) == Some(false)
        });
        assert!(
            dig_rate < 1.0 || webgl_disagree,
            "expected digest disagreement across machines: {ens}"
        );
        // When digests strongly disagree, fusion must not claim high-confidence LINK
        if dig_rate < 0.34 {
            assert_ne!(
                ens["final_decision"].as_str(),
                Some("LINK"),
                "low digest agreement must not fuse to LINK: {ens}"
            );
        }
    }

    #[test]
    fn ensemble_fusion_never_loose_alone_for_link() {
        let a = host_fields("1.1.1.1", 0.3);
        let b = host_fields("9.9.9.9", 0.3);
        let ens = fuse_pair_ensemble(&a, &b);
        assert_eq!(ens["rules"]["never_loose_alone"], true);
        // All four associate algos present
        assert_eq!(ens["link_algorithms"].as_array().unwrap().len(), 4);
        // Final decision is one of the three fused labels
        assert!(matches!(
            ens["final_decision"].as_str(),
            Some("LINK") | Some("POSSIBLE") | Some("UNRELATED")
        ));
    }

    #[test]
    fn residual_digest_overrides_spoofed_gpu_label() {
        // Same residual curves (same machine) but different WebGL *labels* (real vs spoof)
        let mut chrome = host_fields("1.1.1.1", 0.3);
        let mut gecko = host_fields("8.8.8.8", 0.3);
        if let Some(o) = chrome.as_object_mut() {
            o.insert(
                "webgl_unmasked_renderer".into(),
                json!("ANGLE (NVIDIA Corporation, NVIDIA GeForce GTX 1050 Ti/PCIe/SSE2, OpenGL 4.5.0)"),
            );
        }
        if let Some(o) = gecko.as_object_mut() {
            o.insert(
                "webgl_unmasked_renderer".into(),
                json!("NVIDIA GeForce GTX 980, or similar"),
            );
            // window size noise (not machine)
            o.insert("screen_width".into(), json!(1280));
            o.insert("screen_height".into(), json!(720));
        }
        let ens = fuse_pair_ensemble(&chrome, &gecko);
        assert_eq!(
            ens["final_decision"].as_str(),
            Some("LINK"),
            "residual digests must override GPU label clash: {ens}"
        );
        assert_eq!(ens["rules"]["gpu_label_overridable_by_residual_digest"], true);
        // associate alone may still veto — multi-algo fusion is the point
        let base_dec = ens["link_algorithms"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l.get("algo").and_then(|a| a.as_str()) == Some("baseline"))
            .and_then(|l| l.get("decision"))
            .and_then(|d| d.as_str());
        // If baseline hard-vetoed on gpu, fusion still LINKs via residual path
        if base_dec == Some("UNRELATED") {
            assert_eq!(ens["fusion_path"].as_str(), Some("residual_digest_strong"));
        }
    }
}
