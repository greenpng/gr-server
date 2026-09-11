/**
 * greenv5 first-party stream relay for Next.js App Router.
 *
 * Design goals (must NOT slow probe/upload):
 * 1. Stream body through — never JSON.parse / never re-encode.
 * 2. Forward CF / XFF / UA / Content-Type exactly (identity / bot paths).
 * 3. Short connect budget; do not buffer full body on edge.
 * 4. Prefer nginx location /g5 in prod; this route is the portable same-origin path
 *    when the site is pure Next (or nginx not yet wired).
 *
 * Drop into site app/ as:
 *   app/g5/[[...path]]/route.ts      → re-export from this module
 *   app/g5-gw/[[...path]]/route.ts
 *
 * Env:
 *   GR_INGEST_UPSTREAM   default http://127.0.0.1:28765
 *   GR_GW_UPSTREAM       default http://127.0.0.1:28766
 *   GR_RELAY_CONNECT_MS  default 3000
 *   GR_RELAY_TIMEOUT_MS  default 60000  (upload upper bound)
 */

export type RelayRole = 'ingest' | 'gateway';

const HOP_BY_HOP = new Set([
  'connection',
  'keep-alive',
  'proxy-authenticate',
  'proxy-authorization',
  'te',
  'trailers',
  'transfer-encoding',
  'upgrade',
  'host',
  'content-length', // let fetch recompute for non-stream edge cases
]);

/** Headers that must survive for identity / geo / bot correlation. */
const FORWARD_ALWAYS = [
  'user-agent',
  'accept',
  'accept-language',
  'accept-encoding',
  'content-type',
  'content-encoding',
  'origin',
  'referer',
  'cookie',
  // Client IP chain
  'x-forwarded-for',
  'x-forwarded-proto',
  'x-real-ip',
  'x-client-ip',
  'cf-connecting-ip',
  'true-client-ip',
  'cf-ipcountry',
  'cf-ray',
  'cf-visitor',
  // Optional SDK meta
  'x-gv5-site-id',
  'x-gv5-inject-path',
  'x-request-id',
];

function upstreamBase(role: RelayRole): string {
  if (role === 'gateway') {
    return (
      process.env.GR_GW_UPSTREAM ||
      process.env.NEXT_PUBLIC_GR_GW_UPSTREAM ||
      'http://127.0.0.1:28766'
    ).replace(/\/$/, '');
  }
  return (
    process.env.GR_INGEST_UPSTREAM ||
    process.env.NEXT_PUBLIC_GR_INGEST_UPSTREAM ||
    'http://127.0.0.1:28765'
  ).replace(/\/$/, '');
}

function clientIp(req: Request): string {
  const h = req.headers;
  return (
    h.get('cf-connecting-ip') ||
    h.get('true-client-ip') ||
    h.get('x-real-ip') ||
    (h.get('x-forwarded-for') || '').split(',')[0].trim() ||
    ''
  );
}

function buildForwardHeaders(req: Request): Headers {
  const out = new Headers();
  const src = req.headers;
  for (const name of FORWARD_ALWAYS) {
    const v = src.get(name);
    if (v) out.set(name, v);
  }
  // Ensure XFF chain includes peer when CF headers missing (local/dev)
  const ip = clientIp(req);
  if (ip) {
    if (!out.has('x-real-ip')) out.set('x-real-ip', ip);
    if (!out.has('cf-connecting-ip') && !out.has('true-client-ip')) {
      const prev = out.get('x-forwarded-for');
      out.set('x-forwarded-for', prev ? `${ip}, ${prev}` : ip);
    } else if (!out.has('x-forwarded-for')) {
      out.set('x-forwarded-for', ip);
    }
  }
  // Mark first-party hop for origin observability (does not replace CF headers)
  out.set('x-gv5-relay', 'next-first-party');
  // Drop hop-by-hop if any slipped in
  for (const k of HOP_BY_HOP) out.delete(k);
  return out;
}

function pathFromParams(params: { path?: string[] } | undefined): string {
  const parts = params?.path;
  if (!parts || parts.length === 0) return '';
  // Next already splits on `/`; re-encode each segment (not the slashes).
  return parts
    .map((p) => encodeURIComponent(String(p)).replace(/%2F/gi, '/'))
    .join('/');
}

