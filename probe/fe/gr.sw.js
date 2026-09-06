/* green-v5 service worker — optional static layer cache (versioned).
 * Register only when GR_ENABLE_SW=1 or boot data-enable-sw=1.
 * Strategy: cache-first for versioned collectors (*.min.js?v=VERSION); network-first for API.
 */
const CACHE_PREFIX = "gr-static-";

function cacheNameFromVersion(v) {
  return CACHE_PREFIX + String(v || "dev");
}

self.addEventListener("install", (event) => {
  self.skipWaiting();
  event.waitUntil(Promise.resolve());
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches.keys().then((keys) =>
      Promise.all(
        keys
          .filter((k) => k.startsWith(CACHE_PREFIX))
          .map((k) => {
            // Keep only current if clients claim later
            return Promise.resolve();
          })
      ).then(() => self.clients.claim())
    )
  );
});

function isVersionedStatic(url) {
  try {
    const u = new URL(url);
    if (!/\.min\.js$/i.test(u.pathname) && !/\/collectors\//.test(u.pathname)) return false;
    // Prefer ?v= busted assets only
    return u.searchParams.has("v") || /registry\.(static|mid|dense|random)/.test(u.pathname);
  } catch (e) {
    return false;
  }
}

self.addEventListener("fetch", (event) => {
  const req = event.request;
  if (req.method !== "GET") return;
  const url = req.url;
  // Never cache API / ingest / analyze
  if (/\/v1\//.test(url) || /\/s0/.test(url) || /ingest|analyze|session\/open/.test(url)) {
    return;
  }
  if (!isVersionedStatic(url)) return;

  event.respondWith(
    (async () => {
      const cache = await caches.open(CACHE_PREFIX + "layers");
      const hit = await cache.match(req);
      if (hit) return hit;
      try {
        const res = await fetch(req);
        if (res && res.ok) {
          try {
            await cache.put(req, res.clone());
          } catch (e) {}
        }
        return res;
      } catch (e) {
        return hit || Response.error();
      }
    })()
  );
});
