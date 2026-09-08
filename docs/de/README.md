# greenpng — Product Guide (Deutsch)

Browserseitige Menschen-Verifikation und Anti-Bot-Intelligenz: ein signierter
Probe, der im Browser echter Besucher läuft, eine versiegelte
Ingest-Pipeline und eine Analyse-Ebene, die über SDKs in sechs Sprachen
Sitzungs-Verdikte zurückliefert.

> Dieser Guide ist in 12 Sprachen verfügbar — siehe den
> [Sprachindex](../../README.md#documentation) im Repository-Stammverzeichnis.

## 1. Was greenpng leistet

Jede Besuchersitzung wird auf dem Gerät und serverseitig bewertet:

- **Echt-oder-Bot-Verdikt** — `human | watch | bot` mit Konfidenz je Achse
  (Eingabedynamik, Konsistenz des Geräte-Stacks, Authentizität der
  Umgebung, Automatisierungsspuren, Wiederverwendung von Historie).
- **Stabile Geräteidentität** — eine kollisionsbewusste Geräte-ID, die
  über Sitzungen hinweg stabil bleibt, ohne auf Third-Party-Cookies
  angewiesen zu sein.
- **Erfassung von Business-Feldern** — Ihre per Allow-Liste freigegebenen
  Cookie-Felder (`user_id`, `plan_tier`, …) werden beim Öffnen der Sitzung
  erfasst und an das Verdikt angehängt. Sensible Namen
  (password/token/…) werden serverseitig blockiert.
- **IP-Intelligenz** — Besucher-IPs werden beim Ingress datenschutzgerecht
  auf /24 (IPv4) oder /48 (IPv6) maskiert; eine optionale Anreicherung
  ordnet das Subnetz per DB-IP (mitgeliefertes MMDB), IPinfo, MaxMind oder
  einem eigenen HTTP-Enricher ASN/Land/Stadt zu. Secrets verlassen den
  Server nie.
- **Ergebnis-Abfrage-API** — Händler rufen das Verdikt per SDK-Schlüssel
  je Site ab; Projektionen (`public | sdk | diagnostic`) steuern die
  Sichtbarkeit.

## 2. Kernfunktionen

| Funktion | Was Sie erhalten |
|---|---|
| Signierter FE-Probe | Manipulationssichere Browser-Packs (ed25519), versionierte unveränderliche Asset-URLs `/dist/v/<ver>/g/<gen>/…` — CDN-sicher, kein Cache-Poisoning |
| Versiegelter Ingest | Pack-Übermittlungen werden versiegelt; Replay/Manipulation wird vorgelagert abgewiesen |
| Analysemodule (OTA) | identity / brain / analyze / ingest / edge / probe_assets werden als signierte Module ohne Ausfallzeit hot-aktualisiert |
| Admin-Panel | Eigenständige Konsole unter einem zufälligen Pfad, einzelner scrypt-Admin, Audit-Log, Site-/Strategie-/Integrations-/Retention-/DSAR-Verwaltung, EN + 中文 |
| Datenschutz standardmäßig | IP-Maskierung am frühesten Ingress-Punkt, Cookie-Allow-Liste, DSAR-Export/-Löschung, Retention-Purge |
| SDKs in sechs Sprachen | JS / Python / Go / PHP / Shell / Rust Clients für `wait_for_result` |
| Multi-Node-fähig | Eingebautes LB-Modul, Cluster-Heartbeat, OTA-Spiegel |

## 3. Architektur

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

**Deployments-Varianten**

- **gr-service** — ein Prozess: Admin-Konsole + Steuer-APIs + im Baum
  enthaltene Probe-Ebene. Port 28680 (Konsole, zufälliger Pfad) + 28765
  (Plane-Loopback).
- **Nginx der Business-Site** — liefert `/gr.js` aus und proxied
  (optional) den Same-Origin-API-Präfix; Browser-Uploads müssen die
  gebundene GV-Domain über Pingora-TLS verwenden.
- **Datenbanken** — PostgreSQL für Steuer- und Probe-Speicher (ein
  SQLite-Gerüst existiert für das Labor).

## 4. Schnellinstallation

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh
bash install.sh --version <VERSION>            # see the VERSION file / Releases
# or with a dockerized data layer:
bash install.sh --version <VERSION> --with-docker --yes
```

Der Installer prüft sha256- + ELF- + ed25519-Modulsignaturen, installiert
unter `/opt/greenpng`, schreibt `.env`, stellt die sechs signierten Module
bereit und aktiviert sie, aktiviert die systemd-Unit und prüft
`/v1/health` als Gate.

Erste Anmeldung: Einmal-Zugangsdaten werden in
`/opt/greenpng/data/admin/admin_bootstrap_once.txt` geschrieben — dort ist
auch der zufällige Konsolenpfad (`/c-<hex>/`) verzeichnet. Es gibt kein
`/admin`- oder `/console/`-Präfix, und die Passwort-Anmeldung ist der
einzige Zugang zum Panel.

## 5. Nutzungs-Tutorial

### 5.1 Site anlegen

Panel **Sites → Site erstellen**:

- `site_id` — Ihre Tenant-ID, verwendet im Embed
- `root_domains` — die www-Hostnamen (CORS + Hostnamen-Bindung)
- `cookie_fields` — die Cookie-Allow-Liste, z. B.
  `["user_id", "plan_tier", "cart_id", "utm_source"]`

Die Site-Zeile fließt beim Speichern von `control.sites → public.sites`
(Probe-Ebene), und gr-service ergänzt bereits bestehende Sites beim Start.

### 5.2 Probe ausbringen (drei Modi)

**A. Nginx First-Party (empfohlen)** — der vhost Ihrer Site proxied den
Boot-Loader und den Same-Origin-API-Präfix:

```nginx
location = /gr.js      { proxy_pass https://pv.example.com/gr.js; proxy_set_header Host pv.example.com; }
location /gr/dist/v/   { proxy_pass https://pv.example.com/gr/dist/v/; proxy_set_header Host pv.example.com; }
location /gr/v1/       { proxy_pass https://gv.example.com/; proxy_set_header Cookie $http_cookie;
                         proxy_set_header X-Forwarded-For $remote_addr; }
```

```html
<script src="/gr.js" data-site-id="mysite" data-endpoint="/gr" data-inject-path="nginx" defer></script>
```

**B. Cloudflare-Worker** — der Worker injiziert das Boot-Skript und
proxied `/gr` im Origin. Vermeiden Sie den „Under Attack"-Modus auf `/gr`
(Challenge-Seiten brechen den Probe).

**C. Site-Scripting / CDN-Embed** — laden Sie das Boot-JS direkt von PV
und lassen Sie `data-endpoint` auf GV zeigen.

In jedem Modus löst der Loader Packs über das SDK-Bootstrap auf und ruft
ausschließlich versionierte, unveränderliche URLs ab.

### 5.3 Ergebnisse empfangen (SDK in sechs Sprachen)

Legen Sie im Panel (SDK-Seite) einen **Backend-Schlüssel** für Ihre Site
an. Das SDK leitet niemals Probes weiter; es fragt nur Ergebnisse ab:

```
GET {gv_base}/v1/session/{session_id}/result?projection=sdk
X-Gr-Sdk-Key: <site backend key>
```

| Sprache | Einstiegspunkt |
|---|---|
| JS | `sdk/src/index.js` — `new GrResultClient({baseUrl, apiKey}).waitForResult(...)` |
| Python | `sdk/python/gr_results.py` — `GrResultClient(base_url, api_key).wait_for_result(...)` |
| Go | `sdk/go/gr_results.go` — `gr.New(baseURL, key).WaitForResult(...)` |
| PHP | `sdk/php/GrResultsClient.php` — `(new GrResultClient(...))->wait_for_result(...)` |
| Shell | `sdk/shell/gr_sdk.sh` — `gr_wait_for_result <session> sdk 8000` |
| Rust | `sdk/rust/` — `Client::new(base_url, key).wait_for_result(...)` |

Site-Schlüssel sind zugeordnet: Ein für Site A erzeugter Schlüssel kann
Sitzungen von Site B nicht lesen (`403 sdk key site mismatch`), und
zurückgezogene Schlüssel funktionieren sofort nicht mehr (`401`).

### 5.4 End-to-End-Smoke (curl)

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

### 5.5 Aktuell halten

| Kanal | Befehl |
|---|---|
| Panel-OTA (Standard) | Admin-Panel → Modules / Runtime install |
| Updater-Skript | `VERSION=<v> INSTALL_ROOT=/opt/greenpng bash install/release/update_runtime_from_github.sh` |
| Manuell | SSH + Rollback auf die vorherige Runtime (`bin/releases/<v>` bleibt erhalten) |

Jedes Update bezieht dieselben signierten Release-Assets aus diesem
Repository.

## 6. Repository-Struktur

| Verzeichnis | Inhalt |
|---|---|
| `crates/` | Rust-Workspace — gr-service, gr-probe-core, gr-probe-plane, gr-probe-store, gr-ota, gr-admin, gr-runtime, … |
| `modules/` | Signierte Hot-Update-Modulquellen (identity / brain / analyze / ingest / edge / probe_assets) |
| `probe/` | Browser-Probe FE (Loader, Packs, versiegelter Ingest) |
| `panel/` | Admin-Panel — Vue-Quellen (`admin-ui/`) + gebaute SPA (`admin-spa/`) |
| `sdk/` | Ergebnis-Abfrage-SDKs (JS / Python / Go / PHP / Shell / Rust) |
| `spec/` | Zur Laufzeit geladene Specs (Bot-Gewichte, Kataloge) |
| `install/` | Installer + Updater + systemd-Material |
| `release/` | Packaging-Skripte (Multi-Arch-Bundle, SLSA-Attestierung) |
| `scripts/` | FE-Vertragsprüfungen und Helfer |
| `docs/` | Dieser Guide in 12 Sprachen |
| `VERSION` | Single Source of Truth für die Release-Version |

## 7. Links

- Releases & Installations-Einstiegspunkt: dieses Repository
- Offizielle Website: https://www.greenpng.cc (Produktvorstellung, EN + 中文)
- Panel-Sprachen: Englisch + 中文 (synchron gehalten in `panel/admin-ui/src/i18n/`)
