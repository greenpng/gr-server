//! Admin **Deploy Center** — L1–L5 load methods (tasks/panel/01-deploy-spec.md).
//!
//! Script may be reverse-proxied. Upload (`open`/`ingest`/`gateway`) must be
//! bound-domain Pingora TLS direct. `upload_ingest=first_party` is not a
//! recommended profile.

use crate::admin::db::AdminDb;
use crate::admin::sdk::{deploy_guide, resolve_edge_bases};
use serde_json::{json, Value};

/// Full deploy pack for one site (panel Deploy Center SSOT).
pub fn full_deploy_pack(db: &AdminDb, site_id: &str) -> Result<Value, String> {
    let site = db
        .get_site(site_id)?
        .ok_or_else(|| "site_not_found".to_string())?;
    let hosts = db.site_hostnames(site_id)?;
    let domains = db.list_domains(Some(site_id), None).unwrap_or_default();
    let edge = resolve_edge_bases(&site);
    let basic = deploy_guide(&site, &hosts);

    let fe = edge
        .get("fe_load")
        .and_then(|v| v.as_str())
        .unwrap_or("pv");
    let up = edge
        .get("upload_ingest")
        .and_then(|v| v.as_str())
        .unwrap_or("gv");
    let gv = edge
        .get("gv_base")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .trim_end_matches('/');
    let pv = edge
        .get("pv_base")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .trim_end_matches('/');
    let poll = edge
        .get("poll_method")
        .and_then(|v| v.as_str())
        .unwrap_or("both");
    let api = edge
        .get("api_base")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let script = edge
        .get("script_base")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let gw = edge
        .get("gw_base")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let unified_gv = up == "gv"
        || (api.starts_with("https://")
            && gw.starts_with("https://")
            && host_of(api) == host_of(gw)
            && !host_of(api).is_empty());

    let profile_id = if up == "first_party" {
        "proxied_upload_forbidden"
    } else if fe == "pv" && (up == "gv" || unified_gv) {
        "pv_load_gv_upload"
    } else if fe == "first_party" && (up == "gv" || unified_gv) {
        "fp_script_proxy_gv_upload"
    } else if fe == "pv" {
        "pv_load_upload_gv"
    } else {
        "custom"
    };

    let www_hosts: Vec<String> = hosts
        .iter()
        .filter(|h| !h.starts_with("pv.") && !h.starts_with("gv."))
        .cloned()
        .collect();
    let www_primary = www_hosts
        .first()
        .cloned()
        .unwrap_or_else(|| "www.example.com".into());

    let gv_host = if !gv.is_empty() {
        host_of(gv)
    } else {
        format!("gv.{}", base_domain(&www_primary))
    };
    let pv_host = if !pv.is_empty() {
        host_of(pv)
    } else {
        format!("pv.{}", base_domain(&www_primary))
    };

    let site_key = site
        .get("site_id")
        .and_then(|v| v.as_str())
        .unwrap_or(site_id);
    let embed_token = site
        .get("embed_token")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    Ok(json!({
        "ok": true,
        "site_id": site_id,
        "site": site,
        "profile_id": profile_id,
        "profile_label": profile_label(profile_id),
        "resolved": edge,
        "basic_guide": basic,
        "embed_token": embed_token,
        "topology": {
            "fe_load": fe,
            "upload_ingest": up,
            "unified_gv_upload": unified_gv,
            "api_base": api,
            "script_base": script,
            "gw_base": gw,
            "poll_method": poll,
            "www_hosts": www_hosts,
            "gv_host": gv_host,
            "pv_host": pv_host,
            "ascii": topology_ascii(fe, up, unified_gv, &www_primary, api, script, gw),
        },
        "sections": {
            "overview": section_overview(profile_id, fe, up, unified_gv, poll),
            "dns_ssl": section_dns_ssl(&www_primary, &gv_host, &pv_host, fe, up, unified_gv),
            "nginx": section_nginx(site_key, &www_primary, &gv_host, &pv_host, fe, up, unified_gv, embed_token, pv, gw),
            "cloudflare": section_cloudflare(&www_primary, &gv_host, &pv_host, fe, up, unified_gv),
            "inject": section_inject(site_key, api, script, gw, poll, fe, unified_gv, embed_token, site.get("cookie_fields")),
            "site_app": section_site_app(site_key, api, gw, poll, fe),
            "backend_sdk": section_backend_sdk(),
            "verify": section_verify(api, gw, &www_primary, &gv_host, poll, unified_gv),
            "checklist": section_checklist(fe, up, unified_gv, &domains, gv),
        },
        "scripts": script_catalog(),
        "docs": [
            "tasks/panel/01-deploy-spec.md",
            "tasks/panel/02-site-token.md",
            "edge/cf-worker/CF-DASHBOARD-GUIDE.md"
        ],
    }))
}

