# greenpng — Guía del proyecto (Español)

## 1. Qué es este proyecto

greenpng (GR) verifica si una sesión corresponde a un ser humano real
sondeando el **navegador del visitante real**, no inspeccionando únicamente
el tráfico.

El pipeline, de extremo a extremo:

1. **Sondeo (en el navegador)** — un loader FE firmado se ejecuta en tus
   páginas y recopila evidencia de **múltiples fuentes** (dinámica de
   entrada, stack del dispositivo, autenticidad del entorno, rastros de
   automatización) en **múltiples lotes por etapas** por sesión.
2. **Subida** — el navegador envía cada lote al servidor a través de un
   pipeline de ingesta sellado; los envíos reproducidos o alterados son
   rechazados antes de llegar al almacenamiento.
3. **Análisis (en el servidor)** — el plano de análisis puntúa cada sesión
   y produce un veredicto por sesión (`human | watch | bot`) con confianza
   por eje y una identidad de dispositivo estable y resistente a
   colisiones.
4. **Devolución** — tu backend recupera el resultado a través de la API de
   resultados (SDKs en seis lenguajes; las proyecciones
   `public | sdk | diagnostic` controlan lo que ve cada llamador).

En el lado del servidor, greenpng se ejecuta como un único binario de host
con un plano de sondeo integrado en el árbol por defecto, y admite
**despliegue multinodo con balanceo de carga**: un módulo LB reparte el
tráfico de sondeo entre los nodos frente a una capa de datos compartida
(PostgreSQL + Redis), de modo que la recolección y el análisis escalan
horizontalmente.

## 2. Estructura del repositorio y arquitectura

| Directorio | Contenido |
|---|---|
| `crates/` | Workspace de Rust — `gr-service` (plano de control + consola admin + plano de sondeo integrado), `gr-probe-core`, `gr-probe-plane`, `gr-probe-store`, `gr-ota`, `gr-admin`, `gr-runtime`, … |
| `modules/` | Fuentes de módulos firmados de actualización en caliente (identity / brain / analyze / ingest / edge / probe_assets) |
| `probe/` | Sonda FE para navegador (loader, cadena de packs, cliente de ingesta sellado) |
| `panel/` | Panel de administración — fuentes Vue (`admin-ui/`) + SPA compilado (`admin-spa/`), EN + 中文 |
| `sdk/` | SDKs de integración de backend en seis lenguajes (solo recuperación de resultados) |
| `spec/` | Especificaciones de wire/scoring cargadas en tiempo de ejecución |
| `fixtures/` | Datos de pruebas de contrato |
| `scripts/` | Scripts de build y herramientas FE (`scripts/fe/checks/`) |
| `vendor/` | Fuentes de dependencias vendorizadas (pingora) |
| `install/` | Instalador, compose de la capa de datos, scripts de actualización |
| `release/` | Scripts de empaquetado (build multi-arquitectura, SBOM, firma de módulos) |
| `docs/` | Esta guía, un archivo por idioma |

```
        navegador del visitante
              │  <script src="/gr.js">  (fijado, no-store)
              ▼
   FE loader ──► /v1/sdk/bootstrap ──► manifiesto de packs versionado
              │        asset_base /dist/v/<fe>/g/<gen>/
              ▼
   recolectores de packs (entrada · dispositivo · entorno, multilote)
              │  envío sellado (directo al dominio gv vinculado, TLS)
              ▼
 ┌────────────┴─────────────┐   ┌──────────────────────────────┐
 │ gr-probe-plane (Pingora)  │   │ gr-service (plano de control) │
 │  gateway · ingest · ops   │◄──┤  consola admin · config sitios │
 └────────────┬─────────────┘   │  registro de módulos (OTA, firmado)│
              ▼                 └──────────────┬───────────────┘
   PostgreSQL (+ Redis en multinodo)             │
              ▼                                  │
   GET /v1/session/{id}/result ──► SDK del comercio (seis lenguajes)

   multinodo: el módulo LB reparte el tráfico de sondeo entre nodos gr-service
   frente a la capa de datos compartida
```

