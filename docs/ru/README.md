# greenpng — Product Guide (Русский)

Проверка «человек или бот» на стороне браузера и антибот-аналитика:
подписанный проб-зонд, исполняемый в браузерах реальных посетителей,
герметизированный ingest-конвейер и аналитический контур, возвращающий
вердикты по каждой сессии через SDK на шести языках.

> Это руководство доступно на 12 языках — см.
> [указатель языков](../../README.md#documentation) в корне репозитория.

## 1. Что делает greenpng

Каждая посетительская сессия оценивается на устройстве и на сервере:

- **Вердикт «человек или бот»** — `human | watch | bot` с уверенностью
  по каждой оси (динамика ввода, согласованность стека устройства,
  подлинность окружения, следы автоматизации, повторное использование
  истории).
- **Стабильная идентификация устройства** — устойчивый к коллизиям
  идентификатор устройства, стабильный между сессиями и не зависящий от
  сторонних cookie.
- **Захват бизнес-полей** — разрешённые вами cookie-поля из allow-списка
  (`user_id`, `plan_tier`, …) захватываются при открытии сессии и
  прикрепляются к вердикту. Чувствительные имена (password/token/…)
  блокируются на стороне сервера.
- **IP-аналитика** — IP посетителей маскируются в целях приватности до
  /24 (IPv4) или /48 (IPv6) при приёме; опциональное обогащение
  сопоставляет подсеть с ASN/страной/городом через DB-IP (поставляемый
  MMDB), IPinfo, MaxMind или кастомный HTTP-обогатитель. Секреты
  никогда не покидают сервер.
- **API получения результатов** — мерчанты опрашивают вердикт с помощью
  per-site SDK-ключа; проекции (`public | sdk | diagnostic`) управляют
  уровнем раскрытия данных.

## 2. Ключевые возможности

| Возможность | Что вы получаете |
|---|---|
| Подписанный FE-зонд | Защищённые от подмены браузерные паки (ed25519), версионируемые неизменяемые URL ассетов `/dist/v/<ver>/g/<gen>/…` — безопасно для CDN, без отравления кэша |
| Герметизированный ingest | Отправки пакетов опечатываются; replay и подмена отклоняются на верхнем уровне |
| Аналитические модули (OTA) | identity / brain / analyze / ingest / edge / probe_assets горячо обновляются как подписанные модули без простоя |
| Админ-панель | Автономная консоль на случайном пути, единственный scrypt-админ, журнал аудита, управление сайтами/стратегиями/интеграциями/хранением/DSAR, EN + 中文 |
| Приватность по умолчанию | Маскирование IP в самой ранней точке приёма, allow-список cookie, экспорт/стирание по DSAR, очистка по retention |
| SDK на шести языках | Клиенты JS / Python / Go / PHP / Shell / Rust для `wait_for_result` |
| Готовность к мульти-ноду | Встроенный LB-модуль, heartbeat кластера, OTA-зеркало |

## 3. Архитектура

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

**Варианты развёртывания**

- **gr-service** — один процесс: админ-консоль + управляющие API +
  встроенный probe plane. Порт 28680 (консоль, случайный путь) + 28765
  (loopback plane).
- **Nginx бизнес-сайта** — отдаёт `/gr.js` и (опционально) проксирует
  same-origin API-префикс; загрузки из браузера должны идти через
  привязанный GV-домен напрямую через Pingora TLS.
- **БД** — PostgreSQL для управляющего и probe-хранилищ (для лаборатории
  существует скелет на SQLite).

## 4. Быстрая установка

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh
bash install.sh --version <VERSION>            # see the VERSION file / Releases
# or with a dockerized data layer:
bash install.sh --version <VERSION> --with-docker --yes
```

Установщик проверяет sha256 + ELF + подписи модулей ed25519, устанавливает
всё под `/opt/greenpng`, записывает `.env`, размещает и активирует шесть
подписанных модулей, включает systemd-юнит и завершается только при
успешном `/v1/health`.

Первый вход: одноразовые учётные данные записываются в
`/opt/greenpng/data/admin/admin_bootstrap_once.txt` — там же фиксируется
случайный путь консоли (`/c-<hex>/`). Префиксов `/admin` или `/console/`
не существует; вход по паролю — единственный способ попасть в панель.

## 5. Практическое руководство

### 5.1 Создание сайта

В панели **Sites → Create site**:

- `site_id` — ваш идентификатор тенанта, используется во встраивании
- `root_domains` — www-хостнеймы (CORS + привязка хостнейма)
- `cookie_fields` — allow-список cookie, например
  `["user_id", "plan_tier", "cart_id", "utm_source"]`

Строка сайта при сохранении передаётся `control.sites → public.sites`
(probe plane), а gr-service дополняет существующие сайты при старте.

### 5.2 Развёртывание зонда (три режима)

**A. Nginx first-party (рекомендуется)** — vhost вашего сайта проксирует
boot-загрузчик и same-origin API-префикс:

```nginx
location = /gr.js      { proxy_pass https://pv.example.com/gr.js; proxy_set_header Host pv.example.com; }
location /gr/dist/v/   { proxy_pass https://pv.example.com/gr/dist/v/; proxy_set_header Host pv.example.com; }
location /gr/v1/       { proxy_pass https://gv.example.com/; proxy_set_header Cookie $http_cookie;
                         proxy_set_header X-Forwarded-For $remote_addr; }
```

```html
<script src="/gr.js" data-site-id="mysite" data-endpoint="/gr" data-inject-path="nginx" defer></script>
```

**B. Cloudflare worker** — воркер инжектирует boot-скрипт и проксирует
`/gr` в пределах origin. Избегайте режима "Under Attack" на `/gr`
(страницы challenge ломают зонд).

**C. Встраивание через скрипт сайта / CDN** — загружайте boot-JS напрямую
с PV и указывайте `data-endpoint` на GV.

В любом режиме загрузчик резолвит паки через SDK bootstrap и запрашивает
только версионируемые неизменяемые URL.

### 5.3 Получение результатов (SDK на шести языках)

Создайте **backend-ключ** для вашего сайта в панели (страница SDK). SDK
никогда не пересылает пробы; она только запрашивает результаты:

```
GET {gv_base}/v1/session/{session_id}/result?projection=sdk
X-Gr-Sdk-Key: <site backend key>
```

| Язык | Точка входа |
|---|---|
| JS | `sdk/src/index.js` — `new GrResultClient({baseUrl, apiKey}).waitForResult(...)` |
| Python | `sdk/python/gr_results.py` — `GrResultClient(base_url, api_key).wait_for_result(...)` |
| Go | `sdk/go/gr_results.go` — `gr.New(baseURL, key).WaitForResult(...)` |
| PHP | `sdk/php/GrResultsClient.php` — `(new GrResultClient(...))->waitForResult(...)` |
| Shell | `sdk/shell/gr_sdk.sh` — `gr_wait_for_result <session> sdk 8000` |
| Rust | `sdk/rust/` — `Client::new(base_url, key).wait_for_result(...)` |

Ключи сайтов изолированы по scope: ключ, выпущенный для сайта A, не
сможет читать сессии сайта B (`403 sdk key site mismatch`), а отозванные
ключи перестают работать немедленно (`401`).

### 5.4 Сквозная проверка (curl)

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

### 5.5 Актуальность обновлений

| Канал | Команда |
|---|---|
| OTA из панели (по умолчанию) | Админ-панель → Modules / Runtime install |
| Скрипт обновления | `VERSION=<v> INSTALL_ROOT=/opt/greenpng bash install/release/update_runtime_from_github.sh` |
| Вручную | SSH + откат на предыдущий runtime (`bin/releases/<v>` сохраняется) |

Каждое обновление тянет те же подписанные Release-ассеты из этого
репозитория.

## 6. Структура репозитория

| Каталог | Содержимое |
|---|---|
| `crates/` | Rust workspace — gr-service, gr-probe-core, gr-probe-plane, gr-probe-store, gr-ota, gr-admin, gr-runtime, … |
| `modules/` | Исходники подписанных горячо обновляемых модулей (identity / brain / analyze / ingest / edge / probe_assets) |
| `probe/` | Браузерный проб-зонд FE (загрузчик, паки, герметизированный ingest) |
| `panel/` | Админ-панель — Vue-исходники (`admin-ui/`) + собранный SPA (`admin-spa/`) |
| `sdk/` | SDK получения результатов (JS / Python / Go / PHP / Shell / Rust) |
| `spec/` | Загружаемые в рантайм спецификации (веса ботов, каталоги) |
| `install/` | Установщик + обновлятор + материалы systemd |
| `release/` | Скрипты упаковки (мультиархитектурный бандл, SLSA-аттестация) |
| `scripts/` | Контрактные проверки FE и вспомогательные скрипты |
| `docs/` | Это руководство на 12 языках |
| `VERSION` | Единственный источник истины для версии релиза |

## 7. Ссылки

- Releases и точка входа установки: этот репозиторий
- Официальный сайт: https://www.greenpng.cc (описание продукта, EN + 中文)
- Локали панели: English + 中文 (синхронизируются в `panel/admin-ui/src/i18n/`)
