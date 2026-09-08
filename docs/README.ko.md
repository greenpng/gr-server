# greenpng — 프로젝트 가이드（한국어）

## 1. 이 프로젝트란

greenpng(GR)은 트래픽만 검사하는 방식이 아니라 **실제 방문자의 브라우저**를
탐침(probe)하여 해당 세션이 실제 인간인지 판별합니다.

전체 파이프라인:

1. **탐침(브라우저 내)** — 서명된 FE 로더가 페이지에서 실행되어 세션당
   **여러 단계의 배치**로 **다중 출처**(입력 역학, 디바이스 스택, 환경
   진위성, 자동화 흔적)에서 증거를 수집합니다.
2. **업로드** — 브라우저가 각 배치를 봉인된(sealed) 수집 파이프라인을 통해
   서버로 제출하며, 재전송되거나 변조된 제출은 저장소에 도달하기 전에
   거부됩니다.
3. **분석(서버)** — 분석 플레인이 각 세션을 축별 신뢰도와 안정적이고
   충돌을 고려한 디바이스 신원과 함께 세션별 판정(`human | watch | bot`)으로
   점수화합니다.
4. **반환** — 백엔드가 결과 API를 통해 결과를 조회합니다
   (6개 언어 SDK; `public | sdk | diagnostic` 프로젝션이 각 호출자에게
   보이는 범위를 제어).

서버 측에서 greenpng는 기본적으로 트리 내장(in-tree) 탐침 플레인을 포함한
단일 호스트 바이너리로 실행되며, **다중 노드 부하 분산 배포**를 지원합니다.
LB 모듈이 공유 데이터 레이어(PostgreSQL + Redis) 앞에서 탐침 트래픽을
노드들에 분산시켜 수집과 분석이 수평적으로 확장됩니다.

## 2. 저장소 구성 및 아키텍처

| 디렉터리 | 내용 |
|---|---|
| `crates/` | Rust 워크스페이스 — `gr-service`(컨트롤 플레인 + 어드민 콘솔 + 트리 내장 탐침 플레인), `gr-probe-core`, `gr-probe-plane`, `gr-probe-store`, `gr-ota`, `gr-admin`, `gr-runtime`, … |
| `modules/` | 서명된 핫 업데이트 모듈 소스(identity / brain / analyze / ingest / edge / probe_assets) |
| `probe/` | 브라우저 탐침 FE(로더, 팩 체인, 봉인된 수집 클라이언트) |
| `panel/` | 어드민 패널 — Vue 소스(`admin-ui/`) + 빌드된 SPA(`admin-spa/`), EN + 中文 |
| `sdk/` | 6개 언어의 백엔드 통합 SDK(결과 조회 전용) |
| `spec/` | 런타임에 로드되는 와이어/스코어링 사양 |
| `fixtures/` | 컨트랙트 테스트 데이터 |
| `scripts/` | 빌드 스크립트 및 FE 도구(`scripts/fe/checks/`) |
| `vendor/` | 벤더링된 의존성 소스(pingora) |
| `install/` | 설치 프로그램, 데이터 레이어 compose, 업그레이드 스크립트 |
| `release/` | 패키징 스크립트(멀티 아키텍처 빌드, SBOM, 모듈 서명) |
| `docs/` | 본 가이드, 언어별 파일 하나씩 |

```
        visitor browser
              │  <script src="/gr.js">  (pinned, no-store)
              ▼
   FE loader ──► /v1/sdk/bootstrap ──► versioned pack manifest
              │        asset_base /dist/v/<fe>/g/<gen>/
              ▼
   pack collectors (input · device · environment, multi-batch)
              │  sealed submit (bound gv 도메인으로 직접, TLS)
              ▼
 ┌────────────┴─────────────┐   ┌──────────────────────────────┐
 │ gr-probe-plane (Pingora)  │   │ gr-service (control plane)    │
 │  gateway · ingest · ops   │◄──┤  admin console · site config  │
 └────────────┬─────────────┘   │  module registry (OTA, signed)│
              ▼                 └──────────────┬───────────────┘
   PostgreSQL (+ Redis in multi-node)          │
              ▼                                │
   GET /v1/session/{id}/result ──► merchant SDK (6개 언어)

   multi-node: LB module가 공유 데이터 레이어 앞의 gr-service 노드들에
   탐침 트래픽을 분산
```

