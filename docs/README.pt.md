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
   - *Nginx first-party (recomendado)*: no domínio pv, faça proxy de `/gr.js` +
     `/gr/dist/v/` + `/gr/v1/` para o plano de sondas (repasse de Cookie),
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

---

## 4. Parâmetros do painel de administração

Editados na página **Config** do painel (salvar → publicar): o nó que
publica aplica imediatamente; os nós do cluster em ≤30 s, sem reinício.

**Limites de taxa** (política v1.0.14: os totais por site ficam desligados
por padrão; a rota de telemetria é limitada por IP individual; as
respostas 429 nomeiam a camada acionada):

| Parâmetro | Padrão | Significado |
|---|:---:|---|
| `rate_limit_open_per_min` | 0 = ilimitado | aberturas de sessão / site / min |
| `rate_limit_ingest_per_min` | 0 = ilimitado | uploads de lote / site / min |
| `rate_limit_analyze_per_min` | 0 = ilimitado | análises diretas / site / min |
| `rate_limit_complete_per_min` | 0 = ilimitado | recibos complete / site / min |
| `rate_limit_result_per_min` | 0 = ilimitado | leituras de resultado / site / min |
| `rate_limit_client_event_per_min` | 0 = ilimitado | telemetria FE / site / min (total) |
| `rate_limit_client_event_per_ip_per_min` | 100 | telemetria FE **por IP individual** / min — exceder limita apenas esse IP; 0 = desligado |

**Atrás de um CDN**, a camada por-IP usa o IP que o servidor vê. Adicione os
CIDRs do proxy ao `GR_TRUSTED_PROXIES` em `/opt/greenpng/.env` e restaure o
IP real do visitante no proxy frontal (exemplo nginx):

```nginx
set_real_ip_from 173.245.48.0/20;  # Cloudflare IPv4
set_real_ip_from 2400:cb00::/32;   # Cloudflare IPv6
real_ip_header CF-Connecting-IP;
```

**Camadas quente/fria** (nomes reais dos controles): `cold_ttl_ms`
(604800000 = 7 dias), `cold_promote_window_ms` (864000000 = 24 h),
`cold_purge_interval_ms` (300000 = 5 min); a retenção por site é definida na
página **Data Retention** do painel e eliminada em lotes limitados.

## 5. Logs e memória

- Logs do serviço: `journalctl -u greenpng.service`; a telemetria
  operacional (`ops_client_events`) é mantida por `ops_retention_days` (14) dias.
- **Nota de memória (desde v1.0.14)**: em hosts multi-core de longa duração
  o glibc pode manter até 8 arenas por núcleo (~64 MB cada), então o RSS
  pode subir em degraus sob concorrência. O instalador define
  `MALLOC_ARENA_MAX=4` em `/opt/greenpng/.env`; o RSS permanece estável
  sob carga.

## 6. Projetos open source e referências

- [Cloudflare Pingora](https://github.com/cloudflare/pingora) (Apache-2.0) — gateway de borda, terminação TLS, ingest selado (feature openssl).
- [Tokio](https://github.com/tokio-rs/tokio) & [Axum](https://github.com/tokio-rs/axum) (MIT) — runtime assíncrono e framework REST do plano de controle.
- [OpenSSL](https://www.openssl.org/) (Apache-2.0) — backend TLS da borda e do console de administração.
- [ed25519-dalek](https://github.com/dalek-cryptography/curve25519-dalek) (BSD-3) — assinaturas para manifestos de release, ativos de sonda e OTA.
- [PostgreSQL](https://www.postgresql.org/) & [Redis](https://redis.io/) — armazenamento L3 e estado multi-nó.
- [flate2 / zlib](https://github.com/rust-compress/flate2) (MIT) — compressão de payloads (zstd apenas dentro do pingora vendored).
- [Element Plus](https://element-plus.org/) & [Vue 3](https://vuejs.org/) (MIT) — UI do console de administração.
- Referências de pesquisa: [CreepJS](https://github.com/abrahamjuliot/creepjs) (inspiração B1/B12), [FingerprintJS](https://github.com/fingerprintjs/fingerprintjs), [BotD](https://github.com/fingerprintjs/botd).