Componentes clave: `gr-service` es el plano de control (consola admin de
ruta aleatoria, gestión de sitios/config, registro de módulos OTA firmados)
y aloja el plano de sondeo integrado en el árbol; `gr-probe-plane` es el
gateway Pingora con ingesta sellada y APIs de sesión/resultado; el FE del
navegador resuelve cada asset a una URL inmutable por versión, de modo que
las cachés nunca pueden servir una sonda obsoleta entre releases;
PostgreSQL (más Redis en multinodo) almacena sesiones, lotes y resultados
de análisis.

## 3. Instalación y uso

### 3.1 Instalación

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh | bash
# o con una versión / arquitectura explícitas:
bash install/install.sh --version <VERSION> --arch x86_64 --yes
# con una capa de datos dockerizada (solo PostgreSQL/Redis; el servidor sigue siendo un binario de host):
bash install/install.sh --version <VERSION> --with-docker --yes
```

El instalador verifica firmas sha256 + ELF + ed25519 de los módulos,
instala bajo `/opt/greenpng`, escribe `.env` y la unidad de systemd, prepara
y activa los seis módulos firmados, y comprueba el
`/v1/health` del control/plano. Las credenciales del primer inicio de
sesión y la ruta aleatoria de la consola se escriben en
`/opt/greenpng/data/admin/admin_bootstrap_once.txt`.

### 3.2 Actualización

| Prioridad | Canal | Para |
|---|---|---|
| P0 | OTA del panel (set-release-url → install / install-fe / install-runtime) | por defecto |
| P1 | `install/release/update_runtime_from_github.sh`, `update_module_from_github.sh` | sin panel / nodos gratuitos |
| P2 | SSH manual | proceso muerto / primera instalación |

Todas las actualizaciones descargan los assets firmados del Release de la
misma etiqueta desde este repositorio. Docker es un contenedor de
runtime, no un canal de actualización.

### 3.3 Uso

1. **Crea un sitio** en el panel de administración: id del sitio, dominios
   raíz y la lista de cookies permitidas para los campos de negocio que
   quieres adjuntar a cada veredicto (los nombres sensibles como
   `password`/`token` se bloquean en el lado del servidor).
2. **Despliega la sonda** — tres modos:
   - *Nginx first-party (recomendado)*: en el dominio pv, proxya `/gr.js` +
     `/gr/dist/v/` + `/gr/v1/` al plano de sondas (Cookie passthrough), e inserta
     `<script src="/gr.js" data-site-id="…" data-endpoint="/gr"
     data-inject-path="nginx" defer></script>` en el HTML.
   - *Cloudflare worker*: inserta la misma etiqueta y proxya `/gr`
     in-origin.
   - *Incrustación en la app*: carga el loader directamente desde pv/CDN
     con `data-endpoint` apuntando a gv/pv.
   Las subidas del navegador siempre van **directamente al dominio gv
   vinculado a través de TLS**.
3. **Recibe los resultados** — crea una clave SDK del sitio en el panel y
   luego consulta:

```bash
KEY="grsk_..." SID="cycle_..." BASE="https://gv.example.com"
curl -sS -H "X-Gr-Sdk-Key: $KEY" "$BASE/v1/session/$SID/result"
```

4. **Prueba de humo** (curl):

```bash
# abrir una sesión con cookies permitidas
curl -sS -X POST "$BASE/v1/session/open" -H "X-Gr-Sdk-Key: $KEY" \
     -H 'Cookie: user_id=u9; plan_tier=pro' -d '{"site_id":"mysite"}'