/// Static profile catalog for the Deploy Center chooser.
pub fn profile_catalog() -> Value {
    json!({
        "ok": true,
        "default_profile": "pv_load_gv_upload",
        "profiles": [
            {
                "id": "pv_load_gv_upload",
                "name": "L1 直连 pv 加载 + gv 上传（推荐）",
                "fe_load": "pv",
                "upload_ingest": "gv",
                "gv_dns": "dns_to_origin_pingora_tls",
                "www_cf": "orange_ok_under_attack_for_page_load",
                "need_www_g5_api": false,
                "need_www_g5_static": false,
                "summary": "脚本 src=https://pv…/gr.js?grt=token；open/ingest/B8 全走 https://gv Pingora TLS 直连。禁止反代上传。"
            },
            {
                "id": "fp_script_proxy_gv_upload",
                "name": "L2 反代脚本加载 + gv 上传",
                "fe_load": "first_party",
                "upload_ingest": "gv",
                "gv_dns": "dns_to_origin_pingora_tls",
                "www_cf": "orange_ok_under_attack_for_page_load",
                "need_www_g5_api": false,
                "need_www_g5_static": true,
                "summary": "www 只反代 /gr.js 与 /dist/ 到 pv；/v1/* 必须浏览器直连 gv。可同源读业务 cookie。"
            },
            {
                "id": "pv_load_upload_gv",
                "name": "pv 加载 + 绑定域名上传",
                "fe_load": "pv",
                "upload_ingest": "gv",
                "gv_dns": "grey_cloud_dns_only",
                "www_cf": "orange_ok_under_attack_ok",
                "need_www_g5_api": false,
                "need_www_g5_static": false,
                "summary": "JS 挂 pv；上传+B8 走 gv。inject 仅一小段配置。"
            },
            {
                "id": "proxied_upload_forbidden",
                "name": "反代上传（禁止，需改绑定域名直连）",
                "fe_load": "first_party",
                "upload_ingest": "first_party",
                "gv_dns": "n/a",
                "www_cf": "n/a",
                "need_www_g5_api": false,
                "need_www_g5_static": false,
                "summary": "upload_ingest=first_party 会把上传走用户反代，破坏 JA4/真实 IP。面板默认拒绝；设 GR_ALLOW_FIRST_PARTY_UPLOAD=1 才可保存。"
            }
        ]
    })
}

