# greenpng — Projektguide (Deutsch)

## 1. Was dieses Projekt ist

greenpng (GR) überprüft, ob eine Sitzung von einem echten Menschen stammt,
indem der **Browser des echten Besuchers** sondiert wird — nicht allein durch
eine Inspektion des Datenverkehrs.

Die Pipeline, von Ende zu Ende:

1. **Sondierung (im Browser)** — ein signierter FE-Loader läuft auf Ihren
   Seiten und sammelt Nachweise aus **mehreren Quellen** (Eingabedynamik,
   Geräte-Stack, Echtheit der Umgebung, Automatisierungsspuren) in
   **mehreren gestaffelten Batches** pro Sitzung.
2. **Upload** — der Browser übermittelt jeden Batch über eine versiegelte
   Ingest-Pipeline an den Server; abgespielte oder manipulierte Übermittlungen
   werden zurückgewiesen, bevor sie die Speicherung erreichen.
3. **Analyse (auf dem Server)** — die Analyse-Ebene bewertet jede Sitzung zu
   einem Sitzungsurteil (`human | watch | bot`) mit Konfidenz pro Achse und
   einer stabilen, kollisionsfesten Geräteidentität.
4. **Rückgabe** — Ihr Backend ruft das Ergebnis über die Result-API ab
   (SDKs in sechs Sprachen; die Projektionen `public | sdk | diagnostic`
   steuern, was jeder Aufrufer sieht).

Serverseitig läuft greenpng standardmäßig als ein einziges Host-Binary mit
einer im Baum integrierten Probe-Ebene und unterstützt
**Multi-Node-Deployment mit Lastverteilung**: ein LB-Modul verteilt den
Sondierungsverkehr auf Nodes vor einer gemeinsam genutzten Datenschicht
(PostgreSQL + Redis), sodass Erfassung und Analyse horizontal skalieren.

## 2. Repository-Struktur & Architektur

| Verzeichnis | Inhalt |
|---|---|
| `crates/` | Rust-Workspace — `gr-service` (Kontrollebene + Admin-Konsole + integrierte Probe-Ebene), `gr-probe-core`, `gr-probe-plane`, `gr-probe-store`, `gr-ota`, `gr-admin`, `gr-runtime`, … |
| `modules/` | Signierte Hot-Update-Modulquellen (identity / brain / analyze / ingest / edge / probe_assets) |
| `probe/` | Browser-Sonde FE (Loader, Pack-Kette, versiegelter Ingest-Client) |
| `panel/` | Admin-Panel — Vue-Quellen (`admin-ui/`) + gebautes SPA (`admin-spa/`), EN + 中文 |
| `sdk/` | Backend-Integrations-SDKs in sechs Sprachen (nur Ergebnisabruf) |
| `spec/` | Wire-/Scoring-Spezifikationen, die zur Laufzeit geladen werden |
| `fixtures/` | Daten für Vertragstests |
| `scripts/` | Build-Skripte und FE-Werkzeuge (`scripts/fe/checks/`) |
| `vendor/` | Vendorte Abhängigkeitsquellen (pingora) |
| `install/` | Installer, Compose für die Datenschicht, Upgrade-Skripte |
| `release/` | Paketierungsskripte (Multi-Arch-Build, SBOM, Modulsignierung) |
| `docs/` | Dieser Guide, eine Datei pro Sprache |

```
        Besucherbrowser
              │  <script src="/gr.js">  (pinned, no-store)
              ▼
   FE-Loader ──► /v1/sdk/bootstrap ──► versioniertes Pack-Manifest
              │        asset_base /dist/v/<fe>/g/<gen>/
              ▼
   Pack-Collectoren (Eingabe · Gerät · Umgebung, mehrere Batches)
              │  versiegelte Übermittlung (direkt an die gebundene gv-Domain, TLS)
              ▼
 ┌────────────┴─────────────┐   ┌──────────────────────────────┐
 │ gr-probe-plane (Pingora)  │   │ gr-service (control plane)    │
 │  gateway · ingest · ops   │◄──┤  admin console · site config  │
 └────────────┬─────────────┘   │  module registry (OTA, signed)│
              ▼                 └──────────────┬───────────────┘
   PostgreSQL (+ Redis in multi-node)          │
              ▼                                │
   GET /v1/session/{id}/result ──► Händler-SDK (sechs Sprachen)

   multi-node: LB-Modul verteilt den Sondierungsverkehr auf gr-service-Nodes
   vor der gemeinsam genutzten Datenschicht
```

