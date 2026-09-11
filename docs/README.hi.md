# greenpng — परियोजना गाइड (हिन्दी)

## 1. यह परियोजना क्या है

greenpng (GR) किसी सत्र (session) के वास्तविक मानव होने की जाँच
**असली आगंतुक के ब्राउज़र** में जाँच (probe) करके करता है — केवल ट्रैफ़िक
की निगरानी से नहीं।

पाइपलाइन, शुरू से अंत तक:

1. **जाँच (ब्राउज़र में)** — एक हस्ताक्षरित FE लोडर आपके पृष्ठों पर चलता है
   और प्रत्येक सत्र में **कई चरणों वाले बैचों** में **कई स्रोतों** (इनपुट
   गतिशीलता, डिवाइस स्टैक, परिवेश की प्रामाणिकता, ऑटोमेशन के निशान) से
   साक्ष्य एकत्र करता है।
2. **अपलोड** — ब्राउज़र प्रत्येक बैच को एक सीलबंद (sealed) इनजेस्ट
   पाइपलाइन के माध्यम से सर्वर को भेजता है; रिप्ले या छेड़छाड़ वाली
   प्रस्तुतियाँ भंडारण (storage) तक पहुँचने से पहले ही अस्वीकार कर दी जाती हैं।
3. **विश्लेषण (सर्वर पर)** — विश्लेषण प्लेन प्रत्येक सत्र को प्रति-सत्र
   निर्णय (`human | watch | bot`) में, प्रति-अक्ष (per-axis) विश्वास के साथ,
   और एक स्थिर, टकराव-सचेत (collision-aware) डिवाइस पहचान के साथ अंकित
   करता है।
4. **वापसी (रिटर्न)** — आपका बैकएंड परिणाम API के माध्यम से परिणाम प्राप्त
   करता है (छह-भाषाओं के SDK; `public | sdk | diagnostic` प्रोजेक्शन नियंत्रित
   करते हैं कि प्रत्येक कॉलर को क्या दिखाई देता है)।

सर्वर की ओर, greenpng डिफ़ॉल्ट रूप से एक होस्ट बाइनरी के रूप में, ट्री-के-भीतर
(in-tree) जाँच प्लेन के साथ चलता है, और **बहु-नोड, लोड-संतुलित (load-balanced)
परिनियोजन** का समर्थन करता है: एक LB मॉड्यूल साझा डेटा परत (PostgreSQL + Redis)
के सामने नोड्स पर जाँच ट्रैफ़िक फैलाता है, ताकि संग्रहण और विश्लेषण क्षैतिज
रूप से स्केल हो सकें।

## 2. रिपॉज़िटरी संरचना और आर्किटेक्चर

| निर्देशिका | विषय-सामग्री |
|---|---|
| `crates/` | Rust वर्कस्पेस — `gr-service` (कंट्रोल प्लेन + एडमिन कंसोल + in-tree जाँच प्लेन), `gr-probe-core`, `gr-probe-plane`, `gr-probe-store`, `gr-ota`, `gr-admin`, `gr-runtime`, … |
| `modules/` | हस्ताक्षरित हॉट-अपडेट मॉड्यूल स्रोत (identity / brain / analyze / ingest / edge / probe_assets) |
| `probe/` | ब्राउज़र जाँच FE (लोडर, पैक शृंखला, सीलबंद इनजेस्ट क्लाइंट) |
| `panel/` | एडमिन पैनल — Vue स्रोत (`admin-ui/`) + निर्मित SPA (`admin-spa/`), EN + 中文 |
| `sdk/` | छह भाषाओं में बैकएंड एकीकरण SDK (केवल परिणाम प्राप्त करने के लिए) |
| `spec/` | रनटाइम पर लोड होने वाले वायर/स्कोरिंग विनिर्देश (specs) |
| `fixtures/` | कॉन्ट्रैक्ट-टेस्ट डेटा |
| `scripts/` | बिल्ड स्क्रिप्ट और FE टूलिंग (`scripts/fe/checks/`) |
| `vendor/` | Vendored निर्भरता स्रोत (pingora) |
| `install/` | इंस्टॉलर, डेटा-परत compose, अपग्रेड स्क्रिप्ट |
| `release/` | पैकेजिंग स्क्रिप्ट (मल्टी-आर्क बिल्ड, SBOM, मॉड्यूल साइनिंग) |
| `docs/` | यह गाइड, प्रति भाषा एक फ़ाइल |

