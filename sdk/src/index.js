/**
 * Green V7 results SDK — backend only (GR naming since 8.0; X-Gr-Sdk-Key).
 * Never embed this with a site backend key in the browser.
 * Thin client: getResult / waitForResult / query. No probe relay.
 */

export class GrResultClient {
  /**
   * @param {{ baseUrl: string, apiKey: string, timeoutMs?: number, expectedSchema?: string }} opts
   */
  constructor(opts) {
    this.baseUrl = String(opts.baseUrl || "").replace(/\/+$/, "");
    this.apiKey = String(opts.apiKey || "");
    this.timeoutMs = opts.timeoutMs || 15000;
    this.expectedSchema = opts.expectedSchema || "product_public_v1";
  }

  /**
   * @param {string} sessionId
   * @param {{ projection?: "public"|"sdk"|"diagnostic", strategyId?: string, responseProfile?: string, profileCap?: string, lang?: string, wait?: { timeoutMs: number, intervalMs: number } }} [options]
   */
  async getResult(sessionId, options = {}) {
    const projection = options.projection || "sdk";
    if (options.wait) {
      return this.waitForResult(sessionId, { ...options, ...options.wait, projection });
    }
    return this.#fetchResult(sessionId, projection, options);
  }

  /**
   * Query a session result (projection-aware). `wait: true` polls until the
   * analysis is no longer pending or the timeout is reached.
   * @param {string} sessionId
   * @param {{ projection?: string, strategyId?: string, responseProfile?: string, profileCap?: string, lang?: string, wait?: boolean, timeoutMs?: number, intervalMs?: number }} [options]
   */
  async query(sessionId, options = {}) {
    if (options.wait) {
      return this.waitForResult(sessionId, {
        projection: options.projection || "sdk",
        strategyId: options.strategyId,
        responseProfile: options.responseProfile,
        profileCap: options.profileCap,
        lang: options.lang,
        timeoutMs: options.timeoutMs,
        intervalMs: options.intervalMs,
      });
    }
    return this.#fetchResult(sessionId, options.projection || "sdk", options);
  }

  /**
   * Extract the site-owner allowlisted cookies captured server-side at
   * session open/ingest (business-identifier binding). Returns an object
   * like `{ "user_id": "u9", "cart": "c42" }` or null when the site has no
   * cookie allowlist or the visitor sent no cookies.
   * @param {object} result — a getResult/query/waitForResult body.
   * @returns {object|null}
   */
  static cookieFields(result) {
    if (!result || typeof result !== "object") return null;
    const cf =
      result.sdk_projection?.cookie_fields ??
      result.product_public?.cookie_fields ??
      result.cookie_fields;
    if (!cf || typeof cf !== "object") return null;
    return Object.keys(cf).length ? cf : null;
  }

  async waitForResult(sessionId, options = {}) {
    const timeoutMs = options.timeoutMs || 8000;
    const intervalMs = options.intervalMs || 250;
    const projection = options.projection || "sdk";
    const start = Date.now();
    let last = null;
    while (Date.now() - start < timeoutMs) {
      last = await this.#fetchResult(sessionId, projection, options);
      if (last && last.ok && (last.product_public || last.sdk_projection)) {
        const pending = last.product_public?.meta?.analysis_pending;
        if (!pending) return last;
      }
      await new Promise((r) => setTimeout(r, intervalMs));
    }
    const err = new Error("analysis_pending");
    err.body = last;
    throw err;
  }

  async #fetchResult(sessionId, projection, options = {}) {
    const params = new URLSearchParams({ projection });
    for (const [key, value] of [
      ["strategy_id", options.strategyId],
      ["response_profile", options.responseProfile],
      ["profile_cap", options.profileCap],
      ["lang", options.lang],
    ]) {
      if (value !== undefined && value !== null && String(value) !== "") {
        params.set(key, String(value));
      }
    }
    const url = `${this.baseUrl}/v1/session/${encodeURIComponent(sessionId)}/result?${params.toString()}`;
    const ctrl = new AbortController();
    const t = setTimeout(() => ctrl.abort(), this.timeoutMs);
    try {
      const res = await fetch(url, {
        method: "GET",
        headers: {
          Accept: "application/json",
          "X-Gr-Sdk-Key": this.apiKey,
          "X-Request-Id": `req_${Date.now()}`,
        },
        signal: ctrl.signal,
      });
      const body = await res.json().catch(() => ({}));
      if (!res.ok) {
        const err = new Error(body?.error?.code || body?.error || `http_${res.status}`);
        err.status = res.status;
        err.body = body;
        err.retryable = res.status === 429 || res.status >= 502;
        throw err;
      }
      if (this.expectedSchema && body.schema_version && body.schema_version !== this.expectedSchema) {
        // tolerate missing schema_version on older nodes
      }
      return body;
    } finally {
      clearTimeout(t);
    }
  }

  /**
   * @param {string} sessionId
   * @param {{ idempotencyKey?: string }} [options]
   */
  async triggerAnalyze(sessionId, options = {}) {
    return this.#postSession(sessionId, "analyze", {}, options.idempotencyKey);
  }

  /**
   * @param {string} sessionId
   * @param {{ idempotencyKey?: string }} [options]
   */
  async completeSession(sessionId, options = {}) {
    return this.#postSession(sessionId, "complete", {}, options.idempotencyKey);
  }

  async #postSession(sessionId, action, payload, idempotencyKey) {
    const url = `${this.baseUrl}/v1/session/${encodeURIComponent(sessionId)}/${action}`;
    const ctrl = new AbortController();
    const t = setTimeout(() => ctrl.abort(), this.timeoutMs);
    try {
      const headers = {
        Accept: "application/json",
        "Content-Type": "application/json",
        "X-Gr-Sdk-Key": this.apiKey,
        "X-Request-Id": `req_${Date.now()}`,
      };
      if (idempotencyKey) headers["Idempotency-Key"] = idempotencyKey;
      const res = await fetch(url, {
        method: "POST",
        headers,
        body: JSON.stringify(payload || {}),
        signal: ctrl.signal,
      });
      const body = await res.json().catch(() => ({}));
      if (!res.ok) {
        const err = new Error(body?.error?.code || body?.error || `http_${res.status}`);
        err.status = res.status;
        err.body = body;
        err.retryable = res.status === 429 || res.status >= 502;
        throw err;
      }
      return body;
    } finally {
      clearTimeout(t);
    }
  }
}

export default GrResultClient;
