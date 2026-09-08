# greenpng — Guia do projeto (Português)

## 1. O que é este projeto

greenpng (GR) verifica se uma sessão pertence a um humano real sondando o
**navegador do verdadeiro visitante**, e não apenas inspecionando o tráfego.

O pipeline, de ponta a ponta:

1. **Sonda (no navegador)** — um loader FE assinado roda nas suas páginas e
   coleta evidências de **múltiplas fontes** (dinâmica de entrada, pilha de
   dispositivo, autenticidade do ambiente, rastros de automação) em
   **múltiplos lotes encadeados** por sessão.
2. **Upload** — o navegador envia cada lote ao servidor por um pipeline de
   ingestão selado; envios repetidos ou adulterados são rejeitados antes de
   alcançar o armazenamento.
3. **Análise (no servidor)** — o plano de análise pontua cada sessão em um
   veredito por sessão (`human | watch | bot`) com confiança por eixo e uma
   identidade de dispositivo estável e resistente a colisões.
4. **Retorno** — seu backend recupera o resultado pela API de resultados
   (SDKs em seis linguagens; projeções `public | sdk | diagnostic` controlam
   o que cada chamador vê).

No lado do servidor, greenpng roda como um binário de host com um plano de
sonda integrado (in-tree) por padrão, e suporta **implantação multi-nó com
balanceamento de carga**: um módulo LB distribui o tráfego de sondas entre os
nós, à frente de uma camada de dados compartilhada (PostgreSQL + Redis), de
modo que coleta e análise escalam horizontalmente.

## 2. Estrutura do repositório & arquitetura

| Diretório | Conteúdo |
|---|---|
| `crates/` | Workspace Rust — `gr-service` (plano de controle + console admin + plano de sonda in-tree), `gr-probe-core`, `gr-probe-plane`, `gr-probe-store`, `gr-ota`, `gr-admin`, `gr-runtime`, … |
| `modules/` | Fontes dos módulos de atualização a quente assinados (identity / brain / analyze / ingest / edge / probe_assets) |
| `probe/` | FE da sonda no navegador (loader, cadeia de packs, cliente de ingestão selado) |
| `panel/` | Painel admin — fontes Vue (`admin-ui/`) + SPA compilado (`admin-spa/`), EN + 中文 |
| `sdk/` | SDKs de integração de backend em seis linguagens (apenas recuperação de resultados) |
| `spec/` | Especificações de wire/pontuação carregadas em tempo de execução |
| `fixtures/` | Dados de testes de contrato |
| `scripts/` | Scripts de build e ferramentas FE (`scripts/fe/checks/`) |
| `vendor/` | Fontes de dependências vendorizadas (pingora) |
| `install/` | Instalador, compose da camada de dados, scripts de upgrade |
| `release/` | Scripts de empacotamento (build multi-arquitetura, SBOM, assinatura de módulos) |
| `docs/` | Este guia, um arquivo por idioma |

```
        navegador do visitante
              │  <script src="/gr.js">  (pinned, no-store)
              ▼
   FE loader ──► /v1/sdk/bootstrap ──► manifesto de packs versionado
              │        asset_base /dist/v/<fe>/g/<gen>/
              ▼
   coletores do pack (entrada · dispositivo · ambiente, multi-lote)
              │  envio selado (direto ao domínio gv vinculado, TLS)
              ▼
 ┌────────────┴─────────────┐   ┌──────────────────────────────┐
 │ gr-probe-plane (Pingora)  │   │ gr-service (plano de controle)│
 │  gateway · ingest · ops   │◄──┤  console admin · config site │
 └────────────┬─────────────┘   │  registro de módulos (OTA,    │
              ▼                 │  assinados)                   │
   PostgreSQL (+ Redis em       └──────────────┬───────────────┘
   multi-nó)                                   │
              ▼                                │
   GET /v1/session/{id}/result ──► SDK do comerciante (seis linguagens)

   multi-nó: o módulo LB distribui o tráfego de sondas entre nós gr-service
   à frente da camada de dados compartilhada
```

Componentes-chave: `gr-service` é o plano de controle (console admin com
caminho aleatório, gestão de site/config, registro de módulos OTA assinados)
e hospeda o plano de sonda in-tree; `gr-probe-plane` é o gateway Pingora com
ingestão selada e APIs de sessão/resultado; o FE do navegador resolve cada
asset para uma URL imutável por versão, de modo que caches nunca sirvam uma
sonda desatualizada entre releases; PostgreSQL (mais Redis em multi-nó)
armazena sessões, lotes e resultados de análise.

## 3. Instalação & uso

### 3.1 Instalar

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh | bash
# ou com uma versão / arquitetura explícita:
bash install/install.sh --version <VERSION> --arch x86_64 --yes
# com camada de dados via docker (apenas PostgreSQL/Redis; o servidor continua um binário de host):
bash install/install.sh --version <VERSION> --with-docker --yes
```

O instalador verifica assinaturas sha256 + ELF + ed25519 dos módulos, instala
sob `/opt/greenpng`, grava o `.env` e a unidade systemd, prepara e ativa os
seis módulos assinados, e só libera mediante o `/v1/health` de
controle/plano. As credenciais do primeiro login e o caminho aleatório do
console são gravados em
`/opt/greenpng/data/admin/admin_bootstrap_once.txt`.

### 3.2 Atualizar

| Prioridade | Canal | Para |
|---|---|---|
| P0 | OTA pelo painel (set-release-url → install / install-fe / install-runtime) | padrão |
| P1 | `install/release/update_runtime_from_github.sh`, `update_module_from_github.sh` | sem painel / nós livres |
| P2 | SSH manual | processo morto / primeira instalação |

Todas as atualizações baixam os assets assinados da mesma tag do Release
deste repositório. Docker é um contêiner de runtime, não um canal de
atualização.

### 3.3 Usar

1. **Crie um site** no painel admin: id do site, domínios raiz e a
   allow-list de cookies para campos de negócio que você quer anexados a cada
   veredito (nomes sensíveis como `password`/`token` são bloqueados no
   servidor).
2. **Implante a sonda** — três modos:
   - *Nginx first-party (recomendado)*: faça proxy de `/gr.js` + `/gr/dist/v/`
     para o domínio pv e de `/gr/v1/` para o domínio gv (repasse de Cookie),
     e injete
     `<script src="/gr.js" data-site-id="…" data-endpoint="/gr"
     data-inject-path="nginx" defer></script>` no HTML.
   - *Cloudflare worker*: injete a mesma tag e faça proxy de `/gr` na origem.
   - *App embed*: carregue o loader diretamente do pv/CDN com
     `data-endpoint` apontando para gv/pv.
   Os uploads do navegador sempre vão **direto ao domínio gv vinculado sobre
   TLS**.
3. **Receba resultados** — crie uma chave SDK de site no painel e consulte:

```bash
KEY="grsk_..." SID="cycle_..." BASE="https://gv.example.com"
curl -sS -H "X-Gr-Sdk-Key: $KEY" "$BASE/v1/session/$SID/result"
```

4. **Smoke test** (curl):

```bash
# abre uma sessão carregando cookies da allow-list
curl -sS -X POST "$BASE/v1/session/open" -H "X-Gr-Sdk-Key: $KEY" \
     -H 'Cookie: user_id=u9; plan_tier=pro' -d '{"site_id":"mysite"}'
# → {"session_id":"cycle_…", …}
# depois verifique o resultado (após lotes FE reais ou simulados):
curl -sS -H "X-Gr-Sdk-Key: $KEY" "$BASE/v1/session/$SID/result"
```
