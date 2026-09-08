# greenpng — Product Guide (Español)

Verificación humana en el lado del navegador e inteligencia antibot: una
sonda firmada que se ejecuta en los navegadores de los visitantes reales,
un pipeline de ingesta sellado y un plano de análisis que devuelve
veredictos por sesión a través de SDKs en seis idiomas.

> Esta guía está disponible en 12 idiomas — consulte el
> [índice de idiomas](../../README.md#documentation) en la raíz del repositorio.

## 1. Qué hace greenpng

Cada sesión de visitante se puntúa en el dispositivo y en el servidor:

- **Veredicto humano vs bot** — `human | watch | bot` con confianza por eje
  (dinámica de entrada, coherencia del stack del dispositivo, autenticidad
  del entorno, rastros de automatización, reutilización de historial).
- **Identidad de dispositivo estable** — un ID de dispositivo consciente de
  colisiones que se mantiene estable entre sesiones sin depender de cookies
  de terceros.
- **Captura de campos de negocio** — sus campos de cookie autorizados
  (`user_id`, `plan_tier`, …) se capturan al abrir la sesión y se adjuntan
  al veredicto. Los nombres sensibles (password/token/…) se bloquean en el
  servidor.
- **Inteligencia de IP** — las IP de los visitantes se enmascaran por
  privacidad a /24 (IPv4) o /48 (IPv6) en la ingesta; el enriquecimiento
  opcional mapea la subred a ASN/país/ciudad mediante DB-IP (MMDB incluido),
  IPinfo, MaxMind o un enriquecedor HTTP personalizado. Los secretos nunca
  salen del servidor.
- **API de recuperación de resultados** — los comerciantes consultan el
  veredicto con una clave SDK por sitio; las proyecciones
  (`public | sdk | diagnostic`) controlan la exposición.

## 2. Características clave

| Característica | Qué le aporta |
|---|---|
| Sonda FE firmada | Paquetes de navegador a prueba de manipulación (ed25519), URLs de assets versionadas e inmutables `/dist/v/<ver>/g/<gen>/…` — seguras para CDN, sin envenenamiento de caché |
| Ingesta sellada | Los envíos de paquetes quedan sellados; reproducción/manipulación rechazadas aguas arriba |
| Módulos de análisis (OTA) | identity / brain / analyze / ingest / edge / probe_assets se actualizan en caliente como módulos firmados sin tiempo de inactividad |
| Panel de administración | Consola independiente en una ruta aleatoria, un único admin scrypt, registro de auditoría, gestión de sitios/estrategias/integraciones/retención/DSAR, EN + 中文 |
| Privacidad por defecto | Enmascaramiento de IP en el punto de ingesta más temprano, lista de permitidos de cookies, exportación/borrado DSAR, purga por retención |
| SDKs en seis idiomas | Clientes JS / Python / Go / PHP / Shell / Rust para `wait_for_result` |
| Listo para multinodo | Módulo LB integrado, latido de clúster, espejo OTA |

## 3. Arquitectura

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

**Formas de despliegue**

- **gr-service** — un proceso: consola de administración + APIs de control +
  probe plane integrado. Puerto 28680 (consola, ruta aleatoria) + 28765
  (loopback del plano).
- **nginx del sitio de negocio** — sirve `/gr.js` y (opcionalmente) hace
  proxy del prefijo API de mismo origen; las subidas del navegador deben usar
  el dominio GV vinculado a través de Pingora TLS.
- **Bases de datos** — PostgreSQL para los almacenes de control y de sondas
  (existe un esqueleto SQLite para el laboratorio).

## 4. Instalación rápida

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh
bash install.sh --version <VERSION>            # see the VERSION file / Releases
# or with a dockerized data layer:
bash install.sh --version <VERSION> --with-docker --yes
```

El instalador verifica firmas sha256 + ELF + ed25519 de los módulos,
instala bajo `/opt/greenpng`, escribe `.env`, prepara y activa los seis
módulos firmados, habilita la unidad systemd y verifica
`/v1/health` como puerta de salida.

Primer inicio de sesión: las credenciales de un solo uso se escriben en
`/opt/greenpng/data/admin/admin_bootstrap_once.txt` — la ruta aleatoria de
la consola (`/c-<hex>/`) queda registrada allí. No existe un prefijo
`/admin` ni `/console/`, y el inicio de sesión con contraseña es la única
entrada al panel.

## 5. Tutorial de uso

### 5.1 Crear un sitio

En el panel, **Sites → Create site**:

- `site_id` — su id de tenant, usado en el embed
- `root_domains` — los hostnames www (CORS + vinculación de hostname)
- `cookie_fields` — la lista de cookies permitidas, p. ej.
  `["user_id", "plan_tier", "cart_id", "utm_source"]`

La fila del sitio se propaga de `control.sites → public.sites`
(probe plane) al guardar, y gr-service rellena los sitios preexistentes al
arrancar.

### 5.2 Desplegar la sonda (tres modos)

**A. Nginx first-party (recomendado)** — el vhost de su sitio hace proxy
del cargador de arranque y del prefijo API de mismo origen:

```nginx
location = /gr.js      { proxy_pass https://pv.example.com/gr.js; proxy_set_header Host pv.example.com; }
location /gr/dist/v/   { proxy_pass https://pv.example.com/gr/dist/v/; proxy_set_header Host pv.example.com; }
location /gr/v1/       { proxy_pass https://gv.example.com/; proxy_set_header Cookie $http_cookie;
                         proxy_set_header X-Forwarded-For $remote_addr; }
```

```html
<script src="/gr.js" data-site-id="mysite" data-endpoint="/gr" data-inject-path="nginx" defer></script>
```

**B. Worker de Cloudflare** — el worker inyecta el script de arranque y hace
proxy de `/gr` en el mismo origen. Evite el modo "Under Attack" en `/gr`
(las páginas de desafío rompen la sonda).

**C. Scripting del sitio / embed por CDN** — cargue el JS de arranque
directamente desde PV y apunte `data-endpoint` a GV.

En todos los modos, el cargador resuelve los paquetes a través del bootstrap
del SDK y solo obtiene URLs versionadas e inmutables.

### 5.3 Recibir resultados (SDK en seis idiomas)

Cree una **clave de backend** para su sitio en el panel (página SDK). El SDK
nunca retransmite sondas; solo consulta resultados:

```
GET {gv_base}/v1/session/{session_id}/result?projection=sdk
X-Gr-Sdk-Key: <site backend key>
```

| Idioma | Punto de entrada |
|---|---|
| JS | `sdk/src/index.js` — `new GrResultClient({baseUrl, apiKey}).waitForResult(...)` |
| Python | `sdk/python/gr_results.py` — `GrResultClient(base_url, api_key).wait_for_result(...)` |
| Go | `sdk/go/gr_results.go` — `gr.New(baseURL, key).WaitForResult(...)` |
| PHP | `sdk/php/GrResultsClient.php` — `(new GrResultClient(...))->waitForResult(...)` |
| Shell | `sdk/shell/gr_sdk.sh` — `gr_wait_for_result <session> sdk 8000` |
| Rust | `sdk/rust/` — `Client::new(base_url, key).wait_for_result(...)` |

Las claves de sitio tienen alcance restringido: una clave emitida para el
sitio A no puede leer las sesiones del sitio B
(`403 sdk key site mismatch`), y las claves revocadas dejan de funcionar
de inmediato (`401`).

### 5.4 Prueba de humo extremo a extremo (curl)

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

### 5.5 Mantenerlo actualizado

| Canal | Comando |
|---|---|
| OTA del panel (por defecto) | Panel de administración → Modules / Runtime install |
| Script de actualización | `VERSION=<v> INSTALL_ROOT=/opt/greenpng bash install/release/update_runtime_from_github.sh` |
| Manual | SSH + rollback al runtime anterior (se conserva `bin/releases/<v>`) |

Cada actualización obtiene los mismos assets firmados del Release de este
repositorio.

## 6. Estructura del repositorio

| Directorio | Contenido |
|---|---|
| `crates/` | Workspace de Rust — gr-service, gr-probe-core, gr-probe-plane, gr-probe-store, gr-ota, gr-admin, gr-runtime, … |
| `modules/` | Fuentes de los módulos firmados de actualización en caliente (identity / brain / analyze / ingest / edge / probe_assets) |
| `probe/` | Sonda FE del navegador (loader, packs, ingesta sellada) |
| `panel/` | Panel de administración — fuentes Vue (`admin-ui/`) + SPA compilado (`admin-spa/`) |
| `sdk/` | SDKs de recuperación de resultados (JS / Python / Go / PHP / Shell / Rust) |
| `spec/` | Especificaciones cargadas en runtime (pesos de bots, catálogos) |
| `install/` | Instalador + actualizador + material systemd |
| `release/` | Scripts de empaquetado (bundle multiarquitectura, atestación SLSA) |
| `scripts/` | Comprobaciones de contrato FE y utilidades |
| `docs/` | Esta guía en 12 idiomas |
| `VERSION` | Fuente única de verdad para la versión del release |

## 7. Enlaces

- Releases y punto de entrada de instalación: este repositorio
- Sitio oficial: https://www.greenpng.cc (presentación del producto, EN + 中文)
- Idiomas del panel: English + 中文 (sincronizados en `panel/admin-ui/src/i18n/`)