/// Best-effort server-side verify (origin health + optional public HEAD).
pub fn run_verify_checks(db: &AdminDb, site_id: &str) -> Result<Value, String> {
    let pack = full_deploy_pack(db, site_id)?;
    let edge = pack.get("resolved").cloned().unwrap_or(json!({}));
    let gw = edge
        .get("gw_base")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let api = edge
        .get("api_base")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut checks = Vec::new();
    checks.push(http_check(
        "origin_ingest_health",
        "http://127.0.0.1:28765/health",
        "GET",
    ));
    checks.push(http_check(
        "origin_gateway_healthz",
        "http://127.0.0.1:28766/healthz",
        "GET",
    ));

    if gw.starts_with("https://") || gw.starts_with("http://") {
        checks.push(http_check(
            "public_gv_health",
            &format!("{}/v1/health", gw.trim_end_matches('/')),
            "GET",
        ));
    }

    if api.starts_with("https://") || api.starts_with("http://") {
        checks.push(http_check(
            "public_api_health",
            &format!("{}/v1/health", api.trim_end_matches('/')),
            "GET",
        ));
    }

    let hosts = db.site_hostnames(site_id).unwrap_or_default();
    let pass = checks
        .iter()
        .filter(|c| c.get("ok").and_then(|v| v.as_bool()) == Some(true))
        .count();
    let total = checks.len();

    Ok(json!({
        "ok": true,
        "site_id": site_id,
        "summary": { "pass": pass, "total": total, "all_ok": pass == total && total > 0 },
        "checks": checks,
        "hostnames": hosts,
        "note": "公网探测受本机出口/DNS/CF 影响；浏览器侧需另测脚本 token 与 CORS。",
        "browser_manual": pack.pointer("/sections/verify").cloned().unwrap_or(json!({})),
    }))
}

fn pin_url(script: &str, token: &str) -> String {
    let base = if script.starts_with("http://") || script.starts_with("https://") {
        format!("{}/gr.js", script.trim_end_matches('/'))
    } else if script.is_empty() {
        "/gr.js".into()
    } else {
        format!("{}/gr.js", script.trim_end_matches('/'))
    };
    if token.is_empty() {
        base
    } else {
        format!("{base}?grt={token}")
    }
}

fn boot_snippet(
    site_id: &str,
    api: &str,
    gw: &str,
    poll: &str,
    pin: &str,
    inject_path: &str,
    token: &str,
    cookie_fields: &Value,
) -> String {
    let site_json = serde_json::to_string(site_id).unwrap_or_else(|_| "\"\"".into());
    let api_j = serde_json::to_string(api).unwrap_or_else(|_| "\"\"".into());
    let gw_j = serde_json::to_string(gw).unwrap_or_else(|_| "\"\"".into());
    let poll_j = serde_json::to_string(poll).unwrap_or_else(|_| "\"both\"".into());
    let pin_j = serde_json::to_string(pin).unwrap_or_else(|_| "\"/gr.js\"".into());
    let inj_j = serde_json::to_string(inject_path).unwrap_or_else(|_| "\"app\"".into());
    let tok_j = serde_json::to_string(token).unwrap_or_else(|_| "\"\"".into());
    let cookies_j = serde_json::to_string(cookie_fields).unwrap_or_else(|_| "[]".into());
    format!(
        r#"<!-- gr inject site={site_id} -->
<script>
(function(){{
  var S={site_json}, P={api_j}, G={gw_j};
  window.__GR_SITE_ID__=S;
  window.__GR_BOOT__=Object.assign({{}}, window.__GR_BOOT__||{{}}, {{
    apiBase:P, gwBase:G, assetBase:P,
    poll_method:{poll_j}, site_id:S, siteId:S,
    inject_path:{inj_j}, pin_url:{pin_j}, embed_token:{tok_j},
    cookie_fields:{cookies_j}
  }});
}})();
</script>
<script src="{pin}" fetchpriority="high" async></script>
"#
    )
}

fn section_overview(profile: &str, fe: &str, up: &str, unified: bool, poll: &str) -> Value {
    json!({
        "title": "部署总览",
        "profile_id": profile,
        "choices": {
            "fe_load": fe,
            "upload_ingest": up,
            "unified_gv": unified,
            "poll_method": poll,
            "backend_sdk_relay": false
        },
        "principles": [
            "只有前端探测脚本可以反代；上传必须走面板绑定域名 Pingora TLS 直连",
            "新站点默认 fe_load=pv、upload_ingest=gv",
            "脚本 URL 带 ?grt=<embed_token>，服务端 token-first 校验后只对本站发放",
            "业务 cookie 由脚本从 document.cookie 读取后写入 open body，不要指望 Cookie 头打到 gv",
            "apiBase/gwBase 一律绑定域名绝对地址"
        ]
    })
}

