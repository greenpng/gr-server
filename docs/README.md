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

- **Modules and FE are hot** — panel `install` / `install-fe` swap them
  without a process restart; browsers pick up new FE automatically
  (version-immutable, content-hashed asset URLs).
- **The runtime binary needs a restart** — `install-runtime` swaps the
  binary, restarts the service, and **gates the version stamp on a health
  probe**; on `HEALTH_FAIL` it restores the previous binary, FE/spec trees
  and `VERSION` automatically (no half-upgraded, auto-upgrade-locked host).
- **Auto-upgrade** (opt-in via panel `cluster-apply`): target version + apply
  window (default `04:00-05:30`); nodes apply the same health-gated flow.
- **Memory note (long-running hosts)**: the installer writes
  `MALLOC_ARENA_MAX=4` into `/opt/greenpng/.env` — glibc's extra 64 MB
  malloc arenas outlive their threads and hold their high-water mark, which
  otherwise shows up as a slow RSS ratchet on multi-threaded hosts
  (~1.5 GB after 1 h measured on a 6-core node). Existing installs: append
  the line manually and restart; RSS settles at the working-set level
  (~100-200 MB measured). See root README §3 for details.

### 3.3 Uninstall

```bash
bash install/uninstall.sh          # standard removal
bash install/uninstall.sh --purge  # also remove the greenpng system account
```

Stops/disables/removes the systemd units, the polkit self-OTA rule, the
`/opt/greenpng` install tree, runtime locks and OTA caches (`--purge` also
the system account). **Docker data services and database volumes are
deliberately untouched** — drop them with
`docker compose -f install/data-compose.yml down -v` only if you want the
data gone.

### 3.4 Use

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

## 4. Admin panel parameters

Everything below is edited in the panel's **Config page** (save → publish):
**publishing node applies immediately, cluster nodes ≤30s, no restart.**
Each field renders with a one-line hint (EN + 中文); this is the summary.

**Rate limits** (shared cluster-wide via PostgreSQL windows):

| Parameter | Default | Meaning |
|---|---|---|
| `rate_limit_open_per_min` | **0 = unlimited** | session opens / site / min (total) |
| `rate_limit_ingest_per_min` | **0 = unlimited** | batch uploads / site / min |
| `rate_limit_analyze_per_min` | **0 = unlimited** | direct analyze / site / min |
| `rate_limit_complete_per_min` | **0 = unlimited** | complete receipts / site / min |
| `rate_limit_result_per_min` | **0 = unlimited** | result reads / site / min |
| `rate_limit_client_event_per_min` | **0 = unlimited** | FE telemetry / site / min (total) |
| `rate_limit_client_event_per_ip_per_min` | **100** | FE telemetry **per single IP** / min — exceeding caps only that IP; 0 = off |

v1.0.14 policy: site totals default **off** (an aggregate cap can throttle
real users mixed into bot floods); the telemetry route is bounded per-IP
instead. 429 bodies name the tripped layer (`client_event:ip` /
`client_event:site`).

**Behind a CDN** the per-IP layer keys on the IP the server sees — without
real-IP restoration at the fronting proxy that is a **CDN edge IP**. The
server honors `X-Real-IP`/`X-Forwarded-For` only from a trusted proxy peer
(loopback; `GR_TRUSTED_PROXIES` to extend) and never trusts client-supplied
headers directly. With nginx in front, restore visitor IPs with the
`realip` module (ranges from the CDN's published list, header
`CF-Connecting-IP` for Cloudflare / `True-Client-IP` for others) so
per-IP limiting and telemetry attribution key on visitors. Only
connections from the listed ranges get the header honored — forged
headers from direct-to-origin clients stay keyed by their own address.
See the root README §4.1 for a worked example (production-verified
2026-09-11).

**Flood hardening**: `robot_fastlane_enabled` (on), `hot_max_vts` (8192),
`arm_sweep_interval_ms` (15000), `arm_sweep_cap` (256),
`analyze_claim_batch_flood` (16).

**Cycle & tiers**: `cycle_cool_ms` (86400000), `cycle_incomplete_ms`
(259200000), `session_inactivity_ms`, `session_hard_max_ms`, `hot_idle_ms`,
`cold_ttl_ms` (604800000), `cold_promote_window_ms` (86400000),
`cold_purge_interval_ms` (300000), `complete_on_commercial_silicon` (on).

