<p align="center">
  <a href="https://github.com/greenpng/gr-server">
    <img src="https://raw.githubusercontent.com/greenpng/gr-server/main/docs/assets/logo.svg" alt="greenpng logo" width="260" />
  </a>
</p>

<h3 align="center">Enterprise Browser Human Verification & Real-time Anti-Bot Intelligence</h3>

<p align="center">
  A signed, anti-tampering client probe, a sealed upstream ingest pipeline powered by Cloudflare Pingora,
  and instant sub-millisecond fraud verdicts with stable device identities.
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg?style=flat-square" alt="License MIT" /></a>
  <a href="VERSION"><img src="https://img.shields.io/badge/version-1.0.14-00ff88.svg?style=flat-square" alt="Version 1.0.14" /></a>
  <a href="https://rust-lang.org"><img src="https://img.shields.io/badge/rust-2021_edition-orange.svg?style=flat-square" alt="Rust Edition 2021" /></a>
  <a href="https://github.com/cloudflare/pingora"><img src="https://img.shields.io/badge/gateway-Pingora_Rust-00e5ff.svg?style=flat-square" alt="Pingora Powered" /></a>
  <a href="https://github.com/greenpng/gr-server/actions"><img src="https://img.shields.io/badge/build-passing-brightgreen.svg?style=flat-square" alt="Build Passing" /></a>
  <a href="https://www.greenpng.cc"><img src="https://img.shields.io/badge/website-greenpng.cc-blueviolet.svg?style=flat-square" alt="Official Website" /></a>
</p>

---

## 📖 Multilingual Documentation / 多语言文档

The documentation is maintained in **12 languages** under the [`docs/`](docs/) directory:

| Language | Guide Link | Language | Guide Link |
|---|---|---|---|
| 🇬🇧 **English (Default)** | [`docs/README.md`](docs/README.md) | 🇨🇳 **简体中文 (Chinese)** | [`docs/README.zh-CN.md`](docs/README.zh-CN.md) |
| 🇯🇵 **日本語 (Japanese)** | [`docs/README.ja.md`](docs/README.ja.md) | 🇰🇷 **한국어 (Korean)** | [`docs/README.ko.md`](docs/README.ko.md) |
| 🇩🇪 **Deutsch (German)** | [`docs/README.de.md`](docs/README.de.md) | 🇫🇷 **Français (French)** | [`docs/README.fr.md`](docs/README.fr.md) |
| 🇪🇸 **Español (Spanish)** | [`docs/README.es.md`](docs/README.es.md) | 🇵🇹 **Português (Portuguese)** | [`docs/README.pt.md`](docs/README.pt.md) |
| 🇷🇺 **Русский (Russian)** | [`docs/README.ru.md`](docs/README.ru.md) | 🇸🇦 **العربية (Arabic)** | [`docs/README.ar.md`](docs/README.ar.md) |
| 🇮🇳 **हिन्दी (Hindi)** | [`docs/README.hi.md`](docs/README.hi.md) | 🇮🇩 **Bahasa Indonesia** | [`docs/README.id.md`](docs/README.id.md) |

The built-in Admin Console ships with **English + 简体中文** locales (`panel/admin-ui/src/i18n/`).

---

## 📑 Table of Contents

