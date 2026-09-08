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
   - *Nginx من الطرف الأول (موصى به)*: مرّر `/gr.js` + `/gr/dist/v/` إلى
     نطاق pv و`/gr/v1/` إلى نطاق gv (مع تمرير ملفات تعريف الارتباط)،
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
