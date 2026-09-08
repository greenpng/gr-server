# greenpng — Product Guide (한국어)

브라우저 기반 휴먼 검증 및 안티봇 인텔리전스: 실제 방문자의 브라우저에서
실행되는 서명된 프로브, 봉인된 수집 파이프라인, 그리고 6개 언어의 SDK를
통해 세션별 판정을 반환하는 분석 플레인으로 구성됩니다.

> 이 가이드는 12개 언어로 제공됩니다 — 저장소 루트의
> [언어 인덱스](../../README.md#documentation)를 참고하세요.

## 1. greenpng가 하는 일

모든 방문자 세션은 기기 내부와 서버 양쪽에서 채점됩니다:

- **실사용자 vs 봇 판정** — 축별 신뢰도(입력 다이내믹스, 기기 스택 일관성,
  환경 진위성, 자동화 흔적, 히스토리 재사용)를 포함한
  `human | watch | bot` 판정.
- **안정적인 기기 신원** — 서드파티 쿠키에 의존하지 않으면서 세션 간
  안정적으로 유지되는, 충돌을 고려한 기기 ID.
- **비즈니스 필드 캡처** — 허용 목록에 등록된 쿠키 필드
  (`user_id`, `plan_tier`, …)를 세션 개시 시점에 캡처하여 판정에
  첨부합니다. 민감한 이름(password/token/…)은 서버 측에서 차단됩니다.
- **IP 인텔리전스** — 방문자 IP는 수집 시점에 /24(IPv4) 또는
  /48(IPv6)로 프라이버시 마스킹됩니다. 선택적 보강(enrichment)은
  DB-IP(번들 MMDB), IPinfo, MaxMind 또는 커스텀 HTTP enricher를 통해
  서브넷을 ASN/국가/도시로 매핑합니다. 시크릿은 절대 서버를 벗어나지
  않습니다.
- **결과 조회 API** — 가맹점은 사이트별 SDK 키로 판정을 폴링합니다.
  프로젝션(`public | sdk | diagnostic`)이 노출 범위를 제어합니다.

## 2. 핵심 기능

| 기능 | 제공하는 것 |
|---|---|
| 서명된 FE 프로브 | 변조 방지 브라우저 팩(ed25519), 버전 관리되는 불변 에셋 URL `/dist/v/<ver>/g/<gen>/…` — CDN 안전, 캐시 포이즈닝 없음 |
| 봉인된 수집 | 팩 제출은 봉인(sealed)되며, 리플레이/변조는 업스트림에서 거부 |
| 분석 모듈 (OTA) | identity / brain / analyze / ingest / edge / probe_assets가 다운타임 없이 서명된 모듈로 핫 업데이트 |
| 관리자 패널 | 무작위 경로의 독립형 콘솔, 단일 scrypt 관리자, 감사 로그, 사이트/전략/연동/보존/DSAR 관리, EN + 中文 |
| 기본 적용되는 프라이버시 | 가장 이른 수집 지점에서의 IP 마스킹, 쿠키 허용 목록, DSAR 내보내기/삭제, 보존 기간 만료 삭제 |
| 6개 언어 SDK | `wait_for_result`를 위한 JS / Python / Go / PHP / Shell / Rust 클라이언트 |
| 멀티 노드 지원 | 내장 LB 모듈, 클러스터 하트비트, OTA 미러 |

## 3. 아키텍처

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

**배포 형태**

- **gr-service** — 단일 프로세스: 관리자 콘솔 + 컨트롤 API + 인트리(in-tree)
  프로브 플레인. 포트 28680(콘솔, 무작위 경로) + 28765(플레인 루프백).
- **비즈니스 사이트 nginx** — `/gr.js`를 서빙하고 (선택적으로) 동일 오리진
  API 프리픽스를 프록시합니다. 브라우저 업로드는 바인딩된 GV 도메인을 통해
  Pingora TLS로 반드시 전송되어야 합니다.
- **DB** — 컨트롤 + 프로브 저장소로 PostgreSQL(랩용 SQLite 스켈레톤 존재).

## 4. 빠른 설치

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh
bash install.sh --version <VERSION>            # see the VERSION file / Releases
# or with a dockerized data layer:
bash install.sh --version <VERSION> --with-docker --yes
```

설치 프로그램은 sha256 + ELF + ed25519 모듈 서명을 검증하고,
`/opt/greenpng` 하위에 설치하며, `.env`를 작성하고, 6개의 서명된 모듈을
스테이징한 뒤 활성화하고, systemd 유닛을 활성화하며, `/v1/health`를
게이트 조건으로 사용합니다.

첫 로그인: 일회성 자격 증명이
`/opt/greenpng/data/admin/admin_bootstrap_once.txt`에 기록됩니다 —
무작위 콘솔 경로(`/c-<hex>/`)도 그곳에 기록됩니다. `/admin`이나
`/console/` 프리픽스는 존재하지 않으며, 비밀번호 로그인이 유일한 패널
진입 방법입니다.

## 5. 사용 튜토리얼

### 5.1 사이트 생성

패널 **Sites → Create site**:

- `site_id` — 임베드에 사용되는 테넌트 ID
- `root_domains` — www 호스트명(CORS + 호스트명 바인딩)
- `cookie_fields` — 쿠키 허용 목록, 예:
  `["user_id", "plan_tier", "cart_id", "utm_source"]`

사이트 행은 저장 시 `control.sites → public.sites`(프로브 플레인)로
전파되며, gr-service는 시작 시 기존 사이트를 백필합니다.

### 5.2 프로브 배포 (세 가지 모드)

**A. Nginx 퍼스트파티 (권장)** — 사이트 vhost가 부트 로더와 동일 오리진
API 프리픽스를 프록시합니다:

```nginx
location = /gr.js      { proxy_pass https://pv.example.com/gr.js; proxy_set_header Host pv.example.com; }
location /gr/dist/v/   { proxy_pass https://pv.example.com/gr/dist/v/; proxy_set_header Host pv.example.com; }
location /gr/v1/       { proxy_pass https://gv.example.com/; proxy_set_header Cookie $http_cookie;
                         proxy_set_header X-Forwarded-For $remote_addr; }
```

```html
<script src="/gr.js" data-site-id="mysite" data-endpoint="/gr" data-inject-path="nginx" defer></script>
```

**B. Cloudflare worker** — 워커가 부트 스크립트를 주입하고 `/gr`을
오리진 내에서 프록시합니다. `/gr`에서는 "Under Attack" 모드를 피하세요
(챌린지 페이지가 프로브를 망가뜨립니다).

**C. 사이트 스크립팅 / CDN 임베드** — PV에서 부트 JS를 직접 로드하고
`data-endpoint`를 GV로 지정합니다.

어떤 모드에서든 로더는 SDK bootstrap을 통해 팩을 해석하며, 버전 관리되는
불변 URL만 가져옵니다.

### 5.3 결과 수신 (6개 언어 SDK)

패널(SDK 페이지)에서 사이트의 **백엔드 키**를 생성하세요. SDK는 프로브를
전달하지 않으며, 결과만 조회합니다:

```
GET {gv_base}/v1/session/{session_id}/result?projection=sdk
X-Gr-Sdk-Key: <site backend key>
```

| 언어 | 진입점 |
|---|---|
| JS | `sdk/src/index.js` — `new GrResultClient({baseUrl, apiKey}).waitForResult(...)` |
| Python | `sdk/python/gr_results.py` — `GrResultClient(base_url, api_key).wait_for_result(...)` |
| Go | `sdk/go/gr_results.go` — `gr.New(baseURL, key).WaitForResult(...)` |
| PHP | `sdk/php/GrResultsClient.php` — `(new GrResultClient(...))->wait_for_result(...)` |
| Shell | `sdk/shell/gr_sdk.sh` — `gr_wait_for_result <session> sdk 8000` |
| Rust | `sdk/rust/` — `Client::new(base_url, key).wait_for_result(...)` |

사이트 키는 스코프가 지정됩니다: 사이트 A용으로 발급된 키는 사이트 B의
세션을 읽을 수 없으며(`403 sdk key site mismatch`), 폐기된 키는 즉시
작동을 멈춥니다(`401`).

### 5.4 엔드투엔드 스모크 (curl)

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

### 5.5 최신 상태 유지

| 채널 | 명령 |
|---|---|
| 패널 OTA (기본) | 관리자 패널 → Modules / Runtime install |
| 업데이터 스크립트 | `VERSION=<v> INSTALL_ROOT=/opt/greenpng bash install/release/update_runtime_from_github.sh` |
| 수동 | SSH + 이전 런타임 롤백(`bin/releases/<v>` 보관) |

모든 업데이트는 이 저장소의 동일한 서명된 Release 에셋을 가져옵니다.

## 6. 저장소 레이아웃

| 디렉터리 | 내용물 |
|---|---|
| `crates/` | Rust 워크스페이스 — gr-service, gr-probe-core, gr-probe-plane, gr-probe-store, gr-ota, gr-admin, gr-runtime, … |
| `modules/` | 서명된 핫 업데이트 모듈 소스 (identity / brain / analyze / ingest / edge / probe_assets) |
| `probe/` | 브라우저 프로브 FE (로더, 팩, 봉인된 수집) |
| `panel/` | 관리자 패널 — Vue 소스 (`admin-ui/`) + 빌드된 SPA (`admin-spa/`) |
| `sdk/` | 결과 조회 SDK (JS / Python / Go / PHP / Shell / Rust) |
| `spec/` | 런타임 로드 스펙 (봇 가중치, 카탈로그) |
| `install/` | 설치 프로그램 + 업데이터 + systemd 자료 |
| `release/` | 패키징 스크립트 (멀티 아키텍처 번들, SLSA 증명) |
| `scripts/` | FE 계약 검사 및 헬퍼 |
| `docs/` | 12개 언어로 제공되는 이 가이드 |
| `VERSION` | 릴리스 버전의 단일 진실 공급원 |

## 7. 링크

- 릴리스 및 설치 진입점: 이 저장소
- 공식 사이트: https://www.greenpng.cc (제품 소개, EN + 中文)
- 패널 로케일: English + 中文 (`panel/admin-ui/src/i18n/`에서 동기화 유지)
