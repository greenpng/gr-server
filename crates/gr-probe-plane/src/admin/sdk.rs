//! Per-site FE embed + backend SDK keys.

use crate::admin::db::AdminDb;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub fn hash_secret(raw: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"gr_sdk_v1|");
    h.update(raw.as_bytes());
    hex::encode(h.finalize())
}

pub fn mint_raw_secret() -> String {
    use rand::RngCore;
    let mut b = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut b);
    format!("grsk_{}", hex::encode(b))
}

pub fn create_key(
    db: &AdminDb,
    site_id: &str,
    kind: &str,
    allowed_origins: Vec<String>,
) -> Result<Value, String> {
    if db.get_site(site_id)?.is_none() {
        return Err("site_not_found".into());
    }
    let kind = match kind {
        "fe_embed" | "backend" => kind,
        _ => return Err("kind must be fe_embed|backend".into()),
    };
    let raw = mint_raw_secret();
    let prefix: String = raw.chars().take(12).collect();
    let hash = hash_secret(&raw);
    let mut row = db.insert_sdk_key(site_id, kind, &hash, &prefix, &allowed_origins)?;
    if let Some(obj) = row.as_object_mut() {
        obj.insert("secret".into(), json!(raw));
        obj.insert(
            "note".into(),
            json!("secret shown once — store securely"),
        );
    }
    db.audit("system", "sdk_create", site_id, json!({"kind": kind}));
    Ok(row)
}

pub fn rotate_key(
    db: &AdminDb,
    key_id: &str,
    site_id: &str,
    kind: &str,
    allowed_origins: Vec<String>,
) -> Result<Value, String> {
    let _ = db.revoke_sdk_key(key_id);
    create_key(db, site_id, kind, allowed_origins)
}

