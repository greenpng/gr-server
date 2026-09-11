# greenpng — دليل المشروع (العربية)

## 1. ما هو هذا المشروع

يتحقق greenpng (GR) مما إذا كانت الجلسة تنتمي إلى إنسان حقيقي عبر فحص
**متصفح الزائر الفعلي**، لا عبر تفتيش حركة المرور وحدها.

خط المعالجة، من البداية إلى النهاية:

1. **الاستقصاء (في المتصفح)** — محمّل FE موقّع يعمل على صفحاتك ويجمع
   الأدلة من **مصادر متعددة** (ديناميكيات الإدخال، حزمة الجهاز، أصالة
   البيئة، آثار الأتمتة) على **دفعات متتالية متعددة** في كل جلسة.
2. **الرفع** — يرسل المتصفح كل دفعة إلى الخادم عبر مسار استيعاب مختوم؛
   وتُرفض الطلبات المعاد تشغيلها أو المتلاعب بها قبل وصولها إلى التخزين.
3. **التحليل (على الخادم)** — يمنح مستوى التحليل كل جلسة حكمًا خاصًا بها
   (`human | watch | bot`) مع درجة ثقة لكل محور وهوية جهاز مستقرة
   واعية بالتصادمات.
4. **الإرجاع** — يسترجيع نظامك الخلفي النتيجة عبر واجهة برمجة النتائج
   (حزم SDK بست لغات؛ وتتحكم إسقاطات `public | sdk | diagnostic` في
   ما يراه كل مستدعٍ).

على جانب الخادم، يعمل greenpng كملف ثنائي واحد على المضيف مع مستوى
استقصاء مدمج في الشجرة افتراضيًا، ويدعم **النشر متعدد العقد مع موازنة
الحمل**: يوزّع وحدة LB حركة الاستقصاء على العقد أمام طبقة بيانات مشتركة
(PostgreSQL + Redis)، فيتوسع الجمع والتحليل أفقيًا.

## 2. تخطيط المستودع والبنية

| الدليل | المحتويات |
|---|---|
| `crates/` | مساحة عمل Rust — `gr-service` (مستوى التحكم + لوحة الإدارة + مستوى الاستقصاء المدمج)، `gr-probe-core`، `gr-probe-plane`، `gr-probe-store`، `gr-ota`، `gr-admin`، `gr-runtime`، … |
| `modules/` | مصادر الوحدات الموقّعة للتحديث الساخن (identity / brain / analyze / ingest / edge / probe_assets) |
| `probe/` | استقصاء المتصفح FE (المحمّل، سلسلة الحزم، عميل الاستيعاب المختوم) |
| `panel/` | لوحة الإدارة — مصادر Vue (`admin-ui/`) + SPA مبني (`admin-spa/`)، EN + 中文 |
| `sdk/` | حزم تكامل الخلفية بست لغات (استرجاع النتائج فقط) |
| `spec/` | مواصفات السلك/التنقيط المحمّلة وقت التشغيل |
| `fixtures/` | بيانات اختبارات العقود |
| `scripts/` | سكربتات البناء وأدوات FE (`scripts/fe/checks/`) |
| `vendor/` | مصادر تبعيات مدرجة (pingora) |
| `install/` | المثبّت، وطبقة البيانات compose، وسكربتات الترقية |
| `release/` | سكربتات التغليف (بناء متعدد المعماريات، SBOM، توقيع الوحدات) |
| `docs/` | هذا الدليل، ملف واحد لكل لغة |

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

المكونات الرئيسية: `gr-service` هو مستوى التحكم (لوحة إدارة بمسار
عشوائي، وإدارة المواقع/الإعدادات، وسجل وحدات OTA موقّع) ويستضيف مستوى
الاستقصاء مدمجًا في الشجرة؛ و`gr-probe-plane` هو بوابة Pingora مع
استيعاب مختوم وواجهات الجلسة/النتائج؛ ويحلّ متصفح FE كل أصل إلى عنوان
URL ثابت الإصدار فلا يمكن لذاكرات التخزين المؤقت أبدًا تقديم استقصاء
متقادم بين الإصدارات؛ ويخزّن PostgreSQL (مع Redis في وضع تعدد العقد)
الجلسات والدفعات ونتائج التحليل.

