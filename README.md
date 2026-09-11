# GR Server (greenpng)

Browser-side human-verification and anti-bot intelligence — signed probe,
sealed ingest, analysis plane, and per-site result SDKs. This repository is
the **release packaging tree of the 02 probe-analysis product line** and the
**single public entry point for install and updates** (`install.sh` /
updater scripts / panel OTA all point at this repository's Releases).

- Version: see [`VERSION`](VERSION) and the
  [Releases](https://github.com/greenpng/gr-server/releases) page
- License: MIT ([LICENSE](LICENSE))
- Official site (product introduction, EN + 中文): https://www.greenpng.cc

## Documentation

The project guide — what the project is, repository layout & architecture,
and install & usage — is maintained here as **one file per language** under
`docs/`. English is the default; pick your language:

| Language | Guide |
|---|---|
| English | [`docs/README.md`](docs/README.md) |
| 中文 (Chinese) | [`docs/README.zh-CN.md`](docs/README.zh-CN.md) |
| 日本語 (Japanese) | [`docs/README.ja.md`](docs/README.ja.md) |
| 한국어 (Korean) | [`docs/README.ko.md`](docs/README.ko.md) |
| Deutsch (German) | [`docs/README.de.md`](docs/README.de.md) |
| Français (French) | [`docs/README.fr.md`](docs/README.fr.md) |
| Español (Spanish) | [`docs/README.es.md`](docs/README.es.md) |
| Português (Portuguese) | [`docs/README.pt.md`](docs/README.pt.md) |
| Русский (Russian) | [`docs/README.ru.md`](docs/README.ru.md) |
| العربية (Arabic) | [`docs/README.ar.md`](docs/README.ar.md) |
| हिन्दी (Hindi) | [`docs/README.hi.md`](docs/README.hi.md) |
| Bahasa Indonesia | [`docs/README.id.md`](docs/README.id.md) |

The admin panel ships with **English + 中文** locales, kept in sync in
`panel/admin-ui/src/i18n/`.

The rest of this file is the complete operator reference: install,
uninstall, update, admin-panel parameters (including logging), and the
open-source projects the product is built on.

## 1. Install (new node)

Requirements: a Linux host (x86_64 or aarch64), systemd, and either an
existing PostgreSQL (13+) or `--with-docker` to run the data layer
(PostgreSQL + Redis) in Docker. The server itself is always a host binary —
Docker is never the update channel.

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh | bash
# or with an explicit version / arch:
bash install/install.sh --version <VERSION> --arch x86_64 --yes
# with a dockerized data layer (PostgreSQL/Redis only):
bash install/install.sh --version <VERSION> --with-docker --yes
# dry-run (resolve + verify the release manifest without touching the host):
bash install/install.sh --dry-run
```

What the installer does, in order:

1. Resolves the latest compatible release from this repository's Releases
   (any `v<major.minor.patch>`; legacy incompatible majors 6/7/8 are
   excluded) and verifies the **signed manifest chain** (root Ed25519 key
   fingerprint pinned inside `install.sh`).
2. Verifies the bundle sha256 against `manifest-index.json`, then every
   module's ed25519 signature and the runtime ELF before anything is
   written.
3. Installs under `/opt/greenpng` (binary, FE tree, admin SPA, spec/data
   trees, modules), writes `.env` and the `greenpng.service` systemd unit,
   creates the `greenpng` system account, and installs the polkit rule that
   lets the service self-restart for runtime OTA.
4. Stages and activates the six signed modules (identity / brain / analyze /
   ingest / edge / probe_assets), bootstraps the admin database, and gates
   on `/v1/health` before reporting success.

First-login credentials and the **random console path** are written to
`/opt/greenpng/data/admin/admin_bootstrap_once.txt` (mode 0600, one-time —
rotate the password via environment configuration after first login).

## 2. Uninstall

```bash
bash install/uninstall.sh          # standard removal
bash install/uninstall.sh --purge  # also remove the greenpng system account
```

Standard removal stops and disables the three systemd units
(`greenpng.service` and its companion units), removes the polkit self-OTA
rule, the `/opt/greenpng` install tree, runtime locks and OTA caches.
`--purge` additionally removes the system account. **Docker data services
and database volumes are deliberately untouched** — they have their own
lifecycle; drop them with `docker compose -f install/data-compose.yml down
-v` only if you really want the data gone.

## 3. Update (installed node)

| Priority | Channel | For |
|---|---|---|
| P0 | Panel OTA (set-release-url → install / install-fe / install-runtime) | default |
| P1 | `install/release/update_runtime_from_github.sh`, `update_module_from_github.sh` | no panel / free nodes |
| P2 | SSH manual | dead process / first install |

All updates pull the same tag's signed Release assets from this repository.
**Docker is a runtime container, not an update channel.**

- **Modules and FE are hot** — the panel `install` / `install-fe` endpoints
  swap them without a process restart; browsers pick up new FE automatically
  (assets are served from version-immutable, content-hashed URLs).
- **The runtime binary needs a restart** — `install-runtime` swaps the
  binary, restarts the service, and **gates the version stamp on a health
  probe**: on `HEALTH_FAIL` it automatically restores the previous binary,
  FE/spec trees and `VERSION`, so a bad release can never leave a
  half-upgraded, auto-upgrade-locked host.
- **Auto-upgrade** (opt-in via panel `cluster-apply` desired state):
  publishes a target version + apply window (default `04:00-05:30`); nodes
  in the window apply the same health-gated flow. The monotonic version gate
  retries the target instead of locking out.

## 4. The admin panel

Reach it at `http://<host>:28680/<random-console-path>/` (the path itself is
the first access-control layer; find it in `admin_bootstrap_once.txt`).
Login is local admin password by design — the panel is standalone, no
external OAuth. The panel ships English + 中文 (top-right switch).

One-time setup: create a **site** (site id + root domains + the cookie
allow-list for business fields attached to verdicts; sensitive names such
as `password`/`token` are blocked server-side), mint a **site SDK key**, and
deploy the probe tag — three load modes (nginx first-party proxy,
Cloudflare worker in-origin proxy, or app embed); browser uploads always go
**directly to the bound gv domain over TLS**. Result retrieval via the
six-language SDKs (`sdk/`) or plain REST:

```bash
curl -sS -H "X-Gr-Sdk-Key: $KEY" "$BASE/v1/session/$SID/result"
```

### 4.1 Runtime tuning parameters (panel → Config page, hot)

Every parameter below is edited in the panel's Config page
(`POST /api/config` save → `POST /api/config/publish`): **the publishing
node applies immediately, other cluster nodes within ≤30s. No restart.**
The page renders each field with a one-line hint (EN + 中文); the full list
lives there — this table is the operator-oriented summary.

**Rate limits** (shared cluster-wide via PostgreSQL windows):

| Parameter | Default | Meaning |
|---|---|---|
| `rate_limit_open_per_min` | **0 = unlimited** | session opens, per site per minute (total) |
| `rate_limit_ingest_per_min` | **0 = unlimited** | probe batch uploads, per site per minute (total) |
| `rate_limit_analyze_per_min` | **0 = unlimited** | direct analyze calls, per site per minute |
| `rate_limit_complete_per_min` | **0 = unlimited** | complete receipts, per site per minute |
| `rate_limit_result_per_min` | **0 = unlimited** | result reads, per site per minute |
| `rate_limit_client_event_per_min` | **0 = unlimited** | FE telemetry events, per site per minute (total) |
| `rate_limit_client_event_per_ip_per_min` | **100** | FE telemetry events, **per single IP** per minute. Exceeding caps only that IP — other visitors are unaffected. 0 = off |

v1.0.14 policy: aggregate (site-total) caps default **off** so a blunt total
can never throttle real users mixed into bot floods; the telemetry route is
bounded **per IP** instead. Rejections name the layer that tripped
(`client_event:ip` vs `client_event:site`) in the 429 body, and appear
aggregated in the log window line (below). Env seeds (`RATE_LIMIT_*`) still
override pre-publish defaults.

**Flood hardening**: `robot_fastlane_enabled` (default on — UA-declared
crawlers skip L1/L3 storage and analyze arms, early verdict stands),
`hot_max_vts` (8192; 0 = unbounded), `arm_sweep_interval_ms` (15000),
`arm_sweep_cap` (256), `analyze_claim_batch_flood` (16).

**Cycle & hot/cold tiers**: `cycle_cool_ms` (86400000), `cycle_incomplete_ms`
(259200000), `session_inactivity_ms`, `session_hard_max_ms`, `hot_idle_ms`,
`cold_ttl_ms` (604800000), `cold_promote_window_ms` (86400000),
`cold_purge_interval_ms` (300000), `complete_on_commercial_silicon` (true).

**Analyze triggers & FE upload**: `analyze_idle_upload_ms` (20000),
`analyze_debounce_ms` (40), `return_identity_idle_ms` (45000),
`rpa_idle_analyze_ms` (25000), retry ladder `hard_max_attempts` (8) /
`soft_max_attempts` (4) / `deepen_max_attempts` (6) / `rpa_max_attempts` (3),
`fail_budget_n` (24) per `fail_budget_window_ms` (90000), `hard_sla_retries`
(5, base delay 2000 ms), `upload_concurrency` (6) → `upload_mid_ramp` (12)
after `upload_ramp_after` (14), `upload_max_retries` (5),
`client_alive_retry_ms` (30000), `multi_tick_max` (96), `empty_kick_patience`
(20).

### 4.2 Workers, retention, sites (panel, hot)

| Area | Parameters | Default / timing |
|---|---|---|
| Workers | analyze / ingest / gateway counts | analyze hot-rescales immediately; gateway/ingest via cluster desired state (≤30s) |
| Retention | `analysis_retention_days` 30 · `session_retention_days` 30 · `velocity_retention_days` 7 · `ops_retention_days` 14 · `master_retention_days` 90 · `cold_ttl_days` 7 · `batch_delete_limit` 200 · `purge_interval_sec` 300 | next purge cycle (≥30s); manual purge endpoint runs bounded batches immediately |
| Sites | collect on/off, strategy (`observe_only` / `bot_control`), domains + SSL certs, SDK keys | collect switch and strategy effective on the **next request**; domains/SSL hot via SNI; SDK keys usable the moment they are minted |
| Cluster | desired state, apply window, node heartbeats | publishing node immediate, cluster ≤30s |

## 5. Logging & observability

The service logs to **journald**: `journalctl -u greenpng.service`. Normal
operation is INFO-level and quiet by design; what to look for:

| Line | Meaning |
|---|---|
| `ingest_ack … durability_state=stored_durable` | every accepted probe batch, with source + batch type |
| `analyze claimed n=… worker=…` | analyze scheduler claims (throttled 5s/worker) |
| `auto-analyze complete sid=… rev=… eval_ms=…` | verdict produced, with evaluation time |
| `arm sweep armed=…` | hot→cold sweep armed sessions (idle sweeps stay silent) |
| `cold purge loop started / retention purge loop started` | background TTL/retention ownership at boot |
| `http_4xx_5xx_30s n=… sample="…"` | **aggregated** 4xx/5xx window (count + one sample per 30s) — adversarial reject/rate-limit storms surface as one WARN line instead of flooding the journal |
| `pg worker reconnected / admin pg reconnected / control admin pg reconnected` | PostgreSQL self-heal after a database restart (bounded backoff, no operator action) |
| queue-over-cap | re-warns every 60s while the analyze queue is over its soft cap |

**Ops event streams** (queryable in the panel's 结果/审计 pages and via
`/v1/ops/events/export`): `ops_server_events` (sealed accepts, protocol
rejects, throttles) and `ops_client_events` (FE-reported diagnostics —
secrets and raw series stripped server-side, IPs stored as /24). Retention
`ops_retention_days` (default 14) bounds their growth.

**Log-related switches**: FE telemetry upload can be disabled entirely with
the admin setting `ops_client_events_enabled=0` (or env
`GR_OPS_CLIENT_EVENTS=0`); the rate limiter can be forced on/off in lab
shapes with `GR_RATE_LIMIT_FORCE=1` / `GR_RATE_LIMIT_OFF=1`. Admin actions
(login, OTA, config publish, key mint/rotate) are all audit-logged with
actor + detail in the panel's Audit page.

## 6. Repository layout

| Directory | Contents |
|---|---|
| `crates/` | Rust workspace — gr-service (control/probe/gateway), gr-probe-core, gr-probe-plane, gr-probe-store, gr-ota, gr-admin, gr-runtime, … |
| `modules/` | Signed hot-update module sources (identity / brain / analyze / ingest / edge / probe_assets) |
| `probe/` | Browser probe FE (loader, gv5.seal.js pack chain, sealed ingest) |
| `panel/` | Admin panel — Vue sources (`admin-ui/`) + built SPA (`admin-spa/`), EN + 中文 |
| `sdk/` | Backend integration SDKs (result retrieval, six languages) |
| `spec/` | Wire/scoring specs loaded at runtime (not documentation) |
| `fixtures/` | Contract-test data |
| `scripts/` | Build scripts and FE tooling (incl. `scripts/fe/checks/` FE static contract checks) |
| `vendor/` | Vendored dependency sources (pingora) |
| `install/` | Installer + data-layer compose + upgrade/uninstall scripts |
| `release/` | Packaging scripts (build_multiarch / SBOM / module signing) |
| `docs/` | Project guide, one file per language |
| `VERSION` | Release version, single source of truth |

Full documentation of workstreams and lab tests lives in the development
tree and is intentionally not mirrored here; `docs/` in this repository is
the public documentation surface.

## 7. Build and test from source

```bash
# Toolchain: the Rust version pinned in rust-toolchain.toml
cargo build --release -p gr-service -p gr-cli

# Contract tests (same set as CI Gate A)
cargo test -p gr-probe-store --lib
cargo test -p gr-probe-plane --lib
cargo check -p gr-service -p gr-admin
npm ci --prefix panel/admin-ui && npm run build --prefix panel/admin-ui

# FE static contract checks
for f in scripts/fe/checks/*.js; do node "$f"; done
```

Release artifacts are produced only by this repository's CI
(`gate-a.yml` → `release.yml`) and re-verified by Gate B install tests
before publishing.

## 8. Open-source projects

### 8.1 Vendored into the tree

| Project | License | Role |
|---|---|---|
| [Pingora](https://github.com/cloudflare/pingora) (Cloudflare) | Apache-2.0 | the probe plane's serving layer — TLS/SNI gateway, HTTP/3-capable proxy under `vendor/pingora/`, pinned and patched in-tree |

### 8.2 Major dependencies (crates.io)

| Crate | Role |
|---|---|
| axum / tower-http / tokio | control-plane HTTP API and async runtime |
| postgres / rusqlite | PostgreSQL (multi-worker) and SQLite (single-node lab) storage drivers |
| ed25519-dalek / x25519-dalek / aes-gcm / hkdf / hmac / scrypt | release signing + verification, session seals, key derivation, credential hashing |
| sha2 / blake3 / hex / base64 | digests and encodings across manifest + seal chains |
| reqwest (rustls) | release/OTA fetches, webhook delivery |
| dashmap / arc-swap / parking_lot | lock-free shared state (hot maps, config swap) |
| tracing / tracing-subscriber | structured logging |
| serde / serde_json / chrono / uuid / semver / regex / clap / sysinfo / once_cell / flate2 | serialization, time, CLI, system metrics, compression |

A full machine-readable inventory ships with every release as
**`sbom.cdx.json`** (CycloneDX), generated by `release/cargo_cyclonedx.py`.

### 8.3 Referenced / built-on infrastructure

| Project | How it is used |
|---|---|
| PostgreSQL | the data layer for sessions, batches, verdicts, admin/biz/association stores |
| Redis | multi-node coordination and shared state |
| nginx | first-party probe-script load mode (pv domain proxy) and TLS fronting options |
| systemd + polkit | service lifecycle and the self-OTA restart grant |
| Cloudflare (CDN + Workers) | optional CDN/worker load mode in front of the probe plane |

Product design references: the sealed-ingest and signed-asset-chain model
follows the same trust principles as signed package repositories (signed
manifest index → per-asset signature → pinned root key), and the
probe-plane architecture follows Cloudflare's Pingora service model
(vendored above). No third-party probe/anti-bot code is included — the
probe, scoring and analysis code in `crates/` and `modules/` is original to
this project.

## 9. Security

- Release assets are signed by the root Ed25519 key; the installer pins the
  public-key fingerprint (`OTA_ROOT_PUBKEY_SHA256` inside `install.sh`).
- The private key exists only in GitHub Secrets and never enters the tree;
  `ota_ed25519.pk` in this repository is the public key.
- Tags and Release assets are immutable: fixes ship as new PATCH versions,
  history is never rewritten.
- Runtime OTA is health-gated with automatic rollback (see §3); the panel
  console path is random and the session cookie follows the request scheme.
