# greenpng — Panduan Proyek (Bahasa Indonesia)

## 1. Apa proyek ini

greenpng (GR) memverifikasi apakah sebuah sesi adalah manusia nyata dengan
menyelidiki **browser pengunjung yang sebenarnya**, bukan hanya dengan
memeriksa lalu lintas.

Alur kerjanya, dari ujung ke ujung:

1. **Probe (di browser)** — loader FE bertanda tangan berjalan di halaman Anda
   dan mengumpulkan bukti dari **banyak sumber** (dinamika input, tumpukan
   perangkat, keaslian lingkungan, jejak otomatisasi) dalam **banyak batch
   bertahap** per sesi.
2. **Upload** — browser mengirim setiap batch ke server melalui pipeline
   ingest tersegel; kiriman yang direplay atau dimanipulasi ditolak
   sebelum mencapai penyimpanan.
3. **Analisis (di server)** — bidang analisis menilai setiap sesi menjadi
   vonis per sesi (`human | watch | bot`) dengan tingkat keyakinan per poros
   dan identitas perangkat yang stabil dan sadar-kolisi.
4. **Pengembalian** — backend Anda mengambil hasilnya melalui API hasil
   (SDK dalam enam bahasa; proyeksi `public | sdk | diagnostic` mengatur
   apa yang dilihat setiap pemanggil).

Di sisi server, greenpng berjalan sebagai satu biner host dengan probe
plane in-tree secara bawaan, dan mendukung **deployment multi-node dengan
load balancing**: modul LB menyebarkan lalu lintas probe ke berbagai node
di depan lapisan data bersama (PostgreSQL + Redis), sehingga pengumpulan
dan analisis bisa diskalakan secara horizontal.

## 2. Tata letak repositori & arsitektur

| Direktori | Isi |
|---|---|
| `crates/` | Workspace Rust — `gr-service` (control plane + konsol admin + probe plane in-tree), `gr-probe-core`, `gr-probe-plane`, `gr-probe-store`, `gr-ota`, `gr-admin`, `gr-runtime`, … |
| `modules/` | Sumber modul hot-update bertanda tangan (identity / brain / analyze / ingest / edge / probe_assets) |
| `probe/` | Probe FE browser (loader, rantai paket, klien ingest tersegel) |
| `panel/` | Panel admin — sumber Vue (`admin-ui/`) + SPA hasil build (`admin-spa/`), EN + 中文 |
| `sdk/` | SDK integrasi backend dalam enam bahasa (hanya pengambilan hasil) |
| `spec/` | Spesifikasi wire/scoring yang dimuat saat runtime |
| `fixtures/` | Data uji kontrak |
| `scripts/` | Skrip build dan perkakas FE (`scripts/fe/checks/`) |
| `vendor/` | Sumber dependensi yang di-vendor (pingora) |
| `install/` | Installer, compose lapisan data, skrip upgrade |
| `release/` | Skrip pengemasan (build multi-arch, SBOM, penandatanganan modul) |
| `docs/` | Panduan ini, satu file per bahasa |

```
        browser pengunjung
              │  <script src="/gr.js">  (pinned, no-store)
              ▼
   FE loader ──► /v1/sdk/bootstrap ──► manifest paket berversi
              │        asset_base /dist/v/<fe>/g/<gen>/
              ▼
   kolektor paket (input · device · environment, multi-batch)
              │  submit tersegel (langsung ke domain gv terikat, TLS)
              ▼
 ┌────────────┴─────────────┐   ┌──────────────────────────────┐
 │ gr-probe-plane (Pingora)  │   │ gr-service (control plane)    │
 │  gateway · ingest · ops   │◄──┤  konsol admin · config situs  │
 └────────────┬─────────────┘   │  registry modul (OTA, signed) │
              ▼                 └──────────────┬───────────────┘
   PostgreSQL (+ Redis saat multi-node)         │
              ▼                                │
   GET /v1/session/{id}/result ──► SDK merchant (enam bahasa)

   multi-node: modul LB menyebarkan lalu lintas probe ke node-node gr-service
   di depan lapisan data bersama
```