# → {"session_id":"cycle_…", …}
# luego comprobar el resultado (tras lotes FE reales o simulados):
curl -sS -H "X-Gr-Sdk-Key: $KEY" "$BASE/v1/session/$SID/result"
```

---

## 4. Parámetros del panel de administración

Se editan en la página **Config** del panel (guardar → publicar): el nodo
que publica aplica de inmediato; los nodos del clúster en ≤30 s, sin reinicio.

**Límites de tasa** (política v1.0.14: los totales por sitio están
desactivados por defecto; la ruta de telemetría se limita por IP individual;
las respuestas 429 indican la capa activada):

| Parámetro | Por defecto | Significado |
|---|:---:|---|
| `rate_limit_open_per_min` | 0 = sin límite | aperturas de sesión / sitio / min |
| `rate_limit_ingest_per_min` | 0 = sin límite | subidas por lotes / sitio / min |
| `rate_limit_analyze_per_min` | 0 = sin límite | análisis directos / sitio / min |
| `rate_limit_complete_per_min` | 0 = sin límite | acuses complete / sitio / min |
| `rate_limit_result_per_min` | 0 = sin límite | lecturas de resultado / sitio / min |
| `rate_limit_client_event_per_min` | 0 = sin límite | telemetría FE / sitio / min (total) |
| `rate_limit_client_event_per_ip_per_min` | 100 | telemetría FE **por IP individual** / min — superarlo limita solo esa IP; 0 = desactivado |

**Tras un CDN**, la capa por IP usa la IP que ve el servidor. Añada los CIDR
del proxy a `GR_TRUSTED_PROXIES` en `/opt/greenpng/.env` y restaure la IP
real del visitante en el proxy frontal (ejemplo nginx):

```nginx
set_real_ip_from 173.245.48.0/20;  # Cloudflare IPv4
set_real_ip_from 2400:cb00::/32;   # Cloudflare IPv6
real_ip_header CF-Connecting-IP;
```

**Niveles caliente/frío** (nombres reales): `cold_ttl_ms` (604800000 = 7
días), `cold_promote_window_ms` (864000000 = 24 h), `cold_purge_interval_ms`
(300000 = 5 min); la retención por sitio se configura en la página
**Data Retention** del panel y se purga en lotes acotados.

## 5. Registro y memoria

- Registros del servicio: `journalctl -u greenpng.service`; la telemetría
  operativa (`ops_client_events`) se conserva `ops_retention_days` (14) días.
- **Nota de memoria (v1.0.14+)**: en hosts multinúcleo de larga duración
  glibc puede mantener hasta 8 arenas por núcleo (~64 MB cada una), por lo
  que el RSS puede subir en escalera bajo concurrencia. El instalador fija
  `MALLOC_ARENA_MAX=4` en `/opt/greenpng/.env`; el RSS se mantiene estable
  bajo carga.

## 6. Proyectos de código abierto y referencias

- [Cloudflare Pingora](https://github.com/cloudflare/pingora) (Apache-2.0) — pasarela de borde, terminación TLS, ingest sellado (feature openssl).
- [Tokio](https://github.com/tokio-rs/tokio) & [Axum](https://github.com/tokio-rs/axum) (MIT) — runtime asíncrono y framework REST del plano de control.
- [OpenSSL](https://www.openssl.org/) (Apache-2.0) — backend TLS del borde y la consola de administración.
- [ed25519-dalek](https://github.com/dalek-cryptography/curve25519-dalek) (BSD-3) — firmas para manifiestos, activos de sonda y OTA.
- [PostgreSQL](https://www.postgresql.org/) & [Redis](https://redis.io/) — almacenamiento L3 y estado multi-nodo.
- [flate2 / zlib](https://github.com/rust-compress/flate2) (MIT) — compresión de payloads (zstd solo dentro del pingora vendored).
- [Element Plus](https://element-plus.org/) & [Vue 3](https://vuejs.org/) (MIT) — UI de la consola.
- Referencias de investigación: [CreepJS](https://github.com/abrahamjuliot/creepjs) (inspiración B1/B12), [FingerprintJS](https://github.com/fingerprintjs/fingerprintjs), [BotD](https://github.com/fingerprintjs/botd).