/// Resolved FE edge endpoints for a site row.
///
/// Product axes (panel, 2026-09):
/// - **fe_load**: `pv` | `first_party` | `gv` — where the probe script is loaded
/// - **upload_ingest**: `gv` | `pv` | `first_party` — open/ingest target
///   (`first_party` = proxied upload, rejected unless GR_ALLOW_FIRST_PARTY_UPLOAD)
/// - **gv**: absolute HTTPS Pingora — upload + B8 (no reverse proxy)
/// - **poll_method**: get | post | both
pub fn resolve_edge_bases(site: &Value) -> Value {
    let fe = crate::admin::db::normalize_fe_load(
        site.get("fe_load")
            .and_then(|v| v.as_str())
            .unwrap_or("pv"),
    );
    let up = crate::admin::db::normalize_upload_ingest(
        site.get("upload_ingest")
            .and_then(|v| v.as_str())
            .unwrap_or("gv"),
    );
    let poll = crate::admin::db::normalize_poll_method(
        site.get("poll_method")
            .and_then(|v| v.as_str())
            .unwrap_or("both"),
    );
    let mode = crate::admin::db::normalize_edge_mode(
        site.get("edge_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("dual_domain"),
    );
    let pv = site
        .get("pv_base")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .trim_end_matches('/')
        .to_string();
    let gv = site
        .get("gv_base")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .trim_end_matches('/')
        .to_string();

    let gw_base = if !gv.is_empty() {
        gv.clone()
    } else {
        String::new()
    };
    let gv_ok = crate::admin::db::is_absolute_https(&gw_base);

    let api_base = if up == "pv" && !pv.is_empty() {
        pv.clone()
    } else if up == "first_party" {
        String::new()
    } else if gv_ok {
        gw_base.clone()
    } else {
        String::new()
    };
    let script_base = if fe == "gv" && gv_ok {
        gw_base.clone()
    } else if fe == "pv" && !pv.is_empty() {
        pv.clone()
    } else if fe == "first_party" {
        String::new()
    } else {
        pv.clone()
    };
    let upload_on_gv = gv_ok && up != "first_party" && up != "pv";
    let unified_gv = upload_on_gv || (up == "gv" && gv_ok);
    let proxied = up == "first_party";

    json!({
        "edge_mode": if fe == "pv" || up == "gv" { "dual_domain".to_string() } else { mode },
        "fe_load": fe,
        "upload_ingest": if unified_gv && up != "pv" && up != "first_party" { "gv".to_string() } else { up.clone() },
        "poll_method": poll,
        "api_base": api_base,
        "script_base": script_base,
        "gw_base": gw_base,
        "pv_base": pv,
        "gv_base": gv,
        "gv_required": true,
        "gv_ssl_required": true,
        "gv_pingora_ready": gv_ok,
        "gv_nginx_forbidden": true,
        "proxied_upload": proxied,
        "unified_gv_upload": unified_gv,
        "upload_channels": {
            "script_load": "pv pin+dist (direct or user reverse-proxy of /gr.js+/dist/ only)",
            "upload_and_b8": "https://gv only — open/ingest/analyses + early (Pingora TLS direct, no reverse proxy)"
        },
        "upload_path": if proxied { "proxied_forbidden" } else { "browser_direct" },
        "backend_sdk_relay_required": false,
        "cf_note": "脚本路径可反代/Worker；全部上传只走 gv 直连。",
        "note": "load=pv（或 L2 反代 /gr.js）；upload+B8=gv Pingora 直连；禁止反代 gv。",
        "cookie_fields": site.get("cookie_fields").cloned().unwrap_or_else(|| json!([])),
        "embed_token": site.get("embed_token").and_then(|v| v.as_str()).unwrap_or(""),
    })
}

/// Human deploy checklist for admin panel based on site topology.
pub fn deploy_guide(site: &Value, hostnames: &[String]) -> Value {
    let edge = resolve_edge_bases(site);
    let fe = edge.get("fe_load").and_then(|v| v.as_str()).unwrap_or("pv");
    let up = edge
        .get("upload_ingest")
        .and_then(|v| v.as_str())
        .unwrap_or("gv");
    let gv = edge.get("gw_base").and_then(|v| v.as_str()).unwrap_or("");
    let pv = edge.get("pv_base").and_then(|v| v.as_str()).unwrap_or("");
    let poll = edge
        .get("poll_method")
        .and_then(|v| v.as_str())
        .unwrap_or("both");
    let gv_ok = edge
        .get("gv_pingora_ready")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut steps: Vec<String> = vec![
        "1) 创建业务站点并开启 collect（自动生成 embed_token）".into(),
        "2) 在「域名」登记业务 www / pv / gv hostname".into(),
        "3) 配置 **gv 独立域名 + HTTPS**（Pingora TLS 直连上传+B8；灰云 DNS-only）".into(),
        format!(
            "4) 配置 pv 域名 {} 承载 /gr.js?grt=<token> 与 /dist/（默认 :8443）",
            if pv.is_empty() {
                "https://pv.example.com"
            } else {
                pv
            }
        ),
        format!(
            "5) 全部上传+B8：浏览器直连 gv={}（禁止 nginx/CF HTTP 反代）",
            if gv.is_empty() {
                "https://gv.example.com"
            } else {
                gv
            }
        ),
        "6) SDK 页按 L1–L5 复制嵌入片段（脚本 URL 已带 token）".into(),
        format!("7) poll_method={poll}"),
        "8) 后端 SDK 只消费 result，不中转探测上传".into(),
    ];
    if up == "first_party" {
        steps.insert(
            0,
            "⚠ upload_ingest=first_party 被规范拒绝：请改为 gv（或 pv）绑定域名直连".into(),
        );
    }

    let mut warnings: Vec<String> = Vec::new();
    if !gv_ok {
        warnings.push("未配置绝对 HTTPS gv — 上传与 B8 均不可用".into());
    }
    if (fe == "pv" || up == "pv") && pv.is_empty() {
        warnings.push("选择了 pv 加载/上传但未填 pv_base".into());
    }
    if hostnames.is_empty() {
        warnings.push("尚未登记业务域名 — 跨域 CORS 会失败".into());
    }
    if up == "first_party" {
        warnings.push(
            "反代上传会破坏 JA4/真实 IP；面板保存会被拦截，除非 GR_ALLOW_FIRST_PARTY_UPLOAD=1"
                .into(),
        );
    }

    json!({
        "title": "站点部署说明（按当前选择生成）",
        "fe_load": fe,
        "upload_ingest": up,
        "upload_channels": ["pv", "gv"],
        "gv_required": true,
        "gv_ssl_required": true,
        "poll_method": poll,
        "resolved": edge,
        "business_hosts": hostnames,
        "steps": steps,
        "warnings": warnings,
        "topology_ascii": format!(
            "www (page)\n  ├─ FE load: {}\n  ├─ open/ingest: {}\n  └─ B8 Pingora: {} (required HTTPS)\n",
            if fe == "pv" { pv } else { "/gr.js (same-origin proxy)" },
            if up == "pv" { pv } else if up == "gv" { gv } else { "(proxied — forbidden)" },
            if gv.is_empty() { "(missing gv)" } else { gv }
        ),
        "doc": "tasks/panel/01-deploy-spec.md"
    })
}

/// Build FE embed snippets for production installers (L1 default + L1–L5 pack).
///
/// `public_base`: script origin override (pv). Empty → site `script_base`.
/// `gw_base`: upload origin override (gv). Empty → site `gv_base`.
pub fn embed_snippet(
    db: &AdminDb,
    site_id: &str,
    public_base: &str,
    gw_base: Option<&str>,
    mode: Option<&str>,
) -> Result<Value, String> {
    let site = db
        .get_site(site_id)?
        .ok_or_else(|| "site_not_found".to_string())?;
    let hosts = db.site_hostnames(site_id)?;
    let keys = db.list_sdk_keys(Some(site_id))?;
    let fe_prefix = keys
        .iter()
        .find(|k| k.get("kind").and_then(|v| v.as_str()) == Some("fe_embed")
            && k.get("status").and_then(|v| v.as_str()) == Some("active"))
        .and_then(|k| k.get("secret_prefix").and_then(|v| v.as_str()))
        .unwrap_or("");

    let site_edge = resolve_edge_bases(&site);
    let up = site_edge
        .get("upload_ingest")
        .and_then(|v| v.as_str())
        .unwrap_or("gv");
    if up == "first_party" {
        return Err(
            "upload_ingest_first_party_forbidden: set upload_ingest=gv (or pv) for bound-domain direct"
                .into(),
        );
    }
    let site_mode = site_edge
        .get("edge_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("dual_domain");
    let site_api = site_edge
        .get("api_base")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let site_script = site_edge
        .get("script_base")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let site_gw = site_edge
        .get("gw_base")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let site_poll = site_edge
        .get("poll_method")
        .and_then(|v| v.as_str())
        .unwrap_or("both");
    let embed_token = site
        .get("embed_token")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let mode_raw = mode.unwrap_or("").trim().to_ascii_lowercase();
    let mode_label = if mode_raw.is_empty() {
        crate::admin::db::normalize_edge_mode(site_mode)
    } else {
        crate::admin::db::normalize_edge_mode(&mode_raw)
    };

    let q_base = public_base.trim().trim_end_matches('/');
    let base = if !q_base.is_empty() {
        q_base.to_string()
    } else {
        site_api.to_string()
    };
    let script_base = if !site_script.is_empty() {
        site_script.to_string()
    } else if !q_base.is_empty() {
        q_base.to_string()
    } else {
        String::new()
    };

    let gw_from_q = gw_base
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty());
    let gw_from_site = if !site_gw.is_empty() {
        Some(site_gw.to_string())
    } else {
        None
    };
    let gw_from_env = gr_abi::env::get("PUBLIC_GW_BASE")
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty());
    let gw = gw_from_q.or(gw_from_site).or(gw_from_env).unwrap_or_default();

    if !crate::admin::db::is_absolute_https(&gw) {
        return Err(
            "gw_base/gv required as absolute HTTPS (https://gv.example.com) for Pingora B8; configure site gv_base first"
                .into(),
        );
    }
    if !crate::admin::db::is_absolute_https(&base) && !base.is_empty() {
        // apiBase must be bound-domain absolute; empty means operator must fill gv.
        return Err(
            "api_base required as absolute HTTPS bound domain (upload must not be a relative reverse-proxy path)"
                .into(),
        );
    }
    let api_abs = if crate::admin::db::is_absolute_https(&base) {
        base.clone()
    } else {
        gw.clone()
    };

    let pin = if script_base.starts_with("http://") || script_base.starts_with("https://") {
        format!("{}/gr.js", script_base.trim_end_matches('/'))
    } else if script_base.is_empty() {
        "/gr.js".into()
    } else {
        format!("{}/gr.js", script_base.trim_end_matches('/'))
    };
    let pin = if embed_token.is_empty() {
        pin
    } else {
        format!("{pin}?grt={embed_token}")
    };
    let inject_path = if script_base.starts_with("http") {
        "app"
    } else {
        "nginx"
    };

    let snippet = format!(
        r#"<!-- gr FE site={site_id} · mode={mode_label} · gv Pingora required -->
<script>
(function(){{
  window.__GR_SITE_ID__={site_json};
  window.__GR_SDK_PREFIX__={prefix_json};
  window.__GR_BOOT__=window.__GR_BOOT__||{{}};
  window.__GR_BOOT__.apiBase={base_json};
  window.__GR_BOOT__.gwBase={gw_json};
  window.__GR_BOOT__.poll_method={poll_json};
  window.__GR_BOOT__.inject_path={inject_json};
  window.__GR_BOOT__.siteId={site_json};
  window.__GR_BOOT__.embed_token={tok_json};
  window.__GR_BOOT__.pin_url={pin_json};
  window.__GR_BOOT__.cookie_fields={cookies_json};
  var s=document.createElement('script');
  s.async=true;
  s.src={pin_json};
  s.dataset.siteId={site_json};
  s.dataset.endpoint={base_json};
  s.dataset.gwBase={gw_json};
  s.dataset.injectPath={inject_json};
  document.head.appendChild(s);
}})();
</script>"#,
        site_id = site_id,
        mode_label = mode_label,
        site_json = serde_json::to_string(site_id).unwrap_or_else(|_| "\"\"".into()),
        prefix_json = serde_json::to_string(fe_prefix).unwrap_or_else(|_| "\"\"".into()),
        pin_json = serde_json::to_string(&pin).unwrap_or_else(|_| "\"\"".into()),
        base_json = serde_json::to_string(&api_abs).unwrap_or_else(|_| "\"\"".into()),
        inject_json = serde_json::to_string(inject_path).unwrap_or_else(|_| "\"app\"".into()),
        gw_json = serde_json::to_string(&gw).unwrap_or_else(|_| "\"\"".into()),
        poll_json = serde_json::to_string(site_poll).unwrap_or_else(|_| "\"both\"".into()),
        tok_json = serde_json::to_string(embed_token).unwrap_or_else(|_| "\"\"".into()),
        cookies_json = serde_json::to_string(
            site.get("cookie_fields").unwrap_or(&json!([])),
        )
        .unwrap_or_else(|_| "[]".into()),
    );
    let deploy_notes = deploy_guide(&site, &hosts);
    let pack = crate::admin::deploy_center::full_deploy_pack(db, site_id).ok();

    let mut out = json!({
        "ok": true,
        "site": site,
        "hostnames": hosts,
        "mode": mode_label,
        "edge": site_edge,
        "api_base": api_abs,
        "script_base": script_base,
        "gw_base": gw,
        "poll_method": site_poll,
        "boot_url": pin,
        "pin_url": pin,
        "embed_token": embed_token,
        "inject_path": inject_path,
        "snippet": snippet,
        "backend_sdk_relay_required": false,
        "biz_visit": "POST /v1/biz/visit",
        "backend_header": "X-Gr-Sdk-Key: <backend_secret>",
        "result_read": {
            "method": "GET",
            "path": "/v1/session/<cycle_id>/result"
        },
        "gateway_early": {
            "method": "POST",
            "path": "/v1/gateway/early",
            "note": "B8 on absolute gv Pingora TLS (required)"
        },
        "note": "L1–L5 script load; upload always bound-domain direct; backend SDK does not relay probe upload."
    });
    if let Some(obj) = out.as_object_mut() {
        obj.insert("deploy_notes".into(), deploy_notes.clone());
        obj.insert("deploy_guide".into(), deploy_notes);
        if let Some(pv) = site_edge.get("pv_base") {
            obj.insert("pv_base".into(), pv.clone());
        }
        if let Some(p) = pack {
            if let Some(sections) = p.get("sections").cloned() {
                obj.insert("load_methods".into(), sections);
            }
        }
    }
    Ok(out)
}