/**
 * Stream-proxy one request to the greenv5 origin.
 * Safe for POST /v1/ingest body of any size ≤ origin limit (nginx 2m).
 */
export async function relayRequest(
  req: Request,
  role: RelayRole,
  params?: { path?: string[] },
): Promise<Response> {
  const base = upstreamBase(role);
  const sub = pathFromParams(params);
  const url = new URL(req.url);
  const target = `${base}/${sub}${url.search}`;

  const method = req.method.toUpperCase();
  const headers = buildForwardHeaders(req);

  const timeoutMs = Number(process.env.GR_RELAY_TIMEOUT_MS || 60_000);
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), timeoutMs);

  // Body: pass through stream. GET/HEAD have no body.
  const hasBody = method !== 'GET' && method !== 'HEAD' && method !== 'OPTIONS';
  let body: BodyInit | undefined;
  if (hasBody && req.body) {
    // Node/Next fetch duplex streaming — avoid buffering full JSON.
    body = req.body as unknown as BodyInit;
  }

  try {
    if (method === 'OPTIONS') {
      // Preflight: answer locally (same-origin first-party rarely needs CORS,
      // but keep cheap 204 for mixed asset hosts).
      return new Response(null, {
        status: 204,
        headers: {
          'access-control-allow-origin': req.headers.get('origin') || '*',
          'access-control-allow-methods': 'GET,POST,PUT,OPTIONS',
          'access-control-allow-headers':
            req.headers.get('access-control-request-headers') ||
            'content-type,x-gv5-site-id,x-gv5-inject-path',
          'access-control-max-age': '600',
          'x-gv5-relay': 'next-first-party',
        },
      });
    }

    const init: RequestInit & { duplex?: 'half' } = {
      method,
      headers,
      body,
      signal: ctrl.signal,
      // @ts-expect-error undici duplex for streaming request body
      duplex: body ? 'half' : undefined,
      // Do not follow redirects silently — origin should answer 2xx/4xx directly
      redirect: 'manual',
      cache: 'no-store',
    };

    const upstream = await fetch(target, init);
    // Stream response back; strip hop-by-hop
    const respHeaders = new Headers();
    upstream.headers.forEach((v, k) => {
      if (HOP_BY_HOP.has(k.toLowerCase())) return;
      // Avoid compression mismatch if edge re-encodes
      if (k.toLowerCase() === 'content-encoding') return;
      respHeaders.set(k, v);
    });
    respHeaders.set('x-gv5-relay', 'next-first-party');
    respHeaders.set('x-gv5-relay-upstream', role);
    // Cache never for API; static dist handled by separate nginx/asset path when possible
    if (!respHeaders.has('cache-control')) {
      respHeaders.set('cache-control', 'no-store');
    }

    return new Response(upstream.body, {
      status: upstream.status,
      statusText: upstream.statusText,
      headers: respHeaders,
    });
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    const aborted = /abort/i.test(msg);
    return new Response(
      JSON.stringify({
        ok: false,
        error: aborted ? 'relay_timeout' : 'relay_upstream_error',
        detail: msg.slice(0, 200),
        role,
      }),
      {
        status: aborted ? 504 : 502,
        headers: {
          'content-type': 'application/json',
          'x-gv5-relay': 'next-first-party',
          'cache-control': 'no-store',
        },
      },
    );
  } finally {
    clearTimeout(timer);
  }
}

/** App Router handlers factory — zero-copy re-export pattern. */
export function createRelayHandlers(role: RelayRole) {
  const handle = (req: Request, ctx: { params: Promise<{ path?: string[] }> | { path?: string[] } }) => {
    const p = ctx.params;
    if (p && typeof (p as Promise<unknown>).then === 'function') {
      return (p as Promise<{ path?: string[] }>).then((params) =>
        relayRequest(req, role, params),
      );
    }
    return relayRequest(req, role, p as { path?: string[] });
  };
  return {
    GET: handle,
    POST: handle,
    PUT: handle,
    PATCH: handle,
    DELETE: handle,
    OPTIONS: handle,
    HEAD: handle,
  };
}
