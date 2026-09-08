# greenpng — Product Guide (Bahasa Indonesia)

Verifikasi manusia di sisi browser dan intelijen anti-bot: probe bertanda
tangan yang berjalan di peramban pengunjung sungguhan, pipeline ingest
tersegel, serta plane analisis yang mengembalikan putusan per-sesi melalui
SDK dalam enam bahasa.

> Panduan ini tersedia dalam 12 bahasa — lihat
> [indeks bahasa](../../README.md#documentation) di root repositori.

## 1. Apa yang dilakukan greenpng

Setiap sesi pengunjung dinilai di perangkat dan di sisi server:

- **Putusan nyata vs bot** — `human | watch | bot` dengan tingkat keyakinan
  per-sumbu (dinamika input, konsistensi tumpukan perangkat,
  otentisitas lingkungan, jejak otomasi, penggunaan ulang riwayat).
- **Identitas perangkat yang stabil** — ID perangkat yang sadar-tabrakan
  tetap stabil lintas sesi tanpa bergantung pada cookie pihak ketiga.
- **Penangkapan field bisnis** — field cookie yang Anda daftarkan dalam
  allow-list (`user_id`, `plan_tier`, …) ditangkap saat sesi dibuka dan
  dilampirkan pada putusan. Nama yang sensitif (password/token/…)
  diblokir di sisi server.
- **Intelijen IP** — IP pengunjung dimasukkan masker privasi ke /24 (IPv4)
  atau /48 (IPv6) saat ingest; pengayaan opsional memetakan subnet ke
  ASN/negara/kota melalui DB-IP (MMDB bawaan), IPinfo, MaxMind, atau
  pengaya HTTP kustom. Rahasia tidak pernah meninggalkan server.
- **API pengambilan hasil** — merchant melakukan poll terhadap putusan
  dengan SDK key per-situs; proyeksi (`public | sdk | diagnostic`)
  mengatur tingkat keterbukaan.

## 2. Fitur utama

| Fitur | Apa yang Anda dapatkan |
|---|---|
| Probe FE bertanda tangan | Paket peramban tamper-evident (ed25519), URL aset immutable berversi `/dist/v/<ver>/g/<gen>/…` — aman untuk CDN, tanpa cache poisoning |
| Ingest tersegel | Pengiriman paket disegel; replay/tamper ditolak di hulu |
| Modul analisis (OTA) | identity / brain / analyze / ingest / edge / probe_assets diperbarui secara hot sebagai modul bertanda tangan tanpa downtime |
| Panel admin | Konsol mandiri di path acak, admin scrypt tunggal, log audit, pengelolaan situs/strategi/integrasi/retensi/DSAR, EN + 中文 |
| Privasi secara default | Masking IP pada titik ingest paling awal, allow-list cookie, ekspor/penghapusan DSAR, purge retensi |
| SDK enam bahasa | Klien JS / Python / Go / PHP / Shell / Rust untuk `wait_for_result` |
| Siap multi-node | Modul LB bawaan, heartbeat klaster, mirror OTA |

## 3. Arsitektur

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

**Bentuk deployment**

- **gr-service** — satu proses: konsol admin + API kontrol + probe plane
  in-tree. Port 28680 (konsol, path acak) + 28765 (plane loopback).
- **nginx situs bisnis** — menyajikan `/gr.js` dan (opsional) mem-proxy
  prefix API same-origin; upload dari peramban harus menggunakan domain
  GV terikat melalui TLS Pingora.
- **Basis data** — PostgreSQL untuk penyimpanan kontrol + probe (kerangka
  SQLite tersedia untuk lab).

## 4. Instalasi cepat

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh
bash install.sh --version <VERSION>            # see the VERSION file / Releases
# or with a dockerized data layer:
bash install.sh --version <VERSION> --with-docker --yes
```

Installer memverifikasi tanda tangan sha256 + ELF + modul ed25519,
memasang di bawah `/opt/greenpng`, menulis `.env`, men-stage dan
mengaktifkan enam modul bertanda tangan, mengaktifkan unit systemd, dan
menunggu `/v1/health` siap.

Login pertama: kredensial sekali-pakai ditulis ke
`/opt/greenpng/data/admin/admin_bootstrap_once.txt` — path konsol acak
(`/c-<hex>/`) tercatat di sana. Tidak ada prefix `/admin` atau
`/console/`, dan login dengan kata sandi adalah satu-satunya pintu
masuk panel.

## 5. Tutorial penggunaan

### 5.1 Membuat situs

Panel **Sites → Create site**:

- `site_id` — id tenant Anda, digunakan pada embed
- `root_domains` — hostname www (CORS + pengikatan hostname)
- `cookie_fields` — allow-list cookie, mis.
  `["user_id", "plan_tier", "cart_id", "utm_source"]`

Baris situs mengalir `control.sites → public.sites` (probe plane) saat
disimpan, dan gr-service meng-backfill situs yang sudah ada saat startup.

### 5.2 Deploy probe (tiga mode)

**A. Nginx first-party (disarankan)** — vhost situs Anda mem-proxy boot
loader dan prefix API same-origin:

```nginx
location = /gr.js      { proxy_pass https://pv.example.com/gr.js; proxy_set_header Host pv.example.com; }
location /gr/dist/v/   { proxy_pass https://pv.example.com/gr/dist/v/; proxy_set_header Host pv.example.com; }
location /gr/v1/       { proxy_pass https://gv.example.com/; proxy_set_header Cookie $http_cookie;
                         proxy_set_header X-Forwarded-For $remote_addr; }
```

```html
<script src="/gr.js" data-site-id="mysite" data-endpoint="/gr" data-inject-path="nginx" defer></script>
```

**B. Cloudflare worker** — worker menyuntikkan boot script dan mem-proxy
`/gr` dalam-origin. Hindari mode "Under Attack" pada `/gr` (halaman
challenge merusak probe).

**C. Scripting situs / embed CDN** — muat boot JS langsung dari PV dan
arahkan `data-endpoint` ke GV.

Di semua mode, loader me-resolve paket melalui bootstrap SDK dan hanya
mengambil URL immutable berversi.

### 5.3 Menerima hasil (SDK enam bahasa)

Buat **backend key** untuk situs Anda di panel (halaman SDK). SDK tidak
pernah meneruskan probe; ia hanya meng-query hasil:

```
GET {gv_base}/v1/session/{session_id}/result?projection=sdk
X-Gr-Sdk-Key: <site backend key>
```

| Bahasa | Titik masuk |
|---|---|
| JS | `sdk/src/index.js` — `new GrResultClient({baseUrl, apiKey}).waitForResult(...)` |
| Python | `sdk/python/gr_results.py` — `GrResultClient(base_url, api_key).wait_for_result(...)` |
| Go | `sdk/go/gr_results.go` — `gr.New(baseURL, key).WaitForResult(...)` |
| PHP | `sdk/php/GrResultsClient.php` — `(new GrResultClient(...))->waitForResult(...)` |
| Shell | `sdk/shell/gr_sdk.sh` — `gr_wait_for_result <session> sdk 8000` |
| Rust | `sdk/rust/` — `Client::new(base_url, key).wait_for_result(...)` |

Kunci situs bersifat terbatas: key yang dibuat untuk situs A tidak dapat
membaca sesi situs B (`403 sdk key site mismatch`), dan key yang dicabut
langsung berhenti berfungsi (`401`).

### 5.4 Smoke test end-to-end (curl)

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

### 5.5 Tetap terbarui

| Kanal | Perintah |
|---|---|
| OTA panel (default) | Panel admin → Modules / Runtime install |
| Skrip updater | `VERSION=<v> INSTALL_ROOT=/opt/greenpng bash install/release/update_runtime_from_github.sh` |
| Manual | SSH + rollback runtime sebelumnya (`bin/releases/<v>` tetap disimpan) |

Setiap pembaruan menarik aset Release bertanda tangan yang sama dari
repositori ini.

## 6. Tata letak repositori

| Direktori | Isi |
|---|---|
| `crates/` | Workspace Rust — gr-service, gr-probe-core, gr-probe-plane, gr-probe-store, gr-ota, gr-admin, gr-runtime, … |
| `modules/` | Sumber modul hot-update bertanda tangan (identity / brain / analyze / ingest / edge / probe_assets) |
| `probe/` | Probe FE peramban (loader, packs, ingest tersegel) |
| `panel/` | Panel admin — sumber Vue (`admin-ui/`) + SPA hasil build (`admin-spa/`) |
| `sdk/` | SDK pengambilan hasil (JS / Python / Go / PHP / Shell / Rust) |
| `spec/` | Spesifikasi yang dimuat runtime (bobot bot, katalog) |
| `install/` | Installer + updater + material systemd |
| `release/` | Skrip pengemasan (bundle multi-arch, atestasi SLSA) |
| `scripts/` | Pemeriksaan kontrak FE dan helper |
| `docs/` | Panduan ini dalam 12 bahasa |
| `VERSION` | Sumber tunggal kebenaran untuk versi rilis |

## 7. Tautan

- Rilis & titik masuk instalasi: repositori ini
- Situs resmi: https://www.greenpng.cc (pengantar produk, EN + 中文)
- Locale panel: English + 中文 (dijaga sinkron di `panel/admin-ui/src/i18n/`)