Zentrale Komponenten: `gr-service` ist die Kontrollebene (Admin-Konsole mit
zufälligem Pfad, Site-/Konfigurationsverwaltung, signierte OTA-Modulregistrierung)
und beherbergt die Probe-Ebene im Baum; `gr-probe-plane` ist das Pingora-Gateway
mit versiegeltem Ingest und den Sitzungs-/Ergebnis-APIs; das Browser-FE löst
jede Ressource zu einer versionsunveränderlichen URL auf, sodass Caches über
Releases hinweg niemals eine veraltete Sonde ausliefern können; PostgreSQL
(plus Redis bei Multi-Node) speichert Sitzungen, Batches und Analyseergebnisse.

## 3. Installation & Verwendung

### 3.1 Installation

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh | bash
# oder mit expliziter Version / Architektur:
bash install/install.sh --version <VERSION> --arch x86_64 --yes
# mit dockerisierter Datenschicht (nur PostgreSQL/Redis; der Server bleibt ein Host-Binary):
bash install/install.sh --version <VERSION> --with-docker --yes
```

Der Installer verifiziert sha256- + ELF- + ed25519-Modulsignaturen, installiert
unter `/opt/greenpng`, schreibt `.env` und die systemd-Unit, stellt die sechs
signierten Module bereit und aktiviert sie und prüft abschließend die
`/v1/health` von Kontrollebene und Probe-Ebene. Zugangsdaten für die erste
Anmeldung sowie der zufällige Konsolenpfad werden in
`/opt/greenpng/data/admin/admin_bootstrap_once.txt` geschrieben.

### 3.2 Update

| Priorität | Kanal | Für |
|---|---|---|
| P0 | Panel-OTA (set-release-url → install / install-fe / install-runtime) | Standard |
| P1 | `install/release/update_runtime_from_github.sh`, `update_module_from_github.sh` | ohne Panel / freie Nodes |
| P2 | SSH manuell | toter Prozess / Erstinstallation |

Alle Updates ziehen die signierten Release-Assets desselben Tags aus diesem
Repository. Docker ist ein Laufzeit-Container, kein Update-Kanal.

### 3.3 Verwendung

1. **Site anlegen** im Admin-Panel: Site-ID, Root-Domains und die
   Cookie-Allowlist für Geschäftsfelder, die an jedes Urteil angehängt werden
   sollen (sensible Namen wie `password`/`token` werden serverseitig
   blockiert).
2. **Sonde deployen** — drei Modi:
   - *Nginx First-Party (empfohlen)*: `/gr.js` + `/gr/dist/v/` auf die
     pv-Domain und `/gr/v1/` auf die gv-Domain proxen (Cookie-Durchleitung),
     `<script src="/gr.js" data-site-id="…" data-endpoint="/gr"
     data-inject-path="nginx" defer></script>` in das HTML injizieren.
   - *Cloudflare-Worker*: dasselbe Tag injizieren und `/gr` im Origin proxen.
   - *App-Einbettung*: den Loader direkt von pv/CDN laden, wobei
     `data-endpoint` auf gv/pv zeigt.
   Browser-Uploads gehen immer **direkt an die gebundene gv-Domain über TLS**.
3. **Ergebnisse empfangen** — im Panel einen Site-SDK-Schlüssel anlegen, dann
   pollen:

```bash
KEY="grsk_..." SID="cycle_..." BASE="https://gv.example.com"
curl -sS -H "X-Gr-Sdk-Key: $KEY" "$BASE/v1/session/$SID/result"
```

4. **Smoke-Test** (curl):

```bash
# eine Sitzung mit allowlisteten Cookies öffnen
curl -sS -X POST "$BASE/v1/session/open" -H "X-Gr-Sdk-Key: $KEY" \
     -H 'Cookie: user_id=u9; plan_tier=pro' -d '{"site_id":"mysite"}'
# → {"session_id":"cycle_…", …}
# danach das Ergebnis prüfen (nach echten FE-Batches oder simulierten):
curl -sS -H "X-Gr-Sdk-Key: $KEY" "$BASE/v1/session/$SID/result"
```
