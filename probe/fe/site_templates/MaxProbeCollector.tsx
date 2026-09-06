'use client';

import { useEffect, useRef, useState } from 'react';

/**
 * Secondary / fallback loader for green-v5.
 * Supports first-party apiBase (/g5) + optional CDN assetBase for scripts.
 */
export function MaxProbeCollector() {
  const started = useRef(false);
  const [status, setStatus] = useState('idle');

  useEffect(() => {
    if (started.current) return;
    if (typeof window === 'undefined') return;

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const w = window as any;

    if (w.__GV5_BOOT_STARTED__ || w.__MP_BOOT_STARTED__) {
      started.current = true;
      setStatus(String(w.__GV5_SESSION_ID__ ? 'boot-owned' : 'boot-started'));
      return;
    }

    const enabled =
      process.env.NEXT_PUBLIC_MAX_PROBE_ENABLED === '1' ||
      process.env.NEXT_PUBLIC_MAX_PROBE_ENABLED === 'true' ||
      process.env.NEXT_PUBLIC_MAX_PROBE_ENABLED === undefined;
    if (
      process.env.NEXT_PUBLIC_MAX_PROBE_ENABLED === '0' ||
      process.env.NEXT_PUBLIC_MAX_PROBE_ENABLED === 'false'
    ) {
      return;
    }

    const apiBase = (process.env.NEXT_PUBLIC_MAX_PROBE_BASE || '').replace(/\/$/, '');
    if (!enabled && !apiBase) return;
    if (!apiBase) {
      setStatus('fail:no_base');
      return;
    }

    const gwBase = (process.env.NEXT_PUBLIC_MAX_PROBE_GW_BASE || '').replace(/\/$/, '');
    const assetBase = (
      process.env.NEXT_PUBLIC_MAX_PROBE_ASSET_BASE ||
      (apiBase.startsWith('/') ? apiBase : apiBase)
    ).replace(/\/$/, '');
    const firstParty =
      process.env.NEXT_PUBLIC_MAX_PROBE_FIRST_PARTY === '1' ||
      process.env.NEXT_PUBLIC_MAX_PROBE_FIRST_PARTY === 'true' ||
      apiBase.startsWith('/');

    const domainId =
      process.env.NEXT_PUBLIC_MAX_PROBE_DOMAIN ||
      (() => {
        try {
          if (/^https?:\/\//i.test(apiBase)) return new URL(apiBase).hostname;
        } catch {
          /* ignore */
        }
        try {
          return location.hostname;
        } catch {
          return '';
        }
      })();
    const envId = process.env.NEXT_PUBLIC_MAX_PROBE_ENV_ID || 'prod';
    const sdkV = process.env.NEXT_PUBLIC_MAX_PROBE_SDK_V || 'v5';
    const siteId = process.env.NEXT_PUBLIC_MAX_PROBE_SITE_ID || '';

    started.current = true;
    setStatus('loading-boot');

    const resolveScriptSrc = (name: string) => {
      const v = encodeURIComponent(sdkV);
      const base = assetBase || apiBase;
      // absolute or relative both fine for <script src>
      return `${base}/dist/${name}?v=${v}`;
    };

    const startBoot = (injectPath: string) => {
      if (w.__GV5_BOOT_STARTED__ || w.__MP_BOOT_STARTED__) {
        setStatus(String(w.__GV5_SESSION_ID__ ? 'boot-owned' : 'boot-started'));
        return;
      }
      const prev = w.__GV5_BOOT__ || {};
      w.__GV5_BOOT__ = Object.assign({}, prev, {
        apiBase,
        gwBase: gwBase || undefined,
        assetBase: assetBase || undefined,
        domain_id: domainId || undefined,
        env_id: envId,
        version: sdkV,
        site_id: siteId || prev.site_id || prev.siteId || undefined,
        siteId: siteId || prev.siteId || prev.site_id || undefined,
        short_visit: true,
        concurrency: firstParty ? 12 : 10,
        first_party: firstParty || undefined,
        first_party_path: firstParty ? apiBase : undefined,
      });
      w.__GV5_BOOT__.injectPath = injectPath;
      w.__GV5_BOOT__.inject_path = injectPath;
      if (firstParty) w.__GV5_FIRST_PARTY__ = 1;
      if (injectPath === 'nginx' || injectPath === 'cf_worker') {
        w.__GV5_INJECT_PRIMARY__ = injectPath;
      }
      if (w.__GV5_BOOT__.siteId || w.__GV5_BOOT__.site_id) {
        w.__GV5_SITE_ID__ = w.__GV5_BOOT__.siteId || w.__GV5_BOOT__.site_id;
      }
      w.__MP_BOOT__ = w.__GV5_BOOT__;

      // Prefer entry (race+boot); fall back to boot then CDN asset
      const tryLoad = (srcs: string[], i: number) => {
        if (i >= srcs.length) {
          setStatus('fail:boot_load');
          return;
        }
        const s = document.createElement('script');
        s.src = srcs[i];
        s.async = true;
        s.setAttribute('data-inject-path', injectPath);
        s.setAttribute('data-endpoint', apiBase);
        if (gwBase) s.setAttribute('data-gw-base', gwBase);
        if (siteId) s.setAttribute('data-site-id', siteId);
        s.onload = () => setStatus('boot-loaded');
        s.onerror = () => tryLoad(srcs, i + 1);
        (document.head || document.documentElement).appendChild(s);
      };

      const primary = resolveScriptSrc('gv5.entry.min.js');
      const fallbacks = [resolveScriptSrc('gv5.boot.min.js'), resolveScriptSrc('gv5.boot.js')];
      // If assetBase is CDN and differs from first-party apiBase, try CDN first then same-origin
      const srcs = [primary, ...fallbacks];
      if (
        process.env.NEXT_PUBLIC_MAX_PROBE_ASSET_BASE &&
        process.env.NEXT_PUBLIC_MAX_PROBE_ASSET_BASE.replace(/\/$/, '') !== apiBase
      ) {
        // also try apiBase-hosted dist as last resort
        srcs.push(`${apiBase}/dist/gv5.entry.min.js?v=${encodeURIComponent(sdkV)}`);
      }
      tryLoad(srcs, 0);
    };

    const resolvePrimary = (): string => {
      const sticky = w.__GV5_INJECT_PRIMARY__;
      if (sticky === 'nginx' || sticky === 'cf_worker') return sticky;
      const prev = w.__GV5_BOOT__ || {};
      const p = prev.injectPath || prev.inject_path;
      if (p === 'nginx' || p === 'cf_worker') return p;
      try {
        const nodes = document.querySelectorAll('script[data-inject-path]');
        for (let i = 0; i < nodes.length; i++) {
          const v = nodes[i].getAttribute('data-inject-path');
          if (v === 'nginx' || v === 'cf_worker') return v;
        }
      } catch {
        /* ignore */
      }
      return '';
    };

    // Grace window for nginx primary inject
    const t0 = Date.now();
    const tick = () => {
      if (w.__GV5_BOOT_STARTED__ || w.__MP_BOOT_STARTED__) {
        setStatus(String(w.__GV5_SESSION_ID__ ? 'boot-owned' : 'boot-started'));
        return;
      }
      const primary = resolvePrimary();
      if (primary) {
        startBoot(primary);
        return;
      }
      if (Date.now() - t0 < 80) {
        setTimeout(tick, 20);
        return;
      }
      startBoot('app');
    };
    tick();
  }, []);

  if (process.env.NEXT_PUBLIC_MAX_PROBE_SHOW === '1') {
    return <span data-gv5-collector={status} style={{ display: 'none' }} />;
  }
  return null;
}