핵심 구성 요소: `gr-service`는 컨트롤 플레인입니다(랜덤 경로 어드민 콘솔,
사이트/설정 관리, 서명된 OTA 모듈 레지스트리). 그리고 트리 내장 탐침
플레인을 호스팅합니다. `gr-probe-plane`은 봉인된 수집과 세션/결과 API를
갖춘 Pingora 게이트웨이입니다. 브라우저 FE는 모든 애셋을 버전 불변
URL로 해석하므로 캐시가 릴리스 간에 오래된 탐침을 제공하는 일이 없습니다.
PostgreSQL(다중 노드 시 Redis 추가)은 세션, 배치, 분석 결과를 저장합니다.

## 3. 설치 및 사용

### 3.1 설치

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh | bash
# 또는 명시적인 버전 / 아키텍처 지정:
bash install/install.sh --version <VERSION> --arch x86_64 --yes
# docker화된 데이터 레이어 사용(PostgreSQL/Redis만; 서버는 호스트 바이너리 유지):
bash install/install.sh --version <VERSION> --with-docker --yes
```

설치 프로그램은 sha256 + ELF + ed25519 모듈 서명을 검증하고, `/opt/greenpng`
아래에 설치하며, `.env`와 systemd 유닛을 작성하고, 6개의 서명된 모듈을
스테이징하여 활성화하고, 컨트롤/플레인의 `/v1/health`를 확인합니다.
첫 로그인 자격 증명과 랜덤 콘솔 경로는
`/opt/greenpng/data/admin/admin_bootstrap_once.txt`에 기록됩니다.

### 3.2 업데이트

| 우선순위 | 채널 | 대상 |
|---|---|---|
| P0 | 패널 OTA(set-release-url → install / install-fe / install-runtime) | 기본 |
| P1 | `install/release/update_runtime_from_github.sh`, `update_module_from_github.sh` | 패널 없음 / 프리 노드 |
| P2 | SSH 수동 | 죽은 프로세스 / 첫 설치 |

모든 업데이트는 이 저장소에서 동일한 태그의 서명된 Release 애셋을 가져옵니다.
Docker는 런타임 컨테이너이며 업데이트 채널이 아닙니다.

### 3.3 사용

1. **사이트 생성** — 어드민 패널에서 사이트 id, 루트 도메인, 그리고 각 판정에
   첨부할 비즈니스 필드의 쿠키 허용 목록을 설정합니다(`password`/`token` 같은
   민감한 이름은 서버 측에서 차단됩니다).
2. **탐침 배포** — 세 가지 모드:
   - *Nginx 자사 배포(권장)*: `/gr.js`와 `/gr/dist/v/`를 pv 도메인으로,
     `/gr/v1/`을 gv 도메인으로 프록시하고(Cookie 통과), HTML에
     `<script src="/gr.js" data-site-id="…" data-endpoint="/gr"
     data-inject-path="nginx" defer></script>`를 삽입합니다.
   - *Cloudflare worker*: 동일한 태그를 삽입하고 `/gr`을 인오리진(in-origin)으로
     프록시합니다.
   - *앱 임베드*: `data-endpoint`를 gv/pv로 지정하여 pv/CDN에서 로더를 직접
     로드합니다.
   브라우저 업로드는 항상 **바인딩된 gv 도메인으로 TLS를 통해 직접** 전송됩니다.
3. **결과 수신** — 패널에서 사이트 SDK 키를 만든 뒤 폴링:

```bash
KEY="grsk_..." SID="cycle_..." BASE="https://gv.example.com"
curl -sS -H "X-Gr-Sdk-Key: $KEY" "$BASE/v1/session/$SID/result"
```

4. **스모크 테스트**(curl):

```bash
# 허용 목록에 있는 쿠키를 담아 세션 열기
curl -sS -X POST "$BASE/v1/session/open" -H "X-Gr-Sdk-Key: $KEY" \
     -H 'Cookie: user_id=u9; plan_tier=pro' -d '{"site_id":"mysite"}'
# → {"session_id":"cycle_…", …}
# 이후 결과 확인(실제 FE 배치 이후 또는 시뮬레이션 배치 이후):
curl -sS -H "X-Gr-Sdk-Key: $KEY" "$BASE/v1/session/$SID/result"
```
