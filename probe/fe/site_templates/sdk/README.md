# greenv5 first-party Next SDK

Same-origin probe upload for the four Next.js business sites.

## Why

Cross-origin `pv.*` under Cloudflare orange / Under Attack challenges browser uploads (CORS/403).  
First-party path: browser → `https://www.site/g5/v1/ingest` → origin workers (no CF challenge on API).

## Performance contract

| Layer | Rule |
|-------|------|
| Browser FE | Collect packs independent of upload; UploadQueue concurrent, never waits on sibling posts |
| Next relay | **Stream** body; no `JSON.parse`; forward CF/XFF/UA; timeout only |
| nginx (preferred prod) | `proxy_pass` /g5 → :28765, /g5-gw → :28766 (see `scripts/nginx_g5_first_party_snippet.conf`) |
| Origin | ingest debounce analyze; do not block HTTP response on full evaluate |

**Do not** put business logic / JSON transform in the relay mid-layer.

## Install

```bash
# From green-v5 tree into a site app/
cp -a fe/site_templates/sdk/g5 app/
cp -a fe/site_templates/sdk/g5-gw app/
cp fe/site_templates/sdk/relay.ts lib/gr/relay.ts
# fix import paths in route.ts to '@/lib/gr/relay'
cp fe/site_templates/MaxProbeHead.tsx components/
cp fe/site_templates/MaxProbeCollector.tsx components/
```

Env (site):

```
NEXT_PUBLIC_MAX_PROBE_ENABLED=1
NEXT_PUBLIC_MAX_PROBE_BASE=/g5
NEXT_PUBLIC_MAX_PROBE_GW_BASE=/g5-gw
NEXT_PUBLIC_MAX_PROBE_FIRST_PARTY=1
NEXT_PUBLIC_MAX_PROBE_SITE_ID=<site>
NEXT_PUBLIC_MAX_PROBE_SDK_V=v5.8.16-…
# optional CDN for /dist only:
# NEXT_PUBLIC_MAX_PROBE_ASSET_BASE=https://pv.example.com
GR_INGEST_UPSTREAM=http://127.0.0.1:28765
GR_GW_UPSTREAM=http://127.0.0.1:28766
```

Layout head: `<MaxProbeHead />`  
Body fallback: `<MaxProbeCollector />` (only if nginx inject did not already boot).

## Latency hierarchy

1. **nginx** stream proxy (best)  
2. **Next route** stream proxy (this SDK)  
3. **Legacy CDN** `apiBase=https://pv…` (avoid under challenge)

## Local smoke

```bash
# Preferred — production-style nginx inject + domain + TLS (lab-https):
#   ./lab-https/scripts/gen-certs.sh && ./lab-https/scripts/render-nginx-conf.sh
#   cd lab-https && docker compose up -d
#   open https://gr-lab.local:8443/
# After fe/VERSION change:
#   ./lab-https/scripts/render-nginx-conf.sh --reload

# workers on 28765/28766 — HTTP python first-party (no TLS):
python3 scripts/local_first_party_stack.py --port 18090
# or Next site with app/g5 routes + same env
```
