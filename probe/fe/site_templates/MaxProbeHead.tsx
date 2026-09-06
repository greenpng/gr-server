/**
 * Next.js secondary path for green-v5.
 *
 * Primary: nginx sub_filter injects __GV5_BOOT__ + gv5.entry (single-flight).
 * This component merges config into window.__GV5_BOOT__ only.
 *
 * First-party relay (recommended under CF orange / Under Attack):
 *   NEXT_PUBLIC_MAX_PROBE_BASE=/g5          # same-origin reverse proxy to ingest
 *   NEXT_PUBLIC_MAX_PROBE_GW_BASE=/g5-gw    # optional gateway proxy
 *   NEXT_PUBLIC_MAX_PROBE_ASSET_BASE=https://pv…  # CDN for /dist only (optional)
 *   NEXT_PUBLIC_MAX_PROBE_FIRST_PARTY=1
 *
 * Legacy CDN dual-role (script + API both on pv — fragile under challenge):
 *   NEXT_PUBLIC_MAX_PROBE_BASE=https://pv.example.com
 */
import Script from 'next/script';

const apiBase = (process.env.NEXT_PUBLIC_MAX_PROBE_BASE || '').replace(/\/$/, '');
const gwBase = (process.env.NEXT_PUBLIC_MAX_PROBE_GW_BASE || '').replace(/\/$/, '');
const assetBase = (
  process.env.NEXT_PUBLIC_MAX_PROBE_ASSET_BASE ||
  // When API is first-party path, assets default same-origin /g5 (proxied dist)
  (apiBase.startsWith('/') ? apiBase : apiBase)
).replace(/\/$/, '');
const domainId = process.env.NEXT_PUBLIC_MAX_PROBE_DOMAIN || '';
const envId = process.env.NEXT_PUBLIC_MAX_PROBE_ENV_ID || 'prod';
const sdkV = process.env.NEXT_PUBLIC_MAX_PROBE_SDK_V || '';
const siteId = process.env.NEXT_PUBLIC_MAX_PROBE_SITE_ID || '';
const firstParty =
  process.env.NEXT_PUBLIC_MAX_PROBE_FIRST_PARTY === '1' ||
  process.env.NEXT_PUBLIC_MAX_PROBE_FIRST_PARTY === 'true' ||
  (apiBase.startsWith('/') && apiBase.length > 1);

export function MaxProbeHead() {
  if (!apiBase) return null;

  const cfg = {
    apiBase,
    gwBase: gwBase || undefined,
    assetBase: assetBase || undefined,
    domain_id: domainId || undefined,
    env_id: envId,
    version: sdkV || undefined,
    site_id: siteId || undefined,
    siteId: siteId || undefined,
    short_visit: true,
    // First-party same-origin can open more concurrent posts (no CORS tax).
    concurrency: firstParty ? 12 : 10,
    first_party: firstParty || undefined,
    first_party_path: firstParty ? apiBase || '/g5' : undefined,
  };

  // preconnect only for absolute CDN hosts
  const absAsset = assetBase && /^https?:\/\//i.test(assetBase) ? assetBase : '';
  const absApi = apiBase && /^https?:\/\//i.test(apiBase) ? apiBase : '';
  const absGw = gwBase && /^https?:\/\//i.test(gwBase) ? gwBase : '';

  return (
    <>
      {absAsset ? <link rel="preconnect" href={absAsset} crossOrigin="anonymous" /> : null}
      {absAsset ? <link rel="dns-prefetch" href={absAsset} /> : null}
      {absApi && absApi !== absAsset ? (
        <link rel="preconnect" href={absApi} crossOrigin="anonymous" />
      ) : null}
      {absGw ? <link rel="dns-prefetch" href={absGw} /> : null}
      {gwBase && siteId && absGw ? (
        <link
          rel="prefetch"
          href={`${gwBase}/s0?site_id=${encodeURIComponent(siteId)}&e=next_prefetch&inject_path=app`}
        />
      ) : null}
      <Script
        id="gv5-boot-cfg"
        strategy="beforeInteractive"
        dangerouslySetInnerHTML={{
          __html: `(function(){var p=window.__GV5_BOOT__||{};var c=${JSON.stringify(cfg)};var sticky=window.__GV5_INJECT_PRIMARY__;var ip=sticky||p.injectPath||p.inject_path||"";if(ip==="nginx"||ip==="cf_worker"){window.__GV5_INJECT_PRIMARY__=ip;}else if(!ip){}window.__GV5_BOOT__=Object.assign({},p,c);if(c.first_party){window.__GV5_FIRST_PARTY__=1;}if(c.siteId||c.site_id){window.__GV5_SITE_ID__=c.siteId||c.site_id;}if(window.__GV5_INJECT_PRIMARY__){window.__GV5_BOOT__.injectPath=window.__GV5_INJECT_PRIMARY__;window.__GV5_BOOT__.inject_path=window.__GV5_INJECT_PRIMARY__;}window.__MP_BOOT__=window.__GV5_BOOT__;})();`,
        }}
      />
    </>
  );
}