## 3. التثبيت والاستخدام

### 3.1 التثبيت

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh | bash
# or with an explicit version / arch:
bash install/install.sh --version <VERSION> --arch x86_64 --yes
# with a dockerized data layer (PostgreSQL/Redis only; the server stays a host binary):
bash install/install.sh --version <VERSION> --with-docker --yes
```

يتحقق المثبّت من توقيعات sha256 + ELF + ed25519 للوحدات، ويثبّت تحت
`/opt/greenpng`، ويكتب `.env` ووحدة systemd، وينظّم الوحدات الست الموقّعة
وينشّطها، ويشترط نجاح `/v1/health` لمستوَيي التحكم والاستقصاء. وتُكتب
بيانات اعتماد أول تسجيل دخول ومسار لوحة الإدارة العشوائي إلى
`/opt/greenpng/data/admin/admin_bootstrap_once.txt`.

### 3.2 التحديث

| الأولوية | القناة | لمن |
|---|---|---|
| P0 | لوحة OTA (set-release-url ← install / install-fe / install-runtime) | الافتراضي |
| P1 | `install/release/update_runtime_from_github.sh`، `update_module_from_github.sh` | بلا لوحة / عقد حرة |
| P2 | SSH يدوي | عملية متعطلة / تثبيت أولي |

تجذب كل التحديثات أصول Release الموقّعة لنفس الوسوم من هذا المستودع.
إن Docker هو حاوية تشغيل، وليس قناة تحديث.

### 3.3 الاستخدام

1. **أنشئ موقعًا** في لوحة الإدارة: معرّف الموقع، والنطاقات الجذرية، وقائمة
   السماح لملفات تعريف الارتباط لحقول العمل التي تريد إرفاقها بكل حكم
   (الأسماء الحساسة مثل `password`/`token` محجوبة على جانب الخادم).
2. **انشر الاستقصاء** — ثلاثة أوضاع:
   - *Nginx من الطرف الأول (موصى به)*: على نطاق pv مرّر `/gr.js` + `/gr/dist/v/` +
     `/gr/v1/` إلى مستوى التحقيق (مع تمرير ملفات تعريف الارتباط)،
     وأدرج `<script src="/gr.js" data-site-id="…" data-endpoint="/gr"
     data-inject-path="nginx" defer></script>` في HTML.
   - *عامل Cloudflare*: أدرج الوسم نفسه ومرّر `/gr` داخل الأصل.
   - *تضمين في التطبيق*: حمّل المحمّل مباشرة من pv/CDN مع إشارة
     `data-endpoint` إلى gv/pv.
   ترفع المتصفحات دائمًا **مباشرة إلى نطاق gv المرتبط عبر TLS**.
3. **استقبل النتائج** — أنشئ مفتاح SDK للموقع في اللوحة، ثم استطلع:

```bash
KEY="grsk_..." SID="cycle_..." BASE="https://gv.example.com"
curl -sS -H "X-Gr-Sdk-Key: $KEY" "$BASE/v1/session/$SID/result"
```

4. **اختبار سريع** (curl):

```bash
# open a session carrying allowlisted cookies
curl -sS -X POST "$BASE/v1/session/open" -H "X-Gr-Sdk-Key: $KEY" \
     -H 'Cookie: user_id=u9; plan_tier=pro' -d '{"site_id":"mysite"}'
