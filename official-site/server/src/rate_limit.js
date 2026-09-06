/**
 * Shared sliding-window rate limiter (GV6-AUTH-008).
 * Uses PostgreSQL when attached so multiple API processes share one budget.
 * X-Forwarded-For is trusted only when GV6_TRUST_PROXY / GV6_TRUST_FORWARDED=1.
 */

const buckets = new Map();
let dbRef = null;

export function attachRateLimitDb(db) {
  dbRef = db || null;
}

export async function rateLimitAllow(key, { limit = 30, windowMs = 60_000 } = {}) {
  const now = Date.now();
  if (dbRef) {
    try {
      const cutoff = now - windowMs;
      await dbRef.run("DELETE FROM rate_limit_hits WHERE k=? AND ts < ?", key, cutoff);
      await dbRef.run("INSERT INTO rate_limit_hits(k, ts) VALUES (?, ?)", key, now);
      const row = await dbRef.get(
        "SELECT COUNT(*)::int AS n FROM rate_limit_hits WHERE k=? AND ts >= ?",
        key,
        cutoff
      );
      return (row?.n || 0) <= limit;
    } catch {
      /* fall through to in-process */
    }
  }
  let hits = buckets.get(key) || [];
  hits = hits.filter((t) => now - t < windowMs);
  if (hits.length >= limit) {
    buckets.set(key, hits);
    return false;
  }
  hits.push(now);
  buckets.set(key, hits);
  if (buckets.size > 20_000) {
    for (const [k, v] of buckets) {
      if (!v.length || now - v[v.length - 1] > windowMs) buckets.delete(k);
    }
  }
  return true;
}

export function clientIp(req) {
  const trust =
    (process.env.GR_TRUST_PROXY ?? process.env.GV6_TRUST_PROXY) === "1" ||
    (process.env.GR_TRUST_FORWARDED ?? process.env.GV6_TRUST_FORWARDED) === "1" ||
    (process.env.GR_TRUST_FORWARDED ?? process.env.GV5_TRUST_FORWARDED) === "1";
  if (trust) {
    const xf = String(req.headers?.["x-forwarded-for"] || "")
      .split(",")[0]
      .trim();
    if (xf) return xf;
  }
  return req.socket?.remoteAddress || req.ip || "unknown";
}