fn section_dns_ssl(
    www: &str,
    gv_host: &str,
    pv_host: &str,
    fe: &str,
    up: &str,
    unified: bool,
) -> Value {
    let mut records = vec![
        json!({"host": www, "type": "A/AAAA 或 CNAME", "proxy": "橙云可选", "note": "业务站；可 Under Attack"}),
        json!({"host": gv_host, "type": "A/AAAA", "proxy": "灰云 DNS-only（推荐）", "note": "上传+B8；禁止 Under Attack / HTTP 反代"}),
        json!({"host": pv_host, "type": "A/AAAA", "proxy": "灰云推荐", "note": "脚本加载 Pingora TLS（默认 :8443）"}),
    ];
    if fe != "pv" && up != "pv" {
        records.pop();
    }
    json!({
        "title": "DNS 与 SSL",
        "records": records,
        "ssl": [
            format!("业务站 {}：宝塔/LE 或 CF Full(strict)", www),
            format!("gv {}：源站证书 SAN 含 {}（面板上传 PEM 或 ACME）", gv_host, gv_host),
            format!("pv {}：源站证书 SAN 含 {}", pv_host, pv_host),
            if unified {
                "浏览器只信任 https://gv 证书；务必有效".to_string()
            } else {
                "pv 与 gv 各自有效证书；Pingora SNI 选证".to_string()
            }
        ],
        "panel_actions": [
            "域名页登记 www / pv / gv",
            "SSL 页签发或上传 PEM 后 runtime/apply"
        ]
    })
}

fn section_nginx(
    _site_key: &str,
    www: &str,
    gv_host: &str,
    pv_host: &str,
    fe: &str,
    _up: &str,
    _unified: bool,
    token: &str,
    pv: &str,
    gw: &str,
) -> Value {
    let pv_up = if pv.starts_with("http") {
        pv.trim_end_matches('/').to_string()
    } else {
        format!("https://{}", pv_host)
    };
    let q = if token.is_empty() {
        String::new()
    } else {
        format!("?grt={token}")
    };
    let l2 = format!(
        r#"# L2 — {www} 只反代脚本到 pv，禁止代理 /v1/
location = /gr.js {{
  proxy_pass {pv_up}/gr.js{q};
  proxy_ssl_server_name on;
  proxy_set_header Host {pv_host};
  proxy_cache off;
  add_header Cache-Control "no-store";
}}
location ^~ /dist/ {{
  proxy_pass {pv_up}/dist/;
  proxy_ssl_server_name on;
  proxy_set_header Host {pv_host};
  proxy_cache off;
}}
# 不要 location /v1/ — 上传必须浏览器直连 {gw}
"#,
        www = www,
        pv_up = pv_up,
        q = q,
        pv_host = pv_host,
        gw = if gw.is_empty() {
            format!("https://{gv_host}")
        } else {
            gw.to_string()
        }
    );
    json!({
        "title": "Nginx（仅脚本反代）",
        "www_conf": format!("{}.conf", www),
        "gv_role": "Pingora TLS 直连 — 全部 probe API+B8（无 nginx）",
        "gv_host": gv_host,
        "pv_conf": format!("{}.conf", pv_host),
        "steps": [
            "面板绑定 pv/gv 域名并上传证书，runtime/apply",
            "L1：页面直接 <script src=https://pv…/gr.js?grt=token>",
            "L2：www 只反代 /gr.js 与 /dist/，禁止 /v1/",
            "【红线】gv 禁止任何 HTTP 反代；Pingora 终结客户端 TLS"
        ],
        "snippets": {
            "l2_script_proxy_only": l2,
            "gv_pingora_all_upload": format!(
                "# 全部上传+B8 → https://{gv}  Pingora TLS 直连\n# 禁止: Browser --TLS--> nginx --HTTP--> origin\n",
                gv = gv_host
            )
        },
        "script_commands": [],
        "notes": [
            "【红线】gv 禁止任何反代",
            "【红线】apiBase=gwBase=https://gv（绝对地址）",
            if fe == "first_party" { "L2 反代加载可同源读业务 cookie" } else { "L1 直连：业务 cookie 种到 pv，或改 L2 同源" }
        ]
    })
}

