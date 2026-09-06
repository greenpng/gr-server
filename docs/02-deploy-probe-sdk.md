# Green V7 Deployment Cookbook

Install → probe deploy (three modes) → result receive (six-language SDK),
with cookie business-identifier capture.

- **Docs-home**: `docs/guides/12-DEPLOYMENT-COOKBOOK.md` (this file).
- **Official site copy**: `01-official-site/web/docs-deploy.html` (served at
  `/docs/deploy`).

## 0. Components and origins

| Component | Role | Examples |
|---|---|---|
| **PV** | Panel/control origin (admin console, site config, embed assets) | `https://pv.example.com` |
| **GV** | Probe gateway origin (Pingora; session open / ingest / result) | `https://gv.example.com` |
| **FE** | `g.js` boot + collectors, loaded from PV or your CDN | `https://pv.example.com/g.js` |
| Site | Your business site (www) | `https://shop.example.com` |

Cookie capture happens **on the GV open request**: the probe server reads
the request `Cookie` header, extracts only the names allowlisted in the
site config, and echoes them back in the result. No FE change needed, but
cookies must physically reach the open request (see §3).

## 1. Install (fast path)

```bash
curl -fsSL -O https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh
bash install.sh --version 8.0.0            # or: bash install.sh --with-docker
```

What happens: arch detect → signed release download → SHA-256 + ELF +
signature verify → `.env` (`GR_BIND` etc.) → `gr-cli install --data-dir`
bootstrap → module stage/activate (6 modules, OTA) → systemd (or nohup) →
health gate → `install-state.env`.

First login: one-time credentials at

```text
/opt/green-v7/data/admin/admin_bootstrap_once.txt     # adjust for INSTALL_ROOT
```

Open the panel: `http://<GR_BIND>:<port>/console/<hash>/` → `POST
{console}/api/login`. Change the password immediately. Upgrade:

```bash
VERSION=8.0.0 INSTALL_ROOT=/opt/green-v7 bash 04-release-github-ci/install/release/update_runtime_from_github.sh
```

Rollback: keep the previous runtime tar + module set; `upgrade_rollback_check.sh`
validates the dance. See `docs/guides/05-RELEASE.md`, `docs/guides/11-OPERATIONS.md`.

## 2. Create a site + cookie allowlist

Panel **Sites → Create site**:

- `site_id` — business tenant id you will pass in the embed
- `root_domains` — www hostnames (CORS + hostname binding)
- `consent_confirmed` — required on create (probe disclosure gate)
- `cookie_fields` — **cookie allowlist** for business-identifier capture:
  ```json
  ["user_id", "plan_tier", "cart_id", "utm_source"]
  ```

Rules enforced server-side:

| Rule | Value |
|---|---|
| name charset | `[A-Za-z0-9_.-]`, ≤ 64 chars |
| per-session cap | ≤ 16 captured fields |
| value length | truncated to 256 chars |
| sensitive-name blocklist | names containing `password/passwd/pwd/jwt/secret/credential/token/apikey/api_key` are never captured |
| capture points | session open (authoritative) + ingest fallback (first capture wins) |
| empty values | skipped |

The config flows `control.sites` → `public.sites` (probe plane), so GV and
PV never diverge. Optional paid/cloud: bind the site on the Official Site
(domain verify) → auto-sync into the panel.

## 3. Probe deploy — three modes

### A. Nginx first-party (recommended; `inject_path=nginx`)

The business nginx serves the boot JS and (optionally) the API under a
same-origin path, so the visitor's `Cookie` header reaches the open request
naturally — cookie capture works out of the box.

```nginx
# www site vhost
location = /g5/g.js {                       # boot asset from PV
    proxy_pass https://pv.example.com/g.js;
    proxy_set_header Host pv.example.com;
    proxy_ssl_server_name on;
}
location /g5/ {                             # same-origin API prefix (optional)
    proxy_pass https://gv.example.com/;     # strips /g5
    proxy_set_header Host gv.example.com;
    proxy_ssl_server_name on;
    proxy_set_header Cookie $http_cookie;   # CRITICAL for cookie capture
    proxy_set_header X-Forwarded-For $remote_addr;
    proxy_set_header X-Gr-Peer-Ip $remote_addr;
}
```

Embed:

```html
<script src="/g5/g.js"
        data-site-id="mysite"
        data-endpoint="/g5"
        data-inject-path="nginx"
        defer></script>
```

TLS termination stays at your nginx; keep `GR_BIND=127.0.0.1` on the
service. Do **not** rewrite/drop `Cookie` or `X-Session-Id` on the way to GV.

### B. Cloudflare worker (`inject_path=cf_worker`)

The worker injects the boot script into HTML and proxies `/g5` in the same
origin (DNS-only / worker route on the www domain).

```js
export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname.startsWith("/g5")) {
      const target = "https://gv.example.com" + url.pathname.slice(3) + url.search;
      const headers = new Headers(request.headers); // includes Cookie
      headers.set("X-Gr-Peer-Ip", request.headers.get("CF-Connecting-IP") || "");
      const upstream = await fetch(target, {
        method: request.method,
        headers,
        body: request.method === "GET" ? undefined : request.body,
      });
      return upstream;
    }
    // HTML injection for the site pages
    const res = await fetch(request);
    if (!res.headers.get("content-type")?.includes("text/html")) return res;
    const html = await res.text();
    const boot = `<script src="/g5/g.js" data-site-id="mysite" data-endpoint="/g5" data-inject-path="cf_worker" defer></script>`;
    return new Response(html.replace("</head>", boot + "</head>"), res);
  },
};
```