**Analyze & FE upload**: `analyze_idle_upload_ms` (20000),
`analyze_debounce_ms` (40), `return_identity_idle_ms` (45000),
`rpa_idle_analyze_ms` (25000), `hard_max_attempts` (8) / `soft_max_attempts`
(4) / `deepen_max_attempts` (6) / `rpa_max_attempts` (3), `fail_budget_n`
(24) / `fail_budget_window_ms` (90000), `hard_sla_retries` (5, 2000 ms
base), `upload_concurrency` (6) → `upload_mid_ramp` (12) after
`upload_ramp_after` (14), `upload_max_retries` (5), `client_alive_retry_ms`
(30000), `multi_tick_max` (96), `empty_kick_patience` (20).

**Workers / retention / sites**:

| Area | Defaults | Timing |
|---|---|---|
| Workers | analyze / ingest / gateway counts | analyze hot-rescale immediate; others via cluster desired (≤30s) |
| Retention | analysis 30d · session 30d · velocity 7d · ops 14d · master 90d · cold_ttl 7d · batch 200 · interval 300s | next purge cycle; manual purge immediate (bounded batches) |
| Sites | collect on/off, strategy, domains/SSL, SDK keys | collect/strategy next request; SSL hot (SNI); keys immediate |

## 5. Logging & observability

Logs go to journald (`journalctl -u greenpng.service`), INFO-level and
quiet by design:

| Line | Meaning |
|---|---|
| `ingest_ack … durability_state=stored_durable` | accepted probe batch (source + batch type) |
| `analyze claimed n=… worker=…` | scheduler claims (throttled 5s/worker) |
| `auto-analyze complete sid=… rev=… eval_ms=…` | verdict + evaluation time |
| `arm sweep armed=…` | hot→cold sweep armed sessions (idle sweeps silent) |
| `cold purge loop / retention purge loop started` | background TTL/retention ownership at boot |
| `http_4xx_5xx_30s n=… sample="…"` | aggregated 4xx/5xx window (one WARN per 30s instead of per-request flooding) |
| `pg worker reconnected / admin pg reconnected / control admin pg reconnected` | PostgreSQL self-heal after a DB restart |
| queue-over-cap | re-warns every 60s while over the soft cap |

**Ops event streams** (panel 结果/审计 pages, `/v1/ops/events/export`):
`ops_server_events` (sealed accepts, protocol rejects, throttles) and
`ops_client_events` (FE diagnostics; secrets/raw series stripped, IPs
stored as /24), bounded by `ops_retention_days` (14).

**Switches**: FE telemetry upload off via admin setting
`ops_client_events_enabled=0` (or `GR_OPS_CLIENT_EVENTS=0`); limiter forced
on/off in lab shapes via `GR_RATE_LIMIT_FORCE=1` / `GR_RATE_LIMIT_OFF=1`.
Admin actions are audit-logged (actor + detail) in the panel Audit page.

## 6. Open-source projects

**Vendored**: [Pingora](https://github.com/cloudflare/pingora)
(Apache-2.0, Cloudflare) — the probe plane's serving layer under
`vendor/pingora/`.

**Major crates.io dependencies**: axum / tower-http / tokio (control-plane
HTTP + async), postgres / rusqlite (storage), ed25519-dalek / x25519-dalek /
aes-gcm / hkdf / hmac / scrypt (signing, seals, credentials), sha2 / blake3
(digests), reqwest-rustls (OTA/webhook fetch), dashmap / arc-swap /
parking_lot (shared state), tracing (logging), serde / chrono / uuid /
semver / regex / clap / sysinfo / flate2 (utilities). Full inventory per
release: **`sbom.cdx.json`** (CycloneDX).

**Built-on infrastructure**: PostgreSQL (data layer), Redis (multi-node),
nginx (first-party load mode), systemd + polkit (lifecycle + self-OTA
grant), Cloudflare CDN/Workers (optional front). Design references: signed
package-repository trust chains (manifest index → per-asset signature →
pinned root key) and Cloudflare's Pingora service model. No third-party
probe/anti-bot code is included — probe, scoring and analysis code is
original to this project.