- [1. Overview \& Architecture](#1-overview--architecture)
- [2. Key Capabilities](#2-key-capabilities)
- [3. Installation (New Node)](#3-installation-new-node)
- [4. Uninstallation](#4-uninstallation)
- [5. Updates \& OTA Upgrades](#5-updates--ota-upgrades)
- [6. Admin Panel \& Parameter Configurations](#6-admin-panel--parameter-configurations)
  - [6.1 Accessing the Admin Console](#61-accessing-the-admin-console)
  - [6.2 Rate Limits (Anti-DDoS \& API Protection)](#62-rate-limits-anti-ddos--api-protection)
  - [6.3 CDN Real-IP Restoration](#63-cdn-real-ip-restoration)
  - [6.4 Flood Hardening \& Robot Fastlane](#64-flood-hardening--robot-fastlane)
  - [6.5 Hot/Cold Tiering \& Retention Policies](#65-hotcold-tiering--retention-policies)
  - [6.6 Analysis Triggers \& Client Retries](#66-analysis-triggers--client-retries)
- [7. Client Probe Deployment](#7-client-probe-deployment)
- [8. Multi-Language SDKs](#8-multi-language-sdks)
- [9. Open-Source Projects \& References](#9-open-source-projects--references)
- [10. Security \& Trust Model](#10-security--trust-model)

---

## 1. Overview & Architecture

**greenpng** (GR) scores every visitor session inside the **real browser DOM** rather than relying merely on superficial network-level heuristics or easily spoofed headers.

```
        Visitor Browser
              │  <script src="/gr.js">  (pinned, no-store, CDN-safe)
              ▼
    FE Loader ──► /v1/sdk/bootstrap ──► Versioned Pack Manifest
              │        asset_base /dist/v/<fe>/g/<gen>/
              ▼
    Staged Collectors (Input · Hardware · System · WebGL · Silicon · Automation)
              │  Sealed Envelopes (Direct upload to bound GV domain over TLS)
              ▼
  ┌───────────────────────────┐         ┌──────────────────────────────┐
  │  gr-probe-plane (Pingora) │         │ gr-service (Control Plane)   │
  │   TLS Proxy · Ingest Gate │◄────────┤  Admin Console (Random Path) │
  │   B8 TLS SNI Verification │         │  OTA Module Registry (Signed)│
  └─────────────┬─────────────┘         └──────────────┬───────────────┘
                ▼                                      │
       Tiered Data Layer (PostgreSQL + Redis)          │
                ▼                                      │
  GET /v1/session/{id}/result ─────────────────────────┘
                ▼
     Merchant Backend SDKs (Go, Rust, TypeScript, Python, PHP, Shell)
```

- **`gr-service`**: Unified control plane managing administrative sites, secrets, audit trails, and cryptographically verified OTA module distributions.
- **`gr-probe-plane`**: High-performance reverse proxy built on **Cloudflare Pingora**, providing ultra-low latency TLS termination and sealed payload ingestion.
- **`gr-probe-core`**: High-speed risk analysis engine evaluating input dynamics, silicon thermal drift, WebGL shader execution jitter, and multi-session device associations.
- **`gr-probe-store`**: Hybrid storage subsystem balancing sub-millisecond in-memory L1 cache with PostgreSQL L3 relational persistence.

---

## 2. Key Capabilities

1. **Multi-Pack Probe Matrix (B0~B12)**:
   - `B0 Bootstrap`: Basic environment traits and prototype integrity.
   - `B1 Conflict`: Prototype pollution, native function hook detection.
   - `B2 Hardware`: GPU vendor/renderer, CPU concurrency, screen geometry.
   - `B3 System`: Platform architecture, audio context micro-entropy, fonts.
   - `B8 Gateway`: JA4 / TLS SNI consistency verification.
   - `B10 HW Curves`: WebGL shader precision jitter and execution timing profiles.
   - `B10x Silicon`: Crystal oscillator frequency deviations and ultra-low-power thermal drift.
   - `B12 Anti-Camouflage`: Puppeteer, Playwright, Selenium, and headless browser traps.
2. **Deterministic Stable Device ID**: Collision-resistant persistent identifier generated without storing invasive 3rd-party tracking cookies.
3. **Privacy-by-Design**: Client IPs are masked to `/24` (IPv4) or `/48` (IPv6) in memory before disk write; compliant with GDPR, CCPA, and ePrivacy directives.
4. **Server-Side Cookie Allowlist Capture**: Extracts business identifiers (e.g. `user_id`, `plan_tier`) strictly server-side from allowlisted cookies; sensitive names (passwords, tokens) are blocked.
5. **Zero Downtime OTA**: Dynamic hot loading for analysis modules and immutable, content-hashed URLs for frontend assets.

---

## 3. Installation (New Node)

### System Requirements
- **OS**: Linux x86_64 or aarch64 (Ubuntu 20.04+, Debian 11+, RHEL/CentOS/Rocky 8+).
- **Init System**: `systemd` (required for service supervisor & self-OTA restart).
- **Storage**: Existing PostgreSQL 13+ or use `--with-docker` to spin up PostgreSQL + Redis automatically.
- **Ports**: `28680` (Admin console & control-plane API), `28765` (Probe plane, Pingora edge gateway: `/gr.js`, `/dist`, `/v1` open/ingest/result), `28766` (Gateway — early-binding port, reserved).

### Quick Automated Installation
```bash
# Automated install via official signature-verified installer:
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh | bash

# Or with an explicit version and architecture:
bash install/install.sh --version 1.0.14 --arch x86_64 --yes

# With a containerized data layer (PostgreSQL + Redis):
bash install/install.sh --version 1.0.14 --with-docker --yes

# Dry-run validation (checks signature chain and manifest without altering the system):
bash install/install.sh --dry-run
```

### Installation Steps Executed by `install.sh`:
1. **Manifest & Signature Chain Verification**: Resolves the release bundle, checks the root Ed25519 public key fingerprint, verifies bundle SHA-256 against `manifest-index.json`, and validates the ELF executable.
2. **Filesystem Staging**: Deploys binaries to `/opt/greenpng/bin`, static frontend to `/opt/greenpng/fe`, specs to `/opt/greenpng/spec`, and configuration to `/opt/greenpng/.env`.
3. **System Isolation**: Creates the dedicated `greenpng:greenpng` system account and installs the polkit privilege rule (`/etc/polkit-1/rules.d/49-greenpng-self-ota.rules`).
4. **Systemd Service Registration**: Registers and starts `greenpng.service` and `greenpng-auto-upgrade.{service,timer}`.
5. **Health Gate Confirmation**: Probes `http://127.0.0.1:28765/v1/health` until HTTP 200 is achieved.
6. **One-Time Bootstrap Credentials**: Generates initial admin login credentials and the randomized path at:
   ```bash
   cat /opt/greenpng/data/admin/admin_bootstrap_once.txt
   ```

---

## 4. Uninstallation

greenpng provides an official, clean uninstallation script (`install/uninstall.sh`) to safely reclaim all system resources:

```bash
# Standard clean removal (stops services, removes /opt/greenpng, polkit rules & timers):
sudo bash install/uninstall.sh

# Deep clean purge (also removes the greenpng system user and group):
sudo bash install/uninstall.sh --purge

# Specify custom install path if modified:
sudo bash install/uninstall.sh --prefix /custom/greenpng --purge
```

### Uninstallation Actions:
- **Systemd**: Stops, disables, and deletes `greenpng.service`, `greenpng-auto-upgrade.timer`, and `greenpng-auto-upgrade.service`. Executes `systemctl daemon-reload`.
- **Security & Polkit**: Deletes `/etc/polkit-1/rules.d/49-greenpng-self-ota.rules` to remove passwordless restart privileges.
- **Filesystem**: Recursively removes `/opt/greenpng` (binaries, frontend assets, specs, and logs).
- **Runtime Locks**: Removes `/run/greenpng-auto-upgrade.lock` and temporary `/tmp/gr-ota-*` buffers.
- **Safety Guarantee**: PostgreSQL databases and Docker volumes are **never touched** automatically, preventing accidental data loss.

---

## 5. Updates & OTA Upgrades

| Priority | Channel | Trigger / Target | Downtime |
|:---:|:---|:---|:---:|
| **P0** | **Admin Panel OTA** | Click in Console: `Set Release URL` → `Install Runtime / FE` | Zero (for FE/Modules) / < 1s (for Binary) |
| **P1** | **CLI Updater Script** | `sudo bash /opt/greenpng/install/release/update_runtime_from_github.sh` | < 1s restart |
| **P2** | **Automated Daemon** | `greenpng-auto-upgrade.timer` runs during maintenance window | Unattended |

### Atomic Rollback & Health Gate Protection
When updating the runtime binary:
1. The updater stages the new binary and creates an atomic backup (`gr-service.bak.<timestamp>`).
2. The service restarts and performs 8 consecutive health check probes on `/v1/health`.
3. If health checks succeed, the version stamp `/opt/greenpng/VERSION` is updated.
4. If health checks fail (`HEALTH_FAIL`), **an immediate automated rollback is triggered**, restoring the backup binary, frontend tree, and previous version stamp.

---

## 6. Admin Panel & Parameter Configurations

### 6.1 Accessing the Admin Console
The console lives at a **randomized secret path** to shield the login endpoint from automated brute-force attacks:
```
http://<SERVER_IP>:28680/<random-console-path>/
```
*(Find your path in `/opt/greenpng/data/admin/admin_bootstrap_once.txt`).*

### 6.2 Rate Limits (Anti-DDoS & API Protection)
Editable on the **Config** page (`POST /api/config` → `POST /api/config/publish`). Node applies immediately; cluster synchronizes in $\le 30	ext{s}$ without restart:

| Parameter | Default | Description |
|---|:---:|---|
| `rate_limit_open_per_min` | `0` (unlimited) | Max session opens per site per minute |
| `rate_limit_ingest_per_min` | `0` (unlimited) | Max probe batch uploads per site per minute |
| `rate_limit_analyze_per_min` | `0` (unlimited) | Max direct analyze evaluations per site per minute |
| `rate_limit_result_per_min` | `0` (unlimited) | Max result queries per site per minute |
| `rate_limit_client_event_per_min` | `0` (unlimited) | Max total client telemetry events per site |
| `rate_limit_client_event_per_ip_per_min` | `100` | Max client events **per single IP** per minute (prevents single-source flood) |

### 6.3 CDN Real-IP Restoration
greenpng only trusts forwarded headers from specified loopback or upstream CIDRs (`GR_TRUSTED_PROXIES`). When placed behind Cloudflare or Nginx, configure the fronting proxy to pass the real visitor IP:

```nginx
# /etc/nginx/conf.d/realip.conf
set_real_ip_from 173.245.48.0/20;   # Cloudflare IPv4 ranges
set_real_ip_from 103.21.244.0/22;
set_real_ip_from 2400:cb00::/32;    # Cloudflare IPv6 ranges
real_ip_header CF-Connecting-IP;
```

### 6.4 Flood Hardening & Robot Fastlane
- `robot_fastlane_enabled` (`true`): User-Agent declared crawlers (Googlebot, Bingbot) skip heavy L1/L3 stores and deep analyze queues; an early static verdict is assigned immediately.
- `hot_max_vts` (`8192`): Maximum concurrent in-memory visitor terminals before LRU demotion.
- `arm_sweep_interval_ms` (`15000`): Interval for scavenging idle sessions and queueing final verdicts.
- `arm_sweep_cap` (`256`): Maximum batch size per idle sweep tick to prevent database lock spikes.

### 6.5 Hot/Cold Tiering & Retention Policies
- Data retention is configured on the admin panel's **Data Retention** page (per-site policy); the purge loop deletes in bounded batches so large tables never lock.
- `cold_ttl_ms` (`604800000`, 7 days): Lifespan of raw probe batch payloads in `probe_cold` before purge.
- `cold_promote_window_ms` (`864000000`, 24h): Window during which a cold session may be re-opened (promoted hot) instead of starting fresh.
- `cold_purge_interval_ms` (`300000`, 5 min): Interval of the cold-storage purge sweep.
- `ops_retention_days` (`14`, admin setting): Retention for operational logs (`ops_client_events` and related ops tables).

### 6.6 Analysis Triggers & Client Retries
- `analyze_debounce_ms` (`40`): Debounce window for merging closely arriving upload batches.
- `analyze_idle_upload_ms` (`20000`): Idle duration after which an open session triggers analysis.
- `upload_concurrency` (`6`): Initial browser upload concurrency limit.
- `upload_max_retries` (`5`): Maximum exponential backoff retries for client uploads.

---

## 7. Client Probe Deployment

### 7.1 Domain Model (pv / gv)

Production deployments separate two first-party domains:
- **pv domain** (`pv.yourdomain.com`): session lifecycle — serves `/gr.js`, `/dist/*` and `/v1` (open / ingest / result).
- **gv domain** (`gv.yourdomain.com`): sealed payload upload — `/v1/ingest/sealed` with TLS client binding. **Uploads always go direct to the bound gv domain over TLS**; do not proxy this path through another origin, or the sealed binding fails.

All probe script attributes:
| Attribute | Required | Meaning |
|---|:---:|---|
| `data-site-id` | ✓ | Site binding (from the admin panel) |
| `data-endpoint` | — | pv base URL (session open / result). Defaults to same-origin under the inject path |
| `data-inject-path` | — | Custom mount path for `/gr.js` (default `/gr.js`) |
| `data-gw-base` | — | Override for the gv upload base URL (defaults to the bound gv domain issued with the session grant) |

### 7.2 Mode A — Direct Embed
```html
<script
  src="https://pv.yourdomain.com/gr.js"
  data-site-id="site_prod_90b21e"
  data-endpoint="https://pv.yourdomain.com"
  defer>
</script>
```

### 7.3 Mode B — Nginx First-Party Proxy (Recommended)
Proxying the **pv** domain through nginx keeps requests first-party (no cross-origin preflight, less ad-blocker filtering) and lets you cache static assets. The gv upload path is NOT proxied — the browser uploads direct to the bound gv domain:

```nginx
# pv.yourdomain.com — first-party proxy to the probe plane (28765)
location = /gr.js {
    proxy_pass http://127.0.0.1:28765/gr.js;
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
    add_header Cache-Control "no-store" always;   # fe_epoch freshness
}

location /dist/ {
    proxy_pass http://127.0.0.1:28765/dist/;
    proxy_set_header Host $host;
    add_header Cache-Control "public, max-age=31536000, immutable" always;  # content-hashed
}

location /v1/ {
    proxy_pass http://127.0.0.1:28765/v1/;
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
    client_max_body_size 1m;
}
```

When fronted by a proxy, add its CIDR to `GR_TRUSTED_PROXIES` in `/opt/greenpng/.env` so real client IPs (and rate limits) key on the visitor, not the proxy (see §6.3).

### 7.4 Mode C — Cloudflare Worker Injection
Serve `/gr.js` from a Worker on your own domain and rewrite the response to inject the script tag into HTML:

```js
// pv.yourdomain.com/gr.js — Worker fetch-through
export default {
  async fetch(req) {
    const upstream = await fetch("https://<origin>:28765/gr.js", {
      headers: { Host: new URL(req.url).hostname },
    });
    const resp = new Response(upstream.body, upstream);
    resp.headers.set("Cache-Control", "no-store");
    return resp;
  },
};
```
Attach the same `data-site-id` / `data-endpoint` attributes on the injected tag (Mode A form).

### 7.5 Mode D — Application Embed
Render the tag from your server templates (Twig / Jinja / EJS / Thymeleaf) instead of editing static HTML — same attributes as Mode A, injected just before `</body>`.

### 7.6 Deployment Recommendations
- **Cookie allowlist first**: without business cookies in the site's allowlist, the probe cannot correlate sessions to business identities — configure it before going live.
- **Caching**: `/gr.js` must stay `no-store` (it carries the current `fe_epoch`); `/dist/*` is content-hashed — cache it `immutable` for a year. Never cache `/v1/*`.
- **Ad-block resilience**: first-party path + neutral file name (Mode B/C) avoids most filter lists; avoid third-party script domains.
- **Smoke test after wiring**: `curl -s https://pv.yourdomain.com/v1/health` (or `http://127.0.0.1:28765/v1/health`) must return 200, then open a real browser page and confirm a session appears in the admin panel.
```

---

## 8. Multi-Language SDKs

Result SDKs are backend-only clients used to query verdicts during user actions (login, checkout, submission):

```bash
# REST API Query
curl -s "https://pv.yourdomain.com/v1/session/{sessionId}/result?projection=sdk" -H "X-Gr-Sdk-Key: grsk_********************"
```

```go
// Go SDK: single-file client in sdk/go/ (package grresults; copy into your project)
client := grresults.New("https://pv.yourdomain.com", "grsk_58f7a90b4e2d")
verdict, err := client.WaitForResult(sessionID, "sdk", 8000, 250)
if err != nil {
    log.Fatal(err)
}
if verdict["status"] == "bot" {
    // Challenge or reject
}
```

SDK libraries are provided in:
- **Go**: `sdk/go/`
- **Rust**: `sdk/rust/`
- **TypeScript / Node.js**: `sdk/src/`
- **Python**: `sdk/python/`
- **PHP**: `sdk/php/`
- **Shell**: `sdk/shell/`

---

## 9. Open-Source Projects & References

greenpng is built upon foundational open-source engineering and cutting-edge security research:

### 9.1 Core Infrastructure & Frameworks
| Project | License | Role in greenpng |
|---|---|---|
| [Cloudflare Pingora](https://github.com/cloudflare/pingora) | Apache-2.0 | High-performance edge gateway proxy in `gr-probe-plane`, handling TLS termination and sealed ingest (built with its `openssl` feature). |
| [Tokio](https://github.com/tokio-rs/tokio) & [Axum](https://github.com/tokio-rs/axum) | MIT | Asynchronous runtime and ergonomic REST API framework for the control plane. |
| [OpenSSL](https://www.openssl.org/) | Apache-2.0 | TLS backend for the Pingora edge and the admin control plane. |
| [ed25519-dalek](https://github.com/dalek-cryptography/curve25519-dalek) | BSD-3 | Digital signatures for release manifests, tamper-evident probe packs, and OTA verification. |
| [PostgreSQL](https://www.postgresql.org/) & [Redis](https://redis.io/) | PostgreSQL / RSALv2 | L3 relational storage for sessions/audits and distributed cluster state coordination. |
| [Element Plus](https://element-plus.org/) & [Vue 3](https://vuejs.org/) | MIT | Modern UI component framework powering the admin console. |

### 9.2 Research References & Technical Attribution
- **[CreepJS](https://github.com/abrahamjuliot/creepjs)**: Pioneer in browser anti-fingerprinting and prototype tampering detection. greenpng's `B1 Conflict` and `B12 Anti-Camouflage` modules draw foundational inspiration from CreepJS's feature isolation techniques.
- **[FingerprintJS](https://github.com/fingerprintjs/fingerprintjs)**: Open-source reference for client-side hardware enumeration and browser attribute collection.
- **[BotD](https://github.com/fingerprintjs/botd)**: Open-source reference for automated browser heuristics (Puppeteer, Playwright, Selenium detection).

---

## 10. Security & Trust Model

- **Cryptographic Signatures**: All release assets and OTA bundles are signed using a pinned Ed25519 root key (`ota_ed25519.pk`).
- **Immutable Releases**: Release tags and published assets are strictly immutable; updates always increment patch versions.
- **Access Obfuscation**: The administration console path is randomized upon generation to mitigate automated credential stuffing.
- **Reporting Vulnerabilities**: Please review `SECURITY.md` or contact the core security team directly.

---

<p align="center">
  <sub>&copy; 2026 greenpng Project. Released under the MIT License.</sub>
</p>
