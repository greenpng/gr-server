# greenpng — Product Guide (العربية)

تحقّق بشري من جهة المتصفح واستخبارات مضادة للبوتات: مسبار (probe) موقّع
يعمل داخل متصفحات الزوّار الحقيقيين، وخطّ استقبال (ingest) مختوم، ومستوى
تحليل يعيد أحكامًا لكل جلسة عبر حزم SDK بست لغات.

> هذا الدليل متوفر بـ 12 لغة — راجع
> [فهرس اللغات](../../README.md#documentation) في جذر المستودع.

## 1. ما الذي يقدّمه greenpng

تُقيَّم كل جلسة زائر على الجهاز وعلى الخادم معًا:

- **حكم بشري مقابل بوت** — `human | watch | bot` مع درجة ثقة لكل محور
  (ديناميكا الإدخال، اتساق حزمة الجهاز، أصالة البيئة، آثار الأتمتة، إعادة
  استخدام السجل).
- **هوية جهاز مستقرة** — معرّف جهاز واعٍ بالتصادمات يبقى مستقرًا عبر
  الجلسات دون الاعتماد على ملفات تعريف الارتباط (cookies) من جهات خارجية.
- **التقاط حقول العمل** — حقول الكوكيز المدرجة في قائمتك المسموحة
  (`user_id`, `plan_tier`, …) تُلتقط عند فتح الجلسة وتُرفق بالحكم. الأسماء
  الحساسة (password/token/…) تُحظر من جهة الخادم.
- **استخبارات IP** — تُقنَّع عناوين IP للزوّار خصوصيًا إلى /24 (IPv4) أو
  /48 (IPv6) عند الاستقبال؛ الإثراء الاختياري يربط الشبكة الفرعية بـ
  ASN/الدولة/المدينة عبر DB-IP (ملف MMDB مضمّن)، أو IPinfo، أو MaxMind،
  أو مُثرِخ HTTP مخصّص. الأسرار لا تغادر الخادم أبدًا.
- **واجهة استرجاع النتائج** — يستعلم التجّار عن الحكم باستخدام مفتاح SDK
  لكل موقع؛ العروض (projections: `public | sdk | diagnostic`) تتحكم في
  مستوى الكشف.

## 2. الميزات الرئيسية

| الميزة | ما تمنحك إياه |
|---|---|
| مسبار FE موقّع | حزم متصفح مقاومة للعبث (ed25519)، عناصر URL مُصدَرة وغير قابلة للتغيير `/dist/v/<ver>/g/<gen>/…` — آمنة لـ CDN، لا تسمم للذاكرة المؤقتة |
| استقبال مختوم | إرساليات الحزمة مختومة؛ يُرفض الإعادة والعبث في أقرب نقطة upstream |
| وحدات تحليل (OTA) | identity / brain / analyze / ingest / edge / probe_assets تُحدَّث ساخنًا كوحدات موقّعة دون توقف الخدمة |
| لوحة الإدارة | وحدة تحكم مستقلة على مسار عشوائي، مسؤول واحد بصيغة scrypt، سجل تدقيق، إدارة المواقع/الاستراتيجيات/التكامل/الاحتفاظ/DSAR، EN + 中文 |
| الخصوصية افتراضيًا | تقنيع IP في أبكر نقطة استقبال، قائمة كوكيز مسموحة، تصدير/حذف DSAR، تطهير الاحتفاظ |
| حزم SDK بست لغات | عملاء JS / Python / Go / PHP / Shell / Rust لـ `wait_for_result` |
| جاهزية تعدد العقد | وحدة LB مدمجة، نبض قلب للمجموعة (cluster heartbeat)، مرآة OTA |

## 3. البنية

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

**أشكال النشر**

- **gr-service** — عملية واحدة: وحدة تحكم الإدارة + واجهات التحكم + مستوى
  المسبار داخل الشجرة. المنفذ 28680 (الوحدة، مسار عشوائي) + 28765
  (loopback للمستوى).
- **nginx موقع العمل** — يقدّم `/gr.js` ويوكّل (اختياريًا) بادئة الـ API
  نفس المنشأ؛ رفع المتصفح يجب أن يستخدم نطاق GV المرتبط عبر Pingora TLS.
- **قواعد البيانات** — PostgreSQL لمخازن التحكم والمسبار (يوجد هيكل SQLite
  للمختبر).

## 4. التثبيت السريع

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh
bash install.sh --version <VERSION>            # see the VERSION file / Releases
# or with a dockerized data layer:
bash install.sh --version <VERSION> --with-docker --yes
```

يتحقق المثبّت من توقيعات sha256 + ELF + ed25519 للوحدات، ويثبّت تحت
`/opt/greenpng`، ويكتب `.env`، ويُجهّز وينشّط الوحدات الست الموقّعة،
ويفعّل وحدة systemd، ويقوم بالتحقق عبر `/v1/health`.

أول تسجيل دخول: تُكتب بيانات الاعتماد لمرة واحدة في
`/opt/greenpng/data/admin/admin_bootstrap_once.txt` — مسار الوحدة العشوائي
(`/c-<hex>/`) يُسجَّل هناك. لا وجود لبادئة `/admin` أو `/console/`،
وتسجيل الدخول بكلمة المرور هو المدخل الوحيد للوحة.

## 5. درس الاستخدام

### 5.1 إنشاء موقع

لوحة **Sites → Create site**:

- `site_id` — معرّف المستأجر الخاص بك، يُستخدم في التضمين
- `root_domains` — أسماء مضيفي www (CORS + ربط اسم المضيف)
- `cookie_fields` — قائمة الكوكيز المسموحة، مثلًا
  `["user_id", "plan_tier", "cart_id", "utm_source"]`

يتدفّق صف الموقع من `control.sites → public.sites` (مستوى المسبار) عند
الحفظ، ويقوم gr-service بتعويض (backfill) المواقع الموجودة مسبقًا عند
الإقلاع.

### 5.2 نشر المسبار (ثلاثة أنماط)

**A. Nginx من الطرف الأول (موصى به)** — يوكّل مضيف الموقع (vhost) محمّل
الإقلاع وبادئة الـ API نفس المنشأ:

```nginx
location = /gr.js      { proxy_pass https://pv.example.com/gr.js; proxy_set_header Host pv.example.com; }
location /gr/dist/v/   { proxy_pass https://pv.example.com/gr/dist/v/; proxy_set_header Host pv.example.com; }
location /gr/v1/       { proxy_pass https://gv.example.com/; proxy_set_header Cookie $http_cookie;
                         proxy_set_header X-Forwarded-For $remote_addr; }
```

```html
<script src="/gr.js" data-site-id="mysite" data-endpoint="/gr" data-inject-path="nginx" defer></script>
```

**B. عامل Cloudflare** — يحقن العامل سكربت الإقلاع ويوكّل `/gr` داخل
المنشأ نفسه. تجنّب وضع "Under Attack" على `/gr` (صفحات التحدي تكسر
المسبار).

**C. تضمين عبر CDN / سكربت الموقع** — حمّل JS الإقلاع مباشرة من PV وأشِر
بـ `data-endpoint` إلى GV.

في كل نمط، يحل المحمّل الحزم عبر bootstrap الـ SDK ولا يجلب سوى عناصر
URL مُصدَرة غير قابلة للتغيير.

### 5.3 استقبال النتائج (SDK بست لغات)

أنشئ **مفتاح خلفية (backend key)** لموقعك في اللوحة (صفحة SDK). لا
يرحّل الـ SDK أي مسبارات؛ بل يستعلم عن النتائج فقط:

```
GET {gv_base}/v1/session/{session_id}/result?projection=sdk
X-Gr-Sdk-Key: <site backend key>
```

| اللغة | نقطة الدخول |
|---|---|
| JS | `sdk/src/index.js` — `new GrResultClient({baseUrl, apiKey}).waitForResult(...)` |
| Python | `sdk/python/gr_results.py` — `GrResultClient(base_url, api_key).wait_for_result(...)` |
| Go | `sdk/go/gr_results.go` — `gr.New(baseURL, key).WaitForResult(...)` |
| PHP | `sdk/php/GrResultsClient.php` — `(new GrResultClient(...))->waitForResult(...)` |
| Shell | `sdk/shell/gr_sdk.sh` — `gr_wait_for_result <session> sdk 8000` |
| Rust | `sdk/rust/` — `Client::new(base_url, key).wait_for_result(...)` |

مفاتيح المواقع محصورة النطاق: مفتاح سُكّ لموقع A لا يمكنه قراءة جلسات
الموقع B (`403 sdk key site mismatch`)، والمفاتيح الملغاة تتوقف عن
العمل فورًا (`401`).

### 5.4 اختبار دخان شامل (curl)

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

### 5.5 الحفاظ على التحديث

| القناة | الأمر |
|---|---|
| OTA من اللوحة (الافتراضي) | Admin panel → Modules / Runtime install |
| سكربت التحديث | `VERSION=<v> INSTALL_ROOT=/opt/greenpng bash install/release/update_runtime_from_github.sh` |
| يدوي | SSH + تراجع إلى نسخة سابقة (`bin/releases/<v>` محفوظة) |

كل تحديث يجلب نفس أصول الـ Release الموقّعة من هذا المستودع.

## 6. تخطيط المستودع

| الدليل | المحتويات |
|---|---|
| `crates/` | مساحة عمل Rust — gr-service، gr-probe-core، gr-probe-plane، gr-probe-store، gr-ota، gr-admin، gr-runtime، … |
| `modules/` | مصادر وحدات التحديث الساخن الموقّعة (identity / brain / analyze / ingest / edge / probe_assets) |
| `probe/` | مسبار المتصفح FE (المحمّل، الحزم، الاستقبال المختوم) |
| `panel/` | لوحة الإدارة — مصادر Vue (`admin-ui/`) + SPA مبني (`admin-spa/`) |
| `sdk/` | حزم SDK لاسترجاع النتائج (JS / Python / Go / PHP / Shell / Rust) |
| `spec/` | المواصفات المحمّلة في التشغيل (أوزان البوتات، الفهارس) |
| `install/` | المثبّت + المحدّث + ملفات systemd |
| `release/` | سكربتات التغليف (حزمة متعددة البنى، شهادة SLSA) |
| `scripts/` | فحوص عقود FE وأدوات مساعدة |
| `docs/` | هذا الدليل بـ 12 لغة |
| `VERSION` | المصدر الوحيد للحقيقة لنسخة الإصدار |

## 7. الروابط

- الإصدارات ونقطة دخول التثبيت: هذا المستودع
- الموقع الرسمي: https://www.greenpng.cc (تعريف بالمنتج، EN + 中文)
- لغات اللوحة: English + 中文 (تُزامَن في `panel/admin-ui/src/i18n/`)
