# site_templates — Next.js first-party greenv5 embed

| File | Role |
|------|------|
| `MaxProbeHead.tsx` | beforeInteractive boot cfg (`apiBase=/g5`, first_party) |
| `MaxProbeCollector.tsx` | fallback script loader if nginx inject missing |
| `sdk/relay.ts` | **stream** reverse proxy (no JSON reparse) |
| `sdk/g5/[[...path]]/route.ts` | App Router catch-all → ingest :28765 |
| `sdk/g5-gw/[[...path]]/route.ts` | App Router catch-all → gateway :28766 |
| `sdk/README.md` | install + performance contract |

## Latency rule

1. Prefer **nginx** `location /g5/` (see `scripts/nginx_g5_first_party_snippet.conf`)  
2. Else **Next stream relay** (this SDK) — overhead should stay ~1–3ms on localhost  
3. Avoid browser → `https://pv…` under CF challenge  

Browser **probe packs never await upload**. UploadQueue concurrency is independent; first-party mode ramps concurrency earlier because same-origin has no CORS tax.
