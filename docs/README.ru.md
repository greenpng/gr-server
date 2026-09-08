# greenpng — Руководство проекта (Русский)

## 1. Что это за проект

greenpng (GR) определяет, является ли сессия реальным человеком, проверяя
**браузер настоящего посетителя**, а не только анализируя трафик.

Конвейер, от начала до конца:

1. **Зондирование (в браузере)** — подписанный FE-загрузчик выполняется на
   ваших страницах и собирает свидетельства из **нескольких источников**
   (динамика ввода, стек устройства, подлинность окружения, следы
   автоматизации) **несколькими поэтапными пакетами** за сессию.
2. **Отправка** — браузер передаёт каждый пакет на сервер через
   герметизированный ingest-конвейер; повторные или подделанные отправки
   отбрасываются до попадания в хранилище.
3. **Анализ (на сервере)** — аналитическая плоскость оценивает каждую сессию
   и выдаёт вердикт по сессии (`human | watch | bot`) с доверительной
   оценкой по каждой оси и стабильным, устойчивым к коллизиям идентификатором
   устройства.
4. **Возврат** — ваш бэкенд получает результат через API результатов
   (SDK на шести языках; проекции `public | sdk | diagnostic` управляют тем,
   что видит каждый вызывающий).

На стороне сервера greenpng работает как один host-бинарь со встроенным
(in-tree) probe-плоскостью по умолчанию и поддерживает **многоузловое
развёртывание с балансировкой нагрузки**: LB-модуль распределяет probe-трафик
между узлами перед общим слоем данных (PostgreSQL + Redis), поэтому сбор и
анализ масштабируются горизонтально.

## 2. Структура репозитория и архитектура

| Каталог | Содержимое |
|---|---|
| `crates/` | Rust workspace — `gr-service` (control plane + админ-консоль + встроенная probe-плоскость), `gr-probe-core`, `gr-probe-plane`, `gr-probe-store`, `gr-ota`, `gr-admin`, `gr-runtime`, … |
| `modules/` | Исходники подписанных hot-update модулей (identity / brain / analyze / ingest / edge / probe_assets) |
| `probe/` | Браузерный probe FE (загрузчик, цепочка pack-ов, герметизированный ingest-клиент) |
| `panel/` | Админ-панель — Vue-исходники (`admin-ui/`) + собранный SPA (`admin-spa/`), EN + 中文 |
| `sdk/` | SDK интеграции бэкенда на шести языках (только получение результатов) |
| `spec/` | Спецификации протокола/скоринга, загружаемые в рантайме |
| `fixtures/` | Данные контрактных тестов |
| `scripts/` | Сборочные скрипты и FE-инструменты (`scripts/fe/checks/`) |
| `vendor/` | Vendored-исходники зависимостей (pingora) |
| `install/` | Установщик, compose слоя данных, скрипты обновления |
| `release/` | Скрипты упаковки (multi-arch сборка, SBOM, подпись модулей) |
| `docs/` | Это руководство, по одному файлу на язык |

```
        visitor browser
              │  <script src="/gr.js">  (pinned, no-store)
              ▼
   FE loader ──► /v1/sdk/bootstrap ──► versioned pack manifest
              │        asset_base /dist/v/<fe>/g/<gen>/
              ▼
   pack collectors (input · device · environment, multi-batch)
              │  sealed submit (direct to the bound gv domain, TLS)
              ▼
 ┌────────────┴─────────────┐   ┌──────────────────────────────┐
 │ gr-probe-plane (Pingora)  │   │ gr-service (control plane)    │
 │  gateway · ingest · ops   │◄──┤  admin console · site config  │
 └────────────┬─────────────┘   │  module registry (OTA, signed)│
              ▼                 └──────────────┬───────────────┘
   PostgreSQL (+ Redis in multi-node)          │
              ▼                                │
   GET /v1/session/{id}/result ──► merchant SDK (six languages)

   multi-node: LB module spreads probe traffic across gr-service nodes
   in front of the shared data layer
```

Ключевые компоненты: `gr-service` — это control plane (админ-консоль со
случайным путём, управление сайтами/конфигурацией, реестр подписанных
OTA-модулей), он же размещает probe-плоскость in-tree;
`gr-probe-plane` — это Pingora-шлюз с герметизированным ingest и API
сессий/результатов; браузерный FE разрешает каждый ассет в
версионно-неизменяемый URL, поэтому кэши никогда не отдадут устаревший
probe между релизами; PostgreSQL (плюс Redis в многоузловом режиме)
хранит сессии, пакеты и результаты анализа.

## 3. Установка и использование

### 3.1 Установка

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh | bash
# or with an explicit version / arch:
bash install/install.sh --version <VERSION> --arch x86_64 --yes
# with a dockerized data layer (PostgreSQL/Redis only; the server stays a host binary):
bash install/install.sh --version <VERSION> --with-docker --yes
```

Установщик проверяет подписи sha256 + ELF + ed25519 для модулей, ставит
всё под `/opt/greenpng`, записывает `.env` и systemd-юнит, размещает и
активирует шесть подписанных модулей и ориентируется на
`/v1/health` control/plane-плоскостей. Учётные данные первого входа и
случайный путь консоли записываются в
`/opt/greenpng/data/admin/admin_bootstrap_once.txt`.

### 3.2 Обновление

| Приоритет | Канал | Для |
|---|---|---|
| P0 | OTA из панели (set-release-url → install / install-fe / install-runtime) | по умолчанию |
| P1 | `install/release/update_runtime_from_github.sh`, `update_module_from_github.sh` | без панели / свободные узлы |
| P2 | Ручное обновление по SSH | мёртвый процесс / первая установка |

Все обновления тянут подписанные Release-ассеты того же тега из этого
репозитория. Docker — это рантайм-контейнер, а не канал обновлений.

### 3.3 Использование

1. **Создайте сайт** в админ-панели: id сайта, корневые домены и
   allow-список cookie для бизнес-полей, которые вы хотите прикреплять
   к каждому вердикту (чувствительные имена вроде `password`/`token`
   блокируются на стороне сервера).
2. **Разверните probe** — три режима:
   - *Nginx от первой стороны (рекомендуется)*: проксируйте `/gr.js` +
     `/gr/dist/v/` на pv-домен и `/gr/v1/` на gv-домен (передача Cookie),
     внедрите `<script src="/gr.js" data-site-id="…" data-endpoint="/gr"
     data-inject-path="nginx" defer></script>` в HTML.
   - *Cloudflare worker*: внедрите тот же тег и проксируйте `/gr` in-origin.
   - *Встраивание в приложение*: загружайте загрузчик напрямую из pv/CDN,
     указав в `data-endpoint` адрес gv/pv.
   Загрузки из браузера всегда идут **напрямую на привязанный gv-домен
   через TLS**.
3. **Получайте результаты** — создайте SDK-ключ сайта в панели, затем
   опрашивайте:

```bash
KEY="grsk_..." SID="cycle_..." BASE="https://gv.example.com"
curl -sS -H "X-Gr-Sdk-Key: $KEY" "$BASE/v1/session/$SID/result"
```

4. **Проверка (curl)**:

```bash
# open a session carrying allowlisted cookies
curl -sS -X POST "$BASE/v1/session/open" -H "X-Gr-Sdk-Key: $KEY" \
     -H 'Cookie: user_id=u9; plan_tier=pro' -d '{"site_id":"mysite"}'
# → {"session_id":"cycle_…", …}
# then check the result (after real FE batches or simulated ones):
curl -sS -H "X-Gr-Sdk-Key: $KEY" "$BASE/v1/session/$SID/result"
```
