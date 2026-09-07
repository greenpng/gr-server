# V5 FE CDN deployment

> **中文正式说明（一方域名 vs 跨域/CDN、面板操作、验收脚本）** →  
> [`v5-docs2/07-deploy/09-fe-load-first-party-and-cdn.md`](../v5-docs2/07-deploy/09-fe-load-first-party-and-cdn.md)

The probe program is the default-install JS loaded on the website. It supports
**CDN deployment** (any HTTPS origin that serves static assets).

## Minimal embed

```html
<script
  src="https://cdn.example.com/gr/gr.boot.min.js"
  data-site-id="your_site_id"
  data-endpoint="https://probe.example.com"
  data-gw-base="https://probe.example.com"
  defer
></script>
```

| Attribute | Meaning |
|-----------|---------|
| `src` | CDN URL of `gr.boot.min.js` (or `gr.boot.js`) |
| `data-site-id` | Business site id for tenant binding |
| `data-endpoint` | V5 API base (open/ingest/analyze) |
| `data-gw-base` | Gateway base when separate from API |
| `data-inject-path` | Optional: `nginx` \| `cf_worker` \| `app` |
| `data-autostart="0"` | Disable auto start; call `GR.Boot.start()` yourself |

## Cache policy (greenpng 1.0.3+)

Do **not** use `?v=` query busting — queries are not a reliable cache key for
browsers/CDNs. Version lives in the **real asset path**:

```text
/dist/v/<fe_version>/g/<asset_gen>/<content-hash>.<ext>
```

- Every release (new `fe_version`) and every FE rebuild (new `asset_gen`)
  rotates the whole URL path → stale assets and stale 404s can never be
  served from browser/CDN cache across releases.
- The pin (`/gr.js`) is `no-store` (checked fresh every load).
- Plane-side 404s are `no-store`; a missing asset must never become a
  long-lived immutable cache entry at an edge (178 v1.0.2 Cloudflare
  incident: a transient 404 was cached ~1y under a blanket immutable header).
- Server strips the `v/<ver>/[g/<gen>/]` segment when resolving to the
  logical file (`strip_version_route`); versioned paths are long-immutable
  by `static_cache_control`.
- Legacy embeds may still append `?v=5.x.y` — the FE strips it on sight.

## Business link fields (before or after load)

```html
<script>
  // Opaque / HMAC only — never plaintext email/phone/user_id
  window.GR = window.GR || {};
  // If boot already ran, use setLinkFields after load:
</script>
<script src="https://cdn.example.com/gr/gr.boot.min.js" data-site-id="s1" defer></script>
<script>
  document.addEventListener("DOMContentLoaded", function () {
    if (window.GR && GR.setLinkFields) {
      GR.setLinkFields({
        custom_link: { id: "sub_v1_…", cohort: "seller" },
        client_tags: { page: "checkout" },
      });
    }
  });
</script>
```

Server sanitizes `custom_link` / `client_tags` (PII key names stripped, length caps).
These fields **never** enter commercial `device_id` mint materials.

## Same-origin vs cross-origin

- Prefer same-site API base to simplify cookies / CORS.
- Cross-origin CDN for JS is fine; set `data-endpoint` to the probe API origin
  and enable CORS on V5 for that site.

## Files typically published to CDN

- `gr.boot.min.js` (+ `.gz` if edge supports)
- `gr.loader.min.js`, pack registry, collectors as needed by boot
- Do **not** publish backend secrets or SDK tenant HMAC keys in FE