fn section_cloudflare(
    www: &str,
    gv_host: &str,
    pv_host: &str,
    fe: &str,
    up: &str,
    _unified: bool,
) -> Value {
    json!({
        "title": "Cloudflare 设置（L5）",
        "dns_proxy": [
            {"host": www, "proxy": "橙云 DNS+HTTP", "under_attack": "可开", "note": "业务站"},
            {"host": gv_host, "proxy": "灰云 DNS only", "under_attack": "禁止", "note": "上传+B8 必须不被 challenge"},
            {"host": pv_host, "proxy": if fe == "pv" || up == "pv" { "灰云推荐；脚本路径可 WAF Skip" } else { "可闲置" }, "under_attack": "禁止作 API", "note": "仅脚本"}
        ],
        "waf_optional_skip": {
            "expression": "(http.request.uri.path eq \"/gr.js\") or (http.request.uri.path starts_with \"/dist/\")",
            "action": "Skip — Security Level / Bot Fight（仅脚本路径）",
            "when": "L5 Worker 代理脚本时"
        },
        "cf_worker": {
            "use_when": "不能改 nginx head 时",
            "paths": ["/gr.js", "/dist/*"],
            "forbidden": ["/v1/*"],
            "doc": "edge/cf-worker/CF-DASHBOARD-GUIDE.md",
            "mutex": "同一站不要 nginx + CF Worker 双 PRIMARY"
        }
    })
}

fn section_inject(
    site_id: &str,
    api: &str,
    script: &str,
    gw: &str,
    poll: &str,
    _fe: &str,
    _unified: bool,
    token: &str,
    cookie_fields: Option<&Value>,
) -> Value {
    let cookies = cookie_fields.cloned().unwrap_or_else(|| json!([]));
    let pin = pin_url(script, token);
    let l1 = boot_snippet(site_id, api, gw, poll, &pin, "app", token, &cookies);
    let l4 = l1.clone();
    let pv_up = if script.starts_with("http") {
        script.trim_end_matches('/').to_string()
    } else {
        script.to_string()
    };
    let q = if token.is_empty() {
        String::new()
    } else {
        format!("?grt={token}")
    };
    let l2_nginx = format!(
        "location = /gr.js {{ proxy_pass {pv_up}/gr.js{q}; proxy_ssl_server_name on; }}\nlocation ^~ /dist/ {{ proxy_pass {pv_up}/dist/; }}\n"
    );
    let l3 = format!(
        "sub_filter '</head>' '{escaped}</head>';\nsub_filter_once on;\n",
        escaped = l1.replace('\n', "")
    );
    json!({
        "title": "五种加载方式（L1–L5）",
        "primary_rule": "同一站点只允许一处 PRIMARY 注入",
        "methods": [
            {
                "id": "l1_direct_pv",
                "name": "L1 直连绑定 pv 域名",
                "snippet": l1,
                "note": "script src 指向 pv；apiBase/gwBase 为 gv 绝对地址"
            },
            {
                "id": "l2_nginx_proxy",
                "name": "L2 nginx 反代加载（仅脚本）",
                "snippet": l2_nginx,
                "note": "不要 location /v1/"
            },
            {
                "id": "l3_nginx_head",
                "name": "L3 nginx 头注入",
                "snippet": l3,
                "note": "sub_filter 注入 boot+script；上传仍直连 gv"
            },
            {
                "id": "l4_site_app",
                "name": "L4 站点程序嵌入",
                "snippet": l4,
                "note": "Next/PHP/静态模板直接输出"
            },
            {
                "id": "l5_cf_worker",
                "name": "L5 CF Worker / head",
                "doc": "edge/cf-worker/CF-DASHBOARD-GUIDE.md",
                "when": "无法改源站 nginx；Worker 只代理 /gr.js 与 /dist/"
            }
        ],
        "boot_fields": {
            "apiBase": api,
            "gwBase": gw,
            "poll_method": poll,
            "pin_url": pin,
            "embed_token": token,
            "fe_load": _fe
        }
    })
}

