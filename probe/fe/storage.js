/**
 * Visitor terminal + cycle resume + cool window (cookie / localStorage).
 *
 * cycle_id is the probe evidence bag (server sessions.session_id column).
 * Cool must survive Tracking Prevention: dual-write localStorage + first-party cookie
 * (parent domain so www/pv share when possible).
 *
 * **product_version is the authority** (unified FE + core VERSION) — not sticky cycle_id:
 * - same version + cool_until in future → skip re-probe / re-upload / re-analyze
 * - version change → auto clear cool **and** cycle bag, re-probe (no user cookie clear)
 * - cycle bag is stamped with the product_version that created it
 * - ensureProbeStateForVersion(V) is the single FE entry for inject/boot
 */
(function (global) {
  "use strict";
  // Production names are de-branded; legacy gr_* still read for migration.
  var VT_KEY = "_g5_vt";
  var VT_KEY_LEGACY = "gr_visitor_terminal_v1";
  var CYCLE_KEY = "_g5_cy";
  var CYCLE_KEY_LEGACY = "gr_probe_cycle_v1";
  var CYCLE_COOKIE = "_g5_c";
  var CYCLE_COOKIE_LEGACY = "gr_cycle_v1";
  var CYCLE_VER_KEY = "_g5_cv";
  var CYCLE_VER_KEY_LEGACY = "gr_cycle_product_version_v1";
  var CYCLE_VER_COOKIE = "_g5_cv";
  var CYCLE_VER_COOKIE_LEGACY = "gr_cycle_product_version_v1";
  var COOL_KEY = "_g5_k";
  var COOL_KEY_LEGACY = "gr_probe_cool_until_v1";
  var COOL_COOKIE = "_g5_k";
  var COOL_COOKIE_LEGACY = "gr_cool_until_v1";
  var PROD_VER_KEY = "_g5_pv";
  var PROD_VER_KEY_LEGACY = "gr_product_version_v1";
  var PROD_VER_COOKIE = "_g5_pv";
  var PROD_VER_COOKIE_LEGACY = "gr_product_version_v1";
  var BIND_KEY = "_g5_b";
  var BIND_COOKIE = "_g5_b";
  var VT_COOKIE = "_g5_vt";
  var VT_COOKIE_LEGACY = "gr_vt_v1";

  var RESERVED_PARENT_SUFFIX = {
    local: 1,
    test: 1,
    invalid: 1,
    localhost: 1,
    internal: 1,
    lan: 1,
    home: 1,
    corp: 1,
    localdomain: 1,
  };

  function parentCookieDomain() {
    try {
      var h = String((typeof location !== "undefined" && location.hostname) || "");
      if (!h || h === "localhost" || /^\d+\.\d+\.\d+\.\d+$/.test(h) || h.indexOf(":") >= 0) {
        return "";
      }
      var parts = h.split(".");
      if (parts.length < 2) return "";
      if (RESERVED_PARENT_SUFFIX[String(parts[parts.length - 1]).toLowerCase()]) {
        return "";
      }
      var m = h.match(/([a-z0-9-]+\.(?:com|net|org|edu|co)\.[a-z]{2})$/i);
      if (m) return "." + m[1];
      return "." + parts.slice(-2).join(".");
    } catch (e) {}
    return "";
  }

  function siteToken() {
    try {
      var boot = global.__GR_BOOT__ || {};
      var s = boot.site_id || boot.siteId || global.__GR_SITE_ID__ || "";
      s = String(s).replace(/[^a-zA-Z0-9_-]/g, "");
      if (s.length > 48) s = s.slice(0, 48);
      return s;
    } catch (e) {
      return "";
    }
  }

  function scopedCookieName(base) {
    var s = siteToken();
    return s ? base + "." + s : base;
  }

  function cookieSecureTail() {
    return typeof location !== "undefined" && location.protocol === "https:" ? "; secure" : "";
  }

  function expireCookieEverywhere(name) {
    try {
      var secure = cookieSecureTail();
      document.cookie = name + "=; path=/; max-age=0; samesite=lax" + secure;
      var h = String((typeof location !== "undefined" && location.hostname) || "");
      if (!h || h.indexOf(".") < 0) return;
      var parts = h.split(".");
      for (var i = 0; i < parts.length - 1; i++) {
        document.cookie =
          name +
          "=; path=/; max-age=0; samesite=lax; domain=." +
          parts.slice(i).join(".") +
          secure;
      }
    } catch (e) {}
  }

  function writeCookie(name, value, maxAgeSec) {
    try {
      var v = value == null ? "" : String(value);
      var age = maxAgeSec == null ? 86400 : maxAgeSec;
      if (!v) age = 0;
      var b =
        name +
        "=" +
        encodeURIComponent(v) +
        "; path=/; max-age=" +
        age +
        "; samesite=lax" +
        (typeof location !== "undefined" && location.protocol === "https:" ? "; secure" : "");
      var pd = parentCookieDomain();
      document.cookie = pd ? b + "; domain=" + pd : b;
      // also host-only fallback
      if (pd) document.cookie = b;
    } catch (e) {}
  }

  function readCookie(name) {
    try {
      var parts = String(document.cookie || "").split(";");
      for (var i = 0; i < parts.length; i++) {
        var p = parts[i].replace(/^\s+/, "");
        if (p.indexOf(name + "=") === 0) {
          return decodeURIComponent(p.slice(name.length + 1));
        }
      }
    } catch (e) {}
    return "";
  }

  /** Unified product version: inject boot → env → global. */
  function resolveProductVersion(explicit) {
    if (explicit != null && String(explicit).length) return String(explicit);
    try {
      var boot = global.__GR_BOOT__ || {};
      if (boot.version) return String(boot.version);
      if (boot.product_version) return String(boot.product_version);
    } catch (e0) {}
    try {
      if (global.__GR_PRODUCT_VERSION__) return String(global.__GR_PRODUCT_VERSION__);
    } catch (e1) {}
    return "";
  }

  function lsGet(primary, legacy) {
    try {
      var v = global.localStorage && localStorage.getItem(primary);
      if (v && String(v).length) return String(v);
    } catch (e) {}
    if (legacy) {
      try {
        var v2 = global.localStorage && localStorage.getItem(legacy);
        if (v2 && String(v2).length) {
          try {
            localStorage.setItem(primary, v2);
          } catch (eM) {}
          return String(v2);
        }
      } catch (e2) {}
    }
    return "";
  }

  function ckGet(primary, legacy) {
    var c = readCookie(primary);
    if (c) return c;
    if (legacy) {
      c = readCookie(legacy);
      if (c) {
        try {
          writeCookie(primary, c, 86400 * 30);
        } catch (e) {}
        return c;
      }
    }
    return "";
  }

  function getCoolProductVersion() {
    var v = lsGet(PROD_VER_KEY, PROD_VER_KEY_LEGACY);
    if (v) return v;
    return ckGet(PROD_VER_COOKIE, PROD_VER_COOKIE_LEGACY);
  }

  function setCoolProductVersion(ver, maxAgeSec) {
    var v = ver == null ? "" : String(ver);
    try {
      if (v) {
        localStorage.setItem(PROD_VER_KEY, v);
        localStorage.setItem(PROD_VER_KEY_LEGACY, v);
      } else {
        localStorage.removeItem(PROD_VER_KEY);
        localStorage.removeItem(PROD_VER_KEY_LEGACY);
      }
    } catch (e) {}
    var age = maxAgeSec == null ? 86400 : maxAgeSec;
    writeCookie(PROD_VER_COOKIE, v, age);
    // Dual-write legacy so micro/boot fallbacks that only read gr_* stay in sync.
    writeCookie(PROD_VER_COOKIE_LEGACY, v, age);
  }

  /**
   * Persist vt to ALL keys (primary + legacy cookie/ls) so micro and entry share one id.
   */
  function persistVt(id) {
    if (!id || String(id).indexOf("vt_") !== 0) return;
    var v = String(id);
    try {
      localStorage.setItem(VT_KEY, v);
      localStorage.setItem(VT_KEY_LEGACY, v);
    } catch (e) {}
    writeCookie(VT_COOKIE, v, 86400 * 30);
    writeCookie(VT_COOKIE_LEGACY, v, 86400 * 30);
  }

  function getVt() {
    var v = lsGet(VT_KEY, VT_KEY_LEGACY);
    if (v && v.length >= 12 && String(v).indexOf("vt_") === 0) {
      persistVt(v);
      return v;
    }
    var ck = ckGet(VT_COOKIE, VT_COOKIE_LEGACY);
    if (ck && ck.indexOf("vt_") === 0 && ck.length >= 12) {
      persistVt(ck);
      return ck;
    }
    var nid =
      "vt_" + Date.now().toString(36) + "_" + Math.random().toString(36).slice(2, 10);
    persistVt(nid);
    try {
      if (global.GROps && GROps.vtMint) {
        GROps.vtMint("fresh_local", nid, { source: "storage" });
      } else if (global.GROps && GROps.report) {
        GROps.report(
          "vt_mint",
          "storage",
          { reason: "fresh_local", vt_mint_reason: "fresh_local", vt: nid },
          "info"
        );
      }
    } catch (eO) {}
    return nid;
  }

  /** Adopt server-returned vt (never invent if server already bound). */
  function adoptVt(serverVt) {
    if (!serverVt || String(serverVt).indexOf("vt_") !== 0) return getVt();
    persistVt(String(serverVt));
    return String(serverVt);
  }

  function getCycleId() {
    var c = lsGet(CYCLE_KEY, CYCLE_KEY_LEGACY);
    if (c && c.length >= 8) return c;
    var scoped = ckGet(scopedCookieName(CYCLE_COOKIE), scopedCookieName(CYCLE_COOKIE_LEGACY));
    if (scoped && scoped.indexOf("cycle_") === 0 && scoped.length >= 12) return scoped;
    // Unscoped parent-domain cycle joins sibling site_ids (shop.gr.local vs news).
    if (!siteToken()) {
      var ck = ckGet(CYCLE_COOKIE, CYCLE_COOKIE_LEGACY);
      if (ck && ck.indexOf("cycle_") === 0 && ck.length >= 12) return ck;
    }
    return null;
  }

  function getCycleProductVersion() {
    var v = lsGet(CYCLE_VER_KEY, CYCLE_VER_KEY_LEGACY);
    if (v) return v;
    return ckGet(CYCLE_VER_COOKIE, CYCLE_VER_COOKIE_LEGACY);
  }

  function setCycleProductVersion(ver, maxAgeSec) {
    var v = ver == null ? "" : String(ver);
    try {
      if (v) {
        localStorage.setItem(CYCLE_VER_KEY, v);
        localStorage.setItem(CYCLE_VER_KEY_LEGACY, v);
      } else {
        localStorage.removeItem(CYCLE_VER_KEY);
        localStorage.removeItem(CYCLE_VER_KEY_LEGACY);
      }
    } catch (e) {}
    var age = maxAgeSec == null ? 86400 : maxAgeSec;
    writeCookie(CYCLE_VER_COOKIE, v, age);
    writeCookie(CYCLE_VER_COOKIE_LEGACY, v, age);
  }

  /**
   * @param {string} id cycle id
   * @param {string} [productVersion] stamp bag to this product_version
   */
  function setCycleId(id, productVersion) {
    if (!id) return;
    try {
      localStorage.setItem(CYCLE_KEY, id);
      localStorage.setItem(CYCLE_KEY_LEGACY, id);
    } catch (e) {}
    writeCookie(scopedCookieName(CYCLE_COOKIE), id, 86400);
    writeCookie(scopedCookieName(CYCLE_COOKIE_LEGACY), id, 86400);
    expireCookieEverywhere(CYCLE_COOKIE);
    expireCookieEverywhere(CYCLE_COOKIE_LEGACY);
    var ver = resolveProductVersion(productVersion);
    if (ver) setCycleProductVersion(ver, 86400);
  }

  /** Drop sticky cycle (completed bag / version remint). */
  function clearCycleId() {
    try {
      localStorage.removeItem(CYCLE_KEY);
      localStorage.removeItem(CYCLE_KEY_LEGACY);
    } catch (e) {}
    writeCookie(scopedCookieName(CYCLE_COOKIE), "", 0);
    writeCookie(scopedCookieName(CYCLE_COOKIE_LEGACY), "", 0);
    expireCookieEverywhere(CYCLE_COOKIE);
    expireCookieEverywhere(CYCLE_COOKIE_LEGACY);
    setCycleProductVersion("", 0);
  }

  function mintCycleId() {
    try {
      return (
        "cycle_" +
        Date.now().toString(16) +
        Math.random().toString(16).slice(2, 10)
      );
    } catch (e) {
      return "cycle_" + String(Date.now());
    }
  }

  /**
   * Single FE gate: align cool + cycle bag with current product_version.
   * Call from inject micro-kick and boot **before** open/B8/identity.
   * Never requires the user to clear cookies.
   *
   * @returns {{ coolActive: boolean, cycleId: string|null, reminted: boolean, coolCleared: boolean, reason: string }}
   */
  function ensureProbeStateForVersion(productVersion) {
    var ver = resolveProductVersion(productVersion);
    var out = {
      coolActive: false,
      cycleId: null,
      reminted: false,
      coolCleared: false,
      reason: "",
    };
    clearCoolIfExpired(Date.now());
    if (ver) {
      var coolCleared = clearCoolIfVersionMismatch(ver);
      if (coolCleared) {
        out.coolCleared = true;
        out.reason = "product_version_mismatch";
      }
      // Cycle bag stamped under another VERSION cannot accept identity ingest.
      var cycVer = getCycleProductVersion();
      var cyc = getCycleId();
      if (cyc && cycVer && cycVer !== ver) {
        clearCycleId();
        out.reminted = true;
        out.reason = out.reason || "cycle_version_mismatch";
      } else if (cyc && !cycVer) {
        // Legacy unstamped bag — drop so we never POST to a completed pre-version bag.
        clearCycleId();
        out.reminted = true;
        out.reason = out.reason || "cycle_unstamped_legacy";
      }
    }
    out.coolActive = isCoolActive(Date.now(), ver);
    if (out.coolActive) {
      // Cool for this version: keep cycle for cool-open join only.
      out.cycleId = getCycleId();
      return out;
    }
    // Join micro-kick's page-local mint so B8 and identity share one bag.
    try {
      var microSid = global.__GR_CYCLE_HINT__ || global.__GR_SESSION_ID__ || "";
      if (
        global.__GR_MICRO_KICK__ &&
        microSid &&
        String(microSid).indexOf("cycle_") === 0 &&
        String(microSid).length >= 12
      ) {
        setCycleId(String(microSid), ver);
        out.cycleId = String(microSid);
        out.reason = out.reason || "micro_kick_join";
        return out;
      }
    } catch (eM) {}
    // Incomplete same-version: REUSE sticky cycle (gap-fill). Commercial products keep a
    // stable identity bag until final/TTL — reminting every refresh produced empty dg_* cycles
    // while an older bag already held dh_*. Server keeps prior batches; FE only supplements.
    var sticky = getCycleId();
    var stickyVer = getCycleProductVersion();
    if (
      sticky &&
      String(sticky).indexOf("cycle_") === 0 &&
      String(sticky).length >= 12 &&
      (!ver || !stickyVer || stickyVer === ver)
    ) {
      if (ver && !stickyVer) setCycleProductVersion(ver, 86400);
      out.cycleId = sticky;
      out.reminted = false;
      out.reason = out.reason || "resume_incomplete_sticky";
      try {
        global.__GR_KICK_MS__ = global.__GR_KICK_MS__ || {};
        global.__GR_KICK_MS__.probe_state = {
          reminted: false,
          coolCleared: out.coolCleared,
          reason: out.reason,
          version: ver || null,
        };
      } catch (eK0) {}
      return out;
    }
    // Page mint budget: one fresh cycle per page load.
    try {
      if (global.__GR_PAGE_CYCLE_MINTED__) {
        var pageC = String(global.__GR_PAGE_CYCLE_MINTED__);
        if (pageC.indexOf("cycle_") === 0 && pageC.length >= 12) {
          setCycleId(pageC, ver);
          out.cycleId = pageC;
          out.reminted = false;
          out.reason = out.reason || "page_mint_budget";
          return out;
        }
      }
    } catch (ePg) {}
    // Cross-remount page budget: SPA soft-nav may re-run inject; keep one mint per tab+version.
    try {
      var tabKey = "gr_page_cycle_" + String(ver || "dev");
      var tabMint = sessionStorage.getItem(tabKey);
      if (tabMint && String(tabMint).indexOf("cycle_") === 0 && String(tabMint).length >= 12) {
        setCycleId(String(tabMint), ver);
        try {
          global.__GR_PAGE_CYCLE_MINTED__ = String(tabMint);
        } catch (eTm) {}
        out.cycleId = String(tabMint);
        out.reminted = false;
        out.reason = out.reason || "session_storage_page_budget";
        return out;
      }
    } catch (eSs) {}
    // No sticky / version-cleared → mint fresh bag for this product_version.
    var nid = mintCycleId();
    try {
      global.__GR_PAGE_CYCLE_MINTED__ = nid;
      sessionStorage.setItem("gr_page_cycle_" + String(ver || "dev"), nid);
    } catch (ePm) {}
    setCycleId(nid, ver);
    out.cycleId = nid;
    out.reminted = true;
    if (!out.reason) out.reason = "fresh_cycle";
    try {
      global.__GR_KICK_MS__ = global.__GR_KICK_MS__ || {};
      global.__GR_KICK_MS__.probe_state = {
        reminted: out.reminted,
        coolCleared: out.coolCleared,
        reason: out.reason,
        version: ver || null,
      };
      // Ops: at most one remint report per tab+version (SPA remount storms).
      var remKey = "gr_remint_reported_" + String(ver || "dev");
      var alreadyRem = false;
      try {
        alreadyRem = sessionStorage.getItem(remKey) === "1";
      } catch (eRk) {}
      if (!alreadyRem) {
        try {
          sessionStorage.setItem(remKey, "1");
        } catch (eRk2) {}
        if (global.GROps && GROps.cycleRemint) {
          GROps.cycleRemint(out.reason, { version: ver || null });
        } else if (global.GROps && GROps.report) {
          // fresh_cycle is expected on first paint — info (not warn flood).
          var remSev =
            out.reason === "fresh_cycle" || out.reason === "session_storage_page_budget"
              ? "info"
              : "warn";
          GROps.report(
            "cycle_remint",
            "storage",
            { reason: out.reason, version: ver || null },
            remSev
          );
        }
      }
    } catch (eK) {}
    return out;
  }

  function getCoolUntil() {
    var n = 0;
    try {
      n = parseInt(lsGet(COOL_KEY, COOL_KEY_LEGACY) || "0", 10);
      if (!isFinite(n)) n = 0;
    } catch (e) {
      n = 0;
    }
    // Cookie fallback when Tracking Prevention blocks storage
    if (!n) {
      try {
        var ck = ckGet(scopedCookieName(COOL_COOKIE), scopedCookieName(COOL_COOKIE_LEGACY));
        if (!ck && !siteToken()) ck = ckGet(COOL_COOKIE, COOL_COOKIE_LEGACY);
        if (ck) {
          var m = parseInt(ck, 10);
          if (isFinite(m) && m > 0) n = m;
        }
      } catch (e2) {}
    }
    return n;
  }

  function getStorageBind() {
    var b = lsGet(BIND_KEY, "");
    if (b) return b;
    return ckGet(BIND_COOKIE, "");
  }

  function setStorageBind(bind, maxAgeSec) {
    var v = bind == null ? "" : String(bind);
    try {
      if (v) localStorage.setItem(BIND_KEY, v);
      else localStorage.removeItem(BIND_KEY);
    } catch (e) {}
    writeCookie(BIND_COOKIE, v, maxAgeSec == null ? 86400 : maxAgeSec);
  }

  /**
   * @param {number} ms cool_until epoch ms (0 clears)
   * @param {string} [productVersion] stamp for version-scoped cool
   */
  function setCoolUntil(ms, productVersion) {
    var n = ms ? parseInt(ms, 10) : 0;
    if (!isFinite(n)) n = 0;
    try {
      if (n) {
        localStorage.setItem(COOL_KEY, String(n));
        localStorage.setItem(COOL_KEY_LEGACY, String(n));
      } else {
        localStorage.removeItem(COOL_KEY);
        localStorage.removeItem(COOL_KEY_LEGACY);
      }
    } catch (e) {}
    // Cookie max-age = remaining cool window (at least 60s if still cool)
    var now = Date.now();
    var maxAge = 0;
    if (n > now) {
      maxAge = Math.max(60, Math.floor((n - now) / 1000));
    }
    writeCookie(scopedCookieName(COOL_COOKIE), n ? String(n) : "", maxAge);
    writeCookie(scopedCookieName(COOL_COOKIE_LEGACY), n ? String(n) : "", maxAge);
    expireCookieEverywhere(COOL_COOKIE);
    expireCookieEverywhere(COOL_COOKIE_LEGACY);
    if (n > now) {
      var ver = resolveProductVersion(productVersion);
      if (ver) setCoolProductVersion(ver, maxAge);
    } else {
      setCoolProductVersion("", 0);
    }
  }

  /** If cool_until is past, clear it so the next open does a full re-probe. */
  function clearCoolIfExpired(nowMs) {
    var until = getCoolUntil();
    var now = nowMs != null ? nowMs : Date.now();
    if (until > 0 && until <= now) {
      setCoolUntil(0);
      return true;
    }
    return false;
  }

  /**
   * Version mismatch / missing stamp while cool wall-clock remains → clear cool
   * so new product_version re-probes, uploads, and analyzes.
   */
  function clearCoolIfVersionMismatch(productVersion) {
    var cur = resolveProductVersion(productVersion);
    if (!cur) return false;
    var until = getCoolUntil();
    var now = Date.now();
    if (!(until > now)) return false;
    var stored = getCoolProductVersion();
    if (!stored || stored !== cur) {
      setCoolUntil(0);
      // Also drop sticky cycle — completed bag under old version causes ingest 410
      // while gateway/open may mint a new cycle (B8-only / dwell-with-no-identity).
      clearCycleId();
      try {
        global.__GR_KICK_MS__ = global.__GR_KICK_MS__ || {};
        global.__GR_KICK_MS__.cool_cleared_version =
          (stored || "(none)") + "→" + cur;
        global.__GR_KICK_MS__.cycle_cleared_version = true;
      } catch (e) {}
      return true;
    }
    return false;
  }

  /**
   * True while identity cool window is still active **for current product_version**.
   * @param {number} [nowMs]
   * @param {string} [productVersion]
   */
  function isCoolActive(nowMs, productVersion) {
    clearCoolIfExpired(nowMs);
    clearCoolIfVersionMismatch(productVersion);
    var until = getCoolUntil();
    var now = nowMs != null ? nowMs : Date.now();
    if (!(until > now)) return false;
    var cur = resolveProductVersion(productVersion);
    if (!cur) {
      // No version available yet — time-only cool (legacy).
      return true;
    }
    var stored = getCoolProductVersion();
    return !!stored && stored === cur;
  }

  // ─── iss/61 F1: fuzzy-ECC Helper Data (parity-only, no raw fingerprint) ───
  // Server returns helper in route_plan.fuzzy_helper; FE stores and echoes it next
  // session so same-machine curve micro-drift stabilizes instead of wrong-splitting.
  // localStorage only (default permission; no popup, no IDB prompt).
  var FUZZY_HELPER_KEY = "_g5_fh";
  var FUZZY_HELPER_MAX_AGE_MS = 30 * 24 * 3600 * 1000;

  function sanitizeHelperArr(a) {
    if (!a || !a.length) return null;
    var out = [];
    for (var i = 0; i < a.length && i < 256; i++) {
      var n = Number(a[i]);
      if (!isFinite(n) || n < 0 || n > 65535) return null;
      out.push(Math.floor(n));
    }
    return out.length ? out : null;
  }

  function getFuzzyHelper() {
    try {
      var raw = global.localStorage && localStorage.getItem(FUZZY_HELPER_KEY);
      if (!raw) return null;
      var j = JSON.parse(raw);
      if (!j || j.v !== 1) return null;
      if (j.ts && Date.now() - j.ts > FUZZY_HELPER_MAX_AGE_MS) return null;
      var wg = sanitizeHelperArr(j.wg);
      var au = sanitizeHelperArr(j.au);
      if (!wg && !au) return null;
      return { v: 1, wg: wg, au: au, ts: j.ts || 0 };
    } catch (e) {}
    return null;
  }

  function setFuzzyHelper(h) {
    try {
      if (!h || h.v !== 1) return false;
      var wg = sanitizeHelperArr(h.wg);
      var au = sanitizeHelperArr(h.au);
      if (!wg && !au) return false;
      localStorage.setItem(
        FUZZY_HELPER_KEY,
        JSON.stringify({ v: 1, wg: wg, au: au, ts: Date.now() })
      );
      return true;
    } catch (e) {}
    return false;
  }

  function cacheBust(url, version) {
    if (!version) return url;
    return url + (url.indexOf("?") >= 0 ? "&" : "?") + "v=" + encodeURIComponent(version);
  }

  function warmCache(urls) {
    (urls || []).forEach(function (u) {
      try {
        fetch(u, { mode: "cors", credentials: "omit", cache: "force-cache" }).catch(function () {});
      } catch (e) {}
    });
  }

  global.GRStorage = {
    visitorTerminalId: getVt,
    adoptVt: adoptVt,
    persistVt: persistVt,
    getCycleId: getCycleId,
    setCycleId: setCycleId,
    clearCycleId: clearCycleId,
    getCycleProductVersion: getCycleProductVersion,
    setCycleProductVersion: setCycleProductVersion,
    mintCycleId: mintCycleId,
    ensureProbeStateForVersion: ensureProbeStateForVersion,
    getCoolUntil: getCoolUntil,
    setCoolUntil: setCoolUntil,
    getFuzzyHelper: getFuzzyHelper,
    setFuzzyHelper: setFuzzyHelper,
    clearCoolIfExpired: clearCoolIfExpired,
    clearCoolIfVersionMismatch: clearCoolIfVersionMismatch,
    isCoolActive: isCoolActive,
    resolveProductVersion: resolveProductVersion,
    getCoolProductVersion: getCoolProductVersion,
    getStorageBind: getStorageBind,
    setStorageBind: setStorageBind,
    cacheBust: cacheBust,
    warmCache: warmCache,
    readCookie: readCookie,
    writeCookie: writeCookie,
    VT_KEY: VT_KEY,
    CYCLE_KEY: CYCLE_KEY,
    COOL_KEY: COOL_KEY,
    COOL_COOKIE: COOL_COOKIE,
    PROD_VER_KEY: PROD_VER_KEY,
    PROD_VER_COOKIE: PROD_VER_COOKIE,
    BIND_KEY: BIND_KEY,
  };
})(typeof window !== "undefined" ? window : globalThis);