```
        आगंतुक ब्राउज़र
              │  <script src="/gr.js">  (pinned, no-store)
              ▼
   FE लोडर ──► /v1/sdk/bootstrap ──► संस्करणित पैक मैनिफेस्ट
              │        asset_base /dist/v/<fe>/g/<gen>/
              ▼
   पैक संग्राहक (इनपुट · डिवाइस · परिवेश, मल्टी-बैच)
              │  सीलबंद सबमिट (बंधे gv डोमेन पर सीधे, TLS)
              ▼
 ┌────────────┴─────────────┐   ┌──────────────────────────────┐
 │ gr-probe-plane (Pingora)  │   │ gr-service (कंट्रोल प्लेन)    │
 │  गेटवे · इनजेस्ट · ops     │◄──┤  एडमिन कंसोल · साइट कॉन्फ़िग     │
 └────────────┬─────────────┘   │  मॉड्यूल रजिस्ट्री (OTA, हस्ताक्षरित)│
              ▼                 └──────────────┬───────────────┘
   PostgreSQL (बहु-नोड में + Redis)            │
              ▼                                │
   GET /v1/session/{id}/result ──► मर्चेंट SDK (छह भाषाएँ)

   बहु-नोड: LB मॉड्यूल साझा डेटा परत के सामने
   वाले कई gr-service नोड्स पर जाँच ट्रैफ़िक फैलाता है
```

मुख्य घटक: `gr-service` कंट्रोल प्लेन है (रैंडम-पाथ एडमिन कंसोल,
साइट/कॉन्फ़िग प्रबंधन, हस्ताक्षरित OTA मॉड्यूल रजिस्ट्री) और ट्री-के-भीतर
जाँच प्लेन की मेज़बानी करता है; `gr-probe-plane` Pingora गेटवे है जिसमें
सीलबंद इनजेस्ट और सत्र/परिणाम API हैं; ब्राउज़र FE प्रत्येक एसेट को
संस्करण-अपरिवर्तनीय URL में रिज़ॉल्व करता है ताकि कैश कभी भी रिलीज़ के पार
पुरानी जाँच सर्व न कर सकें; PostgreSQL (बहु-नोड होने पर Redis सहित) सत्रों,
बैचों और विश्लेषण परिणामों को संग्रहीत करता है।

## 3. इंस्टॉल और उपयोग

