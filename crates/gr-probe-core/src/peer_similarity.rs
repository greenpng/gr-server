//! Time-window peer similarity (`peer_similarity_v1`).
//!
//! Product semantics (architecture SSOT):
//! - V5 analyzes **current unit-time window** sessions for clustering severity.
//! - Returns an **independent** score on the session result — higher means this
//!   vtid/session looks more like *other* concurrent vtids (farm / multi-open).
//! - Not a business account graph; does not promote commercial device_id.
//! - Cross-account register/login association lives in the site SDK.

use serde_json::{json, Map, Value};

pub const PEER_SIMILARITY_ALGO: &str = "peer_similarity_v1";
/// Default analysis window: 15 minutes.
pub const DEFAULT_WINDOW_SEC: u64 = 900;

fn s(fo: &Map<String, Value>, k: &str) -> Option<String> {
    fo.get(k)
        .and_then(|v| v.as_str())
        .filter(|x| !x.is_empty())
        .map(|x| x.to_string())
}

fn f(fo: &Map<String, Value>, k: &str) -> Option<f64> {
    fo.get(k)
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
}

fn residual_soft_agree(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => ((x / 0.005).round() - (y / 0.005).round()).abs() < 0.5,
        _ => false,
    }
}

/// Pairwise similarity contribution between self fields and one peer fields map.
pub fn pair_peer_contrib(self_f: &Value, peer_f: &Value) -> (f64, Vec<String>) {
    let a = self_f.as_object().cloned().unwrap_or_default();
    let b = peer_f.as_object().cloned().unwrap_or_default();
    let mut score = 0.0_f64;
    let mut reasons: Vec<String> = Vec::new();

    let a_rm = f(&a, "residual_mean");
    let b_rm = f(&b, "residual_mean");
    // Common commercial floor (0.260/0.261): residual_soft alone is a high-collision
    // signal — demote unless multipath fuse or audio also agree (001 § high-freq demote).
    let residual_floor_bucket = |x: f64| -> i64 { (x * 1000.0).round() as i64 };
    let common_floor = match (a_rm, b_rm) {
        (Some(x), Some(y)) => {
            let bx = residual_floor_bucket(x);
            let by = residual_floor_bucket(y);
            bx == by && (bx == 260 || bx == 261 || bx == 262)
        }
        _ => false,
    };
    let webgl_eq = match (s(&a, "hw_webgl_stable"), s(&b, "hw_webgl_stable")) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    };
    let audio_eq = match (s(&a, "hw_audio_stable"), s(&b, "hw_audio_stable")) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    };
    if residual_soft_agree(a_rm, b_rm) {
        if common_floor && !webgl_eq && !audio_eq {
            // shared GPU floor without multipath/audio agreement — weak peer signal only
            score += 0.08;
            reasons.push("residual_soft_common_floor_demote".into());
        } else {
            score += 0.28;
            reasons.push("residual_soft".into());
        }
    }

    for (k, w, tag) in [
        ("hw_webgl_stable", 0.14, "hw_webgl_stable"),
        ("hw_audio_stable", 0.14, "hw_audio_stable"),
        ("hw_silicon_fine", 0.12, "hw_silicon_fine"),
        ("hw_silicon_fusion", 0.10, "hw_silicon_fusion"),
        ("webrtc_host_ip_hash", 0.22, "webrtc_host"),
        ("webrtc_host_ip_hash_v2", 0.22, "webrtc_host_v2"),
        ("os_instance_hash", 0.18, "os_instance"),
    ] {
        if let (Some(x), Some(y)) = (s(&a, k), s(&b, k)) {
            if x == y {
                // avoid double-count webrtc v1+v2
                if tag == "webrtc_host_v2" && reasons.iter().any(|r| r == "webrtc_host") {
                    continue;
                }
                score += w;
                reasons.push(tag.into());
            }
        }
    }

    // Cross-engine same-host: residual_mean within 0.01 + host separator still peer-linkable
    // (gecko/blink multipath means differ slightly; farm still clusters).
    if !reasons.iter().any(|r| r.starts_with("residual_soft")) {
        match (a_rm, b_rm) {
            (Some(x), Some(y)) if (x - y).abs() <= 0.012 => {
                if webgl_eq || audio_eq || reasons.iter().any(|r| r == "webrtc_host" || r == "os_instance")
                {
                    score += 0.16;
                    reasons.push("residual_near_host".into());
                } else {
                    score += 0.06;
                    reasons.push("residual_near_weak".into());
                }
            }
            _ => {}
        }
    }

    // Device multi-segment exact body match (strong public signal)
    if let (Some(x), Some(y)) = (s(&a, "device_id"), s(&b, "device_id")) {
        if x == y && x.starts_with("dv") {
            score += 0.25;
            reasons.push("device_id_exact".into());
        }
    }

    // Soft host cluster id
    if let (Some(x), Some(y)) = (s(&a, "soft_host_cluster_id"), s(&b, "soft_host_cluster_id")) {
        if x == y {
            score += 0.12;
            reasons.push("soft_host_cluster".into());
        }
    }

    // iss/50 P1-3: curve LSH agreement (cosine proxy via shared short digests) on product path
    if let (Some(ax), Some(bx)) = (a.get("curve_descriptors"), b.get("curve_descriptors")) {
        let mut agree = 0u32;
        let mut n = 0u32;
        if let (Some(aslots), Some(bslots)) = (
            ax.get("slots").and_then(|v| v.as_object()),
            bx.get("slots").and_then(|v| v.as_object()),
        ) {
            for (k, av) in aslots {
                if let Some(bv) = bslots.get(k) {
                    let ad = av
                        .get("lsh")
                        .or_else(|| av.get("digest"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let bd = bv
                        .get("lsh")
                        .or_else(|| bv.get("digest"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if ad.is_empty() || bd.is_empty() {
                        continue;
                    }
                    n += 1;
                    if ad == bd {
                        agree += 1;
                    } else if crate::cluster_lsh::hex_digest_distance(ad, bd) <= 4 {
                        agree += 1;
                        n += 0; // already counted
                    }
                }
            }
        }
        if n > 0 {
            let ratio = agree as f64 / n as f64;
            if ratio >= 0.5 {
                score += 0.10 * ratio;
                reasons.push(format!("curve_lsh_agree={ratio:.2}"));
            }
        }
    }

    // Soft stacks: cap pair contribution (env farm, not silicon UV)
    let soft = a
        .get("soft_stack")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || b.get("soft_stack")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        || a.get("residual_soft_like")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    if soft && score > 0.0 {
        score = score.min(0.75);
        reasons.push("soft_stack_cap".into());
    }

    (score.min(1.0), reasons)
}

/// Aggregate peer similarity from a set of peer field maps (other sessions in window).
///
/// `self_vtid` / peer vtids: same-vtid peers are continuity, not "other vtid cluster".
/// Count **distinct other vtids** for farm severity.
pub fn compute_peer_similarity(
    self_fields: &Value,
    self_vtid: Option<&str>,
    peers: &[(Option<String>, Value)],
    window_sec: u64,
) -> Value {
    let mut peer_session_n = 0u32;
    let mut other_vtids: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut reason_counts: std::collections::BTreeMap<String, u32> =
        std::collections::BTreeMap::new();
    let mut best = 0.0_f64;
    let mut sum = 0.0_f64;
    let mut n_linked = 0u32;

    for (peer_vt, pf) in peers {
        peer_session_n += 1;
        let same_vt = match (self_vtid, peer_vt.as_deref()) {
            (Some(a), Some(b)) if !a.is_empty() && a == b => true,
            _ => false,
        };
        if let Some(v) = peer_vt {
            if !v.is_empty() {
                if !same_vt {
                    other_vtids.insert(v.clone());
                }
            }
        }
        let (c, reasons) = pair_peer_contrib(self_fields, pf);
        // Same-vtid continuity contributes less to "farm among other vtids"
        let c_adj = if same_vt { c * 0.35 } else { c };
        // Lower link floor when host silicon materials agree (serial lab browsers).
        let hostish = reasons.iter().any(|r| {
            matches!(
                r.as_str(),
                "webrtc_host"
                    | "webrtc_host_v2"
                    | "os_instance"
                    | "hw_silicon_fine"
                    | "hw_silicon_fusion"
                    | "residual_near_host"
            )
        });
        let link_floor = if hostish { 0.28 } else { 0.35 };
        if c_adj >= link_floor {
            n_linked += 1;
            sum += c_adj;
            best = best.max(c_adj);
            for r in reasons {
                *reason_counts.entry(r).or_insert(0) += 1;
            }
        }
    }

    // Score: blend best pair + volume of other-vtid links + linked fraction
    let peer_vtid_n = other_vtids.len() as f64;
    let volume = if peer_session_n == 0 {
        0.0
    } else {
        (n_linked as f64 / peer_session_n as f64).min(1.0)
    };
    let mut score = if n_linked == 0 {
        0.0
    } else {
        // best match + mean of linked + log-ish other vtids
        let mean = sum / n_linked as f64;
        let vtid_boost = (peer_vtid_n / 4.0).min(0.25);
        (0.45 * best + 0.35 * mean + 0.20 * volume + vtid_boost).min(0.99)
    };

    // No peers observed → low (independent), not "unknown high"
    if peer_session_n == 0 {
        score = 0.0;
    }

    let mut top_reasons: Vec<(String, u32)> = reason_counts.into_iter().collect();
    top_reasons.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let top: Vec<String> = top_reasons
        .into_iter()
        .take(8)
        .map(|(k, _)| k)
        .collect();

    let severity = if score >= 0.72 {
        "high"
    } else if score >= 0.45 {
        "medium"
    } else {
        "low"
    };

    json!({
        "algo": PEER_SIMILARITY_ALGO,
        "score": (score * 10000.0).round() / 10000.0,
        "window_sec": window_sec,
        "peer_vtid_n": other_vtids.len(),
        "peer_session_n": peer_session_n,
        "linked_peer_n": n_linked,
        "best_pair_score": (best * 10000.0).round() / 10000.0,
        "top_reasons": top,
        "severity": severity,
        "promote_to_commercial_id": false,
        "note": "higher = this vtid/session more similar to other concurrent vtids in window; not a business account graph",
        "sdk_use": "fuse with subject_ref / register-login events on site SDK",
    })
}

/// Build environment_flags for soft / cloud-phone / virt (os product surface).
pub fn environment_flags(fields: &Value, soft_stack: bool) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let mut flags: Vec<String> = Vec::new();
    let mut reasons: Vec<String> = Vec::new();

    let residual_soft = fo
        .get("residual_soft_like")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if soft_stack {
        flags.push("soft_stack".into());
        reasons.push("stack_auth_soft_stack".into());
    }
    if residual_soft {
        flags.push("residual_soft_like".into());
        reasons.push("physical_residual_soft".into());
    }

    let emu = fo
        .get("emulator_hint")
        .map(|v| {
            v.as_bool().unwrap_or(false)
                || v.as_str().is_some_and(|s| !s.is_empty() && s != "false")
        })
        .unwrap_or(false);
    let stack_class = s(&fo, "stack_class").unwrap_or_default();
    let sc_l = stack_class.to_ascii_lowercase();
    if emu || matches!(sc_l.as_str(), "emulator" | "virt" | "vm" | "cloud_phone") {
        flags.push("emulator_or_virt".into());
        reasons.push(format!("stack_class_or_hint={stack_class}"));
    }

    // Cloud phone / farm device heuristics (claim + form + soft)
    let ren = s(&fo, "webgl_unmasked_renderer")
        .or_else(|| s(&fo, "webgl_renderer"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    let ua = s(&fo, "user_agent").unwrap_or_default().to_ascii_lowercase();
    let platform = s(&fo, "platform").unwrap_or_default().to_ascii_lowercase();
    let form = s(&fo, "form_class").unwrap_or_default().to_ascii_lowercase();

    let cloud_phone_hint = ua.contains("cloudphone")
        || ua.contains("cloud_phone")
        || ren.contains("virgl")
        || ren.contains("virtio")
        || ren.contains("llvmpipe")
        || ren.contains("swiftshader")
        || ren.contains("microsoft basic render")
        || (form == "mobile" && (platform.contains("linux") || platform.contains("win")))
        || fo.get("x9_emulator_env").and_then(|v| v.as_bool()).unwrap_or(false)
        || fo.get("cloud_phone_hint").and_then(|v| v.as_bool()).unwrap_or(false);

    if cloud_phone_hint {
        flags.push("cloud_phone_or_soft_gpu".into());
        reasons.push("cloud_phone_heuristic".into());
    }

    // Strong soft: soft residual + exotic discrete GPU claim
    let exotic = ren.contains("geforce")
        || ren.contains("radeon")
        || ren.contains("adreno")
        || ren.contains("mali")
        || ren.contains("apple m");
    if (soft_stack || residual_soft) && exotic {
        flags.push("gpu_claim_vs_soft_obs".into());
        reasons.push("exotic_gpu_label_on_soft_path".into());
    }

    if fo
        .get("software_renderer_heuristic")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        flags.push("software_renderer".into());
    }

    let risk_boost = if flags.iter().any(|f| f == "cloud_phone_or_soft_gpu") {
        0.18
    } else if flags.iter().any(|f| f == "emulator_or_virt") {
        0.14
    } else if soft_stack || residual_soft {
        0.10
    } else {
        0.0
    };

    json!({
        "algo": "environment_flags_v1",
        "flags": flags,
        "reasons": reasons,
        "soft_stack": soft_stack || residual_soft,
        "cloud_phone_suspect": flags.iter().any(|f| f == "cloud_phone_or_soft_gpu"),
        "emulator_or_virt": flags.iter().any(|f| f == "emulator_or_virt"),
        "os_risk_boost": risk_boost,
        "note": "V5 environment classification for site risk; not a device_id material",
    })
}

/// Sanitize client-supplied link fields (never plain PII; never mint materials).
pub fn sanitize_custom_link_fields(fields: &mut Map<String, Value>) -> Value {
    let mut kept = Map::new();
    const MAX_KEYS: usize = 8;
    const MAX_LEN: usize = 128;
    let mut n = 0usize;

    // Accept nested custom_link object or flat custom_link string / client_tags
    if let Some(Value::Object(obj)) = fields.get("custom_link").cloned() {
        for (k, v) in obj {
            if n >= MAX_KEYS {
                break;
            }
            if k.len() > 64 || k.starts_with('_') {
                continue;
            }
            // Ban obvious PII key names
            let kl = k.to_ascii_lowercase();
            if matches!(
                kl.as_str(),
                "email" | "phone" | "password" | "user_id" | "userid" | "name" | "ssn" | "card"
            ) {
                continue;
            }
            if let Some(s) = v.as_str() {
                let t: String = s.chars().take(MAX_LEN).collect();
                if !t.is_empty() {
                    kept.insert(k, json!(t));
                    n += 1;
                }
            } else if v.is_number() || v.is_boolean() {
                kept.insert(k, v);
                n += 1;
            }
        }
        fields.insert("custom_link".into(), Value::Object(kept.clone()));
    } else if let Some(s) = fields.get("custom_link").and_then(|v| v.as_str()) {
        let t: String = s.chars().take(MAX_LEN).collect();
        if !t.is_empty() {
            kept.insert("id".into(), json!(t));
            fields.insert("custom_link".into(), json!({ "id": t }));
        }
    }

    if let Some(Value::Object(tags)) = fields.get("client_tags").cloned() {
        let mut tmap = Map::new();
        let mut tn = 0usize;
        for (k, v) in tags {
            if tn >= MAX_KEYS {
                break;
            }
            if let Some(s) = v.as_str() {
                let t: String = s.chars().take(MAX_LEN).collect();
                if !t.is_empty() && k.len() <= 64 {
                    tmap.insert(k, json!(t));
                    tn += 1;
                }
            }
        }
        fields.insert("client_tags".into(), Value::Object(tmap.clone()));
        if kept.is_empty() {
            for (k, v) in tmap {
                kept.insert(k, v);
            }
        }
    }

    // Strip fixed-name materials if client tried to inject into link bag only — already separate.
    // Mark for evidence: not for device mint
    fields.insert(
        "custom_link_policy".into(),
        json!({
            "max_keys": MAX_KEYS,
            "max_value_len": MAX_LEN,
            "pii_keys_stripped": true,
            "never_device_mint": true,
        }),
    );

    json!({
        "custom_link": Value::Object(kept),
        "accepted": n > 0 || fields.get("custom_link").is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_similarity_other_vtids() {
        let self_f = json!({
            "residual_mean": 0.26,
            "webrtc_host_ip_hash": "rtc1",
            "hw_audio_stable": "au1",
            "device_id": "dv0-aaaaaaaaaa-bbbbbbbbbb-0-0-0-0-0-0-0-0",
        });
        let peers = vec![
            (
                Some("vt_b".into()),
                json!({
                    "residual_mean": 0.261,
                    "webrtc_host_ip_hash": "rtc1",
                    "hw_audio_stable": "au1",
                    "device_id": "dv0-aaaaaaaaaa-bbbbbbbbbb-0-0-0-0-0-0-0-0",
                }),
            ),
            (
                Some("vt_c".into()),
                json!({
                    "residual_mean": 0.259,
                    "webrtc_host_ip_hash": "rtc1",
                }),
            ),
        ];
        let out = compute_peer_similarity(&self_f, Some("vt_a"), &peers, 900);
        assert!(out["score"].as_f64().unwrap() >= 0.45, "{out}");
        assert_eq!(out["peer_vtid_n"], 2);
        assert!(matches!(
            out["severity"].as_str(),
            Some("medium") | Some("high")
        ));
    }

    #[test]
    fn low_when_no_peers() {
        let out = compute_peer_similarity(&json!({"residual_mean": 0.2}), Some("vt"), &[], 900);
        assert_eq!(out["score"], 0.0);
        assert_eq!(out["severity"], "low");
    }

    #[test]
    fn environment_cloud_phone() {
        let f = environment_flags(
            &json!({
                "form_class": "mobile",
                "platform": "Linux armv8l",
                "webgl_renderer": "Google SwiftShader",
                "residual_soft_like": true,
            }),
            true,
        );
        assert_eq!(f["cloud_phone_suspect"], true);
        assert!(f["os_risk_boost"].as_f64().unwrap() > 0.0);
    }

    #[test]
    fn sanitize_strips_pii_keys() {
        let mut m = Map::new();
        m.insert(
            "custom_link".into(),
            json!({"email": "a@b.c", "order_cohort": "vip", "id": "sub_v1_abc"}),
        );
        let out = sanitize_custom_link_fields(&mut m);
        let cl = m.get("custom_link").unwrap();
        assert!(cl.get("email").is_none());
        assert_eq!(cl["order_cohort"], "vip");
        assert!(out["accepted"].as_bool().unwrap());
    }
}
