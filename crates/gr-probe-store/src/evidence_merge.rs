//! Shared batch merge for `build_evidence` (sqlite + postgres).

use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

/// Browser-side probe sources (base name before `:`).
pub fn is_browser_side_source(base_src: &str) -> bool {
    matches!(base_src, "main" | "iframe" | "worker" | "sandbox")
}

/// Sources that participate in cross-source conflict detection.
fn is_conflict_source(base_src: &str) -> bool {
    is_browser_side_source(base_src) || base_src == "gateway" || base_src == "cloudflare"
}

/// Page / surface-local RPA and nest metadata — expected to differ across main/iframe/worker.
/// These feed **page-scoped rpa** scoring only; they must NOT raise session xsrc conflicts
/// that demote os/br/device identity.
pub fn is_surface_local_key(key: &str) -> bool {
    matches!(
        key,
        // B11 continuous RPA (per surface stream)
        "behavior_events"
            | "behavior_count"
            | "behavior_early_bound"
            | "behavior_pagehide"
            | "pagehide_flush"
            | "rpa_flush_reason"
            | "rpa_idle_flush"
            | "rpa_source"
            | "worker_rpa"
            // page scope
            | "page_id"
            | "page_url"
            | "page_rev"
            // nest / multi-source tree labels (intentionally different per surface)
            | "sandbox_kind"
            | "nest_depth"
            | "sandbox_triggered"
            | "rpa_surface_ready"
            | "rpa_multi_source"
            // timestamps / mono flush markers always differ per upload
            | "collected_at"
            | "rpa_analyze"
    ) || key.starts_with("behavior_")
        || key.starts_with("rpa_")
}

/// Identity / mint-class keys: always prefer **main** in the merged session view.
/// Nest sources only fill gaps — never overwrite main (device_id authenticity).
pub fn is_identity_prefer_main_key(key: &str) -> bool {
    matches!(
        key,
        "user_agent"
            | "platform"
            | "os_family"
            | "language"
            | "languages"
            | "webdriver"
            | "hardware_concurrency"
            | "device_memory"
            | "max_touch_points"
            | "vendor"
            | "timezone"
            | "timezone_offset_min"
            | "form_class"
            | "screen_width"
            | "screen_height"
            | "architecture"
            | "hw_curve_audio"
            | "hw_curve_webgl"
            | "hw_curve_cpu"
            | "webgl_unmasked_renderer"
            | "webgl_unmasked_vendor"
            | "webrtc_host_ip_hash"
            | "os_instance_hash"
            | "font_count"
            | "fonts_present"
            | "plugins_len"
            | "mime_types_len"
            | "chrome_runtime"
            | "product_sub"
            | "app_version"
    ) || key.starts_with("hw_curve_")
        || key.starts_with("hw_audio_")
        || key.starts_with("hw_webgl_")
}

/// R00–R99 authenticity spot-check batch?
pub fn is_verify_spotcheck_batch(batch_id: &str) -> bool {
    let id = batch_id.trim();
    if id.is_empty() {
        return false;
    }
    // R00_spotcheck … R99_spotcheck
    if id.starts_with('R') && id.contains("spotcheck") {
        return true;
    }
    // payload may use dense_pack / verify_pack aliases
    false
}

fn f64_field(m: &Map<String, Value>, key: &str) -> Option<f64> {
    m.get(key).and_then(|v| {
        v.as_f64()
            .or_else(|| v.as_i64().map(|i| i as f64))
            .or_else(|| v.as_u64().map(|u| u as f64))
    })
}

/// Keys that scorer reads as session-level verify authenticity — aggregate across R packs.
const VERIFY_SCORE_KEYS: &[&str] = &[
    "verify_nonempty_ratio",
    "verify_exec_ratio",
    "verify_empty_n",
    "verify_nonempty_n",
    "verify_ok_n",
    "verify_probe_ops_n",
    "verify_exec_n",
    "verify_dim_fail_n",
];

/// Multi-tick multi-R explicit aggregate (replaces last-write of verify_* ratios).
/// Call after each R*_spotcheck batch is merged into session fields.
///
/// Accumulates ops/empty/ok/nonempty/dim_fail across packs, then rewrites scorer-facing
/// `verify_*` ratios from the sums. Keeps last pack's sample/forensics as `verify_last_*`.
pub fn apply_verify_spotcheck_aggregate(
    batch_id: &str,
    pf: &Map<String, Value>,
    fields: &mut Map<String, Value>,
) {
    if !is_verify_spotcheck_batch(batch_id) {
        // Also accept when payload marks verify_spotcheck without R id (belt).
        let marked = pf
            .get("verify_role")
            .and_then(|v| v.as_str())
            .is_some_and(|s| s.contains("authenticity"))
            || pf
                .get("verify_not_score_material")
                .and_then(|v| v.as_bool())
                == Some(true)
            || pf.get("verify_probe_ops_n").is_some();
        if !marked || !batch_id.contains("spotcheck") {
            return;
        }
    }

    let ops = f64_field(pf, "verify_probe_ops_n")
        .or_else(|| f64_field(pf, "verify_exec_n"))
        .unwrap_or(0.0);
    if ops <= 0.0 {
        return;
    }
    let empty = f64_field(pf, "verify_empty_n").unwrap_or(0.0);
    let nonempty = f64_field(pf, "verify_nonempty_n").unwrap_or_else(|| {
        // derive if missing
        (ops - empty).max(0.0)
    });
    let ok = f64_field(pf, "verify_ok_n").unwrap_or(nonempty);
    let dim_fail = f64_field(pf, "verify_dim_fail_n").unwrap_or(0.0);

    let pack_n = fields
        .get("verify_agg_pack_n")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        + 1;
    let sum_ops = fields
        .get("verify_agg_ops_n")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
        + ops;
    let sum_empty = fields
        .get("verify_agg_empty_n")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
        + empty;
    let sum_nonempty = fields
        .get("verify_agg_nonempty_n")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
        + nonempty;
    let sum_ok = fields
        .get("verify_agg_ok_n")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
        + ok;
    let max_dim_fail = fields
        .get("verify_agg_dim_fail_n_max")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
        .max(dim_fail);
    let sum_dim_fail = fields
        .get("verify_agg_dim_fail_n_sum")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
        + dim_fail;

    fields.insert("verify_agg_pack_n".into(), json!(pack_n));
    fields.insert("verify_agg_ops_n".into(), json!(sum_ops));
    fields.insert("verify_agg_empty_n".into(), json!(sum_empty));
    fields.insert("verify_agg_nonempty_n".into(), json!(sum_nonempty));
    fields.insert("verify_agg_ok_n".into(), json!(sum_ok));
    fields.insert("verify_agg_dim_fail_n_max".into(), json!(max_dim_fail));
    fields.insert("verify_agg_dim_fail_n_sum".into(), json!(sum_dim_fail));
    fields.insert("verify_aggregate_algo".into(), json!("gr_verify_agg_v1"));
    fields.insert("verify_last_pack".into(), json!(batch_id));

    // Track pack ids (cap list length)
    let mut ids: Vec<String> = fields
        .get("verify_agg_pack_ids")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    if !ids.iter().any(|x| x == batch_id) {
        ids.push(batch_id.to_string());
        if ids.len() > 32 {
            let skip = ids.len() - 32;
            ids = ids.into_iter().skip(skip).collect();
        }
    }
    fields.insert("verify_agg_pack_ids".into(), json!(ids));

    // Scorer-facing ratios from multi-pack sums (not last-write alone).
    if sum_ops > 0.0 {
        fields.insert(
            "verify_nonempty_ratio".into(),
            json!(sum_nonempty / sum_ops),
        );
        fields.insert("verify_exec_ratio".into(), json!(sum_ok / sum_ops));
        fields.insert("verify_empty_n".into(), json!(sum_empty));
        fields.insert("verify_nonempty_n".into(), json!(sum_nonempty));
        fields.insert("verify_ok_n".into(), json!(sum_ok));
        fields.insert("verify_probe_ops_n".into(), json!(sum_ops));
        fields.insert("verify_exec_n".into(), json!(sum_ops));
        // Use max dim_fail across packs (worst dimension failure signal)
        fields.insert("verify_dim_fail_n".into(), json!(max_dim_fail));
    }
    // Preserve role markers
    if pf.get("verify_not_score_material").is_some() {
        fields.insert("verify_not_score_material".into(), json!(true));
    }
    if let Some(r) = pf.get("verify_role").cloned() {
        fields.insert("verify_role".into(), r);
    }
    if let Some(a) = pf.get("verify_algo").cloned() {
        fields.insert("verify_algo".into(), a);
    }
}

/// B10 commercial primary path_ids (v3f / fma_pair). B10x deepen must not last-write these.
fn is_b10_primary_residual_path_id(path_id: &str) -> bool {
    let p = path_id.to_ascii_lowercase();
    if p.contains("ulp") || p.contains("noderiv_hard") || p.contains("rint_") {
        return false;
    }
    if p.contains("b10x") || p.contains("legacy") || p.contains("softgl") {
        return false;
    }
    p.contains("v3f") || p.contains("fma_pair")
}

fn is_b10x_batch_id(batch_id: Option<&str>) -> bool {
    batch_id
        .map(|b| b.starts_with("B10x_") || b.starts_with("b10x_"))
        .unwrap_or(false)
}

fn residual_path_ok(v: &Value) -> bool {
    v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false)
        && v.get("curve")
            .and_then(|c| c.as_array())
            .map(|a| a.len() >= 8)
            .unwrap_or(false)
}

/// Union `residual_paths` arrays by `path_id` (later batches fill/overwrite same id).
/// Enables B10 primary multipath + B10x deepen packs to coexist without last-write wipe.
/// B10x must not replace a healthy B10 v3f/fma path — that last-write is how Firefox
/// shop (B10 only) and news (B10+B10x) forked commercial wg on the same GPU.
#[allow(dead_code)] // kept: algorithm reference / .so-module variant (iss/audit WARN-01: silence, do not remove)
fn merge_residual_paths_field(fields: &mut Map<String, Value>, incoming: &Value) {
    merge_residual_paths_field_ex(fields, incoming, None);
}

fn merge_residual_paths_field_ex(
    fields: &mut Map<String, Value>,
    incoming: &Value,
    batch_id: Option<&str>,
) {
    let Some(inc) = incoming.as_array() else {
        return;
    };
    let mut by_id: HashMap<String, Value> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    if let Some(exist) = fields.get("residual_paths").and_then(|v| v.as_array()) {
        for e in exist {
            let pid = e
                .get("path_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let key = if pid.is_empty() {
                format!("anon_{}", by_id.len())
            } else {
                pid
            };
            if !by_id.contains_key(&key) {
                order.push(key.clone());
            }
            by_id.insert(key, e.clone());
        }
    }
    for e in inc {
        let pid = e
            .get("path_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let key = if pid.is_empty() {
            format!("anon_{}", by_id.len())
        } else {
            pid
        };
        if !by_id.contains_key(&key) {
            order.push(key.clone());
        }
        // Prefer higher-quality path: entropy_ok > ok > longer curve > incoming deepen.
        let score = |v: &Value| -> i64 {
            let curve_n = v
                .get("curve")
                .and_then(|c| c.as_array())
                .map(|a| a.len())
                .unwrap_or(0) as i64;
            let ent = v.get("entropy_ok").and_then(|x| x.as_bool()).unwrap_or(false);
            let ok = v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false);
            let timing = v
                .get("eu_timing_ms")
                .and_then(|t| t.as_array())
                .map(|a| a.len())
                .unwrap_or(0) as i64;
            (if ent { 1000 } else { 0 })
                + (if ok { 100 } else { 0 })
                + curve_n
                + timing
        };
        let existing = by_id.get(&key);
        let incoming_b10x = is_b10x_batch_id(batch_id);
        let primary = is_b10_primary_residual_path_id(&key);
        if incoming_b10x && primary {
            if existing.is_some_and(residual_path_ok) {
                continue;
            }
        }
        let prefer_new = existing.is_none() || score(e) >= score(existing.unwrap());
        if prefer_new {
            by_id.insert(key, e.clone());
        }
    }
    let merged: Vec<Value> = order
        .into_iter()
        .filter_map(|k| by_id.remove(&k))
        .collect();
    fields.insert("residual_paths".into(), json!(merged));
    fields.insert(
        "residual_paths_n".into(),
        json!(fields
            .get("residual_paths")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0)),
    );
}