### 3.1 इंस्टॉल

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh | bash
# या स्पष्ट संस्करण / आर्क के साथ:
bash install/install.sh --version <VERSION> --arch x86_64 --yes
# डॉकरयुक्त डेटा परत के साथ (केवल PostgreSQL/Redis; सर्वर होस्ट बाइनरी ही रहता है):
bash install/install.sh --version <VERSION> --with-docker --yes
```

इंस्टॉलर sha256 + ELF + ed25519 मॉड्यूल हस्ताक्षरों की पुष्टि करता है,
`/opt/greenpng` के अंतर्गत इंस्टॉल करता है, `.env` और systemd यूनिट लिखता है,
छह हस्ताक्षरित मॉड्यूल स्टेज और सक्रिय करता है, और कंट्रोल/प्लेन
`/v1/health` पर गेट लगाता है। प्रथम-लॉगिन क्रेडेंशियल और रैंडम कंसोल पाथ
`/opt/greenpng/data/admin/admin_bootstrap_once.txt` में लिखे जाते हैं।

### 3.2 अपडेट

| प्राथमिकता | चैनल | उपयोग |
|---|---|---|
| P0 | पैनल OTA (set-release-url → install / install-fe / install-runtime) | डिफ़ॉल्ट |
| P1 | `install/release/update_runtime_from_github.sh`, `update_module_from_github.sh` | पैनल नहीं / फ्री नोड्स |
| P2 | SSH मैनुअल | मृत प्रक्रिया / प्रथम इंस्टॉल |

सभी अपडेट इस रिपॉज़िटरी से उसी tag के हस्ताक्षरित Release एसेट खींचते हैं।
Docker एक रनटाइम कंटेनर है, अपडेट चैनल नहीं।

### 3.3 उपयोग

1. एडमिन पैनल में **एक साइट बनाएँ**: साइट id, रूट डोमेन, और उन व्यवसाय
   फ़ील्ड के लिए Cookie अनुमति-सूची (allow-list) जिन्हें आप प्रत्येक निर्णय
   के साथ जोड़ना चाहते हैं (`password`/`token` जैसे संवेदनशील नाम
   सर्वर-पक्ष में अवरुद्ध होते हैं)।
2. **जाँच (probe) परिनियोजित करें** — तीन मोड:
   - *Nginx फ़र्स्ट-पार्टी (अनुशंसित)*: pv डोमेन पर `/gr.js` + `/gr/dist/v/` +
     `/gr/v1/` को प्रोब प्लेन पर प्रॉक्सी करें (Cookie पासथ्रू),
     HTML में
     `<script src="/gr.js" data-site-id="…" data-endpoint="/gr"
     data-inject-path="nginx" defer></script>` इंजेक्ट करें।
   - *Cloudflare worker*: वही टैग इंजेक्ट करें और `/gr` को in-origin प्रॉक्सी करें।
   - *ऐप एम्बेड*: लोडर को सीधे pv/CDN से लोड करें, `data-endpoint` को gv/pv
     की ओर इंगित करें।
   ब्राउज़र अपलोड सदैव **बंधे gv डोमेन पर TLS के माध्यम से सीधे** जाते हैं।
3. **परिणाम प्राप्त करें** — पैनल में एक साइट SDK key बनाएँ, फिर पोल करें:

```bash
KEY="grsk_..." SID="cycle_..." BASE="https://gv.example.com"
curl -sS -H "X-Gr-Sdk-Key: $KEY" "$BASE/v1/session/$SID/result"
```

4. **स्मोक टेस्ट** (curl):

```bash
# अनुमति-सूची वाले cookies के साथ एक सत्र खोलें
curl -sS -X POST "$BASE/v1/session/open" -H "X-Gr-Sdk-Key: $KEY" \
     -H 'Cookie: user_id=u9; plan_tier=pro' -d '{"site_id":"mysite"}'
