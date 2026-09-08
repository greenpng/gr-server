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

## Install (new node)

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh | bash
# or with an explicit version / arch:
bash install/install.sh --version <VERSION> --arch x86_64 --yes
# with a dockerized data layer (PostgreSQL/Redis only; the server itself stays a host binary):
bash install/install.sh --version <VERSION> --with-docker --yes
```

The installer verifies sha256 + ELF + ed25519 module signatures, installs
under `/opt/greenpng`, writes `.env` and the systemd unit, stages and
activates the six signed modules, and gates on the control/plane
`/v1/health`. First-login credentials and the random console path are
written to `/opt/greenpng/data/admin/admin_bootstrap_once.txt`.

## Update (installed node)

| Priority | Channel | For |
|---|---|---|
| P0 | Panel OTA (set-release-url → install / install-fe / install-runtime) | default |
| P1 | `install/release/update_runtime_from_github.sh`, `update_module_from_github.sh` | no panel / free nodes |
| P2 | SSH manual | dead process / first install |

All updates pull the same tag's signed Release assets from this repository.
**Docker is a runtime container, not an update channel.**

## Repository layout

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
| `install/` | Installer + data-layer compose + upgrade scripts |
| `release/` | Packaging scripts (build_multiarch / SBOM / module signing) |
| `docs/` | Project guide, one file per language |
| `VERSION` | Release version, single source of truth |

Full documentation of workstreams and lab tests lives in the development
tree and is intentionally not mirrored here; `docs/` in this repository is
the public documentation surface.

## Build and test from source

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

## Security

- Release assets are signed by the root Ed25519 key; the installer pins the
  public-key fingerprint (`OTA_ROOT_PUBKEY_SHA256` inside `install.sh`).
- The private key exists only in GitHub Secrets and never enters the tree;
  `ota_ed25519.pk` in this repository is the public key.
- Tags and Release assets are immutable: fixes ship as new PATCH versions,
  history is never rewritten.
