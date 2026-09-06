//! Claim–observation graph + shadow scores (iss/47 P2–P3 skeleton).
//!
//! Builds a machine-readable graph of claim vs observed signals for os/br,
//! and a **shadow** score that never drives production gate by default.

use serde_json::{json, Map, Value};

pub const CLAIM_OBS_GRAPH_ALGO: &str = "claim_obs_graph_v1";
pub const SHADOW_SCORE_ALGO: &str = "shadow_score_v1";

fn s(fo: &Map<String, Value>, k: &str) -> String {
    fo.get(k)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn present(fo: &Map<String, Value>, k: &str) -> bool {
    match fo.get(k) {
        None | Some(Value::Null) => false,
        Some(Value::String(x)) => !x.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(_) => true,
    }
}

/// Edge: claim field → observation field with relation.
fn edge(claim: &str, obs: &str, relation: &str, severity: &str) -> Value {
    json!({
        "claim": claim,
        "observation": obs,
        "relation": relation,
        "severity": severity,
    })
}

/// Build claim-obs graph from fields (engine, GPU, UA, soft stack).
pub fn build_claim_obs_graph(fields: &Value) -> Value {
    let fo = fields.as_object().cloned().unwrap_or_default();
    let mut edges: Vec<Value> = Vec::new();
    let mut nodes: Vec<String> = Vec::new();

    let push_node = |nodes: &mut Vec<String>, n: &str| {
        if !nodes.iter().any(|x| x == n) {
            nodes.push(n.to_string());
        }
    };

    // Engine family claim vs obs
    let (claim_eng, obs_eng) = crate::protocol_edge::derive_engine_claim_obs(&fo);
    push_node(&mut nodes, "engine_claim");
    push_node(&mut nodes, "engine_obs");
    let eng_rel = if claim_eng == obs_eng {
        "agree"
    } else if claim_eng == "unknown" || obs_eng == "unknown" {
        "partial"
    } else {
        "conflict"
    };
    edges.push(edge(
        "engine_claim",
        "engine_obs",
        eng_rel,
        if eng_rel == "conflict" {
            "high"
        } else {
            "low"
        },
    ));

    // GPU label vs residual soft
    if present(&fo, "webgl_unmasked_renderer") {
        push_node(&mut nodes, "gpu_label");
        push_node(&mut nodes, "residual_mean");
        let soft = fo
            .get("residual_soft_like")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let fancy = s(&fo, "webgl_unmasked_renderer").to_lowercase();
        let fancy_gpu = fancy.contains("nvidia")
            || fancy.contains("radeon")
            || fancy.contains("geforce")
            || fancy.contains("rtx");
        let rel = if soft && fancy_gpu {
            "conflict"
        } else if present(&fo, "residual_mean") {
            "corroborate"
        } else {
            "claim_only"
        };
        edges.push(edge(
            "gpu_label",
            "residual_mean",
            rel,
            if rel == "conflict" { "high" } else { "med" },
        ));
    }

    // UA vs platform
    if present(&fo, "user_agent") && present(&fo, "platform") {
        push_node(&mut nodes, "user_agent");
        push_node(&mut nodes, "platform");
        let ua = s(&fo, "user_agent").to_lowercase();
        let plat = s(&fo, "platform").to_lowercase();
        let conflict = (ua.contains("windows") && plat.contains("linux"))
            || (ua.contains("mac") && plat.contains("win"))
            || (ua.contains("android") && plat.contains("win"));
        edges.push(edge(
            "user_agent",
            "platform",
            if conflict { "conflict" } else { "agree" },
            if conflict { "med" } else { "low" },
        ));
    }

    // Gateway UA vs FE UA
    if present(&fo, "gateway_user_agent") && present(&fo, "user_agent") {
        push_node(&mut nodes, "gateway_user_agent");
        push_node(&mut nodes, "user_agent");
        let g = s(&fo, "gateway_user_agent");
        let u = s(&fo, "user_agent");
        edges.push(edge(
            "user_agent",
            "gateway_user_agent",
            if g == u { "agree" } else { "conflict" },
            if g == u { "low" } else { "med" },
        ));
    }

    // WebDriver / automation claim vs RPA bands
    if present(&fo, "webdriver") || present(&fo, "navigator_webdriver") {
        push_node(&mut nodes, "webdriver_claim");
        push_node(&mut nodes, "rpa_automation");
        let wd = fo
            .get("webdriver")
            .or_else(|| fo.get("navigator_webdriver"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let auto = fo
            .get("rpa_bands")
            .and_then(|v| v.get("automation_evidence"))
            .or_else(|| fo.get("automation_evidence"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let rel = if wd && auto < 0.15 {
            "conflict"
        } else if wd {
            "corroborate"
        } else {
            "agree"
        };
        edges.push(edge(
            "webdriver_claim",
            "rpa_automation",
            rel,
            if rel == "conflict" { "high" } else { "low" },
        ));
    }

    // a11y: reduced motion / forced colors claim vs input diversity (iss/47 a11y)
    if present(&fo, "prefers_reduced_motion")
        || present(&fo, "matchMedia_prefers_reduced_motion")
        || present(&fo, "a11y_reduced_motion")
    {
        push_node(&mut nodes, "a11y_reduced_motion");
        push_node(&mut nodes, "pointer_activity");
        let reduced = fo
            .get("prefers_reduced_motion")
            .or_else(|| fo.get("matchMedia_prefers_reduced_motion"))
            .or_else(|| fo.get("a11y_reduced_motion"))
            .and_then(|v| v.as_bool().or_else(|| v.as_str().map(|s| s == "reduce" || s == "true")))
            .unwrap_or(false);
        let pointer_n = fo
            .get("rpa_features_v2")
            .and_then(|v| v.get("features"))
            .and_then(|v| v.get("pointer_n"))
            .or_else(|| fo.get("pointer_n"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        // High pointer volume with reduced-motion is not a hard conflict (a11y users move);
        // only flag when zero motion on sensitive action.
        let zero_move = fo
            .get("sensitive_action_zero_move")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let rel = if reduced && zero_move {
            "corroborate"
        } else if reduced && pointer_n > 80.0 {
            "partial"
        } else {
            "agree"
        };
        edges.push(edge(
            "a11y_reduced_motion",
            "pointer_activity",
            rel,
            "low",
        ));
    }

    // JA4 / protocol engine vs brand claim
    if present(&fo, "ja4") || present(&fo, "protocol_engine") {
        push_node(&mut nodes, "protocol_obs");
        push_node(&mut nodes, "engine_claim");
        let pe = s(&fo, "protocol_engine").to_lowercase();
        let eng_from_proto = if pe.contains("gecko") || pe.contains("firefox") {
            "gecko"
        } else if pe.contains("webkit") || pe.contains("safari") {
            "webkit"
        } else if pe.contains("chrome") || pe.contains("boring") || pe.contains("blink") {
            "blink"
        } else {
            "unknown"
        };
        let rel = if eng_from_proto == "unknown" || claim_eng == "unknown" {
            "partial"
        } else if eng_from_proto == claim_eng {
            "agree"
        } else {
            "conflict"
        };
        edges.push(edge(
            "engine_claim",
            "protocol_obs",
            rel,
            if rel == "conflict" { "med" } else { "low" },
        ));
    }

    // ── A5 claim-obs deepening (iss/74 §4-A5): pure rules, no new fields ──

    // GPU label vs WebGPU adapter family (A5: GPU 串 vs WebGPU adapter 家族互证)
    if present(&fo, "webgl_unmasked_renderer") {
        push_node(&mut nodes, "gpu_label");
        push_node(&mut nodes, "webgpu_adapter");
        let gl = s(&fo, "webgl_unmasked_renderer").to_lowercase();
        let wg = fo
            .get("webgpu_adapter_device")
            .or_else(|| fo.get("webgpu_adapter_description"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase()
            + " "
            + &fo
                .get("webgpu_adapter_architecture")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase();
        let fam = |t: &str| -> &'static str {
            if t.contains("nvidia") || t.contains("geforce") || t.contains("rtx") || t.contains("quadro") {
                "nvidia"
            } else if t.contains("radeon") || t.contains("amd") {
                "amd"
            } else if t.contains("apple") || t.contains("m1") || t.contains("m2") || t.contains("m3")
                || t.contains("m4")
            {
                "apple"
            } else if t.contains("intel") || t.contains("uhd") || t.contains("iris") {
                "intel"
            } else if t.contains("swiftshader") {
                "swiftshader"
            } else if t.contains("mali") {
                "mali"
            } else if t.contains("adreno") {
                "adreno"
            } else {
                "unknown"
            }
        };
        let fg = fam(&gl);
        let fw = fam(&wg);
        let rel = if fg == "unknown" || fw == "unknown" {
            "claim_only"
        } else if fg == fw {
            "corroborate"
        } else {
            "conflict"
        };
        edges.push(edge(
            "gpu_label",
            "webgpu_adapter",
            rel,
            if rel == "conflict" { "high" } else { "low" },
        ));
    }

    // device_memory claim vs allocation measurement (A5: device_memory 声明 vs B26/B39 分配实测)
    if present(&fo, "device_memory") {
        push_node(&mut nodes, "device_memory_claim");
        push_node(&mut nodes, "mem_alloc_max_mb");
        let claim_gb = fo
            .get("device_memory")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let alloc_max_mb = fo
            .get("mem_alloc_max_mb")
            .or_else(|| fo.get("caps_alloc_fail_at"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        if claim_gb > 0.0 && alloc_max_mb > 0.0 {
            // Allocation exceeding the claimed memory by >50% cannot happen on
            // a real device (claim under-reporting or forged memory value).
            let rel = if alloc_max_mb > claim_gb * 1536.0 {
                "conflict"
            } else {
                "corroborate"
            };
            edges.push(edge(
                "device_memory_claim",
                "mem_alloc_max_mb",
                rel,
                if rel == "conflict" { "high" } else { "low" },
            ));
        }
    }

    // WebGL caps vs allocation stress (A5: caps 声称 vs B33/H05 压力结果)
    if present(&fo, "webgl_max_texture")
        && (present(&fo, "caps_alloc_fail_at") || present(&fo, "mem_alloc_ladder"))
    {
        push_node(&mut nodes, "webgl_caps_claim");
        push_node(&mut nodes, "alloc_stress");
        let max_tex = fo
            .get("webgl_max_texture")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let fail_at = fo
            .get("caps_alloc_fail_at")
            .and_then(|v| v.as_f64());
        let ladder_high = fo
            .get("mem_alloc_ladder")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_f64())
                    .fold(0.0f64, f64::max)
            })
            .unwrap_or(0.0);
        let stress_max = ladder_high.max(fail_at.unwrap_or(0.0));
        // Sustained multi-MB texture cap + failing allocations < 64MB is a
        // weak-capability contradiction (real GPUs allocate well beyond).
        let rel = if max_tex >= 4096.0 && stress_max > 0.0 && stress_max < 64.0 {
            "conflict"
        } else {
            "corroborate"
        };
        edges.push(edge(
            "webgl_caps_claim",
            "alloc_stress",
            rel,
            if rel == "conflict" { "med" } else { "low" },
        ));
    }

    // Storage quota claim vs bucket recompute (A5: storage 配额 vs 声称盘位 —
    // grid class must equal the bucket derived from quota bytes; mismatch is a
    // forgery signal or FE/detection bucket drift).
    if present(&fo, "storage_quota_bytes") && present(&fo, "storage_quota_grid_class") {
        push_node(&mut nodes, "storage_quota_claim");
        let q = fo
            .get("storage_quota_bytes")
            .and_then(|v| v.as_f64().or_else(|| v.as_u64().map(|u| u as f64)))
            .unwrap_or(0.0);
        let gb: f64 = 1024.0 * 1024.0 * 1024.0;
        let tb: f64 = 1099511627776.0;
        let expect = if q >= 4.0 * tb {
            "multi_tb"
        } else if q >= tb {
            "tb"
        } else if q >= 512.0 * gb {
            "512g_1t"
        } else if q >= 128.0 * gb {
            "128g_512g"
        } else if q >= 32.0 * gb {
            "32g_128g"
        } else {
            "lt_32g"
        };
        if q > 0.0 {
            let got = fo
                .get("storage_quota_grid_class")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            edges.push(edge(
                "storage_quota_claim",
                "storage_quota_grid_class",
                if got == expect { "agree" } else { "conflict" },
                if got == expect { "low" } else { "high" },
            ));
        }
    }

    // UA brands whitelist vs UA family (A5: brandCapabilities 白名单哈希的
    // 服务端同构 — Chrome-family UA without any Chrome brand is anomalous).
    if present(&fo, "ua_brands") && present(&fo, "user_agent") {
        push_node(&mut nodes, "ua_brands");
        push_node(&mut nodes, "user_agent");
        let brands = s(&fo, "ua_brands").to_lowercase();
        let ua = s(&fo, "user_agent").to_lowercase();
        let ua_chrome = ua.contains("chrome") || ua.contains("crios") || ua.contains("chromium");
        let ua_ff = ua.contains("firefox");
        let brands_chrome = brands.contains("chrome") || brands.contains("chromium");
        let brands_ff = brands.contains("firefox");
        let conflict = (ua_chrome && !brands_chrome && !brands_ff)
            || (ua_ff && !brands_ff && !brands_chrome);
        edges.push(edge(
            "ua_brands",
            "user_agent",
            if conflict { "conflict" } else { "agree" },
            if conflict { "med" } else { "low" },
        ));
    }

    // blink fork guess vs UA family (B93 fork matrix): a fork guess that is
    // family-incompatible with the UA is a claim-obs conflict (e.g. UA says
    // Chrome but CSS vars say floorp AND fork_claim_vs_obs mismatch).
    if present(&fo, "blink_fork_guess") && present(&fo, "user_agent") {
        push_node(&mut nodes, "blink_fork_guess");
        push_node(&mut nodes, "user_agent");
        let guess = s(&fo, "blink_fork_guess").to_lowercase();
        let ua = s(&fo, "user_agent").to_lowercase();
        let ua_fam = if ua.contains("firefox") {
            "firefox"
        } else if ua.contains("edg/") {
            "edge"
        } else if ua.contains("opr/") {
            "opera"
        } else if ua.contains("safari") && !ua.contains("chrome") {
            "safari"
        } else if ua.contains("chrome") || ua.contains("chromium") {
            "chrome"
        } else {
            ""
        };
        let compatible = ua_fam.is_empty()
            || guess == ua_fam
            || (guess == "brave" && ua_fam == "chrome")
            || (guess == "arc" && ua_fam == "chrome")
            || (guess == "floorp" && ua_fam == "firefox")
            || (guess == "waterfox" && ua_fam == "firefox")
            || (guess == "unknown");
        let fe_says_mismatch = s(&fo, "fork_claim_vs_obs") == "mismatch";
        let conflict = !compatible || fe_says_mismatch;
        edges.push(edge(
            "blink_fork_guess",
            "user_agent",
            if conflict { "conflict" } else { "agree" },
            if conflict { "med" } else { "low" },
        ));
    }

    // sec-ch-ua vs UA brand
    if present(&fo, "sec_ch_ua") && present(&fo, "user_agent") {
        push_node(&mut nodes, "sec_ch_ua");
        push_node(&mut nodes, "user_agent");
        let ch = s(&fo, "sec_ch_ua").to_lowercase();
        let ua = s(&fo, "user_agent").to_lowercase();
        let ch_ff = ch.contains("firefox");
        let ua_ff = ua.contains("firefox");
        let ch_chr = ch.contains("chrome") || ch.contains("chromium");
        let ua_chr = ua.contains("chrome") || ua.contains("crios");
        let conflict = (ch_ff && ua_chr && !ua_ff) || (ch_chr && ua_ff && !ch_ff);
        edges.push(edge(
            "sec_ch_ua",
            "user_agent",
            if conflict { "conflict" } else { "agree" },
            if conflict { "med" } else { "low" },
        ));
    }

    let n_conflict = edges
        .iter()
        .filter(|e| e.get("relation").and_then(|v| v.as_str()) == Some("conflict"))
        .count();
    let n_a11y = edges
        .iter()
        .filter(|e| {
            e.get("claim").and_then(|v| v.as_str()) == Some("a11y_reduced_motion")
        })
        .count();

    json!({
        "algo": CLAIM_OBS_GRAPH_ALGO,
        "nodes": nodes,
        "edges": edges,
        "n_conflict": n_conflict,
        "n_a11y_edges": n_a11y,
        "engine_claim": claim_eng,
        "engine_obs": obs_eng,
        "note": "iss/47 P2: full claim-obs graph (engine/gpu/ua/a11y/protocol); gates use product scores not graph alone",
    })
}

/// Shadow score: parallel risk signal that does **not** replace production os/br/rpa.
pub fn shadow_score_from_graph(graph: &Value, fields: &Value) -> Value {
    let n_conflict = graph.get("n_conflict").and_then(|v| v.as_u64()).unwrap_or(0) as f64;
    let mut shadow_risk = (0.15 + 0.12 * n_conflict).min(0.95);
    let fo = fields.as_object().cloned().unwrap_or_default();
    if fo
        .get("webdriver")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        shadow_risk = (shadow_risk + 0.2).min(0.99);
    }
    if fo
        .get("soft_stack")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        shadow_risk = (shadow_risk + 0.1).min(0.99);
    }
    let shadow_safety = 1.0 - shadow_risk;
    json!({
        "algo": SHADOW_SCORE_ALGO,
        "shadow_risk": (shadow_risk * 10000.0).round() / 10000.0,
        "shadow_safety": (shadow_safety * 10000.0).round() / 10000.0,
        "drives_production_gate": false,
        "canary_eligible": true,
        "note": "iss/47 P3 shadow: log/compare only until canary promotes",
    })
}

/// Canary model (iss/47 P4 skeleton): shadow vs production RPA/OS — log/compare only.
///
/// Does **not** promote shadow into production gate. Used for offline A/B dashboards
/// and eventual canary traffic split when ops enables it.
pub fn canary_model_compare(shadow: &Value, rpa: &Value, os: &Value) -> Value {
    let shadow_risk = shadow
        .get("shadow_risk")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.3);
    let auto = rpa
        .pointer("/rpa_bands/automation_evidence")
        .or_else(|| rpa.get("automation_evidence"))
        .and_then(|v| v.as_f64())
        .or_else(|| {
            rpa.get("risk")
                .and_then(|v| v.as_f64())
                .or_else(|| rpa.pointer("/score/risk").and_then(|v| v.as_f64()))
        })
        .unwrap_or(0.25);
    let os_risk = os
        .get("risk")
        .and_then(|v| v.as_f64())
        .or_else(|| os.pointer("/score/risk").and_then(|v| v.as_f64()))
        .unwrap_or(0.2);
    let prod_risk = (0.55 * auto + 0.45 * os_risk).clamp(0.0, 1.0);
    let delta = shadow_risk - prod_risk;
    let agree = delta.abs() < 0.18;
    // Stable bucket for traffic split bookkeeping (not applied unless ops enables).
    let split_active = gr_abi::env::get("CANARY_TRAFFIC_SPLIT")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let canary_pct: u8 = gr_abi::env::get("CANARY_PERCENT")
        .and_then(|s| s.parse().ok())
        .unwrap_or(10)
        .clamp(0, 100);
    let bucket = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(format!("{shadow_risk:.4}|{prod_risk:.4}").as_bytes());
        let dig = format!("{:x}", h.finalize());
        let n = u8::from_str_radix(&dig[..2], 16).unwrap_or(0) as u16;
        let threshold = (canary_pct as u16) * 256 / 100;
        if n < threshold {
            "canary"
        } else {
            "control"
        }
    };
    // Even when split is active, shadow does not replace production scores —
    // it only tags the session for logging / dual-path comparison.
    json!({
        "algo": "canary_model_v1",
        "shadow_risk": (shadow_risk * 10000.0).round() / 10000.0,
        "production_proxy_risk": (prod_risk * 10000.0).round() / 10000.0,
        "delta_shadow_minus_prod": (delta * 10000.0).round() / 10000.0,
        "signals_agree": agree,
        "bucket_hint": bucket,
        "canary_percent": canary_pct,
        "traffic_split_active": split_active,
        "drives_production_gate": false,
        "promote_requires": "ops canary enable + label sample + ECE gate",
        "note": "iss/47 P4: compare-only; GR_CANARY_TRAFFIC_SPLIT=1 enables bucket tagging only",
    })
}

/// Combined surface for product/diagnostics.
pub fn claim_obs_and_shadow(fields: &Value) -> Value {
    let graph = build_claim_obs_graph(fields);
    let shadow = shadow_score_from_graph(&graph, fields);
    json!({
        "claim_obs_graph": graph,
        "shadow_score": shadow,
    })
}

/// Full claim-obs + shadow + canary surface when product rpa/os available.
pub fn claim_obs_shadow_canary(fields: &Value, rpa: &Value, os: &Value) -> Value {
    let base = claim_obs_and_shadow(fields);
    let shadow = base.get("shadow_score").cloned().unwrap_or(json!({}));
    let canary = canary_model_compare(&shadow, rpa, os);
    json!({
        "claim_obs_graph": base.get("claim_obs_graph").cloned().unwrap_or(json!({})),
        "shadow_score": shadow,
        "canary_model": canary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn soft_gpu_conflicts() {
        let f = json!({
            "webgl_unmasked_renderer": "NVIDIA GeForce RTX 4090",
            "residual_soft_like": true,
            "residual_mean": 0.5,
            "user_agent": "Mozilla/5.0 Chrome/120",
            "platform": "Linux x86_64",
        });
        let g = build_claim_obs_graph(&f);
        assert!(g["n_conflict"].as_u64().unwrap() >= 1);
        let sh = shadow_score_from_graph(&g, &f);
        assert_eq!(sh["drives_production_gate"], false);
        assert!(sh["shadow_risk"].as_f64().unwrap() > 0.2);
        let can = canary_model_compare(
            &sh,
            &json!({"rpa_bands": {"automation_evidence": 0.4}}),
            &json!({"risk": 0.2}),
        );
        assert_eq!(can["drives_production_gate"], false);
        assert_eq!(can["traffic_split_active"], false);
    }

    fn edge_of<'a>(g: &'a Value, claim: &str) -> &'a Value {
        edge_pair(g, claim, None::<&str>)
    }

    fn edge_pair<'a>(g: &'a Value, claim: &str, obs: Option<&str>) -> &'a Value {
        g["edges"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| {
                e.get("claim").and_then(|v| v.as_str()) == Some(claim)
                    && obs
                        .map(|o| e.get("observation").and_then(|v| v.as_str()) == Some(o))
                        .unwrap_or(true)
            })
            .unwrap_or_else(|| panic!("edge {claim}->{obs:?} missing: {g}"))
    }

    #[test]
    fn a5_gpu_label_vs_webgpu_family_conflict() {
        let f = json!({
            "webgl_unmasked_renderer": "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 Direct3D11)",
            "webgpu_adapter_device": "Apple M1",
            "user_agent": "Mozilla/5.0 Chrome/120",
            "platform": "Linux x86_64",
        });
        let g = build_claim_obs_graph(&f);
        let e = edge_pair(&g, "gpu_label", Some("webgpu_adapter"));
        assert_eq!(e["relation"], "conflict", "{e}");
        assert_eq!(e["severity"], "high", "{e}");
    }

    #[test]
    fn a5_gpu_family_agreement_corroborates() {
        let f = json!({
            "webgl_unmasked_renderer": "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 Direct3D11)",
            "webgpu_adapter_description": "NVIDIA GeForce RTX 3060",
            "user_agent": "Mozilla/5.0 Chrome/120",
            "platform": "Linux x86_64",
        });
        let g = build_claim_obs_graph(&f);
        assert_eq!(
            edge_pair(&g, "gpu_label", Some("webgpu_adapter"))["relation"],
            "corroborate"
        );
    }

    #[test]
    fn a5_gpu_label_vs_webgpu_unknown_is_claim_only() {
        let f = json!({
            "webgl_unmasked_renderer": "ANGLE (Some Obscure GPU)",
            "webgpu_adapter_device": "CartoonRenderer",
            "user_agent": "Mozilla/5.0 Chrome/120",
            "platform": "Linux x86_64",
        });
        let g = build_claim_obs_graph(&f);
        assert_eq!(
            edge_pair(&g, "gpu_label", Some("webgpu_adapter"))["relation"],
            "claim_only"
        );
    }

    #[test]
    fn a5_device_memory_claim_vs_allocation_conflict() {
        // Allocating 12 GB on a 4 GB device claim is impossible → conflict.
        let f = json!({
            "device_memory": 4,
            "mem_alloc_max_mb": 12288,
            "user_agent": "Mozilla/5.0 Chrome/120",
            "platform": "Linux x86_64",
        });
        let g = build_claim_obs_graph(&f);
        let e = edge_of(&g, "device_memory_claim");
        assert_eq!(e["observation"], "mem_alloc_max_mb", "{e}");
        assert_eq!(e["relation"], "conflict", "{e}");
        // Realistic under-report (alloc < claim) corroborates.
        let f2 = json!({
            "device_memory": 8,
            "mem_alloc_max_mb": 2048,
            "user_agent": "Mozilla/5.0 Chrome/120",
            "platform": "Linux x86_64",
        });
        let g2 = build_claim_obs_graph(&f2);
        assert_eq!(edge_of(&g2, "device_memory_claim")["relation"], "corroborate");
    }

    #[test]
    fn a5_storage_quota_bucket_recompute_consistency() {
        // 100 GiB must bucket to 32g_128g (matches FE gridCls).
        let f = json!({
            "storage_quota_bytes": 107374182400u64,
            "storage_quota_grid_class": "32g_128g",
            "user_agent": "Mozilla/5.0 Chrome/120",
            "platform": "Linux x86_64",
        });
        let g = build_claim_obs_graph(&f);
        let e = edge_of(&g, "storage_quota_claim");
        assert_eq!(e["relation"], "agree", "{e}");
        // Mismatched class → conflict (forgery / bucket drift).
        let f2 = json!({
            "storage_quota_bytes": 107374182400u64,
            "storage_quota_grid_class": "tb",
            "user_agent": "Mozilla/5.0 Chrome/120",
            "platform": "Linux x86_64",
        });
        let g2 = build_claim_obs_graph(&f2);
        let e2 = edge_of(&g2, "storage_quota_claim");
        assert_eq!(e2["relation"], "conflict", "{e2}");
        assert_eq!(e2["severity"], "high", "{e2}");
    }

    #[test]
    fn a5_ua_brands_whitelist_missing_chrome_brand_conflicts() {
        // Chrome-family UA but brands list omits Chrome entirely → anomalous.
        let f = json!({
            "user_agent": "Mozilla/5.0 (Windows NT 10.0) AppleWebKit/537.36 Chrome/120.0.0.0 Safari/537.36",
            "ua_brands": "Not A(Brand):99.0.0.0,Opera:101.0.0.0",
            "platform": "Windows",
        });
        let g = build_claim_obs_graph(&f);
        let e = edge_of(&g, "ua_brands");
        assert_eq!(e["observation"], "user_agent", "{e}");
        assert_eq!(e["relation"], "conflict", "{e}");
        // Normal Chrome brands agree.
        let f2 = json!({
            "user_agent": "Mozilla/5.0 (Windows NT 10.0) AppleWebKit/537.36 Chrome/120.0.0.0 Safari/537.36",
            "ua_brands": "Not A(Brand):99.0.0.0,Chromium:120.0.0.0,Google Chrome:120.0.0.0",
            "platform": "Windows",
        });
        let g2 = build_claim_obs_graph(&f2);
        assert_eq!(edge_of(&g2, "ua_brands")["relation"], "agree");
    }

    #[test]
    fn b93_fork_guess_vs_ua_family_edge() {
        // Firefox UA + floorp fork (CSS var surface) → compatible → agree.
        let f = json!({
            "user_agent": "Mozilla/5.0 (X11; Linux) Firefox/115.0",
            "blink_fork_guess": "floorp",
            "fork_claim_vs_obs": "agree",
            "platform": "Linux",
        });
        let g = build_claim_obs_graph(&f);
        let e = edge_of(&g, "blink_fork_guess");
        assert_eq!(e["relation"], "agree", "{e}");
        // Chrome UA but FE fork checker already reports a mismatch → conflict.
        let f2 = json!({
            "user_agent": "Mozilla/5.0 Chrome/120",
            "blink_fork_guess": "chrome",
            "fork_claim_vs_obs": "mismatch",
            "platform": "Linux",
        });
        let g2 = build_claim_obs_graph(&f2);
        let e2 = edge_of(&g2, "blink_fork_guess");
        assert_eq!(e2["relation"], "conflict", "{e2}");
        assert_eq!(e2["severity"], "med", "{e2}");
        // Unknown guess never claims conflict.
        let f3 = json!({
            "user_agent": "Mozilla/5.0 Chrome/120",
            "blink_fork_guess": "unknown",
            "platform": "Linux",
        });
        let g3 = build_claim_obs_graph(&f3);
        assert_eq!(edge_of(&g3, "blink_fork_guess")["relation"], "agree");
    }
}