# → {"session_id":"cycle_…", …}
# फिर परिणाम जाँचें (वास्तविक FE बैचों या सिम्युलेटेड बैचों के बाद):
curl -sS -H "X-Gr-Sdk-Key: $KEY" "$BASE/v1/session/$SID/result"
```

---

## 4. एडमिन पैनल पैरामीटर

पैनल के **Config** पेज पर संपादित किए जाते हैं (सहेजें → प्रकाशित करें):
प्रकाशित करने वाला नोड तुरंत लागू करता है; क्लस्टर नोड ≤30 सेकंड में,
पुनरारंभ के बिना।

**रेट सीमाएँ** (v1.0.14 नीति: साइट-कुलयोग डिफ़ॉल्ट रूप से बंद; टेलीमेट्री
मार्ग इसके बजाय प्रति-एकल-IP सीमित; 429 उत्तर ट्रिप हुई परत का नाम बताते हैं):

| पैरामीटर | डिफ़ॉल्ट | अर्थ |
|---|:---:|---|
| `rate_limit_open_per_min` | 0 = असीमित | सत्र-ओपन / साइट / मिनट |
| `rate_limit_ingest_per_min` | 0 = असीमित | बैच-अपलोड / साइट / मिनट |
| `rate_limit_analyze_per_min` | 0 = असीमित | सीधे विश्लेषण / साइट / मिनट |
| `rate_limit_complete_per_min` | 0 = असीमित | complete-रसीदें / साइट / मिनट |
| `rate_limit_result_per_min` | 0 = असीमित | परिणाम-पढ़ना / साइट / मिनट |
| `rate_limit_client_event_per_min` | 0 = असीमित | FE टेलीमेट्री / साइट / मिनट (कुल) |
| `rate_limit_client_event_per_ip_per_min` | 100 | FE टेलीमेट्री **प्रति एकल IP** / मिनट — सीमा पार होने पर केवल वही IP प्रभावित; 0 = बंद |

**CDN के पीछे** प्रति-IP परत उसी IP पर आधारित होती है जो सर्वर देखता है।
प्रॉक्सी के CIDR को `/opt/greenpng/.env` में `GR_TRUSTED_PROXIES` में जोड़ें
और सामने के प्रॉक्सी पर असली विज़िटर IP पुनर्स्थापित करें (nginx उदाहरण):

```nginx
set_real_ip_from 173.245.48.0/20;  # Cloudflare IPv4
set_real_ip_from 2400:cb00::/32;   # Cloudflare IPv6
real_ip_header CF-Connecting-IP;
```

**हॉट/कोल्ड टियरिंग** (वास्तविक नाम): `cold_ttl_ms` (604800000 = 7 दिन),
`cold_promote_window_ms` (864000000 = 24 घंटे), `cold_purge_interval_ms`
(300000 = 5 मिनट); प्रति-साइट प्रतिधारण पैनल के **Data Retention** पेज पर
सेट होता है और सीमित बैचों में पर्ज होता है।

## 5. लॉगिंग और मेमोरी

- सेवा लॉग: `journalctl -u greenpng.service`; संचालन टेलीमेट्री
  (`ops_client_events`) को `ops_retention_days` (14) दिन रखा जाता है।
- **मेमोरी टिप (v1.0.14 से)**: लंबे समय तक चलने वाले मल्टी-कोर होस्ट पर
  glibc प्रति कोर 8 तक arenas (~64 MB प्रत्येक) रख सकता है, जिससे
  समवर्ती भार में RSS बढ़ सकता है। इसलिए इंस्टॉलर `/opt/greenpng/.env`
  में `MALLOC_ARENA_MAX=4` डिफ़ॉल्ट रखता है; भार के अंतर्गत RSS स्थिर रहता है।

## 6. ओपन-सोर्स प्रोजेक्ट और संदर्भ

- [Cloudflare Pingora](https://github.com/cloudflare/pingora) (Apache-2.0) — एज गेटवे, TLS टर्मिनेशन, सील्ड इनजेस्ट (openssl फ़ीचर)।
- [Tokio](https://github.com/tokio-rs/tokio) और [Axum](https://github.com/tokio-rs/axum) (MIT) — कंट्रोल प्लेन का एसिंक रनटाइम और REST फ़्रेमवर्क।
- [OpenSSL](https://www.openssl.org/) (Apache-2.0) — एज और एडमिन कंसोल का TLS बैकएंड।
- [ed25519-dalek](https://github.com/dalek-cryptography/curve25519-dalek) (BSD-3) — रिलीज़ मैनिफ़ेस्ट, प्रोब एसेट और OTA के हस्ताक्षर।
- [PostgreSQL](https://www.postgresql.org/) और [Redis](https://redis.io/) — L3 स्टोरेज और मल्टी-नोड स्थिति।
- [flate2 / zlib](https://github.com/rust-compress/flate2) (MIT) — पेलोड संपीड़न (zstd केवल वेंडर्ड pingora में)।
- [Element Plus](https://element-plus.org/) और [Vue 3](https://vuejs.org/) (MIT) — एडमिन कंसोल UI।
- शोध संदर्भ: [CreepJS](https://github.com/abrahamjuliot/creepjs) (B1/B12 प्रेरणा), [FingerprintJS](https://github.com/fingerprintjs/fingerprintjs), [BotD](https://github.com/fingerprintjs/botd)।