# → {"session_id":"cycle_…", …}
# then check the result (after real FE batches or simulated ones):
curl -sS -H "X-Gr-Sdk-Key: $KEY" "$BASE/v1/session/$SID/result"
```

---

## 4. معلمات لوحة الإدارة

تُعدَّل في صفحة **Config** بلوحة الإدارة (حفظ ← نشر): العقدة الناشرة
تُطبِّق فورًا؛ عقد المجموعة خلال ≤30 ثانية دون إعادة تشغيل.

**حدود المعدل** (سياسة v1.0.14: الإجماليات لكل موقع معطلة افتراضيًا؛
مسار القياس عن بُعد مقيَّد لكل IP فردي؛ استجابات 429 تسمّي الطبقة المُفعِّلة):

| المعلمة | الافتراضي | المعنى |
|---|:---:|---|
| `rate_limit_open_per_min` | 0 = بلا حد | فتح الجلسات / موقع / دقيقة |
| `rate_limit_ingest_per_min` | 0 = بلا حد | رفع الدفعات / موقع / دقيقة |
| `rate_limit_analyze_per_min` | 0 = بلا حد | التحليلات المباشرة / موقع / دقيقة |
| `rate_limit_complete_per_min` | 0 = بلا حد | إيصالات complete / موقع / دقيقة |
| `rate_limit_result_per_min` | 0 = بلا حد | قراءات النتائج / موقع / دقيقة |
| `rate_limit_client_event_per_min` | 0 = بلا حد | قياس FE / موقع / دقيقة (إجمالي) |
| `rate_limit_client_event_per_ip_per_min` | 100 | قياس FE **لكل IP فردي** / دقيقة — التجاوز يقيّد هذا الـIP فقط؛ 0 = معطّل |

**خلف CDN** تعمل طبقة "لكل IP" على العنوان الذي يراه الخادم. أضف نطاقات
CIDR الخاصة بالوكيل إلى `GR_TRUSTED_PROXIES` في `/opt/greenpng/.env`
واستعد IP الزائر الحقيقي عند الوكيل الأمامي (مثال nginx):

```nginx
set_real_ip_from 173.245.48.0/20;  # Cloudflare IPv4
set_real_ip_from 2400:cb00::/32;   # Cloudflare IPv6
real_ip_header CF-Connecting-IP;
```

**الطبقات الساخنة/الباردة** (أسماء حقيقية): `cold_ttl_ms` (604800000 =
7 أيام)، `cold_promote_window_ms` (864000000 = 24 ساعة)،
`cold_purge_interval_ms` (300000 = 5 دقائق)؛ تُضبط مدة الاحتفاظ لكل موقع
في صفحة **Data Retention** باللوحة وتُحذف على دفعات محدودة.

## 5. السجلات والذاكرة

- سجلات الخدمة: `journalctl -u greenpng.service`؛ يُحتفظ بالقياس التشغيلي
  (`ops_client_events`) لمدة `ops_retention_days` (14) يومًا.
- **ملاحظة الذاكرة (منذ v1.0.14)**: على الأجهزة متعددة الأنوية طويلة
  التشغيل قد يحتفظ glibc بما يصل إلى 8 arenas لكل نواة (~64 ميغابايت
  لكل واحدة)، فيرتفع RSS تدريجيًا مع التزامن. لذلك يضبط المُثبِّت
  `MALLOC_ARENA_MAX=4` في `/opt/greenpng/.env`؛ فيبقى RSS مستقرًا تحت الحمل.

## 6. مشاريع المصادر المفتوحة والمراجع

- [Cloudflare Pingora](https://github.com/cloudflare/pingora) (Apache-2.0) — بوابة الحافة، إنهاء TLS، الاستلام المختوم (ميزة openssl).
- [Tokio](https://github.com/tokio-rs/tokio) و[Axum](https://github.com/tokio-rs/axum) (MIT) — بيئة التنفيذ غير المتزامن وإطار REST لمستوى التحكم.
- [OpenSSL](https://www.openssl.org/) (Apache-2.0) — باكِند TLS للحافة ولوحة الإدارة.
- [ed25519-dalek](https://github.com/dalek-cryptography/curve25519-dalek) (BSD-3) — التوقيعات لبيانات الإصدار وأصول التحقيقات وOTA.
- [PostgreSQL](https://www.postgresql.org/) و[Redis](https://redis.io/) — تخزين L3 وحالة تعدد العقد.
- [flate2 / zlib](https://github.com/rust-compress/flate2) (MIT) — ضغط الحمولات (zstd موجود فقط داخل pingora المُضمَّن).
- [Element Plus](https://element-plus.org/) و[Vue 3](https://vuejs.org/) (MIT) — واجهة لوحة الإدارة.
- مراجع بحثية: [CreepJS](https://github.com/abrahamjuliot/creepjs) (إلهام B1/B12)، [FingerprintJS](https://github.com/fingerprintjs/fingerprintjs)، [BotD](https://github.com/fingerprintjs/botd).