/// Merge one batch payload into merged `fields`, per-source views, and gateway/cf slices.
/// When `batch_id` is R*spotcheck, score-facing `verify_*` ratios are multi-pack aggregated.
#[allow(dead_code)] // kept: algorithm reference / .so-module variant (iss/audit WARN-01: silence, do not remove)
pub fn merge_batch(
    base_src: &str,
    pf: Map<String, Value>,
    fields: &mut Map<String, Value>,
    fields_by_source: &mut Map<String, Value>,
    gateway_fields: &mut Map<String, Value>,
    cf_fields: &mut Map<String, Value>,
) {
    merge_batch_with_id(base_src, None, pf, fields, fields_by_source, gateway_fields, cf_fields);
}

/// Same as [`merge_batch`] but with batch_id for verify aggregation.
pub fn merge_batch_with_id(
    base_src: &str,
    batch_id: Option<&str>,
    pf: Map<String, Value>,
    fields: &mut Map<String, Value>,
    fields_by_source: &mut Map<String, Value>,
    gateway_fields: &mut Map<String, Value>,
    cf_fields: &mut Map<String, Value>,
) {
    if !fields_by_source.contains_key(base_src) {
        fields_by_source.insert(base_src.into(), json!({}));
    }
    let bucket = fields_by_source
        .get_mut(base_src)
        .and_then(|v| v.as_object_mut())
        .expect("fields_by_source bucket");
    for (k, v) in &pf {
        bucket.insert(k.clone(), v.clone());
    }

    let is_r = batch_id.is_some_and(is_verify_spotcheck_batch)
        || pf
            .get("verify_not_score_material")
            .and_then(|v| v.as_bool())
            == Some(true);

    // Capture the FE bundle version BEFORE the merge loop overwrites it, so
    // mid-session drift detection (A5) can compare across batches.
    let fe_version_before = if base_src == "main" {
        fields
            .get("fe_code_version")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    } else {
        None
    };

    if base_src == "gateway" {
        for (k, v) in &pf {
            gateway_fields.insert(k.clone(), v.clone());
        }
        for (k, v) in &pf {
            fields.entry(k.clone()).or_insert_with(|| v.clone());
        }
        if let Some(nested) = pf.get("cf_fields").and_then(|v| v.as_object()) {
            for (k, v) in nested {
                cf_fields.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
        for key in [
            "cf_ray",
            "cf_connecting_ip",
            "cf_ipcountry",
            "cf_edge_present",
            "cf_asn",
            "country",
            "bot_score",
        ] {
            if let Some(v) = pf.get(key) {
                cf_fields.entry(key.to_string()).or_insert_with(|| v.clone());
            }
        }
    } else if base_src == "cloudflare" {
        for (k, v) in &pf {
            cf_fields.insert(k.clone(), v.clone());
        }
        for (k, v) in &pf {
            fields.entry(k.clone()).or_insert_with(|| v.clone());
        }
    } else {
        for (k, v) in pf.iter() {
            // R score keys: skip last-write; aggregation rewrites them after loop.
            if is_r && VERIFY_SCORE_KEYS.iter().any(|s| *s == k.as_str()) {
                continue;
            }
            let k = k.clone();
            let v = v.clone();
            // residual_paths: union by path_id across B10 + B10x packs (coexistence).
            if k == "residual_paths" {
                merge_residual_paths_field_ex(fields, &v, batch_id);
                continue;
            }
            // Prefer main's RPA stream for merged session fields used by page block;
            // non-main surface-local keys stay in fields_by_source only unless missing.
            if is_surface_local_key(&k) && base_src != "main" {
                fields.entry(k).or_insert(v);
            } else if is_identity_prefer_main_key(&k) {
                // Product: device_id / OS / BR claim materials trust main first.
                if base_src == "main" {
                    fields.insert(k, v);
                } else {
                    fields.entry(k).or_insert(v);
                }
            } else if base_src == "main" {
                fields.insert(k, v);
            } else {
                // Non-identity nest fields: fill gaps only (avoid silent overwrite)
                fields.entry(k).or_insert(v);
            }
        }
    }

    if let Some(bid) = batch_id {
        apply_verify_spotcheck_aggregate(bid, &pf, fields);
    } else if is_r {
        // batch_id unknown but payload is verify — use pack_id if present
        let bid = pf
            .get("verify_pack")
            .or_else(|| pf.get("pack_id"))
            .or_else(|| pf.get("dense_pack"))
            .and_then(|v| v.as_str())
            .unwrap_or("Rxx_spotcheck");
        apply_verify_spotcheck_aggregate(bid, &pf, fields);
    }

    // ── Channel integrity (A2): FE B8 convergence claim vs server-side edge ──
    // FE may claim post_converge/b8_enrich on its B8 ingest, but the only
    // trustworthy edge evidence lives in gateway_fields (written by the
    // gateway/early handler from request headers). A "converged" claim with
    // zero edge observation is itself an inconsistency signal for br/os.
    if batch_id.is_some_and(|b| b.starts_with("B8_gateway") || b == "gateway.b8") {
        let fe_converged = pf
            .get("post_converge")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || pf
                .get("b8_enrich")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            || pf
                .get("b8_bases")
                .and_then(|v| v.as_array())
                .is_some_and(|a| a.len() >= 2);
        let edge_seen = !gateway_fields.is_empty()
            || fields.contains_key("gateway_user_agent")
            || fields.contains_key("server_client_ip")
            || fields.contains_key("http_header_order")
            || fields.contains_key("cf_edge_present")
            || fields.contains_key("tls_ja4")
            || fields.contains_key("ja4");
        fields.insert("b8_edge_seen".into(), json!(edge_seen));
        fields.insert(
            "b8_converge_without_edge".into(),
            json!(fe_converged && !edge_seen),
        );
    }

    // ── FE build/version integrity (A5): mid-session drift is suspicious ──
    // freezeCapture stamps fe_code_version / fe_build_impl / fe_impl_version on
    // every batch. A version change across batches of one session (or a build
    // stamp that disagrees with the FE epoch) means the page was modified,
    // partially loaded, or mixed bundles — br/os contradict_weak, never digest.
    if base_src == "main" {
        let fv = pf
            .get("fe_code_version")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());
        if let Some(fv) = fv {
            match &fe_version_before {
                Some(prev) if prev != fv => {
                    fields.insert("fe_version_changed".into(), json!(true));
                }
                None => {
                    fields.insert("fe_code_version".into(), json!(fv));
                }
                _ => {}
            }
        }
        let bi = pf.get("fe_build_impl").and_then(|v| v.as_str());
        let ev = pf.get("fe_impl_version").and_then(|v| v.as_str());
        if let (Some(b), Some(e)) = (bi, ev) {
            if !b.is_empty() && !e.is_empty() && b != e {
                fields.insert("fe_build_mismatch".into(), json!(true));
            }
        }
    }

    // ── Sandbox source consistency (A3): nest residual algo vs main algo ──
    // main sends residual_algo; sandbox/iframe sends nest_residual_algo. Equal
    // algorithm families → the sandbox observed the same residual channel
    // (mutual confirmation). Divergence → the sandbox pseudo-measurement must
    // not strengthen the residual class (soft demotion hook for soft_v2).
    if let Some(nest_algo) = pf
        .get("nest_residual_algo")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        if base_src != "main" {
            // Only a *main* residual_algo counts as the reference; the sandbox
            // batch may itself have inserted nest_residual_algo into fields,
            // so never fall back to it.
            let main_algo = fields
                .get("residual_algo")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if main_algo.is_empty() {
                fields.remove("sandbox_residual_algo_match");
            } else {
                fields.insert(
                    "sandbox_residual_algo_match".into(),
                    json!(main_algo == nest_algo),
                );
            }
        }
    }
}

/// Normalize value for soft multi-source compare (class-level, not strict equality).
/// Platform/version/surface strings may differ across browsers and still be “same host”.
pub fn soft_normalize_for_compare(key: &str, v: &Value) -> String {
    match key {
        "platform" | "ua_platform" => {
            let s = v.as_str().unwrap_or("").to_ascii_lowercase();
            if s.contains("win") {
                "win".into()
            } else if s.contains("mac") || s.contains("iphone") || s.contains("ipad") {
                "mac".into()
            } else if s.contains("android") {
                "android".into()
            } else if s.contains("linux") || s.contains("x11") {
                "linux".into()
            } else if s.is_empty() {
                "".into()
            } else {
                "other".into()
            }
        }
        "os_family" => v
            .as_str()
            .unwrap_or("")
            .to_ascii_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .next()
            .unwrap_or("")
            .to_string(),
        "hardware_concurrency" | "device_memory" => {
            // Coarse class: log2-ish buckets (worker under-report common)
            let n = v
                .as_f64()
                .or_else(|| v.as_i64().map(|i| i as f64))
                .or_else(|| v.as_u64().map(|u| u as f64))
                .unwrap_or(0.0);
            if n <= 0.0 {
                "c0".into()
            } else if n <= 2.0 {
                "c1_2".into()
            } else if n <= 4.0 {
                "c3_4".into()
            } else if n <= 8.0 {
                "c5_8".into()
            } else if n <= 16.0 {
                "c9_16".into()
            } else {
                "c17p".into()
            }
        }
        "timezone" | "language" | "vendor" => v
            .as_str()
            .unwrap_or("")
            .to_ascii_lowercase()
            .chars()
            .take(48)
            .collect(),
        "timezone_offset_min" => v
            .as_i64()
            .or_else(|| v.as_f64().map(|f| f as i64))
            .map(|i| i.to_string())
            .unwrap_or_default(),
        "screen_width" | "screen_height" => {
            // 320-grid classes: benign per-surface rounding must not conflict.
            let n = v
                .as_f64()
                .or_else(|| v.as_i64().map(|i| i as f64))
                .or_else(|| v.as_u64().map(|u| u as f64))
                .unwrap_or(0.0) as i64;
            format!("d{}", ((n + 159) / 320).max(0))
        }
        "max_touch_points" => {
            let n = v
                .as_f64()
                .or_else(|| v.as_i64().map(|i| i as f64))
                .or_else(|| v.as_u64().map(|u| u as f64))
                .unwrap_or(0.0);
            let c = if n <= 0.0 {
                0
            } else if n <= 1.0 {
                1
            } else if n <= 4.0 {
                2
            } else if n <= 10.0 {
                3
            } else {
                4
            };
            format!("t{c}")
        }
        "webgl_max_texture" | "webgl_max_viewport" | "webgl_max_tex_units" => {
            // log2-ish class (same tolerance family as hardware_concurrency)
            let n = v
                .as_f64()
                .or_else(|| v.as_i64().map(|i| i as f64))
                .or_else(|| v.as_u64().map(|u| u as f64))
                .unwrap_or(0.0);
            if n <= 0.0 {
                "c0".into()
            } else if n <= 512.0 {
                "c1_512".into()
            } else if n <= 4096.0 {
                "c513_4k".into()
            } else if n <= 16384.0 {
                "c4k_16k".into()
            } else {
                "c16kp".into()
            }
        }
        "device_model" => v
            .as_str()
            .unwrap_or("")
            .to_ascii_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .take(3)
            .collect::<Vec<_>>()
            .join(" "),
        "webgl_precision_high" | "webgl_depth_bits" => match v.as_bool() {
            Some(b) => { if b { "1".into() } else { "0".into() } }
            None => v
                .as_f64()
                .or_else(|| v.as_i64().map(|i| i as f64))
                .map(|f| format!("{f:.0}"))
                .unwrap_or_default(),
        },
        "user_agent" => {
            // Family only — full UA strings differ by version/surface
            let s = v.as_str().unwrap_or("").to_ascii_lowercase();
            if s.contains("edg/") {
                "edge".into()
            } else if s.contains("chrome/") || s.contains("crios/") {
                "chrome".into()
            } else if s.contains("firefox/") || s.contains("fxios/") {
                "firefox".into()
            } else if s.contains("safari/") && !s.contains("chrome") {
                "safari".into()
            } else if s.is_empty() {
                "".into()
            } else {
                "other".into()
            }
        }
        "webdriver" => match v.as_bool() {
            Some(true) => "1".into(),
            Some(false) => "0".into(),
            None => "".into(),
        },
        _ => {
            if let Some(s) = v.as_str() {
                s.to_ascii_lowercase().chars().take(64).collect()
            } else if let Some(b) = v.as_bool() {
                if b { "1".into() } else { "0".into() }
            } else if let Some(n) = v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)) {
                format!("{n:.4}")
            } else {
                v.to_string()
            }
        }
    }
}