fn section_site_app(site_id: &str, api: &str, gw: &str, poll: &str, fe: &str) -> Value {
    json!({
        "title": "站点程序侧配置",
        "do": [
            "HTML 模板 <head> 注入 L4 片段",
            "不要在浏览器嵌入 backend SDK secret",
            "登录/下单后由服务端用 backend Key 调 result",
            "业务 cookie 种到 pv，或脚本反代同源，由探测脚本读取后 POST gv"
        ],
        "next_env_example": {
            "GR_API_BASE": api,
            "GR_GW_BASE": gw,
            "GR_SITE_ID": site_id,
            "GR_POLL_METHOD": poll,
            "GR_FE_LOAD": fe
        },
        "csp_note": "CSP 放行 script-src 与 connect-src 到 pv/gv 源"
    })
}

fn section_backend_sdk() -> Value {
    json!({
        "title": "后端 SDK（非上传）",
        "role": "消费 result · 业务关联",
        "not_role": "不要中转浏览器探测材料上传",
        "steps": [
            "面板 SDK 页签发 backend Key",
            "服务端 GET /v1/session/:id/result + Header X-Gr-Sdk-Key",
            "可选 POST /v1/biz/visit"
        ]
    })
}

fn section_verify(
    api: &str,
    gw: &str,
    www: &str,
    gv_host: &str,
    poll: &str,
    _unified: bool,
) -> Value {
    let api_h = if api.starts_with("http") {
        api.trim_end_matches('/').to_string()
    } else {
        format!("https://{}", gv_host)
    };
    let gw_h = if gw.starts_with("http") {
        gw.trim_end_matches('/').to_string()
    } else {
        format!("https://{}", gv_host)
    };

    json!({
        "title": "部署后验证",
        "server_commands": [
            "curl -sS http://127.0.0.1:28765/v1/health | head -c 300",
            format!("curl -sS -o /dev/null -w '%{{http_code}}' {}/v1/health", gw_h),
            format!("curl -sS -o /dev/null -w '%{{http_code}}' {}/gr.js", api_h)
        ],
        "browser_checklist": [
            format!("打开 https://{}", www),
            format!("Network：GET …/gr.js?grt=… → 200（无 token / 错 token → 403）"),
            format!("Network：POST {}/v1/session/open → 200 JSON", api_h),
            format!("Network：POST {}/v1/gateway/early → 200", gw_h),
            format!("poll_method={}", poll)
        ],
        "fail_matrix": [
            {"symptom": "gr.js 403", "fix": "缺/错/跨站 embed token；轮换后需更新片段"},
            {"symptom": "open CORS", "fix": "域名页未登记 www"},
            {"symptom": "JA4 失真", "fix": "上传被 nginx/CF 反代 — 改为 gv 直连"}
        ]
    })
}

