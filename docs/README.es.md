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
   - *Nginx first-party (recomendado)*: proxya `/gr.js` + `/gr/dist/v/` al
     dominio pv y `/gr/v1/` al dominio gv (Cookie passthrough), e inserta
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
