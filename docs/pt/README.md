# greenpng — Product Guide (Português)

Verificação humana no navegador e inteligência anti-bot: uma sonda
assinada que roda nos navegadores de visitantes reais, um pipeline de
ingestão selado e um plano de análise que devolve vereditos por sessão
através de SDKs em seis linguagens.

> Este guia está disponível em 12 idiomas — consulte o
> [índice de idiomas](../../README.md#documentation) na raiz do repositório.

## 1. O que o greenpng faz

Cada sessão de visitante é pontuada no dispositivo e no servidor:

- **Veredito humano vs bot** — `human | watch | bot` com confiança por eixo
  (dinâmica de entrada, consistência da pilha do dispositivo, autenticidade
  do ambiente, rastros de automação, reuso de histórico).
- **Identidade estável do dispositivo** — um ID de dispositivo consciente de
  colisões que permanece estável entre sessões sem depender de cookies de
  terceiros.
- **Captura de campos de negócio** — seus campos de cookie autorizados
  (`user_id`, `plan_tier`, …) são capturados na abertura da sessão e
  anexados ao veredito. Nomes sensíveis (password/token/…) são bloqueados
  no servidor.
- **Inteligência de IP** — os IPs dos visitantes têm a privacidade
  mascarada para /24 (IPv4) ou /48 (IPv6) na ingestão; o enriquecimento
  opcional mapeia a sub-rede para ASN/país/cidade via DB-IP (MMDB incluído),
  IPinfo, MaxMind ou um enriquecedor HTTP personalizado. Segredos nunca
  saem do servidor.
- **API de recuperação de resultados** — os lojistas consultam o veredito
  com uma chave SDK por site; as projeções (`public | sdk | diagnostic`)
  controlam a exposição.

## 2. Principais recursos

| Recurso | O que entrega |
|---|---|
| Sonda FE assinada | Pacotes de navegador à prova de adulteração (ed25519), URLs de assets versionados e imutáveis `/dist/v/<ver>/g/<gen>/…` — seguro para CDN, sem cache poisoning |
| Ingestão selada | Submissões de pacotes são seladas; replay/adulteração rejeitados a montante |
| Módulos de análise (OTA) | identity / brain / analyze / ingest / edge / probe_assets atualizam a quente como módulos assinados sem downtime |
| Painel administrativo | Console independente em um caminho aleatório, administrador único scrypt, log de auditoria, gerenciamento de sites/estratégias/integrações/retenção/DSAR, EN + 中文 |
| Privacidade por padrão | Mascaramento de IP no ponto de ingestão mais precoce, lista de permissões de cookies, exportação/apagamento DSAR, limpeza por retenção |
| SDKs em seis linguagens | Clientes JS / Python / Go / PHP / Shell / Rust para `wait_for_result` |
| Pronto para multi-nó | Módulo LB embutido, heartbeat de cluster, espelho OTA |

## 3. Arquitetura

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

**Formatos de implantação**

- **gr-service** — um processo: console administrativo + APIs de controle +
  probe plane integrado. Porta 28680 (console, caminho aleatório) + 28765
  (loopback do plane).
- **Nginx do site de negócio** — serve `/gr.js` e (opcionalmente) faz proxy
  do prefixo de API de mesma origem; uploads do navegador devem usar o
  domínio GV vinculado através do TLS do Pingora.
- **Bancos de dados** — PostgreSQL para os repositórios de controle +
  probe (existe um esqueleto em SQLite para o laboratório).

## 4. Instalação rápida

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh
bash install.sh --version <VERSION>            # see the VERSION file / Releases
# or with a dockerized data layer:
bash install.sh --version <VERSION> --with-docker --yes
```

O instalador verifica sha256 + ELF + assinaturas de módulos ed25519,
instala sob `/opt/greenpng`, escreve o `.env`, prepara e ativa os seis
módulos assinados, habilita a unidade systemd e aguarda o
`/v1/health`.

Primeiro login: credenciais de uso único são gravadas em
`/opt/greenpng/data/admin/admin_bootstrap_once.txt` — o caminho aleatório
do console (`/c-<hex>/`) é registrado lá. Não existe prefixo `/admin` ou
`/console/`, e o login por senha é a única entrada do painel.

## 5. Tutorial de uso

### 5.1 Criar um site

Painel **Sites → Create site**:

- `site_id` — o id do seu tenant, usado no embed
- `root_domains` — os hostnames www (CORS + vínculo de hostname)
- `cookie_fields` — a lista de permissões de cookies, ex.
  `["user_id", "plan_tier", "cart_id", "utm_source"]`

A linha do site flui de `control.sites → public.sites` (probe plane) ao
salvar, e o gr-service preenche sites pré-existentes na inicialização.

### 5.2 Implantar a sonda (três modos)

**A. Nginx first-party (recomendado)** — o vhost do seu site faz proxy do
boot loader e do prefixo de API de mesma origem:

```nginx
location = /gr.js      { proxy_pass https://pv.example.com/gr.js; proxy_set_header Host pv.example.com; }
location /gr/dist/v/   { proxy_pass https://pv.example.com/gr/dist/v/; proxy_set_header Host pv.example.com; }
location /gr/v1/       { proxy_pass https://gv.example.com/; proxy_set_header Cookie $http_cookie;
                         proxy_set_header X-Forwarded-For $remote_addr; }
```

```html
<script src="/gr.js" data-site-id="mysite" data-endpoint="/gr" data-inject-path="nginx" defer></script>
```

**B. Cloudflare worker** — o worker injeta o script de boot e faz proxy de
`/gr` na mesma origem. Evite o modo "Under Attack" em `/gr` (páginas de
challenge quebram a sonda).

**C. Script no site / embed via CDN** — carregue o boot JS diretamente do
PV e aponte `data-endpoint` para o GV.

Em todos os modos o loader resolve os pacotes através do bootstrap do SDK
e busca apenas URLs versionadas e imutáveis.

### 5.3 Receber resultados (SDK em seis linguagens)

Crie uma **chave de backend** para o seu site no painel (página SDK). O
SDK nunca repassa sondas; ele apenas consulta resultados:

```
GET {gv_base}/v1/session/{session_id}/result?projection=sdk
X-Gr-Sdk-Key: <site backend key>
```

| Linguagem | Ponto de entrada |
|---|---|
| JS | `sdk/src/index.js` — `new GrResultClient({baseUrl, apiKey}).waitForResult(...)` |
| Python | `sdk/python/gr_results.py` — `GrResultClient(base_url, api_key).wait_for_result(...)` |
| Go | `sdk/go/gr_results.go` — `gr.New(baseURL, key).WaitForResult(...)` |
| PHP | `sdk/php/GrResultsClient.php` — `(new GrResultClient(...))->waitForResult(...)` |
| Shell | `sdk/shell/gr_sdk.sh` — `gr_wait_for_result <session> sdk 8000` |
| Rust | `sdk/rust/` — `Client::new(base_url, key).wait_for_result(...)` |

As chaves de site são isoladas por escopo: uma chave emitida para o site A
não consegue ler as sessões do site B (`403 sdk key site mismatch`), e
chaves revogadas param de funcionar imediatamente (`401`).

### 5.4 Smoke test ponta a ponta (curl)

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

### 5.5 Mantê-lo atualizado

| Canal | Comando |
|---|---|
| OTA pelo painel (padrão) | Painel administrativo → Modules / Runtime install |
| Script de atualização | `VERSION=<v> INSTALL_ROOT=/opt/greenpng bash install/release/update_runtime_from_github.sh` |
| Manual | SSH + rollback do runtime anterior (`bin/releases/<v>` mantido) |

Toda atualização baixa os mesmos assets assinados do Release deste
repositório.

## 6. Estrutura do repositório

| Diretório | Conteúdo |
|---|---|
| `crates/` | Workspace Rust — gr-service, gr-probe-core, gr-probe-plane, gr-probe-store, gr-ota, gr-admin, gr-runtime, … |
| `modules/` | Fontes dos módulos assinados de atualização a quente (identity / brain / analyze / ingest / edge / probe_assets) |
| `probe/` | Sonda FE do navegador (loader, pacotes, ingestão selada) |
| `panel/` | Painel administrativo — fontes Vue (`admin-ui/`) + SPA compilado (`admin-spa/`) |
| `sdk/` | SDKs de recuperação de resultados (JS / Python / Go / PHP / Shell / Rust) |
| `spec/` | Specs carregadas em runtime (pesos de bot, catálogos) |
| `install/` | Instalador + atualizador + material systemd |
| `release/` | Scripts de empacotamento (bundle multi-arquitetura, atestado SLSA) |
| `scripts/` | Verificações de contrato do FE e utilitários |
| `docs/` | Este guia em 12 idiomas |
| `VERSION` | Fonte única de verdade para a versão do release |

## 7. Links

- Releases e ponto de entrada de instalação: este repositório
- Site oficial: https://www.greenpng.cc (apresentação do produto, EN + 中文)
- Idiomas do painel: English + 中文 (mantidos sincronizados em `panel/admin-ui/src/i18n/`)