/// Resolve tenant (site_id) from backend SDK key. When sdk_enforce is off (lab),
/// accept `X-Gr-Tenant` / body site_id as lab tenant — never trust browser for assoc writes.
pub fn resolve_backend_tenant(
    db: &AdminDb,
    raw_key: Option<&str>,
    host: Option<&str>,
    origin: Option<&str>,
    lab_tenant: Option<&str>,
) -> Result<String, String> {
    if !db.sdk_enforce_enabled() {
        if let Some(t) = lab_tenant.map(|s| s.trim()).filter(|s| !s.is_empty()) {
            return Ok(t.to_string());
        }
        return Ok("lab_default".into());
    }
    let Some(raw) = raw_key.filter(|s| !s.is_empty()) else {
        return Err("sdk_key_required".into());
    };
    check_backend_key(db, Some(raw), host, origin)?;
    let hash = hash_secret(raw);
    let Some((_kid, site_id, _allowed_origins)) = db.find_active_backend_key(&hash)? else {
        return Err("sdk_key_invalid".into());
    };
    Ok(site_id)
}

fn origin_hostname(origin: &str) -> Option<String> {
    let t = origin.trim();
    let rest = if let Some((_, r)) = t.split_once("://") {
        r
    } else {
        t.trim_start_matches('/')
    };
    let hostport = rest.split('/').next().unwrap_or("").split('?').next().unwrap_or("");
    let host = hostport.split(':').next().unwrap_or("").trim().to_ascii_lowercase();
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

fn origin_exact_match(allowed: &str, origin: &str) -> bool {
    let a = allowed.trim();
    let o = origin.trim();
    if a.is_empty() || o.is_empty() {
        return false;
    }
    if a == "*" {
        return !matches!(
            gr_abi::env::get("DEPLOY_ENV")
                 .or_else(|| gr_abi::env::get("DEPLOY_ENV"))
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "prod" | "production" | "live"
        );
    }
    if a.eq_ignore_ascii_case(o) {
        return true;
    }
    let a_scheme = a.split_once("://").map(|(s, _)| s.to_ascii_lowercase());
    let o_scheme = o.split_once("://").map(|(s, _)| s.to_ascii_lowercase());
    match (a_scheme.as_deref(), o_scheme.as_deref()) {
        (Some(as_), Some(os_)) if as_ != os_ => return false,
        _ => {}
    }
    match (origin_hostname(a), origin_hostname(o)) {
        (Some(ah), Some(oh)) => ah == oh,
        _ => false,
    }
}

fn hostname_exact_in(origin: &str, host: &str) -> bool {
    origin_hostname(origin).map(|h| h.eq_ignore_ascii_case(host)).unwrap_or(false)
}

/// Validate backend SDK key against Host/Origin and site domains.
pub fn check_backend_key(
    db: &AdminDb,
    raw_key: Option<&str>,
    host: Option<&str>,
    origin: Option<&str>,
) -> Result<(), String> {
    if !db.sdk_enforce_enabled() {
        return Ok(());
    }
    let Some(raw) = raw_key.filter(|s| !s.is_empty()) else {
        return Err("sdk_key_required".into());
    };
    let hash = hash_secret(raw);
    let Some((_kid, site_id, allowed_origins)) = db.find_active_backend_key(&hash)? else {
        return Err("sdk_key_invalid".into());
    };
    let hosts = db.site_hostnames(&site_id)?;
    if let Some(h) = host.map(|s| s.split(':').next().unwrap_or(s).to_ascii_lowercase()) {
        if !h.is_empty() {
            // Prefer exact domain registry match when present
            if let Ok(Some(dom)) = db.get_domain_by_hostname(&h) {
                let dom_site = dom.get("site_id").and_then(|v| v.as_str()).unwrap_or("");
                if !dom_site.is_empty() && dom_site != site_id {
                    return Err("sdk_key_site_mismatch".into());
                }
            } else if !hosts.iter().any(|x| x.eq_ignore_ascii_case(&h))
                && !allowed_origins.iter().any(|o| origin_exact_match(o, &format!("https://{h}")) || origin_hostname(o).map(|oh| oh == h).unwrap_or(false))
            {
                let origin_ok = origin
                    .map(|o| {
                        allowed_origins.iter().any(|a| origin_exact_match(a, o))
                            || hosts.iter().any(|hh| hostname_exact_in(o, hh))
                    })
                    .unwrap_or(false);
                if !origin_ok && !hosts.is_empty() {
                    return Err("sdk_key_site_mismatch".into());
                }
            }
        }
    }
    if let Some(o) = origin.filter(|s| !s.is_empty()) {
        if !allowed_origins.is_empty()
            && !allowed_origins.iter().any(|a| origin_exact_match(a, o))
        {
            let host_ok = hosts.iter().any(|h| hostname_exact_in(o, h));
            if !host_ok {
                return Err("sdk_origin_not_allowed".into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod edge_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn origin_suffix_confusion_rejected() {
        assert!(origin_exact_match("https://example.com", "https://example.com"));
        assert!(!origin_exact_match("https://example.com", "https://example.com.attacker.tld"));
        assert!(!origin_exact_match("https://example.com", "http://example.com"));
        assert!(!hostname_exact_in("https://example.com.attacker.tld", "example.com"));
        assert!(hostname_exact_in("https://shop.example.com", "shop.example.com"));
    }

    #[test]
    fn resolve_pv_load_gv_upload() {
        let site = json!({
            "fe_load": "pv",
            "upload_ingest": "gv",
            "edge_mode": "dual_domain",
            "pv_base": "https://pv.example.com",
            "gv_base": "https://gv.example.com"
        });
        let r = resolve_edge_bases(&site);
        assert_eq!(r["script_base"], "https://pv.example.com");
        assert_eq!(r["api_base"], "https://gv.example.com");
        assert_eq!(r["gw_base"], "https://gv.example.com");
        assert_eq!(r["unified_gv_upload"], true);
        assert_eq!(r["gv_nginx_forbidden"], true);
        assert_eq!(r["backend_sdk_relay_required"], false);
        assert_eq!(r["proxied_upload"], false);
    }

    #[test]
    fn resolve_legacy_fp_upload_no_relative_api() {
        let site = json!({
            "fe_load": "first_party",
            "upload_ingest": "first_party",
            "gv_base": "https://gv.example.com"
        });
        let r = resolve_edge_bases(&site);
        assert_eq!(r["api_base"], "");
        assert_eq!(r["gw_base"], "https://gv.example.com");
        assert_eq!(r["proxied_upload"], true);
        assert_eq!(r["upload_path"], "proxied_forbidden");
    }

    #[test]
    fn hostname_from_base_ok() {
        assert_eq!(
            crate::admin::db::hostname_from_base("https://pv.sozhan.net/foo"),
            Some("pv.sozhan.net".into())
        );
        assert_eq!(crate::admin::db::hostname_from_base("/gr"), None);
    }
}

/// Validate biz visit upload: site must exist; when sdk_enforce, FE/backend key must bind to site.
///
/// Production note: browser FE **must not** embed the raw secret. When `sdk_enforce` is on:
/// - Prefer server upsert on open/pixel/gateway, and/or **backend** `POST /v1/biz/visit` with `X-Gr-Sdk-Key`
/// - Browser same-origin visits may omit the key if `Origin` matches a registered site hostname
pub fn check_biz_visit_auth(
    db: &AdminDb,
    site_id: &str,
    raw_key: Option<&str>,
    host: Option<&str>,
    origin: Option<&str>,
) -> Result<(), String> {
    if site_id.trim().is_empty() {
        return Err("site_id_required".into());
    }
    if db.get_site(site_id)?.is_none() {
        return Err("site_not_found".into());
    }
    let hosts = db.site_hostnames(site_id)?;
    // Soft origin check when Origin present (even if enforce off): must match site hosts if any.
    if let Some(o) = origin.filter(|s| !s.is_empty()) {
        if !hosts.is_empty() {
            let host_ok = hosts.iter().any(|h| hostname_exact_in(o, h));
            if !host_ok && db.sdk_enforce_enabled() {
                return Err("sdk_origin_not_allowed".into());
            }
        }
    }
    if !db.sdk_enforce_enabled() {
        return Ok(());
    }
    // FE browser path: Origin bound to site hosts → no raw secret in page.
    let Some(raw) = raw_key.filter(|s| !s.is_empty()) else {
        if let Some(o) = origin.filter(|s| !s.is_empty()) {
            if hosts.iter().any(|h| hostname_exact_in(o, h)) {
                return Ok(());
            }
        }
        return Err("sdk_key_required".into());
    };
    let hash = hash_secret(raw);
    let Some((_kid, key_site, allowed_origins)) = db.find_active_sdk_key(&hash, None)? else {
        return Err("sdk_key_invalid".into());
    };
    if key_site != site_id {
        return Err("sdk_key_site_mismatch".into());
    }
    if let Some(h) = host.map(|s| s.split(':').next().unwrap_or(s).to_ascii_lowercase()) {
        if !h.is_empty() {
            if let Ok(Some(dom)) = db.get_domain_by_hostname(&h) {
                let dom_site = dom.get("site_id").and_then(|v| v.as_str()).unwrap_or("");
                if !dom_site.is_empty() && dom_site != site_id {
                    return Err("sdk_key_site_mismatch".into());
                }
            }
        }
    }
    if let Some(o) = origin.filter(|s| !s.is_empty()) {
        if !allowed_origins.is_empty()
            && !allowed_origins.iter().any(|a| origin_exact_match(a, o))
        {
            let host_ok = hosts.iter().any(|h| hostname_exact_in(o, h));
            if !host_ok {
                return Err("sdk_origin_not_allowed".into());
            }
        }
    }
    Ok(())
}