Notes:

- Passing `request.headers` through keeps `Cookie` intact → cookie capture
  works with the visitor's real cookies (set by your origin).
- Avoid Cloudflare **Under Attack** mode on the `/g5` path (challenge pages
  break the probe open/ingest). Use DNS-only or a managed challenge that
  exempts `/g5`.

### C. Site scripting / CDN embed (`inject_path=app`)

Load the boot JS directly (from PV or your CDN) and point `data-endpoint` at
GV or PV:

```html
<script src="https://pv.example.com/g.js"
        data-site-id="mysite"
        data-endpoint="https://gv.example.com"
        data-inject-path="app"
        defer></script>
```

Cookie-capture caveats in this mode:

- Cross-origin JS `fetch` does **not** send cookies unless the FE sets
  `credentials: include` **and** the cookie is `SameSite=None; Secure`.
- Reliable capture options:
  1. Serve the page and the endpoint same-site (path or subdomain) — cookies
     flow.
  2. Have your backend open the session server-side (your app POSTs
     `/v1/session/open` with the visitor's `Cookie` header preserved) and
     hand the returned `session_id` to the page; the FE continues probing
     that session.

## 4. Receive results (six-language SDK)

Create a backend key in the SDK section of the site config
(`GR_SITE_RESULT_KEY`). The SDK **never relays probes**; it only queries
results.

| Lang | Path | Call |
|---|---|---|
| JS | `02-probe-analysis/sdk/src/index.js` | `new GrResultClient({baseUrl, apiKey})` → `waitForResult(...)` |
| Python | `02-probe-analysis/sdk/python/gr_results.py` | `GrResultClient(base_url, api_key).wait_for_result(...)` |
| Go | `02-probe-analysis/sdk/go/gr_results.go` | `gr.New(baseURL, key).WaitForResult(...)` |
| PHP | `02-probe-analysis/sdk/php/GrResultsClient.php` | `(new GrResultClient(...))->waitForResult(...)` |
| Shell | `02-probe-analysis/sdk/shell/gr_sdk.sh` | `gr_wait_for_result <session> sdk 8000` |
| Rust | `02-probe-analysis/sdk/rust/` | `Client::new(base_url, key).wait_for_result(...)` |

Common contract:

```
GET {gv_base}/v1/session/{session_id}/result?projection=sdk
X-Gr-Sdk-Key: <site backend key>
```

`projection` ∈ `public` | `sdk` | `diagnostic` (diagnostic = ops token only).
`waitForResult` polls until `product_public.meta.analysis_pending` is false;
`query` = single fetch or poll. Result body:

```json
{
  "ok": true,
  "schema_version": "product_public_v1",
  "session_id": "sess_...",
  "projection": "sdk",
  "sdk_projection": {
    "algo": "sdk_slim_projection_v1",
    "emit_identity": true,
    "cookie_fields": { "user_id": "u9", "plan_tier": "pro" },
    "...": "(slim scores)"
  }
}
```

Business-identifier helper: `cookieFields(result)` / `cookie_fields(result)`
in every language returns the allowlisted cookies or `null`.

## 5. End-to-end smoke (curl)

```bash
# 1. open a session as a site visitor carrying cookies
OPEN=$(curl -fsS -X POST https://gv.example.com/v1/session/open \
  -H 'Content-Type: application/json' \
  -H 'Cookie: user_id=u9; plan_tier=pro; __oauth=skipped' \
  -d '{"site_id":"mysite","visitor_terminal_id":"vt_demo1","meta":{"inject_path":"nginx"}}')
SID=$(printf '%s' "$OPEN" | jq -r .session_id)

# 2. simulate probe batches (or run the real FE), then analyze:
curl -fsS -X POST "https://gv.example.com/v1/session/$SID/analyze" \
  -H "X-Gr-Sdk-Key: $GR_SITE_RESULT_KEY" \
  -H 'Content-Type: application/json' -d '{}' >/dev/null

# 3. query the result with the SDK key
curl -fsS "https://gv.example.com/v1/session/$SID/result?projection=sdk" \
  -H "X-Gr-Sdk-Key: $GR_SITE_RESULT_KEY" \
  | jq '.sdk_projection.cookie_fields'
# → {"user_id":"u9","plan_tier":"pro"}   (oauth cookie NOT captured: not allowlisted)

# 4. wait-for-result in shell SDK (needs an analysis to finish):
GR_SITE_RESULT_KEY=$GR_SITE_RESULT_KEY GR_BASE_URL=https://gv.example.com \
  ./02-probe-analysis/sdk/shell/gr_sdk.sh wait "$SID" sdk 15000 | jq '.sdk_projection.cookie_fields'
```

Health endpoints: `GET {pv}/v1/health`, `GET {gv}/v1/health`, panel
`/api/me` (version), `/api/probe-health`, `/api/lb/status`.

## 6. Security notes

- `cookie_fields` values are **not** PII-scrubbed beyond the blocklist: the
  site owner opted in and the data is their own; keep the allowlist minimal
  and treat values as business identifiers (never raw email/phone without
  hashing in your app).
- Site SDK keys are read-only over results and bound to a site; never ship
  them to the browser.
- Diagnostic projection, evidence, and `analyses` endpoints require ops/admin
  scopes.
- Cookies are captured only for names the owner configured; the sensitive-name
  blocklist is a server-side hard stop on top of that.