/// Identity-class keys: soft mismatch still raises conflict (host integrity).
/// A3 extended set: adds numeric host / viewport / webgl-cap keys
/// (identity set 13 → 19) — all normalized into coarse classes so benign
/// per-source under-reporting does not false-conflict.
fn is_soft_identity_key(key: &str) -> bool {
    matches!(
        key,
        "platform"
            | "os_family"
            | "hardware_concurrency"
            | "timezone"
            | "timezone_offset_min"
            | "webdriver"
            | "user_agent"
            | "language"
            | "vendor"
            | "device_memory"
            // A3 additions (numeric identity material)
            | "device_model"
            | "screen_width"
            | "screen_height"
            | "max_touch_points"
            | "webgl_max_texture"
            | "webgl_max_viewport"
            | "webgl_max_tex_units"
            | "webgl_precision_high"
            | "webgl_depth_bits"
    )
}

/// Keys that require exact (or near-exact) match when present on ≥2 sources.
fn is_hard_equi_key(key: &str) -> bool {
    matches!(
        key,
        "webgl_unmasked_renderer" | "webgl_unmasked_vendor" | "form_class"
    )
}

/// Keys appearing in ≥2 conflict-participating sources with **identity-class** mismatch.
/// Uses soft normalize for platform/cores/UA family — not strict string equality.
/// Excludes surface-local RPA/page keys (expected multi-source variance).
pub fn detect_source_conflicts(fields_by_source: &Map<String, Value>) -> Vec<String> {
    let mut key_sources: HashMap<String, Vec<(String, Value)>> = HashMap::new();
    for (src, bucket) in fields_by_source {
        if !is_conflict_source(src) {
            continue;
        }
        let Some(obj) = bucket.as_object() else {
            continue;
        };
        for (k, v) in obj {
            if is_surface_local_key(k) {
                continue;
            }
            // Only score identity-relevant keys for session conflict
            if !is_soft_identity_key(k) && !is_hard_equi_key(k) {
                continue;
            }
            if v.is_null() {
                continue;
            }
            if let Some(s) = v.as_str() {
                if s.is_empty() {
                    continue;
                }
            }
            key_sources
                .entry(k.clone())
                .or_default()
                .push((src.clone(), v.clone()));
        }
    }

    let mut conflicts = Vec::new();
    for (key, entries) in key_sources {
        if entries.len() < 2 {
            continue;
        }
        if is_hard_equi_key(&key) {
            let first = &entries[0].1;
            if entries.iter().any(|(_, v)| v != first) {
                conflicts.push(format!("source_conflict:{key}"));
            }
            continue;
        }
        // Soft identity class compare
        let classes: Vec<String> = entries
            .iter()
            .map(|(_, v)| soft_normalize_for_compare(&key, v))
            .filter(|s| !s.is_empty())
            .collect();
        if classes.len() < 2 {
            continue;
        }
        let first = &classes[0];
        if classes.iter().any(|c| c != first) {
            // Special case: hardware_concurrency worker under-report adjacent classes ok
            if key == "hardware_concurrency" || key == "device_memory" {
                let ranks: Vec<i32> = classes
                    .iter()
                    .map(|c| match c.as_str() {
                        "c0" => 0,
                        "c1_2" => 1,
                        "c3_4" => 2,
                        "c5_8" => 3,
                        "c9_16" => 4,
                        "c17p" => 5,
                        _ => -1,
                    })
                    .filter(|r| *r >= 0)
                    .collect();
                if ranks.len() >= 2 {
                    let min = *ranks.iter().min().unwrap();
                    let max = *ranks.iter().max().unwrap();
                    // allow one-bucket drift (worker often under-reports)
                    if max - min <= 1 {
                        continue;
                    }
                }
            }
            conflicts.push(format!("source_conflict:{key}"));
        }
    }
    conflicts.sort();
    conflicts
}

/// Expected thin identity key set for a healthy nest payload (main vs nest compare).
const NEST_IDENTITY_KEYS: &[&str] = &[
    "user_agent",
    "platform",
    "os_family",
    "hardware_concurrency",
    "language",
    "webdriver",
    "timezone",
];

// ── A3 cross-source numeric consistency ─────────────────────────────────
// Same-material numeric comparison across main ∥ sandbox ∥ worker with
// 7-sample-point quantization; each slot gets an `xsrc_numeric_delta_{slot}`
// verdict in {consistent, minor, divergent, missing}. Bucket thresholds are
// the strategy §4-A3 lab quantiles; kept as constants until lab data lands.

/// Slots compared numerically across sources (curve materials + residual).
pub const XSRC_NUMERIC_SLOTS: [&str; 5] = [
    "hw_curve_webgl",
    "hw_curve_audio",
    "hw_curve_cpu",
    "audio_deep_curve",
    "residual_mean",
];

/// Number of sample points for curve quantization (7 per strategy §4-A3).
pub const XSRC_NUMERIC_POINTS: usize = 7;

/// Quantization scale: deltas are computed on values rounded to 3 decimals.
const XSRC_QUANT_DP: f64 = 1000.0;

/// Relative delta classes: consistent ≤ 0.02, minor ≤ 0.15, divergent > 0.15.
const XSRC_CONSISTENT_MAX: f64 = 0.02;
const XSRC_MINOR_MAX: f64 = 0.15;

/// Read a numeric series from a field value (flat number array, ≥4 points).
pub fn numeric_series(v: &Value) -> Option<Vec<f64>> {
    let arr = v.as_array()?;
    let mut out = Vec::with_capacity(arr.len());
    for x in arr {
        if let Some(n) = x.as_f64() {
            if n.is_finite() {
                out.push(n);
                continue;
            }
        }
        return None; // mixed / non-numeric curve → not usable (missing)
    }
    if out.len() < 4 {
        return None;
    }
    Some(out)
}

/// Numeric material for xsrc compare: curves (≥4 points) or scalars —
/// residues like `residual_mean` are single values, compared as-is.
fn numeric_material(v: &Value) -> Option<Vec<f64>> {
    match numeric_series(v) {
        Some(s) => Some(s),
        None => v
            .as_f64()
            .filter(|n| n.is_finite())
            .map(|n| vec![n]),
    }
}

/// Evenly sample `points` quantized values across the series (round to 3 dp).
/// Fewer than `points` → resample at equal strides regardless.
pub fn sample_quantized(series: &[f64], points: usize) -> Vec<f64> {
    if series.is_empty() || points == 0 {
        return vec![];
    }
    let n = series.len();
    if n <= points {
        return series
            .iter()
            .map(|x| (x * XSRC_QUANT_DP).round() / XSRC_QUANT_DP)
            .collect();
    }
    (0..points)
        .map(|i| {
            let idx = ((n - 1) as f64 * i as f64 / (points - 1) as f64).round() as usize;
            (series[idx] * XSRC_QUANT_DP).round() / XSRC_QUANT_DP
        })
        .collect()
}

/// Classify the numeric delta between two same-material series:
/// returns (class, max relative pointwise delta).
pub fn classify_numeric_delta(a: &[f64], b: &[f64]) -> (&'static str, f64) {
    let m = a.len().min(b.len());
    if m == 0 {
        return ("missing", 0.0);
    }
    let mut max_rel = 0.0f64;
    for i in 0..m {
        let abs = (a[i] - b[i]).abs();
        let rel = abs / (a[i].abs() + 1e-6);
        max_rel = max_rel.max(rel);
    }
    let class = if max_rel <= XSRC_CONSISTENT_MAX {
        "consistent"
    } else if max_rel <= XSRC_MINOR_MAX {
        "minor"
    } else {
        "divergent"
    };
    (class, max_rel)
}

/// Single-slot comparison across browser sources (main is the reference).
/// Emits `xsrc_numeric_delta_{slot}` with per-source classes and an
/// aggregate verdict.
pub fn xsrc_numeric_compare(fields_by_source: &Map<String, Value>, slot: &str) -> Value {
    let browser_srcs: Vec<(&String, &Value)> = fields_by_source
        .iter()
        .filter(|(s, _)| is_browser_side_source(s.split(':').next().unwrap_or(s)))
        .collect();
    let main = match browser_srcs
        .iter()
        .find(|(s, _)| s.as_str() == "main")
        .map(|(_, v)| v)
        .and_then(|v| v.as_object())
    {
        Some(m) => m,
        None => {
            return json!({
                "slot": slot,
                "verdict": "missing",
                "reason": "no_main_reference",
                "sources": [],
            })
        }
    };
    let Some(main_v) = main.get(slot) else {
        return json!({
            "slot": slot,
            "verdict": "missing",
            "reason": "main_missing_field",
            "sources": [],
        });
    };
    let main_material = match numeric_material(main_v) {
        Some(m) => m,
        None => {
            return json!({
                "slot": slot,
                "verdict": "missing",
                "reason": "main_not_numeric",
                "sources": [],
            })
        }
    };
    let main_s = sample_quantized(&main_material, XSRC_NUMERIC_POINTS);
    let mut sources = Vec::new();
    let mut worst_class = "consistent".to_string();
    let mut worst_delta = 0.0f64;
    let mut compared = 0usize;
    let mut missing_n = 0usize;
    for (src, bucket) in &browser_srcs {
        if src.as_str() == "main" {
            continue;
        }
        let Some(bucket) = bucket.as_object() else {
            continue;
        };
        let entry = match bucket.get(slot) {
            Some(v) => match numeric_material(v) {
                Some(s) => {
                    let comp = sample_quantized(&s, XSRC_NUMERIC_POINTS);
                    let (class, delta) = classify_numeric_delta(&main_s, &comp);
                    compared += 1;
                    json!({
                        "source": src,
                        "class": class,
                        "max_rel_delta": (delta * 1e6).round() / 1e6,
                        "n_points": comp.len(),
                    })
                }
                None => {
                    missing_n += 1;
                    json!({"source": src, "class": "missing", "reason": "not_numeric"})
                }
            },
            None => {
                missing_n += 1;
                json!({"source": src, "class": "missing", "reason": "field_absent"})
            }
        };
        let class = entry
            .get("class")
            .and_then(|v| v.as_str())
            .unwrap_or("missing")
            .to_string();
        let rank = match class.as_str() {
            "divergent" => 3,
            "minor" => 2,
            "consistent" => 1,
            _ => 0,
        };
        let cur_rank = match worst_class.as_str() {
            "divergent" => 3,
            "minor" => 2,
            "consistent" => 1,
            _ => 0,
        };
        if rank > cur_rank {
            worst_class = class;
        }
        if let Some(d) = entry.get("max_rel_delta").and_then(|v| v.as_f64()) {
            worst_delta = worst_delta.max(d);
        }
        sources.push(entry);
    }
    let verdict = if compared == 0 {
        "missing".to_string()
    } else {
        worst_class
    };
    json!({
        "slot": slot,
        "verdict": verdict,
        "max_rel_delta": (worst_delta * 1e6).round() / 1e6,
        "sources_compared": compared,
        "sources_missing": missing_n,
        "points": XSRC_NUMERIC_POINTS,
        "sources": sources,
    })
}