Komponen kunci: `gr-service` adalah control plane (konsol admin dengan
jalur acak, pengelolaan situs/config, registry modul OTA bertanda tangan)
dan menaungi probe plane secara in-tree; `gr-probe-plane` adalah gateway
Pingora dengan ingest tersegel dan API sesi/hasil; FE browser me-resolve
setiap aset ke URL yang immutable terhadap versi sehingga cache tidak akan
pernah menyajikan probe basi lintas rilis; PostgreSQL (plus Redis saat
multi-node) menyimpan sesi, batch, dan hasil analisis.

## 3. Instalasi & penggunaan

### 3.1 Instalasi

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh | bash
# atau dengan versi / arsitektur eksplisit:
bash install/install.sh --version <VERSION> --arch x86_64 --yes
# dengan lapisan data ter-docker (hanya PostgreSQL/Redis; server tetap biner host):
bash install/install.sh --version <VERSION> --with-docker --yes
```

Installer memverifikasi tanda tangan sha256 + ELF + ed25519 untuk modul,
memasang di bawah `/opt/greenpng`, menulis `.env` dan unit systemd,
menyiapkan dan mengaktifkan enam modul bertanda tangan, dan menunggu
`/v1/health` dari control/plane lulus. Kredensial login pertama dan jalur
konsol acak ditulis ke
`/opt/greenpng/data/admin/admin_bootstrap_once.txt`.

### 3.2 Pembaruan

| Prioritas | Kanal | Untuk |
|---|---|---|
| P0 | OTA Panel (set-release-url → install / install-fe / install-runtime) | bawaan |
| P1 | `install/release/update_runtime_from_github.sh`, `update_module_from_github.sh` | tanpa panel / node gratis |
| P2 | SSH manual | proses mati / instalasi pertama |

Semua pembaruan menarik aset Release bertanda tangan dari tag yang sama di
repositori ini. Docker adalah kontainer runtime, bukan kanal pembaruan.

### 3.3 Penggunaan

1. **Buat situs** di panel admin: id situs, domain root, dan allow-list
   cookie untuk kolom bisnis yang ingin Anda sertakan pada setiap vonis
   (nama sensitif seperti `password`/`token` diblokir di sisi server).
2. **Deploy probe** — tiga mode:
   - *Nginx first-party (disarankan)*: proxy `/gr.js` + `/gr/dist/v/` ke
     domain pv dan `/gr/v1/` ke domain gv (Cookie passthrough), sisipkan
     `<script src="/gr.js" data-site-id="…" data-endpoint="/gr"
     data-inject-path="nginx" defer></script>` ke dalam HTML.
   - *Cloudflare worker*: sisipkan tag yang sama dan proxy `/gr` in-origin.
   - *App embed*: muat loader langsung dari pv/CDN dengan
     `data-endpoint` menunjuk ke gv/pv.
   Upload dari browser selalu **langsung ke domain gv terikat melalui TLS**.
3. **Terima hasil** — buat kunci SDK situs di panel, lalu polling:

```bash
KEY="grsk_..." SID="cycle_..." BASE="https://gv.example.com"
curl -sS -H "X-Gr-Sdk-Key: $KEY" "$BASE/v1/session/$SID/result"
```

4. **Smoke test** (curl):

```bash
# buka sesi dengan membawa cookie yang di-allowlist
curl -sS -X POST "$BASE/v1/session/open" -H "X-Gr-Sdk-Key: $KEY" \
     -H 'Cookie: user_id=u9; plan_tier=pro' -d '{"site_id":"mysite"}'
# → {"session_id":"cycle_…", …}
# lalu periksa hasilnya (setelah batch FE nyata atau batch tersimulasi):
curl -sS -H "X-Gr-Sdk-Key: $KEY" "$BASE/v1/session/$SID/result"
```
