//! Session cool-down tickets (G-PROD-8 / iss/07).
//!
//! After **silicon-grade** session materials are present (residual and/or B10 curves),
//! issue a short-lived ticket so reloads can skip session probe packs while
//! **page-level rpa** still runs.
//!
//! Hard rules (v5.8.48+):
//! - No residual / B10 silicon → **never** issue or honor skip_session_probe.
//! - force_identity / product_version change → ticket invalid (caller must not skip).
//! - Tickets are field-driven fingerprints — never host/env gold.

use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// Session cool-down ticket TTL: skip session-level re-probe while materials hold.
/// Aligned with product: session-level analysis valid **24h**; page rpa always required.
pub const TICKET_TTL_MS: i64 = 24 * 60 * 60 * 1000; // 24 hours

/// Ticket algo id — v5 = server-keyed HMAC-SHA256 (iss/opus5 §2.3).
pub const SESSION_TICKET_ALGO: &str = "session_ticket_v5_hmac";

/// Resolve the server-side ticket signing key (iss/opus5 §2.3 P1).
///
/// Chain: `GR_TICKET_SECRET` → `GR_SEAL_SECRET` → `GR_CHALLENGE_SECRET`.
/// Non-prod falls back to the lab challenge constant (mirrors challenge-secret
/// resolution); prod refuses — callers must treat `None` as "do not issue /
/// reject ticket".
fn ticket_key() -> Option<Vec<u8>> {
    for var in ["GR_TICKET_SECRET", "GR_SEAL_SECRET", "GR_CHALLENGE_SECRET"] {
        if let Ok(v) = std::env::var(var) {
            let t = v.trim();
            if !t.is_empty() {
                return Some(t.as_bytes().to_vec());
            }
        }
    }
    let deploy = gr_abi::env::get("DEPLOY_ENV")
         .or_else(|| gr_abi::env::get("DEPLOY_ENV"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    let prod = matches!(deploy.as_str(), "prod" | "production" | "live");
    if prod {
        return None;
    }
    Some(crate::challenge_pow::DEFAULT_CHALLENGE_SECRET
        .as_bytes()
        .to_vec())
}

/// Keyed ticket token: `st_` + truncated hex HMAC-SHA256(key, raw).
fn ticket_token(key: &[u8], raw: &str) -> String {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key).expect("hmac accepts any key length");
    mac.update(raw.as_bytes());
    let hex = format!("{:x}", mac.finalize().into_bytes());
    format!("st_{}", &hex[..25])
}

/// Constant-time-ish token comparison (iss/opus5 S-17: remote timing).
pub fn token_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.bytes().zip(b.bytes()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Max age (ms) of a cycle without B10 while gaps still demand it before SLA fire.
/// B10 hard-anchor SLA grace. Was 8s — short dwell + multipath + visitor network
/// blips routinely miss B10 and fire `b10_sla_miss` with zero later land. 15s keeps
/// pressure on FE hard_sla kicks while reducing false terminal errors.
pub const B10_SLA_MS: i64 = 15_000;
/// Extra grace when the session still has recent batch activity (upload in flight).
pub const B10_SLA_ACTIVE_UPLOAD_GRACE_MS: i64 = 8_000;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[allow(dead_code)] // kept: algorithm reference / .so-module variant (iss/audit WARN-01: silence, do not remove)
fn non_empty_str(v: Option<&Value>) -> bool {
    match v {
        Some(Value::String(s)) => !s.trim().is_empty() && s != "null" && s != "undefined",
        Some(Value::Number(_)) => true,
        Some(Value::Bool(true)) => true,
        Some(Value::Object(o)) => !o.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        _ => false,
    }
}

fn num_ok(v: Option<&Value>) -> bool {
    v.and_then(|x| x.as_f64())
        .map(|n| n.is_finite())
        .unwrap_or(false)
}

fn curve_len_ok(v: Option<&Value>, min: usize) -> bool {
    match v {
        Some(Value::Array(a)) => a.len() >= min,
        Some(Value::String(s)) => {
            // Non-empty digest/token is not a full curve — do not count as B10 silicon.
            let t = s.trim();
            !t.is_empty() && t != "null" && t != "undefined" && t.starts_with('[') && t.len() > 20
        }
        _ => false,
    }
}

/// Silicon materials for commercial cool / skip.
///
/// Hard rule (178 Firefox multi-site): **residual mean/std AND primary webgl curve**.
/// - B10x-only residual without primary curve must NOT cool.
/// - webrtc / cpu alone must NEVER cool (that skipped B10_hw_curves re-upload).
pub fn has_silicon_materials(fields: &Value) -> bool {
    let fo = match fields.as_object() {
        Some(o) => o,
        None => return false,
    };
    let residual = num_ok(fo.get("residual_std"))
        || num_ok(fo.get("residual_mean"))
        || fo.get("residual_ok").and_then(|v| v.as_bool()) == Some(true);
    // Primary commercial webgl curve (B10_hw_curves). Multipath array also counts.
    let webgl = curve_len_ok(fo.get("hw_curve_webgl"), 8)
        || curve_len_ok(fo.get("webgl_residual_multipath"), 8)
        || fo
            .get("residual_paths")
            .and_then(|v| v.as_array())
            .is_some_and(|a| {
                a.iter().any(|p| {
                    p.get("curve")
                        .and_then(|c| c.as_array())
                        .is_some_and(|c| c.len() >= 8)
                        && p.get("ok").and_then(|o| o.as_bool()).unwrap_or(true)
                })
            });
    residual && webgl
}

/// True when **primary** B10 pack landed (`B10_hw_curves` / legacy `mid.curves`).
///
/// **Bugfix (6.0.5)**: previous `starts_with("B10_")` treated `B10x_*` deepen packs
/// as primary B10 → cool/halt without ever requiring `B10_hw_curves` (searchchina case).
pub fn evidence_has_b10(evidence: &Value) -> bool {
    let is_primary = |id: &str| id == "B10_hw_curves" || id == "mid.curves";
    if let Some(arr) = evidence.get("batches").and_then(|v| v.as_array()) {
        for b in arr {
            let id = b
                .get("batch_id")
                .or_else(|| b.get("pack_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if is_primary(id) {
                return true;
            }
        }
    }
    if let Some(rec) = evidence.get("received_batches").and_then(|v| v.as_array()) {
        for b in rec {
            let id = b
                .as_str()
                .or_else(|| b.get("batch_id").and_then(|v| v.as_str()))
                .unwrap_or("");
            if is_primary(id) {
                return true;
            }
        }
    }
    // Do **not** infer primary B10 from fields alone — B10x can write residual/curves
    // without the primary pack, which previously caused false cool.
    false
}

/// Gaps / battle demand B10 before commercial finalize.
pub fn gaps_need_b10(evidence: &Value) -> bool {
    let check = |v: &Value| -> bool {
        if let Some(s) = v.as_str() {
            return s.contains("need_b10")
                || s.contains("b10_curves")
                || s == "need_device_hard_anchor"
                || s == "stack_residual_missing";
        }
        if let Some(o) = v.as_object() {
            if let Some(c) = o.get("code").and_then(|x| x.as_str()) {
                return c.contains("need_b10")
                    || c.contains("b10_curves")
                    || c == "need_device_hard_anchor"
                    || c == "stack_residual_missing";
            }
        }
        false
    };
    for key in ["gaps", "frontier_gaps", "gaps_selected"] {
        if let Some(arr) = evidence.get(key).and_then(|v| v.as_array()) {
            if arr.iter().any(check) {
                return true;
            }
        }
    }
    if let Some(arr) = evidence
        .pointer("/meta/battle_log")
        .and_then(|v| v.as_array())
        .or_else(|| {
            evidence
                .pointer("/meta/battle_log/gaps_selected")
                .and_then(|v| v.as_array())
        })
    {
        // battle_log may be array of entries or a single object with gaps_selected
        for e in arr {
            if check(e) {
                return true;
            }
            if let Some(gs) = e.get("gaps_selected").and_then(|v| v.as_array()) {
                if gs.iter().any(check) {
                    return true;
                }
            }
        }
    }
    if let Some(gs) = evidence
        .pointer("/meta/battle_log/gaps_selected")
        .and_then(|v| v.as_array())
    {
        if gs.iter().any(check) {
            return true;
        }
    }
    // Device ineligible / empty_anchor without silicon still needs B10.
    let digest = evidence
        .pointer("/device/digest_path")
        .or_else(|| evidence.get("digest_path"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if digest.contains("empty_anchor") || digest.contains("gateway_only") {
        return true;
    }
    false
}

fn material_fingerprint(fields: &Value, device_id: Option<&str>) -> String {
    silicon_materials_fingerprint(fields, device_id)
}

/// Commercial multipath / Lane-C digest algorithm generation.
/// Bump when `device_segments` commercial body rules change so cool tickets re-probe.
/// v10: cool requires primary B10_hw_curves (B10x alone must not cool).
pub const COMMERCIAL_SEG_ALGO: &str = "lane_c_multipath_sig_v10_primary_b10";

/// Public fingerprint of **silicon / identity materials** used to skip re-analyze
/// when only soft interaction packs arrive (shared-algorithm: avoid wasted evaluate).
pub fn silicon_materials_fingerprint(fields: &Value, device_id: Option<&str>) -> String {
    let mut h = Sha256::new();
    // v4: bind commercial segment algo so cool tickets die when Lane-C mint changes (iss/73).
    h.update(b"gr_silicon_fp_v4|");
    h.update(COMMERCIAL_SEG_ALGO.as_bytes());
    h.update(b"|");
    if let Some(d) = device_id {
        h.update(d.as_bytes());
    }
    // Prefer stable digests + multipath fuse over raw curves (cheaper, shared with mint).
    let keys = [
        "form_class",
        "hardware_concurrency",
        "device_memory",
        "timezone",
        "platform",
        "os_family",
        "residual_std",
        "residual_mean",
        "hw_curve_webgl",
        "hw_curve_audio",
        "hw_curve_canvas",
        "hw_webgl_stable",
        "hw_audio_stable",
        "hw_canvas_stable",
        "residual_paths",
        "stack_class",
        "webgl_unmasked_renderer",
        "webrtc_host_ip_hash_v2",
        "webrtc_host_ip_hash",
        "ja4h_lite",
        "ja4l_lite",
        "os_instance_hash",
        "product_version",
    ];
    if let Some(fo) = fields.as_object() {
        for k in keys {
            h.update(k.as_bytes());
            h.update(b"=");
            if let Some(v) = fo.get(k) {
                // Cap huge curve dumps in fingerprint input
                let s = v.to_string();
                if s.len() > 512 {
                    h.update(&s.as_bytes()[..256]);
                    h.update(format!("…{}", s.len()).as_bytes());
                } else {
                    h.update(s.as_bytes());
                }
            }
            h.update(b"|");
        }
    }
    format!("{:x}", h.finalize())[..24].to_string()
}

/// Product score breadth alone is **not** enough — silicon materials required.
/// Caller must also gate on [`evidence_has_b10`] when evidence is available (cool ticket).
pub fn session_probe_sufficient(product: &Value, fields: &Value) -> bool {
    if !has_silicon_materials(fields) {
        return false;
    }
    let os_cov = product
        .pointer("/os/coverage")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let br_cov = product
        .pointer("/br/coverage")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let os_s = product
        .pointer("/os/status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let br_s = product
        .pointer("/br/status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let os_hits = product
        .pointer("/os/field_hits")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let br_hits = product
        .pointer("/br/field_hits")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let breadth = os_cov >= 0.12
        || br_cov >= 0.12
        || os_hits >= 3
        || br_hits >= 3
        || (os_s != "unknown" && br_s != "unknown");
    breadth && !(os_s == "unknown" && br_s == "unknown" && os_hits == 0 && br_hits == 0)
}

/// Issue cool-down ticket from evaluate product + fields.
/// Returns None when silicon materials are missing (no skip_session_probe).
pub fn issue_session_ticket(
    session_id: &str,
    fields: &Value,
    product: &Value,
    now: Option<i64>,
) -> Option<Value> {
    issue_session_ticket_versioned(session_id, fields, product, now, None)
}

/// Version-aware ticket issue (bind product_version for force/version invalidate).
///
/// `primary_b10`: must be true when evidence includes `B10_hw_curves`. Without it,
/// never issue cool ticket (B10x-only sessions must keep probing).
pub fn issue_session_ticket_versioned(
    session_id: &str,
    fields: &Value,
    product: &Value,
    now: Option<i64>,
    product_version: Option<&str>,
) -> Option<Value> {
    issue_session_ticket_versioned_ex(session_id, fields, product, now, product_version, true)
}

/// Extended issue: set `require_primary_b10=false` only for unit tests of fingerprint shape.
pub fn issue_session_ticket_versioned_ex(
    session_id: &str,
    fields: &Value,
    product: &Value,
    now: Option<i64>,
    product_version: Option<&str>,
    require_primary_b10_marker: bool,
) -> Option<Value> {
    if !session_probe_sufficient(product, fields) {
        return None;
    }
    // Production cool requires primary B10 marker on ticket; fields alone (B10x) insufficient.
    // When require_primary_b10_marker is true, caller must only invoke after evidence_has_b10.
    // We stamp primary_b10=true always when issuing; open-path validate rejects tickets
    // without this flag (kills pre-fix cool tickets that skipped B10_hw_curves).
    let ts = now.unwrap_or_else(now_ms);
    let fp = material_fingerprint(fields, None);
    let ver = product_version
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            product
                .get("product_version")
                .or_else(|| product.get("version"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_default();
    // iss/opus5 §2.3: ticket token must be a server-keyed MAC — the previous
    // keyless SHA256 (never even re-computed at validate time) let clients
    // forge cool-down tickets. No key → refuse to issue.
    let key = ticket_key()?;
    let raw = format!("{session_id}|{fp}|{ver}|primary_b10|{ts}");
    let token = ticket_token(&key, &raw);
    let _ = require_primary_b10_marker; // documented contract; caller gates with evidence_has_b10
    Some(json!({
        "ticket": token,
        "session_id": session_id,
        "issued_ms": ts,
        "exp_ms": ts + TICKET_TTL_MS,
        "material_fingerprint": fp,
        "product_version": ver,
        "commercial_seg_algo": COMMERCIAL_SEG_ALGO,
        "silicon_ok": true,
        "primary_b10": true,
        "skip_session_probe": true,
        "page_probe_required": true,
        "note": "session silicon cooled after B10_hw_curves; page rpa must still run",
        "algo": SESSION_TICKET_ALGO,
    }))
}

/// Validate ticket. `opts.force` / version mismatch / missing silicon flag → false.
pub fn validate_session_ticket(
    ticket: &Value,
    session_id: &str,
    fields: Option<&Value>,
    now: Option<i64>,
) -> bool {
    validate_session_ticket_ex(ticket, session_id, fields, now, None, false)
}

/// Extended validate: product_version + force_identity kill switch.
pub fn validate_session_ticket_ex(
    ticket: &Value,
    session_id: &str,
    fields: Option<&Value>,
    now: Option<i64>,
    current_product_version: Option<&str>,
    force_identity: bool,
) -> bool {
    if force_identity {
        return false;
    }
    // iss/73: reject cool tickets issued under a prior commercial segment algo
    // (e.g. pre–Lane-C multipath) so sticky digests cannot mask mint fixes.
    let ticket_seg = ticket
        .get("commercial_seg_algo")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if ticket_seg != COMMERCIAL_SEG_ALGO {
        return false;
    }
    // v5 (iss/opus5 §2.3): reject anything not MAC-bound to the server key —
    // pre-v5 tickets used a keyless digest that was never even re-computed at
    // validation time, so any client could forge a cool-down ticket.
    let primary_b10 = ticket
        .get("primary_b10")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let algo = ticket.get("algo").and_then(|v| v.as_str()).unwrap_or("");
    if !primary_b10 || algo != SESSION_TICKET_ALGO {
        return false;
    }
    let tok = ticket.get("ticket").and_then(|v| v.as_str()).unwrap_or("");
    if !tok.starts_with("st_") || tok.len() < 10 {
        return false;
    }
    // Recompute the server-keyed MAC over the ticket's own claims and compare.
    let Some(key) = ticket_key() else {
        return false; // no server key → fail closed
    };
    let t_sid = ticket
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let t_fp = ticket
        .get("material_fingerprint")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let t_ver = ticket
        .get("product_version")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let t_issued = ticket.get("issued_ms").and_then(|v| v.as_i64()).unwrap_or(0);
    if t_sid.is_empty() || t_issued <= 0 {
        return false;
    }
    let raw = format!("{t_sid}|{t_fp}|{t_ver}|primary_b10|{t_issued}");
    let expected = ticket_token(&key, &raw);
    if !token_eq(&expected, tok) {
        return false;
    }
    let sid = ticket
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !session_id.is_empty() && sid != session_id {
        return false;
    }
    let exp = ticket.get("exp_ms").and_then(|v| v.as_i64()).unwrap_or(0);
    let ts = now.unwrap_or_else(now_ms);
    if exp > 0 && ts > exp {
        return false;
    }
    if !ticket
        .get("skip_session_probe")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return false;
    }
    // v2 tickets must declare silicon; legacy tickets without silicon_ok are rejected.
    let silicon_flag = ticket.get("silicon_ok").and_then(|v| v.as_bool());
    if silicon_flag != Some(true) {
        // Allow legacy only if fields prove silicon right now.
        match fields {
            Some(f) if has_silicon_materials(f) => {}
            _ => return false,
        }
    }
    if let Some(cur) = current_product_version.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        let ticket_ver = ticket
            .get("product_version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if !ticket_ver.is_empty() && ticket_ver != cur {
            return false;
        }
    }
    if let Some(f) = fields {
        if !has_silicon_materials(f) {
            // Page-only reload: fields may be sparse — allow when ticket silicon_ok and
            // no rich mismatch. If FE sends full fields without silicon, reject.
            let fo = f.as_object();
            let rich = fo
                .map(|o| {
                    o.contains_key("hw_curve_webgl")
                        || o.contains_key("residual_std")
                        || o.contains_key("form_class")
                        || o.contains_key("webgl_unmasked_renderer")
                })
                .unwrap_or(false);
            if rich {
                return false;
            }
        }
        let fo = f.as_object();
        let rich = fo
            .map(|o| {
                o.contains_key("hw_curve_webgl")
                    || o.contains_key("form_class")
                    || o.contains_key("webgl_unmasked_renderer")
                    || o.contains_key("residual_std")
            })
            .unwrap_or(false);
        if rich {
            if let Some(fp) = ticket.get("material_fingerprint").and_then(|v| v.as_str()) {
                let cur = material_fingerprint(f, None);
                if fp.len() >= 8 && cur.len() >= 8 && &fp[..8] != &cur[..8] {
                    return false;
                }
            }
        }
    }
    true
}

/// Whether brain should skip session-level packs (only page/rpa remain).
/// Force / missing silicon / invalid ticket → never skip.
pub fn should_skip_session_probe(evidence: &Value) -> bool {
    if evidence
        .get("force_identity")
        .or_else(|| evidence.get("force_identity_probe"))
        .or_else(|| evidence.pointer("/meta/force_identity"))
        .or_else(|| evidence.pointer("/meta/force_identity_probe"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return false;
    }
    if evidence
        .get("force_reprobe")
        .or_else(|| evidence.pointer("/meta/force_reprobe"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return false;
    }
    let cur_ver = evidence
        .get("product_version")
        .or_else(|| evidence.pointer("/meta/product_version"))
        .and_then(|v| v.as_str());

    // Explicit meta skip only honored when silicon present on fields.
    if evidence
        .pointer("/meta/skip_session_probe")
        .or_else(|| evidence.get("skip_session_probe"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        if let Some(f) = evidence.get("fields") {
            if has_silicon_materials(f) {
                // still require ticket validity when present
                if let Some(t) = evidence
                    .get("session_ticket")
                    .or_else(|| evidence.pointer("/meta/session_ticket"))
                {
                    let sid = evidence
                        .get("session_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    return validate_session_ticket_ex(
                        t,
                        sid,
                        Some(f),
                        None,
                        cur_ver,
                        false,
                    );
                }
                return true;
            }
        }
        return false;
    }

    if let Some(t) = evidence
        .get("session_ticket")
        .or_else(|| evidence.pointer("/meta/session_ticket"))
    {
        let sid = evidence
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        return validate_session_ticket_ex(
            t,
            sid,
            evidence.get("fields"),
            None,
            cur_ver,
            false,
        );
    }
    false
}

/// B10 SLA: need curves, no B10 yet, cycle old enough → force schedule + ops code.
///
/// Does **not** fire while uploads are still landing (recent batch / last_upload within
/// grace) — those are mid-flight, not permanent misses. Force-schedule still happens via
/// brain when gaps_need_b10; this gate only controls the **error** ops severity path.
pub fn b10_sla_violation(evidence: &Value, now: Option<i64>) -> Option<&'static str> {
    if evidence_has_b10(evidence) {
        return None;
    }
    if should_skip_session_probe(evidence) {
        // Silicon already cooled — B10 not required on page-only pass.
        return None;
    }
    let need = gaps_need_b10(evidence)
        || evidence
            .get("fields")
            .map(|f| !has_silicon_materials(f))
            .unwrap_or(true);
    if !need {
        return None;
    }
    let ts = now.unwrap_or_else(now_ms);
    let created = evidence
        .get("created_ms")
        .or_else(|| evidence.pointer("/meta/created_ms"))
        .or_else(|| evidence.pointer("/session/created_ms"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if created > 0 && ts.saturating_sub(created) < B10_SLA_MS {
        return None; // grace window
    }
    // Active upload grace: any recent batch or last_upload → still probing, not a miss.
    let last_upload = evidence
        .get("last_upload_ms")
        .or_else(|| evidence.pointer("/meta/last_upload_ms"))
        .or_else(|| evidence.pointer("/session/last_upload_ms"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let mut latest_batch_ms = last_upload;
    if let Some(arr) = evidence.get("batches").and_then(|v| v.as_array()) {
        for b in arr {
            let bm = b
                .get("created_ms")
                .or_else(|| b.get("recv_ms"))
                .or_else(|| b.get("ts_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            if bm > latest_batch_ms {
                latest_batch_ms = bm;
            }
        }
    }
    if latest_batch_ms > 0
        && ts.saturating_sub(latest_batch_ms) < B10_SLA_ACTIVE_UPLOAD_GRACE_MS
    {
        return None;
    }
    // If we already have many revs / analyze rounds, fire immediately.
    let rev = evidence
        .get("analysis_rev")
        .or_else(|| evidence.pointer("/meta/analysis_rev"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if created == 0 && rev < 3 {
        return None;
    }
    Some("b10_sla_miss")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b10_sla_respects_grace_and_active_upload() {
        let t0 = 1_000_000_i64;
        // Within base grace → no miss
        let young = json!({
            "created_ms": t0,
            "fields": {"platform": "Linux"},
            "batches": []
        });
        assert!(b10_sla_violation(&young, Some(t0 + 5_000)).is_none());
        // Past grace, no batches → miss
        assert_eq!(
            b10_sla_violation(&young, Some(t0 + B10_SLA_MS + 1)),
            Some("b10_sla_miss")
        );
        // Past grace but recent batch activity → still probing, no miss
        let active = json!({
            "created_ms": t0,
            "fields": {"platform": "Linux"},
            "batches": [{"batch_id": "B0_bootstrap", "created_ms": t0 + B10_SLA_MS + 2_000}]
        });
        assert!(b10_sla_violation(
            &active,
            Some(t0 + B10_SLA_MS + 2_000 + B10_SLA_ACTIVE_UPLOAD_GRACE_MS - 1)
        )
        .is_none());
        // Recent activity aged out → miss
        assert_eq!(
            b10_sla_violation(
                &active,
                Some(t0 + B10_SLA_MS + 2_000 + B10_SLA_ACTIVE_UPLOAD_GRACE_MS + 1)
            ),
            Some("b10_sla_miss")
        );
        // Has B10 → never miss
        let with_b10 = json!({
            "created_ms": t0,
            "batches": [{"batch_id": "B10_hw_curves", "created_ms": t0 + 100}]
        });
        assert!(b10_sla_violation(&with_b10, Some(t0 + 60_000)).is_none());
    }

    #[test]
    fn no_ticket_without_silicon() {
        let fields = json!({"platform": "Linux", "form_class": "desktop"});
        let product = json!({
            "os": {"coverage": 0.5, "status": "clear", "field_hits": ["a","b","c"]},
            "br": {"coverage": 0.5, "status": "clear", "field_hits": ["a","b","c"]},
        });
        assert!(!session_probe_sufficient(&product, &fields));
        assert!(issue_session_ticket("s1", &fields, &product, Some(1_000_000)).is_none());
    }

    fn sample_webgl_curve() -> Vec<f64> {
        (0..16).map(|i| 0.2 + i as f64 * 0.01).collect()
    }

    #[test]
    fn b10x_alone_is_not_primary_b10() {
        let ev = json!({
            "batches": [
                {"batch_id": "B10x_silicon_noderiv"},
                {"batch_id": "B10x_silicon_rint"},
                {"batch_id": "B10x_silicon_ulp"}
            ],
            "fields": {
                "residual_mean": 0.26,
                "hw_curve_webgl": sample_webgl_curve()
            }
        });
        assert!(
            !evidence_has_b10(&ev),
            "B10x packs must not count as primary B10_hw_curves"
        );
        let with_primary = json!({
            "batches": [
                {"batch_id": "B10_hw_curves"},
                {"batch_id": "B10x_silicon_noderiv"}
            ]
        });
        assert!(evidence_has_b10(&with_primary));
    }

    #[test]
    fn webrtc_alone_is_not_silicon_for_cool() {
        let fields = json!({
            "webrtc_host_ip_hash": "abc123",
            "platform": "Linux"
        });
        assert!(!has_silicon_materials(&fields));
        let product = json!({
            "os": {"coverage": 0.5, "status": "clear", "field_hits": ["a","b","c"]},
            "br": {"coverage": 0.5, "status": "clear", "field_hits": ["a","b","c"]},
        });
        assert!(!session_probe_sufficient(&product, &fields));
    }

    #[test]
    fn ticket_with_residual() {
        let fields = json!({
            "residual_std": 0.09,
            "residual_mean": 0.26,
            "platform": "Linux",
            "form_class": "desktop",
            "hw_curve_webgl": sample_webgl_curve()
        });
        let product = json!({
            "os": {"coverage": 0.5, "status": "clear", "field_hits": ["a","b","c"]},
            "br": {"coverage": 0.5, "status": "clear", "field_hits": ["a","b","c"]},
        });
        let t = issue_session_ticket_versioned(
            "s1",
            &fields,
            &product,
            Some(1_000_000),
            Some("6.0.5"),
        )
        .expect("ticket");
        assert_eq!(
            t.get("algo").and_then(|v| v.as_str()),
            Some(SESSION_TICKET_ALGO)
        );
        assert_eq!(t.get("primary_b10").and_then(|v| v.as_bool()), Some(true));
        assert!(validate_session_ticket_ex(
            &t,
            "s1",
            Some(&fields),
            Some(1_000_100),
            Some("6.0.5"),
            false
        ));
        // iss/opus5 §2.3: tampering with any signed claim kills the ticket.
        let mut forged = t.clone();
        forged
            .as_object_mut()
            .unwrap()
            .insert("session_id".into(), json!("s_attacker"));
        assert!(!validate_session_ticket_ex(
            &forged,
            "s_attacker",
            Some(&fields),
            Some(1_000_100),
            Some("6.0.5"),
            false
        ));
        // Keyless-style forged token (correct shape, no MAC) must fail.
        let mut keyless = t.clone();
        keyless.as_object_mut().unwrap().insert(
            "ticket".into(),
            json!("st_0000000000000000000000000"),
        );
        assert!(!validate_session_ticket_ex(
            &keyless,
            "s1",
            Some(&fields),
            Some(1_000_100),
            Some("6.0.5"),
            false
        ));
        // version change kills
        assert!(!validate_session_ticket_ex(
            &t,
            "s1",
            Some(&fields),
            Some(1_000_100),
            Some("6.0.6"),
            false
        ));
        // force kills
        assert!(!validate_session_ticket_ex(
            &t,
            "s1",
            Some(&fields),
            Some(1_000_100),
            Some("6.0.5"),
            true
        ));
    }

    #[test]
    fn force_blocks_skip() {
        let fields = json!({
            "residual_std": 0.1,
            "residual_mean": 0.26,
            "hw_curve_webgl": sample_webgl_curve()
        });
        let product = json!({
            "os": {"status": "clear", "field_hits": ["a","b","c"]},
            "br": {"status": "clear", "field_hits": ["a","b","c"]},
        });
        let t = issue_session_ticket("s1", &fields, &product, Some(1)).unwrap();
        let ev = json!({
            "session_id": "s1",
            "fields": fields,
            "session_ticket": t,
            "force_identity": true,
        });
        assert!(!should_skip_session_probe(&ev));
    }

    #[test]
    fn cool_ticket_rejects_prior_commercial_seg_algo() {
        let fields = json!({
            "residual_std": 0.09,
            "residual_mean": 0.26,
            "platform": "Linux",
            "form_class": "desktop",
            "hw_curve_webgl": sample_webgl_curve()
        });
        let product = json!({
            "os": {"coverage": 0.5, "status": "clear", "field_hits": ["a","b","c"]},
            "br": {"coverage": 0.5, "status": "clear", "field_hits": ["a","b","c"]},
        });
        let mut t = issue_session_ticket_versioned(
            "s1",
            &fields,
            &product,
            Some(1_000_000),
            Some("6.0.5"),
        )
        .expect("ticket");
        assert_eq!(
            t.get("commercial_seg_algo").and_then(|v| v.as_str()),
            Some(COMMERCIAL_SEG_ALGO)
        );
        assert_eq!(
            t.get("algo").and_then(|v| v.as_str()),
            Some(SESSION_TICKET_ALGO)
        );
        // Legacy cool ticket without commercial_seg_algo must not skip remint.
        t.as_object_mut()
            .unwrap()
            .remove("commercial_seg_algo");
        assert!(!validate_session_ticket_ex(
            &t,
            "s1",
            Some(&fields),
            Some(1_000_100),
            Some("6.0.5"),
            false
        ));
        // Wrong algo generation likewise rejected.
        t.as_object_mut().unwrap().insert(
            "commercial_seg_algo".into(),
            json!("pre_lane_c_v2"),
        );
        assert!(!validate_session_ticket_ex(
            &t,
            "s1",
            Some(&fields),
            Some(1_000_100),
            Some("v5.8.159"),
            false
        ));
    }
}