/// A3 aggregated view: each `xsrc_numeric_delta_{slot}` + overall verdict
/// (worst class across slots; "missing" when nothing comparable).
pub fn xsrc_numeric_summary(fields_by_source: &Map<String, Value>) -> Value {
    let mut deltas = Map::new();
    let mut worst = "consistent".to_string();
    let mut any_main_material = false;
    for slot in XSRC_NUMERIC_SLOTS {
        let d = xsrc_numeric_compare(fields_by_source, slot);
        let verdict = d
            .get("verdict")
            .and_then(|v| v.as_str())
            .unwrap_or("missing")
            .to_string();
        if verdict != "missing" {
            any_main_material = true;
        }
        let rank = match verdict.as_str() {
            "divergent" => 3,
            "minor" => 2,
            "consistent" => 1,
            _ => 0,
        };
        let cur_rank = match worst.as_str() {
            "divergent" => 3,
            "minor" => 2,
            "consistent" => 1,
            _ => 0,
        };
        if rank > cur_rank {
            worst = verdict;
        }
        deltas.insert(format!("xsrc_numeric_delta_{slot}"), d);
    }
    json!({
        "algo": "gr_xsrc_numeric_v1",
        "xsrc_numeric_verdict": if any_main_material { worst } else { "missing".to_string() },
        "slots": deltas,
    })
}

fn nest_payload_richness(bucket: &Map<String, Value>) -> usize {
    NEST_IDENTITY_KEYS
        .iter()
        .filter(|k| {
            bucket.get(**k).is_some_and(|v| match v {
                Value::Null => false,
                Value::String(s) => !s.is_empty(),
                _ => true,
            })
        })
        .count()
}