fn section_checklist(
    fe: &str,
    up: &str,
    _unified: bool,
    domains: &[Value],
    gv: &str,
) -> Value {
    let has_www = domains.iter().any(|d| {
        d.get("hostname")
            .and_then(|h| h.as_str())
            .map(|h| !h.starts_with("pv.") && !h.starts_with("gv."))
            .unwrap_or(false)
    });
    let gv_https = gv.starts_with("https://");
    json!({
        "title": "上线前勾选",
        "items": [
            {"id": "site_created", "label": "已创建站点且 collect 开启", "auto": true},
            {"id": "www_domain", "label": "已登记业务 www 域名（CORS）", "ok": has_www},
            {"id": "gv_https", "label": "gv_base 为绝对 HTTPS", "ok": gv_https},
            {"id": "embed_token", "label": "站点嵌入令牌已生成并写入脚本 URL", "manual": true},
            {"id": "fe_load", "label": format!("fe_load 已选：{}", fe), "ok": true},
            {"id": "upload", "label": format!("upload_ingest 已选：{}", up), "ok": up != "first_party"},
            {"id": "no_v1_proxy", "label": "反代只覆盖 /gr.js 与 /dist/，无 /v1/", "manual": true},
            {"id": "inject", "label": "PRIMARY inject 仅一处", "manual": true},
            {"id": "ssl_gv", "label": "gv 证书有效", "manual": true},
            {"id": "no_sdk_relay", "label": "未使用后端 SDK 中转上传", "ok": true}
        ]
    })
}

fn script_catalog() -> Value {
    json!([
        {"path": "edge/cf-worker/", "use": "L5 CF Worker 仅代理脚本"},
        {"path": "tasks/panel/01-deploy-spec.md", "use": "部署规范 SSOT"}
    ])
}

fn profile_label(id: &str) -> &'static str {
    match id {
        "pv_load_gv_upload" => "L1 直连 pv 加载 + gv 上传（推荐）",
        "fp_script_proxy_gv_upload" => "L2 反代脚本加载 + gv 上传",
        "pv_load_upload_gv" => "pv 加载+上传拓扑",
        "proxied_upload_forbidden" => "反代上传（禁止）",
        _ => "自定义",
    }
}

fn topology_ascii(
    fe: &str,
    up: &str,
    unified: bool,
    www: &str,
    api: &str,
    script: &str,
    gw: &str,
) -> String {
    format!(
        "www {www}\n  ├─ FE load ({fe}): {script}\n  ├─ upload ({up}): {api}\n  └─ B8/gw: {gw}\n  unified_gv={unified}\n",
        www = www,
        fe = fe,
        script = script,
        up = up,
        api = api,
        gw = if gw.is_empty() { "(missing)" } else { gw },
        unified = unified
    )
}

fn host_of(base: &str) -> String {
    crate::admin::db::hostname_from_base(base).unwrap_or_default()
}

fn base_domain(www: &str) -> String {
    let h = www.trim().trim_start_matches("www.");
    if h.is_empty() {
        "example.com".into()
    } else {
        h.to_string()
    }
}

fn http_check(id: &str, url: &str, method: &str) -> Value {
    let start = std::time::Instant::now();
    let mut cmd = std::process::Command::new("curl");
    cmd.args([
        "-sS",
        "-o",
        "/dev/null",
        "-w",
        "%{http_code}",
        "--connect-timeout",
        "3",
        "--max-time",
        "8",
        "-X",
        method,
    ]);
    if method.eq_ignore_ascii_case("POST") {
        cmd.args(["-H", "content-type: application/json", "-d", "{}"]);
    }
    cmd.arg(url);
    let output = cmd.output();
    let ms = start.elapsed().as_millis() as u64;
    match output {
        Ok(o) if o.status.success() || !o.stdout.is_empty() => {
            let code_s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let status: u16 = code_s.parse().unwrap_or(0);
            let ok = (200..400).contains(&status);
            let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
            if status == 0 && !err.is_empty() {
                json!({
                    "id": id, "url": url, "method": method,
                    "ok": false, "status": 0, "ms": ms, "error": err
                })
            } else {
                json!({
                    "id": id, "url": url, "method": method,
                    "ok": ok, "status": status, "ms": ms
                })
            }
        }
        Ok(o) => json!({
            "id": id, "url": url, "method": method, "ok": false,
            "status": 0, "ms": ms,
            "error": String::from_utf8_lossy(&o.stderr).trim()
        }),
        Err(e) => json!({
            "id": id, "url": url, "method": method, "ok": false,
            "status": 0, "ms": ms, "error": format!("curl_missing_or_fail:{e}")
        }),
    }
}
