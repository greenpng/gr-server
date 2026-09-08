# greenpng — Project Guide

## 1. What this project is

greenpng (GR) verifies whether a session is a real human by probing the
**real visitor's browser**, not by inspecting traffic alone.

The pipeline, end to end:

1. **Probe (in the browser)** — a signed FE loader runs on your pages and
   collects evidence from **multiple sources** (input dynamics, device stack,
   environment authenticity, automation traces) in **multiple staged batches**
   per session.
2. **Upload** — the browser submits every batch to the server through a
   sealed ingest pipeline; replayed or tampered submissions are rejected
   before they reach storage.
3. **Analyze (on the server)** — the analysis plane scores each session into
   a per-session verdict (`human | watch | bot`) with per-axis confidence and
   a stable, collision-aware device identity.
4. **Return** — your backend retrieves the result through the result API
   (six-language SDKs; `public | sdk | diagnostic` projections control what
   each caller sees).

On the server side, greenpng runs as one host binary with an in-tree probe
plane by default, and supports **multi-node, load-balanced deployment**: an
LB module spreads probe traffic across nodes in front of a shared data layer
(PostgreSQL + Redis), so collection and analysis scale horizontally.

## 2. Repository layout & architecture

| Directory | Contents |
|---|---|
| `crates/` | Rust workspace — `gr-service` (control plane + admin console + in-tree probe plane), `gr-probe-core`, `gr-probe-plane`, `gr-probe-store`, `gr-ota`, `gr-admin`, `gr-runtime`, … |
| `modules/` | Signed hot-update module sources (identity / brain / analyze / ingest / edge / probe_assets) |
| `probe/` | Browser probe FE (loader, pack chain, sealed ingest client) |
| `panel/` | Admin panel — Vue sources (`admin-ui/`) + built SPA (`admin-spa/`), EN + 中文 |
| `sdk/` | Backend integration SDKs in six languages (result retrieval only) |
| `spec/` | Wire/scoring specs loaded at runtime |
| `fixtures/` | Contract-test data |
| `scripts/` | Build scripts and FE tooling (`scripts/fe/checks/`) |
| `vendor/` | Vendored dependency sources (pingora) |
| `install/` | Installer, data-layer compose, upgrade scripts |
| `release/` | Packaging scripts (multi-arch build, SBOM, module signing) |
| `docs/` | This guide, one file per language |

```
        visitor browser
              │  <script src="/gr.js">  (pinned, no-store)
              ▼
   FE loader ──► /v1/sdk/bootstrap ──► versioned pack manifest
              │        asset_base /dist/v/<fe>/g/<gen>/
              ▼
   pack collectors (input · device · environment, multi-batch)
              │  sealed submit (direct to the bound gv domain, TLS)
              ▼
 ┌────────────┴─────────────┐   ┌──────────────────────────────┐
 │ gr-probe-plane (Pingora)  │   │ gr-service (control plane)    │
 │  gateway · ingest · ops   │◄──┤  admin console · site config  │
 └────────────┬─────────────┘   │  module registry (OTA, signed)│
              ▼                 └──────────────┬───────────────┘
   PostgreSQL (+ Redis in multi-node)          │
              ▼                                │
   GET /v1/session/{id}/result ──► merchant SDK (six languages)

   multi-node: LB module spreads probe traffic across gr-service nodes
   in front of the shared data layer
```

Key components: `gr-service` is the control plane (random-path admin console,
site/config management, signed OTA module registry) and hosts the probe plane
in-tree; `gr-probe-plane` is the Pingora gateway with sealed ingest and
session/result APIs; the browser FE resolves every asset to a
version-immutable URL so caches can never serve a stale probe across
releases; PostgreSQL (plus Redis when multi-node) stores sessions, batches
and analysis results.

## 3. Install & usage

### 3.1 Install

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh | bash
# or with an explicit version / arch:
bash install/install.sh --version <VERSION> --arch x86_64 --yes
# with a dockerized data layer (PostgreSQL/Redis only; the server stays a host binary):
bash install/install.sh --version <VERSION> --with-docker --yes
```

The installer verifies sha256 + ELF + ed25519 module signatures, installs
under `/opt/greenpng`, writes `.env` and the systemd unit, stages and
activates the six signed modules, and gates on the control/plane
`/v1/health`. First-login credentials and the random console path are
written to `/opt/greenpng/data/admin/admin_bootstrap_once.txt`.

### 3.2 Update

| Priority | Channel | For |
|---|---|---|
| P0 | Panel OTA (set-release-url → install / install-fe / install-runtime) | default |
| P1 | `install/release/update_runtime_from_github.sh`, `update_module_from_github.sh` | no panel / free nodes |
| P2 | SSH manual | dead process / first install |

All updates pull the same tag's signed Release assets from this repository.
Docker is a runtime container, not an update channel.

### 3.3 Use

1. **Create a site** in the admin panel: site id, root domains, and the
   cookie allow-list for business fields you want attached to each verdict
   (sensitive names such as `password`/`token` are blocked server-side).
2. **Deploy the probe** — three modes:
   - *Nginx first-party (recommended)*: proxy `/gr.js` + `/gr/dist/v/` to the
     pv domain and `/gr/v1/` to the gv domain (Cookie passthrough), inject
     `<script src="/gr.js" data-site-id="…" data-endpoint="/gr"
     data-inject-path="nginx" defer></script>` into HTML.
   - *Cloudflare worker*: inject the same tag and proxy `/gr` in-origin.
   - *App embed*: load the loader directly from pv/CDN with
     `data-endpoint` pointing at gv/pv.
   Browser uploads always go **directly to the bound gv domain over TLS**.
3. **Receive results** — create a site SDK key in the panel, then poll:

```bash
KEY="grsk_..." SID="cycle_..." BASE="https://gv.example.com"
curl -sS -H "X-Gr-Sdk-Key: $KEY" "$BASE/v1/session/$SID/result"
```

4. **Smoke** (curl):

```bash
# open a session carrying allowlisted cookies
curl -sS -X POST "$BASE/v1/session/open" -H "X-Gr-Sdk-Key: $KEY" \
     -H 'Cookie: user_id=u9; plan_tier=pro' -d '{"site_id":"mysite"}'
# → {"session_id":"cycle_…", …}
# then check the result (after real FE batches or simulated ones):
curl -sS -H "X-Gr-Sdk-Key: $KEY" "$BASE/v1/session/$SID/result"
```