/// Multi-source consistency diagnostic for analyze (match ratio, sandbox capability).
///
/// Product: normal browsers execute ≥1 nest with identity payload; fake browsers often
/// yield empty/thin/conflict nests — capability score feeds OS/BR/RPA demotion.
pub fn assess_multi_source_consistency(fields_by_source: &Map<String, Value>) -> Value {
    let browser_srcs: Vec<(&String, &Value)> = fields_by_source
        .iter()
        .filter(|(s, _)| is_browser_side_source(s.split(':').next().unwrap_or(s)))
        .collect();
    let n_src = browser_srcs.len();
    let mut compared = 0u32;
    let mut matched = 0u32;
    let mut soft_mismatches = Vec::new();
    let keys = [
        "platform",
        "os_family",
        "hardware_concurrency",
        "timezone",
        "timezone_offset_min",
        "webdriver",
        "language",
        "user_agent",
        // A3 extended identity material (numeric keys, class-normalized)
        "device_memory",
        "device_model",
        "screen_width",
        "screen_height",
        "max_touch_points",
        "webgl_max_texture",
        "webgl_max_viewport",
    ];
    for key in keys {
        let mut classes = Vec::new();
        for (src, bucket) in &browser_srcs {
            if let Some(v) = bucket.get(key) {
                if v.is_null() {
                    continue;
                }
                let c = soft_normalize_for_compare(key, v);
                if !c.is_empty() {
                    classes.push(((*src).clone(), c));
                }
            }
        }
        if classes.len() < 2 {
            continue;
        }
        compared += 1;
        let first = &classes[0].1;
        if classes.iter().all(|(_, c)| c == first) {
            matched += 1;
        } else if key == "hardware_concurrency" || key == "device_memory" {
            // Same one-bucket drift allowance as detect_source_conflicts
            let ranks: Vec<i32> = classes
                .iter()
                .map(|(_, c)| match c.as_str() {
                    "c0" => 0,
                    "c1_2" => 1,
                    "c3_4" => 2,
                    "c5_8" => 3,
                    "c9_16" => 4,
                    "c17p" => 5,
                    _ => -1,
                })
                .filter(|r| *r >= 0)
                .collect();
            if ranks.len() >= 2 {
                let min = *ranks.iter().min().unwrap();
                let max = *ranks.iter().max().unwrap();
                if max - min <= 1 {
                    matched += 1;
                } else {
                    soft_mismatches.push(key.to_string());
                }
            } else {
                soft_mismatches.push(key.to_string());
            }
        } else {
            soft_mismatches.push(key.to_string());
        }
    }
    let match_ratio = if compared == 0 {
        1.0
    } else {
        matched as f64 / compared as f64
    };

    // Sandbox health from main summary if present
    let main = fields_by_source.get("main").and_then(|v| v.as_object());
    let sandbox_blocked = main
        .and_then(|m| m.get("sandbox_blocked"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let sandbox_partial = main
        .and_then(|m| m.get("sandbox_partial"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let kinds_planned = main
        .and_then(|m| {
            m.get("sandbox_kinds_planned")
                .or_else(|| m.get("kinds"))
                .and_then(|v| v.as_array())
                .map(|a| a.len())
        })
        .unwrap_or(0);
    let sources_received = main
        .and_then(|m| {
            m.get("sandbox_sources_received")
                .or_else(|| m.get("sources_received"))
        })
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    // Nest sources with real identity payload (exclude main-only)
    let mut nest_payload_n = 0u32;
    let mut nest_rich_max = 0usize;
    let mut nest_kinds_with_data: Vec<String> = Vec::new();
    for (src, bucket) in fields_by_source {
        let base = src.split(':').next().unwrap_or(src.as_str());
        if base == "main" || !is_browser_side_source(base) {
            continue;
        }
        let Some(obj) = bucket.as_object() else {
            continue;
        };
        let rich = nest_payload_richness(obj);
        if rich >= 2 {
            nest_payload_n += 1;
            nest_rich_max = nest_rich_max.max(rich);
            if !nest_kinds_with_data.iter().any(|s| s == base) {
                nest_kinds_with_data.push(base.to_string());
            }
        }
    }
    // Also count sources_received from summary when nest buckets not yet split
    let payload_n = nest_payload_n.max(sources_received as u32);

    let main_rich = main.map(nest_payload_richness).unwrap_or(0);
    let thin_vs_main = payload_n > 0
        && main_rich >= 5
        && nest_rich_max > 0
        && nest_rich_max + 2 < main_rich.min(NEST_IDENTITY_KEYS.len());

    let sandbox_all_empty = sandbox_blocked || (payload_n == 0 && (kinds_planned >= 2 || n_src >= 1));
    let sandbox_under_two_kinds = nest_kinds_with_data.len() < 2 && payload_n < 2;
    // True OK only when ≥1 nest has data and not fully blocked
    let sandbox_ok = !sandbox_blocked && payload_n >= 1;
    let js_ok_sandbox_dead = sandbox_blocked || (payload_n == 0 && n_src >= 1);

    // Capability score ∈ [0,1] for OS/BR/RPA demotion
    let capability_score = if js_ok_sandbox_dead || sandbox_all_empty {
        0.0
    } else if payload_n == 0 {
        0.05
    } else if thin_vs_main && match_ratio < 0.5 {
        0.25
    } else if thin_vs_main {
        0.35
    } else if payload_n == 1 || sandbox_under_two_kinds {
        if match_ratio >= 0.85 {
            0.55
        } else if match_ratio < 0.5 {
            0.35
        } else {
            0.45
        }
    } else if match_ratio >= 0.85 && payload_n >= 2 {
        0.9
    } else if match_ratio >= 0.6 {
        0.7
    } else if match_ratio < 0.5 {
        0.3
    } else {
        0.6
    };

    let capability_band = if capability_score <= 0.05 {
        "dead"
    } else if capability_score < 0.35 {
        "empty_or_conflict"
    } else if capability_score < 0.55 {
        "thin"
    } else if capability_score < 0.8 {
        "partial"
    } else {
        "ok"
    };
    let cap_rounded = ((capability_score as f64) * 10000.0).round() / 10000.0;
    let match_rounded = ((match_ratio as f64) * 10000.0).round() / 10000.0;

    json!({
        "algo": "gr_soft_xsrc_v2_capability",
        "browser_source_count": n_src,
        "keys_compared": compared,
        "keys_matched": matched,
        "match_ratio": match_rounded,
        "soft_mismatches": soft_mismatches,
        "sandbox_blocked": sandbox_blocked,
        "sandbox_partial": sandbox_partial || sandbox_under_two_kinds,
        "sandbox_sources_received_n": sources_received,
        "sandbox_payload_source_n": payload_n,
        "sandbox_kinds_with_data": nest_kinds_with_data,
        "sandbox_kinds_planned_n": kinds_planned,
        "sandbox_all_empty": sandbox_all_empty,
        "sandbox_under_two_kinds": sandbox_under_two_kinds,
        "sandbox_thin_vs_main": thin_vs_main,
        "sandbox_ok": sandbox_ok,
        "js_ok_sandbox_dead": js_ok_sandbox_dead,
        "sandbox_capability_score": cap_rounded,
        "sandbox_capability_band": capability_band,
        "xsrc_numeric": xsrc_numeric_summary(fields_by_source),
    })
}

/// Realm layers for the structured cross-realm diff (audit report §4.2 L1–L4).
/// L1 identity / L2 hardware / L3 network / L4 behavior. Network keys are
/// gateway/CF-side; browsers normally never carry them, so L3 compares only
/// when ≥2 network sources exist.
const REALM_LAYERS: [(&str, &[&str]); 4] = [
    (
        "L1_identity",
        &[
            "user_agent",
            "platform",
            "os_family",
            "language",
            "timezone",
            "timezone_offset_min",
            "webdriver",
        ],
    ),
    (
        "L2_hardware",
        &[
            "hardware_concurrency",
            "device_memory",
            "vendor",
            "max_touch_points",
            "screen_width",
            "screen_height",
            "webgl_unmasked_renderer",
            "webgl_unmasked_vendor",
        ],
    ),
    (
        "L3_network",
        &["asn", "colo", "country", "bot_score", "cf_edge_present", "ip_reputation"],
    ),
    (
        "L4_behavior",
        &[
            "form_class",
            "fonts_present",
            "plugins_len",
            "mime_types_len",
            "chrome_runtime",
            "webrtc_host_ip_hash",
        ],
    ),
];

/// Collapse a source id into an execution realm (`main` ∥ `iframe` ∥ `worker` ∥
/// `sandbox` ∥ `gateway` ∥ `cloudflare`). Sandbox iframe shares the iframe realm,
/// service/shared workers share the worker realm.
pub fn realm_of_source(src: &str) -> &'static str {
    let base = src.split(':').next().unwrap_or(src);
    match base {
        "main" => "main",
        "worker" | "shared_worker" | "service_worker" => "worker",
        "iframe" | "sandbox_iframe" | "sandbox:iframe" => "iframe",
        "sandbox" => "sandbox",
        "gateway" => "gateway",
        "cloudflare" | "cf" => "cloudflare",
        _ => {
            if base.starts_with("worker") {
                "worker"
            } else if base.starts_with("iframe") {
                "iframe"
            } else if base.starts_with("sandbox") {
                "sandbox"
            } else {
                "other"
            }
        }
    }
}

/// One-bucket drift allowance for concurrency/memory classes (worker under-report).
/// Returns true when the class ranks differ by at most one bucket.
fn hardware_class_drift_ok(classes: &[&str]) -> bool {
    let ranks: Vec<i32> = classes
        .iter()
        .map(|c| match *c {
            "c0" => 0,
            "c1_2" => 1,
            "c3_4" => 2,
            "c5_8" => 3,
            "c9_16" => 4,
            "c17p" => 5,
            _ => -1,
        })
        .filter(|r| *r >= 0)
        .collect();
    if ranks.len() < 2 {
        return false;
    }
    let min = *ranks.iter().min().unwrap();
    let max = *ranks.iter().max().unwrap();
    max - min <= 1
}

/// Key verdict for the structured diff: inconsistent values across ≥2 realms are
/// `hard_conflict` for hard-equi / identity keys, `soft_mismatch` otherwise.
fn realm_key_verdict(key: &str, layer: &str, classes: &[String]) -> &'static str {
    let distinct: Vec<&str> = classes.iter().map(|s| s.as_str()).collect();
    let all_same = distinct.iter().all(|c| *c == distinct[0]);
    if all_same {
        return "agree";
    }
    if key == "hardware_concurrency" || key == "device_memory" {
        if hardware_class_drift_ok(&distinct) {
            return "agree";
        }
        return "soft_mismatch";
    }
    // webdriver is excluded from the L1 hard set: worker/iframe sandboxes commonly
    // fail to observe a main-thread automation install, so bool divergence between
    // realms is a soft (weighted) mismatch, not a hard identity lie.
    if is_hard_equi_key(key)
        || (matches!(layer, "L1_identity" | "L2_hardware") && key != "webdriver")
    {
        "hard_conflict"
    } else {
        "soft_mismatch"
    }
}

/// Structured L1–L4 cross-realm diff + conflict graph for analyze (§4.2).
///
/// Supersedes the flat `source_conflicts` string list for severity weighting:
/// keeps per-layer verdicts, per-realm-pair edges and a majority-baseline outlier
/// detection so analyze can weight OS/BR/RPA demotion by *which* realm disagrees
/// and *how hard*, instead of a binary "any conflict".
pub fn structured_realm_diff(fields_by_source: &Map<String, Value>) -> Value {
    // key -> (source, realm, soft-normalized class)
    let mut key_entries: HashMap<String, Vec<(String, String, String)>> = HashMap::new();
    for (src, bucket) in fields_by_source {
        let base = src.split(':').next().unwrap_or(src.as_str());
        if !is_conflict_source(base) {
            continue;
        }
        let Some(obj) = bucket.as_object() else {
            continue;
        };
        let realm = realm_of_source(src);
        for (k, v) in obj {
            if is_surface_local_key(k) || v.is_null() {
                continue;
            }
            if v.as_str().is_some_and(|s| s.is_empty()) {
                continue;
            }
            let norm = soft_normalize_for_compare(k, v);
            if norm.is_empty() {
                continue;
            }
            key_entries
                .entry(k.clone())
                .or_default()
                .push((src.clone(), realm.to_string(), norm));
        }
    }

    // Per realm pair, all conflicted keys with their severity.
    let mut edges: HashMap<(String, String), (Vec<String>, Vec<String>)> = HashMap::new();
    let mut layer_out: Map<String, Value> = Map::new();
    let mut conflict_weight_by_realm: HashMap<String, f64> = HashMap::new();
    let mut realm_conflict_keys: HashMap<String, Vec<String>> = HashMap::new();
    let mut total_hard = 0usize;
    let mut total_soft = 0usize;

    let mut compared_total = 0usize;
    let mut conflicted_total = 0usize;
    for (layer, layer_keys) in REALM_LAYERS {
        let mut compared = 0usize;
        let mut conflicted = 0usize;
        let mut layer_conflict_keys: Vec<String> = Vec::new();
        let mut layer_has_hard = false;
        let mut layer_has_soft = false;
        for key in layer_keys {
            let Some(entries) = key_entries.get(*key) else {
                continue;
            };
            if entries.len() < 2 {
                continue;
            }
            compared += 1;
            compared_total += 1;
            let classes: Vec<String> = entries.iter().map(|e| e.2.clone()).collect();
            let verdict = realm_key_verdict(key, layer, &classes);
            if verdict != "agree" {
                conflicted += 1;
                layer_conflict_keys.push((*key).to_string());
                if verdict == "hard_conflict" {
                    layer_has_hard = true;
                    total_hard += 1;
                } else {
                    layer_has_soft = true;
                    total_soft += 1;
                }
                let weight = if verdict == "hard_conflict" { 2.0 } else { 1.0 };
                for i in 0..entries.len() {
                    for j in (i + 1)..entries.len() {
                        let (a, b) = (&entries[i], &entries[j]);
                        if a.2 == b.2 {
                            continue;
                        }
                        let (ra, rb) = (a.1.clone(), b.1.clone());
                        let edge_key = if ra <= rb {
                            (ra.clone(), rb.clone())
                        } else {
                            (rb.clone(), ra.clone())
                        };
                        let entry = edges.entry(edge_key).or_default();
                        entry.0.push((*key).to_string());
                        entry.1.push(verdict.to_string());
                        *conflict_weight_by_realm.entry(ra.clone()).or_default() += weight;
                        *conflict_weight_by_realm.entry(rb.clone()).or_default() += weight;
                        realm_conflict_keys.entry(ra.clone()).or_default().push((*key).to_string());
                        realm_conflict_keys.entry(rb.clone()).or_default().push((*key).to_string());
                    }
                }
            }
        }
        let layer_verdict = if layer_has_hard {
            "hard_conflict"
        } else if layer_has_soft {
            "soft_mismatch"
        } else if compared > 0 {
            "agree"
        } else {
            "not_compared"
        };
        layer_conflict_keys.sort();
        layer_conflict_keys.dedup();
        conflicted_total += conflicted;
        let mut lov = Map::new();
        lov.insert("verdict".into(), json!(layer_verdict));
        lov.insert("compared".into(), json!(compared));
        lov.insert("conflicted".into(), json!(conflicted));
        lov.insert("conflicted_keys".into(), json!(layer_conflict_keys));
        layer_out.insert(layer.to_string(), Value::Object(lov));
    }

    // Nodes = every conflict-participating realm with ≥1 comparable key;
    // edges = only *conflicting* pairs, deduped, capped for output sparseness.
    let mut nodes: Vec<String> = Vec::new();
    let mut realm_seen: HashSet<String> = HashSet::new();
    for entries in key_entries.values() {
        if entries.len() < 2 {
            continue;
        }
        for e in entries {
            if realm_seen.insert(e.1.clone()) {
                nodes.push(e.1.clone());
            }
        }
    }
    nodes.sort();
    let mut edge_list: Vec<Value> = Vec::new();
    let mut sorted: Vec<((String, String), (Vec<String>, Vec<String>))> = edges.into_iter().collect();
    sorted.sort_by(|a, b| {
        b.1 .0
            .len()
            .cmp(&a.1 .0.len())
            .then_with(|| a.0 .0.cmp(&b.0 .0))
    });
    for ((a, b), (mut keys, sevs)) in sorted.into_iter().take(8) {
        keys.sort();
        keys.dedup();
        let severe = sevs.iter().any(|s| s == "hard_conflict");
        edge_list.push(json!({
            "a": a, "b": b,
            "conflicted_keys": keys,
            "severity": if severe { "hard" } else { "soft" },
        }));
    }

    // Majority-baseline outlier: a realm whose disagreement weight clearly
    // exceeds every other realm (≥3 and ≥2× the second-largest).
    let mut outlier = Value::Null;
    if let Some(max_realm) = conflict_weight_by_realm
        .iter()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(r, w)| (r.clone(), *w))
    {
        let mut second: f64 = 0.0;
        for (r, w) in &conflict_weight_by_realm {
            if r != &max_realm.0 && *w > second {
                second = *w;
            }
        }
        if max_realm.1 >= 3.0 && max_realm.1 >= second * 2.0 {
            let mut keys: Vec<String> = realm_conflict_keys.get(&max_realm.0).cloned().unwrap_or_default();
            keys.sort();
            keys.dedup();
            outlier = json!({
                "realm": max_realm.0,
                "disagree_weight": max_realm.1,
                "keys": keys,
            });
        }
    }

    let verdict = if total_hard > 0 {
        "hard_conflict"
    } else if total_soft > 0 {
        "soft_mismatch"
    } else if compared_total > 0 {
        "agree"
    } else {
        "not_compared"
    };
    let severity = ((total_hard as f64 * 0.4 + total_soft as f64 * 0.15) * 100.0).round() / 100.0;

    // M1 realm coherence (iss/74 §6-M1): numeric-level cross-realm agreement —
    // key-level ratio of agreed vs compared identity/caps keys, capped when any
    // hard conflict exists, score 1.0 (qualified) when nothing is comparable.
    let _compared_f = compared_total as f64;
    let (realm_coherence_score, realm_coherence_verdict) = if compared_total == 0 {
        (1.0f64, "qualified")
    } else {
        let base = 1.0 - conflicted_total as f64 / compared_total as f64;
        let capped = if total_hard > 0 { base.min(0.5) } else { base };
        let score = (capped * 10000.0).round() / 10000.0;
        let v = if score >= 0.9 {
            "agree"
        } else if score >= 0.6 {
            "partial"
        } else {
            "conflict"
        };
        (score, v)
    };
    // M1 numeric cap keys: deviceMemory/hwc/model/tz/webgl caps — how many were
    // actually compared across realms (≥2 sources with non-empty classes).
    const M1_NUMERIC_CAPS: [&str; 9] = [
        "device_memory",
        "hardware_concurrency",
        "device_model",
        "timezone",
        "timezone_offset_min",
        "max_touch_points",
        "screen_width",
        "screen_height",
        "webgl_max_texture",
    ];
    let numeric_caps_compared = M1_NUMERIC_CAPS
        .iter()
        .filter(|k| {
            key_entries
                .get(**k)
                .map(|e| e.len() >= 2)
                .unwrap_or(false)
        })
        .count();

    let mut all_conflicted: Vec<String> = Vec::new();
    for v in layer_out.values() {
        if let Some(ks) = v.get("conflicted_keys").and_then(|x| x.as_array()) {
            for k in ks {
                if let Some(s) = k.as_str() {
                    all_conflicted.push(s.to_string());
                }
            }
        }
    }
    all_conflicted.sort();
    all_conflicted.dedup();

    json!({
        "algo": "gr_realm_conflict_graph_v1",
        "verdict": verdict,
        "severity": severity,
        "conflicted_keys": all_conflicted,
        "layers": Value::Object(layer_out),
        "graph": {"nodes": nodes, "edges": edge_list},
        "outlier_realm": outlier,
        "realm_coherence_score": realm_coherence_score,
        "realm_coherence_verdict": realm_coherence_verdict,
        "numeric_caps_compared": numeric_caps_compared,
    })
}

/// Source trust score for store-side auth view (mirrors gr-core source_trust ladder).
/// Higher = harder for page JS to forge.
fn store_source_trust_score(src: &str, key: &str) -> u8 {
    let base = src.split(':').next().unwrap_or(src);
    let silicon = key.starts_with("hw_curve_")
        || matches!(
            key,
            "residual_std"
                | "residual_mean"
                | "webgl_unmasked_renderer"
                | "unit_surface_id"
        );
    match base {
        "gateway" => 95,
        "cloudflare" | "cf" => 85,
        "worker" | "shared_worker" | "service_worker" => 70,
        "iframe" | "sandbox" | "sandbox_iframe" => 55,
        "main" => {
            if silicon {
                45
            } else {
                30
            }
        }
        _ => {
            if base.starts_with("worker") {
                70
            } else if base.starts_with("iframe") {
                55
            } else {
                0
            }
        }
    }
}

/// Which source is authoritative for each identity key (for ops + device_id mint).
///
/// **v2 policy**: prefer **harder-to-forge** source on conflict (worker > iframe > main soft).
/// Silicon hard conflict → not mint_usable. Soft conflict → harder nest wins for mint_usable.
pub fn build_source_auth_view(
    fields_by_source: &Map<String, Value>,
    source_conflicts: &[String],
) -> Value {
    let conflict_keys: HashSet<String> = source_conflicts
        .iter()
        .filter_map(|c| c.strip_prefix("source_conflict:").map(|s| s.to_string()))
        .collect();
    let main = fields_by_source.get("main").and_then(|v| v.as_object());
    let mut map = Map::new();
    let auth_keys = [
        "user_agent",
        "platform",
        "os_family",
        "hardware_concurrency",
        "timezone",
        "webdriver",
        "language",
        "form_class",
        "hw_curve_audio",
        "hw_curve_webgl",
        "webgl_unmasked_renderer",
        "font_count",
        "webrtc_host_ip_hash",
        "device_memory",
        "residual_std",
        "residual_mean",
    ];
    for k in auth_keys {
        let conflicted = conflict_keys.contains(k);
        let main_has = main.is_some_and(|m| {
            m.get(k).is_some_and(|v| match v {
                Value::Null => false,
                Value::String(s) => !s.is_empty(),
                _ => true,
            })
        });
        let mut candidates: Vec<(String, u8)> = Vec::new();
        for (src, bucket) in fields_by_source {
            if let Some(obj) = bucket.as_object() {
                if obj.get(k).is_some_and(|v| !v.is_null() && v.as_str() != Some("")) {
                    let score = store_source_trust_score(src, k);
                    candidates.push((src.clone(), score));
                }
            }
        }
        candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let other_has: Vec<String> = candidates
            .iter()
            .filter(|(s, _)| {
                let b = s.split(':').next().unwrap_or(s);
                b != "main"
            })
            .map(|(s, _)| s.split(':').next().unwrap_or(s).to_string())
            .collect();

        let silicon = k.starts_with("hw_curve_")
            || matches!(k, "residual_std" | "residual_mean" | "webgl_unmasked_renderer");

        let (authentic, mint_usable) = if candidates.is_empty() {
            ("missing".to_string(), false)
        } else if conflicted && silicon {
            // Silicon conflict: do not mint
            ("conflict_silicon_skip".into(), false)
        } else if conflicted {
            // Soft conflict: harder-to-forge source
            let (src, _) = &candidates[0];
            (src.clone(), true)
        } else if silicon && main_has {
            // Silicon agree path: main fidelity
            ("main".into(), true)
        } else {
            let (src, _) = &candidates[0];
            (src.clone(), true)
        };

        map.insert(
            k.into(),
            json!({
                "authentic_source": authentic,
                "main_present": main_has,
                "nest_present": other_has,
                "conflicted": conflicted,
                "mint_usable": mint_usable,
                "trust_score": candidates.first().map(|(_, s)| *s).unwrap_or(0),
                "policy": "prefer_harder_to_forge_v2",
            }),
        );
    }
    json!({
        "algo": "gr_source_auth_v2_trust_ladder",
        "policy": "prefer_harder_to_forge_on_conflict",
        "ladder": ["gateway(95)", "cloudflare(85)", "worker(70)", "iframe(55)", "main_silicon(45)", "main_soft(30)"],
        "keys": map,
    })
}

/// Freeze fields for commercial device_id mint using trust ladder (v2).
///
/// - Silicon curves: prefer **main** when present (fidelity); drop on silicon conflict.
/// - Soft identity on conflict: prefer **worker** then iframe over main.
/// - Non-conflict soft: prefer harder nest when present, else main.
pub fn authentic_fields_for_mint(
    merged: &Map<String, Value>,
    fields_by_source: &Map<String, Value>,
    source_conflicts: &[String],
) -> Map<String, Value> {
    let mut out = merged.clone();
    let conflict_keys: HashSet<String> = source_conflicts
        .iter()
        .filter_map(|c| c.strip_prefix("source_conflict:").map(|s| s.to_string()))
        .collect();

    let silicon_keys = [
        "hw_curve_audio",
        "hw_curve_webgl",
        "hw_curve_cpu",
        "residual_mean",
        "residual_std",
        "unit_surface_digest",
        "unit_surface_id",
        "webgl_unmasked_renderer",
    ];
    let soft_keys = [
        "user_agent",
        "platform",
        "os_family",
        "hardware_concurrency",
        "device_memory",
        "webdriver",
        "timezone",
        "timezone_offset_min",
        "language",
        "form_class",
        "webrtc_host_ip_hash",
        "os_instance_hash",
    ];

    // Silicon: main preferred; conflict drop
    if let Some(main) = fields_by_source.get("main").and_then(|v| v.as_object()) {
        for k in silicon_keys {
            if conflict_keys.contains(k) {
                out.remove(k);
                continue;
            }
            if let Some(v) = main.get(k) {
                if !v.is_null() {
                    out.insert(k.into(), v.clone());
                }
            }
        }
    }

    // Soft: pick highest trust source; on conflict still pick worker/iframe over main
    for k in soft_keys {
        let mut best: Option<(u8, Value)> = None;
        for (src, bucket) in fields_by_source {
            if let Some(obj) = bucket.as_object() {
                if let Some(v) = obj.get(k) {
                    if v.is_null() || v.as_str() == Some("") {
                        continue;
                    }
                    let score = store_source_trust_score(src, k);
                    if best.as_ref().map(|(s, _)| score > *s).unwrap_or(true) {
                        best = Some((score, v.clone()));
                    }
                }
            }
        }
        if let Some((_, v)) = best {
            // Silicon-style hard skip only for pure silicon keys (handled above)
            out.insert(k.into(), v);
        } else if conflict_keys.contains(k) {
            out.remove(k);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── A3 cross-source numeric consistency tests ──

    fn src_map(main: Value, nests: &[(&str, Value)]) -> Map<String, Value> {
        let mut fbs = Map::new();
        fbs.insert("main".into(), main);
        for (k, v) in nests {
            fbs.insert((*k).into(), v.clone());
        }
        fbs
    }

    fn webgl_curve(base: f64, step: f64, n: usize) -> Vec<f64> {
        (0..n).map(|i| base + i as f64 * step).collect()
    }

    #[test]
    fn sample_quantized_returns_seven_points_rounded() {
        let c: Vec<f64> = (0..64).map(|i| 0.1 + i as f64 * 0.0012456789).collect();
        let s = sample_quantized(&c, XSRC_NUMERIC_POINTS);
        assert_eq!(s.len(), 7);
        assert_eq!(s[0], 0.1);
        assert!((s[6] - (0.1 + 63.0 * 0.0012456789)).abs() < 0.001);
        assert!(s.iter().all(|x| (x * 1000.0).fract().abs() < 1e-9 + 0.001), "3dp quantized");
    }

    #[test]
    fn classify_numeric_delta_buckets() {
        let a = webgl_curve(0.2, 0.001, 32);
        let b = webgl_curve(0.2, 0.001, 32); // identical
        assert_eq!(classify_numeric_delta(&a, &b).0, "consistent");
        let c: Vec<f64> = a.iter().map(|x| x + 0.003).collect(); // rel ≤ 1.8% → consistent
        assert_eq!(classify_numeric_delta(&a, &c).0, "consistent");
        let d: Vec<f64> = a.iter().map(|x| x + 0.05).collect(); // rel ~0.2
        assert_eq!(classify_numeric_delta(&a, &d).0, "divergent");
        let e: Vec<f64> = a.iter().map(|x| x + 0.015).collect(); // rel ~0.06
        assert_eq!(classify_numeric_delta(&a, &e).0, "minor");
        assert_eq!(classify_numeric_delta(&[], &a).0, "missing");
    }

    #[test]
    fn xsrc_numeric_compare_consistent_when_nest_matches_main() {
        let main = json!({"hw_curve_webgl": webgl_curve(0.2, 0.001, 32)});
        let worker = json!({"hw_curve_webgl": webgl_curve(0.201, 0.001, 32)});
        // quantized deltas → 0.001/0.205 ≈ 0.005 rel → consistent
        let fbs = src_map(main.clone(), &[("worker", worker.clone())]);
        let d = xsrc_numeric_compare(&fbs, "hw_curve_webgl");
        assert_eq!(d["verdict"], "consistent", "{d}");
        assert_eq!(d["sources_compared"], 1);
        assert_eq!(d["sources"][0]["class"], "consistent");
    }

    #[test]
    fn xsrc_numeric_compare_divergent_and_missing() {
        let main = json!({"hw_curve_webgl": webgl_curve(0.2, 0.001, 32)});
        let worker = json!({"hw_curve_webgl": webgl_curve(0.2, 0.3, 32)}); // wildly different
        let iframe = json!({}); // field absent
        let fbs = src_map(main.clone(), &[("worker", worker.clone()), ("iframe", iframe)]);
        let d = xsrc_numeric_compare(&fbs, "hw_curve_webgl");
        assert_eq!(d["verdict"], "divergent", "{d}");
        assert_eq!(d["sources_compared"], 1);
        assert_eq!(d["sources_missing"], 1);
        let m2 = json!({"platform": "Linux"});
        let fbs2 = src_map(m2, &[]);
        assert_eq!(xsrc_numeric_compare(&fbs2, "hw_curve_webgl")["verdict"], "missing");
    }

    #[test]
    fn residual_mean_cross_source_consistent() {
        let main = json!({"residual_mean": 0.2611});
        let sandbox = json!({"residual_mean": 0.2622});
        let fbs = src_map(main, &[("sandbox", sandbox)]);
        let d = xsrc_numeric_compare(&fbs, "residual_mean");
        assert_eq!(d["verdict"], "consistent", "{d}");
    }

    #[test]
    fn xsrc_numeric_summary_wires_slots_and_verdict() {
        let main = json!({
            "hw_curve_webgl": webgl_curve(0.2, 0.001, 32),
            "hw_curve_audio": webgl_curve(0.5, 0.0005, 16),
            "residual_mean": 0.2611,
        });
        let worker = json!({
            "hw_curve_webgl": webgl_curve(0.2, 0.9, 32), // divergent
            "hw_curve_audio": webgl_curve(0.501, 0.0005, 16),
            "residual_mean": 0.2619,
        });
        let fbs = src_map(main, &[("worker", worker)]);
        let s = xsrc_numeric_summary(&fbs);
        assert_eq!(s["xsrc_numeric_verdict"], "divergent", "{s}");
        assert_eq!(
            s["slots"]["xsrc_numeric_delta_hw_curve_webgl"]["verdict"],
            "divergent"
        );
        assert_eq!(
            s["slots"]["xsrc_numeric_delta_hw_curve_audio"]["verdict"],
            "consistent"
        );
        assert_eq!(
            s["slots"]["xsrc_numeric_delta_hw_curve_cpu"]["verdict"],
            "missing"
        );
    }

    #[test]
    fn extended_identity_keys_participate_in_conflict() {
        let mut fbs = Map::new();
        fbs.insert(
            "main".into(),
            json!({"screen_width": 1920, "max_touch_points": 10, "device_model": "MacBookPro18,3"}),
        );
        fbs.insert(
            "worker".into(),
            json!({"screen_width": 640, "max_touch_points": 0, "device_model": "MacBookPro18,3"}),
        );
        let c = detect_source_conflicts(&fbs);
        assert!(
            c.iter().any(|s| s == "source_conflict:screen_width"),
            "{c:?}"
        );
        assert!(
            c.iter().any(|s| s == "source_conflict:max_touch_points"),
            "{c:?}"
        );
        assert!(!c.iter().any(|s| s.contains("device_model")), "{c:?}");
    }

    #[test]
    fn assess_multi_source_consistency_includes_xsrc_numeric() {
        let main = json!({
            "platform": "Linux x86_64",
            "hardware_concurrency": 16,
            "residual_mean": 0.25,
        });
        let worker = json!({
            "platform": "Linux x86_64",
            "hardware_concurrency": 8,
        });
        let fbs = src_map(main, &[("worker", worker)]);
        let cons = assess_multi_source_consistency(&fbs);
        assert!(cons.get("xsrc_numeric").is_some());
        assert_eq!(
            cons["xsrc_numeric"]["slots"]["xsrc_numeric_delta_residual_mean"]["verdict"],
            "missing",
            "worker lacks residual_mean → per-source missing, verdict missing when no source compared"
        );
    }

    // ── M1 realm coherence numeric-level tests ──

    #[test]
    fn realm_coherence_score_agrees_when_realms_match() {
        let mut fbs = Map::new();
        fbs.insert(
            "main".into(),
            json!({
                "platform": "Linux x86_64",
                "os_family": "linux",
                "hardware_concurrency": 16,
                "device_memory": 32,
                "timezone": "Asia/Shanghai",
                "max_touch_points": 10,
                "screen_width": 1920,
                "webgl_max_texture": 16384,
            }),
        );
        fbs.insert(
            "worker".into(),
            json!({
                "platform": "Linux x86_64",
                "os_family": "linux",
                // worker under-reports cores by one bucket — allowed
                "hardware_concurrency": 8,
                "device_memory": 32,
                "timezone": "Asia/Shanghai",
                "max_touch_points": 10,
                "screen_width": 1920,
                "webgl_max_texture": 16384,
            }),
        );
        let d = structured_realm_diff(&fbs);
        assert_eq!(d["verdict"], "agree", "{d}");
        let score = d["realm_coherence_score"].as_f64().unwrap();
        assert!(score >= 0.9, "score: {score} {d}");
        assert_eq!(d["realm_coherence_verdict"], "agree", "{d}");
        assert!(d["numeric_caps_compared"].as_u64().unwrap() >= 4, "{d}");
    }

    #[test]
    fn realm_coherence_score_drops_with_hard_conflicts() {
        let mut fbs = Map::new();
        fbs.insert(
            "main".into(),
            json!({
                "platform": "Linux x86_64",
                "os_family": "linux",
                "webgl_unmasked_renderer": "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 Direct3D11)",
                "hardware_concurrency": 16,
                "timezone": "Asia/Shanghai",
            }),
        );
        fbs.insert(
            "worker".into(),
            json!({
                "platform": "Windows NT 10.0",
                "os_family": "windows",
                "webgl_unmasked_renderer": "ANGLE (Apple M1)",
                "hardware_concurrency": 4,
                "timezone": "UTC",
            }),
        );
        let d = structured_realm_diff(&fbs);
        assert_eq!(d["verdict"], "hard_conflict", "{d}");
        let score = d["realm_coherence_score"].as_f64().unwrap();
        assert!(score <= 0.5, "hard conflicts cap coherence at 0.5: {score} {d}");
        assert_eq!(d["realm_coherence_verdict"], "conflict", "{d}");
    }

    #[test]
    fn realm_coherence_qualified_without_comparables() {
        let mut fbs = Map::new();
        fbs.insert("main".into(), json!({"behavior_count": 3}));
        fbs.insert("iframe".into(), json!({"behavior_count": 7}));
        let d = structured_realm_diff(&fbs);
        assert_eq!(d["realm_coherence_score"], 1.0, "{d}");
        assert_eq!(d["realm_coherence_verdict"], "qualified", "{d}");
        assert_eq!(d["numeric_caps_compared"], 0, "{d}");
    }

    #[test]
    fn b10x_does_not_overwrite_healthy_b10_primary_path() {
        let mut fields = Map::new();
        let mut fbs = Map::new();
        let mut gw = Map::new();
        let mut cf = Map::new();
        let b10_curve: Vec<f64> = (0..32).map(|i| 0.229 + i as f64 * 0.001).collect();
        let b10x_curve: Vec<f64> = (0..32).map(|i| 0.111 + i as f64 * 0.001).collect();
        let b10 = json!({
            "residual_paths": [{
                "path_id": "v3f_128_noderiv",
                "shader_mode": "noderiv",
                "ok": true,
                "entropy_ok": true,
                "mean": 0.261126875,
                "curve": b10_curve
            }]
        });
        merge_batch_with_id(
            "main",
            Some("B10_hw_curves"),
            b10.as_object().unwrap().clone(),
            &mut fields,
            &mut fbs,
            &mut gw,
            &mut cf,
        );
        let b10x = json!({
            "residual_paths": [{
                "path_id": "v3f_128_noderiv",
                "shader_mode": "noderiv",
                "ok": true,
                "entropy_ok": true,
                "mean": 0.2600390625,
                "curve": b10x_curve
            }]
        });
        merge_batch_with_id(
            "main",
            Some("B10x_legacy_webgl1"),
            b10x.as_object().unwrap().clone(),
            &mut fields,
            &mut fbs,
            &mut gw,
            &mut cf,
        );
        let rp = fields
            .get("residual_paths")
            .and_then(|v| v.as_array())
            .expect("residual_paths");
        assert_eq!(rp.len(), 1);
        let c0 = rp[0]["curve"][0].as_f64().unwrap();
        assert!(
            (c0 - 0.229).abs() < 1e-9,
            "B10 primary curve must survive B10x last-write: {rp:?}"
        );
        assert_eq!(rp[0]["mean"].as_f64(), Some(0.261126875));
    }

    #[test]
    fn rpa_surface_local_keys_do_not_conflict() {
        let mut fbs = Map::new();
        fbs.insert(
            "main".into(),
            json!({
                "platform": "Linux x86_64",
                "behavior_events": [{"kind":"click","source":"main"}],
                "behavior_count": 4,
                "rpa_source": "main",
                "sandbox_kind": "main",
                "page_url": "https://demo.example/app",
            }),
        );
        fbs.insert(
            "iframe".into(),
            json!({
                "platform": "Linux x86_64",
                "behavior_events": [{"kind":"pointerdown","source":"iframe:d1"}],
                "behavior_count": 1,
                "rpa_source": "iframe:d1",
                "sandbox_kind": "iframe",
                "page_url": "https://demo.example/app",
            }),
        );
        fbs.insert(
            "worker".into(),
            json!({
                "platform": "Linux x86_64",
                "behavior_events": [{"kind":"worker_tick","source":"worker:d1"}],
                "behavior_count": 1,
                "rpa_source": "worker:d1",
                "sandbox_kind": "worker",
                "worker_rpa": true,
            }),
        );
        let c = detect_source_conflicts(&fbs);
        assert!(
            c.is_empty(),
            "RPA multi-source variance must not xsrc-conflict: {c:?}"
        );
    }

    #[test]
    fn real_identity_field_conflict_still_detected() {
        let mut fbs = Map::new();
        fbs.insert(
            "main".into(),
            json!({
                "hardware_concurrency": 32,
                "platform": "Linux x86_64",
                "behavior_count": 2,
            }),
        );
        fbs.insert(
            "iframe".into(),
            json!({
                "hardware_concurrency": 2,
                "platform": "Linux x86_64",
                "behavior_count": 9,
            }),
        );
        let c = detect_source_conflicts(&fbs);
        assert!(
            c.iter().any(|s| s == "source_conflict:hardware_concurrency"),
            "large cores class mismatch must conflict: {c:?}"
        );
        assert!(
            !c.iter().any(|s| s.contains("behavior")),
            "behavior must be excluded: {c:?}"
        );
    }

    #[test]
    fn soft_platform_and_adjacent_cores_do_not_false_conflict() {
        let mut fbs = Map::new();
        // Same OS class, different platform strings; adjacent core buckets (worker under-report)
        fbs.insert(
            "main".into(),
            json!({
                "platform": "MacIntel",
                "os_family": "macos",
                "hardware_concurrency": 8,
                "timezone": "Asia/Shanghai",
                "user_agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Chrome/120.0.0.0",
            }),
        );
        fbs.insert(
            "worker".into(),
            json!({
                "platform": "MacIntel",
                "os_family": "macos",
                "hardware_concurrency": 4,
                "timezone": "Asia/Shanghai",
                "user_agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Chrome/119.0.0.0",
            }),
        );
        let c = detect_source_conflicts(&fbs);
        assert!(
            c.is_empty(),
            "soft same-host multi-surface must not false-conflict: {c:?}"
        );
        let cons = assess_multi_source_consistency(&fbs);
        assert!(
            cons["match_ratio"].as_f64().unwrap() >= 0.85,
            "match_ratio high: {cons}"
        );
    }

    #[test]
    fn os_family_cross_platform_conflicts() {
        let mut fbs = Map::new();
        fbs.insert(
            "main".into(),
            json!({"os_family": "macos", "platform": "MacIntel", "hardware_concurrency": 8}),
        );
        fbs.insert(
            "worker".into(),
            json!({"os_family": "linux", "platform": "Linux x86_64", "hardware_concurrency": 8}),
        );
        let c = detect_source_conflicts(&fbs);
        assert!(
            c.iter().any(|s| s.contains("os_family") || s.contains("platform")),
            "cross-OS must conflict: {c:?}"
        );
    }

    #[test]
    fn main_prefer_identity_not_overwritten_by_worker() {
        let mut fields = Map::new();
        let mut fbs = Map::new();
        let mut gw = Map::new();
        let mut cf = Map::new();
        let mut main_pf = Map::new();
        main_pf.insert("user_agent".into(), json!("MAIN_UA"));
        main_pf.insert("platform".into(), json!("Linux x86_64"));
        merge_batch("main", main_pf, &mut fields, &mut fbs, &mut gw, &mut cf);
        let mut w_pf = Map::new();
        w_pf.insert("user_agent".into(), json!("WORKER_LIE_UA"));
        w_pf.insert("platform".into(), json!("Win32"));
        merge_batch("worker", w_pf, &mut fields, &mut fbs, &mut gw, &mut cf);
        assert_eq!(fields.get("user_agent").and_then(|v| v.as_str()), Some("MAIN_UA"));
        assert_eq!(
            fields.get("platform").and_then(|v| v.as_str()),
            Some("Linux x86_64")
        );
        assert_eq!(
            fbs["worker"]["user_agent"].as_str(),
            Some("WORKER_LIE_UA"),
            "nest still kept in fields_by_source"
        );
    }

    #[test]
    fn capability_dead_when_no_nest_payload() {
        let mut fbs = Map::new();
        fbs.insert(
            "main".into(),
            json!({
                "user_agent": "Chrome",
                "platform": "Linux",
                "os_family": "linux",
                "hardware_concurrency": 8,
                "language": "en",
                "webdriver": false,
                "timezone": "UTC",
                "sandbox_blocked": true,
                "sandbox_sources_received": [],
                "sandbox_kinds_planned": ["iframe", "worker"],
            }),
        );
        let cons = assess_multi_source_consistency(&fbs);
        assert_eq!(cons["sandbox_capability_band"], "dead");
        assert!(cons["js_ok_sandbox_dead"].as_bool().unwrap());
        assert!(cons["sandbox_all_empty"].as_bool().unwrap());
        assert!(cons["sandbox_capability_score"].as_f64().unwrap() <= 0.05);
    }

    #[test]
    fn capability_ok_with_dual_nest_payload() {
        let mut fbs = Map::new();
        fbs.insert(
            "main".into(),
            json!({
                "user_agent": "Chrome",
                "platform": "Linux x86_64",
                "os_family": "linux",
                "hardware_concurrency": 8,
                "language": "en-US",
                "webdriver": false,
                "timezone": "UTC",
                "sandbox_blocked": false,
                "sandbox_sources_received": ["iframe:d1", "worker:d1"],
                "sandbox_kinds_planned": ["iframe", "worker"],
            }),
        );
        fbs.insert(
            "iframe".into(),
            json!({
                "user_agent": "Chrome",
                "platform": "Linux x86_64",
                "os_family": "linux",
                "hardware_concurrency": 8,
                "language": "en-US",
                "webdriver": false,
                "timezone": "UTC",
            }),
        );
        fbs.insert(
            "worker".into(),
            json!({
                "user_agent": "Chrome",
                "platform": "Linux x86_64",
                "os_family": "linux",
                "hardware_concurrency": 8,
                "language": "en-US",
                "webdriver": false,
                "timezone": "UTC",
            }),
        );
        let cons = assess_multi_source_consistency(&fbs);
        assert!(cons["sandbox_ok"].as_bool().unwrap(), "{cons}");
        assert!(
            cons["sandbox_capability_score"].as_f64().unwrap() >= 0.7,
            "{cons}"
        );
        let auth = build_source_auth_view(&fbs, &[]);
        // Soft identity: prefer harder-to-forge worker over main when multi-source present.
        assert_eq!(
            auth["keys"]["user_agent"]["authentic_source"],
            "worker",
            "{auth}"
        );
        assert_eq!(auth["policy"], "prefer_harder_to_forge_on_conflict");
        // Silicon curves still prefer main when only main has them (not in this fixture).
    }
}

#[cfg(test)]
mod verify_agg_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn multi_r_pack_ratios_are_sum_not_last_write() {
        let mut fields = Map::new();
        let mut fbs = Map::new();
        let mut gw = Map::new();
        let mut cf = Map::new();
        let p1 = json!({
            "verify_probe_ops_n": 100,
            "verify_empty_n": 40,
            "verify_nonempty_n": 60,
            "verify_ok_n": 50,
            "verify_dim_fail_n": 1,
            "verify_nonempty_ratio": 0.6,
            "verify_not_score_material": true,
            "verify_role": "authenticity_spotcheck",
            "verify_pack": "R01_spotcheck",
        })
        .as_object()
        .cloned()
        .unwrap();
        let p2 = json!({
            "verify_probe_ops_n": 100,
            "verify_empty_n": 10,
            "verify_nonempty_n": 90,
            "verify_ok_n": 80,
            "verify_dim_fail_n": 3,
            "verify_nonempty_ratio": 0.9,
            "verify_not_score_material": true,
            "verify_role": "authenticity_spotcheck",
            "verify_pack": "R02_spotcheck",
        })
        .as_object()
        .cloned()
        .unwrap();
        merge_batch_with_id("main", Some("R01_spotcheck"), p1, &mut fields, &mut fbs, &mut gw, &mut cf);
        merge_batch_with_id("main", Some("R02_spotcheck"), p2, &mut fields, &mut fbs, &mut gw, &mut cf);
        assert_eq!(fields.get("verify_agg_pack_n").and_then(|v| v.as_u64()), Some(2));
        assert!((fields.get("verify_probe_ops_n").and_then(|v| v.as_f64()).unwrap() - 200.0).abs() < 1e-9);
        // nonempty 60+90=150 / 200 = 0.75 — not last-write 0.9
        let r = fields.get("verify_nonempty_ratio").and_then(|v| v.as_f64()).unwrap();
        assert!((r - 0.75).abs() < 1e-9, "got {r}");
        assert!((fields.get("verify_dim_fail_n").and_then(|v| v.as_f64()).unwrap() - 3.0).abs() < 1e-9);
        assert_eq!(fields.get("verify_aggregate_algo").and_then(|v| v.as_str()), Some("gr_verify_agg_v1"));
    }
}

#[cfg(test)]
mod realm_conflict_tests {
    use super::*;
    use serde_json::json;

    fn fbs_with(entries: &[(&str, Value)]) -> Map<String, Value> {
        let mut m = Map::new();
        for (k, v) in entries {
            m.insert((*k).to_string(), v.clone());
        }
        m
    }

    #[test]
    fn consistent_realms_agree_with_no_outlier() {
        let fbs = fbs_with(&[
            (
                "main",
                json!({
                    "user_agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
                    "platform": "Linux x86_64",
                    "language": "en-US",
                    "webdriver": false,
                    "hardware_concurrency": 8,
                }),
            ),
            (
                "iframe:child0",
                json!({
                    "user_agent": "Mozilla/5.0 (X11; Linux x86_64) Chrome/126.0.0.0",
                    "platform": "Linux",
                    "language": "en-US",
                    "webdriver": false,
                    "hardware_concurrency": 8,
                }),
            ),
            (
                "worker:probe",
                json!({
                    "user_agent": "Mozilla/5.0 (X11; Linux x86_64) Chrome/126.0.0.0",
                    "platform": "Linux x86_64",
                    "language": "en-US",
                    "webdriver": false,
                    "hardware_concurrency": 8,
                }),
            ),
        ]);
        let diff = structured_realm_diff(&fbs);
        assert_eq!(diff["verdict"], "agree", "{diff}");
        assert_eq!(diff["outlier_realm"], Value::Null);
        assert_eq!(diff["severity"], 0.0);
        assert_eq!(diff["graph"]["nodes"].as_array().unwrap().len(), 3);
        assert_eq!(diff["layers"]["L1_identity"]["verdict"], "agree");
        assert_eq!(diff["graph"]["edges"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn hardware_bucket_drift_is_not_a_conflict() {
        let fbs = fbs_with(&[
            (
                "main",
                json!({"platform": "Linux x86_64", "hardware_concurrency": 8}),
            ),
            (
                "worker:probe",
                json!({"platform": "Linux", "hardware_concurrency": 6}),
            ),
        ]);
        let diff = structured_realm_diff(&fbs);
        assert_eq!(diff["verdict"], "agree", "{diff}");
        let l2 = &diff["layers"]["L2_hardware"];
        assert_eq!(l2["verdict"], "agree", "{l2}");
    }

    #[test]
    fn divergent_sandbox_realm_is_outlier_with_hard_conflict() {
        let fbs = fbs_with(&[
            (
                "main",
                json!({
                    "user_agent": "Mozilla/5.0 (X11; Linux x86_64) Chrome/126.0.0.0",
                    "platform": "Linux x86_64",
                    "language": "en-US",
                    "webdriver": false,
                }),
            ),
            (
                "iframe:child0",
                json!({
                    "user_agent": "Mozilla/5.0 (X11; Linux x86_64) Chrome/126.0.0.0",
                    "platform": "Linux",
                    "language": "en-US",
                    "webdriver": false,
                }),
            ),
            (
                "worker:probe",
                json!({
                    "user_agent": "Mozilla/5.0 (X11; Linux x86_64) Chrome/126.0.0.0",
                    "platform": "Linux",
                    "language": "en-US",
                    "webdriver": false,
                }),
            ),
            (
                "sandbox",
                json!({
                    "user_agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/126.0.0.0",
                    "platform": "Win32",
                    "language": "zh-CN",
                    "webdriver": true,
                }),
            ),
        ]);
        let diff = structured_realm_diff(&fbs);
        assert_eq!(diff["verdict"], "hard_conflict", "{diff}");
        assert!(diff["severity"].as_f64().unwrap() > 0.0);
        let out = diff["outlier_realm"].as_object().expect("outlier present");
        assert_eq!(out["realm"], "sandbox");
        let keys = out["keys"].as_array().unwrap();
        assert!(keys.iter().any(|k| k == "platform"), "{keys:?}");
        // graph edge exists between sandbox and a browser realm
        let edges = diff["graph"]["edges"].as_array().unwrap();
        assert!(edges.iter().any(|e| e["a"] == "sandbox" || e["b"] == "sandbox"), "{edges:?}");
        assert_eq!(diff["layers"]["L1_identity"]["verdict"], "hard_conflict");
    }

    #[test]
    fn soft_mismatch_adds_severity_without_hard_outlier_threshold() {
        let fbs = fbs_with(&[
            ("main", json!({"platform": "Linux x86_64", "webdriver": false})),
            (
                "worker:probe",
                json!({"platform": "Linux", "webdriver": true}),
            ),
        ]);
        let diff = structured_realm_diff(&fbs);
        // webdriver bool divergence = soft conflict, not hard (L1 hard only for
        // identity *class* keys; webdriver is its own dedicated equality check)
        assert_eq!(diff["verdict"], "soft_mismatch", "{diff}");
        assert!(diff["severity"].as_f64().unwrap() > 0.0);
        assert_eq!(diff["outlier_realm"], Value::Null);
    }

    fn merge_one(base_src: &str, bid: &str, pf: Value) -> (Map<String, Value>, Map<String, Value>) {
        let mut fields = Map::new();
        let mut fbs = Map::new();
        let mut gw = Map::new();
        let mut cf = Map::new();
        merge_batch_with_id(
            base_src,
            Some(bid),
            pf.as_object().cloned().unwrap_or_default(),
            &mut fields,
            &mut fbs,
            &mut gw,
            &mut cf,
        );
        (fields, gw)
    }

    /// A2: FE B8 convergence claim with zero server edge evidence → flag; with
    /// gateway evidence → b8_edge_seen only.
    #[test]
    fn b8_converge_without_edge_flags_fe_only_claim() {
        // Server-written gateway batch first (supplies gateway_fields)
        let (_, gw) = merge_one(
            "gateway",
            "B8_gateway",
            json!({
                "gateway_user_agent": "curl-edge",
                "server_client_ip": "203.0.113.9",
            }),
        );
        let mut fields = Map::new();
        let mut fbs = Map::new();
        let mut gw2 = gw.clone();
        let mut cf = Map::new();
        // FE ingest claims convergence but carries no edge evidence of its own
        merge_batch_with_id(
            "main",
            Some("B8_gateway"),
            json!({"post_converge": true, "b8_bases": [1, 2]})
                .as_object()
                .cloned()
                .unwrap(),
            &mut fields,
            &mut fbs,
            &mut gw2,
            &mut cf,
        );
        assert_eq!(
            fields.get("b8_edge_seen").unwrap(),
            &json!(true),
            "server gateway_fields count as edge evidence"
        );
        assert_eq!(
            fields.get("b8_converge_without_edge").unwrap(),
            &json!(false),
            "with real edge evidence the claim is fine: {fields:?}"
        );

        // No gateway batch at all → converge claim is the flag
        let (fields2, _) = merge_one(
            "main",
            "B8_gateway",
            json!({"post_converge": true, "b8_enrich": true}),
        );
        assert_eq!(fields2.get("b8_edge_seen").unwrap(), &json!(false));
        assert_eq!(
            fields2.get("b8_converge_without_edge").unwrap(),
            &json!(true),
            "FE-only convergence claim without edge observation: {fields2:?}"
        );

        // Non-B8 batches never stamp the flags
        let (fields3, _) = merge_one("main", "B10_hw_curves", json!({"post_converge": true}));
        assert!(!fields3.contains_key("b8_converge_without_edge"));
    }

    /// A3: sandbox nest_residual_algo vs main residual_algo.
    #[test]
    fn sandbox_residual_algo_match_requires_main_algo() {
        let (mut fields, _) = merge_one(
            "main",
            "B10_hw_curves",
            json!({"residual_algo": "gr_residual_v2"}),
        );
        let mut fbs = Map::new();
        let mut gw = Map::new();
        let mut cf = Map::new();
        merge_batch_with_id(
            "sandbox",
            Some("B7_sandbox"),
            json!({"nest_residual_algo": "gr_residual_v2"})
                .as_object()
                .cloned()
                .unwrap(),
            &mut fields,
            &mut fbs,
            &mut gw,
            &mut cf,
        );
        assert_eq!(
            fields.get("sandbox_residual_algo_match").unwrap(),
            &json!(true),
            "same algo family confirms: {fields:?}"
        );
        // Diverging sandbox algo → false
        let (mut fields2, _) = merge_one(
            "main",
            "B10_hw_curves",
            json!({"residual_algo": "gr_residual_v2"}),
        );
        let mut fbs2 = Map::new();
        let mut gw2 = Map::new();
        let mut cf2 = Map::new();
        merge_batch_with_id(
            "sandbox",
            Some("B7_sandbox"),
            json!({"nest_residual_algo": "gr_residual_v1"})
                .as_object()
                .cloned()
                .unwrap(),
            &mut fields2,
            &mut fbs2,
            &mut gw2,
            &mut cf2,
        );
        assert_eq!(
            fields2.get("sandbox_residual_algo_match").unwrap(),
            &json!(false)
        );
        // No main algo → no claim (fields absent, not false)
        let (fields3, _) = merge_one(
            "sandbox",
            "B7_sandbox",
            json!({"nest_residual_algo": "gr_residual_v1"}),
        );
        assert!(
            !fields3.contains_key("sandbox_residual_algo_match"),
            "missing main algo must not stamp: {fields3:?}"
        );
    }

    /// A5: FE freezeCapture stamp drift inside a session.
    #[test]
    fn fe_version_drift_and_build_mismatch_flags() {
        // First main batch stamps fe_code_version
        let (mut fields, _) = merge_one(
            "main",
            "B0_bootstrap",
            json!({"fe_code_version": "7.0.0", "fe_build_impl": "bld_a", "fe_impl_version": "bld_a"}),
        );
        assert_eq!(fields.get("fe_code_version").unwrap(), &json!("7.0.0"));
        assert!(!fields.contains_key("fe_version_changed"));
        assert!(!fields.contains_key("fe_build_mismatch"));

        // Second batch with the same version → no drift
        let mut fbs = Map::new();
        let mut gw = Map::new();
        let mut cf = Map::new();
        merge_batch_with_id(
            "main",
            Some("B10_hw_curves"),
            json!({"fe_code_version": "7.0.0"})
                .as_object()
                .cloned()
                .unwrap(),
            &mut fields,
            &mut fbs,
            &mut gw,
            &mut cf,
        );
        assert!(!fields.contains_key("fe_version_changed"));

        // Mid-session bundle change → drift flag
        merge_batch_with_id(
            "main",
            Some("B2_texture_caps"),
            json!({"fe_code_version": "7.0.1-canary"})
                .as_object()
                .cloned()
                .unwrap(),
            &mut fields,
            &mut fbs,
            &mut gw,
            &mut cf,
        );
        assert_eq!(fields.get("fe_version_changed").unwrap(), &json!(true));

        // Build stamp vs its own FE epoch mismatch
        let (fields2, _) = merge_one(
            "main",
            "B0_bootstrap",
            json!({"fe_build_impl": "bld_x", "fe_impl_version": "bld_y"}),
        );
        assert_eq!(fields2.get("fe_build_mismatch").unwrap(), &json!(true));
    }
}
