# greenpng — Product Guide (English)

Browser-side human-verification and anti-bot intelligence: a signed probe
that runs in real visitors' browsers, a sealed ingest pipeline, and an
analysis plane that returns per-session verdicts through SDKs in six
languages.

> This guide is available in 12 languages — see the
> [language index](../../README.md#documentation) at the repository root.

## 1. What greenpng does

Every visitor session is scored on-device and server-side:

- **Real vs bot verdict** — `human | watch | bot` with per-axis confidence
  (input dynamics, device stack consistency, environment authenticity,
  automation traces, history reuse).
- **Stable device identity** — a collision-aware device ID that stays
  stable across sessions without relying on third-party cookies.
- **Business field capture** — your allow-listed cookie fields
  (`user_id`, `plan_tier`, …) are captured at session open and attached to
  the verdict. Sensitive names (password/token/…) are blocked server-side.
- **IP intelligence** — visitor IPs are privacy-masked to /24 (IPv4) or
  /48 (IPv6) at ingestion; optional enrichment maps the subnet to
  ASN/country/city via DB-IP (bundled MMDB), IPinfo, MaxMind, or a custom
  HTTP enricher. Secrets never leave the server.
- **Result retrieval API** — merchants poll the verdict with a per-site
  SDK key; projections (`public | sdk | diagnostic`) control exposure.

## 2. Key features

| Feature | What it gives you |
|---|---|
| Signed FE probe | Tamper-evident browser packs (ed25519), versioned immutable asset URLs `/dist/v/<ver>/g/<gen>/…` — CDN-safe, no cache poisoning |
| Sealed ingest | Pack submissions are sealed; replay/tamper rejected upstream |
| Analysis modules (OTA) | identity / brain / analyze / ingest / edge / probe_assets hot-update as signed modules without downtime |
| Admin panel | Standalone console at a random path, single scrypt admin, audit log, site/strategy/integration/retention/DSAR management, EN + 中文 |
| Privacy by default | IP masking at the earliest ingestion point, cookie allow-list, DSAR export/erase, retention purge |
| Six-language SDKs | JS / Python / Go / PHP / Shell / Rust clients for `wait_for_result` |
| Multi-node ready | Built-in LB module, cluster heartbeat, OTA mirror |

## 3. Architecture

```
            visitor browser
                  │  <script src="/gr.js"> (pinned, no-store)
                  ▼
        FE loader ──► /v1/sdk/bootstrap ──► versioned pack manifest
                  │        (asset_base /dist/v/<fe>/g/<gen>/)
                  ▼
        pack collectors (input, device, environment)
                  │  sealed submit
                  ▼
   ┌──────────────┴───────────────┐
   │ gr-probe-plane (Pingora)      │  gateway / ingest / session APIs
   │  ├─ B8 gateway (TLS SNI)      │  bound-domain direct upload
   │  ├─ sealed ingest + batching  │
   │  └─ /v1/ops/* (token-gated)   │
   └──────────────┬───────────────┘
                  ▼
        gr-service (control plane)
          ├─ admin console + admin API (axum)
          ├─ site / strategy / integration config
          ├─ startup repair: panel policy + site backfill
          └─ module registry (OTA, signed)
                  ▼
        PostgreSQL (sessions, probe_batches, analysis_latest, admin)
                  ▼
        GET /v1/session/{id}/result  ──► merchant SDK (6 languages)
```

**Deployment shapes**

- **gr-service** — one process: admin console + control APIs + in-tree
  probe plane. Port 28680 (console, random path) + 28765 (plane loopback).
- **Business site nginx** — serves `/gr.js` and (optionally) proxies the
  same-origin API prefix; browser uploads must use the bound GV domain
  through Pingora TLS.
- **DBs** — PostgreSQL for control + probe stores (SQLite skeleton exists
  for the lab).

## 4. Quick install

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh
bash install.sh --version <VERSION>            # see the VERSION file / Releases
# or with a dockerized data layer:
bash install.sh --version <VERSION> --with-docker --yes
```

The installer verifies sha256 + ELF + ed25519 module signatures, installs
under `/opt/greenpng`, writes `.env`, stages and activates the six signed
modules, enables the systemd unit, and gates on `/v1/health`.

First login: one-time credentials are written to
`/opt/greenpng/data/admin/admin_bootstrap_once.txt` — the random console
path (`/c-<hex>/`) is recorded there. There is no `/admin` or `/console/`
prefix, and password login is the only panel entry.

## 5. Usage tutorial

### 5.1 Create a site

Panel **Sites → Create site**:

- `site_id` — your tenant id, used in the embed
- `root_domains` — the www hostnames (CORS + hostname binding)
- `cookie_fields` — the cookie allow-list, e.g.
  `["user_id", "plan_tier", "cart_id", "utm_source"]`

The site row flows `control.sites → public.sites` (probe plane) on save,
and gr-service backfills pre-existing sites at startup.

### 5.2 Deploy the probe (three modes)

**A. Nginx first-party (recommended)** — your site vhost proxies the boot
loader and the same-origin API prefix:

```nginx
location = /gr.js      { proxy_pass https://pv.example.com/gr.js; proxy_set_header Host pv.example.com; }
location /gr/dist/v/   { proxy_pass https://pv.example.com/gr/dist/v/; proxy_set_header Host pv.example.com; }
location /gr/v1/       { proxy_pass https://gv.example.com/; proxy_set_header Cookie $http_cookie;
                         proxy_set_header X-Forwarded-For $remote_addr; }
```

```html
<script src="/gr.js" data-site-id="mysite" data-endpoint="/gr" data-inject-path="nginx" defer></script>
```

**B. Cloudflare worker** — the worker injects the boot script and proxies
`/gr` in-origin. Avoid "Under Attack" mode on `/gr` (challenge pages break
the probe).

**C. Site scripting / CDN embed** — load the boot JS directly from PV and
point `data-endpoint` at GV.

In every mode the loader resolves packs through the SDK bootstrap and only
fetches versioned immutable URLs.

### 5.3 Receive results (six-language SDK)

Create a **backend key** for your site in the panel (SDK page). The SDK
never relays probes; it only queries results:

```
GET {gv_base}/v1/session/{session_id}/result?projection=sdk
X-Gr-Sdk-Key: <site backend key>
```

| Language | Entry point |
|---|---|
| JS | `sdk/src/index.js` — `new GrResultClient({baseUrl, apiKey}).waitForResult(...)` |
| Python | `sdk/python/gr_results.py` — `GrResultClient(base_url, api_key).wait_for_result(...)` |
| Go | `sdk/go/gr_results.go` — `gr.New(baseURL, key).WaitForResult(...)` |
| PHP | `sdk/php/GrResultsClient.php` — `(new GrResultClient(...))->waitForResult(...)` |
| Shell | `sdk/shell/gr_sdk.sh` — `gr_wait_for_result <session> sdk 8000` |
| Rust | `sdk/rust/` — `Client::new(base_url, key).wait_for_result(...)` |

Site keys are scoped: a key minted for site A cannot read site B's
sessions (`403 sdk key site mismatch`), and revoked keys stop working
immediately (`401`).

### 5.4 End-to-end smoke (curl)

```bash
OPEN=$(curl -fsS -X POST https://gv.example.com/v1/session/open \
  -H 'Content-Type: application/json' \
  -H 'Cookie: user_id=u9; plan_tier=pro' \
  -d '{"site_id":"mysite","visitor_terminal_id":"vt_demo1"}')
SID=$(printf '%s' "$OPEN" | jq -r .session_id)

curl -fsS -X POST "https://gv.example.com/v1/session/$SID/analyze" \
  -H "X-Gr-Sdk-Key: $GR_SITE_RESULT_KEY" -H 'Content-Type: application/json' -d '{}'

curl -fsS "https://gv.example.com/v1/session/$SID/result?projection=sdk" \
  -H "X-Gr-Sdk-Key: $GR_SITE_RESULT_KEY" | jq '.sdk_projection'
```

### 5.5 Keep it updated

| Channel | Command |
|---|---|
| Panel OTA (default) | Admin panel → Modules / Runtime install |
| Updater script | `VERSION=<v> INSTALL_ROOT=/opt/greenpng bash install/release/update_runtime_from_github.sh` |
| Manual | SSH + previous runtime rollback (`bin/releases/<v>` kept) |

Every update pulls the same signed Release assets from this repository.

## 6. Repository layout

| Directory | Contents |
|---|---|
| `crates/` | Rust workspace — gr-service, gr-probe-core, gr-probe-plane, gr-probe-store, gr-ota, gr-admin, gr-runtime, … |
| `modules/` | Signed hot-update module sources (identity / brain / analyze / ingest / edge / probe_assets) |
| `probe/` | Browser probe FE (loader, packs, sealed ingest) |
| `panel/` | Admin panel — Vue sources (`admin-ui/`) + built SPA (`admin-spa/`) |
| `sdk/` | Result-retrieval SDKs (JS / Python / Go / PHP / Shell / Rust) |
| `spec/` | Runtime-loaded specs (bot weights, catalogs) |
| `install/` | Installer + updater + systemd material |
| `release/` | Packaging scripts (multi-arch bundle, SLSA attestation) |
| `scripts/` | FE contract checks and helpers |
| `docs/` | This guide in 12 languages |
| `VERSION` | Single source of truth for the release version |

## 7. Links

- Releases & install entry point: this repository
- Official site: https://www.greenpng.cc (product introduction, EN + 中文)
- Panel locales: English + 中文 (kept in sync in `panel/admin-ui/src/i18n/`)
