# greenpng — Product Guide (हिन्दी)

ब्राउज़र-साइड मानव-सत्यापन और एंटी-बॉट इंटेलिजेंस: एक हस्ताक्षरित प्रोब
जो वास्तविक विज़िटरों के ब्राउज़रों में चलता है, एक सीलबद्ध इनजेस्ट पाइपलाइन,
और एक विश्लेषण प्लेन जो छह भाषाओं की SDKs के माध्यम से प्रति-सेशन निर्णय
लौटाता है।

> यह गाइड 12 भाषाओं में उपलब्ध है — रिपॉज़िटरी रूट पर
> [भाषा सूचकांक](../../README.md#documentation) देखें।

## 1. greenpng क्या करता है

प्रत्येक विज़िटर सेशन को डिवाइस-पर और सर्वर-साइड पर स्कोर किया जाता है:

- **वास्तविक बनाम बॉट निर्णय** — `human | watch | bot`, प्रत्येक-अक्ष के
  भरोसे (confidence) के साथ (इनपुट डायनामिक्स, डिवाइस स्टैक की संगति,
  परिवेश की प्रामाणिकता, ऑटोमेशन के निशान, इतिहास का पुन:उपयोग)।
- **स्थिर डिवाइस पहचान** — एक टकराव-जागरूक (collision-aware) डिवाइस ID
  जो तृतीय-पक्ष कुकीज़ पर निर्भर किए बिना सेशनों के बीच स्थिर रहती है।
- **बिज़नेस फ़ील्ड कैप्चर** — आपकी अनुमत-सूची (allow-list) वाली कुकी फ़ील्ड
  (`user_id`, `plan_tier`, …) सेशन खुलने पर कैप्चर होती हैं और निर्णय के
  साथ जुड़ जाती हैं। संवेदनशील नाम (password/token/…) सर्वर-साइड पर
  अवरुद्ध होते हैं।
- **IP इंटेलिजेंस** — विज़िटर IP को इनजेस्टन पर /24 (IPv4) या /48 (IPv6) तक
  प्राइवेसी-मास्क किया जाता है; वैकल्पिक एनरिचमेंट सबनेट को DB-IP (बंडल
  MMDB), IPinfo, MaxMind, या किसी कस्टम HTTP एनरिचर के माध्यम से
  ASN/देश/शहर पर मैप करता है। सीक्रेट कभी भी सर्वर से बाहर नहीं जाते।
- **परिणाम प्राप्ति API** — व्यापारी (merchants) पर-साइट SDK key से निर्णय
  को पोल करते हैं; प्रोजेक्शन (`public | sdk | diagnostic`) एक्सपोज़र
  को नियंत्रित करते हैं।

## 2. मुख्य विशेषताएँ

| विशेषता | यह आपको क्या देती है |
|---|---|
| हस्ताक्षरित FE प्रोब | टैम्पर-सबूत ब्राउज़र पैक (ed25519), वर्ज़नयुक्त अपरिवर्तनीय ऐसेट URL `/dist/v/<ver>/g/<gen>/…` — CDN-सुरक्षित, कोई कैश पॉइज़निंग नहीं |
| सीलबद्ध इनजेस्ट | पैक सबमिशन सीलबद्ध होते हैं; रीप्ले/टैम्पर अपस्ट्रीम पर अस्वीकृत |
| विश्लेषण मॉड्यूल (OTA) | identity / brain / analyze / ingest / edge / probe_assets — हस्ताक्षरित मॉड्यूल के रूप में, बिना डाउनटाइम हॉट-अपडेट |
| एडमिन पैनल | यादृच्छिक पथ पर स्टैंडअलोन कंसोल, एकल scrypt एडमिन, ऑडिट लॉग, साइट/रणनीति/इंटीग्रेशन/रिटेंशन/DSAR प्रबंधन, EN + 中文 |
| डिफ़ॉल्ट रूप से प्राइवेसी | सबसे शुरुआती इनजेस्टन बिंदु पर IP मास्किंग, कुकी अनुमत-सूची, DSAR एक्सपोर्ट/मिटाना, रिटेंशन पर्ज |
| छह-भाषा SDKs | `wait_for_result` के लिए JS / Python / Go / PHP / Shell / Rust क्लाइंट |
| मल्टी-नोड तैयार | बिल्ट-इन LB मॉड्यूल, क्लस्टर हार्टबीट, OTA मिरर |

## 3. आर्किटेक्चर

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

**डिप्लॉयमेंट आकार**

- **gr-service** — एक प्रोसेस: एडमिन कंसोल + कंट्रोल APIs + इन-ट्री प्रोब
  प्लेन। पोर्ट 28680 (कंसोल, यादृच्छिक पथ) + 28765 (प्लेन लूपबैक)।
- **बिज़नेस साइट nginx** — `/gr.js` सर्व करता है और (वैकल्पिक रूप से)
  सेम-ऑरिजिन API प्रीफ़िक्स को प्रॉक्सी करता है; ब्राउज़र अपलोड को बंधी
  GV डोमेन के माध्यम से Pingora TLS का उपयोग करना होगा।
- **DBs** — कंट्रोल + प्रोब स्टोर के लिए PostgreSQL (लैब के लिए SQLite
  स्केलेटन मौजूद है)।

## 4. त्वरित इंस्टॉल

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh
bash install.sh --version <VERSION>            # see the VERSION file / Releases
# or with a dockerized data layer:
bash install.sh --version <VERSION> --with-docker --yes
```

इंस्टॉलर sha256 + ELF + ed25519 मॉड्यूल हस्ताक्षरों को सत्यापित करता है,
`/opt/greenpng` के अंतर्गत इंस्टॉल करता है, `.env` लिखता है, छह हस्ताक्षरित
मॉड्यूल को स्टेज और सक्रिय करता है, systemd यूनिट सक्षम करता है, और
`/v1/health` पर गेट लगाता है।

पहला लॉगिन: वन-टाइम क्रेडेंशियल
`/opt/greenpng/data/admin/admin_bootstrap_once.txt` में लिखे जाते हैं —
यादृच्छिक कंसोल पथ (`/c-<hex>/`) वहीं अंकित होता है। कोई `/admin` या
`/console/` प्रीफ़िक्स नहीं है, और पैनल में प्रवेश केवल पासवर्ड लॉगिन से
होता है।

## 5. उपयोग ट्यूटोरियल

### 5.1 साइट बनाएँ

पैनल **Sites → Create site**:

- `site_id` — आपका टेनेंट id, एम्बेड में उपयोग होता है
- `root_domains` — www होस्टनेम (CORS + होस्टनेम बाइंडिंग)
- `cookie_fields` — कुकी अनुमत-सूची, उदाहरण के लिए
  `["user_id", "plan_tier", "cart_id", "utm_source"]`

सेव करने पर साइट पंक्ति `control.sites → public.sites` (प्रोब प्लेन) में
प्रवाहित होती है, और gr-service स्टार्टअप पर पूर्व-स्थित साइटों को
बैकफ़िल करता है।

### 5.2 प्रोब परिनियोजित करें (तीन मोड)

**A. Nginx फ़र्स्ट-पार्टी (अनुशंसित)** — आपकी साइट का vhost बूट लोडर और
सेम-ऑरिजिन API प्रीफ़िक्स को प्रॉक्सी करता है:

```nginx
location = /gr.js      { proxy_pass https://pv.example.com/gr.js; proxy_set_header Host pv.example.com; }
location /gr/dist/v/   { proxy_pass https://pv.example.com/gr/dist/v/; proxy_set_header Host pv.example.com; }
location /gr/v1/       { proxy_pass https://gv.example.com/; proxy_set_header Cookie $http_cookie;
                         proxy_set_header X-Forwarded-For $remote_addr; }
```

```html
<script src="/gr.js" data-site-id="mysite" data-endpoint="/gr" data-inject-path="nginx" defer></script>
```

**B. Cloudflare worker** — वर्कर बूट स्क्रिप्ट इंजेक्ट करता है और `/gr` को
इन-ऑरिजिन प्रॉक्सी करता है। `/gr` पर "Under Attack" मोड से बचें (चैलेंज
पेज प्रोब को तोड़ देते हैं)।

**C. साइट स्क्रिप्टिंग / CDN एम्बेड** — बूट JS को सीधे PV से लोड करें और
`data-endpoint` को GV की ओर इशारा करें।

हर मोड में लोडर पैक को SDK bootstrap के माध्यम से हल करता है और केवल
वर्ज़नयुक्त अपरिवर्तनीय URL ही फ़ेच करता है।

### 5.3 परिणाम प्राप्त करें (छह-भाषा SDK)

पैनल (SDK पेज) में अपनी साइट के लिए एक **backend key** बनाएँ। SDK कभी
भी प्रोब रिले नहीं करता; वह केवल परिणामों को क्वेरी करता है:

```
GET {gv_base}/v1/session/{session_id}/result?projection=sdk
X-Gr-Sdk-Key: <site backend key>
```

| भाषा | एंट्री पॉइंट |
|---|---|
| JS | `sdk/src/index.js` — `new GrResultClient({baseUrl, apiKey}).waitForResult(...)` |
| Python | `sdk/python/gr_results.py` — `GrResultClient(base_url, api_key).wait_for_result(...)` |
| Go | `sdk/go/gr_results.go` — `gr.New(baseURL, key).WaitForResult(...)` |
| PHP | `sdk/php/GrResultsClient.php` — `(new GrResultClient(...))->waitForResult(...)` |
| Shell | `sdk/shell/gr_sdk.sh` — `gr_wait_for_result <session> sdk 8000` |
| Rust | `sdk/rust/` — `Client::new(base_url, key).wait_for_result(...)` |

साइट keys स्कोप्ड होती हैं: साइट A के लिए बनाई गई key साइट B के सेशन
नहीं पढ़ सकती (`403 sdk key site mismatch`), और रद्द (revoke) की गई keys
तुरंत काम करना बंद कर देती हैं (`401`)।

### 5.4 एंड-टू-एंड स्मोक (curl)

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

### 5.5 अपडेट रखें

| चैनल | कमांड |
|---|---|
| पैनल OTA (डिफ़ॉल्ट) | एडमिन पैनल → Modules / Runtime install |
| अपडेटर स्क्रिप्ट | `VERSION=<v> INSTALL_ROOT=/opt/greenpng bash install/release/update_runtime_from_github.sh` |
| मैनुअल | SSH + पिछले रनटाइम रोलबैक (`bin/releases/<v>` सुरक्षित रहता है) |

हर अपडेट इस रिपॉज़िटरी से उन्हीं हस्ताक्षरित Release ऐसेट को खींचता है।

## 6. रिपॉज़िटरी लेआउट

| निर्देशिका | सामग्री |
|---|---|
| `crates/` | Rust वर्कस्पेस — gr-service, gr-probe-core, gr-probe-plane, gr-probe-store, gr-ota, gr-admin, gr-runtime, … |
| `modules/` | हस्ताक्षरित हॉट-अपडेट मॉड्यूल स्रोत (identity / brain / analyze / ingest / edge / probe_assets) |
| `probe/` | ब्राउज़र प्रोब FE (लोडर, पैक, सीलबद्ध इनजेस्ट) |
| `panel/` | एडमिन पैनल — Vue स्रोत (`admin-ui/`) + निर्मित SPA (`admin-spa/`) |
| `sdk/` | परिणाम-प्राप्ति SDKs (JS / Python / Go / PHP / Shell / Rust) |
| `spec/` | रनटाइम-लोडेड स्पेक्स (बॉट वेट, कैटलॉग) |
| `install/` | इंस्टॉलर + अपडेटर + systemd सामग्री |
| `release/` | पैकेजिंग स्क्रिप्ट (मल्टी-आर्च बंडल, SLSA एटेस्टेशन) |
| `scripts/` | FE कॉन्ट्रैक्ट जाँच और हेल्पर |
| `docs/` | यह गाइड 12 भाषाओं में |
| `VERSION` | रिलीज़ संस्करण का एकमात्र सत्य स्रोत |

## 7. लिंक

- रिलीज़ और इंस्टॉल एंट्री पॉइंट: यह रिपॉज़िटरी
- आधिकारिक साइट: https://www.greenpng.cc (उत्पाद परिचय, EN + 中文)
- पैनल लोकेल: English + 中文 (`panel/admin-ui/src/i18n/` में सिंक रखा जाता है)
